use crate::{Result, client};
use hyper::Method;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub async fn run(socket: &Path, backend: &str) -> Result<()> {
    if backend != "rust" {
        return Err("Only the rust backend is available.".into());
    }
    let home = ws_daemon::import::backend::home()?;
    fs::create_dir_all(&home)?;
    let root = env::var_os("WS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map_or_else(|| home.join("worktrees"), |home| home.join(".ws/worktrees"))
        });
    let lock = ws_core::lockfile::open_lock_file(&home.join("backend.lock"))?;
    lock.lock()?;
    client::request(
        socket,
        Method::POST,
        "/v1/backend",
        serde_json::to_vec(&serde_json::json!({"backend":backend, "home":home, "root":root}))?,
    )
    .await?;
    let temporary = home.join(format!(".backend-{}", ws_core::ids::Ulid::new()));
    fs::write(&temporary, format!("{backend}\n"))?;
    fs::File::open(&temporary)?.sync_all()?;
    fs::rename(&temporary, home.join("backend"))?;
    fs::File::open(&home)?.sync_all()?;
    println!("ws backend: {backend}");
    Ok(())
}
