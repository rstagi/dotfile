use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Subcommand;
use ws_core::{config::Config, device::Device};
use ws_hub::{client::HubClient, identity, pair};

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
    /// Pair this device with a hub. Print a code, then wait for an approval.
    Join {
        /// The hub URL, for example https://hub.example.com
        hub_url: String,
        #[arg(long)]
        name: String,
        /// CA file that signs the hub server certificate. The default is the system roots.
        #[arg(long)]
        ca: Option<PathBuf>,
    },
    /// Approve the code of a joining device. Run it on a device that the hub has.
    Approve {
        code: String,
        /// Command that prints the root key on stdout.
        #[arg(long, default_value = identity::DEFAULT_ROOT_KEY_COMMAND)]
        root_key_cmd: String,
        #[arg(long, default_value_t = identity::DEFAULT_DEVICE_DAYS)]
        days: i64,
    },
    /// List the devices of the hub.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Revoke a device at the hub.
    Remove { id: String },
}

#[derive(Subcommand)]
pub enum CaCommand {
    /// Write a new root key and a root certificate.
    Init {
        #[arg(long)]
        out: PathBuf,
    },
}

pub async fn device(command: DeviceCommand) -> Result<ExitCode> {
    match command {
        DeviceCommand::List { json } => return super::hub::device_list(json).await,
        DeviceCommand::Remove { id } => return super::hub::device_remove(&id).await,
        DeviceCommand::Join { hub_url, name, ca } => {
            device_join(&hub_url, &name, ca.as_deref()).await?
        }
        DeviceCommand::Approve {
            code,
            root_key_cmd,
            days,
        } => return device_approve(&code, &root_key_cmd, days).await,
        DeviceCommand::Init => device_init()?,
        DeviceCommand::Sign {
            csr,
            root_cert,
            root_key_cmd,
            days,
        } => device_sign(&csr, &root_cert, &root_key_cmd, days)?,
    }
    Ok(ExitCode::SUCCESS)
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

/// Pairs this device. The hub answers the wait when an approval arrives.
async fn device_join(hub_url: &str, name: &str, ca: Option<&Path>) -> Result<()> {
    let state = crate::init::state_path()?;
    if Config::load(&state)?.hub.is_some() {
        return Err("This device already has a [hub] section in config.toml.".into());
    }
    device_init()?;
    let directory = Device::key_directory(&state);
    let device_id = Device::load(&state)?.device_id.to_string();
    let csr = fs::read_to_string(directory.join(identity::DEVICE_CSR_FILE))?;
    let client = HubClient::anonymous(hub_url, ca)?;
    let code = pair::start(&client, name, &csr).await?;
    println!("Pairing code: {code}");
    println!("On a trusted device run: ws device approve {code}");
    let result = pair::wait_for_certificate(&client, &code).await?;
    if identity::verify_device_cert(&result.ca, &result.cert)? != device_id {
        return Err("The hub sent a certificate for another device.".into());
    }
    let cert = directory.join("cert.pem");
    let root = directory.join("root.pem");
    fs::write(&cert, &result.cert)?;
    fs::write(&root, &result.ca)?;
    write_hub_section(&state, hub_url, &cert, &directory.join("key.pem"), &root)?;
    println!("Paired as device {device_id}");
    Ok(())
}

async fn device_approve(code: &str, root_key_cmd: &str, days: i64) -> Result<ExitCode> {
    let Some(hub) = super::hub::hub_config()? else {
        return Ok(super::hub::no_hub());
    };
    let client = HubClient::new(&hub.url, &hub.cert, &hub.key, &hub.ca)?;
    let request = pair::fetch_request(&client, code).await?;
    let root_cert = fs::read_to_string(&hub.ca)?;
    let root_key = identity::read_root_key(root_key_cmd)?;
    let cert = identity::sign_device_csr(&request.csr, &root_cert, &root_key, days)?;
    pair::send_certificate(&client, code, &cert).await?;
    println!(
        "Approved {} ({})",
        request.name,
        identity::certificate_subject(&cert)?
    );
    Ok(ExitCode::SUCCESS)
}

/// Appends the `[hub]` section. The rest of config.toml stays as it is.
fn write_hub_section(state: &Path, url: &str, cert: &Path, key: &Path, ca: &Path) -> Result<()> {
    let quote = |path: &Path| serde_json::to_string(&path.to_string_lossy());
    let mut section = format!(
        "[hub]\nurl = {}\ncert = {}\nkey = {}\nca = {}\n",
        serde_json::to_string(url)?,
        quote(cert)?,
        quote(key)?,
        quote(ca)?
    );
    let path = state.join("config.toml");
    if fs::read_to_string(&path).is_ok_and(|text| !text.is_empty() && !text.ends_with('\n')) {
        section.insert(0, '\n');
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?
        .write_all(section.as_bytes())?;
    Ok(())
}
