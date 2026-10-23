use std::path::Path;

use crate::{Result, client};

pub async fn run(socket: &Path, labels: &[String], json: bool) -> Result<()> {
    let [label] = labels else {
        return Err("name: expected <label>".into());
    };
    let current = super::whoami::current(socket).await?;
    let named = client::rename(socket, &current.id.to_string(), label).await?;
    if json {
        println!("{}", super::session_json(&named)?);
    }
    Ok(())
}
