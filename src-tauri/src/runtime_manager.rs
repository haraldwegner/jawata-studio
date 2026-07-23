use crate::config::{display_path, AppPaths};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// Sprint 16.1 (bugs.md #16): suppress the console window Windows allocates
/// when a GUI app spawns `java.exe`. `CREATE_NO_WINDOW` (0x08000000) keeps
/// the child windowless while leaving stdio pipes intact. No-op off Windows.
pub(crate) fn spawn_without_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

// ---------------------------------------------------------------------------
// Sprint 27a Stage 10 (studio #1): adopt/probe/kill primitives. Studio persists
// intent but not OWNERSHIP, so after any restart in which residents survive the
// two disagree. These free functions let the manager recover ownership from the
// listening port + the running process, with no new persistence: a healthy
// jawata is identified by answering on its port with the expected token, and its
// PID is discoverable from the port.
// ---------------------------------------------------------------------------

/// Does a healthy jawata resident already answer on this port with this token?
/// Identifies OUR resident so we can adopt it (issue #1). Two guards, so a
/// foreign service or a wrong token can never be mistaken for ours:
///   - jawata's HTTP transport returns 401 on a missing/wrong Bearer token, so
///     a non-2xx status fails closed;
///   - a 2xx alone is not enough (any service could 200 a POST to /mcp), so the
///     body must be a JSON-RPC success — a `result`, no `error` — from
///     `health_check`.
/// Timeout is short: this is localhost, and a real resident answers in ms; the
/// probe runs on the dashboard poll, so it must not stall it.
pub(crate) fn resident_answers(port: u16, token: &str) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "health_check", "arguments": {} }
    });
    let response = match client
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
    {
        Ok(response) => response,
        Err(_) => return false,
    };
    if !response.status().is_success() {
        return false; // 401 wrong/missing token, or anything non-2xx
    }
    match response.json::<serde_json::Value>() {
        Ok(value) => value.get("result").is_some() && value.get("error").is_none(),
        Err(_) => false,
    }
}

/// Is the process still alive? Unix: `kill -0` (signal 0 tests existence without
/// touching the process). Windows: a `tasklist` filter. A discovered orphan that
/// has since died must NOT be reported as a live-but-unmanaged runtime.
pub(crate) fn process_alive(pid: u32) -> bool {
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

/// Forcefully kill a process by PID — used to stop a resident this Studio does
/// not own a `Child` for (an adopted orphan). Never silently succeeds having
/// done nothing: a failure is reported (issue #1, step 2 — "Stop must act or
/// report honestly").
pub(crate) fn kill_pid(pid: u32) -> Result<(), String> {
    #[cfg(not(windows))]
    let result = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    #[cfg(windows)]
    let result = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .status();
    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("kill of pid {pid} exited with {status}")),
        Err(error) => Err(format!("failed to kill pid {pid}: {error}")),
    }
}

/// The PID listening on a local TCP port, discovered from the OS (Linux/macOS
/// `ss`/`lsof`). Best-effort — `None` when no tool is available or nothing
/// listens. Stored at adopt time so an adopted resident can later be stopped.
pub(crate) fn pid_listening_on(port: u16) -> Option<u32> {
    #[cfg(not(windows))]
    {
        // Prefer `ss` (Linux, always present); its `pid=<N>` field is stable.
        if let Ok(output) = Command::new("ss").args(["-ltnpH"]).output() {
            let text = String::from_utf8_lossy(&output.stdout);
            let needle = format!(":{port} ");
            for line in text.lines() {
                if line.contains(&needle) {
                    if let Some(pid) = parse_ss_pid(line) {
                        return Some(pid);
                    }
                }
            }
        }
        // Fall back to `lsof` (macOS default).
        if let Ok(output) = Command::new("lsof")
            .args(["-nP", "-sTCP:LISTEN", "-t", &format!("-iTCP:{port}")])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = text.split_whitespace().next() {
                if let Ok(pid) = first.parse::<u32>() {
                    return Some(pid);
                }
            }
        }
        None
    }
    #[cfg(windows)]
    {
        let _ = port;
        None
    }
}

/// Extract `pid=<N>` from an `ss -ltnpH` line
/// (`... users:(("java",pid=12345,fd=7))`).
fn parse_ss_pid(line: &str) -> Option<u32> {
    let start = line.find("pid=")? + "pid=".len();
    let rest = &line[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse::<u32>().ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimePhase {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Status record for a project's runtime. Sprint 10 v0.10.4: multiple
/// projects sharing a `workspace_name` reflect the same underlying jawata
/// process — same PID, same workspace_dir, same log file. They differ only
/// in `project_id`. The frontend continues to read these per-project for
/// rendering, but the underlying process is shared.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatusRecord {
    pub project_id: String,
    pub phase: RuntimePhase,
    /// Sprint 10 v0.10.4: the logical workspace this project belongs to.
    /// All projects sharing this name run as one MCP service.
    pub workspace_name: String,
    pub transport: String,
    pub pid: Option<u32>,
    pub workspace_dir: String,
    pub log_path: String,
    pub runtime_label: String,
    pub resolved_jar_path: String,
    pub service_mode: String,
    pub detail: String,
    /// Sprint 14 (v0.14.0, bugs.md #2): captured from the dead-process
    /// branch of `try_join_running_workspace*` when the child exited
    /// without the manager initiating the stop. None for healthy or
    /// cleanly-stopped runtimes. `#[serde(default)]` lets older
    /// runtime-state.json files (pre-v0.14.0) deserialize cleanly.
    #[serde(default)]
    pub exit_code: Option<i32>,
}

/// Sprint 12 (v0.12.0): one entry per workspace_name, with a phase
/// aggregated from the workspace's member projects. Used by the system-tray
/// menu to render per-workspace status icons and toggle entries.
///
/// Aggregation rules (applied in order):
///   any project Failed   → Failed
///   else any Starting    → Starting
///   else all Running     → Running
///   else (Stopped/empty) → Stopped
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatusSummary {
    pub workspace_name: String,
    pub phase: RuntimePhase,
    pub project_count: usize,
}

impl RuntimeStatusRecord {
    pub fn unresolved(
        project_id: String,
        workspace_name: String,
        workspace_dir: String,
        runtime_label: String,
        detail: String,
    ) -> Self {
        Self {
            phase: RuntimePhase::Failed,
            workspace_name,
            transport: "http".into(),
            pid: None,
            log_path: String::new(),
            resolved_jar_path: String::new(),
            service_mode: "manager-process".into(),
            project_id,
            workspace_dir,
            runtime_label,
            detail,
            exit_code: None,
        }
    }
}

/// Reference to a project's runtime — identifies which workspace process
/// the project belongs to. Built by manager_service from a `ProjectRecord`
/// + the resolved jawata runtime location.
#[derive(Debug, Clone)]
pub struct RuntimeReference {
    pub project_id: String,
    pub workspace_name: String,
    /// Eclipse `-data` directory for the workspace's jawata process.
    /// Lives at `<data_root>/workspaces/<workspace_name>/`. Manager_service
    /// writes `workspace.json` into here before spawn.
    pub workspace_dir: String,
    pub runtime_label: String,
    pub resolved_jar_path: String,
    /// Sprint 21a (item F): `-D` system properties handed to the resident JVM
    /// (knowledge-store mode, memory roots, crawl caps). MUST precede `-jar`.
    pub jvm_properties: Vec<String>,
    /// Sprint 15 Stage 10: bind port for the resident-JVM HTTP transport.
    /// Allocated once per workspace via
    /// `ConfigStore::get_or_allocate_workspace_state` and stable across
    /// manager restarts.
    pub resident_port: u16,
    /// Sprint 15 Stage 10: Bearer token the resident JVM accepts and the
    /// manager-deployed MCP-config writes into client `Authorization`
    /// headers (Stage 11). Allocated alongside `resident_port`.
    pub resident_token: String,
}

/// Launch request for one jawata spawn. Manager_service has already
/// written `<workspace_dir>/workspace.json` with the full project list of
/// the workspace before calling `start_runtime`.
#[derive(Debug, Clone)]
pub struct RuntimeLaunchRequest {
    pub project_path: String,
    pub reference: RuntimeReference,
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub log_path: String,
}

/// One jawata process owned by the manager. Sprint 10 v0.10.4: shared
/// across every project whose `workspace_name` matches this entry's key.
/// Whether a managed runtime is still up, and if not, its exit code (Owned
/// only — an adopted orphan has no captured exit status, hence `None`).
enum Liveness {
    Alive,
    Dead { code: Option<i32> },
}

/// The process behind a managed runtime. Sprint 27a Stage 10 (studio #1):
/// Studio used to hold only a `Child`, so a resident that outlived Studio
/// could never be recovered. `Adopted` records a resident THIS Studio did not
/// spawn — reached by the port it answers on and killable by its discovered
/// PID — so ownership survives a Studio restart.
enum RuntimeProcess {
    Owned(Child),
    /// A resident this Studio did not spawn. `port`/`token` are kept so the
    /// identity can be RE-VERIFIED at kill time (a PID alone can be recycled to
    /// an unrelated process — killing it would be a `kill -9` on a stranger).
    Adopted { pid: u32, port: u16, token: String },
}

impl RuntimeProcess {
    fn id(&self) -> u32 {
        match self {
            RuntimeProcess::Owned(child) => child.id(),
            RuntimeProcess::Adopted { pid, .. } => *pid,
        }
    }

    /// Is the process still alive? Owned: reap via `try_wait`. Adopted: probe
    /// the PID (`kill -0`), since we hold no `Child` to wait on.
    fn liveness(&mut self) -> Result<Liveness, String> {
        match self {
            RuntimeProcess::Owned(child) => child
                .try_wait()
                .map(|opt| match opt {
                    Some(status) => Liveness::Dead { code: status.code() },
                    None => Liveness::Alive,
                })
                .map_err(|error| format!("failed to inspect JAWATA process state: {error}")),
            RuntimeProcess::Adopted { pid, .. } => Ok(if process_alive(*pid) {
                Liveness::Alive
            } else {
                Liveness::Dead { code: None }
            }),
        }
    }

    /// Kill the process and reap it. Owned: `Child::kill` + `wait`. Adopted:
    /// re-verify the resident still answers on its port with its token (so a
    /// recycled PID is never killed), then kill by PID — and REPORT a failure
    /// rather than silently succeeding (issue #1, "Stop must act or report
    /// honestly"). If it no longer answers it is already gone, so reporting
    /// stopped is honest and killing the (possibly reused) PID is refused.
    fn kill_and_reap(&mut self) -> Result<(), String> {
        match self {
            RuntimeProcess::Owned(child) => {
                child
                    .kill()
                    .map_err(|error| format!("failed to stop JAWATA process: {error}"))?;
                let _ = child.wait();
                Ok(())
            }
            RuntimeProcess::Adopted { pid, port, token } => {
                // Kill only if OUR pid is still the process listening on the port
                // AND it answers our token. This refuses two dangers: a recycled
                // pid (stored pid no longer the listener), and a resident that
                // relaunched on the same port with a NEW pid (the listener's pid
                // would differ from ours). If either holds, the thing we adopted
                // is gone — report stopped, never kill a stranger (F4).
                if pid_listening_on(*port) == Some(*pid) && resident_answers(*port, token) {
                    kill_pid(*pid)
                } else {
                    Ok(())
                }
            }
        }
    }
}

struct ManagedRuntime {
    process: RuntimeProcess,
    started_at: Instant,
    log_path: String,
    /// Project IDs whose `workspace_name` made them members of this
    /// process. The process is killed when the last member leaves.
    members: HashSet<String>,
    /// Snapshot of the reference used to start the process — re-applied
    /// to per-project status records when other members query status.
    workspace_dir: String,
    runtime_label: String,
    resolved_jar_path: String,
    /// Sprint 15 Stage 10: per-workspace HTTP port the fork is bound to
    /// (`-port <N>`). Carried so status records expose the URL the
    /// manager-deployed MCP clients connect to.
    resident_port: u16,
    /// Sprint 15 Stage 10: flipped to true by the stdout-capture thread
    /// the moment the fork emits its `READY url=... token=...` line.
    /// Phase transitions consult this to mark Running on a real readiness
    /// signal rather than the legacy 2s-elapsed heuristic.
    ready: Arc<AtomicBool>,
}

pub struct RuntimeManager {
    paths: AppPaths,
    /// Sprint 10 v0.10.4: keyed by `workspace_name`, not `project_id`.
    handles: Mutex<HashMap<String, ManagedRuntime>>,
    /// Per-project snapshot cache. Multiple snapshots may point at the
    /// same `workspace_name` and reflect the same workspace process.
    snapshots: Mutex<HashMap<String, RuntimeStatusRecord>>,
}

impl RuntimeManager {
    pub fn new(paths: AppPaths) -> Self {
        let snapshots = read_runtime_state(&paths.runtime_state_file).unwrap_or_default();
        Self {
            paths,
            handles: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(snapshots),
        }
    }

    /// Start (or join) the workspace's runtime for `launch_request.reference`.
    /// If the workspace's process is already running, this just adds the
    /// project as a member and returns the workspace's status. Otherwise
    /// spawns jawata. Caller (manager_service) must have written
    /// `workspace.json` into `workspace_dir` before calling.
    pub fn start_runtime(
        &self,
        launch_request: &RuntimeLaunchRequest,
    ) -> Result<RuntimeStatusRecord, String> {
        let spec = self.command_spec_for(launch_request);
        self.start_runtime_with_spec(&launch_request.reference, spec)
    }

    /// Internal entry point that takes the already-built `CommandSpec`.
    /// Public-in-crate so unit tests can spawn a tiny stand-in command
    /// (e.g. `sleep`) instead of `java -jar jawata.jar` to verify the
    /// workspace-grouped membership lifecycle without depending on a real
    /// jawata runtime.
    pub(crate) fn start_runtime_with_spec(
        &self,
        reference: &RuntimeReference,
        spec: CommandSpec,
    ) -> Result<RuntimeStatusRecord, String> {
        // Fast path: workspace already running. Add membership, return
        // workspace's PID as this project's status.
        if let Some(status) = self.try_join_running_workspace(reference)? {
            return Ok(status);
        }

        // studio #1: before spawning, ADOPT a resident that outlived this
        // Studio. Spawning a second copy on a port an orphan still holds makes
        // the new resident exit 0 (jawata-mcp#2) → phase Failed → an autostart
        // retry loop. Probing the port first breaks that loop at its root.
        if let Some(status) = self.try_adopt(reference)? {
            return Ok(status);
        }

        self.paths.ensure_dirs()?;
        fs::create_dir_all(&reference.workspace_dir).map_err(|error| {
            format!(
                "failed to create workspace dir {}: {error}",
                reference.workspace_dir
            )
        })?;

        let log_path = spec.log_path.clone();
        let stderr_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| format!("failed to open {log_path}: {error}"))?;

        let mut command = Command::new(&spec.command);
        command.args(&spec.args);
        spawn_without_console(&mut command);
        command.stdin(Stdio::piped());
        // Sprint 15 Stage 10: pipe stdout (was direct-to-file) so the
        // capture thread can both tee to the log AND watch for the
        // `READY url=... token=...` line the fork v1.8.5 emits when its
        // HTTP listener is bound. Stderr stays piped directly to the log.
        command.stdout(Stdio::piped());
        command.stderr(Stdio::from(stderr_file));

        for (key, value) in &spec.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to launch JAWATA. Confirm Java and the resolved runtime path are valid: {error}"
            )
        })?;

        let pid = child.id();
        let ready = Arc::new(AtomicBool::new(false));

        // Sprint 15 Stage 10: stdout-capture thread. Tees each line to the
        // log file AND watches for the READY contract. The thread exits on
        // EOF (when the child exits or closes its stdout). Errors writing
        // to the log are best-effort — they shouldn't tear down the
        // workspace just because the log file rotated.
        if let Some(stdout) = child.stdout.take() {
            let log_path_for_thread = log_path.clone();
            let ready_flag = Arc::clone(&ready);
            std::thread::Builder::new()
                .name(format!("runtime-stdout:{}", reference.workspace_name))
                .spawn(move || {
                    let mut log_file = match OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path_for_thread)
                    {
                        Ok(f) => f,
                        Err(_) => return, // log open failed; nothing to tee to
                    };
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = writeln!(log_file, "{}", line);
                        // The fork emits exactly one READY line of the form
                        // `READY url=http://<bind>:<port> token=<token>`
                        // when its HTTP listener is bound. First occurrence
                        // wins; subsequent matches are no-ops.
                        if line.starts_with("READY url=") {
                            ready_flag.store(true, Ordering::Release);
                        }
                    }
                })
                .map_err(|error| format!("failed to spawn stdout-capture thread: {error}"))?;
        }

        let status = RuntimeStatusRecord {
            project_id: reference.project_id.clone(),
            phase: RuntimePhase::Starting,
            workspace_name: reference.workspace_name.clone(),
            transport: "http".into(),
            pid: Some(pid),
            workspace_dir: reference.workspace_dir.clone(),
            log_path: log_path.clone(),
            runtime_label: reference.runtime_label.clone(),
            resolved_jar_path: reference.resolved_jar_path.clone(),
            service_mode: "manager-process".into(),
            detail: "Process launched. Waiting for fork READY line on stdout.".into(),
            exit_code: None,
        };

        let mut members = HashSet::new();
        members.insert(reference.project_id.clone());
        self.handles.lock().expect("runtime mutex poisoned").insert(
            reference.workspace_name.clone(),
            ManagedRuntime {
                process: RuntimeProcess::Owned(child),
                started_at: Instant::now(),
                log_path,
                members,
                workspace_dir: reference.workspace_dir.clone(),
                runtime_label: reference.runtime_label.clone(),
                resolved_jar_path: reference.resolved_jar_path.clone(),
                resident_port: reference.resident_port,
                ready,
            },
        );
        self.persist_snapshot(status.clone())?;

        Ok(status)
    }

    /// Test helper: returns the membership snapshot for a workspace, or
    /// None if no process is registered for that workspace. Used by unit
    /// tests to assert join/leave semantics without poking at private
    /// state. `pub(crate)` to keep it out of the public API.
    #[cfg(test)]
    pub(crate) fn workspace_members(&self, workspace_name: &str) -> Option<Vec<String>> {
        let handles = self.handles.lock().expect("runtime mutex poisoned");
        handles
            .get(workspace_name)
            .map(|h| h.members.iter().cloned().collect())
    }

    /// Test helper: returns the PID of the workspace's process, or None.
    #[cfg(test)]
    pub(crate) fn workspace_pid(&self, workspace_name: &str) -> Option<u32> {
        let handles = self.handles.lock().expect("runtime mutex poisoned");
        handles.get(workspace_name).map(|h| h.process.id())
    }

    /// Project leaves the workspace. If the workspace's process has no
    /// remaining members, it is killed. The caller (manager_service) is
    /// responsible for rewriting `workspace.json` so the still-running
    /// jawata (when there are remaining members) drops the leaving
    /// project from its in-memory state via the file watcher.
    pub fn stop_runtime(
        &self,
        reference: &RuntimeReference,
    ) -> Result<RuntimeStatusRecord, String> {
        // studio #1 (F1): a stop must ACT even when the handle map is cold — the
        // tray "Stop all" can fire before any dashboard poll adopted the
        // orphans. Self-adopt first, so a resident that outlived Studio becomes
        // a handle this stop then kills, instead of returning Ok having killed
        // nothing. No-op when already managed or nothing answers.
        self.try_adopt(reference)?;

        // N2: take the handle to kill out UNDER the lock, then release the lock
        // BEFORE kill_and_reap — its adopted re-verify is a ≤500ms network probe,
        // and holding the handles mutex across it would freeze every other
        // status/start/stop for that long.
        let to_kill = {
            let mut handles = self.handles.lock().expect("runtime mutex poisoned");
            if let Some(handle) = handles.get_mut(&reference.workspace_name) {
                handle.members.remove(&reference.project_id);
                if handle.members.is_empty() {
                    handles.remove(&reference.workspace_name)
                } else {
                    None
                }
            } else {
                None
            }
        };
        let mut killed = false;
        if let Some(mut handle) = to_kill {
            handle.process.kill_and_reap()?;
            killed = true;
        }

        let detail = if killed {
            "Workspace runtime stopped (last project left).".into()
        } else {
            "Project left the workspace; runtime continues for remaining members.".into()
        };
        let status = RuntimeStatusRecord {
            project_id: reference.project_id.clone(),
            phase: RuntimePhase::Stopped,
            workspace_name: reference.workspace_name.clone(),
            transport: "http".into(),
            pid: None,
            workspace_dir: reference.workspace_dir.clone(),
            log_path: self.default_log_path(&reference.workspace_name),
            runtime_label: reference.runtime_label.clone(),
            resolved_jar_path: reference.resolved_jar_path.clone(),
            service_mode: "manager-process".into(),
            detail,
            exit_code: None,
        };

        self.persist_snapshot(status.clone())?;
        Ok(status)
    }

    /// Sprint 10 v0.10.4: stop the entire workspace process unconditionally.
    /// All members' snapshots become Stopped. Used by the "Stop workspace"
    /// button in the grouped Dashboard view.
    pub fn stop_workspace_runtime(&self, workspace_name: &str) -> Result<(), String> {
        let removed = {
            let mut handles = self.handles.lock().expect("runtime mutex poisoned");
            handles.remove(workspace_name)
        };
        if let Some(mut handle) = removed {
            let members = handle.members.clone();
            // kill_and_reap kills an Owned child OR an Adopted orphan by PID —
            // and REPORTS a failure. An orphan is in this map because adopt-on-
            // start / adopt-on-status put it here, so "Stop workspace" acts on
            // it instead of silently returning Ok having killed nothing.
            handle.process.kill_and_reap()?;

            // Mark every member's snapshot as Stopped.
            for project_id in members {
                let snapshot = {
                    let snapshots = self
                        .snapshots
                        .lock()
                        .expect("runtime snapshot mutex poisoned");
                    snapshots.get(&project_id).cloned()
                };
                if let Some(mut s) = snapshot {
                    s.phase = RuntimePhase::Stopped;
                    s.pid = None;
                    s.detail = "Workspace runtime stopped.".into();
                    self.persist_snapshot(s)?;
                }
            }
        }
        Ok(())
    }

    pub fn get_runtime_status(
        &self,
        reference: &RuntimeReference,
    ) -> Result<RuntimeStatusRecord, String> {
        if let Some(status) = self.try_join_running_workspace_readonly(reference)? {
            return Ok(status);
        }

        // studio #1: with no live handle, a stale snapshot may claim Running
        // (an orphan that outlived Studio) or Failed (the doomed-respawn loop).
        // When intent says this workspace should be up, probe the port and
        // adopt a resident that is genuinely serving — the dashboard then reads
        // "Running (adopted)" instead of a false Running 0 / Failed. Guarded by
        // the snapshot phase so a genuinely-stopped workspace is not probed on
        // every poll (each probe is a bounded HTTP call).
        let expected_up = matches!(
            self.snapshots
                .lock()
                .expect("runtime snapshot mutex poisoned")
                .get(&reference.project_id)
                .map(|s| s.phase.clone()),
            Some(RuntimePhase::Running | RuntimePhase::Starting | RuntimePhase::Failed)
        );
        if expected_up {
            if let Some(status) = self.try_adopt(reference)? {
                return Ok(status);
            }
        }

        Ok(self
            .snapshots
            .lock()
            .expect("runtime snapshot mutex poisoned")
            .get(&reference.project_id)
            .cloned()
            .unwrap_or_else(|| RuntimeStatusRecord {
                project_id: reference.project_id.clone(),
                phase: RuntimePhase::Stopped,
                workspace_name: reference.workspace_name.clone(),
                transport: "http".into(),
                pid: None,
                workspace_dir: reference.workspace_dir.clone(),
                log_path: self.default_log_path(&reference.workspace_name),
                runtime_label: reference.runtime_label.clone(),
                resolved_jar_path: reference.resolved_jar_path.clone(),
                service_mode: "manager-process".into(),
                detail: "Runtime has not been started yet.".into(),
                exit_code: None,
            }))
    }

    /// Forcefully forget a project's runtime association. Removes from
    /// snapshots and from any workspace's member set. If the project was
    /// the last member, the workspace process is killed.
    pub fn remove_project_runtime(&self, project_id: &str) -> Result<(), String> {
        // Find which workspace (if any) hosts this project, and leave it.
        let host_workspace = {
            let handles = self.handles.lock().expect("runtime mutex poisoned");
            handles
                .iter()
                .find_map(|(ws, h)| {
                    if h.members.contains(project_id) {
                        Some(ws.clone())
                    } else {
                        None
                    }
                })
        };

        if let Some(ws) = host_workspace {
            // N2: take the handle out under the lock, kill outside it (the
            // adopted re-verify is a network probe).
            let to_kill = {
                let mut handles = self.handles.lock().expect("runtime mutex poisoned");
                if let Some(handle) = handles.get_mut(&ws) {
                    handle.members.remove(project_id);
                    if handle.members.is_empty() {
                        handles.remove(&ws)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(mut handle) = to_kill {
                let _ = handle.process.kill_and_reap();
            }
        }

        let snapshots = {
            let mut snapshots = self
                .snapshots
                .lock()
                .expect("runtime snapshot mutex poisoned");
            snapshots.remove(project_id);
            snapshots.clone()
        };
        write_runtime_state(&self.paths.runtime_state_file, &snapshots)
    }

    pub fn command_spec_for(&self, launch_request: &RuntimeLaunchRequest) -> CommandSpec {
        let log_path = self.default_log_path(&launch_request.reference.workspace_name);
        let reference = &launch_request.reference;

        // Sprint 10 v0.10.4: jawata reads its project list from
        // <workspace_dir>/workspace.json (written by manager_service).
        //
        // Sprint 15 Stage 10: against fork v1.8.5 the default transport is
        // HTTP, so the manager pins it explicitly (-port + -token). The
        // resident JVM emits its READY line on stdout which the spawn path
        // captures to flip the phase to Running.
        let mut args: Vec<String> = Vec::new();

        // Sprint 27 D1: the Vector API module, for the embedder's matrix
        // multiply. Like every JVM option this MUST precede `-jar`.
        //
        // GUARDED, and the guard is not caution — it is measured. A JVM given
        // `--add-modules` for a module it does not have REFUSES TO START
        // ("Error occurred during initialization of boot layer", exit 1); it
        // does not warn and continue. Adding this unconditionally would brick
        // every resident on any JDK without the module — including the JDK
        // where the Vector API finally graduates out of incubator and
        // `jdk.incubator.vector` ceases to exist.
        //
        // Without the flag the embedder still produces IDENTICAL vectors: the
        // scalar implementation answers instead, slower. So this is a speed
        // flag and never a correctness one, and `health_check` reports which
        // implementation actually won rather than leaving anyone to infer it
        // from the presence of this argument.
        if vector_module_available() {
            args.push("--add-modules".into());
            args.push("jdk.incubator.vector".into());
        }

        // Sprint 15 B5c: conditional Lombok comprehension agent. JVM options
        // (`-javaagent`) MUST precede `-jar`, so this goes first. Only added
        // when the project uses Lombok AND the product ships lombok.jar.
        if let Some(agent) = crate::lombok::javaagent_arg(
            &[std::path::PathBuf::from(&launch_request.project_path)],
            std::path::Path::new(&reference.resolved_jar_path),
        ) {
            args.push(agent);
        }

        // Sprint 21a (item F): knowledge-store + memory-crawl configuration as system
        // properties — like -javaagent these MUST precede -jar.
        args.extend(reference.jvm_properties.iter().cloned());

        args.extend([
            "-jar".into(),
            reference.resolved_jar_path.clone(),
            "-data".into(),
            reference.workspace_dir.clone(),
            "-port".into(),
            reference.resident_port.to_string(),
            "-token".into(),
            reference.resident_token.clone(),
        ]);

        CommandSpec {
            command: "java".into(),
            args,
            env: vec![],
            log_path,
        }
    }

    /// If the workspace's process is already running, register the
    /// project as a member and return the workspace's PID as this
    /// project's status. Returns None if no process exists for the
    /// workspace yet (caller should spawn).
    /// studio #1: recover ownership of a resident that outlived this Studio.
    /// When we hold no handle for the workspace but a healthy jawata answers on
    /// its port with the workspace's own token, ADOPT it — record it as a
    /// running (adopted) handle so we neither spawn a doomed second copy nor
    /// render `Running 0`/`Failed` while it serves invisibly, and a later stop
    /// can act on it. Returns the Running status on adoption, else `None`.
    fn try_adopt(
        &self,
        reference: &RuntimeReference,
    ) -> Result<Option<RuntimeStatusRecord>, String> {
        if self
            .handles
            .lock()
            .expect("runtime mutex poisoned")
            .contains_key(&reference.workspace_name)
        {
            return Ok(None); // already managed — nothing to adopt
        }
        if !resident_answers(reference.resident_port, &reference.resident_token) {
            return Ok(None); // nothing (of ours) is listening
        }

        // It answers. Discover its PID so we can later stop it.
        let Some(pid) = pid_listening_on(reference.resident_port) else {
            // Answers but the PID is not discoverable (no ss/lsof): state the
            // truth without a handle — a stop honestly cannot act on what it
            // cannot find, and the render is "not managed", never "Failed".
            let status = self.adopted_status(
                reference,
                None,
                "Running, not managed by this Studio instance (PID not discoverable).",
            );
            self.persist_snapshot(status.clone())?;
            return Ok(Some(status));
        };

        // N1: an adopted handle must carry the FULL member set of the workspace,
        // not just the project that happened to adopt it. Otherwise stopping one
        // project of a multi-project workspace would find members={that project},
        // go empty, and kill the shared resident the OTHER projects still use.
        // Reconstruct membership from the persisted snapshots (every project that
        // shares this workspace_name), which is the same set start_runtime would
        // have accumulated warm.
        let mut members: HashSet<String> = {
            let snapshots = self
                .snapshots
                .lock()
                .expect("runtime snapshot mutex poisoned");
            snapshots
                .values()
                .filter(|s| s.workspace_name == reference.workspace_name)
                .map(|s| s.project_id.clone())
                .collect()
        };
        members.insert(reference.project_id.clone());
        {
            let mut handles = self.handles.lock().expect("runtime mutex poisoned");
            // Re-check under the final lock: a concurrent start may have spawned
            // and inserted an Owned handle in the probe window. If so, keep it —
            // do not overwrite an Owned process with an Adopted record and lose
            // the Child (F5).
            if handles.contains_key(&reference.workspace_name) {
                drop(handles);
                return self.try_join_running_workspace(reference);
            }
            handles.insert(
                reference.workspace_name.clone(),
                ManagedRuntime {
                    process: RuntimeProcess::Adopted {
                        pid,
                        port: reference.resident_port,
                        token: reference.resident_token.clone(),
                    },
                    started_at: Instant::now(),
                    log_path: self.default_log_path(&reference.workspace_name),
                    members,
                    workspace_dir: reference.workspace_dir.clone(),
                    runtime_label: reference.runtime_label.clone(),
                    resolved_jar_path: reference.resolved_jar_path.clone(),
                    resident_port: reference.resident_port,
                    // It answered a health_check, so it is READY by definition.
                    ready: Arc::new(AtomicBool::new(true)),
                },
            );
        }
        let status = self.adopted_status(
            reference,
            Some(pid),
            "Running (adopted — recovered after a Studio restart).",
        );
        self.persist_snapshot(status.clone())?;
        Ok(Some(status))
    }

    fn adopted_status(
        &self,
        reference: &RuntimeReference,
        pid: Option<u32>,
        detail: &str,
    ) -> RuntimeStatusRecord {
        RuntimeStatusRecord {
            project_id: reference.project_id.clone(),
            phase: RuntimePhase::Running,
            workspace_name: reference.workspace_name.clone(),
            transport: "http".into(),
            pid,
            workspace_dir: reference.workspace_dir.clone(),
            log_path: self.default_log_path(&reference.workspace_name),
            runtime_label: reference.runtime_label.clone(),
            resolved_jar_path: reference.resolved_jar_path.clone(),
            service_mode: "manager-process".into(),
            detail: detail.into(),
            exit_code: None,
        }
    }

    fn try_join_running_workspace(
        &self,
        reference: &RuntimeReference,
    ) -> Result<Option<RuntimeStatusRecord>, String> {
        let mut handles = self.handles.lock().expect("runtime mutex poisoned");
        let Some(handle) = handles.get_mut(&reference.workspace_name) else {
            return Ok(None);
        };

        // Check if the process is still alive.
        if let Liveness::Dead { code } = handle.process.liveness()? {
            // Sprint 14 (v0.14.0, bugs.md #2): the process died without the
            // manager initiating the stop. All stop paths (stop_runtime /
            // stop_workspace_runtime / remove_project_runtime) hold the
            // handles mutex while they `handles.remove()` + kill + reap,
            // so by the time anyone re-locks the map a manager-initiated
            // stop is visible as handle-not-present. Reaching this branch
            // with the handle still in the map therefore IS the external-
            // death case (kill -9, crash, OOM) — persist Failed with the
            // captured exit code. Pre-v0.14.0 wrote Stopped here; the tray
            // glyph stayed gray on external kill instead of going red ✗.
            let exit_code = code;
            let detail = if code == Some(0) {
                "Previous workspace runtime exited unexpectedly (status 0); respawning.".into()
            } else {
                format!(
                    "Previous workspace runtime exited unexpectedly with status {}; respawning.",
                    code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into())
                )
            };
            handles.remove(&reference.workspace_name);
            drop(handles);

            let failed = RuntimeStatusRecord {
                project_id: reference.project_id.clone(),
                phase: RuntimePhase::Failed,
                workspace_name: reference.workspace_name.clone(),
                transport: "http".into(),
                pid: None,
                workspace_dir: reference.workspace_dir.clone(),
                log_path: self.default_log_path(&reference.workspace_name),
                runtime_label: reference.runtime_label.clone(),
                resolved_jar_path: reference.resolved_jar_path.clone(),
                service_mode: "manager-process".into(),
                detail,
                exit_code,
            };
            self.persist_snapshot(failed)?;
            return Ok(None);
        }

        handle.members.insert(reference.project_id.clone());
        // Sprint 15 Stage 10: prefer the READY-line signal when present.
        // The 2s-elapsed fallback remains for back-compat with stub test
        // commands (`sleep`, etc.) that don't print READY; in production
        // fork v1.8.5 emits READY within ~5-10s of spawn, so the signal
        // path is dominant in the real flow.
        let phase = if handle.ready.load(Ordering::Acquire) {
            RuntimePhase::Running
        } else if handle.started_at.elapsed() < Duration::from_secs(2) {
            RuntimePhase::Starting
        } else {
            RuntimePhase::Running
        };
        let status = RuntimeStatusRecord {
            project_id: reference.project_id.clone(),
            phase,
            workspace_name: reference.workspace_name.clone(),
            transport: "http".into(),
            pid: Some(handle.process.id()),
            workspace_dir: handle.workspace_dir.clone(),
            log_path: handle.log_path.clone(),
            runtime_label: handle.runtime_label.clone(),
            resolved_jar_path: handle.resolved_jar_path.clone(),
            service_mode: "manager-process".into(),
            detail: "Joined live workspace runtime; tools/list reflects current workspace.json."
                .into(),
            exit_code: None,
        };
        drop(handles);
        self.persist_snapshot(status.clone())?;
        Ok(Some(status))
    }

    /// Same as try_join_running_workspace but does not add the project to
    /// the member set. Used by get_runtime_status, which mustn't have
    /// side effects on membership.
    fn try_join_running_workspace_readonly(
        &self,
        reference: &RuntimeReference,
    ) -> Result<Option<RuntimeStatusRecord>, String> {
        let mut handles = self.handles.lock().expect("runtime mutex poisoned");
        let Some(handle) = handles.get_mut(&reference.workspace_name) else {
            return Ok(None);
        };

        if let Liveness::Dead { code } = handle.process.liveness()? {
            // Sprint 14 (v0.14.0, bugs.md #2): same external-death case as
            // `try_join_running_workspace`. The readonly variant previously
            // just removed the handle and returned None, leaving a stale
            // Running snapshot visible to the next get_runtime_status. Now
            // persist a Failed snapshot here too so the dashboard / tray
            // reflect the death without waiting for an explicit start.
            let exit_code = code;
            let detail = if code == Some(0) {
                "Workspace runtime exited unexpectedly (status 0).".into()
            } else {
                format!(
                    "Workspace runtime exited unexpectedly with status {}.",
                    code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into())
                )
            };
            let failed = RuntimeStatusRecord {
                project_id: reference.project_id.clone(),
                phase: RuntimePhase::Failed,
                workspace_name: reference.workspace_name.clone(),
                transport: "http".into(),
                pid: None,
                workspace_dir: reference.workspace_dir.clone(),
                log_path: self.default_log_path(&reference.workspace_name),
                runtime_label: reference.runtime_label.clone(),
                resolved_jar_path: reference.resolved_jar_path.clone(),
                service_mode: "manager-process".into(),
                detail,
                exit_code,
            };
            handles.remove(&reference.workspace_name);
            drop(handles);
            self.persist_snapshot(failed)?;
            return Ok(None);
        }

        // Sprint 15 Stage 10: same precedence as try_join_running_workspace —
        // ready-signal first, then the 2s legacy heuristic for stub-command
        // test paths.
        let phase = if handle.ready.load(Ordering::Acquire) {
            RuntimePhase::Running
        } else if handle.started_at.elapsed() < Duration::from_secs(2) {
            RuntimePhase::Starting
        } else {
            RuntimePhase::Running
        };
        let status = RuntimeStatusRecord {
            project_id: reference.project_id.clone(),
            phase,
            workspace_name: reference.workspace_name.clone(),
            transport: "http".into(),
            pid: Some(handle.process.id()),
            workspace_dir: handle.workspace_dir.clone(),
            log_path: handle.log_path.clone(),
            runtime_label: handle.runtime_label.clone(),
            resolved_jar_path: handle.resolved_jar_path.clone(),
            service_mode: "manager-process".into(),
            detail: "Live workspace runtime.".into(),
            exit_code: None,
        };
        Ok(Some(status))
    }

    fn persist_snapshot(&self, status: RuntimeStatusRecord) -> Result<(), String> {
        let snapshots = {
            let mut snapshots = self
                .snapshots
                .lock()
                .expect("runtime snapshot mutex poisoned");
            snapshots.insert(status.project_id.clone(), status);
            snapshots.clone()
        };

        write_runtime_state(&self.paths.runtime_state_file, &snapshots)
    }

    fn default_log_path(&self, workspace_name: &str) -> String {
        // Sprint 10 v0.10.4: log path keyed by workspace_name (one log per
        // workspace process), not per project_id.
        display_path(&self.paths.log_dir.join(format!("{workspace_name}.log")))
    }
}

/// Sprint 27 D1: does the `java` on PATH actually have `jdk.incubator.vector`?
///
/// Asked by RUNNING it, once per process, because the only reliable answer is
/// the JVM's own: a JVM given `--add-modules` for a module it lacks refuses to
/// start (exit 1, "Error occurred during initialization of boot layer"), so a
/// wrong guess here does not degrade the resident — it prevents it.
///
/// A probe that cannot run at all answers FALSE: launching without the flag
/// costs speed, launching with a flag the JVM rejects costs the resident.
fn vector_module_available() -> bool {
    static PROBE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PROBE.get_or_init(|| {
        std::process::Command::new("java")
            .args(["--add-modules", "jdk.incubator.vector", "-version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn read_runtime_state(path: &Path) -> Result<HashMap<String, RuntimeStatusRecord>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read runtime state {}: {error}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse runtime state {}: {error}", path.display()))
}

fn write_runtime_state(
    path: &Path,
    snapshots: &HashMap<String, RuntimeStatusRecord>,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(snapshots).map_err(|error| {
        format!(
            "failed to serialize runtime state {}: {error}",
            path.display()
        )
    })?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("failed to write runtime state {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppPaths;
    use std::path::PathBuf;

    // ---- Sprint 27a Stage 10 (studio #1): adopt/kill primitives ----

    #[test]
    fn parse_ss_pid_extracts_the_pid_field() {
        let line = "LISTEN 0 4096 127.0.0.1:8890 0.0.0.0:* \
                    users:((\"java\",pid=12345,fd=7))";
        assert_eq!(parse_ss_pid(line), Some(12345));
        assert_eq!(parse_ss_pid("LISTEN 0 4096 127.0.0.1:8890 0.0.0.0:*"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn process_alive_then_kill_pid_reaps_it() {
        // A real short-lived child so the liveness+kill path is exercised end
        // to end (the mechanism honest-stop uses on an adopted orphan).
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        assert!(process_alive(pid), "the just-spawned process is alive");

        kill_pid(pid).expect("kill_pid succeeds on a live process");
        let _ = child.wait();
        // Give the OS a moment to reap, then confirm it is gone.
        std::thread::sleep(Duration::from_millis(200));
        assert!(!process_alive(pid), "after kill_pid the process is gone");
    }

    #[test]
    fn resident_answers_is_false_when_nothing_listens() {
        // Port 1 needs privilege to bind and nothing of ours listens there, so
        // the probe must fail closed (never adopt a phantom). Bounded by the
        // 500ms client timeout.
        assert!(!resident_answers(1, "any-token"));
    }

    fn fake_paths() -> AppPaths {
        AppPaths {
            config_dir: PathBuf::from("/tmp/jawata-studio/config"),
            state_dir: PathBuf::from("/tmp/jawata-studio/state"),
            cache_dir: PathBuf::from("/tmp/jawata-studio/cache"),
            projects_file: PathBuf::from("/tmp/jawata-studio/config/projects.json"),
            settings_file: PathBuf::from("/tmp/jawata-studio/config/settings.json"),
            runtime_state_file: PathBuf::from("/tmp/jawata-studio/state/runtime-state.json"),
            default_data_root: PathBuf::from("/tmp/jawata-studio/cache"),
            log_dir: PathBuf::from("/tmp/jawata-studio/state/logs"),
        }
    }

    fn fake_launch_request() -> RuntimeLaunchRequest {
        RuntimeLaunchRequest {
            project_path: "/projects/example-service".into(),
            reference: RuntimeReference {
                project_id: "example-service-1".into(),
                workspace_name: "test-ws".into(),
                workspace_dir: "/cache/jawata/test-ws".into(),
                runtime_label: "Managed JAWATA 1.4.0".into(),
                resolved_jar_path: "/tools/jawata/jawata.jar".into(),
                jvm_properties: vec!["-Djawata.experience.store=shared".into()],
                resident_port: 8800,
                resident_token: "test-token".into(),
            },
        }
    }

    #[test]
    fn command_spec_uses_workspace_dir_and_no_env_var() {
        let manager = RuntimeManager::new(fake_paths());
        let launch_request = fake_launch_request();

        let spec = manager.command_spec_for(&launch_request);

        assert_eq!(spec.command, "java");

        // Sprint 27 D1: the Vector API flag is PROBED, so whether it is present
        // depends on the JVM this test runs under. Assert the rule rather than
        // one machine's answer: present exactly when the module is available,
        // and always as the leading pair (JVM options must precede -jar).
        let mut args = spec.args.clone();
        if vector_module_available() {
            assert_eq!(
                &args[0..2],
                &["--add-modules".to_string(), "jdk.incubator.vector".to_string()],
                "the module is available here, so the flag must lead the args"
            );
            args.drain(0..2);
        } else {
            assert!(
                !args.contains(&"jdk.incubator.vector".to_string()),
                "the module is NOT available here — passing the flag anyway would \
                 stop the JVM from starting at all"
            );
        }

        // Sprint 15 Stage 10: -port + -token added so the fork v1.8.5
        // HTTP listener binds where the URL-emitting MCP writer expects.
        // Sprint 21a (item F): knowledge-store system properties precede -jar.
        assert_eq!(
            args,
            vec![
                "-Djawata.experience.store=shared",
                "-jar",
                "/tools/jawata/jawata.jar",
                "-data",
                "/cache/jawata/test-ws",
                "-port",
                "8800",
                "-token",
                "test-token"
            ]
        );
        // Sprint 10 v0.10.4: no JAVA_PROJECT_PATH — workspace.json drives
        // project loading inside jawata.
        assert!(spec.env.is_empty());
        assert!(spec.log_path.ends_with("test-ws.log"));
    }

    #[test]
    fn unresolved_runtime_status_carries_runtime_label() {
        let status = RuntimeStatusRecord::unresolved(
            "project-1".into(),
            "test-ws".into(),
            "/tmp/workspace".into(),
            "Managed JAWATA 1.4.0".into(),
            "Missing runtime".into(),
        );

        assert!(matches!(status.phase, RuntimePhase::Failed));
        assert_eq!(status.workspace_name, "test-ws");
        assert_eq!(status.runtime_label, "Managed JAWATA 1.4.0");
        assert_eq!(status.detail, "Missing runtime");
    }

    // ============================================================
    // Sprint 10 v0.10.4: workspace-grouped spawn lifecycle tests.
    //
    // These spawn real processes (a tiny `sleep` command stand-in for
    // jawata) so the membership / kill / join lifecycle is exercised
    // end-to-end without depending on a real jawata runtime.
    // Skipped on non-Unix platforms because `sleep` isn't on Windows.
    // ============================================================

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-studio-rmtest-{label}-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
        std::fs::create_dir_all(&dir).expect("failed to create test tempdir");
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::create_dir_all(dir.join("ws")).unwrap();
        dir
    }

    fn paths_in(dir: &std::path::Path) -> AppPaths {
        AppPaths {
            config_dir: dir.to_path_buf(),
            state_dir: dir.to_path_buf(),
            cache_dir: dir.to_path_buf(),
            projects_file: dir.join("projects.json"),
            settings_file: dir.join("settings.json"),
            runtime_state_file: dir.join("runtime-state.json"),
            default_data_root: dir.to_path_buf(),
            log_dir: dir.join("logs"),
        }
    }

    /// Build a CommandSpec that runs `sleep 60` — long enough to outlast
    /// the test's lifecycle assertions but quick to clean up via kill.
    fn sleep_spec(workspace_dir: &str, log_path: String) -> CommandSpec {
        CommandSpec {
            command: "sleep".into(),
            args: vec!["60".into()],
            env: vec![],
            log_path,
        }
    }

    fn make_reference(project_id: &str, workspace_name: &str, workspace_dir: &str) -> RuntimeReference {
        // Sprint 15 Stage 10: tests pass dummy (port, token) values; the
        // real allocator is exercised in src/resident.rs tests and the
        // config-store integration tests.
        RuntimeReference {
            project_id: project_id.into(),
            workspace_name: workspace_name.into(),
            workspace_dir: workspace_dir.into(),
            runtime_label: "test-runtime".into(),
            resolved_jar_path: "/dev/null".into(),
            jvm_properties: vec![],
            resident_port: 8800,
            resident_token: "test-token".into(),
        }
    }

    /// A minimal HTTP/1.1 stand-in for a jawata resident: it answers a bounded
    /// number of probes with a JSON-RPC `result` (or `error` when `ok=false`),
    /// on an ephemeral localhost port. Lets the adopt/probe/kill paths run
    /// against a real socket without a real jawata.
    fn spawn_stub_resident(ok: bool) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..40 {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 4096];
                        let _ = stream.read(&mut buf); // drain the request (best-effort)
                        let body = if ok {
                            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#
                        } else {
                            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"no"}}"#
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(_) => break,
                }
            }
        });
        (port, handle)
    }

    #[test]
    fn resident_answers_true_on_a_result_false_on_an_error() {
        // F2: a 2xx is not enough — the body must be a JSON-RPC success.
        let (ok_port, _a) = spawn_stub_resident(true);
        assert!(resident_answers(ok_port, "test-token"), "a result means it is ours");
        let (err_port, _b) = spawn_stub_resident(false);
        assert!(
            !resident_answers(err_port, "test-token"),
            "a JSON-RPC error (e.g. a foreign 200) is NOT adoption"
        );
    }

    /// A jawata resident stand-in as a REAL subprocess that binds an ephemeral
    /// port and answers `POST /mcp` with a JSON-RPC `result`. Unlike the thread
    /// stub, it has its own PID that `pid_listening_on(port)` resolves — so the
    /// adopt (PID discovery) and kill (identity re-verify) paths run faithfully.
    /// Returns (port, child); the caller owns the child.
    #[cfg(unix)]
    fn spawn_http_resident() -> (u16, Child) {
        let script = "import http.server, sys\n\
            class H(http.server.BaseHTTPRequestHandler):\n\
            \x20   def do_POST(self):\n\
            \x20       n=int(self.headers.get('Content-Length', 0) or 0)\n\
            \x20       self.rfile.read(n)\n\
            \x20       b=b'{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}'\n\
            \x20       self.send_response(200)\n\
            \x20       self.send_header('Content-Type','application/json')\n\
            \x20       self.send_header('Content-Length',str(len(b)))\n\
            \x20       self.end_headers()\n\
            \x20       self.wfile.write(b)\n\
            \x20   def log_message(self,*a):\n\
            \x20       pass\n\
            srv=http.server.HTTPServer(('127.0.0.1',0),H)\n\
            print(srv.server_address[1]); sys.stdout.flush()\n\
            srv.serve_forever()\n";
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn python http resident");
        let stdout = child.stdout.take().expect("stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read port line");
        let port: u16 = line.trim().parse().expect("parse port");
        (port, child)
    }

    /// Reap a killed child and report whether it exited within the timeout.
    /// The test is the resident's PARENT, so a SIGKILLed child lingers as a
    /// zombie (still "alive" to `kill -0`) until we `wait` it — unlike a real
    /// adopted resident, whose parent is init. Poll `try_wait` to reap.
    #[cfg(unix)]
    fn wait_gone(child: &mut Child, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => return false,
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn try_adopt_records_a_running_adopted_handle() {
        // studio #1 point 1/3: a resident answering on its port is adopted as a
        // Running handle (not spawned again, not rendered Failed).
        let dir = unique_tempdir("try-adopt");
        let manager = RuntimeManager::new(paths_in(&dir));
        let (port, mut resident) = spawn_http_resident();

        let mut reference = make_reference("p-a", "test", "");
        reference.resident_port = port;
        reference.resident_token = "test-token".into();

        let status = manager
            .try_adopt(&reference)
            .expect("adopt ok")
            .expect("a live resident is adopted");
        assert_eq!(status.phase, RuntimePhase::Running);
        assert!(
            status.detail.to_lowercase().contains("adopt")
                || status.detail.to_lowercase().contains("not managed"),
            "the render distinguishes adopted/unmanaged, not Failed: {}",
            status.detail
        );
        assert!(
            manager.workspace_members("test").is_some(),
            "the adopted resident is now a managed handle a stop can act on"
        );
        let _ = resident.kill();
        let _ = resident.wait();
    }

    #[cfg(unix)]
    #[test]
    fn stop_workspace_kills_an_adopted_orphan_by_pid() {
        // studio #1 point 2: an orphan adopted as a handle with NO Child is
        // still killed by its discovered PID. The resident is a real listening
        // process, so the identity re-verify (pid == port's listener) holds.
        let dir = unique_tempdir("adopt-stop");
        let manager = RuntimeManager::new(paths_in(&dir));
        let (port, mut resident) = spawn_http_resident();
        let pid = resident.id();

        let mut members = HashSet::new();
        members.insert("p-a".to_string());
        manager.handles.lock().unwrap().insert(
            "test".to_string(),
            ManagedRuntime {
                process: RuntimeProcess::Adopted {
                    pid,
                    port,
                    token: "test-token".into(),
                },
                started_at: Instant::now(),
                log_path: String::new(),
                members,
                workspace_dir: String::new(),
                runtime_label: "test-runtime".into(),
                resolved_jar_path: "/dev/null".into(),
                resident_port: port,
                ready: Arc::new(AtomicBool::new(true)),
            },
        );

        // The identity gate needs ss to resolve OUR pid as the port's listener.
        assert_eq!(
            pid_listening_on(port),
            Some(pid),
            "ss must resolve the resident's pid as the port's listener"
        );
        manager
            .stop_workspace_runtime("test")
            .expect("stop must act on an adopted orphan, not lie");
        assert!(
            wait_gone(&mut resident, Duration::from_secs(2)),
            "stop killed the adopted resident by PID"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_refuses_when_a_different_pid_holds_the_port() {
        // F4 identity: the resident answers on the port, but our STORED pid is
        // not the one listening (it died / relaunched on a new pid). We must NOT
        // kill the stored pid — it may have been recycled to an innocent.
        let dir = unique_tempdir("adopt-identity");
        let manager = RuntimeManager::new(paths_in(&dir));
        let (port, mut resident) = spawn_http_resident(); // pid P listens here

        let mut innocent = Command::new("sleep").arg("30").spawn().expect("spawn sleep");
        let stored_pid = innocent.id(); // NOT the listener

        let mut members = HashSet::new();
        members.insert("p-a".to_string());
        manager.handles.lock().unwrap().insert(
            "test".to_string(),
            ManagedRuntime {
                process: RuntimeProcess::Adopted {
                    pid: stored_pid,
                    port,
                    token: "test-token".into(),
                },
                started_at: Instant::now(),
                log_path: String::new(),
                members,
                workspace_dir: String::new(),
                runtime_label: "test-runtime".into(),
                resolved_jar_path: "/dev/null".into(),
                resident_port: port,
                ready: Arc::new(AtomicBool::new(true)),
            },
        );

        manager.stop_workspace_runtime("test").expect("stop reports honestly");
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            process_alive(stored_pid),
            "a stored pid that is NOT the port's listener must not be killed"
        );
        let _ = innocent.kill();
        let _ = innocent.wait();
        let _ = resident.kill();
        let _ = resident.wait();
    }

    #[cfg(unix)]
    #[test]
    fn stop_one_of_two_projects_keeps_the_shared_resident_alive() {
        // N1: a multi-project workspace adopted after a restart must NOT be killed
        // by stopping ONE of its projects. try_adopt seeds membership from the
        // snapshots (both projects), so the shared resident survives until the
        // last member leaves.
        let dir = unique_tempdir("adopt-multi");
        let manager = RuntimeManager::new(paths_in(&dir));
        let (port, mut resident) = spawn_http_resident();
        let pid = resident.id();

        // Two projects share workspace "test" (their persisted snapshots).
        for project in ["p-a", "p-b"] {
            let mut reference = make_reference(project, "test", "");
            reference.resident_port = port;
            reference.resident_token = "test-token".into();
            let mut snap = manager.adopted_status(&reference, Some(pid), "restored");
            snap.phase = RuntimePhase::Running;
            manager
                .snapshots
                .lock()
                .unwrap()
                .insert(project.to_string(), snap);
        }

        let mut ref_a = make_reference("p-a", "test", "");
        ref_a.resident_port = port;
        ref_a.resident_token = "test-token".into();
        // Cold handle map: stopping p-a self-adopts (members from snapshots =
        // {p-a, p-b}), removes p-a, leaves p-b → the resident LIVES.
        manager.stop_runtime(&ref_a).expect("stop p-a");
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            process_alive(pid),
            "stopping one project of a shared workspace must NOT kill the resident"
        );

        // Stopping the last project kills it.
        let mut ref_b = make_reference("p-b", "test", "");
        ref_b.resident_port = port;
        ref_b.resident_token = "test-token".into();
        manager.stop_runtime(&ref_b).expect("stop p-b");
        assert!(
            wait_gone(&mut resident, Duration::from_secs(2)),
            "stopping the last member kills the resident"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_refuses_to_kill_a_pid_whose_resident_no_longer_answers() {
        // F4: if the adopted resident no longer answers (died; its PID may have
        // been recycled), a stop must NOT kill whatever now holds that PID.
        let dir = unique_tempdir("adopt-refuse");
        let manager = RuntimeManager::new(paths_in(&dir));

        // A live process standing in for a recycled PID — nothing of ours
        // answers on this port, so it must be left alone.
        let mut innocent = Command::new("sleep").arg("30").spawn().expect("spawn sleep");
        let pid = innocent.id();

        let mut members = HashSet::new();
        members.insert("p-a".to_string());
        manager.handles.lock().unwrap().insert(
            "test".to_string(),
            ManagedRuntime {
                process: RuntimeProcess::Adopted {
                    pid,
                    port: 2, // nothing of ours listens here
                    token: "test-token".into(),
                },
                started_at: Instant::now(),
                log_path: String::new(),
                members,
                workspace_dir: String::new(),
                runtime_label: "test-runtime".into(),
                resolved_jar_path: "/dev/null".into(),
                resident_port: 2,
                ready: Arc::new(AtomicBool::new(true)),
            },
        );

        manager
            .stop_workspace_runtime("test")
            .expect("stop reports honestly");
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            process_alive(pid),
            "a PID whose resident does not answer must NOT be killed (recycle safety)"
        );
        let _ = innocent.kill();
        let _ = innocent.wait();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_grouped_spawn_two_projects_share_one_process() {
        // Two projects sharing a workspace_name → start_runtime spawns
        // ONCE for the first project and the second project JOINS the
        // same process (no second spawn). Both members are tracked.
        let dir = unique_tempdir("two-share");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let ref_a = make_reference("p-a", "test", &ws_dir);
        let ref_b = make_reference("p-b", "test", &ws_dir);

        let log_a = dir.join("logs").join("test.log").to_string_lossy().to_string();
        let status_a = manager
            .start_runtime_with_spec(&ref_a, sleep_spec(&ws_dir, log_a.clone()))
            .expect("first spawn must succeed");
        let pid_a = status_a.pid.expect("first spawn produces a PID");

        let status_b = manager
            .start_runtime_with_spec(&ref_b, sleep_spec(&ws_dir, log_a.clone()))
            .expect("second start must JOIN the running workspace");
        let pid_b = status_b.pid.expect("joining returns a PID too");

        // Same process for both projects — no second spawn.
        assert_eq!(pid_a, pid_b, "joining must reuse the running PID");

        let members = manager.workspace_members("test").unwrap();
        assert_eq!(members.len(), 2, "both projects in members set");
        assert!(members.contains(&"p-a".to_string()));
        assert!(members.contains(&"p-b".to_string()));

        // Cleanup.
        manager.stop_workspace_runtime("test").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_stop_runtime_keeps_process_alive_for_remaining_members() {
        // Two members → stop one → workspace process keeps running for
        // the other; only the leaving project's snapshot is "stopped".
        let dir = unique_tempdir("keeps-alive");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let ref_a = make_reference("p-a", "test", &ws_dir);
        let ref_b = make_reference("p-b", "test", &ws_dir);
        let log = dir.join("logs").join("test.log").to_string_lossy().to_string();

        manager.start_runtime_with_spec(&ref_a, sleep_spec(&ws_dir, log.clone())).unwrap();
        manager.start_runtime_with_spec(&ref_b, sleep_spec(&ws_dir, log.clone())).unwrap();
        let pid_before = manager.workspace_pid("test").unwrap();

        // p-a leaves. Workspace must still be alive for p-b.
        manager.stop_runtime(&ref_a).unwrap();
        let members_after = manager.workspace_members("test").unwrap();
        assert_eq!(members_after, vec!["p-b".to_string()]);
        let pid_after = manager.workspace_pid("test").unwrap();
        assert_eq!(pid_before, pid_after, "process must NOT have been killed");

        // Cleanup.
        manager.stop_workspace_runtime("test").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_stop_runtime_kills_process_when_last_member_leaves() {
        // Single member → stop_runtime → kills the process and removes
        // the handle (workspace_pid returns None).
        let dir = unique_tempdir("kill-last");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let ref_only = make_reference("p-only", "test", &ws_dir);
        let log = dir.join("logs").join("test.log").to_string_lossy().to_string();

        manager.start_runtime_with_spec(&ref_only, sleep_spec(&ws_dir, log)).unwrap();
        assert!(manager.workspace_pid("test").is_some());

        // Last member leaves → process killed.
        manager.stop_runtime(&ref_only).unwrap();
        assert!(
            manager.workspace_pid("test").is_none(),
            "workspace handle removed when last member leaves"
        );
        assert!(
            manager.workspace_members("test").is_none(),
            "no members map for a dead workspace"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_stop_workspace_runtime_kills_unconditionally() {
        // stop_workspace_runtime is the "Stop workspace" button — it
        // kills regardless of how many members are still attached.
        let dir = unique_tempdir("force-stop-ws");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let ref_a = make_reference("p-a", "test", &ws_dir);
        let ref_b = make_reference("p-b", "test", &ws_dir);
        let log = dir.join("logs").join("test.log").to_string_lossy().to_string();

        manager.start_runtime_with_spec(&ref_a, sleep_spec(&ws_dir, log.clone())).unwrap();
        manager.start_runtime_with_spec(&ref_b, sleep_spec(&ws_dir, log)).unwrap();
        assert_eq!(manager.workspace_members("test").unwrap().len(), 2);

        manager.stop_workspace_runtime("test").unwrap();
        assert!(
            manager.workspace_pid("test").is_none(),
            "workspace handle removed by force-stop"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_remove_project_runtime_decrements_membership() {
        // remove_project_runtime is called by manager_service.delete_project.
        // It scans handles for the project and removes its membership
        // (killing the process iff this was the last member).
        let dir = unique_tempdir("remove-project");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let ref_a = make_reference("p-a", "test", &ws_dir);
        let ref_b = make_reference("p-b", "test", &ws_dir);
        let log = dir.join("logs").join("test.log").to_string_lossy().to_string();

        manager.start_runtime_with_spec(&ref_a, sleep_spec(&ws_dir, log.clone())).unwrap();
        manager.start_runtime_with_spec(&ref_b, sleep_spec(&ws_dir, log)).unwrap();

        manager.remove_project_runtime("p-a").unwrap();
        // p-a gone, p-b still a member, process still alive.
        let members = manager.workspace_members("test").unwrap();
        assert_eq!(members, vec!["p-b".to_string()]);
        assert!(manager.workspace_pid("test").is_some());

        // Now remove the last member → process dies.
        manager.remove_project_runtime("p-b").unwrap();
        assert!(manager.workspace_pid("test").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_two_distinct_workspaces_have_independent_processes() {
        // Two workspace_names → two processes. Stopping one does NOT
        // affect the other.
        let dir = unique_tempdir("two-ws-independent");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_a_dir = dir.join("ws").join("a").to_string_lossy().to_string();
        let ws_b_dir = dir.join("ws").join("b").to_string_lossy().to_string();
        let ref_a = make_reference("p-a", "ws-a", &ws_a_dir);
        let ref_b = make_reference("p-b", "ws-b", &ws_b_dir);
        let log_a = dir.join("logs").join("a.log").to_string_lossy().to_string();
        let log_b = dir.join("logs").join("b.log").to_string_lossy().to_string();

        manager.start_runtime_with_spec(&ref_a, sleep_spec(&ws_a_dir, log_a)).unwrap();
        manager.start_runtime_with_spec(&ref_b, sleep_spec(&ws_b_dir, log_b)).unwrap();

        let pid_a = manager.workspace_pid("ws-a").unwrap();
        let pid_b = manager.workspace_pid("ws-b").unwrap();
        assert_ne!(pid_a, pid_b, "distinct workspaces → distinct processes");

        // Stop ws-a only.
        manager.stop_workspace_runtime("ws-a").unwrap();
        assert!(manager.workspace_pid("ws-a").is_none());
        assert_eq!(manager.workspace_pid("ws-b"), Some(pid_b), "ws-b unaffected");

        // Cleanup.
        manager.stop_workspace_runtime("ws-b").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============================================================
    // Sprint 14 (v0.14.0, bugs.md #2): process-death → Failed (was Stopped)
    // ============================================================

    /// Spawn a command that exits immediately with code 1. The next
    /// status read must observe Failed with exit_code Some(1) — not
    /// Stopped (the pre-v0.14.0 behavior that this bug fix corrects).
    #[cfg(unix)]
    #[test]
    fn unexpected_exit_transitions_to_failed_with_code() {
        let dir = unique_tempdir("unexpected-exit");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let reference = make_reference("p-1", "test", &ws_dir);
        let log = dir
            .join("logs")
            .join("test.log")
            .to_string_lossy()
            .to_string();

        let spec = CommandSpec {
            command: "sh".into(),
            args: vec!["-c".into(), "exit 1".into()],
            env: vec![],
            log_path: log,
        };
        manager
            .start_runtime_with_spec(&reference, spec)
            .expect("start must succeed before the child exits");

        // Give the child time to exit and be reaped by try_wait on the
        // next status read.
        std::thread::sleep(Duration::from_millis(200));

        let status = manager
            .get_runtime_status(&reference)
            .expect("status read must succeed after death");
        assert!(
            matches!(status.phase, RuntimePhase::Failed),
            "phase should be Failed after unexpected exit, was {:?}",
            status.phase
        );
        assert_eq!(
            status.exit_code,
            Some(1),
            "exit_code should reflect the child's exit status"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// User-initiated stop (single-member workspace) → handle removed +
    /// kill + wait, all synchronous under the handles mutex. The
    /// resulting status must be Stopped with no exit_code attribution.
    #[cfg(unix)]
    #[test]
    fn user_stop_transitions_to_stopped() {
        let dir = unique_tempdir("user-stop");
        let paths = paths_in(&dir);
        let manager = RuntimeManager::new(paths);

        let ws_dir = dir.join("ws").join("test").to_string_lossy().to_string();
        let reference = make_reference("p-1", "test", &ws_dir);
        let log = dir
            .join("logs")
            .join("test.log")
            .to_string_lossy()
            .to_string();

        manager
            .start_runtime_with_spec(&reference, sleep_spec(&ws_dir, log))
            .unwrap();
        let status = manager.stop_runtime(&reference).unwrap();

        assert!(
            matches!(status.phase, RuntimePhase::Stopped),
            "user stop should produce Stopped, was {:?}",
            status.phase
        );
        assert_eq!(status.exit_code, None);
        assert!(manager.workspace_pid("test").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 15 Stage 10: READY-line capture =====

    /// Stub `CommandSpec` that emits a READY line (matching the fork's
    /// v1.8.5 contract) then sleeps long enough that the test can observe
    /// the captured signal.
    #[cfg(unix)]
    fn ready_emitting_spec(workspace_name: &str, log_path: String) -> CommandSpec {
        CommandSpec {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "printf 'READY url=http://127.0.0.1:8800 token=stub-token\\n'; sleep 20; echo done {}",
                    workspace_name
                ),
            ],
            env: vec![],
            log_path,
        }
    }

    /// Stub `CommandSpec` that prints nothing on stdout for ~3 s, then a
    /// distinct marker. Used to verify the legacy heuristic still flips
    /// the phase to Running even when READY never arrives (back-compat).
    #[cfg(unix)]
    fn silent_then_marker_spec(log_path: String) -> CommandSpec {
        CommandSpec {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                "sleep 20; printf 'late marker (not READY)\\n'".into(),
            ],
            env: vec![],
            log_path,
        }
    }

    #[cfg(unix)]
    #[test]
    fn spawns_and_captures_ready_line_flips_phase_to_running() {
        // The fork's READY contract — `READY url=... token=...` on the
        // first line of stdout — must flip the workspace from Starting to
        // Running well within the legacy 2 s heuristic, so the dashboard
        // / tray reflect "ready to use" as soon as the JVM is bound.
        let dir = unique_test_dir("ready-flips-running");
        let ws_dir = dir.to_string_lossy().to_string();
        let log = dir.join("ready.log").to_string_lossy().to_string();
        let manager = RuntimeManager::new(fake_paths_in(&dir));
        let reference = make_reference("ready-1", "ready-ws", &ws_dir);

        let initial = manager
            .start_runtime_with_spec(&reference, ready_emitting_spec("ready-ws", log))
            .unwrap();
        assert!(
            matches!(initial.phase, RuntimePhase::Starting),
            "spawn returns Starting immediately, was {:?}",
            initial.phase
        );

        // Give the stdout-capture thread time to read the READY line.
        // The shell prints it before sleep, so a few hundred ms is ample.
        std::thread::sleep(Duration::from_millis(500));

        // Re-querying the workspace now returns Running because the ready
        // flag is set — within the 2 s elapsed window, the ready signal
        // takes precedence over the legacy heuristic.
        let again = manager
            .start_runtime_with_spec(&reference, ready_emitting_spec("ready-ws", String::new()))
            .unwrap();
        assert!(
            matches!(again.phase, RuntimePhase::Running),
            "after READY, phase must be Running (got {:?})",
            again.phase
        );

        let _ = manager.stop_workspace_runtime("ready-ws");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn stdout_capture_tees_lines_to_log_file() {
        // Sanity: the capture thread MUST still tee stdout to the per-
        // workspace log file (regression guard for the change from
        // direct-to-file stdout in pre-Stage-10 to piped+capture).
        let dir = unique_test_dir("ready-tee");
        let ws_dir = dir.to_string_lossy().to_string();
        let log = dir.join("tee.log").to_string_lossy().to_string();
        let manager = RuntimeManager::new(fake_paths_in(&dir));
        let reference = make_reference("tee-1", "tee-ws", &ws_dir);

        manager
            .start_runtime_with_spec(&reference, ready_emitting_spec("tee-ws", log.clone()))
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));

        let contents = std::fs::read_to_string(&log).expect("read log file");
        assert!(
            contents.contains("READY url="),
            "log file must contain the READY line teed from stdout: {:?}",
            contents
        );

        let _ = manager.stop_workspace_runtime("tee-ws");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn missing_ready_falls_back_to_heuristic_running() {
        // Back-compat: stub commands that don't emit READY still flip to
        // Running once the legacy 2 s window elapses. Without this fallback
        // the existing sleep-based membership tests would stay Starting
        // forever.
        let dir = unique_test_dir("ready-fallback");
        let ws_dir = dir.to_string_lossy().to_string();
        let log = dir.join("fb.log").to_string_lossy().to_string();
        let manager = RuntimeManager::new(fake_paths_in(&dir));
        let reference = make_reference("fb-1", "fb-ws", &ws_dir);

        manager
            .start_runtime_with_spec(&reference, silent_then_marker_spec(log))
            .unwrap();

        // Past the 2 s heuristic window, phase is Running even though
        // ready never flipped.
        std::thread::sleep(Duration::from_millis(2_200));
        let again = manager
            .start_runtime_with_spec(
                &reference,
                silent_then_marker_spec(String::new()),
            )
            .unwrap();
        assert!(
            matches!(again.phase, RuntimePhase::Running),
            "post-2s without READY must fall back to Running (got {:?})",
            again.phase
        );

        let _ = manager.stop_workspace_runtime("fb-ws");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn fake_paths_in(dir: &Path) -> AppPaths {
        AppPaths {
            config_dir: dir.to_path_buf(),
            state_dir: dir.to_path_buf(),
            cache_dir: dir.to_path_buf(),
            projects_file: dir.join("projects.json"),
            settings_file: dir.join("settings.json"),
            runtime_state_file: dir.join("runtime-state.json"),
            default_data_root: dir.to_path_buf(),
            log_dir: dir.to_path_buf(),
        }
    }

    #[cfg(unix)]
    fn unique_test_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-stage10-{}-{}-{}",
            label,
            nanos,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create test dir");
        dir
    }
}
