use std::{path::Path, process::Command};

pub struct SandboxRoot(tempfile::TempDir);

impl SandboxRoot {
    pub fn new() -> Self {
        Self(tempfile::tempdir_in("/tmp").unwrap())
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn tmux_socket(&self) -> String {
        self.path().join("tmux.sock").to_str().unwrap().to_owned()
    }
}

impl Drop for SandboxRoot {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .arg("-S")
            .arg(self.path().join("tmux.sock"))
            .arg("kill-server")
            .output();
    }
}
