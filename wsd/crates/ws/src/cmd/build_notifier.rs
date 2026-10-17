use crate::Result;
use std::{env, fs, path::PathBuf, process::Command};

const ICON_SIZES: [u32; 5] = [16, 32, 128, 256, 512];

/// Copies and brands terminal-notifier for the user's notification settings.
pub fn run() -> Result<()> {
    let source = env::var_os("WS_NOTIFIER_SOURCE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| "/opt/homebrew/opt/terminal-notifier/terminal-notifier.app".into());
    if !source.is_dir() {
        return Err(format!(
            "terminal-notifier.app not found: {} (brew install terminal-notifier)",
            source.display()
        )
        .into());
    }
    let home = env::var_os("WS_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws")))
        .ok_or("Set HOME or WS_HOME to locate the notifier app.")?;
    fs::create_dir_all(&home)?;
    let app = home.join("ws.app");
    if let Ok(destination) = app.canonicalize()
        && source.canonicalize()?.starts_with(destination)
    {
        return Err("The notifier source must be outside the destination app.".into());
    }
    if let Ok(metadata) = fs::symlink_metadata(&app) {
        if metadata.is_dir() && !metadata.is_symlink() {
            fs::remove_dir_all(&app)?;
        } else {
            fs::remove_file(&app)?;
        }
    }
    checked(
        Command::new("cp").arg("-R").arg(&source).arg(&app),
        "Cannot copy the notifier app.",
    )?;
    checked(
        Command::new("chmod").args(["-R", "u+w"]).arg(&app),
        "Cannot update the notifier permissions.",
    )?;
    let plist = app.join("Contents/Info.plist");
    checked(
        Command::new("/usr/libexec/PlistBuddy")
            .args([
                "-c",
                "Set :CFBundleIdentifier sh.ratel.ws.notifier",
                "-c",
                "Set :CFBundleName ws",
            ])
            .arg(&plist),
        &format!("Cannot edit {}.", plist.display()),
    )?;
    let temporary = tempfile::tempdir()?;
    let iconset = temporary.path().join("ws.iconset");
    fs::create_dir(&iconset)?;
    let image = ws_daemon::notify::asset_path("app.png");
    for size in ICON_SIZES {
        for (pixels, suffix) in [(size, ""), (size * 2, "@2x")] {
            checked(
                Command::new("sips")
                    .args(["-z", &pixels.to_string(), &pixels.to_string()])
                    .arg(&image)
                    .arg("--out")
                    .arg(iconset.join(format!("icon_{size}x{size}{suffix}.png"))),
                "Cannot resize the notifier icon.",
            )?;
        }
    }
    checked(
        Command::new("iconutil")
            .args(["-c", "icns"])
            .arg(&iconset)
            .arg("-o")
            .arg(app.join("Contents/Resources/Terminal.icns")),
        "iconutil failed",
    )?;
    if checked(
        Command::new("codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&app),
        "codesign failed",
    )
    .is_err()
    {
        eprintln!("ws: codesign failed (notifications may be blocked)");
    }
    checked(
        Command::new("touch").arg(&app),
        "Cannot update the notifier timestamp.",
    )?;
    println!("built {}", app.display());
    Ok(())
}

fn checked(command: &mut Command, message: &str) -> Result<()> {
    if !command.output()?.status.success() {
        return Err(message.to_owned().into());
    }
    Ok(())
}
