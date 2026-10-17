use std::{collections::BTreeMap, path::Path};

use crate::Result;
use super::list;

pub async fn rows(socket: &Path) -> Result<()> {
    let mut workspaces = BTreeMap::new();
    for row in list::rows(socket).await? {
        workspaces.insert(row.name.clone(), row);
    }
    let mut previous = String::new();
    for pinned in [true, false] {
        for row in workspaces.values().filter(|row| row.pinned == pinned) {
            let (repo, branch) = row.name.split_once('/').unwrap_or((&row.name, &row.name));
            let style = if pinned || repo != previous { "\x1b[1m" } else { "\x1b[2m" };
            let mark = if pinned { "📌" } else { "  " };
            println!("{mark} {style}{repo:<14}\x1b[0m {} {branch:<24} {} \x1b[2m{}\x1b[0m\t{}\t{}",
                unread_mark(row.unread), state_icon(&row.state),
                if row.agent == "-" { &row.state } else { &row.agent }, row.path, row.window);
            if !pinned { previous = repo.into(); }
        }
    }
    Ok(())
}

pub async fn summary(socket: &Path) -> Result<()> {
    let rows = list::rows(socket).await?;
    let mut parts = Vec::new();
    for state in ["waiting", "working", "idle", "stopped"] {
        let count = rows.iter().filter(|row| row.state == state).count();
        if count > 0 { parts.push(format!("{} {count} {state}", state_icon(state))); }
    }
    let unread = rows.iter().filter(|row| row.unread).count();
    if unread > 0 { parts.push(format!("\x1b[36m•\x1b[0m {unread} unread")); }
    println!("{}", if parts.is_empty() { "no workspaces".into() } else { parts.join(" · ") });
    println!("\x1b[2m↵ open · ^s sessions · ^p pin · ^u unread · ^x rm\x1b[0m");
    println!("\x1b[2m^n/^o/^r new claude/codex/grok · ^a/^t/^v add claude/codex/grok tab\x1b[0m");
    Ok(())
}

fn unread_mark(unread: bool) -> &'static str {
    if unread { "\x1b[1;36m•\x1b[0m" } else { " " }
}

fn state_icon(state: &str) -> &'static str {
    match state {
        "waiting" => "\x1b[31m●\x1b[0m",
        "working" => "\x1b[33m◐\x1b[0m",
        "idle" => "\x1b[32m○\x1b[0m",
        _ => "\x1b[2m·\x1b[0m",
    }
}
