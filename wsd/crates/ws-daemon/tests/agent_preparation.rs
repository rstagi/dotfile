use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, process::Command};
use ws_daemon::agents::{AgentSession, Mode, prepare};

#[test]
fn preparation_matches_zsh_launch_flags_and_hook_files_for_every_agent() {
    let home = tempfile::tempdir().unwrap();
    let context = AgentSession {
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        ws_home: home.path().join(".ws"),
        ws_bin: PathBuf::from("/test/ws-rs"),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    for agent in ["claude", "codex", "grok"] {
        let prepared = prepare(agent, Mode::Interactive, &context, Some("resume-123")).unwrap();
        assert_eq!(prepared.program, agent);
        assert_eq!(prepared.env["WS_SESSION"], context.id);
        assert_eq!(prepared.env["WS_SESSION_ID"], context.id);
        let script = r#"
            source "$REPO/ws/lib/tabs.sh"
            source "$REPO/ws/lib/hooks.sh"
            hooks_migrate() { :; }
            write_grok_hooks() { :; }
            WS_SESSIONS="$HOME/sessions"
            agent_command "$AGENT" resume-123 "" "$SID"
        "#;
        let output = Command::new("zsh")
            .args(["-f", "-c", script])
            .env(
                "REPO",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."),
            )
            .env("HOME", home.path())
            .env("WS_HOME", &context.ws_home)
            .env("WS_BIN", &context.ws_bin)
            .env("AGENT", agent)
            .env("SID", &context.id)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let mut expected =
            shell_words::split(std::str::from_utf8(&output.stdout).unwrap().trim()).unwrap();
        assert_eq!(expected.remove(0), agent);
        let mut actual = prepared.argv.clone();
        if agent == "claude" {
            let i = actual.iter().position(|v| v == "--settings").unwrap() + 1;
            let expected_i = expected.iter().position(|v| v == "--settings").unwrap() + 1;
            actual[i] = expected[expected_i].clone();
            let legacy: Value = serde_json::from_slice(
                &std::fs::read(context.ws_home.join("claude-settings.json")).unwrap(),
            )
            .unwrap();
            let generated: Value = serde_json::from_slice(&prepared.files[0].contents).unwrap();
            assert_eq!(generated, legacy);
        }
        assert_eq!(actual, expected, "{agent}");
        assert_eq!(
            prepared.env,
            BTreeMap::from([
                ("WS_SESSION".into(), context.id.clone()),
                ("WS_SESSION_ID".into(), context.id.clone()),
            ])
        );
        assert!(!context.home.join(".grok/hooks/ws.json").exists());
    }
}

#[test]
fn headless_prompt_precedes_variadic_flags_and_generated_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let session = AgentSession {
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        ws_home: home.path().join(".ws"),
        ws_bin: PathBuf::from("/test/ws rs'bin"),
        spawned: false,
        prompt: Some("Return exactly READY".into()),
        arguments: vec!["--allowedTools".into(), "Read".into(), "Bash".into()],
    };
    let prepared = prepare("claude", Mode::Headless, &session, None).unwrap();
    assert_eq!(&prepared.argv[..2], ["-p", "Return exactly READY"]);
    assert_eq!(
        &prepared.argv[prepared.argv.len() - 3..],
        ["--allowedTools", "Read", "Bash"]
    );
    let second = prepare("claude", Mode::Interactive, &session, None).unwrap();
    assert_ne!(prepared.files[0].path, second.files[0].path);
    prepared.write_files().unwrap();
    assert_eq!(
        std::fs::metadata(&prepared.files[0].path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let hooks: Value = serde_json::from_slice(&prepared.files[0].contents).unwrap();
    assert_eq!(
        shell_words::split(
            hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
        )
        .unwrap(),
        ["/test/ws rs'bin", "hook", "stop"]
    );
    let grok = prepare("grok", Mode::Interactive, &session, None).unwrap();
    grok.write_files().unwrap();
    assert!(session.home.join(".grok/hooks/ws.json").is_file());
}
