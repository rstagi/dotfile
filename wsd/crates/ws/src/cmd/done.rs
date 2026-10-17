use std::{
    io::{self, Read},
    path::Path,
};

use serde_json::json;

use crate::{Result, client};

pub async fn run(socket: &Path, output: Option<&str>) -> Result<()> {
    let session = super::whoami::current(socket).await?;
    let output = match output {
        None => String::new(),
        Some("-") => {
            let mut text = String::new();
            io::stdin().read_to_string(&mut text)?;
            text
        }
        Some(path) => std::fs::read_to_string(path)
            .map_err(|_| format!("done: cannot read output file: {path}"))?,
    };
    client::request(
        socket,
        hyper::Method::POST,
        &format!("/v1/sessions/{}/done", session.id),
        serde_json::to_vec(&json!({"output":output, "parent_session_id":session.id}))?,
    )
    .await?;
    Ok(())
}
