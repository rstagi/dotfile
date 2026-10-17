use crate::{Result, client};
use std::path::Path;
use ws_daemon::triggers::Trigger;

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let response = client::request(socket, hyper::Method::GET, "/v1/triggers", vec![]).await?;
    let triggers: Vec<Trigger> = serde_json::from_slice(&response)?;
    if json {
        println!("{}", serde_json::to_string(&triggers)?);
    } else {
        for trigger in triggers {
            let sources = trigger
                .sources
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let template = trigger
                .template
                .replace('\\', "\\\\")
                .replace('\t', "\\t")
                .replace('\n', "\\n")
                .replace('\r', "\\r");
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                trigger.id,
                sources,
                trigger.event,
                trigger.action,
                trigger.target.map(|id| id.to_string()).unwrap_or_default(),
                template
            );
        }
    }
    Ok(())
}
