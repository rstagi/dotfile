use std::{env, path::PathBuf, process::Command};

use ws_core::device::Device;

use crate::Result;

pub fn run() -> Result<()> {
    let state = state_path()?;
    let output = Command::new("hostname").output()?;
    if !output.status.success() {
        return Err("Cannot read the host name.".into());
    }
    let hostname = String::from_utf8(output.stdout)?;
    let device = Device::init(&state, hostname.trim())?;
    println!("Device {} ({})", device.device_id, device.hostname);
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WS_STATE") {
        return Ok(path.into());
    }
    let home = env::var_os("HOME").ok_or("Set HOME or WS_STATE to locate the device identity.")?;
    Ok(PathBuf::from(home).join(".ws/state"))
}
