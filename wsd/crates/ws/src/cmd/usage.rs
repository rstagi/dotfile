use std::path::Path;

use hyper::Method;
use ws_daemon::quota::{Source, usage::Usage};

use crate::{Result, client};

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let body = client::request(socket, Method::GET, "/v1/usage", Vec::new()).await?;
    let usage: Usage = serde_json::from_slice(&body)?;
    if json {
        println!("{}", serde_json::to_string(&usage)?);
    } else {
        println!("RUNNER\tACCOUNT\tBUCKET\tWINDOW\tREMAINING\tSOURCE\tAGE");
        for window in usage.windows {
            let remaining = window
                .remaining_ratio
                .map(|ratio| format!("{:.1}%", ratio * 100.0))
                .unwrap_or(window.status);
            let source = match window.source {
                Some(Source::Reported) => "reported",
                Some(Source::Estimated) => "estimated",
                None => "unknown",
            };
            let age = window
                .age_secs
                .map(|age| format!("{age}s ago"))
                .unwrap_or_else(|| "unknown".into());
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                window.runner,
                window.account_id.as_deref().unwrap_or("unknown"),
                window.bucket.as_deref().unwrap_or("-"),
                window.window,
                remaining,
                source,
                age
            );
        }
    }
    Ok(())
}
