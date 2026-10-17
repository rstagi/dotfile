use std::path::Path;

use rmcp::{ServiceExt, transport::stdio};

use crate::{Result, mcp::Control};

pub async fn run(socket: &Path) -> Result<()> {
    let service = Control::from_env(socket)?.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

pub async fn print(socket: &Path, name: &str, arguments: serde_json::Value) -> Result<()> {
    let value = Control::from_env(socket)?.call(name, arguments).await?;
    println!("{value}");
    Ok(())
}
