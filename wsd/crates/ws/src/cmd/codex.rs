use std::{path::Path, process::ExitCode};

pub async fn run(socket: &Path, arguments: Vec<String>) -> crate::Result<ExitCode> {
    super::claude::run(socket, "codex", arguments).await
}
