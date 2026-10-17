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
