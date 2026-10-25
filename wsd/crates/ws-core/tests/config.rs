use std::{fs, io};
use ws_core::config::Config;

#[test]
fn missing_config_and_partial_limits_use_defaults() {
    let sandbox = tempfile::tempdir().unwrap();
    let defaults = Config::load(sandbox.path()).unwrap();
    assert_eq!(defaults.limits.max_children, 8);
    assert_eq!(defaults.limits.max_depth, 3);
    for (contents, children, depth) in [
        ("", 8, 3),
        ("[limits]\nmax_children = 2\n", 2, 3),
        ("[limits]\nmax_depth = 1\n", 8, 1),
    ] {
        fs::write(sandbox.path().join("config.toml"), contents).unwrap();
        let config = Config::load(sandbox.path()).unwrap();
        assert_eq!(config.limits.max_children, children);
        assert_eq!(config.limits.max_depth, depth);
    }
}

#[test]
fn invalid_limits_return_an_error_and_preserve_the_configuration() {
    let sandbox = tempfile::tempdir().unwrap();
    for contents in [
        "[limits]\nmax_children = -1\n",
        "[limits]\nmax_depth = false\n",
        "[limits]\nmax_dept = 2\n",
        "[limts]\nmax_children = 2\n",
        "[limits",
    ] {
        let path = sandbox.path().join("config.toml");
        fs::write(&path, contents).unwrap();
        let error = Config::load(sandbox.path()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("config.toml"));
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
    }
}

#[test]
fn agent_priority_defaults_to_background_and_rejects_unknown_values() {
    use ws_core::config::AgentPriority;
    let sandbox = tempfile::tempdir().unwrap();
    assert_eq!(
        Config::load(sandbox.path()).unwrap().agent.priority,
        AgentPriority::Background
    );
    for (contents, priority) in [
        (
            "[agent]\npriority = \"background\"\n",
            AgentPriority::Background,
        ),
        ("[agent]\npriority = \"normal\"\n", AgentPriority::Normal),
        ("[agent]\npriority = \"utility\"\n", AgentPriority::Utility),
    ] {
        fs::write(sandbox.path().join("config.toml"), contents).unwrap();
        assert_eq!(
            Config::load(sandbox.path()).unwrap().agent.priority,
            priority
        );
    }
    fs::write(
        sandbox.path().join("config.toml"),
        "[agent]\npriority = \"fast\"\n",
    )
    .unwrap();
    assert_eq!(
        Config::load(sandbox.path()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn priorities_cycle_from_background_through_utility_and_normal() {
    use ws_core::config::AgentPriority;
    assert_eq!(AgentPriority::Background.next(), AgentPriority::Utility);
    assert_eq!(AgentPriority::Utility.next(), AgentPriority::Normal);
    assert_eq!(AgentPriority::Normal.next(), AgentPriority::Background);
}

#[test]
fn only_a_clamped_priority_adds_a_taskpolicy_prefix() {
    use ws_core::config::AgentPriority;
    assert!(AgentPriority::Normal.prefix().is_empty());
    if cfg!(target_os = "macos") {
        assert_eq!(
            AgentPriority::Utility.prefix(),
            ["/usr/sbin/taskpolicy", "-c", "utility"]
        );
        assert_eq!(
            AgentPriority::Background.prefix(),
            ["/usr/sbin/taskpolicy", "-c", "background"]
        );
    } else {
        assert!(AgentPriority::Utility.prefix().is_empty());
    }
}

#[test]
fn naming_defaults_to_haiku_through_claude() {
    let sandbox = tempfile::tempdir().unwrap();
    let naming = Config::load(sandbox.path()).unwrap().naming;
    assert_eq!(
        (naming.model.as_str(), naming.program.as_str()),
        ("haiku", "claude")
    );
    fs::write(
        sandbox.path().join("config.toml"),
        "[naming]\nmodel = \"off\"\n",
    )
    .unwrap();
    let naming = Config::load(sandbox.path()).unwrap().naming;
    assert_eq!(
        (naming.model.as_str(), naming.program.as_str()),
        ("off", "claude")
    );
}

#[test]
fn loop_spawn_cap_accepts_configuration() {
    let sandbox = tempfile::tempdir().unwrap();
    assert_eq!(Config::load(sandbox.path()).unwrap().r#loop.max_spawns, 500);
    let path = sandbox.path().join("config.toml");
    fs::write(&path, "[loop]\nmax_spawns = 12\n").unwrap();
    assert_eq!(Config::load(sandbox.path()).unwrap().r#loop.max_spawns, 12);
    for value in ["-1", "false"] {
        fs::write(&path, format!("[loop]\nmax_spawns = {value}\n")).unwrap();
        assert_eq!(
            Config::load(sandbox.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
