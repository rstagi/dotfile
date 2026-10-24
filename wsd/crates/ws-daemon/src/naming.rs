//! Automatic session names from the first user prompt.
//!
//! The daemon sets a slug of the first prompt at once. A small model then proposes a better
//! name in the background. The model name replaces the slug only while the slug stays.

use crate::{
    api::{self, Store},
    hooks::Callback,
    sessions::{Error, Result, Sessions},
};
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;
use ws_core::ids::SessionId;

const MAX_WORDS: usize = 5;
const MAX_LENGTH: usize = 40;
const MODEL_DEADLINE: Duration = Duration::from_secs(20);
// The model reads only the start of a long prompt.
const MODEL_INPUT_LIMIT: usize = 2000;
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from", "i",
    "if", "in", "into", "is", "it", "me", "my", "of", "on", "or", "please", "so", "that", "the",
    "then", "this", "to", "we", "with", "you",
];
const MODEL_PROMPT: &str = "Write a name for a work session. \
Use 2 to 5 English words in lowercase kebab case. Write only the name.\n\n\
The first request of the session:\n";
// The model process gets only these variables. Claude Code needs them to find its login.
const MODEL_ENV: &[&str] = &[
    "HOME", "PATH", "USER", "LOGNAME", "TMPDIR", "LANG", "CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY",
];

/// The automatic name of a session and the model step that can improve it.
pub(crate) struct AutoName {
    id: SessionId,
    slug: String,
    prompt: String,
    model: String,
    program: String,
}

/// Returns a kebab-case name from text, or `None` when the text has no usable word.
pub fn slug(text: &str) -> Option<String> {
    if text.trim_start().starts_with('/') {
        return None;
    }
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(|word| {
            word.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .skip_while(|word| !word.starts_with(|c: char| c.is_ascii_alphabetic()))
        .collect();
    let content: Vec<&String> = words
        .iter()
        .filter(|word| !STOP_WORDS.contains(&word.as_str()))
        .collect();
    let words: Vec<&String> = if content.is_empty() {
        words.iter().collect()
    } else {
        content
    };
    let mut name = String::new();
    for word in words.into_iter().take(MAX_WORDS) {
        let length = name.len() + usize::from(!name.is_empty()) + word.len();
        if length > MAX_LENGTH {
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(word);
    }
    (!name.is_empty()).then_some(name)
}

/// Returns the text of a main-agent prompt callback.
pub(crate) fn first_prompt(callback: &Callback) -> Option<String> {
    let payload = &callback.payload;
    let event = payload["hook_event_name"]
        .as_str()
        .or_else(|| payload["hookEventName"].as_str())?;
    if !matches!(event, "UserPromptSubmit" | "user_prompt_submit")
        || callback.state != "working"
        || payload["agent_id"].as_str().is_some_and(|id| !id.is_empty())
    {
        return None;
    }
    payload["prompt"].as_str().map(str::to_owned)
}

/// Starts the model step in the background. Errors never reach the hook caller.
pub(crate) fn improve(store: Store, name: AutoName) {
    // Unit tests set an explicit program. They never start the Claude CLI from PATH.
    if name.model == "off" || (cfg!(test) && name.program == "claude") {
        return;
    }
    tokio::spawn(async move {
        let proposal = match propose(&name).await {
            Ok(proposal) => proposal,
            Err(error) => {
                eprintln!("Cannot get a session name from the model: {error}");
                return;
            }
        };
        let Some(proposal) = proposal.as_deref().and_then(slug) else {
            return;
        };
        let result = api::call(store, move |sessions| {
            sessions.replace_auto_name(name.id, &name.slug, &proposal)
        })
        .await;
        if let Err(error) = result {
            eprintln!("Cannot apply the model session name: {}", error.0);
        }
    });
}

impl Sessions {
    /// Names an unnamed session from its first prompt. Returns `None` when no name was set.
    pub(crate) fn auto_name(&mut self, id: SessionId, prompt: &str) -> Result<Option<AutoName>> {
        let session = self.get_record(&id.to_string())?;
        if session.name.is_some() || session.body["kind"] == "headless" {
            return Ok(None);
        }
        let Some(slug) = slug(prompt) else {
            return Ok(None);
        };
        let Some(name) = self.rename_unique(id, &slug)? else {
            return Ok(None);
        };
        Ok(Some(AutoName {
            id,
            slug: name,
            prompt: prompt.chars().take(MODEL_INPUT_LIMIT).collect(),
            model: self.config.naming.model.clone(),
            program: self.config.naming.program.clone(),
        }))
    }

    fn replace_auto_name(&mut self, id: SessionId, slug: &str, proposal: &str) -> Result<()> {
        let session = self.get_record(&id.to_string())?;
        if session.name.as_deref() == Some(slug) && proposal != slug {
            self.rename_unique(id, proposal)?;
        }
        Ok(())
    }

    /// Sets `name`, or `name-2` to `name-9` when another session has the name.
    fn rename_unique(&mut self, id: SessionId, name: &str) -> Result<Option<String>> {
        for suffix in 1..10 {
            let candidate = if suffix == 1 {
                name.to_owned()
            } else {
                format!("{name}-{suffix}")
            };
            match self.rename(&id.to_string(), &candidate, None) {
                Ok(_) => return Ok(Some(candidate)),
                Err(Error::NameTaken(_)) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }
}

async fn propose(name: &AutoName) -> std::io::Result<Option<String>> {
    let mut command = tokio::process::Command::new(&name.program);
    command
        .args(["-p", "--model", &name.model])
        // A project-only setting source skips user hooks and plugins. The empty directory
        // has no project settings.
        .args(["--setting-sources", "project", "--strict-mcp-config"])
        .args(["--tools", "", "--no-session-persistence"])
        .current_dir(std::env::temp_dir())
        .env_clear()
        .envs(MODEL_ENV.iter().filter_map(|key| Some((key, std::env::var_os(key)?))))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = format!("{MODEL_PROMPT}{}", name.prompt);
    let output = tokio::time::timeout(MODEL_DEADLINE, async move {
        stdin.write_all(input.as_bytes()).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .map_err(|_| std::io::Error::other("The model did not answer before the deadline."))??;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "The model exited with {}.",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::slug;
    use crate::sessions::Sessions;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::Path,
        time::{Duration, Instant},
    };
    use tower::ServiceExt;

    #[test]
    fn slug_keeps_five_ascii_words_in_kebab_case() {
        assert_eq!(
            slug("Fix the picker: scratch sessions show the FULL path!").as_deref(),
            Some("fix-picker-scratch-sessions-show")
        );
        assert_eq!(slug("  Résumé  café bug ").as_deref(), Some("rsum-caf-bug"));
        assert_eq!(slug("42 tests fail in ws").as_deref(), Some("tests-fail-ws"));
        assert_eq!(slug("the and of").as_deref(), Some("the-and-of"));
        assert_eq!(slug("日本語 🎉 ..."), None);
        assert_eq!(slug("/clear"), None);
        assert_eq!(slug(""), None);
    }

    #[test]
    fn slug_stops_at_forty_characters_on_a_word_boundary() {
        let name = slug("internationalization localization configuration verification").unwrap();
        assert!(name.len() <= 40, "{name}");
        assert_eq!(name, "internationalization-localization");
    }

    #[tokio::test]
    async fn the_first_prompt_names_the_session_and_the_model_improves_the_name() {
        let sandbox = tempfile::tempdir().unwrap();
        let fake = fake_model(sandbox.path(), "print -r -- '`better-picker-name`'");
        let app = app(sandbox.path(), "haiku", &fake);
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        prompt(&app, id, "Fix the picker path for scratch sessions", None).await;
        let named = get(&app, id).await;
        assert!(
            ["fix-picker-path-scratch-sessions", "better-picker-name"]
                .contains(&named["name"].as_str().unwrap()),
            "{named}"
        );
        assert_eq!(wait_name(&app, id, "better-picker-name").await, "better-picker-name");
        let args = fs::read_to_string(sandbox.path().join("model.args")).unwrap();
        assert!(args.contains("--model haiku"), "{args}");
        assert!(
            fs::read_to_string(sandbox.path().join("model.stdin"))
                .unwrap()
                .contains("Fix the picker path")
        );
        // A later prompt does not rename a named session.
        prompt(&app, id, "Now update the README", None).await;
        assert_eq!(get(&app, id).await["name"], "better-picker-name");
    }

    #[tokio::test]
    async fn model_output_applies_only_while_the_automatic_name_stays() {
        let sandbox = tempfile::tempdir().unwrap();
        let gate = sandbox.path().join("gate");
        let fake = fake_model(
            sandbox.path(),
            &format!(
                "while [ ! -e '{}' ]; do sleep 0.05; done; print -r -- late-model-name",
                gate.display()
            ),
        );
        let app = app(sandbox.path(), "haiku", &fake);
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        prompt(&app, id, "Deploy the docs site", None).await;
        assert_eq!(get(&app, id).await["name"], "deploy-docs-site");
        let (status, renamed) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/name"),
            json!({"name":"my-own-name"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{renamed}");
        fs::write(&gate, "").unwrap();
        wait_for_file(&sandbox.path().join("model.done")).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(get(&app, id).await["name"], "my-own-name");
    }

    #[tokio::test]
    async fn names_stay_unique_and_off_skips_the_model() {
        let sandbox = tempfile::tempdir().unwrap();
        let fake = fake_model(sandbox.path(), "print -r -- never-used");
        let app = app(sandbox.path(), "off", &fake);
        let (_, first) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, second) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (first, second) = (first["id"].as_str().unwrap(), second["id"].as_str().unwrap());
        prompt(&app, first, "Fix login bug", None).await;
        prompt(&app, second, "fix LOGIN bug", None).await;
        assert_eq!(get(&app, first).await["name"], "fix-login-bug");
        assert_eq!(get(&app, second).await["name"], "fix-login-bug-2");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!sandbox.path().join("model.args").exists());
    }

    #[tokio::test]
    async fn subagent_and_headless_prompts_do_not_name_a_session() {
        let sandbox = tempfile::tempdir().unwrap();
        let fake = fake_model(sandbox.path(), "print -r -- never-used");
        let app = app(sandbox.path(), "off", &fake);
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        prompt(&app, id, "Search the code base", Some("subagent-1")).await;
        assert!(get(&app, id).await["name"].is_null());
        let (status, headless) = request(
            &app,
            "POST",
            "/v1/sessions",
            json!({"command":["/bin/sh","-c","sleep 5"], "cwd": sandbox.path(), "headless": true}),
        )
        .await;
        assert!(status.is_success(), "{headless}");
        let id = headless["id"].as_str().unwrap();
        prompt(&app, id, "Implement phase three", None).await;
        assert!(get(&app, id).await["name"].is_null());
        request(&app, "POST", &format!("/v1/sessions/{id}/stop"), json!({})).await;
    }

    fn app(state: &Path, model: &str, program: &Path) -> Router {
        fs::write(
            state.join("config.toml"),
            format!(
                "[naming]\nmodel = \"{model}\"\nprogram = \"{}\"\n",
                program.display()
            ),
        )
        .unwrap();
        crate::api::router(Sessions::open(state, 1).unwrap())
    }

    fn fake_model(directory: &Path, body: &str) -> std::path::PathBuf {
        let path = directory.join("fake-claude");
        fs::write(
            &path,
            format!(
                "#!/bin/zsh\nprint -r -- \"$*\" > '{dir}/model.args'\ncat > '{dir}/model.stdin'\n{body}\n: > '{dir}/model.done'\n",
                dir = directory.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    async fn prompt(app: &Router, id: &str, text: &str, agent: Option<&str>) {
        let mut payload = json!({"hook_event_name":"UserPromptSubmit", "prompt": text});
        if let Some(agent) = agent {
            payload["agent_id"] = json!(agent);
        }
        let (status, body) = request(
            app,
            "POST",
            "/v1/hooks/ingest",
            json!({"session_id":id, "callback_id":ws_core::ids::Ulid::new(),
                "observed_at_us":crate::hooks::receipt_time_us().unwrap(),
                "state":"working", "pane":null, "payload":payload}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    async fn get(app: &Router, id: &str) -> Value {
        request(app, "GET", &format!("/v1/sessions/{id}"), Value::Null)
            .await
            .1
    }

    async fn wait_name(app: &Router, id: &str, name: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let current = get(app, id).await["name"].as_str().unwrap_or("").to_owned();
            if current == name || Instant::now() > deadline {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(Instant::now() < deadline, "{} did not appear", path.display());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}
