use std::{path::Path, process::ExitCode};

use crate::Result;

pub async fn run(socket: &Path) -> Result<ExitCode> {
    if super::list::workspaces(socket).await?.is_empty() {
        super::new::run(
            socket,
            super::new::Options {
                repo: None,
                branch: None,
                scratch: false,
                agent: "claude".into(),
                detach: false,
            },
        )
        .await?;
        Ok(ExitCode::SUCCESS)
    } else {
        super::pick::run()
    }
}
