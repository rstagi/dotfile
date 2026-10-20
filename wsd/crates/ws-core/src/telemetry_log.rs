//! Durable storage for local telemetry.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ids::{DeviceId, EventId, Ulid},
    semconv,
};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Logs,
    Metrics,
    Traces,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TelemetryRecord {
    pub event_id: EventId,
    pub received_at_ms: u64,
    pub signal: Signal,
    /// One OTLP export request with one log record, metric or span.
    pub payload: Value,
}

/// Position after the last complete record that a reader consumed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TelemetryCursor {
    inode: Option<u64>,
    offset: u64,
}

/// Records after a cursor. `restarted` is true when retention replaced the file and reading began again at its start.
#[derive(Debug)]
pub struct TelemetryTail {
    pub records: Vec<TelemetryRecord>,
    pub cursor: TelemetryCursor,
    pub restarted: bool,
}

#[derive(Deserialize)]
struct ReceivedAt {
    received_at_ms: u64,
}

#[derive(Clone, Debug)]
pub struct TelemetryLog {
    directory: PathBuf,
    device_id: DeviceId,
}

impl TelemetryLog {
    pub fn new(state: &Path, device_id: DeviceId) -> Self {
        Self {
            directory: state.join("telemetry"),
            device_id,
        }
    }

    /// Appends a batch and syncs it before returning. Replaces all incoming device tags.
    pub fn append(
        &self,
        signal: Signal,
        payloads: Vec<Value>,
        received_at_ms: u64,
    ) -> io::Result<Vec<TelemetryRecord>> {
        let mut records = Vec::with_capacity(payloads.len());
        let mut bytes = Vec::new();
        for mut payload in payloads {
            tag_device(&mut payload, signal, self.device_id)?;
            let record = TelemetryRecord {
                event_id: Ulid::new(),
                received_at_ms,
                signal,
                payload,
            };
            serde_json::to_writer(&mut bytes, &record)?;
            bytes.push(b'\n');
            records.push(record);
        }
        let _lock = self.lock()?;
        let mut file = crate::lockfile::open_lock_file(&self.directory.join("log.jsonl"))?;
        repair_tail(&mut file)?;
        file.seek(SeekFrom::End(0))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        File::open(&self.directory)?.sync_all()?;
        Ok(records)
    }

    /// Returns complete records in ingestion order. Ignores an incomplete crash tail.
    pub fn read(&self) -> io::Result<Vec<TelemetryRecord>> {
        let bytes = {
            let _lock = self.lock()?;
            match File::open(self.directory.join("log.jsonl")) {
                Ok(mut file) => read_bytes(&mut file)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error),
            }
        };
        Ok(parse_records(&bytes)?.0)
    }

    /// Returns complete records after the cursor in ingestion order. Ignores an incomplete crash tail.
    pub fn read_after(&self, cursor: TelemetryCursor) -> io::Result<TelemetryTail> {
        let lock = self.lock()?;
        let mut file = match File::open(self.directory.join("log.jsonl")) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(TelemetryTail {
                    records: Vec::new(),
                    restarted: cursor != TelemetryCursor::default(),
                    cursor: TelemetryCursor::default(),
                });
            }
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        let same = cursor.inode.is_none_or(|inode| inode == metadata.ino())
            && cursor.offset <= metadata.len();
        let offset = if same { cursor.offset } else { 0 };
        file.seek(SeekFrom::Start(offset))?;
        let bytes = read_bytes(&mut file)?;
        drop(lock);
        let (records, valid_bytes) = parse_records(&bytes)?;
        Ok(TelemetryTail {
            records,
            cursor: TelemetryCursor {
                inode: Some(metadata.ino()),
                offset: offset + valid_bytes,
            },
            restarted: !same,
        })
    }

    /// Removes expired records by atomic replacement under the same lock as append.
    pub fn prune_before(&self, cutoff_ms: u64) -> io::Result<usize> {
        let _lock = self.lock()?;
        let path = self.directory.join("log.jsonl");
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        let bytes = read_bytes(&mut file)?;
        // Parse only the timestamps. Full payloads make this scan hold the append lock for seconds.
        let mut kept = Vec::new();
        let mut removed = 0;
        for line in complete_lines(&bytes) {
            let stamp: ReceivedAt = serde_json::from_slice(line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if stamp.received_at_ms < cutoff_ms {
                removed += 1;
            } else {
                kept.push(line);
            }
        }
        if removed == 0 {
            return Ok(0);
        }
        let mut replacement = tempfile::NamedTempFile::new_in(&self.directory)?;
        for line in kept {
            replacement.write_all(line)?;
            replacement.write_all(b"\n")?;
        }
        replacement.as_file().sync_all()?;
        replacement.persist(path).map_err(|error| error.error)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(removed)
    }

    fn lock(&self) -> io::Result<File> {
        crate::create_state_directory(&self.directory)?;
        let lock = crate::lockfile::open_lock_file(&self.directory.join("log.lock"))?;
        lock.lock()?;
        Ok(lock)
    }
}

impl Signal {
    pub fn resource_key(self) -> &'static str {
        match self {
            Self::Logs => "resourceLogs",
            Self::Metrics => "resourceMetrics",
            Self::Traces => "resourceSpans",
        }
    }
}

fn tag_device(payload: &mut Value, signal: Signal, device_id: DeviceId) -> io::Result<()> {
    let resources = payload
        .get_mut(signal.resource_key())
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "The OTLP resource array is missing.",
            )
        })?;
    for resource in resources {
        if resource.get("resource").is_none_or(Value::is_null) {
            resource["resource"] = json!({});
        }
        if resource["resource"]
            .get("attributes")
            .is_none_or(Value::is_null)
        {
            resource["resource"]["attributes"] = json!([]);
        }
        let attributes = resource["resource"]["attributes"]
            .as_array_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "The OTLP resource attributes are invalid.",
                )
            })?;
        attributes.retain(|attribute| attribute["key"] != semconv::DEVICE_ID);
        attributes.push(
            json!({"key": semconv::DEVICE_ID, "value": {"stringValue": device_id.to_string()}}),
        );
    }
    Ok(())
}

fn repair_tail(file: &mut File) -> io::Result<()> {
    let mut end = file.metadata()?.len();
    let mut buffer = [0_u8; 8192];
    while end > 0 {
        let start = end.saturating_sub(buffer.len() as u64);
        let length = (end - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..length])?;
        if let Some(index) = buffer[..length].iter().rposition(|byte| *byte == b'\n') {
            return file.set_len(start + index as u64 + 1);
        }
        end = start;
    }
    file.set_len(0)
}

fn read_bytes(file: &mut File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn parse_records(bytes: &[u8]) -> io::Result<(Vec<TelemetryRecord>, u64)> {
    let mut records = Vec::new();
    for line in complete_lines(bytes) {
        records.push(
            serde_json::from_slice(line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        );
    }
    Ok((records, valid_length(bytes) as u64))
}

/// Non-empty lines before the last newline. Ignores an incomplete crash tail.
fn complete_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes[..valid_length(bytes)]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
}

fn valid_length(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ids::Ulid, semconv};
    use serde_json::json;

    #[test]
    fn concurrent_append_and_prune_preserve_all_retained_records() {
        use std::{
            collections::BTreeSet,
            sync::{Arc, Barrier},
            thread,
        };
        let sandbox = tempfile::tempdir().unwrap();
        let device = Ulid::new();
        let log = TelemetryLog::new(sandbox.path(), device);
        let payload = json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": "record"}}]}]}]});
        log.append(Signal::Logs, vec![payload.clone()], 0).unwrap();
        let start = Arc::new(Barrier::new(5));
        thread::scope(|scope| {
            for _ in 0..4 {
                let start = start.clone();
                let log = TelemetryLog::new(sandbox.path(), device);
                let payload = payload.clone();
                scope.spawn(move || {
                    start.wait();
                    for _ in 0..8 {
                        log.append(Signal::Logs, vec![payload.clone()], 1000)
                            .unwrap();
                    }
                });
            }
            start.wait();
            for _ in 0..8 {
                log.prune_before(1000).unwrap();
            }
        });
        let records = log.read().unwrap();
        assert_eq!(records.len(), 32);
        assert!(records.iter().all(|record| record.received_at_ms == 1000));
        assert_eq!(
            records
                .iter()
                .map(|record| record.event_id)
                .collect::<BTreeSet<_>>()
                .len(),
            32
        );
    }

    #[test]
    fn append_repairs_an_incomplete_tail_without_losing_complete_records() {
        use std::fs::OpenOptions;
        let sandbox = tempfile::tempdir().unwrap();
        let log = TelemetryLog::new(sandbox.path(), Ulid::new());
        let payload =
            json!({"resourceMetrics": [{"scopeMetrics": [{"metrics": [{"name": "tokens"}]}]}]});
        let first = log
            .append(Signal::Metrics, vec![payload.clone()], 1000)
            .unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(sandbox.path().join("telemetry/log.jsonl"))
            .unwrap();
        file.write_all(b"{\"partial\":").unwrap();
        assert_eq!(log.read().unwrap(), first);
        let second = log.append(Signal::Metrics, vec![payload], 2000).unwrap();
        assert_eq!(log.read().unwrap(), [first, second].concat());
    }

    #[test]
    fn cursor_reads_return_only_new_records_and_restart_after_prune() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = TelemetryLog::new(sandbox.path(), Ulid::new());
        let payload = json!({"resourceLogs": [{}]});
        log.append(Signal::Logs, vec![payload.clone()], 1).unwrap();
        let first = log.read_after(TelemetryCursor::default()).unwrap();
        assert_eq!((first.records.len(), first.restarted), (1, false));
        log.append(Signal::Logs, vec![payload.clone()], 2).unwrap();
        let second = log.read_after(first.cursor).unwrap();
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].received_at_ms, 2);
        assert!(log.read_after(second.cursor).unwrap().records.is_empty());
        log.prune_before(2).unwrap();
        let third = log.read_after(second.cursor).unwrap();
        assert!(third.restarted);
        assert_eq!(third.records.len(), 1);
    }

    #[test]
    fn retention_removes_only_records_before_the_cutoff() {
        let sandbox = tempfile::tempdir().unwrap();
        let log = TelemetryLog::new(sandbox.path(), Ulid::new());
        let payload = json!({"resourceSpans": [{"scopeSpans": [{"spans": [{"name": "turn"}]}]}]});
        log.append(Signal::Traces, vec![payload.clone()], 999)
            .unwrap();
        let boundary = log
            .append(Signal::Traces, vec![payload.clone()], 1000)
            .unwrap();
        let recent = log.append(Signal::Traces, vec![payload], 2000).unwrap();
        assert_eq!(log.prune_before(1000).unwrap(), 1);
        assert_eq!(log.read().unwrap(), [boundary, recent].concat());
        assert_eq!(log.prune_before(1000).unwrap(), 0);
        assert_eq!(log.prune_before(3000).unwrap(), 2);
        assert!(log.read().unwrap().is_empty());
    }

    #[test]
    fn accepted_records_survive_reopen_with_the_local_device_identity() {
        let sandbox = tempfile::tempdir().unwrap();
        let device = Ulid::new();
        let log = TelemetryLog::new(sandbox.path(), device);
        let payload = json!({"resourceLogs": [{"resource": {"attributes": [
            {"key": semconv::DEVICE_ID, "value": {"stringValue": "untrusted"}},
            {"key": semconv::SESSION_ID, "value": {"stringValue": "session-a"}}
        ]}, "scopeLogs": [{"logRecords": [{"body": {"stringValue": "hello"}}]}]}]});
        let written = log.append(Signal::Logs, vec![payload], 1000).unwrap();
        let reopened = TelemetryLog::new(sandbox.path(), device);
        assert_eq!(reopened.read().unwrap(), written);
        assert_eq!(written.len(), 1);
        let attributes = written[0].payload["resourceLogs"][0]["resource"]["attributes"]
            .as_array()
            .unwrap();
        assert_eq!(
            attributes
                .iter()
                .filter(|a| a["key"] == semconv::DEVICE_ID)
                .count(),
            1
        );
        assert!(
            attributes.iter().any(|a| a["key"] == semconv::DEVICE_ID
                && a["value"]["stringValue"] == device.to_string())
        );
        assert!(
            attributes.iter().any(
                |a| a["key"] == semconv::SESSION_ID && a["value"]["stringValue"] == "session-a"
            )
        );
    }
}
