//! Desktop adapters for ordered window state notifications.

use crate::hooks::{self, Callback};
use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, env, io, path::PathBuf, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

/// Guards delivery against a hung adapter. Delivery runs on its own task, so this bound
/// delays no hook reply. It must stay above the process start latency that ordinary
/// scheduling load causes, because a timeout drops the notification.
const ADAPTER_DEADLINE: Duration = Duration::from_secs(10);
type WindowKey = (Option<String>, Option<String>, String);
static DELIVERY_LOCK: tokio::sync::Mutex<BTreeMap<WindowKey, u64>> =
    tokio::sync::Mutex::const_new(BTreeMap::new());
const ENVIRONMENT: &[&str] = &[
    "HOME",
    "PATH",
    "WS_HOME",
    "WS_BIN",
    "WS_WORKSPACE",
    "WS_NOTIFY",
    "WS_NOTIFIER",
    "WS_BORINGNOTCH_SOCKET",
    "WS_TERMINAL_BUNDLE",
    "WS_NOTIFY_SOUND_DONE",
    "WS_NOTIFY_SOUND_WAITING",
    "WS_NOTIFY_ICON_DONE",
    "WS_NOTIFY_ICON_WAITING",
];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config(BTreeMap<String, String>);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Context {
    tmux_socket: Option<String>,
    tmux_path: Option<String>,
}

#[derive(Default)]
pub(crate) struct Target {
    pub window: String,
    pub session: String,
    pub path: PathBuf,
    pub agent: String,
    pub focused: bool,
}

#[derive(Deserialize, Serialize)]
pub struct DismissRequest {
    target: String,
    context: Context,
    config: Config,
}

/// Queues one event after an accepted window state transition.
pub(crate) async fn transition(callback: &Callback, window: &Target, previous: &str, state: &str) {
    let config = callback.notify.clone().unwrap_or_else(Config::from_env);
    if previous == state || config.value("WS_NOTIFY", "1") == "0" {
        return;
    }
    let context = Context {
        tmux_socket: callback.tmux_socket.clone(),
        tmux_path: callback.tmux_path.clone(),
    };
    if let Err(error) = emit(
        &context,
        window,
        previous,
        state,
        config,
        callback.observed_at_us,
        &callback.payload,
    )
    .await
    {
        eprintln!(
            "Cannot queue the {state} notification for {}: {error}",
            window.window
        );
    }
}

pub(crate) fn router() -> Router<crate::api::Store> {
    Router::new().route("/v1/notifications/dismiss", post(dismiss))
}

pub fn asset_path(name: &str) -> PathBuf {
    Config::from_env().asset(name)
}

impl DismissRequest {
    pub fn from_env(target: &str) -> Self {
        Self {
            target: target.into(),
            context: Context {
                tmux_socket: env::var("WS_TMUX_SOCKET")
                    .ok()
                    .filter(|value| !value.is_empty()),
                tmux_path: env::var("TMUX").ok().and_then(|value| {
                    value
                        .split(',')
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                }),
            },
            config: Config::from_env(),
        }
    }
}

async fn dismiss(Json(request): Json<DismissRequest>) -> Json<Value> {
    let seq = hooks::receipt_time_us().unwrap_or(0);
    let _guard = hooks::INGEST_LOCK.lock().await;
    if !request.target.is_empty() && request.config.value("WS_NOTIFY", "1") != "0" {
        let _ = dismiss_open(request, seq).await;
    }
    Json(json!({}))
}

async fn dismiss_open(request: DismissRequest, seq: u64) -> io::Result<()> {
    let snapshot = request
        .context
        .tmux(&[
            "display-message",
            "-p",
            "-t",
            &request.target,
            "#{@ws_notify_open}|#{@ws_state}|#{window_id}\u{1f}#{session_id}\u{1f}#{@ws_path}\u{1f}#{@ws_agent}\u{1f}#{&&:#{window_active},#{session_attached}}",
        ])
        .await?;
    let metadata: Vec<_> = snapshot.split('\u{1f}').collect();
    let fields: Vec<_> = metadata[0].split('|').collect();
    if metadata.len() == 5 && fields.len() == 3 && fields[0] == "1" {
        let target = Target {
            window: fields[2].into(),
            session: metadata[1].into(),
            path: metadata[2].into(),
            agent: metadata[3].into(),
            focused: metadata[4] == "1",
        };
        emit(
            &request.context,
            &target,
            fields[1],
            "closed",
            request.config,
            seq,
            &Value::Null,
        )
        .await?;
    }
    Ok(())
}

impl Context {
    async fn tmux(&self, args: &[&str]) -> io::Result<String> {
        hooks::tmux_server(self.tmux_socket.as_deref(), self.tmux_path.as_deref(), args).await
    }
}

/// Captures notification overrides from the hook process.
impl Config {
    pub fn from_env() -> Self {
        let mut values = ENVIRONMENT
            .iter()
            .filter_map(|key| env::var(key).ok().map(|value| ((*key).into(), value)))
            .collect::<BTreeMap<String, String>>();
        if let Ok(directory) = env::current_dir() {
            values.insert("PWD".into(), directory.to_string_lossy().into_owned());
        }
        Self(values)
    }

    fn value(&self, key: &str, fallback: &str) -> String {
        self.0
            .get(key)
            .filter(|value| !value.is_empty())
            .map(String::as_str)
            .unwrap_or(fallback)
            .into()
    }
}

async fn emit(
    context: &Context,
    target: &Target,
    previous: &str,
    state: &str,
    config: Config,
    seq: u64,
    payload: &Value,
) -> io::Result<()> {
    let window = target.window.as_str();
    let path = &target.path;
    let default_name = format!(
        "{}/{}",
        path.parent()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy(),
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    let name = config.value("WS_WORKSPACE", &default_name);
    let message = match state {
        "waiting" => format!("{name} needs input"),
        "closed" => format!("{name} dismissed"),
        _ => format!("{name} is {state}"),
    };
    let bundle = terminal_bundle(context, &config).await;
    let mut click = shell_words::quote(&executable("tmux", &config).to_string_lossy()).into_owned();
    if let Some(socket) = &context.tmux_socket {
        click.push_str(&format!(" -L {}", shell_words::quote(socket)));
    } else if let Some(path) = &context.tmux_path {
        click.push_str(&format!(" -S {}", shell_words::quote(path)));
    }
    click.push_str(&format!(
        " select-window -t {} \\; switch-client -t {}",
        shell_words::quote(window),
        shell_words::quote(&target.session)
    ));
    if !bundle.is_empty() {
        click.push_str(&format!(" ; open -b {}", shell_words::quote(&bundle)));
    }
    let detail = [
        "message",
        "last_assistant_message",
        "lastAssistantMessage",
        "prompt",
    ]
    .iter()
    .filter_map(|key| payload[*key].as_str())
    .find(|text| !text.is_empty())
    .unwrap_or("")
    .chars()
    .take(1024)
    .collect::<String>();
    let event = json!({"v":1, "id":format!("ws:{window}"), "source":"ws", "state":state,
        "prev":previous, "title":"ws", "message":message, "detail":detail, "agent":target.agent,
        "repo":name.split('/').next().unwrap_or(""), "branch":branch(path, &config).await, "focused":target.focused,
        "sound":match state { "idle"=>"done", "waiting"=>"waiting", _=>"" },
        "actions":[{"id":"focus", "label":"Focus tab", "command":click}],
        "seq":seq, "ts":hooks::receipt_time_us().unwrap_or(seq) / 1_000_000});
    let _ = context
        .tmux(&[
            "set-option",
            "-w",
            "-t",
            window,
            "@ws_notify_open",
            if state == "closed" { "0" } else { "1" },
        ])
        .await;
    let context = context.clone();
    let window = window.to_owned();
    tokio::spawn(async move {
        let mut delivered = DELIVERY_LOCK.lock().await;
        let key = (
            context.tmux_socket.clone(),
            context
                .tmux_path
                .clone()
                .filter(|_| context.tmux_socket.is_none()),
            window.clone(),
        );
        let seq = event["seq"].as_u64().unwrap_or(0);
        if delivered.get(&key).is_some_and(|last| seq <= *last) {
            return;
        }
        delivered.insert(key, seq);
        if let Err(error) = deliver(&context, &window, &event, &config).await {
            eprintln!(
                "Cannot deliver the {} notification for {window}: {error}",
                event["state"]
            );
        }
    });
    Ok(())
}

async fn deliver(
    context: &Context,
    window: &str,
    event: &Value,
    config: &Config,
) -> io::Result<()> {
    let mut adapter = config.value("WS_NOTIFIER", "auto");
    if adapter == "boringnotch" {
        let socket = config.value(
            "WS_BORINGNOTCH_SOCKET",
            &format!(
                "{}/Library/Application Support/boringNotch/notify.sock",
                config.value("HOME", "")
            ),
        );
        if run_input(
            "nc",
            &["-U".into(), "-w".into(), "1".into(), socket],
            config,
            &format!("{event}\n"),
        )
        .await
        .unwrap_or(false)
        {
            return Ok(());
        }
        adapter = "auto".into();
    }
    if !["auto", "terminal-notifier", "osascript"].contains(&adapter.as_str()) {
        if adapter.contains('/') && is_executable(&config.resolve(std::path::Path::new(&adapter))) {
            run_input(&adapter, &[], config, &format!("{event}\n")).await?;
            return Ok(());
        }
        let _ = context
            .tmux(&[
                "display-message",
                &format!("ws: unknown WS_NOTIFIER '{adapter}', using auto"),
            ])
            .await;
        adapter = "auto".into();
    }
    let last = context
        .tmux(&["display-message", "-p", "-t", window, "#{@ws_notify_seq}"])
        .await
        .unwrap_or_default();
    let seq = event["seq"].as_u64().unwrap_or(0);
    if last.parse::<u64>().is_ok_and(|last| seq <= last) {
        return Ok(());
    }
    let _ = context
        .tmux(&[
            "set-option",
            "-w",
            "-t",
            window,
            "@ws_notify_seq",
            &seq.to_string(),
        ])
        .await;
    if event["state"] == "closed" {
        if adapter != "osascript" {
            let _ = run(
                &notifier(config).to_string_lossy(),
                &["-remove".into(), format!("ws-{window}")],
                config,
            )
            .await;
        }
        return Ok(());
    }
    if !matches!(event["state"].as_str(), Some("idle" | "waiting")) || event["focused"] == true {
        return Ok(());
    }
    play_sound(event, config);
    let _ = context
        .tmux(&[
            "display-message",
            &format!("ws: {}", event["message"].as_str().unwrap_or("")),
        ])
        .await;
    if adapter != "osascript"
        && terminal_notify(context, window, event, config)
            .await
            .unwrap_or(false)
    {
        return Ok(());
    }
    run(
        "osascript",
        &[
            "-e".into(),
            "on run argv".into(),
            "-e".into(),
            "display notification (item 1 of argv) with title (item 2 of argv)".into(),
            "-e".into(),
            "end run".into(),
            event["message"].as_str().unwrap_or("").into(),
            "ws".into(),
        ],
        config,
    )
    .await?;
    Ok(())
}

async fn terminal_notify(
    context: &Context,
    window: &str,
    event: &Value,
    config: &Config,
) -> io::Result<bool> {
    let bundle = terminal_bundle(context, config).await;
    let mut args = vec![
        "-title".into(),
        "ws".into(),
        "-message".into(),
        event["message"].as_str().unwrap_or("").into(),
        "-group".into(),
        format!("ws-{window}"),
    ];
    let (key, image) = if event["sound"] == "waiting" {
        ("WS_NOTIFY_ICON_WAITING", "waiting.png")
    } else {
        ("WS_NOTIFY_ICON_DONE", "done.png")
    };
    let image = config.value(key, &config.asset(image).to_string_lossy());
    if config.resolve(std::path::Path::new(&image)).is_file() {
        args.extend(["-contentImage".into(), image]);
    }
    if !bundle.is_empty() {
        args.extend(["-activate".into(), bundle]);
    }
    args.extend([
        "-execute".into(),
        event["actions"][0]["command"].as_str().unwrap_or("").into(),
    ]);
    run(&notifier(config).to_string_lossy(), &args, config).await
}

fn notifier(config: &Config) -> PathBuf {
    let branded = config
        .ws_home()
        .join("ws.app/Contents/MacOS/terminal-notifier");
    if is_executable(&branded) {
        branded
    } else {
        executable("terminal-notifier", config)
    }
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

impl Config {
    fn cwd(&self) -> PathBuf {
        PathBuf::from(self.value("PWD", "."))
    }

    fn resolve(&self, path: &std::path::Path) -> PathBuf {
        self.cwd().join(path)
    }

    fn ws_home(&self) -> PathBuf {
        self.resolve(std::path::Path::new(
            &self.value("WS_HOME", &format!("{}/.ws", self.value("HOME", ""))),
        ))
    }

    fn asset(&self, name: &str) -> PathBuf {
        for binary in [
            PathBuf::from(self.value("WS_BIN", "")),
            self.ws_home().join("bin/ws"),
        ] {
            if let Ok(binary) = binary.canonicalize() {
                for parent in binary.ancestors().skip(1) {
                    let assets = parent.join("assets/ws");
                    if assets.is_dir() {
                        return assets.join(name);
                    }
                }
            }
        }
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/ws");
        assets.canonicalize().unwrap_or(assets).join(name)
    }
}

async fn terminal_bundle(context: &Context, config: &Config) -> String {
    let bundle = config.value("WS_TERMINAL_BUNDLE", "");
    if !bundle.is_empty() {
        return bundle;
    }
    context
        .tmux(&["show-environment", "-g", "__CFBundleIdentifier"])
        .await
        .ok()
        .and_then(|text| text.split_once('=').map(|(_, value)| value.to_owned()))
        .unwrap_or_default()
}

async fn branch(path: &std::path::Path, config: &Config) -> String {
    let mut command = tokio::process::Command::new(executable("git", config));
    let output = tokio::time::timeout(
        Duration::from_millis(100),
        command
            .args(["-C"])
            .arg(path)
            .args(["branch", "--show-current"])
            .envs(&config.0)
            .current_dir(config.cwd())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    match output {
        Ok(Ok(output)) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_owned(),
        _ => String::new(),
    }
}

fn play_sound(event: &Value, config: &Config) {
    let (key, asset) = match event["sound"].as_str() {
        Some("done") => ("WS_NOTIFY_SOUND_DONE", "sounds/done.wav"),
        Some("waiting") => ("WS_NOTIFY_SOUND_WAITING", "sounds/waiting.wav"),
        _ => return,
    };
    let mut sound = PathBuf::from(config.value(key, &config.asset(asset).to_string_lossy()));
    if !config.resolve(&sound).is_file() && !sound.to_string_lossy().contains('/') {
        let name = sound.to_string_lossy().into_owned();
        let directories = [
            PathBuf::from(config.value("HOME", "")).join("Library/Sounds"),
            PathBuf::from("/Library/Sounds"),
            PathBuf::from("/System/Library/Sounds"),
        ];
        let found = directories
            .iter()
            .flat_map(|directory| {
                let mut entries: Vec<_> = std::fs::read_dir(directory)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect();
                entries.sort();
                entries
            })
            .find(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .is_some_and(|file| file.to_string_lossy().starts_with(&format!("{name}.")))
            });
        let Some(found) = found else {
            return;
        };
        sound = found;
    }
    if !config.resolve(&sound).is_file() {
        return;
    }
    let config = config.clone();
    tokio::spawn(async move {
        let _ = tokio::process::Command::new(executable("afplay", &config))
            .arg(sound)
            .envs(&config.0)
            .current_dir(config.cwd())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await;
    });
}

fn executable(program: &str, config: &Config) -> PathBuf {
    if program.contains('/') {
        return config.resolve(std::path::Path::new(program));
    }
    env::split_paths(&config.value("PATH", &env::var("PATH").unwrap_or_default()))
        .map(|directory| config.resolve(&directory).join(program))
        .find(|path| is_executable(path))
        .unwrap_or_else(|| program.into())
}

async fn run(program: &str, args: &[String], config: &Config) -> io::Result<bool> {
    let output = tokio::time::timeout(
        ADAPTER_DEADLINE,
        tokio::process::Command::new(executable(program, config))
            .args(args)
            .envs(&config.0)
            .current_dir(config.cwd())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "The notification adapter exceeded its deadline.",
        )
    })??;
    Ok(output.success())
}

async fn run_input(
    program: &str,
    args: &[String],
    config: &Config,
    input: &str,
) -> io::Result<bool> {
    tokio::time::timeout(ADAPTER_DEADLINE, async {
        let mut child = tokio::process::Command::new(executable(program, config))
            .args(args)
            .envs(&config.0)
            .current_dir(config.cwd())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .await?;
        Ok(child.wait().await?.success())
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "The notification adapter exceeded its deadline.",
        )
    })?
}
