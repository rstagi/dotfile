use crate::Result;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

pub async fn run(socket: &Path) -> Result<()> {
    let (name, wt) = current_workspace()?;
    let common = PathBuf::from(git(
        &wt,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    if common.file_name().is_none_or(|name| name != ".git") {
        return Err(format!("cannot find main checkout for {}", wt.display()).into());
    }
    let repo = common.parent().unwrap();
    let branch = git(&wt, &["branch", "--show-current"])?;
    if branch.is_empty() {
        return Err(format!("merge: detached HEAD in {}", wt.display()).into());
    }
    let output = Command::new("gh")
        .current_dir(&wt)
        .args([
            "pr",
            "view",
            &branch,
            "--json",
            "number,title,url",
            "--jq",
            "\"#\\(.number) \\(.title) (\\(.url))\"",
        ])
        .output()
        .map_err(|_| format!("merge: no PR found for {branch}"))?;
    let pr = String::from_utf8(output.stdout)?;
    let pr = pr.trim_end();
    if pr.is_empty() {
        return Err(format!("merge: no PR found for {branch}").into());
    }
    let method = std::env::var("WS_MERGE_METHOD")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "squash".into());
    if !confirm(&format!("merge PR {pr} ({method})?"))? {
        println!("not merged");
        return Ok(());
    }
    println!("→ merging PR for {branch} ({method})");
    if !Command::new("gh")
        .current_dir(&wt)
        .args(["pr", "merge", &branch, &format!("--{method}")])
        .status()?
        .success()
    {
        let state = Command::new("gh")
            .current_dir(&wt)
            .args(["pr", "view", &branch, "--json", "state", "-q", ".state"])
            .output();
        if !state.is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim_end() == "MERGED")
        {
            return Err("merge: gh pr merge failed".into());
        }
        println!("  (already merged)");
    }
    let default = git(
        repo,
        &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"],
    )
    .unwrap_or_default();
    let default = default.strip_prefix("origin/").unwrap_or(&default);
    let default = if default.is_empty() { "main" } else { default };
    let display = match std::env::var_os("HOME") {
        Some(home) => repo
            .strip_prefix(home)
            .map(|path| format!("~/{}", path.display()))
            .unwrap_or_else(|_| repo.display().to_string()),
        None => repo.display().to_string(),
    };
    println!("→ updating {default} in {display}");
    git_run(repo, &["fetch", "-q", "origin"], "merge: fetch failed")?;
    let failure = format!("merge: cannot fast-forward {default}");
    if git(repo, &["branch", "--show-current"])? == default {
        git_run(
            repo,
            &["merge", "-q", "--ff-only", &format!("origin/{default}")],
            &failure,
        )?;
    } else {
        git_run(
            repo,
            &["fetch", "-q", "origin", &format!("{default}:{default}")],
            &failure,
        )?;
    }
    if !confirm(&format!(
        "delete {name} (worktree, tabs, local + remote {branch})?"
    ))? {
        println!("kept {name}");
        return Ok(());
    }
    let dirty = git(&wt, &["status", "--short"]).unwrap_or_default();
    let force = !dirty.is_empty();
    if force {
        println!("can't delete {name}: uncommitted/untracked files:");
        for line in dirty.lines() {
            println!("  {line}");
        }
        if !confirm("force delete anyway (these are lost)?")? {
            println!("kept {name}");
            return Ok(());
        }
    }
    let _ = git(repo, &["push", "-q", "origin", "--delete", &branch]);
    super::rm::run(socket, wt.to_str().unwrap(), force, false, true, false).await
}

pub async fn popup(socket: &Path) -> Result<ExitCode> {
    if let Err(error) = run(socket).await {
        eprintln!("ws: {error}");
    }
    print!("\npress enter to close");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(if answer.ends_with('\n') {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn current_workspace() -> Result<(String, PathBuf)> {
    if let Ok(top) = git(Path::new("."), &["rev-parse", "--show-toplevel"])
        && let Ok(workspace) = ws_daemon::workspaces::workspace_path(&top)
    {
        return Ok(workspace);
    }
    if std::env::var_os("TMUX").is_some_and(|value| !value.is_empty()) {
        let mut args = vec!["display-message", "-p"];
        let pane = std::env::var("TMUX_PANE").unwrap_or_default();
        if !pane.is_empty() {
            args.extend(["-t", &pane]);
        }
        args.push("#{@ws_path}");
        if let Ok(path) = ws_daemon::tmux::Tmux::from_env().execute(&args)
            && !path.is_empty()
            && let Ok(workspace) = ws_daemon::workspaces::workspace_path(&path)
        {
            return Ok(workspace);
        }
    }
    Err("merge: not inside a workspace".into())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_owned()
            .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim_end().into())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.ends_with('\n') && matches!(answer.trim(), "y" | "Y"))
}

fn git_run(repo: &Path, args: &[&str], failure: &str) -> Result<()> {
    if !Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()?
        .success()
    {
        return Err(failure.to_owned().into());
    }
    Ok(())
}
