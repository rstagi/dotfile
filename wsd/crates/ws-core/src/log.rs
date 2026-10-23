//! Durable JSON Lines storage for one session.

use std::{
    fs::File,
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ids::{DeviceId, EventId, SessionId, Ulid};

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    pub event_id: EventId,
    pub session_id: SessionId,
    pub epoch: u64,
    pub seq: u64,
    pub prev_hash: String,
    pub ts_ms: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub body: Value,
    pub device_id: DeviceId,
    pub sig: String,
}

#[derive(Debug, Default)]
pub struct LogRead {
    pub entries: Vec<LogEntry>,
    /// The one-based line number of an incomplete final record.
    pub truncated_line: Option<usize>,
    valid_bytes: u64,
    head_hash: String,
}

/// Complete entries after a byte offset. A log shorter than the offset restarts at zero.
#[derive(Debug, Default, PartialEq)]
pub struct LogTail {
    pub entries: Vec<LogEntry>,
    /// The byte offset after the last complete line.
    pub offset: u64,
    pub restarted: bool,
}

#[derive(Clone, Debug)]
pub struct SessionLog {
    path: PathBuf,
    session_id: SessionId,
    device_id: DeviceId,
}

/// Retains a verified head. Every append checks file identity, size and modification time.
pub struct LogWriter {
    log: SessionLog,
    head: Option<WriterHead>,
}

struct WriterHead {
    stamp: FileStamp,
    seq: u64,
    epoch: u64,
    valid_bytes: u64,
    hash: String,
}

#[derive(PartialEq)]
struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: SystemTime,
    changed: (i64, i64),
}

impl SessionLog {
    pub fn new(state: &Path, session_id: SessionId, device_id: DeviceId) -> Self {
        Self {
            path: state
                .join("sessions")
                .join(session_id.to_string())
                .join("log.jsonl"),
            session_id,
            device_id,
        }
    }

    /// Appends and fsyncs one record. Sequence numbers start at 1. The first hash is empty.
    /// A crash tail is removed under the writer lock before the next append.
    /// Hashes use RFC 8785. Store integers outside the exact JSON range as strings.
    pub fn append(&self, epoch: u64, event_type: &str, body: Value) -> io::Result<LogEntry> {
        self.writer().append(Some(epoch), event_type, body)
    }

    /// Creates a writer that avoids repeated validation when the log file does not change externally.
    pub fn writer(&self) -> LogWriter {
        LogWriter {
            log: self.clone(),
            head: None,
        }
    }

    /// Checks sequence numbers and hash links. Reports an incomplete final line separately.
    /// An unsigned chain cannot detect removal or alteration of its final entry without a trusted head.
    pub fn read(&self) -> io::Result<LogRead> {
        match File::open(&self.path) {
            Ok(mut file) => {
                file.lock_shared()?;
                read_file(&mut file, self.session_id)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LogRead::default()),
            Err(error) => Err(error),
        }
    }

    /// Reads complete entries after `offset` without hash-chain validation.
    /// Use it for incremental readers. Use `read` when the chain must be verified.
    pub fn read_after(&self, offset: u64) -> io::Result<LogTail> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LogTail {
                    restarted: offset > 0,
                    ..LogTail::default()
                });
            }
            Err(error) => return Err(error),
        };
        file.lock_shared()?;
        let restarted = file.metadata()?.len() < offset;
        let mut tail = LogTail {
            offset: if restarted { 0 } else { offset },
            restarted,
            ..LogTail::default()
        };
        file.seek(SeekFrom::Start(tail.offset))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 || !line.ends_with(b"\n") {
                return Ok(tail);
            }
            tail.offset += line.len() as u64;
            tail.entries.push(serde_json::from_slice(&line)?);
        }
    }
}

impl LogWriter {
    /// Appends in the current epoch. A changed file forces complete hash-chain validation.
    pub fn append_current(&mut self, event_type: &str, body: Value) -> io::Result<LogEntry> {
        self.append(None, event_type, body)
    }

    fn append(
        &mut self,
        epoch: Option<u64>,
        event_type: &str,
        body: Value,
    ) -> io::Result<LogEntry> {
        let directory = self.log.path.parent().unwrap();
        crate::create_state_directory(directory)?;
        let mut file = crate::lockfile::open_lock_file(&self.log.path)?;
        file.lock()?;
        let result = self.append_locked(&mut file, epoch, event_type, body);
        let unlocked = file.unlock();
        match (result, unlocked) {
            (Ok(entry), Ok(())) => Ok(entry),
            (Err(error), _) | (_, Err(error)) => {
                self.head = None;
                Err(error)
            }
        }
    }

    fn append_locked(
        &mut self,
        file: &mut File,
        epoch: Option<u64>,
        event_type: &str,
        body: Value,
    ) -> io::Result<LogEntry> {
        let stamp = file_stamp(file)?;
        let previous = match self.head.take() {
            Some(head) if head.stamp == stamp => head,
            _ => {
                let read = read_file(file, self.log.session_id)?;
                WriterHead {
                    stamp,
                    seq: read.entries.len() as u64,
                    epoch: read.entries.last().map_or(0, |entry| entry.epoch),
                    valid_bytes: read.valid_bytes,
                    hash: read.head_hash,
                }
            }
        };
        let epoch = epoch.unwrap_or(previous.epoch);
        if epoch < previous.epoch {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The append epoch is older than the log epoch.",
            ));
        }
        let entry = LogEntry {
            event_id: Ulid::new(),
            session_id: self.log.session_id,
            epoch,
            seq: previous.seq + 1,
            prev_hash: previous.hash,
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis()
                .try_into()
                .map_err(io::Error::other)?,
            event_type: event_type.into(),
            body,
            device_id: self.log.device_id,
            sig: String::new(),
        };
        let value = serde_json::to_value(&entry)?;
        validate_numbers(&value)?;
        let mut bytes = serde_json::to_vec(&entry)?;
        bytes.push(b'\n');
        // Use the same stored representation that a reader validates.
        let hash = entry_hash(&serde_json::from_slice(&bytes)?).map_err(io::Error::other)?;
        if previous.stamp.size != previous.valid_bytes {
            file.set_len(previous.valid_bytes)?;
        }
        file.seek(SeekFrom::Start(previous.valid_bytes))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        File::open(self.log.path.parent().unwrap())?.sync_all()?;
        self.head = Some(WriterHead {
            stamp: file_stamp(file)?,
            seq: entry.seq,
            epoch,
            valid_bytes: previous.valid_bytes + bytes.len() as u64,
            hash,
        });
        Ok(entry)
    }
}

fn file_stamp(file: &File) -> io::Result<FileStamp> {
    let metadata = file.metadata()?;
    Ok(FileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified: metadata.modified()?,
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

fn read_file(file: &mut File, session_id: SessionId) -> io::Result<LogRead> {
    let mut result = LogRead::default();
    let mut epoch = 0;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let number = result.entries.len() + 1;
        if !line.ends_with(b"\n") {
            result.truncated_line = Some(number);
            break;
        }
        // Parse once. The hash uses the stored JSON. Typed ULIDs normalize their spelling.
        let value: Value = serde_json::from_slice(&line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid log entry {number}: {error}"),
            )
        })?;
        let entry = LogEntry::deserialize(&value).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid log entry {number}: {error}"),
            )
        })?;
        let reason = if entry.session_id != session_id {
            Some("session_id does not match the log")
        } else if entry.seq != number as u64 {
            Some("seq is not consecutive")
        } else if entry.prev_hash != result.head_hash {
            Some("prev_hash does not match the previous entry")
        } else if entry.epoch < epoch {
            Some("epoch decreased")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid log entry {number} ({}): {reason}.", entry.event_id),
            ));
        }
        result.head_hash = entry_hash(&value).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid log entry {number} ({}): {error}", entry.event_id),
            )
        })?;
        epoch = entry.epoch;
        result.entries.push(entry);
        result.valid_bytes += line.len() as u64;
    }
    Ok(result)
}

fn entry_hash(entry: &Value) -> io::Result<String> {
    validate_numbers(entry)?;
    let canonical = serde_json_canonicalizer::to_vec(entry)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn validate_numbers(value: &Value) -> io::Result<()> {
    match value {
        Value::Number(number)
            if number.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|n| n < -(MAX_SAFE_INTEGER as i64)) =>
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The integer exceeds the exact JSON range. Store it as a string.",
            ));
        }
        Value::Array(values) => {
            for value in values {
                validate_numbers(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_numbers(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Ulid;
    use serde_json::json;
    use std::fs::{self, OpenOptions};

    #[test]
    fn preserves_float_bodies_and_their_hashes_after_reopening() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        let body = json!({"numbers": [51.248178375505404, -93.31137037688033, 2.0030397744267762e-253, 7.101215824554616e260, -0.0]});
        let first = log.append(0, "test.number", body).unwrap();
        let second = log.append(0, "test.number", json!({})).unwrap();
        assert_eq!(log.read().unwrap().entries, [first, second]);
    }

    #[test]
    fn reads_only_complete_entries_after_an_offset_and_restarts_on_a_shorter_log() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        assert_eq!(log.read_after(0).unwrap(), LogTail::default());
        let first = log.append(0, "a", json!({})).unwrap();
        let head = log.read_after(0).unwrap();
        assert_eq!((head.entries, head.restarted), (vec![first], false));
        let second = log.append(0, "b", json!({})).unwrap();
        let path = sandbox
            .path()
            .join("sessions")
            .join(second.session_id.to_string())
            .join("log.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"partial")
            .unwrap();
        let tail = log.read_after(head.offset).unwrap();
        assert_eq!(tail.entries, [second]);
        assert_eq!(tail.offset, fs::metadata(&path).unwrap().len() - 9);
        assert!(log.read_after(tail.offset).unwrap().entries.is_empty());
        fs::write(&path, b"").unwrap();
        let restarted = log.read_after(tail.offset).unwrap();
        assert!(restarted.restarted && restarted.entries.is_empty());
        assert_eq!(restarted.offset, 0);
    }

    #[test]
    fn rejects_integers_that_canonical_json_would_round() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        for number in [
            json!(9_007_199_254_740_993_u64),
            json!(-9_007_199_254_740_993_i64),
        ] {
            let error = log
                .append(0, "test.number", json!({"nested": [number]}))
                .unwrap_err();
            assert!(error.to_string().contains("integer"));
        }
        assert!(log.read().unwrap().entries.is_empty());
        let entry = log
            .append(
                0,
                "test.number",
                json!({"value": 9_007_199_254_740_991_u64}),
            )
            .unwrap();
        assert_eq!(entry.seq, 1);
        assert_eq!(log.read().unwrap().entries, [entry]);
    }

    #[test]
    fn rejects_an_older_epoch_without_changing_the_log() {
        let sandbox = tempfile::tempdir().unwrap();
        let session = Ulid::new();
        let log = SessionLog::new(sandbox.path(), session, Ulid::new());
        let first = log.append(1, "session.created", json!({})).unwrap();
        let error = log.append(0, "session.started", json!({})).unwrap_err();
        assert!(error.to_string().contains("epoch"));
        assert_eq!(log.read().unwrap().entries, [first]);
        assert_eq!(log.append(2, "session.started", json!({})).unwrap().seq, 2);
    }

    #[test]
    fn two_processes_append_one_chain_without_sequence_gaps() {
        let sandbox = tempfile::tempdir().unwrap();
        let session = Ulid::new();
        let device = Ulid::new();
        let mut workers = Vec::new();
        for worker in 0..2 {
            workers.push(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--ignored", "--exact", "log::tests::append_worker"])
                    .env("WS_LOG_TEST_ROOT", sandbox.path())
                    .env("WS_LOG_TEST_SESSION", session.to_string())
                    .env("WS_LOG_TEST_DEVICE", device.to_string())
                    .env("WS_LOG_TEST_WORKER", worker.to_string())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !(sandbox.path().join("ready-0").exists() && sandbox.path().join("ready-1").exists())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "Append workers did not become ready."
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        fs::write(sandbox.path().join("start"), "").unwrap();
        let outputs: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.wait_with_output().unwrap())
            .collect();
        for output in outputs {
            assert!(output.status.success(), "{output:?}");
        }
        let read = SessionLog::new(sandbox.path(), session, device)
            .read()
            .unwrap();
        assert_eq!(read.entries.len(), 80);
        assert_eq!(read.truncated_line, None);
        let ids: std::collections::HashSet<_> =
            read.entries.iter().map(|entry| entry.event_id).collect();
        assert_eq!(ids.len(), 80);
        for (index, entry) in read.entries.iter().enumerate() {
            assert_eq!(entry.seq, index as u64 + 1);
        }
        for worker in 0..2 {
            assert_eq!(
                read.entries
                    .iter()
                    .filter(|entry| entry.body["worker"] == worker)
                    .count(),
                40
            );
        }
    }

    #[test]
    #[ignore = "Runs in a child process for the concurrent append test."]
    fn append_worker() {
        let Some(root) = std::env::var_os("WS_LOG_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let session = std::env::var("WS_LOG_TEST_SESSION")
            .unwrap()
            .parse()
            .unwrap();
        let device = std::env::var("WS_LOG_TEST_DEVICE")
            .unwrap()
            .parse()
            .unwrap();
        let worker: u64 = std::env::var("WS_LOG_TEST_WORKER")
            .unwrap()
            .parse()
            .unwrap();
        fs::write(root.join(format!("ready-{worker}")), "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("start").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "Append worker did not receive the start signal."
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let log = SessionLog::new(&root, session, device);
        for index in 0..40 {
            log.append(0, "test.append", json!({"worker": worker, "index": index}))
                .unwrap();
        }
    }

    #[test]
    fn reports_and_ignores_a_crash_tail_then_resumes_without_a_gap() {
        for suffix in [b"{\"event_id\":\"partial".as_slice(), b"{}", &[0xff, 0xfe]] {
            let sandbox = tempfile::tempdir().unwrap();
            let session = Ulid::new();
            let log = SessionLog::new(sandbox.path(), session, Ulid::new());
            let first = log.append(0, "session.created", json!({})).unwrap();
            let path = sandbox
                .path()
                .join("sessions")
                .join(session.to_string())
                .join("log.jsonl");
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(suffix)
                .unwrap();
            let before = fs::read(&path).unwrap();
            let read = log.read().unwrap();
            assert_eq!(read.entries.as_slice(), std::slice::from_ref(&first));
            assert_eq!(read.truncated_line, Some(2));
            assert_eq!(fs::read(&path).unwrap(), before);
            let second = log.append(0, "session.started", json!({})).unwrap();
            assert_eq!(second.seq, 2);
            let read = log.read().unwrap();
            assert_eq!(read.entries, [first, second]);
            assert_eq!(read.truncated_line, None);
        }
    }

    #[test]
    fn appends_survive_reopening_with_all_v6_fields() {
        let sandbox = tempfile::tempdir().unwrap();
        let session = Ulid::new();
        let device = Ulid::new();
        let log = SessionLog::new(sandbox.path(), session, device);
        let first = log
            .append(0, "session.created", json!({"name": "build"}))
            .unwrap();
        let second = log.append(0, "session.started", json!({})).unwrap();
        let reopened = SessionLog::new(sandbox.path(), session, device);
        let read = reopened.read().unwrap();
        assert_eq!(read.entries, [first.clone(), second.clone()]);
        assert_eq!(read.truncated_line, None);
        assert_eq!(first.session_id, session);
        assert_eq!(first.device_id, device);
        assert_eq!(first.epoch, 0);
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert!(first.prev_hash.is_empty());
        assert_ne!(first.event_id, second.event_id);
        assert!(first.ts_ms > 0);
        assert!(first.sig.is_empty());
        let path = sandbox
            .path()
            .join("sessions")
            .join(session.to_string())
            .join("log.jsonl");
        let text = std::fs::read_to_string(path).unwrap();
        let entries: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["type"], "session.created");
        assert_eq!(entries[0]["body"], json!({"name": "build"}));
        assert_eq!(entries[0].as_object().unwrap().len(), 10);
        assert!(text.ends_with('\n'));
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn detects_changes_to_the_stored_identifier_spelling() {
        let sandbox = tempfile::tempdir().unwrap();
        let session: SessionId = "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap();
        let log = SessionLog::new(sandbox.path(), session, Ulid::new());
        log.append(0, "session.created", json!({})).unwrap();
        log.append(0, "session.started", json!({})).unwrap();
        let path = sandbox
            .path()
            .join("sessions")
            .join(session.to_string())
            .join("log.jsonl");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace(&session.to_string(), &session.to_string().to_lowercase());
        fs::write(path, text).unwrap();
        let error = log.read().unwrap_err().to_string();
        assert!(error.contains("entry 2"), "{error}");
        assert!(error.contains("prev_hash"), "{error}");
    }

    #[test]
    fn hashes_canonical_json_and_reports_the_first_broken_link() {
        let sandbox = tempfile::tempdir().unwrap();
        let session = "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap();
        let device = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let log = SessionLog::new(sandbox.path(), session, device);
        let path = sandbox
            .path()
            .join("sessions")
            .join(session.to_string())
            .join("log.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let seed = r#"{"type":"session.created","ts_ms":123,"sig":"","session_id":"01ARZ3NDEKTSV4RRFFQ69G5FAX","seq":1,"prev_hash":"","event_id":"01ARZ3NDEKTSV4RRFFQ69G5FAW","epoch":0,"device_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","body":{"z":true,"a":1}}"#;
        fs::write(&path, format!("{seed}\n")).unwrap();
        let second = log.append(0, "session.started", json!({})).unwrap();
        assert_eq!(
            second.prev_hash,
            "cb853bba3787d347fa72453b97f80d6854a0157d7a6cbf218450ce5ffa1edb20"
        );
        assert_eq!(log.read().unwrap().entries.len(), 2);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("\"a\":1", "\"a\":2");
        fs::write(&path, text).unwrap();
        let error = log.read().unwrap_err().to_string();
        assert!(error.contains("entry 2"), "{error}");
        assert!(error.contains(&second.event_id.to_string()), "{error}");
        assert!(error.contains("prev_hash"), "{error}");
        let before = fs::read(&path).unwrap();
        assert!(log.append(0, "session.finished", json!({})).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[cfg(test)]
mod writer_tests {
    use super::*;
    use serde_json::json;
    use std::fs::OpenOptions;

    #[test]
    fn retained_writer_revalidates_external_appends_crash_tails_and_corruption() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        let mut writer = log.writer();
        writer.append_current("first", json!({"value":1})).unwrap();
        log.append(2, "external", json!({})).unwrap();
        let third = writer.append_current("third", json!({})).unwrap();
        assert_eq!((third.seq, third.epoch), (3, 2));
        OpenOptions::new()
            .append(true)
            .open(&log.path)
            .unwrap()
            .write_all(b"{partial")
            .unwrap();
        assert_eq!(writer.append_current("fourth", json!({})).unwrap().seq, 4);
        assert_eq!(log.read().unwrap().entries.len(), 4);
        let text = std::fs::read_to_string(&log.path)
            .unwrap()
            .replace("\"value\":1", "\"value\":2");
        std::fs::write(&log.path, text).unwrap();
        assert!(writer.append_current("rejected", json!({})).is_err());
    }

    #[test]
    fn retained_writer_hashes_the_stored_float_representation() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        let mut writer = log.writer();
        writer
            .append_current("float", json!({"numbers":[51.248178375505404, -93.31137037688033, 2.0030397744267762e-253, 7.101215824554616e260, -0.0]}))
            .unwrap();
        writer.append_current("next", json!({})).unwrap();
        assert_eq!(log.read().unwrap().entries.len(), 2);
    }

    #[test]
    fn independent_retained_writers_keep_one_hash_chain() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        let mut first = log.writer();
        let mut second = log.writer();
        for seq in 1..=20 {
            let writer = if seq % 2 == 0 {
                &mut first
            } else {
                &mut second
            };
            assert_eq!(writer.append_current("event", json!({})).unwrap().seq, seq);
        }
        assert_eq!(log.read().unwrap().entries.len(), 20);
    }
}
