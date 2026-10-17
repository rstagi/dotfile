//! Locks the same files and byte ranges as zsh/system.

use std::{
    fs::File,
    io,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Locks {
    files: Vec<File>,
    absent: Vec<PathBuf>,
}

impl Locks {
    pub fn acquire(&mut self, path: &Path, dry_run: bool) -> io::Result<()> {
        if dry_run && !path.try_exists()? {
            self.absent.push(path.into());
            return Ok(());
        }
        let file = ws_core::lockfile::open_lock_file(path)?;
        // zsystem flock uses POSIX record locks, not the flock syscall.
        let request = libc::flock {
            l_start: 0,
            l_len: 0,
            l_pid: 0,
            l_type: libc::F_WRLCK as _,
            l_whence: libc::SEEK_SET as _,
        };
        // SAFETY: the descriptor is open and request points to an initialized flock.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &request) } == -1 {
            let error = io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::EAGAIN | libc::EACCES)) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "The zsh state is busy at {}. Retry the import.",
                        path.display()
                    ),
                ));
            }
            return Err(error);
        }
        self.files.push(file);
        Ok(())
    }

    pub fn finish(&self) -> io::Result<()> {
        // A first writer creates its lock before it changes an artifact.
        // Dry-run must detect that writer without creating missing lock files.
        for path in &self.absent {
            if path.try_exists()? {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "The zsh lock set changed. Retry the import.",
                ));
            }
        }
        Ok(())
    }
}
