//! Slot files for git worktrees and session directories.

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
};
use ws_core::{ids::Ulid, lockfile::open_lock_file};

pub fn resume(state: &Path, wt: &Path, id: &str) -> io::Result<Option<String>> {
    Ok(read(&path(wt, Some(state))?)?.lines().find_map(|row| {
        let fields: Vec<_> = row.split('\t').collect();
        (fields.len() >= 4 && fields[3] == id && !fields[2].is_empty())
            .then(|| fields[2].to_owned())
    }))
}

pub fn register(
    state: &Path,
    wt: &Path,
    preferred: Option<u64>,
    agent: &str,
    id: &str,
) -> io::Result<u64> {
    rewrite(&path(wt, Some(state))?, |rows| {
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

pub fn remove(state: &Path, wt: &Path, id: &str) -> io::Result<()> {
    rewrite(&path(wt, Some(state))?, |rows| {
        rows.retain(|row| row.split('\t').nth(3) != Some(id));
        Ok(())
    })
}

/// Frees a zsh slot row unless the slot now belongs to another session.
pub fn remove_legacy(wt: &Path, slot: &str, id: &str) -> io::Result<()> {
    rewrite(&path(wt, None)?, |rows| {
        rows.retain(|row| {
            let mut fields = row.split('\t');
            fields.next() != Some(slot)
                || !matches!(fields.nth(2), Some(owner) if owner == id || owner.is_empty())
        });
        Ok(())
    })
}

fn read(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

fn rewrite<T>(path: &Path, edit: impl FnOnce(&mut Vec<String>) -> io::Result<T>) -> io::Result<T> {
    let lock = open_lock_file(&path.with_extension("lock"))?;
    lock.lock()?;
    let mut rows = read(path)?.lines().map(str::to_owned).collect();
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
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map(|()| value)
}

fn path(wt: &Path, state: Option<&Path>) -> io::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(wt)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()?;
    if !output.status.success() {
        let state =
            state.ok_or_else(|| io::Error::other("Cannot locate the workspace git directory."))?;
        let directory = wt.canonicalize()?;
        let hash = format!(
            "{:x}",
            Sha256::digest(directory.as_os_str().as_encoded_bytes())
        );
        let directory = state.join("places").join(hash);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        return Ok(directory.join("ws-sessions"));
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)
            .map_err(io::Error::other)?
            .trim_end(),
    )
    .join("ws-sessions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_without_repos_have_private_independent_slots() {
        let sandbox = tempfile::tempdir().unwrap();
        let state = sandbox.path().join("state");
        let first = sandbox.path().join("first");
        let second = sandbox.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        assert_eq!(register(&state, &first, None, "claude", "one").unwrap(), 1);
        assert_eq!(register(&state, &first, None, "codex", "two").unwrap(), 2);
        assert_eq!(
            register(&state, &second, None, "claude", "three").unwrap(),
            1
        );
        let link = sandbox.path().join("link");
        std::os::unix::fs::symlink(&first, &link).unwrap();
        assert_eq!(register(&state, &link, None, "claude", "one").unwrap(), 1);
        remove(&state, &first, "one").unwrap();
        assert_eq!(
            register(&state, &first, Some(1), "grok", "four").unwrap(),
            1
        );
        let directories: Vec<_> = fs::read_dir(state.join("places"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(directories.len(), 2);
        for directory in directories {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(directory.file_name().unwrap().len(), 64);
            assert_eq!(
                fs::metadata(directory.join("ws-sessions"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert!(
                !fs::read_to_string(directory.join("ws-sessions"))
                    .unwrap()
                    .contains("\tone\n")
            );
        }
    }
}
