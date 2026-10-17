use std::{path::Path, process::ExitCode};

use crate::Result;

pub async fn run(socket: &Path) -> Result<ExitCode> {
    if super::list::workspaces(socket).await?.is_empty() {
        super::new::run(socket, None, None, "claude".into(), false).await?;
        Ok(ExitCode::SUCCESS)
    } else {
        super::pick::run()
    }
}
