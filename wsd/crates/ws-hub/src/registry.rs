//! The device registry: one SQLite row for each device that may call the hub.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::identity::{Result, certificate_subject};

pub const DB_FILE: &str = "hub.db";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub added_at: i64,
    pub revoked_at: Option<i64>,
}

pub struct Registry {
    connection: Mutex<Connection>,
}

impl Registry {
    pub fn open(state: &Path) -> Result<Self> {
        fs::create_dir_all(state)?;
        let connection = Connection::open(state.join(DB_FILE))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS devices (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                fingerprint TEXT NOT NULL UNIQUE,
                added_at INTEGER NOT NULL,
                revoked_at INTEGER
            );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Adds the device of a certificate file. Returns the new row.
    pub fn add_device(&self, cert: &Path, name: &str) -> Result<DeviceRow> {
        self.add_device_pem(&fs::read_to_string(cert)?, name)
    }

    /// Adds the device of a certificate in PEM form. Returns the new row.
    pub fn add_device_pem(&self, pem: &str, name: &str) -> Result<DeviceRow> {
        let row = DeviceRow {
            id: certificate_subject(pem)?,
            name: name.to_owned(),
            fingerprint: pem_fingerprint(pem)?,
            added_at: now(),
            revoked_at: None,
        };
        self.connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO devices (id, name, fingerprint, added_at) VALUES (?1, ?2, ?3, ?4)",
                params![row.id, row.name, row.fingerprint, row.added_at],
            )
            .map_err(|error| format!("Cannot add device {}: {error}", row.id))?;
        Ok(row)
    }

    /// Returns the device that holds an unrevoked certificate with this fingerprint.
    pub fn active_by_fingerprint(&self, fingerprint: &str) -> Result<Option<DeviceRow>> {
        Ok(self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT id, name, fingerprint, added_at, revoked_at FROM devices
                 WHERE fingerprint = ?1 AND revoked_at IS NULL",
                [fingerprint],
                read_row,
            )
            .optional()?)
    }

    pub fn list(&self) -> Result<Vec<DeviceRow>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, name, fingerprint, added_at, revoked_at FROM devices ORDER BY added_at, id",
        )?;
        let rows = statement
            .query_map([], read_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Revokes a device. Returns false when no unrevoked device has this id.
    pub fn revoke(&self, id: &str) -> Result<bool> {
        let changed = self.connection.lock().unwrap().execute(
            "UPDATE devices SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![id, now()],
        )?;
        Ok(changed > 0)
    }
}

pub fn state_db(state: &Path) -> PathBuf {
    state.join(DB_FILE)
}

pub fn der_fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pem_fingerprint(pem: &str) -> Result<String> {
    let (_, parsed) = x509_parser::pem::parse_x509_pem(pem.as_bytes())?;
    Ok(der_fingerprint(&parsed.contents))
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        id: row.get(0)?,
        name: row.get(1)?,
        fingerprint: row.get(2)?,
        added_at: row.get(3)?,
        revoked_at: row.get(4)?,
    })
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}
