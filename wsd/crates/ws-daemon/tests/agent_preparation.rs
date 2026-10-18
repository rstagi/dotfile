use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, process::Command};
use ws_daemon::agents::{AgentSession, Mode, prepare};

#[test]
fn preparation_matches_zsh_launch_flags_and_hook_files_for_every_agent() {
    let home = tempfile::tempdir().unwrap();
    let context = AgentSession {
        agent_program: None,
        state: home.path().join("state"),
        parent_session_id: None,
        project: "sandbox-project".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: None,
        ws_home: home.path().join(".ws"),
        ws_bin: std::env::current_exe().unwrap(),
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
            .env("GROK_HOOKS", context.home.join(".grok/hooks/ws.json"))
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
            let index = actual.iter().position(|arg| arg == "--mcp-config").unwrap();
            actual.drain(index..index + 2);
        } else if agent == "codex" {
            let mut index = 0;
            while index + 1 < actual.len() {
                if actual[index] == "-c" && actual[index + 1].starts_with("mcp_servers.ws.") {
                    actual.drain(index..index + 2);
                } else {
                    index += 1;
                }
            }
        }
        if agent == "claude" {
            let i = actual.iter().position(|v| v == "--settings").unwrap() + 1;
            let expected_i = expected.iter().position(|v| v == "--settings").unwrap() + 1;
            actual[i] = expected[expected_i].clone();
            let legacy: Value = serde_json::from_slice(
                &std::fs::read(context.ws_home.join("claude-settings.json")).unwrap(),
            )
            .unwrap();
            let mut generated: Value = serde_json::from_slice(&prepared.files[0].contents).unwrap();
            let exit = generated["hooks"]
                .as_object_mut()
                .unwrap()
                .remove("SessionEnd")
                .expect("Claude must report session exit.");
            assert!(
                exit[0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap()
                    .ends_with(" hook exit")
            );
            assert_eq!(generated, legacy);
        }
        if agent == "grok" {
            let legacy: Value = serde_json::from_slice(
                &std::fs::read(context.home.join(".grok/hooks/ws.json")).unwrap(),
            )
            .unwrap();
            let generated: Value = serde_json::from_slice(&prepared.files[0].contents).unwrap();
            assert_eq!(generated, legacy);
            std::fs::remove_file(context.home.join(".grok/hooks/ws.json")).unwrap();
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
        agent_program: None,
        state: home.path().join("state"),
        parent_session_id: None,
        project: "sandbox-project".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: None,
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

#[test]
fn headless_resume_uses_each_agents_noninteractive_mode() {
    let home = tempfile::tempdir().unwrap();
    let context = AgentSession {
        agent_program: None,
        state: home.path().join("state"),
        parent_session_id: None,
        project: "sandbox-project".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: None,
        ws_home: home.path().join(".ws"),
        ws_bin: PathBuf::from("/test/ws-rs"),
        spawned: false,
        prompt: Some("Return exactly READY\nThen stop.".into()),
        arguments: vec![],
    };
    let codex = prepare("codex", Mode::Headless, &context, Some("resume-123")).unwrap();
    assert_eq!(&codex.argv[..3], ["exec", "resume", "resume-123"]);
    let grok = prepare("grok", Mode::Headless, &context, Some("resume-123")).unwrap();
    let index = grok.argv.iter().position(|v| v == "--prompt-file").unwrap();
    let prompt = grok
        .files
        .iter()
        .find(|file| file.path.to_str() == Some(&grok.argv[index + 1]))
        .unwrap();
    assert_eq!(prompt.contents, b"Return exactly READY\nThen stop.");
    assert!(
        !grok
            .argv
            .iter()
            .any(|arg| arg == context.prompt.as_ref().unwrap())
    );
    assert!(
        grok.argv
            .windows(2)
            .any(|pair| pair == ["--resume", "resume-123"])
    );
}

#[test]
fn grok_hooks_follow_the_configured_home_inside_the_sandbox() {
    let home = tempfile::tempdir().unwrap();
    let context = AgentSession {
        agent_program: None,
        state: home.path().join("state"),
        parent_session_id: None,
        project: "sandbox-project".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: Some(home.path().join("custom-grok")),
        ws_home: home.path().join(".ws"),
        ws_bin: PathBuf::from("/test/ws-rs"),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    let prepared = prepare("grok", Mode::Interactive, &context, None).unwrap();
    prepared.write_files().unwrap();
    assert!(home.path().join("custom-grok/hooks/ws.json").is_file());
    assert!(!home.path().join(".grok/hooks/ws.json").exists());
}

#[test]
fn mcp_registration_extends_each_launch_mode_and_preserves_user_configuration() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".codex")).unwrap();
    let claude_config = home.path().join(".claude.json");
    let codex_config = home.path().join(".codex/config.toml");
    let claude_contents = r#"{"mcpServers":{"own":{"command":"own-server"}}}"#;
    let codex_contents = "[mcp_servers.own]\ncommand = 'own-server'\n";
    std::fs::write(&claude_config, claude_contents).unwrap();
    std::fs::write(&codex_config, codex_contents).unwrap();
    let session = AgentSession {
        agent_program: None,
        state: home.path().join("state"),
        parent_session_id: None,
        project: "sandbox-project".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: None,
        ws_home: home.path().join(".ws"),
        ws_bin: PathBuf::from("/test/ws rs'bin"),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    for mode in [Mode::Interactive, Mode::Headless] {
        for resume in [None, Some("resume-123")] {
            for agent in ["claude", "codex"] {
                let prepared = prepare(agent, mode, &session, resume).unwrap();
                prepared.write_files().unwrap();
                if agent == "claude" {
                    let flag = prepared
                        .argv
                        .iter()
                        .position(|arg| arg == "--mcp-config")
                        .expect("Claude must receive the ws MCP configuration.");
                    let config: Value =
                        serde_json::from_slice(&std::fs::read(&prepared.argv[flag + 1]).unwrap())
                            .unwrap();
                    assert_eq!(config["mcpServers"]["ws"]["command"], "/test/ws rs'bin");
                    assert_eq!(
                        config["mcpServers"]["ws"]["args"],
                        serde_json::json!(["mcp"])
                    );
                    assert_eq!(
                        config["mcpServers"]["ws"]["env"]["WS_SESSION_ID"],
                        session.id
                    );
                    assert_eq!(config["mcpServers"].as_object().unwrap().len(), 1);
                    assert!(!prepared.argv.iter().any(|arg| arg == "--strict-mcp-config"));
                } else {
                    for setting in [
                        "mcp_servers.ws.command=\"/test/ws rs'bin\"".to_owned(),
                        "mcp_servers.ws.args=[\"mcp\"]".to_owned(),
                        format!("mcp_servers.ws.env.WS_SESSION_ID=\"{}\"", session.id),
                        "mcp_servers.ws.env_vars=[\"WS_SOCK\"]".to_owned(),
                    ] {
                        assert!(
                            prepared
                                .argv
                                .windows(2)
                                .any(|pair| pair == ["-c", &setting]),
                            "{setting}"
                        );
                    }
                    assert!(
                        !prepared
                            .argv
                            .iter()
                            .any(|arg| arg.starts_with("mcp_servers="))
                    );
                }
                assert_eq!(
                    std::fs::read_to_string(&claude_config).unwrap(),
                    claude_contents
                );
                assert_eq!(
                    std::fs::read_to_string(&codex_config).unwrap(),
                    codex_contents
                );
            }
        }
    }
}

#[test]
fn telemetry_contract_covers_both_agents_in_fresh_and_resumed_launch_modes() {
    use ws_core::{device::Device, semconv};
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let device = Device::init(&state, "sandbox").unwrap();
    std::fs::write(state.join("otlp.endpoint"), "http://127.0.0.1:43123\n").unwrap();
    let parent = "01K7Q0K5M00000000000000002";
    let context = AgentSession {
        agent_program: Some("/usr/bin/true".into()),
        state,
        parent_session_id: Some(parent.parse().unwrap()),
        project: "sandbox-project".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: None,
        ws_home: home.path().join(".ws"),
        ws_bin: PathBuf::from("/test/ws"),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    for mode in [Mode::Interactive, Mode::Headless] {
        for resume in [None, Some("resume-123")] {
            for agent in ["claude", "codex"] {
                let prepared = prepare(agent, mode, &context, resume).unwrap();
                let resources: BTreeMap<_, _> = prepared.env["OTEL_RESOURCE_ATTRIBUTES"]
                    .split(',')
                    .map(|part| part.split_once('=').unwrap())
                    .collect();
                assert_eq!(resources[semconv::SESSION_ID], context.id);
                assert_eq!(resources[semconv::PARENT_SESSION_ID], parent);
                assert_eq!(resources[semconv::DEVICE_ID], device.device_id.to_string());
                assert_eq!(resources[semconv::PROJECT], "sandbox-project");
                for key in [
                    "TRACEPARENT",
                    "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA",
                    "ENABLE_BETA_TRACING_DETAILED",
                    "BETA_TRACING_ENDPOINT",
                    "OTEL_LOG_USER_PROMPTS",
                    "OTEL_LOG_TOOL_DETAILS",
                    "OTEL_LOG_TOOL_CONTENT",
                ] {
                    assert!(!prepared.env.contains_key(key), "{agent}: {key}");
                }
                if agent == "claude" {
                    assert_eq!(prepared.env["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
                    for key in [
                        "OTEL_METRICS_EXPORTER",
                        "OTEL_LOGS_EXPORTER",
                        "OTEL_TRACES_EXPORTER",
                    ] {
                        assert_eq!(prepared.env[key], "otlp");
                    }
                    assert_eq!(
                        prepared.env["OTEL_EXPORTER_OTLP_ENDPOINT"],
                        "http://127.0.0.1:43123"
                    );
                    assert_eq!(prepared.env["OTEL_EXPORTER_OTLP_PROTOCOL"], "http/json");
                } else {
                    for setting in [
                        "analytics.enabled=true",
                        "otel.log_user_prompt=false",
                        "otel.exporter={otlp-http={endpoint=\"http://127.0.0.1:43123/v1/logs\",protocol=\"json\"}}",
                        "otel.metrics_exporter={otlp-http={endpoint=\"http://127.0.0.1:43123/v1/metrics\",protocol=\"json\"}}",
                        "otel.trace_exporter=\"none\"",
                    ] {
                        assert!(
                            prepared.argv.windows(2).any(|p| p == ["-c", setting]),
                            "{setting}"
                        );
                    }
                    assert!(prepared.files.is_empty());
                }
            }
        }
    }
}

#[test]
fn native_tracing_requires_explicit_config_and_uses_the_session_trace() {
    use ws_core::device::Device;
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    Device::init(&state, "sandbox").unwrap();
    std::fs::write(state.join("otlp.endpoint"), "http://127.0.0.1:43123\n").unwrap();
    std::fs::write(
        state.join("config.toml"),
        "[telemetry]\nnative_tracing = true\n",
    )
    .unwrap();
    let context = AgentSession {
        agent_program: Some("/usr/bin/true".into()),
        state,
        parent_session_id: Some("01K7Q0K5M00000000000000002".parse().unwrap()),
        project: "sandbox".into(),
        id: "01K7Q0K5M00000000000000001".into(),
        home: home.path().into(),
        grok_home: None,
        ws_home: home.path().join(".ws"),
        ws_bin: "/test/ws".into(),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    for mode in [Mode::Interactive, Mode::Headless] {
        for resume in [None, Some("resume-123")] {
            for agent in ["claude", "codex"] {
                let prepared = prepare(agent, mode, &context, resume).unwrap();
                let traceparent: Vec<_> = prepared.env["TRACEPARENT"].split('-').collect();
                assert_eq!(traceparent[0], "00");
                assert_eq!(traceparent[1], "0199ee09968000000000000000000001");
                assert_eq!(traceparent[2].len(), 16);
                assert_ne!(traceparent[2], "0000000000000000");
                assert_eq!(traceparent[3], "01");
                if agent == "claude" {
                    assert_eq!(prepared.env["CLAUDE_CODE_ENHANCED_TELEMETRY_BETA"], "1");
                    assert_eq!(prepared.env["ENABLE_BETA_TRACING_DETAILED"], "1");
                    assert_eq!(
                        prepared.env["BETA_TRACING_ENDPOINT"],
                        "http://127.0.0.1:43123"
                    );
                } else {
                    assert!(prepared.argv.windows(2).any(|p| p == ["-c", "otel.trace_exporter={otlp-http={endpoint=\"http://127.0.0.1:43123/v1/traces\",protocol=\"json\"}}"]));
                }
            }
        }
    }
}

#[test]
fn setting_names_and_versions_match_the_captured_s2_contracts() {
    use std::os::unix::fs::PermissionsExt;
    use ws_core::device::Device;
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    Device::init(&state, "sandbox").unwrap();
    std::fs::write(state.join("otlp.endpoint"), "http://127.0.0.1:43123\n").unwrap();
    std::fs::write(
        state.join("config.toml"),
        "[telemetry]\nnative_tracing = true\n",
    )
    .unwrap();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spikes/s2");
    for (agent, manifest) in [
        ("claude", "manifest.json"),
        ("codex", "codex-manifest.json"),
    ] {
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(fixtures.join(manifest)).unwrap()).unwrap();
        let version = manifest["version"].as_str().unwrap();
        let program = home.path().join(agent);
        std::fs::write(
            &program,
            format!("#!/bin/sh\nprintf '{agent} {version}\\n'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let context = AgentSession {
            agent_program: Some(program),
            state: state.clone(),
            parent_session_id: None,
            project: "sandbox".into(),
            id: "01K7Q0K5M00000000000000001".into(),
            home: home.path().into(),
            grok_home: None,
            ws_home: home.path().join(".ws"),
            ws_bin: "/test/ws".into(),
            spawned: false,
            prompt: None,
            arguments: vec![],
        };
        let prepared = prepare(agent, Mode::Headless, &context, None).unwrap();
        let metadata = prepared.telemetry.as_ref().unwrap();
        assert_eq!(
            metadata.settings_version, version,
            "New versions need S2 contract coverage."
        );
        assert!(metadata.version_covered);
        if agent == "claude" {
            let captured = manifest["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| run["label"] == "detailed-json")
                .unwrap();
            for (key, value) in &prepared.env {
                if key.starts_with("WS_") {
                    continue;
                }
                assert!(
                    captured["settings"].get(key).is_some(),
                    "The S2 sample does not cover {key}."
                );
                if key == "OTEL_RESOURCE_ATTRIBUTES" || key == "TRACEPARENT" {
                    continue;
                }
                assert_eq!(
                    value.replace(":43123", ":<port>"),
                    captured["settings"][key].as_str().unwrap()
                );
            }
        } else {
            let captured = manifest["runs"][0]["otel_flags"].as_array().unwrap();
            let settings: Vec<_> = prepared
                .argv
                .windows(2)
                .filter(|p| {
                    p[0] == "-c" && (p[1].starts_with("otel.") || p[1].starts_with("analytics."))
                })
                .map(|p| p[1].replace(":43123", ":<port>"))
                .collect();
            for setting in &settings {
                assert!(
                    captured.iter().any(|value| value == setting),
                    "The S2 sample does not cover {setting}."
                );
            }
            for setting in captured {
                if setting.as_str().unwrap().starts_with("otel.environment=") {
                    continue;
                }
                assert!(
                    settings.iter().any(|value| setting == value),
                    "Missing captured setting: {setting}"
                );
            }
        }
    }
}
