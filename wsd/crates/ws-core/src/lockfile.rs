//! Lock files that a forked child never inherits.
//!
//! `flock` binds a lock to the open file description. A child that a
//! multithreaded process spawns copies every descriptor until `exec` closes
//! the close-on-exec ones. During that window the child shares the lock, so a
//! `try_lock` in the parent on a fresh description fails. Close-on-fork keeps
//! the descriptor out of the child entirely.

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// `O_CLOFORK` from the macOS SDK `sys/fcntl.h`.
#[cfg(target_vendor = "apple")]
const O_CLOFORK: i32 = 0x0800_0000;

/// Opens (and creates, mode 0600) a lock file. Keeps existing contents, so
/// every contender locks the same inode.
pub fn open_lock_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true).mode(0o600);
    #[cfg(target_vendor = "apple")]
    options.custom_flags(O_CLOFORK);
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forked_child_does_not_keep_the_lock_alive() {
        let sandbox = tempfile::tempdir().unwrap();
        let path = sandbox.path().join("test.lock");
        let first = open_lock_file(&path).unwrap();
        first.try_lock().unwrap();
        // SAFETY: the child only calls async-signal-safe functions, then exits.
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            unsafe {
                libc::usleep(300_000);
                libc::_exit(0);
            }
        }
        drop(first);
        let second = open_lock_file(&path).unwrap();
        let relocked = second.try_lock();
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(child, &mut status, 0) }, child);
        assert!(relocked.is_ok(), "{relocked:?}");
    }

    #[test]
    fn keeps_existing_contents_and_restricts_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = tempfile::tempdir().unwrap();
        let path = sandbox.path().join("test.lock");
        std::fs::write(&path, "content").unwrap();
        let lock = open_lock_file(&path).unwrap();
        lock.try_lock().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "content");
        let created = open_lock_file(&sandbox.path().join("new.lock")).unwrap();
        assert_eq!(
            created.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
