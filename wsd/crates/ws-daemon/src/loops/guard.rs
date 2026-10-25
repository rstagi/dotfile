//! Guard context from durable session ancestry.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, io, path::Path};
use ws_core::{
    ids::SessionId,
    log::{LogEntry, SessionLog},
};

use crate::{
    api::{self, ApiError, Store},
    sessions::{Result as SessionResult, Sessions},
};

pub(crate) struct Context {
    pub(crate) id: SessionId,
    role: Option<String>,
    entries: Vec<LogEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    session: SessionId,
    agent: String,
    payload: Value,
}

pub(crate) async fn run(
    State(store): State<Store>,
    Json(request): Json<Request>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| check(sessions, request)).await?,
    ))
}

/// Finds the nearest loop ancestor. Reads no registry and changes no state.
pub(crate) fn context(
    state: &Path,
    session: SessionId,
    parent: Option<SessionId>,
) -> io::Result<Option<Context>> {
    let mut next = Some(session);
    let mut seen = BTreeSet::new();
    let mut role = None;
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "The session parents contain a cycle.",
            ));
        }
        let entries = SessionLog::new(state, id, SessionId::default())
            .read()?
            .entries;
        let created = entries
            .iter()
            .find(|entry| entry.event_type == "session.created");
        if created.is_some_and(|entry| entry.body["kind"] == "loop") {
            return Ok(Some(Context { id, role, entries }));
        }
        if role.is_none() {
            role = created.and_then(|entry| entry.body["loop"]["role"].as_str().map(str::to_owned));
        }
        next = match entries
            .iter()
            .rev()
            .find(|entry| entry.event_type == "session.reparented")
        {
            Some(entry) => entry.body["parent"].as_str().and_then(|id| id.parse().ok()),
            None => created
                .and_then(|entry| entry.body["parent_session_id"].as_str())
                .and_then(|id| id.parse().ok())
                .or_else(|| (id == session).then_some(parent).flatten()),
        };
    }
    Ok(None)
}

fn check(sessions: &mut Sessions, request: Request) -> SessionResult<Value> {
    let session = sessions.get_record(&request.session.to_string())?;
    let Some(context) = context(&sessions.state, session.id, None)? else {
        return Ok(json!({}));
    };
    let Some(command) = shell_command(&request.payload) else {
        return Ok(json!({}));
    };
    let branches = integration_branches(&context);
    let cwd = request.payload["cwd"].as_str().map(Path::new);
    let Some((rule, reason)) = classify(
        &command,
        &branches,
        cwd,
        context.role.as_deref() == Some("fix"),
        0,
    ) else {
        return Ok(json!({}));
    };
    sessions.append_daemon_loop_event(
        &context.id.to_string(),
        "loop.guard",
        json!({
            "session":session.id, "rule":rule, "command":command
        })
        .as_object()
        .unwrap()
        .clone(),
    )?;
    // Grok command hooks report observations but cannot cancel a tool call.
    let agent = session.body["agent"].as_str().unwrap_or(&request.agent);
    if !matches!(agent, "claude" | "codex") {
        return Ok(json!({}));
    }
    Ok(json!({"hookSpecificOutput":{
        "hookEventName":"PreToolUse", "permissionDecision":"deny", "permissionDecisionReason":reason
    }}))
}

fn shell_command(payload: &Value) -> Option<String> {
    let tool = payload["tool_name"]
        .as_str()
        .or_else(|| payload["toolName"].as_str())?;
    if !matches!(
        tool.rsplit('.').next()?,
        "Bash" | "bash" | "shell" | "exec_command" | "shell_command" | "run_shell_command"
    ) {
        return None;
    }
    let input = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))?;
    let parsed;
    let input = if let Some(text) = input.as_str() {
        parsed = serde_json::from_str::<Value>(text).ok()?;
        &parsed
    } else {
        input
    };
    let command = input.get("command").or_else(|| input.get("cmd"))?;
    command.as_str().map(str::to_owned).or_else(|| {
        Some(shell_words::join(
            command
                .as_array()?
                .iter()
                .map(|word| word.as_str())
                .collect::<Option<Vec<_>>>()?,
        ))
    })
}

fn integration_branches(context: &Context) -> Vec<String> {
    let text = context
        .entries
        .iter()
        .rev()
        .find(|entry| entry.event_type == "loop.plan")
        .and_then(|entry| entry.body["text"].as_str());
    let fallback = context
        .entries
        .first()
        .and_then(|entry| entry.body["loop_dir"].as_str())
        .and_then(|directory| std::fs::read_to_string(Path::new(directory).join("plan.md")).ok());
    text.or(fallback.as_deref())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (_, value) = line.split_once("**Integration branch")?;
            let (_, value) = value.split_once(":**")?;
            let branch = value.trim().trim_matches('`').trim();
            (!branch.is_empty()).then(|| branch.to_owned())
        })
        .collect()
}

fn classify(
    command: &str,
    branches: &[String],
    cwd: Option<&Path>,
    fix: bool,
    depth: u8,
) -> Option<(&'static str, &'static str)> {
    if depth > 8 {
        return None;
    }
    for words in commands(command, 0) {
        let index = command_start(&words);
        let Some(program) = words
            .get(index)
            .and_then(|word| Path::new(word).file_name())
            .and_then(|word| word.to_str())
        else {
            continue;
        };
        let args = &words[index + 1..];
        match program {
            "git" => {
                let mut index = 0;
                let mut directory = cwd.map(Path::to_path_buf);
                while let Some(arg) = args.get(index).filter(|arg| arg.starts_with('-')) {
                    let value_option = [
                        "-C",
                        "-c",
                        "--git-dir",
                        "--work-tree",
                        "--namespace",
                        "--config-env",
                    ]
                    .contains(&arg.as_str());
                    if arg == "-C"
                        && let Some(path) = args.get(index + 1)
                    {
                        directory = Some(directory.unwrap_or_default().join(path));
                    }
                    index += if value_option { 2 } else { 1 };
                }
                match args.get(index).map(String::as_str) {
                    Some("merge") => {
                        return Some(("no-merge", "Use loop-merge.sh to merge a lane."));
                    }
                    Some("push")
                        if !fix
                            && push_targets_integration(
                                &args[index + 1..],
                                branches,
                                directory.as_deref(),
                            ) =>
                    {
                        return Some((
                            "no-push",
                            "Use loop-merge.sh to push the integration branch.",
                        ));
                    }
                    _ => {}
                }
            }
            "gh" if args.windows(2).any(|pair| pair == ["pr", "merge"]) => {
                return Some((
                    "no-pr-merge",
                    "Use loop-merge.sh for lane merges. The human merges the PR.",
                ));
            }
            "claude"
                if args
                    .iter()
                    .any(|arg| arg == "-p" || arg == "--print" || arg.starts_with("--print=")) =>
            {
                return Some((
                    "no-agent-spawn",
                    "Use phase_spawn or ws session create to start an agent.",
                ));
            }
            "codex"
                if subcommand(
                    args,
                    &[
                        "-c",
                        "--config",
                        "-m",
                        "--model",
                        "-p",
                        "--profile",
                        "-C",
                        "--cd",
                        "--sandbox",
                        "--ask-for-approval",
                    ],
                ) == Some("exec") =>
            {
                return Some((
                    "no-agent-spawn",
                    "Use phase_spawn or ws session create to start an agent.",
                ));
            }
            "grok" => {
                return Some((
                    "no-agent-spawn",
                    "Use phase_spawn or ws session create to start an agent.",
                ));
            }
            "sh" | "bash" | "zsh" | "dash" => {
                if let Some(index) = args
                    .iter()
                    .position(|arg| arg.starts_with('-') && arg.contains('c'))
                    && let Some(script) = args.get(index + 1)
                    && let Some(rule) = classify(script, branches, cwd, fix, depth + 1)
                {
                    return Some(rule);
                }
            }
            _ => {}
        }
    }
    None
}

fn push_targets_integration(args: &[String], branches: &[String], cwd: Option<&Path>) -> bool {
    if branches.is_empty() {
        return false;
    }
    let mut positional = Vec::new();
    let mut index = 0;
    let mut repo_option = false;
    while let Some(arg) = args.get(index) {
        if matches!(arg.as_str(), "--all" | "--mirror" | "--matching") {
            return true;
        }
        if ["--repo", "--receive-pack", "--exec", "--push-option", "-o"].contains(&arg.as_str()) {
            repo_option |= arg == "--repo";
            index += 2;
            continue;
        }
        if arg.starts_with("--repo=") {
            repo_option = true;
        }
        if !arg.starts_with('-') {
            positional.push(arg.as_str());
        }
        index += 1;
    }
    let refs = if repo_option {
        positional.as_slice()
    } else {
        positional.get(1..).unwrap_or_default()
    };
    if !refs.is_empty() {
        return refs.iter().any(|reference| {
            let target = reference
                .rsplit_once(':')
                .map_or(*reference, |(_, target)| target)
                .trim_start_matches('+')
                .trim_start_matches("refs/heads/");
            target.is_empty()
                || branches.iter().any(|branch| {
                    target == branch
                        || (target == "HEAD" && current_branch(cwd).as_deref() == Some(branch))
                        || glob::Pattern::new(target).is_ok_and(|pattern| pattern.matches(branch))
                })
        });
    }
    if args.iter().any(|arg| arg == "--tags") {
        return false;
    }
    current_branch(cwd).is_some_and(|current| branches.contains(&current))
}

fn current_branch(cwd: Option<&Path>) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd?)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn command_start(words: &[String]) -> usize {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if word.contains('=') {
            index += 1;
            continue;
        }
        let program = Path::new(word)
            .file_name()
            .and_then(|word| word.to_str())
            .unwrap_or_default();
        if !matches!(program, "command" | "exec" | "env" | "sudo") {
            break;
        }
        index += 1;
        while let Some(option) = words.get(index).filter(|word| word.starts_with('-')) {
            index += if matches!(
                option.as_str(),
                "-u" | "--unset" | "-C" | "--chdir" | "-g" | "--user" | "--group"
            ) {
                2
            } else {
                1
            };
            if option == "--" {
                break;
            }
        }
    }
    index
}

fn subcommand<'a>(args: &'a [String], value_options: &[&str]) -> Option<&'a str> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if !arg.starts_with('-') {
            return Some(arg);
        }
        index += if value_options.contains(&arg.as_str()) {
            2
        } else {
            1
        };
    }
    None
}

/// Separates shell commands without interpreting quoted prose as executable text.
fn commands(input: &str, depth: u8) -> Vec<Vec<String>> {
    if depth > 8 {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut characters = input.chars().peekable();
    while let Some(character) = characters.next() {
        if escaped {
            word.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if quote != Some('\'')
            && (character == '`' || (character == '$' && characters.peek() == Some(&'(')))
        {
            if character == '$' {
                characters.next();
            }
            let script = substitution(&mut characters, character == '`');
            result.extend(commands(&script, depth + 1));
            continue;
        }
        if quote.is_none() && character == '#' && word.is_empty() {
            while characters
                .peek()
                .is_some_and(|character| *character != '\n')
            {
                characters.next();
            }
            continue;
        }
        if let Some(current) = quote {
            if character == current {
                quote = None;
            } else {
                word.push(character);
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            continue;
        }
        if character.is_whitespace() || matches!(character, ';' | '|' | '&' | '(' | ')') {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if (character == '\n' || matches!(character, ';' | '|' | '&' | '(' | ')'))
                && !words.is_empty()
            {
                result.push(std::mem::take(&mut words));
            }
        } else {
            word.push(character);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    if !words.is_empty() {
        result.push(words);
    }
    result
}

fn substitution(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    backtick: bool,
) -> String {
    let mut script = String::new();
    let mut nesting = 1;
    let mut quote = None;
    let mut escaped = false;
    for character in characters.by_ref() {
        if escaped {
            script.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            script.push(character);
            escaped = true;
            continue;
        }
        if backtick && character == '`' {
            break;
        }
        if let Some(current) = quote {
            if character == current {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if !backtick && character == '(' {
            nesting += 1;
        } else if !backtick && character == ')' {
            nesting -= 1;
            if nesting == 0 {
                break;
            }
        }
        script.push(character);
    }
    script
}
