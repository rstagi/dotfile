use std::{fs, path::PathBuf};

use clap::Subcommand;
use ws_core::device::Device;
use ws_hub::identity;

use crate::Result;

#[derive(Subcommand)]
pub enum DeviceCommand {
    /// Write the device key and a certificate request. The device id stays the same.
    Init,
    /// Sign a certificate request with the root key. Print the device certificate.
    Sign {
        #[arg(long)]
        csr: PathBuf,
        #[arg(long)]
        root_cert: PathBuf,
        /// Command that prints the root key on stdout.
        #[arg(long, default_value = identity::DEFAULT_ROOT_KEY_COMMAND)]
        root_key_cmd: String,
        #[arg(long, default_value_t = identity::DEFAULT_DEVICE_DAYS)]
        days: i64,
    },
}

#[derive(Subcommand)]
pub enum CaCommand {
    /// Write a new root key and a root certificate.
    Init {
        #[arg(long)]
        out: PathBuf,
    },
}

pub fn device(command: DeviceCommand) -> Result<()> {
    match command {
        DeviceCommand::Init => device_init(),
        DeviceCommand::Sign {
            csr,
            root_cert,
            root_key_cmd,
            days,
        } => device_sign(&csr, &root_cert, &root_key_cmd, days),
    }
}

pub fn ca(command: CaCommand) -> Result<()> {
    let CaCommand::Init { out } = command;
    let (cert, key) = identity::write_root_ca(&out)?;
    println!("Root certificate: {}", cert.display());
    println!("Root key: {}", key.display());
    println!("Store the key in 1Password with this command:");
    println!(
        "  op document create '{}' --title root-ca-key",
        key.display()
    );
    println!("Then delete the local key file: rm '{}'", key.display());
    Ok(())
}

fn device_init() -> Result<()> {
    crate::init::run()?;
    let state = crate::init::state_path()?;
    let device = Device::load(&state)?;
    let directory = Device::key_directory(&state);
    let created = identity::init_device_key(&directory, &device.device_id.to_string())?;
    println!(
        "Device key {} at {}",
        if created { "written" } else { "kept" },
        directory.display()
    );
    Ok(())
}

fn device_sign(csr: &PathBuf, root_cert: &PathBuf, root_key_cmd: &str, days: i64) -> Result<()> {
    let csr = fs::read_to_string(csr)?;
    let root_cert = fs::read_to_string(root_cert)?;
    let root_key = identity::read_root_key(root_key_cmd)?;
    print!(
        "{}",
        identity::sign_device_csr(&csr, &root_cert, &root_key, days)?
    );
    Ok(())
}
