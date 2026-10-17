//! Daemon-owned processes and durable exit records.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    future::Future,
    io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;
use ws_core::{
    ids::{SessionId, Ulid},
    log::SessionLog,
    registry::Session,
    state::SessionState,
};

use crate::{
    agents::{self, AgentSession, Mode},
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};

const TERM_GRACE: Duration = Duration::from_secs(10);
const TIMEOUT_REASON: &str = "The phase exceeded its maximum runtime.";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateHeadless {
    pub headless: bool,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub parent_session_id: Option<SessionId>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub stdin: Option<PathBuf>,
    pub stdout: Option<PathBuf>,
    pub stderr: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Process {
    pid: u32,
    pgid: u32,
    start: Option<String>,
    #[serde(default)]
    deadline_ms: Option<u64>,
}

struct Snapshot {
    pgid: u32,
    start: String,
    alive: bool,
}

pub(crate) async fn create(
    store: Store,
    request: CreateHeadless,
) -> std::result::Result<Session, ApiError> {
    launch(store, request, serde_json::Value::Null).await
}

pub(crate) async fn stop(
    store: Store,
    input: String,
    caller: Option<SessionId>,
) -> std::result::Result<Session, ApiError> {
    let (session, process) = api::call(store.clone(), move |sessions| {
        let session = match sessions.get_record(&input) {
            Ok(session) => session,
            Err(error) => {
                sessions.record_call(caller, "session.stop", Some(&input))?;
                return Err(error);
            }
        };
        if session.body["kind"] != "headless" {
            return sessions.stop(&input, caller).map(|session| (session, None));
        }
        sessions.record_call(caller, "session.stop", Some(&input))?;
        if sessions.process_exited(session.id)? {
            return Ok((session, None));
        }
        let entries = SessionLog::new(&sessions.state, session.id, session.device_id)
            .read()?
            .entries;
        let process = entries
            .iter()
            .rev()
            .find(|entry| entry.event_type == "session.running")
            .and_then(|entry| serde_json::from_value::<Process>(entry.body["process"].clone()).ok())
            .ok_or(Error::InvalidRequest(
                "The headless session has no process record.",
            ))?;
        Ok((session, Some(process)))
    })
    .await?;
    let Some(process) = process else {
        return Ok(session);
    };
    // The stop must continue if the HTTP client disconnects.
    tokio::spawn(async move {
        terminate_group(&process)
            .await
            .map_err(Error::from)
            .map_err(ApiError)?;
        loop {
            let id = session.id.to_string();
            let (current, exited) = api::call(store.clone(), move |sessions| {
                let current = sessions.get_record(&id)?;
                let exited = sessions.process_exited(current.id)?;
                Ok((current, exited))
            })
            .await?;
            if exited {
                return Ok(current);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|error| ApiError(Error::Io(io::Error::other(error))))?
}

pub(crate) async fn launch(
    store: Store,
    request: CreateHeadless,
    metadata: serde_json::Value,
) -> std::result::Result<Session, ApiError> {
    let monitor_store = store.clone();
    api::call(store, move |sessions| {
        let (session, child, process) = sessions.spawn_headless(request, metadata)?;
        let id = session.id;
        let parent = session.body["parent_session_id"]
            .as_str()
            .and_then(|id| id.parse().ok());
        if session.body.get("phase").is_some()
            && let Some(parent) = parent
        {
            let result = sessions.append(
                parent,
                "phase.spawned",
                json!({
                    "parent_session_id": parent, "session_id": id,
                    "phase": session.body["phase"], "repo": session.body["repo"],
                    "base": session.body["base"],
                }),
            );
            if let Err(error) = result {
                let _ = signal_group(child.id().unwrap(), libc::SIGKILL);
                return Err(error);
            }
        }
        let expiry = deadline(process.deadline_ms);
        tokio::spawn(async move {
            let status = wait_child(child, process, expiry).await;
            let result = api::call(monitor_store, move |sessions| {
                let (status, timed_out) = status?;
                let code = status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
                finish(
                    sessions,
                    id,
                    parent,
                    Some(code),
                    status.signal(),
                    timed_out.then_some(TIMEOUT_REASON),
                )?;
                Ok(())
            })
            .await;
            if let Err(error) = result {
                eprintln!("Cannot record the exit for session {id}: {}", error.0);
            }
        });
        Ok(session)
    })
    .await
}

/// Restores supervision before the daemon accepts requests. Adopted processes use liveness polls.
pub(crate) async fn adopt(store: Store) -> std::result::Result<(), ApiError> {
    let monitor_store = store.clone();
    api::call(store, move |sessions| {
        for session in sessions.registry.sessions()? {
            if session.body["kind"] != "headless" || sessions.process_exited(session.id)? {
                continue;
            }
            let parent = session.body["parent_session_id"]
                .as_str()
                .and_then(|id| id.parse().ok());
            let entries = SessionLog::new(&sessions.state, session.id, session.device_id)
                .read()?
                .entries;
            let process = entries
                .iter()
                .rev()
                .find(|entry| entry.event_type == "session.running")
                .and_then(|entry| {
                    serde_json::from_value::<Process>(entry.body["process"].clone()).ok()
                });
            let Some(process) = process else {
                finish(
                    sessions,
                    session.id,
                    parent,
                    None,
                    None,
                    Some("No process record exists after the daemon restart."),
                )?;
                continue;
            };
            if !process.is_alive()? {
                finish(
                    sessions,
                    session.id,
                    parent,
                    None,
                    None,
                    Some("The process is absent or its identity changed after the daemon restart."),
                )?;
                continue;
            }
            sessions.append(
                session.id,
                "process.adopted",
                json!({
                    "parent_session_id": parent, "process": process,
                }),
            )?;
            monitor_adopted(monitor_store.clone(), session.id, parent, process);
        }
        Ok(())
    })
    .await
}

async fn wait_child(
    mut child: tokio::process::Child,
    process: Process,
    expiry: impl Future<Output = ()> + Send,
) -> io::Result<(ExitStatus, bool)> {
    tokio::select! {
        biased;
        status = async {
            let status = child.wait().await?;
            wait_group_gone(process.pgid).await?;
            Ok::<_, io::Error>(status)
        } => Ok((status?, false)),
        _ = expiry => {
            let (_, status) = tokio::try_join!(terminate_group(&process), child.wait())?;
            Ok((status, true))
        }
    }
}

fn deadline(at_ms: Option<u64>) -> impl Future<Output = ()> + Send {
    // Anchor the monotonic timer before the monitor task can be delayed by scheduling.
    let at = at_ms.map(|at_ms| {
        tokio::time::Instant::now() + Duration::from_millis(at_ms.saturating_sub(unix_ms()))
    });
    async move {
        match at {
            Some(at) => tokio::time::sleep_until(at).await,
            None => std::future::pending().await,
        }
    }
}

async fn terminate_group(process: &Process) -> io::Result<()> {
    // A living leader must still match the persisted birth identity.
    if let Some(current) = snapshot(process.pid)?
        && (current.pgid != process.pgid || process.start.as_ref() != Some(&current.start))
    {
        return Err(io::Error::other(
            "The process identity changed. The group was not signalled.",
        ));
    }
    let grace_deadline = tokio::time::Instant::now() + TERM_GRACE;
    signal_group(process.pgid, libc::SIGTERM)?;
    tokio::select! {
        result = wait_group_gone(process.pgid) => return result,
        _ = tokio::time::sleep_until(grace_deadline) => {}
    }
    signal_group(process.pgid, libc::SIGKILL)?;
    wait_group_gone(process.pgid).await
}

async fn wait_group_gone(pgid: u32) -> io::Result<()> {
    while group_exists(pgid)? {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

fn group_exists(pgid: u32) -> io::Result<bool> {
    if pgid <= 1 || pgid > i32::MAX as u32 {
        return Err(io::Error::other("The process group id is invalid."));
    }
    #[cfg(target_os = "macos")]
    {
        // kill(-pgid, 0) can return EPERM for a group during process teardown.
        // Inspect group membership instead. One pid is enough to prove membership.
        let mut pid: libc::pid_t = 0;
        // SAFETY: The buffer is valid and has the specified size.
        let count = unsafe {
            libc::proc_listpgrppids(
                pgid as i32,
                (&mut pid as *mut libc::pid_t).cast(),
                std::mem::size_of_val(&pid) as i32,
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(count > 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // SAFETY: A negative, validated pid checks only this child's process group.
        if unsafe { libc::kill(-(pgid as i32), 0) } == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(false)
        } else {
            Err(error)
        }
    }
}

fn monitor_adopted(store: Store, id: SessionId, parent: Option<SessionId>, process: Process) {
    let expiry = deadline(process.deadline_ms);
    tokio::spawn(async move {
        tokio::pin!(expiry);
        loop {
            let timed_out = tokio::select! {
                _ = &mut expiry => true,
                _ = tokio::time::sleep(Duration::from_millis(200)) => false,
            };
            if timed_out && let Err(error) = terminate_group(&process).await {
                eprintln!("Cannot stop session {id}: {error}");
                return;
            }
            let process = process.clone();
            let result = api::call(store.clone(), move |sessions| {
                if !timed_out && process.is_alive()? {
                    return Ok(true);
                }
                finish(
                    sessions,
                    id,
                    parent,
                    None,
                    None,
                    Some(if timed_out {
                        TIMEOUT_REASON
                    } else {
                        "The adopted process exited. Its exit status is unavailable."
                    }),
                )?;
                Ok(false)
            })
            .await;
            match result {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    eprintln!("Cannot monitor session {id}: {}", error.0);
                    return;
                }
            }
        }
    });
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn finish(
    sessions: &mut Sessions,
    id: SessionId,
    parent: Option<SessionId>,
    code: Option<i32>,
    signal: Option<i32>,
    reason: Option<&str>,
) -> Result<()> {
    let session = sessions.get_record(&id.to_string())?;
    if sessions.process_exited(id)? {
        return Ok(());
    }
    let state = if code == Some(0) && reason.is_none() {
        SessionState::Finished
    } else {
        SessionState::Failed
    };
    sessions.append(
        id,
        "exit",
        json!({
            "code": code, "signal": signal, "reason": reason, "parent_session_id": parent,
        }),
    )?;
    if ["finished", "failed"].contains(&session.state.as_str()) {
        return Ok(());
    }
    sessions.append(
        id,
        state.event_type(),
        json!({
            "parent_session_id": parent, "reason": reason,
        }),
    )
}

impl Process {
    fn is_alive(&self) -> io::Result<bool> {
        if self.pid <= 1 || self.pgid != self.pid || self.pid > i32::MAX as u32 {
            return Ok(false);
        }
        Ok(snapshot(self.pid)?.is_some_and(|current| {
            current.alive
                && current.pgid == self.pgid
                && self.start.as_ref() == Some(&current.start)
        }))
    }
}

impl Sessions {
    pub(crate) fn process_exited(&self, id: SessionId) -> Result<bool> {
        let Some(exit) = self.registry.latest_process_exit(id)? else {
            return Ok(false);
        };
        let running = self.registry.latest_event(id, "session.running")?;
        Ok(running.is_none_or(|running| exit.seq > running.seq))
    }

    fn spawn_headless(
        &mut self,
        request: CreateHeadless,
        metadata: serde_json::Value,
    ) -> Result<(Session, tokio::process::Child, Process)> {
        if !request.headless || request.command.first().is_none_or(|s| s.is_empty()) {
            return Err(Error::InvalidRequest(
                "Specify a command for the headless session.",
            ));
        }
        for (key, value) in &request.env {
            if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
                return Err(Error::InvalidRequest(
                    "Each environment key must be nonempty and contain no equals sign or NUL.",
                ));
            }
        }
        let deadline_ms = metadata
            .get("max_runtime")
            .and_then(|value| value.as_u64())
            .map(|seconds| {
                let duration = Duration::from_secs(seconds);
                if tokio::time::Instant::now().checked_add(duration).is_none() {
                    return Err(Error::InvalidRequest("The maximum runtime is too large."));
                }
                seconds
                    .checked_mul(1000)
                    .and_then(|ms| unix_ms().checked_add(ms))
                    .ok_or(Error::InvalidRequest("The maximum runtime is too large."))
            })
            .transpose()?;
        let cwd = request.cwd.canonicalize()?;
        if !cwd.is_dir() {
            return Err(Error::InvalidRequest(
                "The working directory is not a directory.",
            ));
        }
        let id = Ulid::new();
        self.record_call(
            request.parent_session_id,
            if metadata.get("phase").is_some() {
                "phase.spawn"
            } else {
                "session.create"
            },
            Some(&id.to_string()),
        )?;
        let directory = self
            .state
            .canonicalize()?
            .join("sessions")
            .join(id.to_string());
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        let mut command = headless_command(&request, id)?;
        let stdout = request
            .stdout
            .unwrap_or_else(|| directory.join("stdout.log"));
        let stderr = request
            .stderr
            .unwrap_or_else(|| directory.join("stderr.log"));
        let input = request.stdin.map(File::open).transpose()?;
        let output = output_file(&stdout)?;
        let errors = output_file(&stderr)?;
        if same_file(&output, &errors)?
            || input
                .as_ref()
                .map(|input| same_file(input, &output))
                .transpose()?
                .unwrap_or(false)
            || input
                .as_ref()
                .map(|input| same_file(input, &errors))
                .transpose()?
                .unwrap_or(false)
        {
            return Err(Error::InvalidRequest(
                "Use separate files for stdin, stdout and stderr.",
            ));
        }
        output.set_len(0)?;
        errors.set_len(0)?;
        let mut body = json!({
            "alias": self.registry.next_alias()?, "kind": "headless",
            "command": request.command, "cwd": cwd, "stdout": stdout, "stderr": stderr,
            "parent_session_id": request.parent_session_id,
        });
        if let Some(metadata) = metadata.as_object() {
            body.as_object_mut().unwrap().extend(metadata.clone());
        }
        self.append(id, "session.created", body)?;
        command
            .current_dir(cwd)
            .envs(&request.env)
            .env("WS_SESSION_ID", id.to_string())
            .env("WS_SESSION", id.to_string())
            .stdin(input.map(Stdio::from).unwrap_or_else(Stdio::null))
            .stdout(output)
            .stderr(errors)
            .process_group(0)
            .kill_on_drop(false);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.transition(
                    &id.to_string(),
                    SessionState::Failed,
                    request.parent_session_id,
                )?;
                return Err(error.into());
            }
        };
        let pid = child.id().expect("A newly spawned child has a pid.");
        let persisted = (|| {
            let process = Process {
                pid,
                pgid: pid,
                start: snapshot(pid)?.map(|snapshot| snapshot.start),
                deadline_ms,
            };
            self.append(
                id,
                "session.running",
                json!({
                    "parent_session_id": request.parent_session_id, "process": process,
                }),
            )?;
            Ok((self.get_record(&id.to_string())?, process))
        })();
        match persisted {
            Ok((session, process)) => Ok((session, child, process)),
            Err(error) => {
                let _ = signal_group(pid, libc::SIGKILL);
                Err(error)
            }
        }
    }
}

fn headless_command(request: &CreateHeadless, id: SessionId) -> io::Result<Command> {
    let mut command = Command::new(&request.command[0]);
    let agent = Path::new(&request.command[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if ["claude", "codex", "grok"].contains(&agent) {
        let environment = |key: &str| {
            request
                .env
                .get(key)
                .map(std::ffi::OsString::from)
                .or_else(|| std::env::var_os(key))
        };
        let home =
            PathBuf::from(environment("HOME").ok_or_else(|| {
                io::Error::other("Set HOME to locate agent configuration files.")
            })?);
        let mut arguments = request.command[1..].to_vec();
        if arguments.first().is_some_and(|argument| match agent {
            "claude" => ["-p", "--print"].contains(&argument.as_str()),
            "codex" => argument == "exec",
            _ => false,
        }) {
            arguments.remove(0);
        }
        let context = AgentSession {
            id: id.to_string(),
            ws_home: environment("WS_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".ws")),
            grok_home: environment("GROK_HOME").map(PathBuf::from),
            home,
            ws_bin: std::env::current_exe()?,
            spawned: false,
            prompt: None,
            arguments,
        };
        let prepared = agents::prepare(agent, Mode::Headless, &context, None)?;
        prepared.write_files()?;
        command.args(prepared.argv).envs(prepared.env);
    } else {
        command.args(&request.command[1..]);
    }
    Ok(command)
}

fn output_file(path: &PathBuf) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)
}

fn same_file(left: &File, right: &File) -> io::Result<bool> {
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

fn signal_group(pgid: u32, signal: i32) -> io::Result<()> {
    if pgid <= 1 || pgid > i32::MAX as u32 {
        return Err(io::Error::other("The process group id is invalid."));
    }
    // SAFETY: A negative, validated pid addresses only this child's process group.
    if unsafe { libc::kill(-(pgid as i32), signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(target_os = "macos")]
fn snapshot(pid: u32) -> io::Result<Option<Snapshot>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: The buffer has the size and alignment required by PROC_PIDTBSDINFO.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read == size {
        // SAFETY: proc_pidinfo initialized the complete buffer.
        let info = unsafe { info.assume_init() };
        return Ok(Some(Snapshot {
            pgid: info.pbi_pgid,
            start: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
            alive: info.pbi_status != libc::SZOMB,
        }));
    }
    let error = io::Error::last_os_error();
    if [Some(libc::ESRCH), Some(libc::ENOENT)].contains(&error.raw_os_error()) {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
fn snapshot(pid: u32) -> io::Result<Option<Snapshot>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    // The command name can contain spaces and parentheses. Field 3 follows the last ')'.
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("The process record is invalid."))?
        .1
        .split_whitespace()
        .collect();
    let pgid = fields
        .get(2)
        .and_then(|field| field.parse().ok())
        .ok_or_else(|| io::Error::other("The process group is absent."))?;
    let start = fields
        .get(19)
        .ok_or_else(|| io::Error::other("The process start time is absent."))?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(Some(Snapshot {
        pgid,
        start: format!("{}:{start}", boot.trim()),
        alive: fields.first() != Some(&"Z"),
    }))
}
