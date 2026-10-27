use std::{io::Write, net::SocketAddr, path::PathBuf, process::ExitCode};

use clap::Subcommand;
use hyper::Method;
use ws_core::config::{Config, HubConfig};
use ws_hub::{
    client::HubClient,
    registry::{DeviceRow, Registry},
    server::{self, HubOptions},
};

use crate::Result;

#[derive(Subcommand)]
pub enum HubCommand {
    /// Serve the hub over mTLS.
    Serve {
        #[arg(long)]
        listen: SocketAddr,
        /// Server certificate chain (PEM).
        #[arg(long)]
        tls_cert: PathBuf,
        #[arg(long)]
        tls_key: PathBuf,
        /// WS root certificate that signs the device certificates.
        #[arg(long)]
        ca_cert: PathBuf,
        #[arg(long)]
        state: PathBuf,
    },
    /// Add a device to the registry on the hub host. Needs no mTLS.
    AddDevice {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        name: String,
    },
}

pub async fn hub(command: HubCommand) -> Result<ExitCode> {
    match command {
        HubCommand::Serve {
            listen,
            tls_cert,
            tls_key,
            ca_cert,
            state,
        } => {
            let hub = server::bind(&HubOptions {
                listen,
                tls_cert,
                tls_key,
                ca_cert,
                state,
            })
            .await?;
            println!("hub ready {}", hub.local_addr()?);
            std::io::stdout().flush()?;
            hub.serve().await?;
        }
        HubCommand::AddDevice { state, cert, name } => {
            let row = Registry::open(&state)?.add_device(&cert, &name)?;
            println!("Added device {} ({})", row.id, row.name);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs `ws device list`. Exit code 2 means the `[hub]` section is missing.
pub async fn device_list(json: bool) -> Result<ExitCode> {
    let Some(client) = client()? else {
        return Ok(no_hub());
    };
    let response = client.request(Method::GET, "/v1/devices").await?;
    if response.status != 200 {
        return Err(format!("The hub answered {}: {}", response.status, response.body).into());
    }
    if json {
        println!("{}", response.body);
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<DeviceRow> = serde_json::from_str(&response.body)?;
    for row in rows {
        let state = if row.revoked_at.is_some() {
            "revoked"
        } else {
            "active"
        };
        println!("{}\t{}\t{}", row.id, row.name, state);
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs `ws device remove`. The hub revokes the device at once.
pub async fn device_remove(id: &str) -> Result<ExitCode> {
    let Some(client) = client()? else {
        return Ok(no_hub());
    };
    let response = client
        .request(Method::DELETE, &format!("/v1/devices/{id}"))
        .await?;
    match response.status {
        204 => println!("Removed device {id}"),
        status => return Err(format!("The hub answered {status}: {}", response.body).into()),
    }
    Ok(ExitCode::SUCCESS)
}

pub(super) fn client() -> Result<Option<HubClient>> {
    let Some(hub) = hub_config()? else {
        return Ok(None);
    };
    Ok(Some(HubClient::new(
        &hub.url, &hub.cert, &hub.key, &hub.ca,
    )?))
}

pub(super) fn hub_config() -> Result<Option<HubConfig>> {
    let state = crate::init::state_path()?;
    Ok(Config::load(&state)?.hub)
}

pub(super) fn no_hub() -> ExitCode {
    eprintln!("ws: Add a [hub] section to config.toml in the state directory.");
    ExitCode::from(2)
}
