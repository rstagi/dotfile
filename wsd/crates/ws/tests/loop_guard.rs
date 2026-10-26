#[allow(dead_code)]
mod common;

use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
use ws_daemon::agents::Prepared;

struct Sandbox {
    _daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
}

#[test]
fn preparation_guards_loop_descendants_and_removes_inherited_loop_identity_elsewhere() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    for agent in ["claude", "codex", "grok"] {
        let parent = sandbox.session(Some(&owner));
        let child = sandbox.session(Some(&parent));
        let plain = sandbox.session(None);
        for id in [&parent, &child] {
            let prepared = sandbox.prepare(id, agent);
            assert_eq!(prepared.env.get("WS_LOOP_ID"), Some(&owner));
            let all = format!(
                "{:?} {:?}",
                prepared.argv,
                prepared
                    .files
                    .iter()
                    .map(|file| String::from_utf8_lossy(&file.contents))
                    .collect::<Vec<_>>()
            );
            assert!(all.contains("hook guard"), "{agent}: {all}");
            if agent != "codex" {
                let file = prepared
                    .files
                    .iter()
                    .find(|file| {
                        serde_json::from_slice::<Value>(&file.contents)
                            .ok()
                            .is_some_and(|v| v["hooks"]["PreToolUse"].is_array())
                    })
                    .unwrap();
                let settings: Value = serde_json::from_slice(&file.contents).unwrap();
                let pre = settings["hooks"]["PreToolUse"].to_string();
                assert!(pre.contains("hook working"));
                assert!(pre.contains("hook guard"));
            }
        }
        let prepared = sandbox.prepare(&plain, agent);
        assert!(!prepared.env.contains_key("WS_LOOP_ID"));
        assert!(prepared.unset_env.iter().any(|key| key == "WS_LOOP_ID"));
        assert!(!format!("{:?}", prepared.argv).contains("hook guard"));
        assert!(
            prepared
                .files
                .iter()
                .all(|file| !String::from_utf8_lossy(&file.contents).contains("hook guard"))
        );
    }
}

#[test]
fn guard_denies_recorded_tool_bypasses_and_records_daemon_evidence() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    let parent = sandbox.session(Some(&owner));
    let child = sandbox.session(Some(&parent));
    sandbox.post(
        &format!("/v1/sessions/{owner}/events"),
        json!({
            "type":"loop.plan", "source":"tool",
            "body":{"text":"# Plan\n- **Integration branch:** `feat/integration`\n", "sha":"test"}
        }),
    );
    let cases = [
        ("git merge lane", "no-merge", "loop-merge.sh"),
        (
            "git -C /tmp merge --no-ff lane",
            "no-merge",
            "loop-merge.sh",
        ),
        ("true && git merge lane", "no-merge", "loop-merge.sh"),
        (
            "git push origin feat/integration",
            "no-push",
            "loop-merge.sh",
        ),
        (
            "git push origin HEAD:refs/heads/feat/integration",
            "no-push",
            "loop-merge.sh",
        ),
        ("gh pr merge 23 --squash", "no-pr-merge", "loop-merge.sh"),
        ("claude -p 'Do work.'", "no-agent-spawn", "phase_spawn"),
        ("codex exec 'Do work.'", "no-agent-spawn", "phase_spawn"),
        (
            "grok --prompt 'Do work.'",
            "no-agent-spawn",
            "ws session create",
        ),
    ];
    for (command, rule, tool) in cases {
        let reply = sandbox.guard(&child, "claude", command);
        assert_eq!(
            reply["hookSpecificOutput"]["permissionDecision"], "deny",
            "{command}: {reply}"
        );
        assert!(
            reply["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains(tool)
        );
        let events = sandbox.guards(&owner);
        let last = events.last().unwrap();
        assert_eq!(last["body"]["source"], "daemon");
        assert_eq!(last["body"]["session"], child);
        assert_eq!(last["body"]["rule"], rule);
        assert_eq!(last["body"]["command"], command);
    }
    assert_eq!(sandbox.guards(&owner).len(), cases.len());
    for command in [
        "git status",
        "git push origin lane",
        "echo 'git merge lane'",
        "loop-merge.sh --lane-branch lane",
        "ws session create --headless -- codex exec task",
    ] {
        let reply = sandbox.guard(&child, "claude", command);
        assert!(
            reply["hookSpecificOutput"]["permissionDecision"].is_null(),
            "{command}: {reply}"
        );
    }
    assert_eq!(sandbox.guards(&owner).len(), cases.len());
}

#[test]
fn guard_supports_fix_pushes_observation_only_agents_and_unrelated_sessions() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    sandbox.post(&format!("/v1/sessions/{owner}/events"), json!({
        "type":"loop.plan", "source":"tool", "body":{"text":"- **Integration branch:** `feat/integration`", "sha":"test"}
    }));
    let fix = sandbox.post("/v1/sessions", json!({"headless":true, "command":["/bin/true"], "cwd":sandbox.home.path(),
        "loop":{"loop":owner, "phase":2, "attempt":1, "repo":"owner/repo", "role":"fix", "provider":"claude", "round":1, "inner":1}
    }))["id"].as_str().unwrap().to_owned();
    assert_eq!(
        sandbox.guard(&fix, "claude", "git push origin HEAD:feat/integration"),
        json!({})
    );
    assert!(sandbox.guards(&owner).is_empty());
    let reply = sandbox.guard(&fix, "claude", "git merge lane");
    assert_eq!(reply["hookSpecificOutput"]["permissionDecision"], "deny");
    let worker = sandbox.session(Some(&owner));
    assert_eq!(sandbox.guard(&worker, "grok", "git merge lane"), json!({}));
    assert_eq!(sandbox.guards(&owner).len(), 2);
    let reply = sandbox.guard(&worker, "codex", "git merge lane");
    assert_eq!(reply["hookSpecificOutput"]["permissionDecision"], "deny");
    let outside = sandbox.session(None);
    assert_eq!(
        sandbox.guard(&outside, "claude", "git merge lane"),
        json!({})
    );
    assert_eq!(sandbox.guards(&owner).len(), 3);
}

#[test]
fn guard_recognizes_implicit_push_targets_and_shell_wrappers() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    let child = sandbox.session(Some(&owner));
    sandbox.post(&format!("/v1/sessions/{owner}/events"), json!({
        "type":"loop.plan", "source":"tool", "body":{"text":"- **Integration branch:** `feat/integration`", "sha":"test"}
    }));
    let output = Command::new("git")
        .current_dir(sandbox.home.path())
        .args(["init", "--initial-branch", "feat/integration"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    for command in [
        "git push origin HEAD",
        "git push origin",
        "git push",
        "env -u UNUSED git merge lane",
        "command -- git merge lane",
        "bash -lc 'git merge lane'",
        "codex -c model=example exec task",
    ] {
        let reply = sandbox.guard(&child, "claude", command);
        assert_eq!(
            reply["hookSpecificOutput"]["permissionDecision"], "deny",
            "{command}: {reply}"
        );
    }
    for command in [
        "git push origin HEAD:lane",
        "git push origin --tags",
        "printf '%s' 'codex exec task'",
        "git log --format='git merge lane'",
    ] {
        assert_eq!(
            sandbox.guard(&child, "claude", command),
            json!({}),
            "{command}"
        );
    }
}

#[test]
fn guard_checks_shell_substitutions_but_ignores_literal_text_and_other_tools() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    let child = sandbox.session(Some(&owner));
    for command in [
        r#"echo "$(git merge lane)""#,
        "echo `codex exec task`",
        "printf x; /usr/bin/claude -p task",
    ] {
        let reply = sandbox.guard(&child, "claude", command);
        assert_eq!(
            reply["hookSpecificOutput"]["permissionDecision"], "deny",
            "{command}: {reply}"
        );
    }
    for command in [
        "echo '$(git merge lane)'",
        "echo '`codex exec task`'",
        "echo ok # git merge lane",
    ] {
        assert_eq!(
            sandbox.guard(&child, "claude", command),
            json!({}),
            "{command}"
        );
    }
    for payload in [
        json!({"tool_name":"Read", "tool_input":{"command":"git merge lane"}}),
        json!({"tool_name":"Bash", "tool_input":{}}),
        json!({"tool_name":"Bash", "tool_input":{"command":null}}),
        json!({"raw":"invalid"}),
    ] {
        assert_eq!(sandbox.guard_payload(&child, "claude", payload), json!({}));
    }
    assert_eq!(sandbox.guards(&owner).len(), 3);
    let reply = sandbox.guard_payload(
        &child,
        "codex",
        json!({
            "tool_name":"functions.exec_command", "tool_input":{"cmd":"git merge lane"}
        }),
    );
    assert_eq!(reply["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn preparation_reads_parent_changes_and_does_not_require_a_live_daemon() {
    let mut sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    let child = sandbox.session(Some(&owner));
    let grandchild = sandbox.session(Some(&child));
    sandbox.post(
        &format!("/v1/sessions/{child}/parent"),
        json!({"parent":null}),
    );
    assert!(
        !sandbox
            .prepare(&grandchild, "claude")
            .env
            .contains_key("WS_LOOP_ID")
    );
    sandbox.post(
        &format!("/v1/sessions/{child}/parent"),
        json!({"parent":owner}),
    );
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    assert_eq!(
        sandbox.prepare(&grandchild, "claude").env["WS_LOOP_ID"],
        owner
    );
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let mut command = common::ws(home.path(), &socket);
        command
            .env_remove("WS_LOOP_ID")
            .env_remove("WS_LOOP_GUARD_AGENT");
        let daemon = common::Daemon::start(command, &socket);
        Self {
            _daemon: daemon,
            home,
            socket,
        }
    }

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env_remove("WS_LOOP_ID")
            .env_remove("WS_LOOP_GUARD_AGENT");
        command
    }

    fn post(&self, path: &str, body: Value) -> Value {
        let response = common::request(&self.socket, "POST", path, &body.to_string());
        assert!(response.starts_with("HTTP/1.1 20"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn loop_session(&self) -> String {
        let directory = self.home.path().join("loop");
        fs::create_dir(&directory).unwrap();
        self.post(
            "/v1/loops/create",
            json!({"loop_dir":directory,"plan_id":format!("fixture:{}", directory.display())}),
        )["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn session(&self, parent: Option<&str>) -> String {
        self.post("/v1/sessions", json!({"parent_session_id":parent}))["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn guards(&self, owner: &str) -> Vec<Value> {
        let response = common::get(
            &self.socket,
            &format!("/v1/sessions/{owner}/log?type=loop.guard"),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let body: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        body["entries"].as_array().unwrap().clone()
    }

    fn guard(&self, id: &str, agent: &str, command: &str) -> Value {
        self.guard_payload(id, agent, json!({"hook_event_name":"PreToolUse", "tool_name":"Bash", "cwd":self.home.path(), "tool_input":{"command":command}}))
    }

    fn guard_payload(&self, id: &str, agent: &str, payload: Value) -> Value {
        let mut process = self
            .command()
            .env("WS_SESSION_ID", id)
            .env("WS_LOOP_GUARD_AGENT", agent)
            .args(["hook", "guard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = process.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        if output.stdout.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&output.stdout).unwrap()
        }
    }

    fn prepare(&self, id: &str, agent: &str) -> Prepared {
        let output = self
            .command()
            .env("WS_LOOP_ID", "inherited-wrong-loop")
            .args([
                "agent",
                "prepare",
                "--agent",
                agent,
                "--mode",
                "headless",
                "--session",
                id,
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
