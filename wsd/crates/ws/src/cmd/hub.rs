use std::{io::Write, net::SocketAddr, path::PathBuf, process::ExitCode};

use clap::Subcommand;
use hyper::Method;
use ws_core::config::{Config, HubConfig};
use ws_hub::{
    client::HubClient,
    registry::{DeviceRow, Registry},
    server::{self, HubOptions},
    tokens::{AuditRow, TokenRow},
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
    /// Manage the bearer tokens of the remote MCP endpoint.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Show the audit rows of the remote MCP calls.
    Audit {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Create a read token. The command prints the token one time.
    Create {
        #[arg(long)]
        name: String,
    },
    /// List the tokens. The list never shows a token.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Revoke the token with this name at once.
    Revoke { name: String },
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
        HubCommand::Token { command } => return token(command).await,
        HubCommand::Audit { json } => return audit(json).await,
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs `ws hub token`. Exit code 2 means the `[hub]` section is missing.
async fn token(command: TokenCommand) -> Result<ExitCode> {
    let Some(client) = client()? else {
        return Ok(no_hub());
    };
    match command {
        TokenCommand::Create { name } => {
            let body = serde_json::json!({ "name": name }).to_string();
            let response = client
                .request_json(Method::POST, "/v1/tokens", &body)
                .await?;
            if response.status != 201 {
                return Err(hub_error(&response));
            }
            let created: serde_json::Value = serde_json::from_str(&response.body)?;
            println!("{}", created["token"].as_str().unwrap_or_default());
            eprintln!("ws: The hub shows this token one time. Store it now.");
        }
        TokenCommand::List { json } => {
            let response = client.request(Method::GET, "/v1/tokens").await?;
            if response.status != 200 {
                return Err(hub_error(&response));
            }
            if json {
                println!("{}", response.body);
                return Ok(ExitCode::SUCCESS);
            }
            let rows: Vec<TokenRow> = serde_json::from_str(&response.body)?;
            for row in rows {
                let state = if row.revoked_at.is_some() {
                    "revoked"
                } else {
                    "active"
                };
                println!("{}\t{}\t{}", row.name, row.scope, state);
            }
        }
        TokenCommand::Revoke { name } => {
            let response = client
                .request(Method::DELETE, &format!("/v1/tokens/{name}"))
                .await?;
            if response.status != 204 {
                return Err(hub_error(&response));
            }
            println!("Revoked token {name}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs `ws hub audit`.
async fn audit(json: bool) -> Result<ExitCode> {
    let Some(client) = client()? else {
        return Ok(no_hub());
    };
    let response = client.request(Method::GET, "/v1/audit").await?;
    if response.status != 200 {
        return Err(hub_error(&response));
    }
    if json {
        println!("{}", response.body);
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<AuditRow> = serde_json::from_str(&response.body)?;
    for row in rows {
        println!("{}\t{}\t{}\t{}", row.ts_ms, row.token, row.tool, row.result);
    }
    Ok(ExitCode::SUCCESS)
}

fn hub_error(response: &ws_hub::client::HubResponse) -> Box<dyn std::error::Error + Send + Sync> {
    format!("The hub answered {}: {}", response.status, response.body).into()
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
