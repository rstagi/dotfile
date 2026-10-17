//! The worktree slot file remains compatible with the zsh backend.

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};
use ws_core::{ids::Ulid, lockfile::open_lock_file};

pub fn resume(wt: &Path, id: &str) -> io::Result<Option<String>> {
    Ok(read(wt)?.lines().find_map(|row| {
        let fields: Vec<_> = row.split('\t').collect();
        (fields.len() >= 4 && fields[3] == id && !fields[2].is_empty())
            .then(|| fields[2].to_owned())
    }))
}

pub fn register(wt: &Path, preferred: Option<u64>, agent: &str, id: &str) -> io::Result<u64> {
    rewrite(wt, |rows| {
        let fields = |row: &String| row.split('\t').map(str::to_owned).collect::<Vec<_>>();
        if let Some(slot) = rows.iter().map(fields).find_map(|f| {
            (f.len() >= 4 && f[3] == id)
                .then(|| f[0].parse::<u64>().ok())
                .flatten()
        }) {
            return Ok(slot);
        }
        let occupied = rows
            .iter()
            .filter_map(|row| row.split('\t').next()?.parse::<u64>().ok())
            .collect::<Vec<_>>();
        let slot = preferred
            .filter(|slot| !occupied.contains(slot))
            .unwrap_or_else(|| occupied.iter().max().unwrap_or(&0) + 1);
        rows.push(format!("{slot}\t{agent}\t\t{id}"));
        Ok(slot)
    })
}

pub fn remove(wt: &Path, id: &str) -> io::Result<()> {
    rewrite(wt, |rows| {
        rows.retain(|row| row.split('\t').nth(3) != Some(id));
        Ok(())
    })
}

/// Frees a zsh slot row unless the slot now belongs to another session.
pub fn remove_legacy(wt: &Path, slot: &str, id: &str) -> io::Result<()> {
    rewrite(wt, |rows| {
        rows.retain(|row| {
            let mut fields = row.split('\t');
            fields.next() != Some(slot)
                || !matches!(fields.nth(2), Some(owner) if owner == id || owner.is_empty())
        });
        Ok(())
    })
}

fn read(wt: &Path) -> io::Result<String> {
    match fs::read_to_string(path(wt)?) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

fn rewrite<T>(wt: &Path, edit: impl FnOnce(&mut Vec<String>) -> io::Result<T>) -> io::Result<T> {
    let path = path(wt)?;
    let lock = open_lock_file(&path.with_extension("lock"))?;
    lock.lock()?;
    let mut rows = read(wt)?.lines().map(str::to_owned).collect();
    let value = edit(&mut rows)?;
    let temporary = path.with_extension(format!("tmp-{}", Ulid::new()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        for row in rows {
            writeln!(file, "{row}")?;
        }
        fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map(|()| value)
}

fn path(wt: &Path) -> io::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(wt)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            "Cannot locate the workspace git directory.",
        ));
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)
            .map_err(io::Error::other)?
            .trim_end(),
    )
    .join("ws-sessions"))
}
