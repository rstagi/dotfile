//! Persistent identity for the local device.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, Ulid};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub device_id: DeviceId,
    pub hostname: String,
}

impl Device {
    /// Creates the identity atomically, or returns the existing identity.
    pub fn init(state: &Path, hostname: &str) -> io::Result<Self> {
        crate::create_state_directory(state)?;
        // Keep the lock inode stable across initializations.
        let lock = crate::lockfile::open_lock_file(&state.join("device.lock"))?;
        lock.lock()?;
        match Self::load(state) {
            Ok(device) => return Ok(device),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if hostname.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The host name is empty.",
            ));
        }
        let device = Self {
            device_id: Ulid::new(),
            hostname: hostname.to_owned(),
        };
        let mut temporary = tempfile::NamedTempFile::new_in(state)?;
        temporary.write_all(&serde_json::to_vec(&device)?)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(state.join("device.json"))
            .map_err(|error| error.error)?;
        File::open(state)?.sync_all()?;
        Ok(device)
    }

    /// Returns the directory of the device key and certificate request.
    pub fn key_directory(state: &Path) -> PathBuf {
        state.join("device")
    }

    pub fn load(state: &Path) -> io::Result<Self> {
        let path = state.join("device.json");
        let bytes = fs::read(&path)?;
        serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid device identity at {}: {error}", path.display()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_a_persistent_device_without_replacing_its_identity() {
        let sandbox = tempfile::tempdir().unwrap();
        let state = sandbox.path().join("state");
        let first = Device::init(&state, "test-host").unwrap();
        assert_eq!(first.hostname, "test-host");
        assert_eq!(Device::load(&state).unwrap(), first);
        assert_eq!(Device::init(&state, "renamed-host").unwrap(), first);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state.join("device.json")).unwrap()).unwrap();
        assert_eq!(value["device_id"], first.device_id.to_string());
        assert_eq!(value["hostname"], "test-host");
    }
}
