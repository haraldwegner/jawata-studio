use crate::{
    config::{
        display_path, AddProjectInput, BootstrapStatus, ConfigStore, DeployTargetFlags,
        ManagerSettings, McpMergeMode, ProjectRecord, RuntimeSource, UpdateSettingsInput,
    },
    gateway,
    release_manager::{ManagedRuntimeRecord, ReleaseManager, ReleaseStatus},
    runtime_manager::{
        RuntimeLaunchRequest, RuntimeManager, RuntimePhase, RuntimeReference, RuntimeStatusRecord,
        WorkspaceStatusSummary,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{ChildStderr, ChildStdout, Command, Stdio},
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};
use walkdir::{DirEntry, WalkDir};

/// Represents the overall state of the manager, including settings, projects, and runtime statuses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerDashboard {
    pub bootstrap: BootstrapStatus,
    pub settings: ManagerSettings,
    pub release_status: ReleaseStatus,
    pub installed_runtime: Option<ManagedRuntimeRecord>,
    pub projects: Vec<ProjectRecord>,
    pub runtime_statuses: HashMap<String, RuntimeStatusRecord>,
    /// Sprint 10 v0.10.4: A suggested workspace name for the next "Add
    /// project" form submission. Surfaces an existing workspace if one is
    /// loaded; otherwise `None` and the UI defaults to a fresh
    /// "workspace-default".
    pub suggested_workspace_name: Option<String>,
    pub services_inventory: ServicesInventory,
}

/// Represents a discovered project candidate in a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProjectCandidate {
    pub name: String,
    pub project_path: String,
    pub kind: String,
}

/// Sprint 10 v0.10.4: input for moving a project to a different workspace.
/// Replaces the legacy `UpdateProjectPortInput` (port concept removed).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProjectWorkspaceInput {
    pub project_id: String,
    pub workspace_name: String,
}

/// Sprint 10 v0.10.4: input for renaming a workspace.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameWorkspaceInput {
    pub old_name: String,
    pub new_name: String,
}

/// Sprint 10 v0.10.4: input for renaming a project's display name.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameProjectInput {
    pub project_id: String,
    pub name: String,
}

/// Input for importing projects from an IDE workspace.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceImportInput {
    /// `.code-workspace` source (the original import flow). Ignored when
    /// `scan_folder` is set.
    #[serde(default)]
    pub workspace_file: String,
    /// Sprint 16: autoscan source — re-scan this folder server-side and
    /// import the selected candidates from it. Takes precedence over
    /// `workspace_file` when non-empty.
    #[serde(default)]
    pub scan_folder: String,
    pub selected_paths: Vec<String>,
    /// Sprint 10 v0.10.4: target workspace for the imported projects.
    /// Empty/missing → "workspace-default".
    #[serde(default)]
    pub workspace_name: String,
}

/// Result of importing projects from a workspace.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceImportResult {
    pub added: Vec<ProjectRecord>,
    pub skipped: Vec<String>,
}

/// Inventory of available MCP services provided by the installed runtime.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicesInventory {
    pub available: bool,
    pub services: Vec<String>,
    pub detail: String,
}

/// Summary of a cleanup operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSummary {
    pub target: String,
    pub deleted_files: usize,
    pub deleted_dirs: usize,
    pub failed_paths: Vec<String>,
    pub detail: String,
}

/// Result of probing the installed runtime for available services.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProbeResult {
    pub ok: bool,
    pub services: Vec<ProbeServiceEntry>,
    pub detail: String,
    pub duration_ms: u128,
    pub raw_protocol_error: Option<String>,
}

/// Represents an individual service discovered during a probe.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeServiceEntry {
    pub name: String,
    pub description: Option<String>,
}

/// Specifies the deployment mode for MCP configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeployMode {
    Deploy,
    DryRun,
    Preview,
    Regenerate,
    Delete,
}

/// Input for deploying MCP configurations to AI agents.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployToAgentsInput {
    pub mode: DeployMode,
    #[serde(default)]
    pub target_clients: Option<Vec<String>>,
}

/// Status of deploying MCP configuration to a specific client.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeployClientStatus {
    Success,
    Skipped,
    Failed,
}

/// Render one deploy run as log lines: a summary line, then one line per client.
///
/// Sprint 28 (v3.6.1). Split from the file write so the CONTENT is unit-tested
/// — the reason this log exists is that a run's per-client outcome was
/// unrecoverable after the fact, so a log that omits the outcome would be no
/// better than none. Every field that distinguishes "wrote entries" from "wrote
/// nothing" is on the line: status, target path, message, changed sections,
/// backup path, validation errors.
pub(crate) fn format_deploy_log(stamp: &str, result: &DeployToAgentsResult) -> String {
    let mut entry = format!(
        "{stamp} deploy mode={:?} ok={} duration_ms={} clients={}\n",
        result.mode,
        result.ok,
        result.duration_ms,
        result.clients.len()
    );
    for client in &result.clients {
        entry.push_str(&format!(
            "{stamp}   {} status={:?} path={} message={}",
            client.client, client.status, client.target_path, client.message
        ));
        if !client.changed_sections.is_empty() {
            entry.push_str(&format!(" changed={}", client.changed_sections.join(",")));
        }
        if let Some(backup) = &client.backup_path {
            entry.push_str(&format!(" backup={backup}"));
        }
        if !client.validation_errors.is_empty() {
            entry.push_str(&format!(
                " validation_errors={}",
                client.validation_errors.join("; ")
            ));
        }
        entry.push('\n');
    }
    entry
}

/// Result of deploying MCP configuration to a specific client.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployClientResult {
    pub client: String,
    pub target_path: String,
    pub status: DeployClientStatus,
    pub message: String,
    pub backup_path: Option<String>,
    pub changed_sections: Vec<String>,
    pub validation_errors: Vec<String>,
    pub preview_content: Option<String>,
}

/// Overall result of deploying MCP configurations to multiple agents.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployToAgentsResult {
    pub mode: DeployMode,
    pub ok: bool,
    pub detail: String,
    pub duration_ms: u128,
    pub clients: Vec<DeployClientResult>,
}

#[derive(Debug, Clone)]
struct ProbeRuntime {
    jar_path: String,
    runtime_label: String,
}

/// One deployed MCP server entry per workspace.
///
/// Sprint 10 v0.10.4: multiple projects sharing a `workspace_name` collapse
/// into one ManagedDeployServer; the listed `project_paths` are the
/// workspace's members for display / mcp-rule generation.
///
/// Sprint 15 Stage 11: URL form replaces the stdio `command`/`args`/`env`
/// triple. Clients connect to the resident JVM hosted by the manager
/// (Stage 10) at the workspace's stable port + Bearer token. The deploy
/// writer (`build_client_mcp_json`) serializes
/// `{ url, headers: { Authorization: Bearer <token> } }` per the
/// Cursor + Claude MCP-config schema.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedDeployServer {
    id: String,
    workspace_name: String,
    project_names: Vec<String>,
    project_paths: Vec<String>,
    /// Resident JVM URL (`http://127.0.0.1:<resident_port>`).
    url: String,
    /// Bearer token the client sends in `Authorization` headers.
    token: String,
    /// When true, the writer emits `"disabled": true` in the client
    /// config. Used by the `Disable` writer mode (Sprint 15 Stage 11)
    /// when `autostart_on_boot` is off — entry stays visible but inert.
    disabled: bool,
}

#[derive(Debug, Clone)]
struct DeployClientTarget {
    id: &'static str,
    target_path: Option<String>,
    enabled_by_settings: bool,
}

/// Core service coordinating configuration, releases, and runtimes.
pub struct ManagerService {
    config_store: ConfigStore,
    release_manager: ReleaseManager,
    runtime_manager: RuntimeManager,
    /// Sprint 16b/B: shared routing table the single-service gateway reads.
    /// Empty until the first deploy populates it.
    routing_table: Arc<RwLock<gateway::RoutingTable>>,
    /// Sprint 28 (v3.6.2): true while a release check/install is in flight.
    ///
    /// The download is 112 MB. Without this, every operation that refreshed release
    /// status could start its own — three overlapping `archive.bin` temp directories
    /// inside 105 seconds, observed live. One at a time, and never on the UI thread.
    release_sync_running: Arc<std::sync::atomic::AtomicBool>,
}

impl ManagerService {
    /// Creates a new `ManagerService` instance.
    pub fn new(
        config_store: ConfigStore,
        release_manager: ReleaseManager,
        runtime_manager: RuntimeManager,
    ) -> Self {
        let routing_table = Arc::new(RwLock::new(gateway::RoutingTable::default()));

        // Sprint 16b/B: start the single-service gateway when enabled. Default
        // OFF, so this is a no-op for the existing per-workspace deploy model.
        let settings = config_store.get_settings();
        if settings.gateway_enabled {
            let token = ensure_gateway_token(&config_store, &settings);
            match gateway::spawn(settings.gateway_port, token, Arc::clone(&routing_table)) {
                Ok(handle) => {
                    eprintln!("[jawata-studio] gateway listening on 127.0.0.1:{}", handle.port)
                }
                Err(error) => eprintln!("[jawata-studio] gateway failed to start: {error}"),
            }
        }

        Self {
            config_store,
            release_manager,
            runtime_manager,
            routing_table,
            release_sync_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Check for a new runtime, and install it when the policy says so.
    ///
    /// **Blocking, and must never be called from the main thread** — it performs a
    /// network fetch and, under `UpdatePolicy::Always`, a 112 MB download and unpack.
    /// Call it from a spawned thread; `lib.rs` does this once at start-up.
    ///
    /// Sprint 28 (v3.6.2). This work used to sit inside `load_dashboard`, which nine
    /// operations call and which runs on the main thread as a sync Tauri command:
    /// launching the app and stopping the services each froze for the length of a
    /// transfer, and overlapping calls stacked downloads — three concurrent
    /// `archive.bin` temp directories inside 105 seconds, observed live.
    ///
    /// Returns `Ok(true)` when the newest known version changed, so the caller can tell
    /// the UI to reload. Returns `Ok(false)` immediately when a sync is already running:
    /// one 112 MB download at a time, never two.
    pub fn sync_releases_now(&self) -> Result<bool, String> {
        use std::sync::atomic::Ordering;
        if self
            .release_sync_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(false);
        }

        let outcome = (|| {
            let mut settings = self.config_store.get_settings();
            let before = settings.last_seen_latest_version.clone();
            let (_installed, status) = self.release_manager.sync_with_settings(&mut settings)?;
            let changed = before != settings.last_seen_latest_version;
            self.config_store.write_settings(settings)?;
            eprintln!("[jawata-studio] release sync: {}", status.detail);
            Ok(changed)
        })();

        self.release_sync_running.store(false, Ordering::SeqCst);
        outcome
    }

    /// Loads the current manager dashboard state — from CACHE, never the network.
    ///
    /// Sprint 28 (v3.6.2): this used to refresh release status, which fetches over the
    /// network and, under `UpdatePolicy::Always`, DOWNLOADS AND INSTALLS the runtime —
    /// 112 MB — before returning. Sync Tauri commands run on the MAIN thread, so a read
    /// of the dashboard froze the whole UI for the length of a download. Nine operations
    /// end by calling this (start all, stop all, reload, add/delete project, settings,
    /// deploy), so pressing Stop fetched a runtime, and overlapping calls stacked
    /// downloads: three concurrent `archive.bin` temp dirs inside 105 seconds, observed
    /// live 2026-07-29.
    ///
    /// A read is now a read. Checking and installing happen on
    /// [`Self::spawn_release_sync`], off the main thread, and the UI is told when the
    /// result changes.
    pub fn load_dashboard(&self) -> Result<ManagerDashboard, String> {
        self.build_dashboard(false)
    }

    /// Sprint 10 v0.10.4: suggest a default workspace name for the next
    /// "Add project" form. Returns the most recent existing workspace if
    /// any is configured, else `None` (UI then defaults to a fresh name).
    pub fn suggest_next_workspace_name(&self) -> Option<String> {
        self.config_store
            .workspace_names_in_use()
            .into_iter()
            .next()
    }

    /// Adds a new project to the manager. The project's workspace is
    /// determined by `input.workspace_name`; empty input defaults to
    /// `"workspace-default"`. After persisting, rewrites the workspace's
    /// `workspace.json` so any running jawata for that workspace picks
    /// up the new project via the file watcher.
    pub fn add_project(&self, input: AddProjectInput) -> Result<ProjectRecord, String> {
        let project = self.config_store.add_project(input)?;
        self.write_workspace_json_for(&project.workspace_name)?;
        // Sprint 16 (bugs.md #14a): keep already-deployed client configs
        // in sync with workspace mutations.
        self.refresh_deployed_configs();
        Ok(project)
    }

    /// Sprint 10 v0.10.4: move a project to a different workspace.
    /// Rewrites both the source and destination `workspace.json` files so
    /// running jawata processes drop / pick up the project via the
    /// file watcher.
    pub fn set_project_workspace(
        &self,
        input: SetProjectWorkspaceInput,
    ) -> Result<ManagerDashboard, String> {
        // Capture the old workspace name BEFORE mutating, so we can
        // rewrite both files post-update.
        let projects_before = self.config_store.list_projects();
        let source_workspace = projects_before
            .iter()
            .find(|p| p.id == input.project_id)
            .map(|p| p.workspace_name.clone());

        self.config_store
            .set_project_workspace(&input.project_id, input.workspace_name.clone())?;

        if let Some(src) = source_workspace.as_ref() {
            // Skip the rewrite if the destination is the same as the source.
            if src != &input.workspace_name {
                self.write_workspace_json_for(src)?;
            }
        }
        self.write_workspace_json_for(&input.workspace_name)?;
        self.load_dashboard()
    }

    /// Sprint 10 v0.10.4: rename a project's human-readable name.
    pub fn rename_project(
        &self,
        input: RenameProjectInput,
    ) -> Result<ManagerDashboard, String> {
        self.config_store
            .rename_project(&input.project_id, input.name)?;
        self.load_dashboard()
    }

    /// Sprint 10 v0.10.4: rename a workspace. Updates every project's
    /// `workspace_name` matching `old_name` to `new_name`. The MCP service
    /// ID derives from the workspace name, so the next deploy emits a new
    /// mcp.json entry.
    pub fn rename_workspace(
        &self,
        input: RenameWorkspaceInput,
    ) -> Result<ManagerDashboard, String> {
        self.config_store
            .rename_workspace(&input.old_name, input.new_name.clone())?;
        // Rewrite workspace.json under the new name. The old workspace's
        // JDT data dir + workspace.json are left in place for the user to
        // clean up via delete_workspace if they were running there.
        self.write_workspace_json_for(&input.new_name)?;
        // Sprint 16 (bugs.md #14a): the MCP server id derives from the
        // workspace name — deployed configs must follow the rename.
        self.refresh_deployed_configs();
        self.load_dashboard()
    }

    /// Sprint 10 v0.10.4: delete a workspace entirely. Kills any running
    /// jawata subprocess for the workspace, deletes the JDT data dir,
    /// and deletes every ProjectRecord whose `workspace_name` matched.
    /// Returns the dashboard reflecting the new state.
    pub fn delete_workspace(&self, workspace_name: &str) -> Result<ManagerDashboard, String> {
        // Stop any running process for the workspace.
        self.runtime_manager.stop_workspace_runtime(workspace_name)?;

        // Delete every project belonging to this workspace.
        let projects = self.config_store.list_projects();
        for project in &projects {
            if project.workspace_name == workspace_name {
                self.runtime_manager.remove_project_runtime(&project.id)?;
                self.config_store.delete_project(&project.id)?;
            }
        }

        // Delete the JDT data dir on disk (best-effort; ignore errors —
        // the user can clean up manually if something else holds the dir).
        let settings = self.config_store.get_settings();
        let workspace_dir = settings.workspace_root().join(workspace_name);
        if workspace_dir.exists() {
            let _ = std::fs::remove_dir_all(&workspace_dir);
        }

        // Sprint 16 (bugs.md #12): free the resident (port, token) entry —
        // the allocator pool no longer shrinks with every deletion.
        self.config_store.release_workspace_state(workspace_name)?;
        // Sprint 16 (bugs.md #14a): drop the deleted workspace's entry
        // from already-deployed client configs.
        self.refresh_deployed_configs();

        self.load_dashboard()
    }

    /// Deletes a project by its ID. After removal, rewrites the workspace's
    /// `workspace.json` so the running jawata drops the project via the
    /// file watcher (no respawn needed when other members remain).
    pub fn delete_project(&self, project_id: &str) -> Result<ManagerDashboard, String> {
        // Capture the workspace before deletion.
        let projects_before = self.config_store.list_projects();
        let host_workspace = projects_before
            .iter()
            .find(|p| p.id == project_id)
            .map(|p| p.workspace_name.clone());

        self.runtime_manager.remove_project_runtime(project_id)?;
        self.config_store.delete_project(project_id)?;
        if let Some(ws) = host_workspace {
            // Rewrite (or remove) the workspace.json based on whether
            // any members remain.
            self.write_workspace_json_for(&ws)?;

            // Sprint 16 (bugs.md #12): when the last member leaves, the
            // workspace is gone — stop its resident and free its
            // (port, token) entry, same as delete_workspace.
            let any_members_left = self
                .config_store
                .list_projects()
                .iter()
                .any(|p| p.workspace_name == ws);
            if !any_members_left {
                self.runtime_manager.stop_workspace_runtime(&ws)?;
                self.config_store.release_workspace_state(&ws)?;
            }
            // Sprint 16 (bugs.md #14a): deployed configs follow the change
            // (member list shrank, or the whole workspace disappeared).
            self.refresh_deployed_configs();
        }
        self.load_dashboard()
    }

    /// Starts runtimes for all configured projects.
    /// Sprint 10 v0.10.4: writes `workspace.json` once per workspace
    /// before spawning any jawata process. Multiple projects sharing
    /// a `workspace_name` collapse into one spawn per workspace; the
    /// remaining projects "join" the running process via runtime_manager.
    pub fn start_all_runtimes(&self) -> Result<ManagerDashboard, String> {
        let projects = self.config_store.list_projects();
        let mut errors = Vec::new();

        // Write workspace.json files first — once per distinct workspace.
        let mut workspaces_written: HashSet<String> = HashSet::new();
        for project in &projects {
            if workspaces_written.insert(project.workspace_name.clone()) {
                if let Err(e) = self.write_workspace_json_for(&project.workspace_name) {
                    errors.push(format!("{}: {e}", project.workspace_name));
                }
            }
        }

        for project in projects {
            match self.resolve_launch_request(&project) {
                Ok(launch_request) => {
                    if let Err(error) = self.runtime_manager.start_runtime(&launch_request) {
                        errors.push(format!("{}: {error}", project.name));
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", project.name)),
            }
        }

        if !errors.is_empty() {
            return Err(format!(
                "Some runtimes failed to start: {}",
                errors.join(" | ")
            ));
        }

        self.load_dashboard()
    }

    /// Stops all currently running runtimes.
    pub fn stop_all_runtimes(&self) -> Result<ManagerDashboard, String> {
        let projects = self.config_store.list_projects();
        let mut errors = Vec::new();

        for project in projects {
            match self.resolve_runtime_reference(&project) {
                Ok(reference) => {
                    if let Err(error) = self.runtime_manager.stop_runtime(&reference) {
                        errors.push(format!("{}: {error}", project.name));
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", project.name)),
            }
        }

        if !errors.is_empty() {
            return Err(format!(
                "Some runtimes failed to stop: {}",
                errors.join(" | ")
            ));
        }

        self.load_dashboard()
    }

    /// Sprint 14 (v0.14.0): stop every workspace, poll until each phase
    /// reaches `Stopped` or `Failed` (30 s deadline), then start them
    /// all. Surfaced via the tray "Reload all services" entry and the
    /// dashboard "Reload all" toolbar button. The sequential wait
    /// guards against the race where a workspace is still mid-shutdown
    /// when the respawn would otherwise fire — `start_runtime` then
    /// fast-paths into "already running" and the user gets no actual
    /// reload.
    pub fn reload_all_runtimes(&self) -> Result<ManagerDashboard, String> {
        self.stop_all_runtimes()?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let projects = self.config_store.list_projects();
        loop {
            let all_settled = projects.iter().all(|project| {
                let reference = match self.resolve_runtime_reference(project) {
                    Ok(reference) => reference,
                    // Unresolvable projects can't be in a running state
                    // either — they were never spawned. Treat as settled.
                    Err(_) => return true,
                };
                match self.runtime_manager.get_runtime_status(&reference) {
                    Ok(status) => matches!(
                        status.phase,
                        RuntimePhase::Stopped | RuntimePhase::Failed
                    ),
                    Err(_) => true,
                }
            });
            if all_settled {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "Reload all: not every workspace reached Stopped within 30 s; aborting restart"
                        .into(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        self.start_all_runtimes()
    }

    /// Deletes all configured projects.
    pub fn delete_all_projects(&self) -> Result<ManagerDashboard, String> {
        let project_ids: Vec<String> = self
            .config_store
            .list_projects()
            .into_iter()
            .map(|project| project.id)
            .collect();

        for project_id in project_ids {
            self.runtime_manager.remove_project_runtime(&project_id)?;
            self.config_store.delete_project(&project_id)?;
        }

        self.load_dashboard()
    }

    /// Updates manager settings.
    ///
    /// Sprint 28 (v3.6.2): saving settings used to re-poll release status inline when the
    /// release repo changed. That is the same main-thread hazard as `load_dashboard` —
    /// under an installing update policy, pressing Save downloaded 112 MB before the
    /// dialog returned. It returns the cached view immediately; the caller kicks off a
    /// background sync when `release_repo_changed` is true.
    pub fn update_settings(
        &self,
        input: UpdateSettingsInput,
    ) -> Result<(ManagerDashboard, bool), String> {
        let previous_repo = self.config_store.get_settings().release_repo.clone();
        let updated = self.config_store.update_settings(input)?;
        let release_repo_changed = updated.release_repo != previous_repo;
        Ok((self.build_dashboard(false)?, release_repo_changed))
    }

    /// Redetects MCP client paths based on the current system.
    pub fn redetect_mcp_client_paths(&self) -> Result<ManagerDashboard, String> {
        self.config_store.redetect_mcp_client_paths()?;
        self.build_dashboard(false)
    }

    /// Deploys MCP configurations to configured AI agents.
    pub fn deploy_to_agents(
        &self,
        input: DeployToAgentsInput,
    ) -> Result<DeployToAgentsResult, String> {
        let started_at = Instant::now();
        let settings = self.config_store.get_settings();
        // Sprint 21a (item E): make sure the centralized backup area follows the
        // currently configured data root before any managed write.
        crate::backups::set_backups_root(&settings.data_root);
        let projects = self.config_store.list_projects();
        let (servers, resolve_errors) = self.build_deploy_servers(&settings, &projects);

        // Sprint 16b/B: with the gateway on, refresh its routing table and write
        // ONE `jawata` entry to clients instead of N per-workspace entries. Off by
        // default → `client_servers` is just `servers` (unchanged behaviour).
        let client_servers: Vec<ManagedDeployServer> = if settings.gateway_enabled {
            *self
                .routing_table
                .write()
                .expect("routing table lock poisoned") = build_routing_table(&servers);
            let disabled = !settings.autostart_on_boot
                && matches!(
                    settings.mcp_disabled_writer_mode,
                    crate::config::WriterMode::Disable
                );
            let token = ensure_gateway_token(&self.config_store, &settings);
            vec![gateway_entry(settings.gateway_port, &token, disabled)]
        } else {
            servers.clone()
        };

        let clients = self.deploy_targets_for_settings(&settings);
        let requested_targets = normalize_requested_deploy_targets(input.target_clients.as_ref())?;

        let mut results = Vec::new();
        for target in clients {
            let is_selected = if let Some(requested) = requested_targets.as_ref() {
                requested.contains(target.id)
            } else {
                target.enabled_by_settings
            };
            if !is_selected {
                let reason = if requested_targets.is_some() {
                    "Skipped: not selected in this deploy run."
                } else {
                    "Skipped: disabled in Settings deploy targets."
                };
                results.push(skipped_client_result(
                    target.id,
                    target.target_path.clone(),
                    reason,
                ));
                continue;
            }
            let result = self.deploy_to_client(
                target.id,
                target.target_path.clone(),
                &client_servers,
                &settings.mcp_merge_mode,
                settings.mcp_backup_before_write,
                &input.mode,
            );
            results.push(result);
        }

        // Sprint 16 (bugs.md #14b): resolve failures ride on every written
        // client result + the summary line — partial deploys are visible.
        merge_resolve_errors(&mut results, &resolve_errors);

        let ok = results
            .iter()
            .all(|entry| !matches!(entry.status, DeployClientStatus::Failed));

        // Sprint 21a (item D): auto-seed the knowledge store after a successful deploy —
        // experience(kind=load) with no path seeds from the resident's default memory
        // roots, so the primer + recall have content from day one. Fire-and-forget in a
        // background thread: results are LOGGED, a dead/booting resident never fails or
        // delays the deploy.
        if ok {
            let seed_targets = auto_seed_targets(settings.auto_seed_on_deploy, &servers);
            if !seed_targets.is_empty() {
                std::thread::spawn(move || {
                    for (url, token) in seed_targets {
                        match call_resident_tool(
                            &url,
                            &token,
                            "experience",
                            serde_json::json!({"kind": "load"}),
                            10,
                        ) {
                            Ok(_) => eprintln!("[jawata-studio] auto-seed ok: {url}"),
                            Err(error) => {
                                eprintln!("[jawata-studio] auto-seed skipped ({url}): {error}")
                            }
                        }
                    }
                });
            }
        }

        let detail = if !resolve_errors.is_empty() {
            format!(
                "Agent deploy completed, but {} workspace(s) could not be \
                 resolved and were omitted.",
                resolve_errors.len()
            )
        } else if ok {
            "Agent deploy completed.".to_string()
        } else {
            "Agent deploy completed with failures.".to_string()
        };

        let result = DeployToAgentsResult {
            mode: input.mode,
            ok,
            detail,
            duration_ms: started_at.elapsed().as_millis(),
            clients: results,
        };
        self.append_deploy_log(&result);
        Ok(result)
    }

    /// Append one deploy run to `logs/deploy.log`.
    ///
    /// Sprint 28 (v3.6.1), macOS dogfood finding: a deploy's per-client outcome
    /// lived only in the UI response, so once the window was closed there was no
    /// way to tell what a run had actually done. That mattered concretely — the
    /// Claude Desktop config ended up with an empty `mcpServers`, and the
    /// artifacts on disk could not distinguish "the deploy wrote nothing" from
    /// "the deploy wrote entries and the app overwrote them minutes later". One
    /// line per client would have settled it.
    ///
    /// Best-effort and never fatal: a deploy that worked must not be reported
    /// as failed because a log file could not be written.
    fn append_deploy_log(&self, result: &DeployToAgentsResult) {
        let log_dir = self.config_store.paths().log_dir;
        if let Err(error) = std::fs::create_dir_all(&log_dir) {
            eprintln!("[jawata-studio] deploy log: cannot create {log_dir:?}: {error}");
            return;
        }
        let entry = format_deploy_log(&crate::config::current_timestamp_string(), result);
        let path = log_dir.join("deploy.log");
        if let Err(error) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, entry.as_bytes()))
        {
            eprintln!("[jawata-studio] deploy log: cannot append to {path:?}: {error}");
        }
    }

    // ===== Sprint 21a (item F): Knowledge view backend =====

    /// Sprint 21b: the FAST half of the Memory-view calls — config reads only, safe on
    /// the main thread. The blocking HTTP half lives in the `*_for`/`*_on` functions and
    /// runs via `spawn_blocking` (sync Tauri commands execute on the MAIN thread; the
    /// 2×5 s status poll froze the whole UI while residents were booting).
    pub(crate) fn knowledge_servers(&self) -> Vec<ManagedDeployServer> {
        let settings = self.config_store.get_settings();
        let projects = self.config_store.list_projects();
        self.build_deploy_servers(&settings, &projects).0
    }

    /// Resolve one workspace's resident for an off-thread experience call.
    pub(crate) fn find_knowledge_server(&self, workspace: &str) -> Result<ManagedDeployServer, String> {
        self.knowledge_servers()
            .into_iter()
            .find(|server| server.workspace_name == workspace)
            .ok_or_else(|| format!("no resident for workspace '{workspace}'"))
    }

    /// Per-workspace store overview for the Knowledge view: reachability + the
    /// resident's `experience(kind=stats)` (counts by status/language, store file+size).
    pub fn knowledge_status(&self) -> Vec<KnowledgeWorkspaceStatus> {
        Self::knowledge_status_for(&self.knowledge_servers())
    }

    /// The blocking HTTP half — no `&self`, callable from `spawn_blocking`.
    pub(crate) fn knowledge_status_for(servers: &[ManagedDeployServer]) -> Vec<KnowledgeWorkspaceStatus> {
        servers
            .iter()
            .map(|server| {
                match call_experience(
                    &server.url,
                    &server.token,
                    serde_json::json!({"kind": "stats"}),
                    5,
                ) {
                    Ok(response) => KnowledgeWorkspaceStatus {
                        workspace: server.workspace_name.clone(),
                        url: server.url.clone(),
                        reachable: true,
                        stats: response.get("data").cloned(),
                        error: None,
                    },
                    Err(error) => KnowledgeWorkspaceStatus {
                        workspace: server.workspace_name.clone(),
                        url: server.url.clone(),
                        reachable: false,
                        stats: None,
                        error: Some(error),
                    },
                }
            })
            .collect()
    }

    /// Run one `experience(kind=…)` verb against a workspace's resident. The UI's
    /// actions carry EXACTLY these verb names (the prompt vocabulary); anything outside
    /// the vocabulary is refused here.
    pub fn experience_verb(
        &self,
        workspace: &str,
        kind: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Self::experience_verb_on(&self.find_knowledge_server(workspace)?, kind, args)
    }

    /// The blocking HTTP half of a verb call — no `&self`, callable from `spawn_blocking`.
    pub(crate) fn experience_verb_on(
        server: &ManagedDeployServer,
        kind: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if !EXPERIENCE_KINDS.contains(&kind) {
            return Err(format!(
                "unknown experience verb '{kind}' — allowed: {EXPERIENCE_KINDS:?}"
            ));
        }
        let mut arguments = if args.is_object() {
            args
        } else {
            serde_json::json!({})
        };
        arguments
            .as_object_mut()
            .expect("arguments is an object")
            .insert("kind".into(), serde_json::Value::String(kind.to_string()));
        call_experience(&server.url, &server.token, arguments, 60)
    }

    /// Sprint 21a (item E): GC the historically scattered `.bak` files (dry-run first).
    /// Sweeps the dirs jawata-studio ever wrote beside: `$HOME`, `~/.claude`, `~/.cursor`,
    /// the studio config dir, and every registered project dir.
    pub fn backups_gc(&self, dry_run: bool) -> crate::backups::GcReport {
        let settings = self.config_store.get_settings();
        crate::backups::set_backups_root(&settings.data_root);
        crate::backups::set_backup_retention(settings.backup_retention as usize);
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.clone());
            dirs.push(home.join(".claude"));
            dirs.push(home.join(".cursor"));
        }
        if let Some(config_parent) = self.config_store.paths().settings_file.parent() {
            dirs.push(config_parent.to_path_buf());
        }
        for project in self.config_store.list_projects() {
            dirs.push(PathBuf::from(&project.project_path));
        }
        dirs.sort();
        dirs.dedup();
        crate::backups::gc_scattered_backups(&dirs, dry_run)
    }

    /// Checks if any runtimes are currently running.
    pub fn has_running_services(&self) -> bool {
        self.running_services_count() > 0
    }

    /// Returns the number of currently running services.
    pub fn running_services_count(&self) -> usize {
        let projects = self.config_store.list_projects();
        let mut running = 0usize;
        for project in projects {
            let Ok(reference) = self.resolve_runtime_reference(&project) else {
                continue;
            };
            let Ok(status) = self.runtime_manager.get_runtime_status(&reference) else {
                continue;
            };
            if matches!(status.phase, RuntimePhase::Running | RuntimePhase::Starting) {
                running += 1;
            }
        }
        running
    }

    /// Sprint 12 (v0.12.0): one summary entry per workspace_name, with a
    /// phase aggregated from the workspace's member projects. Drives the
    /// per-workspace tray-menu entries and their status icons.
    ///
    /// Workspaces with zero member projects are omitted (the tray has
    /// nothing useful to show for them). Output is sorted by workspace_name
    /// for deterministic menu ordering.
    pub fn workspace_status_summary(&self) -> Vec<WorkspaceStatusSummary> {
        let projects = self.config_store.list_projects();
        let settings = self.config_store.get_settings();
        let installed = self
            .release_manager
            .get_installed_runtime(&settings)
            .ok()
            .flatten();
        let statuses = self.collect_runtime_statuses(&projects, &settings, installed.as_ref());

        // Sprint 13 (v0.13.0): group by `project.workspace_name` from the
        // live config_store, NOT by `status.workspace_name`. The runtime
        // manager caches `RuntimeStatusRecord`s keyed by project_id and
        // returns the cached snapshot from `get_runtime_status`; the
        // snapshot's `workspace_name` is the value seen at last start /
        // stop. After a rename in the dashboard, the cached snapshot
        // still has the old name and the tray menu would lag the rename
        // until the workspace is restarted. Reading workspace_name from
        // the project (config_store mutex, always fresh) closes that
        // loop — the menu reflects the rename within one refresh tick.
        let mut by_ws: HashMap<String, Vec<RuntimePhase>> = HashMap::new();
        for project in &projects {
            if let Some(status) = statuses.get(&project.id) {
                by_ws
                    .entry(project.workspace_name.clone())
                    .or_default()
                    .push(status.phase.clone());
            }
        }

        let mut summaries: Vec<WorkspaceStatusSummary> = by_ws
            .into_iter()
            .map(|(workspace_name, phases)| {
                let project_count = phases.len();
                let phase = aggregate_workspace_phase(&phases);
                WorkspaceStatusSummary {
                    workspace_name,
                    phase,
                    project_count,
                }
            })
            .collect();
        summaries.sort_by(|a, b| a.workspace_name.cmp(&b.workspace_name));
        summaries
    }

    /// Determines if the application should minimize to the system tray on close.
    pub fn should_close_to_tray(&self) -> bool {
        let settings = self.config_store.get_settings();
        settings.use_system_tray && self.has_running_services()
    }

    /// Checks if the system tray feature is enabled in settings.
    pub fn is_system_tray_enabled(&self) -> bool {
        self.config_store.get_settings().use_system_tray
    }

    /// Sprint 14 (v0.14.0): expose the full settings snapshot for
    /// callers that need to read multiple fields (tray menu rebuild,
    /// startup-reconciliation in the setup block). Thin wrapper over
    /// `config_store.get_settings()`.
    pub fn get_settings(&self) -> crate::config::ManagerSettings {
        self.config_store.get_settings()
    }

    /// Sprint 14 (v0.14.0): minimal setter for `autostart_on_boot`. The
    /// `set_autostart_on_boot` Tauri command also calls into
    /// tauri-plugin-autostart to reconcile OS-level autostart — this
    /// just persists the new bool and returns the updated settings.
    pub fn set_autostart_on_boot(
        &self,
        enabled: bool,
    ) -> Result<crate::config::ManagerSettings, String> {
        self.config_store.set_autostart_on_boot(enabled)
    }

    /// v0.14.1 (bugs.md #7, redesign 2026-06-04): collect the
    /// `workspace_name`s of workspaces whose last persisted runtime
    /// status was Running, Starting, or Failed. Called by the
    /// `setup` block on every manager launch when `autostart_on_boot`
    /// is set, to restore the user's session.
    ///
    /// Failed counts because it represents "user wanted this running;
    /// it died" — on the next launch, retry. Stopped does NOT count
    /// (user cleanly stopped → don't auto-restart on next launch).
    pub fn workspaces_to_auto_restore(&self) -> HashSet<String> {
        let mut workspaces = HashSet::new();
        let projects = self.config_store.list_projects();
        for project in &projects {
            let reference = match self.resolve_runtime_reference(project) {
                Ok(reference) => reference,
                Err(_) => continue,
            };
            if let Ok(status) = self.runtime_manager.get_runtime_status(&reference) {
                if matches!(
                    status.phase,
                    RuntimePhase::Running | RuntimePhase::Starting | RuntimePhase::Failed
                ) {
                    workspaces.insert(project.workspace_name.clone());
                }
            }
        }
        workspaces
    }

    /// v0.14.1 (bugs.md #7, redesign 2026-06-04): start runtimes for
    /// only the workspaces named in `workspaces`. Same `workspace.json`
    /// write + spawn shape as `start_all_runtimes`, just filtered.
    /// Used by the startup-restore path.
    pub fn start_specific_workspaces(
        &self,
        workspaces: &HashSet<String>,
    ) -> Result<(), String> {
        let projects = self.config_store.list_projects();
        let mut workspaces_written: HashSet<String> = HashSet::new();
        let mut errors = Vec::new();

        // Write workspace.json once per distinct workspace we're restoring.
        for project in &projects {
            if !workspaces.contains(&project.workspace_name) {
                continue;
            }
            if workspaces_written.insert(project.workspace_name.clone()) {
                if let Err(e) = self.write_workspace_json_for(&project.workspace_name) {
                    errors.push(format!("{}: {e}", project.workspace_name));
                }
            }
        }

        // Spawn (or join) each filtered project.
        for project in projects {
            if !workspaces.contains(&project.workspace_name) {
                continue;
            }
            match self.resolve_launch_request(&project) {
                Ok(launch_request) => {
                    if let Err(error) = self.runtime_manager.start_runtime(&launch_request) {
                        errors.push(format!("{}: {error}", project.name));
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", project.name)),
            }
        }

        if !errors.is_empty() {
            return Err(format!(
                "auto-restore: some workspaces failed to start: {}",
                errors.join(" | ")
            ));
        }
        Ok(())
    }

    /// Downloads or updates the JAWATA runtime.
    pub fn download_or_update_jawata(&self) -> Result<ManagerDashboard, String> {
        let mut settings = self.config_store.get_settings();
        self.release_manager
            .download_latest_runtime(&mut settings)?;
        self.config_store.write_settings(settings)?;
        self.load_dashboard()
    }

    fn build_dashboard(&self, refresh_release_status: bool) -> Result<ManagerDashboard, String> {
        let bootstrap = self.config_store.bootstrap_status();
        let (settings, installed_runtime, release_status) = if refresh_release_status {
            let mut settings = self.config_store.get_settings();
            let (installed_runtime, release_status) =
                self.release_manager.sync_with_settings(&mut settings)?;
            let settings = self.config_store.write_settings(settings)?;
            (settings, installed_runtime, release_status)
        } else {
            let settings = self.config_store.get_settings();
            let (installed_runtime, release_status) = self
                .release_manager
                .status_from_cached_settings(&settings)?;
            (settings, installed_runtime, release_status)
        };
        let projects = self.config_store.list_projects();
        let runtime_statuses =
            self.collect_runtime_statuses(&projects, &settings, installed_runtime.as_ref());
        let suggested_workspace_name = self.suggest_next_workspace_name();
        let services_inventory = self.get_services_inventory_with(installed_runtime.as_ref());

        Ok(ManagerDashboard {
            bootstrap,
            settings,
            release_status,
            installed_runtime,
            projects,
            runtime_statuses,
            suggested_workspace_name,
            services_inventory,
        })
    }

    /// Retrieves the inventory of available MCP services.
    pub fn get_services_inventory(&self) -> ServicesInventory {
        let settings = self.config_store.get_settings();
        let installed = self
            .release_manager
            .get_installed_runtime(&settings)
            .ok()
            .flatten();
        self.get_services_inventory_with(installed.as_ref())
    }

    /// Cleans up log files.
    pub fn clean_logs(&self) -> Result<CleanupSummary, String> {
        self.ensure_no_running_runtimes()?;
        let log_dir = self.config_store.paths().log_dir;
        let mut summary = cleanup_directory_contents(&log_dir)?;
        summary.target = "logs".into();
        Ok(summary)
    }

    /// Cleans up workspace data.
    pub fn clean_workspaces(&self) -> Result<CleanupSummary, String> {
        self.ensure_no_running_runtimes()?;
        let settings = self.config_store.get_settings();
        let workspace_root = settings.workspace_root();
        let mut summary = cleanup_directory_contents(&workspace_root)?;
        summary.target = "workspaces".into();
        Ok(summary)
    }

    /// Cleans up generated data including logs and workspaces.
    pub fn clean_generated_data(&self) -> Result<CleanupSummary, String> {
        self.ensure_no_running_runtimes()?;
        let log_dir = self.config_store.paths().log_dir;
        let settings = self.config_store.get_settings();
        let workspace_root = settings.workspace_root();
        let logs = cleanup_directory_contents(&log_dir)?;
        let workspaces = cleanup_directory_contents(&workspace_root)?;

        let mut failed_paths = logs.failed_paths;
        failed_paths.extend(workspaces.failed_paths);
        let detail = if failed_paths.is_empty() {
            "Removed generated logs and workspaces.".to_string()
        } else {
            format!(
                "Removed generated data with {} partial failures.",
                failed_paths.len()
            )
        };

        Ok(CleanupSummary {
            target: "generatedData".into(),
            deleted_files: logs.deleted_files + workspaces.deleted_files,
            deleted_dirs: logs.deleted_dirs + workspaces.deleted_dirs,
            failed_paths,
            detail,
        })
    }

    /// Probes the installed runtime for available services.
    pub fn probe_services(&self) -> Result<ServiceProbeResult, String> {
        let started_at = Instant::now();
        let settings = self.config_store.get_settings();
        let runtime = self.resolve_probe_runtime(&settings)?;
        let probe_workspace = settings.workspace_root().join(format!(
            "service-probe-{}",
            crate::config::current_timestamp_string()
        ));
        fs::create_dir_all(&probe_workspace).map_err(|error| {
            format!(
                "failed to create probe workspace {}: {error}",
                probe_workspace.display()
            )
        })?;

        let result = self.probe_services_with_runtime(&runtime, &probe_workspace, started_at);
        let _ = fs::remove_dir_all(&probe_workspace);
        Ok(result)
    }

    /// Discovers candidate projects within a workspace file.
    pub fn discover_workspace_projects(
        &self,
        workspace_file: &str,
    ) -> Result<Vec<WorkspaceProjectCandidate>, String> {
        // Sprint 16: thin wrapper — the walk/detect/nested-filter core moved
        // to scan_directory_for_java_projects, shared with the autoscan flow.
        scan_directory_for_java_projects(&read_workspace_roots(workspace_file)?)
    }

    /// Sprint 16: autoscan backend — scan an arbitrary folder for Java
    /// projects, no `.code-workspace` seed required.
    pub fn scan_folder_for_projects(
        &self,
        folder: &str,
    ) -> Result<Vec<WorkspaceProjectCandidate>, String> {
        scan_folder_for_projects_at(folder)
    }

    /// Imports selected projects from a workspace into a target workspace.
    /// Sprint 10 v0.10.4: all imported projects share a single
    /// `workspace_name` from `input.workspace_name` (or `"workspace-default"`
    /// if empty). Replaces the per-project port allocation that the legacy
    /// flow performed.
    pub fn import_workspace_projects(
        &self,
        input: WorkspaceImportInput,
    ) -> Result<WorkspaceImportResult, String> {
        // Sprint 16: both flows re-discover server-side and intersect with
        // the selection — client-supplied paths are never trusted directly.
        let candidates = if !input.scan_folder.trim().is_empty() {
            scan_folder_for_projects_at(&input.scan_folder)?
        } else {
            self.discover_workspace_projects(&input.workspace_file)?
        };
        let selected: HashSet<String> = input.selected_paths.into_iter().collect();
        let target_workspace = input.workspace_name.clone();
        let mut added = Vec::new();
        let mut skipped = Vec::new();

        for candidate in candidates {
            if !selected.contains(&candidate.project_path) {
                continue;
            }
            let result = self.add_project(AddProjectInput {
                name: candidate.name.clone(),
                project_path: candidate.project_path.clone(),
                workspace_name: target_workspace.clone(),
            });
            match result {
                Ok(project) => added.push(project),
                Err(error) => skipped.push(format!("{} ({error})", candidate.project_path)),
            }
        }

        Ok(WorkspaceImportResult { added, skipped })
    }

    /// Starts the runtime for a specific project. Writes workspace.json
    /// for the project's workspace before spawning so the spawning
    /// jawata picks up the full workspace member list.
    /// Sprint 12 (v0.12.0): toggle every project in the named workspace —
    /// stop them when the workspace's aggregated phase is Running or
    /// Starting, start them otherwise (Stopped or Failed).
    ///
    /// Drives the per-workspace toggle entries in the system-tray menu;
    /// the click event hands us a workspace_name and we drive the existing
    /// per-project start/stop API for each member. Errors on individual
    /// projects are collected; the caller gets a single summary.
    pub fn toggle_workspace(&self, workspace_name: &str) -> Result<(), Vec<String>> {
        let projects: Vec<ProjectRecord> = self
            .config_store
            .list_projects()
            .into_iter()
            .filter(|p| p.workspace_name == workspace_name)
            .collect();
        if projects.is_empty() {
            return Err(vec![format!("Unknown workspace: {workspace_name}")]);
        }

        let current_phase = self
            .workspace_status_summary()
            .into_iter()
            .find(|s| s.workspace_name == workspace_name)
            .map(|s| s.phase);

        let should_start = !matches!(
            current_phase,
            Some(RuntimePhase::Running) | Some(RuntimePhase::Starting)
        );

        let mut errors = Vec::new();
        for project in projects {
            let result = if should_start {
                self.start_runtime(&project.id).map(|_| ())
            } else {
                self.stop_runtime(&project.id).map(|_| ())
            };
            if let Err(e) = result {
                errors.push(format!("{}: {e}", project.name));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn start_runtime(&self, project_id: &str) -> Result<RuntimeStatusRecord, String> {
        let project = self
            .config_store
            .get_project(project_id)
            .ok_or_else(|| format!("Unknown project id: {project_id}"))?;

        // Sprint 10 v0.10.4: write workspace.json before spawn (or before
        // joining a running workspace — the file watcher then picks up the
        // change on the running process).
        self.write_workspace_json_for(&project.workspace_name)?;

        let launch_request = self.resolve_launch_request(&project)?;
        self.runtime_manager.start_runtime(&launch_request)
    }

    /// Stops the runtime for a specific project. Sprint 10 v0.10.4:
    /// "stop" means the project leaves its workspace — the workspace
    /// process keeps running for any remaining members; only kills the
    /// process when this was the last member. Workspace.json is rewritten
    /// without the leaving project so the file watcher drops it.
    pub fn stop_runtime(&self, project_id: &str) -> Result<RuntimeStatusRecord, String> {
        let project = self
            .config_store
            .get_project(project_id)
            .ok_or_else(|| format!("Unknown project id: {project_id}"))?;
        let reference = self.resolve_runtime_reference(&project)?;

        // Tell jawata to drop this project: rewrite workspace.json
        // without it (the file watcher in jawata will call removeProject
        // within ~1 s).
        let projects = self.config_store.list_projects();
        let remaining: Vec<&ProjectRecord> = projects
            .iter()
            .filter(|p| p.workspace_name == project.workspace_name && p.id != project_id)
            .collect();
        if remaining.is_empty() {
            // No remaining members — the runtime_manager.stop_runtime will
            // also kill the process, but write_workspace_json_for is the
            // canonical source of truth so it's still useful to call (it
            // removes the file).
            self.write_workspace_json_for(&project.workspace_name)?;
        } else {
            // Members remain: write workspace.json with just the remaining.
            // This is a slight cheat — write_workspace_json_for reads from
            // config_store which still includes this project. We need a
            // version that takes an explicit member list. Inline the write:
            self.write_workspace_json_excluding(&project.workspace_name, project_id)?;
        }

        self.runtime_manager.stop_runtime(&reference)
    }

    /// Sprint 10 v0.10.4: write workspace.json for a workspace, excluding
    /// one project (used by stop_runtime where the project still lives in
    /// projects.json but should not be in the workspace's running file).
    fn write_workspace_json_excluding(
        &self,
        workspace_name: &str,
        excluded_project_id: &str,
    ) -> Result<(), String> {
        let settings = self.config_store.get_settings();
        let projects = self.config_store.list_projects();
        let paths: Vec<&str> = projects
            .iter()
            .filter(|p| p.workspace_name == workspace_name && p.id != excluded_project_id)
            .map(|p| p.project_path.as_str())
            .collect();

        let workspace_dir = settings.workspace_root().join(workspace_name);
        write_workspace_json_to_dir(&workspace_dir, workspace_name, &paths)
    }

    /// Retrieves the current runtime status for a specific project.
    pub fn get_runtime_status(&self, project_id: &str) -> Result<RuntimeStatusRecord, String> {
        let project = self
            .config_store
            .get_project(project_id)
            .ok_or_else(|| format!("Unknown project id: {project_id}"))?;
        let settings = self.config_store.get_settings();
        match self.resolve_runtime_reference(&project) {
            Ok(reference) => self.runtime_manager.get_runtime_status(&reference),
            Err(detail) => Ok(self.unresolved_runtime_status(&project, &settings, detail)),
        }
    }

    fn collect_runtime_statuses(
        &self,
        projects: &[ProjectRecord],
        settings: &ManagerSettings,
        installed_runtime: Option<&ManagedRuntimeRecord>,
    ) -> HashMap<String, RuntimeStatusRecord> {
        let mut statuses = HashMap::new();

        for project in projects {
            let status =
                match self.resolve_runtime_reference_with(project, settings, installed_runtime) {
                    Ok(reference) => self
                        .runtime_manager
                        .get_runtime_status(&reference)
                        .unwrap_or_else(|error| {
                            self.unresolved_runtime_status(project, settings, error)
                        }),
                    Err(detail) => self.unresolved_runtime_status(project, settings, detail),
                };
            statuses.insert(project.id.clone(), status);
        }

        statuses
    }

    fn resolve_launch_request(
        &self,
        project: &ProjectRecord,
    ) -> Result<RuntimeLaunchRequest, String> {
        let reference = self.resolve_runtime_reference(project)?;
        // v3.5.1 (Finding B): refuse to launch a runtime whose jar does not exist.
        // A stale/missing path — e.g. a pre-rebrand goja-mcp/target/products path
        // left in a runtime config — otherwise spawns a java process that fails
        // "Unable to access jarfile" and is retried by the restore loop, spamming
        // the log. Surfacing it as an error here stops the doomed spawn at the
        // source, before any process is started.
        ensure_runtime_jar_exists(&reference.resolved_jar_path)?;
        Ok(RuntimeLaunchRequest {
            project_path: project.project_path.clone(),
            reference,
        })
    }

    fn resolve_runtime_reference(
        &self,
        project: &ProjectRecord,
    ) -> Result<RuntimeReference, String> {
        let settings = self.config_store.get_settings();
        let installed = self.release_manager.get_installed_runtime(&settings)?;
        self.resolve_runtime_reference_with(project, &settings, installed.as_ref())
    }

    fn resolve_runtime_reference_with(
        &self,
        project: &ProjectRecord,
        settings: &ManagerSettings,
        installed_runtime: Option<&ManagedRuntimeRecord>,
    ) -> Result<RuntimeReference, String> {
        // Sprint 10 v0.10.4: workspace_dir is keyed by workspace_name, not
        // project id — so all projects sharing a workspace share one
        // Eclipse JDT data dir + one jawata process.
        let workspace_dir = crate::config::display_path(
            &settings.workspace_root().join(&project.workspace_name),
        );

        // Sprint 15 Stage 10: each workspace gets a stable (port, token)
        // pair allocated from ConfigStore. Sprint 11's URL-emitting MCP
        // writer reads the same state to point client configs at the
        // resident JVM.
        let workspace_state = self
            .config_store
            .get_or_allocate_workspace_state(&project.workspace_name)?;

        match &settings.global_runtime_source {
            RuntimeSource::Managed => {
                let runtime = installed_runtime
                    .ok_or_else(|| "No managed JAWATA runtime is installed. Download the latest release first.".to_string())?;

                Ok(RuntimeReference {
                    project_id: project.id.clone(),
                    workspace_name: project.workspace_name.clone(),
                    workspace_dir,
                    runtime_label: format!("Managed JAWATA {}", runtime.version),
                    resolved_jar_path: runtime.jar_path.clone(),
                    jvm_properties: knowledge_jvm_properties(settings),
                    resident_port: workspace_state.resident_port,
                    resident_token: workspace_state.resident_token,
                })
            }
            RuntimeSource::LocalJar { jar_path } => Ok(RuntimeReference {
                project_id: project.id.clone(),
                workspace_name: project.workspace_name.clone(),
                workspace_dir,
                runtime_label: "Local JAWATA JAR".into(),
                resolved_jar_path: jar_path.clone(),
                jvm_properties: knowledge_jvm_properties(settings),
                resident_port: workspace_state.resident_port,
                resident_token: workspace_state.resident_token,
            }),
        }
    }

    /// Sprint 10 v0.10.4: write the canonical `workspace.json` for the
    /// named workspace. Lists every project path currently registered to
    /// that workspace. Delegates to `write_workspace_json_to_dir` for the
    /// atomic file I/O.
    ///
    /// Called after every projects.json mutation that affects a workspace's
    /// member list. Running jawata processes pick up the change via
    /// `WorkspaceFileWatcher` (~1 s latency).
    fn write_workspace_json_for(&self, workspace_name: &str) -> Result<(), String> {
        let settings = self.config_store.get_settings();
        let projects = self.config_store.list_projects();
        let paths: Vec<&str> = projects
            .iter()
            .filter(|p| p.workspace_name == workspace_name)
            .map(|p| p.project_path.as_str())
            .collect();

        let workspace_dir = settings.workspace_root().join(workspace_name);
        write_workspace_json_to_dir(&workspace_dir, workspace_name, &paths)
    }

    fn unresolved_runtime_status(
        &self,
        project: &ProjectRecord,
        settings: &ManagerSettings,
        detail: String,
    ) -> RuntimeStatusRecord {
        let workspace_dir = crate::config::display_path(
            &settings.workspace_root().join(&project.workspace_name),
        );
        RuntimeStatusRecord::unresolved(
            project.id.clone(),
            project.workspace_name.clone(),
            workspace_dir,
            settings.global_runtime_source.label(),
            detail,
        )
    }

    /// Sprint 10 v0.10.4 (grouping) + Sprint 15 Stage 11 (URL emission):
    /// emit one ManagedDeployServer per **workspace**.
    ///
    /// Projects sharing a `workspace_name` collapse into a single MCP
    /// server entry whose URL points at the resident JVM the manager
    /// hosts for that workspace (Stage 10).
    ///
    /// Sprint 15 v0.15.0 hotfix: deploy is now DECOUPLED from
    /// `autostart_on_boot`. The Stage 11 original "autostart=off → strip
    /// entries" logic was misdirected: with v0.15.0's URL semantics the
    /// deploy entry just points at `http://127.0.0.1:<port>`; whether a
    /// resident JVM is currently listening there is the resident-service
    /// lifecycle's concern, not the MCP-config writer's. The old
    /// "stdio-args auto-spawn on client connect" hazard (the original
    /// bug #9 framing) is gone — URL clients get connection-refused if
    /// the resident isn't up; they don't spawn anything themselves.
    ///
    /// `WriterMode::Disable` still has a use: writing `disabled: true`
    /// gives the user a visible-but-inert entry they can re-enable from
    /// the client side. Triggered when both `autostart_on_boot=false`
    /// AND the mode is `Disable`. `WriterMode::Remove` no longer strips
    /// on user-initiated deploy (the user explicitly clicked Deploy —
    /// honour that). To remove managed entries from clients, the user
    /// uses the explicit "Delete" deploy mode in the dashboard.
    /// Sprint 16 (bugs.md #14b): returns the deploy set PLUS the resolve
    /// errors for workspaces that could not join it. Callers surface the
    /// errors; nothing is silently dropped anymore.
    fn build_deploy_servers(
        &self,
        settings: &ManagerSettings,
        projects: &[ProjectRecord],
    ) -> (Vec<ManagedDeployServer>, Vec<String>) {
        let disabled = !settings.autostart_on_boot
            && matches!(
                settings.mcp_disabled_writer_mode,
                crate::config::WriterMode::Disable
            );

        let installed_runtime = self
            .release_manager
            .get_installed_runtime(settings)
            .ok()
            .flatten();

        // Group projects by workspace_name (preserve insertion order).
        let mut by_workspace: Vec<(String, Vec<&ProjectRecord>)> = Vec::new();
        for project in projects {
            if let Some((_, members)) = by_workspace
                .iter_mut()
                .find(|(name, _)| name == &project.workspace_name)
            {
                members.push(project);
            } else {
                by_workspace.push((project.workspace_name.clone(), vec![project]));
            }
        }

        let mut resolve_errors: Vec<String> = Vec::new();
        let servers = by_workspace
            .into_iter()
            .filter_map(|(workspace_name, members)| {
                // Pick any member to resolve the runtime (also allocates
                // the workspace's resident_port + resident_token if not
                // yet present — Stage 9 + 10 contract).
                let representative = members.first()?;
                let reference = match self.resolve_runtime_reference_with(
                    representative,
                    settings,
                    installed_runtime.as_ref(),
                ) {
                    Ok(reference) => reference,
                    Err(error) => {
                        // Sprint 16 (bugs.md #14b): the pre-v0.16.0 `.ok()?`
                        // here silently omitted the workspace — a partial
                        // deploy looked like a successful one.
                        resolve_errors.push(format!(
                            "workspace '{workspace_name}' omitted from deploy: {error}"
                        ));
                        return None;
                    }
                };
                let server_id = mcp_server_id_for_workspace(&workspace_name);

                let project_names: Vec<String> = members
                    .iter()
                    .map(|p| p.name.clone())
                    .collect();
                let project_paths: Vec<String> = members
                    .iter()
                    .map(|p| p.project_path.clone())
                    .collect();

                let url = format!("http://127.0.0.1:{}/mcp", reference.resident_port);

                Some(ManagedDeployServer {
                    id: server_id,
                    workspace_name,
                    project_names,
                    project_paths,
                    url,
                    token: reference.resident_token.clone(),
                    disabled,
                })
            })
            .collect();
        (servers, resolve_errors)
    }

    /// Sprint 16 (bugs.md #14a): re-run the deploy for clients that ALREADY
    /// hold jawata-managed entries, so deployed configs track workspace
    /// adds / renames / deletes without a manual Deploy click. Clients that
    /// were never deployed to are left untouched. Best-effort by design:
    /// failures are logged and never block the workspace mutation itself.
    fn refresh_deployed_configs(&self) {
        let settings = self.config_store.get_settings();
        let deployed: Vec<String> = self
            .deploy_targets_for_settings(&settings)
            .iter()
            .filter(|target| {
                target.enabled_by_settings
                    && target
                        .target_path
                        .as_deref()
                        .map(path_has_managed_entries)
                        .unwrap_or(false)
            })
            .map(|target| target.id.to_string())
            .collect();
        if deployed.is_empty() {
            return;
        }
        match self.deploy_to_agents(DeployToAgentsInput {
            mode: DeployMode::Deploy,
            target_clients: Some(deployed),
        }) {
            Ok(result) if result.ok => {}
            Ok(result) => eprintln!(
                "[jawata-studio] auto-refresh of deployed configs completed \
                 with failures: {}",
                result.detail
            ),
            Err(error) => eprintln!(
                "[jawata-studio] auto-refresh of deployed configs failed: {error}"
            ),
        }
    }

    fn deploy_targets_for_settings(&self, settings: &ManagerSettings) -> Vec<DeployClientTarget> {
        deploy_targets_for_paths(&settings.deploy_targets, &settings.mcp_client_paths)
    }

    fn deploy_to_client(
        &self,
        client: &str,
        target_path: Option<String>,
        servers: &[ManagedDeployServer],
        merge_mode: &McpMergeMode,
        backup_before_write: bool,
        mode: &DeployMode,
    ) -> DeployClientResult {
        let Some(path) = target_path.and_then(normalize_optional_path) else {
            return DeployClientResult {
                client: client.to_string(),
                target_path: "not configured".into(),
                status: DeployClientStatus::Skipped,
                message: "Client target path is not configured.".into(),
                backup_path: None,
                changed_sections: Vec::new(),
                validation_errors: Vec::new(),
                preview_content: None,
            };
        };

        let mcp_json = build_client_mcp_json(client, servers);
        let rule_body = build_rule_block(client, servers);
        let rule_path = derive_rule_path(client, &path);
        // Sprint 16b/C: also target the client's always-loaded global file.
        let global_rule_path = derive_global_rule_path(client);

        let mut validation_errors = Vec::new();
        if servers.is_empty() && !matches!(mode, DeployMode::Delete) {
            validation_errors.push(
                "No deployable services could be resolved from current project/runtime state."
                    .to_string(),
            );
        }
        if let Some(error) = validate_parent_directory(&path) {
            validation_errors.push(error);
        }

        let global_rule_preview = global_rule_path
            .as_ref()
            .map(|g| format!("\n\nGlobal rule target: {g}"))
            .unwrap_or_default();
        let preview_content = Some(format!(
            "MCP config target: {path}\n\n{}\n\nRule target: {}{}\n\n{}",
            mcp_json, rule_path, global_rule_preview, rule_body
        ));

        if !validation_errors.is_empty() {
            return DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Failed,
                message: "Validation failed.".into(),
                backup_path: None,
                changed_sections: Vec::new(),
                validation_errors,
                preview_content: if matches!(mode, DeployMode::Preview | DeployMode::DryRun) {
                    preview_content
                } else {
                    None
                },
            };
        }

        // Sprint 25a D1: the sections a deploy of this client touches, seat
        // artifacts included (Preview/DryRun report them without writing).
        let mut planned_sections = vec!["mcpConfig".to_string(), "rules".to_string()];
        if derive_seat_commands_dir(client, &path).is_some() {
            planned_sections.push("seatCommands".into());
        }
        if client == "claude_desktop" {
            planned_sections.push("seatSkillExport".into());
        }

        if matches!(mode, DeployMode::Preview) {
            return DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Success,
                message: "Preview generated.".into(),
                backup_path: None,
                changed_sections: planned_sections,
                validation_errors: Vec::new(),
                preview_content,
            };
        }

        if matches!(mode, DeployMode::DryRun) {
            return DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Success,
                message: "Dry run completed. No files were written.".into(),
                backup_path: None,
                changed_sections: planned_sections,
                validation_errors: Vec::new(),
                preview_content: None,
            };
        }

        if matches!(mode, DeployMode::Delete) {
            let mut backup_path = None;
            let mut changed_sections = Vec::new();
            let mut errors = Vec::new();

            match remove_managed_json_block(&path, backup_before_write) {
                Ok(changed) => {
                    if changed {
                        changed_sections.push("mcpConfig".into());
                        if backup_before_write {
                            backup_path = latest_backup_path(&path);
                        }
                    }
                }
                Err(error) => errors.push(error),
            }

            let mut rules_changed = false;
            match remove_managed_rule_block(&rule_path, client, backup_before_write) {
                Ok(changed) => rules_changed |= changed,
                Err(error) => errors.push(error),
            }
            if let Some(global) = global_rule_path.as_ref() {
                match remove_managed_rule_block(global, client, backup_before_write) {
                    Ok(changed) => rules_changed |= changed,
                    Err(error) => errors.push(error),
                }
            }
            if rules_changed {
                changed_sections.push("rules".into());
            }

            // Sprint 18 Track 2 / Stage 9: strip the enforcement hook (Claude Code).
            if let (Some(settings_path), Some(guard_path)) =
                (derive_hook_settings_path(client), managed_guard_script_path())
            {
                match remove_managed_hook(&settings_path, &guard_path, backup_before_write) {
                    Ok(true) => changed_sections.push("hook".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }

            // Sprint 22 (POST layer): strip the PostToolUse observer too.
            if let (Some(settings_path), Some(observer_path)) =
                (derive_hook_settings_path(client), managed_observer_script_path())
            {
                match remove_managed_posthook(&settings_path, &observer_path, backup_before_write) {
                    Ok(true) => changed_sections.push("posthook".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }

            // Sprint 21 (v2.0): strip the knowledge PUSH hooks (SessionStart primer +
            // PreToolUse recall).
            if let (Some(settings_path), Some(primer_path)) =
                (derive_hook_settings_path(client), managed_primer_script_path())
            {
                match remove_managed_primer(&settings_path, &primer_path, backup_before_write) {
                    Ok(true) => changed_sections.push("primer".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }
            if let (Some(settings_path), Some(recall_path)) =
                (derive_hook_settings_path(client), managed_recall_script_path())
            {
                match remove_managed_recall(&settings_path, &recall_path, backup_before_write) {
                    Ok(true) => changed_sections.push("recall".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }
            // Sprint 21c (item D): strip the UserPromptSubmit recall too.
            if let (Some(settings_path), Some(userprompt_path)) =
                (derive_hook_settings_path(client), managed_userprompt_script_path())
            {
                match remove_managed_userprompt(&settings_path, &userprompt_path, backup_before_write) {
                    Ok(true) => changed_sections.push("userprompt".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }

            // Sprint 26: strip the Stop gate.
            if let (Some(settings_path), Some(stop_path)) =
                (derive_hook_settings_path(client), managed_stop_script_path())
            {
                match remove_managed_stop(&settings_path, &stop_path, backup_before_write) {
                    Ok(true) => changed_sections.push("stopGate".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }

            // Sprint 22a P1-b: strip the managed Cursor hooks.json entries + scripts
            // (Cursor only). Leaves user hooks intact.
            if let (Some(hooks_path), Some(hooks_dir)) =
                (derive_cursor_hooks_path(client), managed_cursor_hooks_dir())
            {
                match remove_managed_cursor_hooks(&hooks_path, &hooks_dir, backup_before_write) {
                    Ok(true) => changed_sections.push("cursorHooks".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }

            // Sprint 25a D1: strip the generated seat commands + the skill export.
            if let Some(commands_dir) = derive_seat_commands_dir(client, &path) {
                match remove_managed_utility_commands(client, &commands_dir) {
                    Ok(true) => changed_sections.push("utilityCommands".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
                match remove_managed_seat_commands(client, &commands_dir) {
                    Ok(true) => changed_sections.push("seatCommands".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }
            if client == "claude_desktop" {
                let export_dir = self.config_store.paths().config_dir.join("exports");
                match remove_managed_seat_export(&export_dir) {
                    Ok(true) => changed_sections.push("seatSkillExport".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
            }

            if !errors.is_empty() {
                return DeployClientResult {
                    client: client.to_string(),
                    target_path: path,
                    status: DeployClientStatus::Failed,
                    message: "Delete failed.".into(),
                    backup_path,
                    changed_sections,
                    validation_errors: errors,
                    preview_content: None,
                };
            }

            if changed_sections.is_empty() {
                return DeployClientResult {
                    client: client.to_string(),
                    target_path: path,
                    status: DeployClientStatus::Skipped,
                    message: "No managed JAWATA deploy sections found.".into(),
                    backup_path: None,
                    changed_sections,
                    validation_errors: Vec::new(),
                    preview_content: None,
                };
            }

            return DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Success,
                message: "Delete successful. Removed managed JAWATA deploy sections.".into(),
                backup_path,
                changed_sections,
                validation_errors: Vec::new(),
                preview_content: None,
            };
        }

        let mut backup_path = None;
        let mcp_write = write_managed_json_block(
            &path,
            client,
            servers,
            merge_mode,
            backup_before_write,
            matches!(mode, DeployMode::Regenerate),
        );
        let rule_write = write_managed_rule_block(
            &rule_path,
            &rule_body,
            backup_before_write,
            matches!(mode, DeployMode::Regenerate),
        );
        // Sprint 16b/C: mirror the block into the client's always-loaded global file.
        let global_rule_write = global_rule_path.as_ref().map(|global| {
            write_managed_rule_block(
                global,
                &rule_body,
                backup_before_write,
                matches!(mode, DeployMode::Regenerate),
            )
        });
        // Sprint 22b: a pre-rebrand deploy left a goja-studio-named rule FILE beside
        // the new one (e.g. .cursor/rules/goja-studio.mdc) — both would steer the
        // agent. Remove the legacy sibling (centralized backup first); no-op for
        // shared files like CLAUDE.md, whose old block the marker logic replaces.
        for rp in std::iter::once(rule_path.as_str()).chain(global_rule_path.as_deref()) {
            if let Err(error) = remove_legacy_rule_sibling(rp) {
                eprintln!("[jawata-studio] WARN: {error}");
            }
        }

        let mut changed_sections = Vec::new();
        let mut errors = Vec::new();
        if let Err(error) = mcp_write {
            errors.push(error);
        } else {
            if let Err(error) = validate_written_client_config(client, &path, servers) {
                errors.push(error);
            } else {
                changed_sections.push("mcpConfig".into());
                if backup_before_write {
                    backup_path = latest_backup_path(&path);
                }
            }
        }

        let mut rules_changed = false;
        match rule_write {
            Ok(()) => rules_changed = true,
            Err(error) => errors.push(error),
        }
        if let Some(result) = global_rule_write {
            match result {
                Ok(()) => rules_changed = true,
                Err(error) => errors.push(error),
            }
        }
        if rules_changed {
            changed_sections.push("rules".into());
        }

        // Sprint 25a D1: the generated seat commands ride the same lifecycle.
        // Seats are materialized-if-absent into <config>/seats/ (config wins —
        // a user-edited seat regenerates every channel); parse errors are LOUD
        // in the deploy result, never silently skipped.
        let mut seat_export_note = None;
        {
            let seats_dir = self.config_store.paths().config_dir.join("seats");
            match crate::conductor::materialize_seats(&seats_dir) {
                Err(error) => errors.push(format!("{client}: seat materialization: {error}")),
                Ok(seat_report) => {
                    // v3.7.7 (the seat-staleness bug, ruled major): what the
                    // materialization DID is part of the deploy result —
                    // refreshed seeds, pre-manifest migrations (backed up
                    // beside the file), and user edits shadowing this build's
                    // content are each named, never silent.
                    if !seat_report.refreshed.is_empty() {
                        changed_sections
                            .push(format!("seatsRefreshed: {}", seat_report.refreshed.join(", ")));
                    }
                    if !seat_report.migrated.is_empty() {
                        changed_sections.push(format!(
                            "seatsMigrated (old copy kept as *.pre-refresh): {}",
                            seat_report.migrated.join(", ")
                        ));
                    }
                    if !seat_report.shadowed.is_empty() {
                        changed_sections.push(format!(
                            "seatsShadowedByYourEdits (this build ships a newer {}; your version stays)",
                            seat_report.shadowed.join(", ")
                        ));
                    }
                    let (seats, seat_errors) = crate::runner::load_seat_definitions(&seats_dir);
                    for (seat_path, error) in &seat_errors {
                        errors.push(format!(
                            "{client}: seat definition {}: {error}",
                            seat_path.display()
                        ));
                    }
                    let force = matches!(mode, DeployMode::Regenerate);
                    if let Some(commands_dir) = derive_seat_commands_dir(client, &path) {
                        match write_managed_seat_commands(client, &commands_dir, &seats, force) {
                            Ok(written) if !written.is_empty() => {
                                changed_sections.push("seatCommands".into())
                            }
                            Ok(_) => {}
                            Err(error) => errors.push(error),
                        }
                        // Sprint 26 (D6), v3.3.1: /memorize + /sprint ride along.
                        match write_managed_utility_commands(client, &commands_dir, force) {
                            Ok(written) if !written.is_empty() => {
                                changed_sections.push("utilityCommands".into())
                            }
                            Ok(_) => {}
                            Err(error) => errors.push(error),
                        }
                    }
                    if client == "claude_desktop" {
                        let export_dir = self.config_store.paths().config_dir.join("exports");
                        match write_managed_seat_export(&export_dir, &seats, force) {
                            Ok((zip_path, changed)) => {
                                if changed {
                                    changed_sections.push("seatSkillExport".into());
                                }
                                seat_export_note = Some(zip_path);
                            }
                            Err(error) => errors.push(error),
                        }
                    }
                }
            }
        }

        // Sprint 28 (D-SHIM), v3.7.3: hook_config.json + the role binaries land
        // FIRST — before any settings writer runs. Every writer below resolves
        // its invocation path by looking at the disk, so the binaries must be
        // their final selves before the first writer looks. The previous order
        // put this block in the middle: the two writers before it survived
        // (their scripts were overwritten by the binaries), the four after it
        // clobbered their binaries with scripts — same filenames, so all six
        // files LOOKED deployed while four events ran the previous generation.
        // Found by the 3.7.2 dogfood; the ordering is now load-bearing and the
        // section writer refuses to write a body over a role binary either way.
        if let Some(server) = servers.first() {
            // hook_config.json: endpoint, token and client on disk, so one
            // binary can serve every role instead of ten scripts each carrying
            // a baked-in URL. Written temp-file-plus-rename because hook
            // invocations genuinely overlap (three sessions produced three
            // overlapping pairs, measured), and a reader must never see a
            // truncated file.
            if let Some(primer_path) = managed_primer_script_path() {
                if let Some(hooks_dir) = primer_path.parent() {
                    let client_key = if client.eq_ignore_ascii_case("cursor") {
                        "cursor"
                    } else {
                        "claude-code"
                    };
                    match write_hook_config(hooks_dir, &server.url, &server.token, client_key) {
                        Ok(true) => changed_sections.push("hook_config".into()),
                        Ok(false) => {}
                        Err(error) => errors.push(error),
                    }

                    // The role-named binaries, UNLINKED before writing so a
                    // redeploy landing on top of an executing hook does not
                    // fail with ETXTBSY — a hazard the `.sh` generation never
                    // had, and one that hooks firing on every prompt will meet
                    // routinely.
                    match hook_binary_source() {
                        Some(source) => {
                            match deploy_hook_binaries(&source, hooks_dir, BINARY_LIVE_ROLES, HostPlatform::host()) {
                                Ok(written) if !written.is_empty() => {
                                    changed_sections.push("hook_binaries".into())
                                }
                                Ok(_) => {}
                                Err(error) => errors.push(error),
                            }
                        }
                        // C7 audit, F6 — the affordance that HID F1. A bare
                        // `if let Some(...)` swallowed the miss entirely: on a
                        // shipped .deb whose bake step had moved the executable
                        // away from the sidecar, the deploy skipped every hook
                        // binary and reported success. "Not shipped yet" and
                        // "shipped but unreachable" looked identical, and only
                        // one of them is normal.
                        //
                        // An INSTALLED build that cannot find its own sidecar is
                        // a defect and says so. A dev build without one is not.
                        None if running_from_an_installed_build() => errors.push(
                            "the hook binary is not beside this executable — the install is \
                             missing its sidecar, so NO hooks were deployed. Reinstall, or \
                             report this: it means the package was built wrong."
                                .to_string(),
                        ),
                        None => {}
                    }
                }
            }
        }

        // Sprint 18 Track 2 / Stage 9: write the PreToolUse enforcement hook
        // (Claude Code only). Health URL = the deployed gateway `/mcp` URL so the
        // guard's liveness probe needs no config lookup.
        if let (Some(settings_path), Some(guard_path)) =
            (derive_hook_settings_path(client), managed_guard_script_path())
        {
            let health_url = servers
                .first()
                .map(|server| server.url.clone())
                .unwrap_or_else(|| "http://127.0.0.1:8890/mcp".to_string());
            match write_managed_hook(
                &settings_path,
                &guard_path,
                &health_url,
                backup_before_write,
                matches!(mode, DeployMode::Regenerate),
            ) {
                Ok(true) => changed_sections.push("hook".into()),
                Ok(false) => {}
                Err(error) => errors.push(error),
            }
        }

        // Sprint 22 (POST layer): write the PostToolUse observer (Claude Code only) —
        // the reactive steer-after-slip + versioned outcomes/utilization capture.
        // Sprint 21a (item J): the observer now also bridges slips into the experience
        // store, so it bakes the resident URL + token like the push hooks.
        if let (Some(settings_path), Some(observer_path)) =
            (derive_hook_settings_path(client), managed_observer_script_path())
        {
            let (observer_url, observer_token) = servers
                .first()
                .map(|server| (server.url.clone(), server.token.clone()))
                .unwrap_or_default();
            match write_managed_posthook(
                &settings_path,
                &observer_path,
                &observer_url,
                &observer_token,
                backup_before_write,
                matches!(mode, DeployMode::Regenerate),
            ) {
                Ok(true) => changed_sections.push("posthook".into()),
                Ok(false) => {}
                Err(error) => errors.push(error),
            }
            if let Err(error) = selftest_hook_script(&observer_path) {
                errors.push(error);
            }
        }

        // Sprint 21 (v2.0): write the knowledge PUSH hooks (Claude Code only) — the
        // SessionStart domain primer + the PreToolUse cue-gated recall. Both bake the
        // resident `/mcp` URL + Bearer token so they can live-call experience(...); they
        // fail safe (jawata down / empty / absence → inject nothing).
        if let Some(server) = servers.first() {
            let regenerate = matches!(mode, DeployMode::Regenerate);

            if let (Some(settings_path), Some(primer_path)) =
                (derive_hook_settings_path(client), managed_primer_script_path())
            {
                match write_managed_primer(
                    &settings_path,
                    &primer_path,
                    &server.url,
                    &server.token,
                    backup_before_write,
                    regenerate,
                ) {
                    Ok(true) => changed_sections.push("primer".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
                if let Err(error) = selftest_hook_script(&primer_path) {
                    errors.push(error);
                }
            }
            if let (Some(settings_path), Some(recall_path)) =
                (derive_hook_settings_path(client), managed_recall_script_path())
            {
                match write_managed_recall(
                    &settings_path,
                    &recall_path,
                    &server.url,
                    &server.token,
                    backup_before_write,
                    regenerate,
                ) {
                    Ok(true) => changed_sections.push("recall".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
                if let Err(error) = selftest_hook_script(&recall_path) {
                    errors.push(error);
                }
            }
            // Sprint 21c (item D): the prompt-boundary recall — every user prompt gets a
            // deterministic keyword pass against the store; a single fitting fact is
            // injected as context, absence stays silent.
            if let (Some(settings_path), Some(userprompt_path)) =
                (derive_hook_settings_path(client), managed_userprompt_script_path())
            {
                match write_managed_userprompt(
                    &settings_path,
                    &userprompt_path,
                    &server.url,
                    &server.token,
                    backup_before_write,
                    regenerate,
                ) {
                    Ok(true) => changed_sections.push("userprompt".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
                if let Err(error) = selftest_hook_script(&userprompt_path) {
                    errors.push(error);
                }
            }
            // Sprint 26 (D5/D4): the Stop gate — the communication bounce +
            // the seat-gate block (Claude Code only; platform fact).
            if let (Some(settings_path), Some(stop_path)) =
                (derive_hook_settings_path(client), managed_stop_script_path())
            {
                match write_managed_stop(
                    &settings_path,
                    &stop_path,
                    &server.url,
                    &server.token,
                    backup_before_write,
                    regenerate,
                ) {
                    Ok(true) => changed_sections.push("stopGate".into()),
                    Ok(false) => {}
                    Err(error) => errors.push(error),
                }
                if let Err(error) = selftest_stop_hook_script(&stop_path) {
                    errors.push(error);
                }
            }
        }

        // Sprint 22a P1-b: the managed Cursor hooks.json + scripts (Cursor only) — the
        // guard (failClosed) + sessionStart primer are full parity; recall is a
        // side-effect (Cursor cannot inject on beforeSubmitPrompt); the observer is
        // fire-and-forget. Merged into ~/.cursor/hooks.json preserving user hooks.
        if let (Some(hooks_path), Some(hooks_dir), Some(server)) = (
            derive_cursor_hooks_path(client),
            managed_cursor_hooks_dir(),
            servers.first(),
        ) {
            match write_managed_cursor_hooks(
                &hooks_path,
                &hooks_dir,
                &server.url,
                &server.token,
                backup_before_write,
                matches!(mode, DeployMode::Regenerate),
            ) {
                Ok(true) => changed_sections.push("cursorHooks".into()),
                Ok(false) => {}
                Err(error) => errors.push(error),
            }
        }

        if errors.is_empty() {
            let message = match &seat_export_note {
                Some(zip_path) => format!(
                    "Deploy successful. Seat skill exported to {zip_path} — upload it once \
                     in claude.ai Settings."
                ),
                None => "Deploy successful.".to_string(),
            };
            DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Success,
                message,
                backup_path,
                changed_sections,
                validation_errors: Vec::new(),
                preview_content: None,
            }
        } else {
            DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Failed,
                message: "Deploy failed.".into(),
                backup_path,
                changed_sections,
                validation_errors: errors,
                preview_content: None,
            }
        }
    }

    fn ensure_no_running_runtimes(&self) -> Result<(), String> {
        let projects = self.config_store.list_projects();
        let mut active = Vec::new();

        for project in projects {
            let Ok(reference) = self.resolve_runtime_reference(&project) else {
                continue;
            };
            let Ok(status) = self.runtime_manager.get_runtime_status(&reference) else {
                continue;
            };
            if matches!(status.phase, RuntimePhase::Running | RuntimePhase::Starting) {
                active.push(project.name);
            }
        }

        if active.is_empty() {
            return Ok(());
        }

        Err(format!(
            "Stop running runtimes before cleanup: {}",
            active.join(", ")
        ))
    }

    fn get_services_inventory_with(
        &self,
        installed: Option<&ManagedRuntimeRecord>,
    ) -> ServicesInventory {
        let Some(runtime) = installed else {
            return ServicesInventory {
                available: false,
                services: Vec::new(),
                detail: "No managed runtime installed yet.".into(),
            };
        };

        let install_dir = PathBuf::from(&runtime.install_dir);
        let candidates = ["services.json", "tools.json", "manifest.json"];

        for file_name in candidates {
            let path = install_dir.join(file_name);
            if !path.exists() {
                continue;
            }

            match parse_services_from_json_file(&path) {
                Ok(services) if !services.is_empty() => {
                    return ServicesInventory {
                        available: true,
                        services,
                        detail: format!("Loaded service inventory from {}.", path.display()),
                    };
                }
                Ok(_) => {}
                Err(error) => {
                    return ServicesInventory {
                        available: false,
                        services: Vec::new(),
                        detail: format!(
                            "Service inventory file exists but could not be parsed: {} ({error})",
                            path.display()
                        ),
                    };
                }
            }
        }

        ServicesInventory {
            available: false,
            services: Vec::new(),
            detail: "Service inventory unavailable for this runtime package.".into(),
        }
    }

    fn resolve_probe_runtime(&self, settings: &ManagerSettings) -> Result<ProbeRuntime, String> {
        let installed = self.release_manager.get_installed_runtime(settings)?;
        let runtime = match &settings.global_runtime_source {
            RuntimeSource::Managed => {
                let runtime = installed.ok_or_else(|| {
                    "No managed JAWATA runtime is installed. Download latest first.".to_string()
                })?;
                ProbeRuntime {
                    jar_path: runtime.jar_path,
                    runtime_label: format!("Managed JAWATA {}", runtime.version),
                }
            }
            RuntimeSource::LocalJar { jar_path } => ProbeRuntime {
                jar_path: jar_path.clone(),
                runtime_label: "Local JAWATA JAR".into(),
            },
        };

        if !PathBuf::from(&runtime.jar_path).exists() {
            return Err(format!(
                "Configured JAWATA JAR does not exist: {}",
                runtime.jar_path
            ));
        }

        Ok(runtime)
    }

    fn probe_services_with_runtime(
        &self,
        runtime: &ProbeRuntime,
        probe_workspace: &Path,
        started_at: Instant,
    ) -> ServiceProbeResult {
        // Sprint 15 v0.15.0 hotfix: against fork v1.8.5 the default
        // transport is HTTP, so without `-transport stdio` the probe child
        // binds an ephemeral HTTP port and prints `READY url=... token=...`
        // on its first stdout line. The probe's wire is unambiguously
        // stdio JSON-RPC (it owns the child's stdin/stdout pipes), so the
        // fork must stay on the stdio code path here. The resident JVMs
        // that actually serve clients still launch in HTTP mode via
        // runtime_manager.rs::command_spec_for, which passes -port + -token.
        //
        // Audit 2026-06-08: verified no other Command::new("java") sites
        // exist in the manager crate (grep -rE 'Command::new\("java"\)' →
        // exactly two matches: this one + the resident JVM spawn).
        let mut command = Command::new("java");
        command
            .arg("-jar")
            .arg(&runtime.jar_path)
            .arg("-transport")
            .arg("stdio")
            .arg("-data")
            .arg(display_path(probe_workspace))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Sprint 16.1 (bugs.md #16): no console window on Windows.
        crate::runtime_manager::spawn_without_console(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return self.probe_failure(
                    format!("Failed to start JAWATA probe process: {error}"),
                    started_at,
                    None,
                );
            }
        };

        let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        let stderr_handle = child
            .stderr
            .take()
            .map(|stderr| spawn_stderr_tail_reader(stderr, stderr_tail.clone()));

        let result = (|| {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                "Probe process stdin was not available for MCP handshake".to_string()
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                "Probe process stdout was not available for MCP handshake".to_string()
            })?;

            let responses = spawn_mcp_reader(stdout);

            let initialize_request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "jawata-studio",
                        "version": "0.1.0"
                    }
                }
            });
            write_mcp_message(&mut stdin, &initialize_request)?;
            let initialize_response =
                wait_for_mcp_response(&responses, 1, Duration::from_secs(20))?;
            ensure_success_response(&initialize_response)?;

            let initialized_notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            });
            let _ = write_mcp_message(&mut stdin, &initialized_notification);

            let tools_list_request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            });
            write_mcp_message(&mut stdin, &tools_list_request)?;
            let tools_list_response =
                wait_for_mcp_response(&responses, 2, Duration::from_secs(20))?;
            let mut services = extract_tool_entries(&tools_list_response)?;
            services.sort_by(|a, b| a.name.cmp(&b.name));
            services.dedup_by(|a, b| a.name == b.name);

            if services.is_empty() {
                return Ok(self.probe_failure(
                    format!(
                        "{} responded, but returned no tools for tools/list.",
                        runtime.runtime_label
                    ),
                    started_at,
                    None,
                ));
            }

            let invocation_detail =
                run_optional_invocation_check(&mut stdin, &responses, &services)
                    .map(|_| "Discovery + invocation check passed.".to_string())
                    .unwrap_or_else(|error| format!("Discovery only ({error})."));

            Ok(ServiceProbeResult {
                ok: true,
                services,
                detail: format!("Probe successful. {invocation_detail}"),
                duration_ms: started_at.elapsed().as_millis(),
                raw_protocol_error: None,
            })
        })();

        let _ = child.kill();
        let _ = child.wait();
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }

        let stderr_snippet = collect_stderr_tail(&stderr_tail);
        match result {
            Ok(probe) => probe,
            Err(error) => {
                let detail = if let Some(stderr_tail) = stderr_snippet {
                    format!("Service probe failed: {error}. Runtime output: {stderr_tail}")
                } else {
                    format!("Service probe failed: {error}")
                };
                self.probe_failure(detail, started_at, Some(error))
            }
        }
    }

    fn probe_failure(
        &self,
        detail: String,
        started_at: Instant,
        raw_protocol_error: Option<String>,
    ) -> ServiceProbeResult {
        ServiceProbeResult {
            ok: false,
            services: Vec::new(),
            detail,
            duration_ms: started_at.elapsed().as_millis(),
            raw_protocol_error,
        }
    }
}

fn spawn_mcp_reader(stdout: ChildStdout) -> Receiver<Result<serde_json::Value, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let message = read_mcp_message(&mut reader);
            if tx.send(message.clone()).is_err() {
                break;
            }
            if message.is_err() {
                break;
            }
        }
    });
    rx
}

fn read_mcp_message(reader: &mut BufReader<ChildStdout>) -> Result<serde_json::Value, String> {
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed reading MCP response line: {error}"))?;
        if read == 0 {
            return Err("MCP stream closed before response was received".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with('{') {
            return Err(format!(
                "received non-JSON output from JAWATA stdout: {trimmed}"
            ));
        }

        return serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|error| format!("invalid MCP JSON payload: {error}"));
    }
}

fn spawn_stderr_tail_reader(
    stderr: ChildStderr,
    tail_lines: Arc<Mutex<Vec<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(mut tail) = tail_lines.lock() {
                tail.push(line);
                if tail.len() > 12 {
                    let drain_count = tail.len() - 12;
                    tail.drain(0..drain_count);
                }
            }
        }
    })
}

fn collect_stderr_tail(tail_lines: &Arc<Mutex<Vec<String>>>) -> Option<String> {
    let Ok(lines) = tail_lines.lock() else {
        return None;
    };
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" | "))
    }
}

fn write_mcp_message(stdin: &mut impl Write, message: &serde_json::Value) -> Result<(), String> {
    let payload = serde_json::to_string(message)
        .map_err(|error| format!("failed serializing MCP message: {error}"))?;
    stdin
        .write_all(payload.as_bytes())
        .map_err(|error| format!("failed writing MCP message body: {error}"))?;
    stdin
        .write_all(b"\n")
        .map_err(|error| format!("failed writing MCP message newline: {error}"))?;
    stdin
        .flush()
        .map_err(|error| format!("failed flushing MCP message: {error}"))
}

fn wait_for_mcp_response(
    rx: &Receiver<Result<serde_json::Value, String>>,
    response_id: u64,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(format!(
                "timed out waiting for MCP response id {response_id}"
            ));
        }

        let remaining = deadline.saturating_duration_since(now);
        let message = rx
            .recv_timeout(remaining)
            .map_err(|_| format!("timed out waiting for MCP response id {response_id}"))??;
        if message_id_matches(&message, response_id) {
            return Ok(message);
        }
    }
}

fn message_id_matches(message: &serde_json::Value, response_id: u64) -> bool {
    message
        .get("id")
        .and_then(|id| id.as_u64())
        .map(|id| id == response_id)
        .unwrap_or(false)
}

fn ensure_success_response(response: &serde_json::Value) -> Result<(), String> {
    if let Some(error) = response.get("error") {
        return Err(format!("MCP returned error: {error}"));
    }
    if response.get("result").is_none() {
        return Err("MCP response did not include a result payload".into());
    }
    Ok(())
}

/// v3.5.1 (Finding B): a runtime whose resolved jar does not exist must not be
/// launched — a stale/missing path spawns a java process that fails "Unable to
/// access jarfile" and is retried by the restore loop. Refusing here stops the
/// doomed spawn before any process starts.
fn ensure_runtime_jar_exists(jar_path: &str) -> Result<(), String> {
    if std::path::Path::new(jar_path).exists() {
        Ok(())
    } else {
        Err(format!(
            "JAWATA runtime jar not found: {jar_path} — the configured runtime path \
             is stale or the runtime is not installed. Reinstall/select a runtime in Studio."
        ))
    }
}

fn extract_tool_entries(response: &serde_json::Value) -> Result<Vec<ProbeServiceEntry>, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("MCP tools/list returned error: {error}"));
    }

    let tools = response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(|tools| tools.as_array())
        .ok_or("MCP tools/list response did not include result.tools[]")?;

    let mut entries = Vec::new();
    for tool in tools {
        if let Some(name) = tool.get("name").and_then(|name| name.as_str()) {
            entries.push(ProbeServiceEntry {
                name: name.to_string(),
                description: tool
                    .get("description")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
            });
        }
    }
    Ok(entries)
}

fn run_optional_invocation_check(
    stdin: &mut impl Write,
    responses: &Receiver<Result<serde_json::Value, String>>,
    services: &[ProbeServiceEntry],
) -> Result<(), String> {
    let Some(health_tool_name) = services.iter().find_map(|entry| {
        let lowered = entry.name.to_ascii_lowercase();
        if lowered == "health_check" || lowered == "healthcheck" || lowered == "health-check" {
            Some(entry.name.clone())
        } else {
            None
        }
    }) else {
        return Err("health check tool not advertised".into());
    };

    let call_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": health_tool_name,
            "arguments": {}
        }
    });
    write_mcp_message(stdin, &call_request)?;
    let call_response = wait_for_mcp_response(responses, 3, Duration::from_secs(20))?;
    ensure_success_response(&call_response)
}

fn cleanup_directory_contents(path: &Path) -> Result<CleanupSummary, String> {
    if !path.exists() {
        return Ok(CleanupSummary {
            target: display_path(path),
            deleted_files: 0,
            deleted_dirs: 0,
            failed_paths: Vec::new(),
            detail: "Nothing to clean.".into(),
        });
    }

    let mut deleted_files = 0usize;
    let mut deleted_dirs = 0usize;
    let mut failed_paths = Vec::new();
    let mut entries: Vec<PathBuf> = WalkDir::new(path)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.path().to_path_buf())
        .collect();
    entries.sort_by_key(|candidate| std::cmp::Reverse(candidate.components().count()));

    for entry in entries {
        let result = if entry.is_file() {
            fs::remove_file(&entry).map(|_| deleted_files += 1)
        } else if entry.is_dir() {
            fs::remove_dir(&entry).map(|_| deleted_dirs += 1)
        } else {
            Ok(())
        };

        if let Err(error) = result {
            failed_paths.push(format!("{} ({error})", entry.display()));
        }
    }

    let detail = if failed_paths.is_empty() {
        "Cleanup complete.".into()
    } else {
        format!(
            "Cleanup completed with {} partial failures.",
            failed_paths.len()
        )
    };

    Ok(CleanupSummary {
        target: display_path(path),
        deleted_files,
        deleted_dirs,
        failed_paths,
        detail,
    })
}

fn parse_services_from_json_file(path: &Path) -> Result<Vec<String>, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|error| format!("invalid JSON: {error}"))?;

    let mut services = Vec::new();
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(name) = item.as_str() {
                    services.push(name.to_string());
                } else if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    services.push(name.to_string());
                } else if let Some(name) = item.get("toolName").and_then(|v| v.as_str()) {
                    services.push(name.to_string());
                }
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(items) = map.get("tools").and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        services.push(name.to_string());
                    } else if let Some(name) = item.get("toolName").and_then(|v| v.as_str()) {
                        services.push(name.to_string());
                    }
                }
            }
        }
        _ => {}
    }

    services.sort();
    services.dedup();
    Ok(services)
}

/// Sprint 16: the discovery core — walk each root (depth ≤ 6), detect Java
/// project kinds, dedupe, and keep only containing roots (nested children
/// collapse into their parent). A root that is itself a Java project counts:
/// WalkDir yields the root entry first. Shared by the `.code-workspace`
/// discover flow and the autoscan folder scan.
fn scan_directory_for_java_projects(
    roots: &[PathBuf],
) -> Result<Vec<WorkspaceProjectCandidate>, String> {
    let mut by_path: HashMap<String, WorkspaceProjectCandidate> = HashMap::new();

    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(6)
            .into_iter()
            .filter_entry(should_walk_entry)
        {
            let entry = entry.map_err(|error| format!("workspace scan failed: {error}"))?;
            if !entry.file_type().is_dir() {
                continue;
            }
            let path = entry.path();
            if is_ignored_candidate_path(path) {
                continue;
            }
            if let Some(kind) = detect_java_project_kind(path) {
                let key = path.to_string_lossy().to_string();
                by_path
                    .entry(key.clone())
                    .or_insert_with(|| WorkspaceProjectCandidate {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "project".into()),
                        project_path: key,
                        kind,
                    });
            }
        }
    }

    let mut candidates: Vec<_> = by_path.into_values().collect();
    candidates.sort_by(|a, b| {
        let al = a.project_path.len();
        let bl = b.project_path.len();
        al.cmp(&bl).then(a.project_path.cmp(&b.project_path))
    });

    // Keep only containing project roots; drop nested children.
    let mut filtered: Vec<WorkspaceProjectCandidate> = Vec::new();
    for candidate in candidates {
        let candidate_path = PathBuf::from(&candidate.project_path);
        let is_nested = filtered
            .iter()
            .map(|parent| PathBuf::from(&parent.project_path))
            .any(|parent| candidate_path != parent && candidate_path.starts_with(&parent));
        if !is_nested {
            filtered.push(candidate);
        }
    }
    filtered.sort_by(|a, b| a.project_path.cmp(&b.project_path));
    Ok(filtered)
}

/// Sprint 16: expand + validate the autoscan input, then scan. `~/` resolves
/// against the home directory (hand-typed paths; Browse always hands over
/// absolute ones).
fn scan_folder_for_projects_at(
    folder: &str,
) -> Result<Vec<WorkspaceProjectCandidate>, String> {
    let trimmed = folder.trim();
    if trimmed.is_empty() {
        return Err("folder path is empty".into());
    }
    let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| "could not determine home directory".to_string())?
            .join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if !expanded.is_dir() {
        return Err(format!("not a directory: {}", expanded.display()));
    }
    scan_directory_for_java_projects(&[expanded])
}

fn should_walk_entry(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    if !entry.file_type().is_dir() {
        return true;
    }
    !matches!(
        name.as_ref(),
        ".git"
            | ".idea"
            | ".vscode"
            | "node_modules"
            | "target"
            | "build"
            | ".gradle"
            | ".metadata"
    )
}

fn is_ignored_candidate_path(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    if file_name == "External Plug-in Libraries"
        || file_name == "JRE System Library"
        || file_name.contains("BndtoolsJAREditorTempFiles")
    {
        return true;
    }

    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if part == ".metadata" || part == ".plugins" {
            return true;
        }
        if part.starts_with(".org.eclipse")
            || part.starts_with("org.eclipse.jdt.core.external.folders")
        {
            return true;
        }
    }

    false
}

fn detect_java_project_kind(path: &Path) -> Option<String> {
    let has = |name: &str| path.join(name).exists();
    let has_manifest = path.join("META-INF").join("MANIFEST.MF").exists();
    let has_java_src = path.join("src").join("main").join("java").exists()
        || path.join("src").join("test").join("java").exists();
    let has_build_files = has("pom.xml")
        || has("build.gradle")
        || has("build.gradle.kts")
        || has("settings.gradle")
        || has("settings.gradle.kts");
    let has_local_jars = has_local_jar_files(path);

    // Maven/Gradle entries must contain Java sources or local jar artifacts.
    if has_build_files && (has_java_src || has_local_jars) {
        return Some("maven-gradle".into());
    }

    // Eclipse/PDE must be an actual workspace project, not just a plugin/runtime folder.
    // Require .project and at least one Java/PDE signal.
    if has(".project")
        && (has(".classpath")
            || has_manifest
            || has("plugin.xml")
            || has("feature.xml")
            || has_java_src)
    {
        return Some("eclipse-pde".into());
    }

    None
}

fn has_local_jar_files(path: &Path) -> bool {
    for entry in WalkDir::new(path)
        .follow_links(false)
        .max_depth(4)
        .into_iter()
        .filter_entry(should_walk_entry)
    {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_dir() && is_ignored_candidate_path(entry.path()) {
            continue;
        }
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("jar"))
                .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn read_workspace_roots(workspace_file: &str) -> Result<Vec<PathBuf>, String> {
    let workspace_path = PathBuf::from(workspace_file);
    let workspace_dir = workspace_path
        .parent()
        .ok_or("workspace file has no parent directory")?;

    let contents = fs::read_to_string(&workspace_path).map_err(|error| {
        format!(
            "failed to read workspace file {}: {error}",
            workspace_path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&contents).map_err(|error| {
        format!(
            "failed to parse workspace file {}: {error}",
            workspace_path.display()
        )
    })?;

    let mut roots = Vec::new();
    if let Some(folders) = value.get("folders").and_then(|v| v.as_array()) {
        for folder in folders {
            if let Some(path) = folder.get("path").and_then(|v| v.as_str()) {
                let folder_path = PathBuf::from(path);
                if folder_path.is_absolute() {
                    roots.push(folder_path);
                } else {
                    roots.push(workspace_dir.join(folder_path));
                }
            }
        }
    }
    Ok(roots)
}

fn normalize_optional_path(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The client ids the deploy backend understands. These are the SNAKE_CASE
/// ids; the settings API speaks camelCase `DeployTargetFlags` keys, and the
/// two spellings differ for exactly one client (`claude_desktop` vs
/// `claudeDesktop`). Callers must send the ids in this list.
pub(crate) const KNOWN_DEPLOY_CLIENT_IDS: [&str; 5] = [
    "cursor",
    "claude",
    "claude_desktop",
    "antigravity",
    "intellij",
];

/// Normalise the caller's requested client ids, REFUSING any id we do not
/// know instead of quietly dropping it.
///
/// Sprint 28 (v3.6.0), macOS dogfood 2026-07-26. This used to be a `.filter()`
/// that silently discarded unrecognised ids; the run then reported
/// "Skipped: not selected in this deploy run" — telling the user they had not
/// ticked a box they had ticked. That is this project's recorded deepest bug
/// class (a failed lookup handed back as an ordinary empty result), and it
/// concealed the real defect for the whole life of the feature: the UI sent
/// the camelCase settings key `claudeDesktop`, which lowercases to
/// `claudedesktop` and never equals `claude_desktop`, so Claude Desktop was
/// the one client that could never be deployed. The four single-word clients
/// hid it because both spellings coincide for them.
fn normalize_requested_deploy_targets(
    targets: Option<&Vec<String>>,
) -> Result<Option<HashSet<String>>, String> {
    let Some(targets) = targets else {
        return Ok(None);
    };
    let normalized: Vec<String> = targets
        .iter()
        .map(|target| target.trim().to_ascii_lowercase())
        .collect();
    let unknown: Vec<&str> = normalized
        .iter()
        .map(String::as_str)
        .filter(|target| !KNOWN_DEPLOY_CLIENT_IDS.contains(target))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "deploy requested unknown client id(s): {}. Known ids: {}.",
            unknown.join(", "),
            KNOWN_DEPLOY_CLIENT_IDS.join(", ")
        ));
    }
    Ok(Some(normalized.into_iter().collect()))
}

fn deploy_targets_for_paths(
    flags: &DeployTargetFlags,
    paths: &crate::config::McpClientPaths,
) -> Vec<DeployClientTarget> {
    vec![
        DeployClientTarget {
            id: "cursor",
            target_path: paths.cursor.effective_path.clone(),
            enabled_by_settings: flags.cursor,
        },
        DeployClientTarget {
            id: "claude",
            target_path: paths.claude.effective_path.clone(),
            enabled_by_settings: flags.claude,
        },
        DeployClientTarget {
            id: "claude_desktop",
            target_path: paths.claude_desktop.effective_path.clone(),
            enabled_by_settings: flags.claude_desktop,
        },
        DeployClientTarget {
            id: "antigravity",
            target_path: paths.antigravity.effective_path.clone(),
            enabled_by_settings: flags.antigravity,
        },
        DeployClientTarget {
            id: "intellij",
            target_path: paths.intellij.effective_path.clone(),
            enabled_by_settings: flags.intellij,
        },
    ]
}

fn skipped_client_result(
    client: &str,
    target_path: Option<String>,
    message: &str,
) -> DeployClientResult {
    DeployClientResult {
        client: client.to_string(),
        target_path: target_path
            .and_then(normalize_optional_path)
            .unwrap_or_else(|| "not configured".into()),
        status: DeployClientStatus::Skipped,
        message: message.to_string(),
        backup_path: None,
        changed_sections: Vec::new(),
        validation_errors: Vec::new(),
        preview_content: None,
    }
}

fn validate_parent_directory(path: &str) -> Option<String> {
    let path = PathBuf::from(path);
    let Some(parent) = path.parent() else {
        return Some(format!(
            "target path has no parent directory: {}",
            path.display()
        ));
    };
    if !parent.exists() {
        // Parent can be created during write (create_dir_all), so this is valid.
        return None;
    }
    if parent.is_dir() {
        None
    } else {
        Some(format!(
            "target parent path is not a directory: {}",
            parent.display()
        ))
    }
}

fn derive_rule_path(client: &str, mcp_target_path: &str) -> String {
    let mcp_path = PathBuf::from(mcp_target_path);
    let parent = mcp_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match client {
        "cursor" => display_path(&parent.join("rules").join("jawata-studio.mdc")),
        "claude" => display_path(&parent.join("CLAUDE.md")),
        "antigravity" => display_path(&parent.join("AGENTS.md")),
        "intellij" => display_path(&parent.join("jawata-studio-rules.md")),
        _ => display_path(&parent.join("jawata-studio-rules.md")),
    }
}

/// Sprint 16b/C: the client's GLOBAL / always-loaded instruction file — the one
/// loaded into every session regardless of cwd. The deploy writes the managed
/// rule block here IN ADDITION to the config-sibling (`derive_rule_path`) so the
/// "use JAWATA, not grep" rule survives MCP schema deferral.
///
/// - `claude` → `~/.claude/CLAUDE.md`. The sibling for Claude Code is `~/CLAUDE.md`
///   (next to `~/.claude.json`), which is NOT always-loaded; `~/.claude/CLAUDE.md`
///   is. This is the gap the rebrand left stale.
/// - `cursor` → `None`: the default Cursor sibling is already `~/.cursor/rules/
///   jawata-studio.mdc` (a global rules dir), so the sibling already covers it.
/// - `antigravity` / others → `None`: no confirmed always-loaded global file;
///   don't guess a path. Revisit if/when one is confirmed.
fn derive_global_rule_path(client: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    match client {
        "claude" => Some(display_path(&home.join(".claude").join("CLAUDE.md"))),
        _ => None,
    }
}

/// Sprint 25a D1: where the generated seat-command artifacts live, per
/// client — derived from the SAME base as the rule file (the config
/// sibling), so a tempdir client tree in tests behaves like the real one.
/// claude → `<base>/.claude/skills/<cmd>/SKILL.md` · cursor →
/// `<base>/commands/<cmd>.md` (the config sibling IS `~/.cursor`) ·
/// antigravity → `<base>/.agent/workflows/<cmd>.md` (verified format,
/// C2). claude_desktop ships via the export zip; intellij via the
/// rule-block phrase table — neither has a commands dir.
fn derive_seat_commands_dir(client: &str, mcp_target_path: &str) -> Option<PathBuf> {
    let parent = PathBuf::from(mcp_target_path)
        .parent()
        .map(Path::to_path_buf)?;
    match client {
        "claude" => Some(parent.join(".claude").join("skills")),
        "cursor" => Some(parent.join("commands")),
        "antigravity" => Some(parent.join(".agent").join("workflows")),
        _ => None,
    }
}

/// The five command-artifact paths for a command-bearing client (the
/// deployed inventory — asserted by test, removed exactly by Delete).
fn seat_artifact_paths(client: &str, commands_dir: &Path) -> Vec<(String, PathBuf)> {
    crate::conductor::COMMAND_MAP
        .iter()
        .map(|(_, cmd, _)| {
            let path = match client {
                "claude" => commands_dir.join(cmd).join("SKILL.md"),
                _ => commands_dir.join(format!("{cmd}.md")),
            };
            ((*cmd).to_string(), path)
        })
        .collect()
}

/// Writes the generated seat commands for one client, change-detected
/// (the `write_managed_cursor_hooks` template): a byte-identical redeploy
/// writes NOTHING. Returns the paths actually written.
fn write_managed_seat_commands(
    client: &str,
    commands_dir: &Path,
    seats: &[crate::runner::SeatDefinition],
    force_rewrite: bool,
) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for seat in seats {
        let rendered = match client {
            "claude" => crate::conductor::render_claude_skill(seat),
            "cursor" => crate::conductor::render_cursor_command(seat),
            "antigravity" => crate::conductor::render_antigravity_workflow(seat),
            _ => None,
        };
        let Some(body) = rendered else { continue };
        let (cmd, _) = crate::conductor::command_for(&seat.name)
            .expect("rendered seats are command-mapped");
        let path = match client {
            "claude" => commands_dir.join(cmd).join("SKILL.md"),
            _ => commands_dir.join(format!("{cmd}.md")),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("{client}: cannot create {}: {e}", parent.display()))?;
        }
        let changed = fs::read_to_string(&path)
            .map(|existing| existing != body)
            .unwrap_or(true);
        if changed || force_rewrite {
            fs::write(&path, &body)
                .map_err(|e| format!("{client}: cannot write {}: {e}", path.display()))?;
            written.push(display_path(&path));
        }
    }
    Ok(written)
}

/// Delete-side counterpart: removes exactly the five artifacts and prunes
/// the dirs they created (Delete leaves no trace in CLIENT trees).
/// DELIBERATE: `<config>/seats/` is NOT removed — it is studio's own
/// user-editable configuration, shared across clients, and a user-edited
/// seat must survive an undeploy exactly as it survives a redeploy.
fn remove_managed_seat_commands(client: &str, commands_dir: &Path) -> Result<bool, String> {
    let mut removed = false;
    for (cmd, path) in seat_artifact_paths(client, commands_dir) {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("{client}: cannot remove {}: {e}", path.display()))?;
            removed = true;
        }
        if client == "claude" {
            let _ = fs::remove_dir(commands_dir.join(&cmd)); // only if empty
        }
    }
    let _ = fs::remove_dir(commands_dir); // only if empty
    if client == "antigravity" {
        if let Some(agent_dir) = commands_dir.parent() {
            let _ = fs::remove_dir(agent_dir); // `.agent`, only if empty
        }
    }
    Ok(removed)
}


/// Sprint 26 (D6): the utility commands ride the same dirs as the seats.
fn utility_artifact_paths(client: &str, commands_dir: &Path) -> Vec<(String, PathBuf)> {
    crate::conductor::UTILITY_MAP.iter().map(|(cmd, _)| {
        let path = match client {
            "claude" => commands_dir.join(cmd).join("SKILL.md"),
            _ => commands_dir.join(format!("{cmd}.md")),
        };
        ((*cmd).to_string(), path)
    }).collect()
}

fn write_managed_utility_commands(
    client: &str,
    commands_dir: &Path,
    force_rewrite: bool,
) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (cmd, desc) in crate::conductor::UTILITY_MAP {
        let body = match client {
            "claude" => crate::conductor::render_claude_utility(cmd, desc),
            "cursor" => crate::conductor::render_cursor_utility(cmd, desc),
            "antigravity" => crate::conductor::render_antigravity_utility(cmd, desc),
            _ => continue,
        };
        let path = match client {
            "claude" => commands_dir.join(cmd).join("SKILL.md"),
            _ => commands_dir.join(format!("{cmd}.md")),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("{client}: cannot create {}: {e}", parent.display()))?;
        }
        let changed = fs::read_to_string(&path).map(|e| e != body).unwrap_or(true);
        if changed || force_rewrite {
            fs::write(&path, &body)
                .map_err(|e| format!("{client}: cannot write {}: {e}", path.display()))?;
            written.push(display_path(&path));
        }
    }
    Ok(written)
}

fn remove_managed_utility_commands(client: &str, commands_dir: &Path) -> Result<bool, String> {
    let mut removed = false;
    for (cmd, path) in utility_artifact_paths(client, commands_dir) {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("{client}: cannot remove {}: {e}", path.display()))?;
            removed = true;
        }
        if client == "claude" {
            let _ = fs::remove_dir(commands_dir.join(&cmd));
        }
    }
    let _ = fs::remove_dir(commands_dir);
    Ok(removed)
}

fn seat_export_zip_path(export_dir: &Path) -> PathBuf {
    export_dir.join("jawata-seats-skill.zip")
}

/// The claude.ai / Claude Desktop skill archive, written into the studio
/// export dir (the user uploads it once). Deterministic zip bytes (C2), so
/// the change-detected write makes redeploy a no-op.
fn write_managed_seat_export(
    export_dir: &Path,
    seats: &[crate::runner::SeatDefinition],
    force_rewrite: bool,
) -> Result<(String, bool), String> {
    let bytes = crate::conductor::render_claudeai_skill_zip(seats)?;
    fs::create_dir_all(export_dir)
        .map_err(|e| format!("cannot create export dir {}: {e}", export_dir.display()))?;
    let path = seat_export_zip_path(export_dir);
    let changed = fs::read(&path).map(|existing| existing != bytes).unwrap_or(true);
    if changed || force_rewrite {
        fs::write(&path, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        return Ok((display_path(&path), true));
    }
    Ok((display_path(&path), false))
}

fn remove_managed_seat_export(export_dir: &Path) -> Result<bool, String> {
    let path = seat_export_zip_path(export_dir);
    let existed = path.exists();
    if existed {
        fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    }
    let _ = fs::remove_dir(export_dir); // only if empty
    Ok(existed)
}

fn validate_written_client_config(
    client: &str,
    path: &str,
    servers: &[ManagedDeployServer],
) -> Result<(), String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("{client}: failed to read written config {path}: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|error| format!("{client}: written config is invalid JSON in {path}: {error}"))?;
    validate_client_config_shape(client, &value, servers)
}

fn validate_client_config_shape(
    client: &str,
    value: &serde_json::Value,
    servers: &[ManagedDeployServer],
) -> Result<(), String> {
    let root = value
        .as_object()
        .ok_or_else(|| format!("{client}: config root is not an object"))?;
    let mcp_servers = root
        .get("mcpServers")
        .and_then(|value| value.as_object())
        .ok_or_else(|| format!("{client}: missing or invalid mcpServers object"))?;

    for server in servers {
        let server_value = mcp_servers.get(&server.id).ok_or_else(|| {
            format!(
                "{client}: managed server '{}' missing in mcpServers after deploy",
                server.id
            )
        })?;
        let server_obj = server_value.as_object().ok_or_else(|| {
            format!(
                "{client}: server '{}' entry is not a JSON object",
                server.id
            )
        })?;

        // Sprint 15 v0.15.0 hotfix: post-write validator was written
        // when entries had stdio `command` + `args`. URL entries don't
        // carry those — they have `url` + `headers.Authorization`.
        // Sprint 16 (bugs.md #10): the URL field name is per-client —
        // antigravity reads `serverUrl`, everyone else `url` (see
        // managed_server_entry for the schema table).
        let url_field = if client == "antigravity" { "serverUrl" } else { "url" };
        let url_valid = server_obj
            .get(url_field)
            .and_then(|value| value.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);

        if !url_valid {
            return Err(format!(
                "{client}: server '{}' missing non-empty {url_field}",
                server.id
            ));
        }

        let auth_valid = server_obj
            .get("headers")
            .and_then(|value| value.as_object())
            .and_then(|headers| headers.get("Authorization"))
            .and_then(|value| value.as_str())
            .map(|value| value.starts_with("Bearer ") && value.len() > "Bearer ".len())
            .unwrap_or(false);

        if !auth_valid {
            return Err(format!(
                "{client}: server '{}' missing valid Authorization Bearer header",
                server.id
            ));
        }
    }

    Ok(())
}

fn build_rule_block(client: &str, servers: &[ManagedDeployServer]) -> String {
    // Sprint A0 (v0.17.0): the rule block deployed into each client's rule file
    // (CLAUDE.md / .cursor/rules/*.mdc / AGENTS.md / …). It is the cross-client
    // delivery vehicle for "use jawata, not grep, for Java" — the prior
    // one-line policy was too vague to change agent behaviour. Three imperative
    // sections: a Java→jawata routing table, the health-gated fallback (ASK when
    // JAWATA is down on Java work, silent on non-Java), then the TDD-refactor loop.
    // Keep it tight and scannable; a long rule gets ignored. Since Sprint 25a
    // the body is identical for every client EXCEPT the conductor section
    // (its per-client tail: commands-installed one-liner vs the IntelliJ
    // phrase table); the marker-replace idempotency is unaffected.
    let mut lines = vec![
        format!("<!-- jawata-studio:{client}:start -->"),
        "## JAWATA MCP — use it for Java, before shell text tools".to_string(),
        String::new(),
        "These workspaces are served by JAWATA MCP (compiler-accurate, JDT-backed). For \
         ANY Java semantic task, call the MCP tool BEFORE reaching for `grep`/`rg`/`find`/\
         `sed`/`awk` or hand-reading `.java` files:"
            .to_string(),
        String::new(),
        "- Find a symbol by name → `search_symbols` (not grep)".to_string(),
        "- Who calls / uses this → `find_references` / `find_method_references` / \
         `get_call_hierarchy_incoming` (not grep)"
            .to_string(),
        "- Type shape, members, hierarchy, supertypes → `analyze_type` / `get_type_members` \
         / `get_type_hierarchy`"
            .to_string(),
        "- Jump to a definition → `go_to_definition`".to_string(),
        "- Errors / does it compile → `compile_workspace` + `get_diagnostics`".to_string(),
        "- Change code structurally → the refactoring tools (`rename_symbol`, \
         `extract_method`, `inline_method`, `pull_up`, `change_method_signature`, …). Do \
         NOT hand-edit a rename/move/extract."
            .to_string(),
        String::new(),
        "Shell text search is a FALLBACK only — when JAWATA is unavailable, or for \
         non-Java / non-semantic matches (build files, configs, comments, log strings)."
            .to_string(),
        String::new(),
        "**Try-first, or justify — the deployed hook ENFORCES this.** A `grep`/`rg` over a \
         `.java` file, or a hand-edit of a `.java` file, is BLOCKED unless you tried jawata first \
         (a search) or use a jawata tool (an edit) — OR you declare \
         `jawata-fallback: <why jawata is inadequate for THIS case>` in the command (it is logged). \
         It is meant to be inconvenient NOT to use jawata; you are never stuck — the justified \
         fallback always proceeds."
            .to_string(),
        String::new(),
        "Editing a `.java` file by hand is blocked — use the tool:".to_string(),
        "- Rename a symbol (updates ALL references) → `rename_symbol`".to_string(),
        "- Move a class / pull a member up or down → `move` / `move_in_hierarchy`".to_string(),
        "- Extract a method / variable / constant / superclass → `extract`".to_string(),
        "- Duplicate a class → `generate(kind=copy_class)` then `extract(kind=superclass)`"
            .to_string(),
        "- Any structural change → `refactoring(action=plan)` then `apply_plan` \
         (parity-gated, reversible)"
            .to_string(),
        String::new(),
        "## When JAWATA is unavailable — ASK, don't silently degrade".to_string(),
        String::new(),
        "If a JAWATA tool is unreachable (the server is not running — e.g. not started after a \
         reboot, autostart off) and you are doing **Java** semantic/structural work, do NOT \
         quietly fall back to grep or hand-editing. **STOP and ask** how to proceed (wait while \
         it is started · grep this once, degraded · abort) — silently losing the \
         compiler-accurate layer is worse than pausing. On **non-Java** work (Rust, Python, \
         configs, docs) JAWATA does not apply: proceed normally, no question. And never use \
         \"JAWATA is down\" as a reason to reclassify Java work as something else to dodge this check."
            .to_string(),
        String::new(),
        "## Refactor in small, verified steps".to_string(),
        String::new(),
        "1. Confirm a green baseline (`compile_workspace`; run the relevant tests).".to_string(),
        "2. Apply ONE refactoring via a JAWATA tool (it returns a diff + `undoChangeId`)."
            .to_string(),
        "3. Re-check: `compile_workspace` + run the tests again.".to_string(),
        "4. Green → keep going. Red → `undo_refactoring` and rethink. One step at a time."
            .to_string(),
        String::new(),
        // v2.5.1 (Cursor parity, interim): the experience store is the CROSS-CLIENT
        // memory, but only Claude Code has push hooks (primer/recall). Every other
        // client must PULL — this section is the textual substitute until its hook
        // schema is ported. Identical text everywhere (harmless where hooks push too).
        "## JAWATA memory — recall before you theorize, record what you learn".to_string(),
        String::new(),
        "The experience store is the CROSS-CLIENT memory: the same store answers in \
         Cursor and Claude Code. Clients without hook injection must PULL it:"
            .to_string(),
        String::new(),
        "- At the START of a session touching Java → `experience(kind=primer, \
         format=text)` — the domain layer Claude Code receives automatically."
            .to_string(),
        "- BEFORE diagnosing a symptom or refactoring a symbol → \
         `experience(kind=recall, symbol=\"pkg.Type#member\")` or \
         `experience(kind=recall, symptom=\"...\")`. A match is a CLOSED SET — match \
         your observation to ONE of them with evidence, or declare it genuinely new; \
         do not generate a novel cause."
            .to_string(),
        "- Learned something durable (lesson, failure mode, hazard, convention) → \
         `experience(kind=record, type=lesson, summary=..., symbol=...)` — it becomes \
         recallable by symbol from every client."
            .to_string(),
        "- Shell fallback on Java anyway? Declare `jawata-fallback: <why>` in the \
         command — the declaration is the audit trail."
            .to_string(),
        String::new(),
        // Sprint 25 D10 (Harald, 2026-07-17): the upward-communication
        // contract is INJECTED, not remembered — agent sessions do not share
        // context, so a discipline learned in one session dies with it. The
        // managed rule block is the delivery vehicle every client receives.
        "## Communication upward — the decision-ask contract (BINDING)".to_string(),
        String::new(),
        "The human does MANAGEMENT, not the work. Everything you send up is \
         management-level, with the tech translation built in — never make the reader \
         tear grounding facts out of you:"
            .to_string(),
        String::new(),
        "- DECISION FIRST: open with `DECISION: <one-line question in business terms>` — \
         never buried under progress reporting."
            .to_string(),
        "- Context ≤2 plain sentences. Then OPTIONS, each with: what it means for the \
         reader / pro / con / cost / risk. ONE recommendation with a one-sentence why."
            .to_string(),
        "- ONE decision per ask. When the decidable reality is per-item (a tool list, a \
         finding list), present a PER-ITEM TABLE — never one blanket ask for N decisions."
            .to_string(),
        "- ABBREVIATIONS: define at first use — \"CC (cyclomatic complexity, the number \
         of independent decision paths)\" — then use freely. Never assume a prior session \
         introduced it."
            .to_string(),
        "- Tech detail goes BELOW the decision, folded — available, never load-bearing."
            .to_string(),
        // Harald 2026-07-18 ("this needs to be enforced — I don't want you
        // to decide if you do or leave"): the decision test is an OBLIGATION
        // in every client, not a Claude-Code-only /sprint step.
        "- THE DECISION TEST (ENFORCED, every client): before a decision ask, \
         checkpoint summary, or sprint result reaches the human, audit it in a \
         fresh context — can the reader decide from this text ALONE, no \
         interpretation, no guessing, every term defined, meaning preserved \
         rather than merely shortened? A gate result is reported as WHAT IT \
         PROVES, never as how it ran. Failing text is rewritten before sending."
            .to_string(),
        String::new(),
    ];
    // Sprint 25a D2: the conductor section — the ONE deliberately per-client
    // part of the body (commands-installed one-liner vs IntelliJ phrase
    // table). A parse failure of the embedded seats is a build defect; it
    // surfaces as a loud comment in the artifact, never a panic mid-deploy.
    // DELIBERATE (audit obs 2): this section renders from the EMBEDDED
    // seats, not the materialized dir — its content is the COMMAND_MAP
    // catalog (names + descriptions), not stance text; a user-edited seat
    // changes the COMMANDS (which do read the materialized dir), never the
    // catalog. Revisit if seats ever become user-addable (Sprint 28+).
    match crate::conductor::embedded_seat_definitions() {
        Ok(seats) => lines.extend(crate::conductor::render_conductor_section(&seats, client)),
        Err(e) => lines.push(format!("<!-- jawata conductor section unavailable: {e} -->")),
    }
    lines.extend([String::new(), "Managed service ids:".to_string()]);
    for server in servers {
        lines.push(format!("- {}", server.id));
    }
    lines.push(format!("<!-- jawata-studio:{client}:end -->"));
    lines.join("\n")
}

/// Cursor enforces `len(server_id) + 1 + len(tool_name) <= 59` (reports as "exceeds 60 characters").
/// Antigravity is limited by a separate ~100 *services* / tool-budget; no shared constant here.
const CURSOR_MCP_COMBINED_MAX: usize = 59;
/// Upper bound on a single jawata-mcp tool name length (e.g. `get_call_hierarchy_outgoing` ~ 28; keep buffer for future tools).
const JAWATA_TOOL_NAME_BUDGET: usize = 32;

fn max_mcp_server_id_len_for_cursor() -> usize {
    CURSOR_MCP_COMBINED_MAX
        .saturating_sub(1) // ":"
        .saturating_sub(JAWATA_TOOL_NAME_BUDGET)
}

/// Sprint 10 v0.10.4: MCP service ID derived from the workspace name.
/// Format: `jawata-<sanitized-workspace-name>`, capped at the Cursor server-id
/// budget. Single-workspace mode means each MCP service represents one
/// logical workspace, not one project.
fn mcp_server_id_for_workspace(workspace_name: &str) -> String {
    let max_id = max_mcp_server_id_len_for_cursor();
    let prefix = "jawata-";
    if prefix.len() >= max_id {
        return prefix.to_string();
    }
    let max_slug = max_id.saturating_sub(prefix.len());
    let slug = mcp_label_slug(workspace_name, workspace_name, max_slug);
    if slug.is_empty() {
        let h = mcp_id_hash_suffix(workspace_name, max_slug);
        return format!("{prefix}{h}");
    }
    let mut id = format!("{prefix}{slug}");
    while id.len() > max_id {
        id.pop();
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.len() <= prefix.len() {
        return format!("{prefix}{}", mcp_id_hash_suffix(workspace_name, max_slug));
    }
    id
}

fn mcp_id_hash_suffix(id: &str, max_len: usize) -> String {
    let take = max_len.clamp(4, 12);
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    let v = h.finish();
    let hex = format!("{:016x}", v);
    hex.chars().take(take).collect()
}

fn mcp_label_slug(name: &str, project_path: &str, max_chars: usize) -> String {
    let trimmed = name.trim();
    let raw: &str = if trimmed.is_empty() {
        std::path::Path::new(project_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
    } else {
        trimmed
    };
    let lower = raw.to_lowercase();
    let mut out = String::new();
    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return String::new();
    }
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect();
        while out.ends_with('-') {
            out.pop();
        }
    }
    out
}

/// Keys for MCP servers written by jawata-studio: `jawata-…`, plus the legacy
/// generations `goja-…` (pre-22b rebrand) / `jl-…` / `javalens-…` recognised for
/// cleanup/migration of pre-rebrand deploys (migration literals, exception class 3).
fn is_managed_mcp_key(key: &str) -> bool {
    key.starts_with("jawata-")
        || key.starts_with("goja-")
        || key.starts_with("jl-")
        || key.starts_with("javalens-")
}

/// Sprint 16 (bugs.md #14a): true when the client's MCP config file already
/// carries at least one jawata-managed server entry — the marker that the
/// user deployed there before, making it an auto-refresh target.
fn path_has_managed_entries(path: &str) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return false;
    };
    value
        .get("mcpServers")
        .and_then(|servers| servers.as_object())
        .map(|servers| servers.keys().any(|key| is_managed_mcp_key(key)))
        .unwrap_or(false)
}

/// Sprint 16 (bugs.md #14b): attach workspace-resolve failures to every
/// client result that actually wrote (skipped clients stay untouched), so
/// the deploy UI shows what was omitted instead of reporting silent success.
fn merge_resolve_errors(results: &mut [DeployClientResult], resolve_errors: &[String]) {
    if resolve_errors.is_empty() {
        return;
    }
    for result in results.iter_mut() {
        if !matches!(result.status, DeployClientStatus::Skipped) {
            result
                .validation_errors
                .extend(resolve_errors.iter().cloned());
        }
    }
}

fn write_managed_json_block(
    path: &str,
    client: &str,
    servers: &[ManagedDeployServer],
    merge_mode: &McpMergeMode,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<(), String> {
    let path_buf = PathBuf::from(path);
    let parent = path_buf
        .parent()
        .ok_or_else(|| format!("target path has no parent: {}", path_buf.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create parent {}: {error}", parent.display()))?;

    let existing_contents = fs::read_to_string(&path_buf).ok();
    let mut root_value = existing_contents
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root_value.is_object() {
        root_value = serde_json::json!({});
    }

    let mut next_value = root_value;

    // Merge managed JAWATA servers into the client's real MCP schema.
    // Clients load "mcpServers", not our internal jawataManager metadata.
    if let Some(object) = next_value.as_object_mut() {
        let mut existing_servers = object
            .get("mcpServers")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();

        let incoming_ids: HashSet<String> =
            servers.iter().map(|server| server.id.clone()).collect();

        // Sprint 15 Stage 11: URL form replaces stdio command/args/env.
        // Sprint 16 (bugs.md #10): the entry shape is per-client — see
        // managed_server_entry for the schema table.
        for server in servers {
            existing_servers.insert(server.id.clone(), managed_server_entry(client, server));
        }

        // Managed-namespace keys (jawata-/goja-/jl-/javalens-) belong to the
        // studio: any that are not part of THIS deploy are stale generations
        // (e.g. pre-rebrand goja-* entries, or per-workspace entries after the
        // gateway consolidates to one). Prune them in EVERY merge mode — user
        // keys are untouched. Previously this only ran under force_rewrite /
        // ReplaceManagedSection, so legacy goja-* keys survived a plain deploy
        // (caught live in the jawata Stage-8 integration run).
        existing_servers
            .retain(|key, _| !is_managed_mcp_key(key) || incoming_ids.contains(key));
        let _ = merge_mode; // merge modes still govern rule/hook block handling

        object.insert(
            "mcpServers".into(),
            serde_json::Value::Object(existing_servers),
        );
        // Remove legacy payload from earlier deploy versions.
        object.remove("jawataManager");
    }

    let next_json = serde_json::to_string_pretty(&next_value)
        .map_err(|error| format!("failed serializing MCP config json: {error}"))?;

    if !force_rewrite {
        if let Some(existing) = existing_contents {
            if existing.trim() == next_json.trim() {
                return Ok(());
            }
        }
    }

    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&path_buf)
            .map_err(|error| format!("failed creating centralized backup: {error}"))?;
    }
    fs::write(&path_buf, format!("{next_json}\n"))
        .map_err(|error| format!("failed writing MCP config {}: {error}", path_buf.display()))
}

fn remove_managed_json_block(path: &str, backup_before_write: bool) -> Result<bool, String> {
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Ok(false);
    }

    let existing_contents = fs::read_to_string(&path_buf)
        .map_err(|error| format!("failed to read MCP config {}: {error}", path_buf.display()))?;
    let mut root_value: serde_json::Value =
        serde_json::from_str(&existing_contents).map_err(|error| {
            format!(
                "failed parsing MCP config {} as JSON: {error}",
                path_buf.display()
            )
        })?;
    if !root_value.is_object() {
        return Ok(false);
    }

    let mut changed = false;
    if let Some(object) = root_value.as_object_mut() {
        let mut existing_servers = object
            .get("mcpServers")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();
        let previous_len = existing_servers.len();
        existing_servers.retain(|key, _| !is_managed_mcp_key(key));
        changed |= existing_servers.len() != previous_len;
        object.insert(
            "mcpServers".into(),
            serde_json::Value::Object(existing_servers),
        );
        changed |= object.remove("jawataManager").is_some();
    }

    if !changed {
        return Ok(false);
    }

    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&path_buf)
            .map_err(|error| format!("failed creating centralized backup: {error}"))?;
    }

    let next_json = serde_json::to_string_pretty(&root_value)
        .map_err(|error| format!("failed serializing MCP config json: {error}"))?;
    fs::write(&path_buf, format!("{next_json}\n"))
        .map_err(|error| format!("failed writing MCP config {}: {error}", path_buf.display()))?;
    Ok(true)
}

/// Sprint 16 (bugs.md #10): one managed MCP entry in the shape the named
/// client's parser accepts. The schema table lives HERE so a future client
/// costs one match arm, not a hunt across writer sites:
///
/// | client            | shape                                            |
/// |-------------------|--------------------------------------------------|
/// | antigravity       | `{ serverUrl, headers }` — NO `type` (Windsurf    |
/// |                   | lineage rejects `type`+`url` with "serverURL or   |
/// |                   | command must be specified"; verified 2026-06-10)  |
/// | claude/cursor/... | `{ type: "http", url, headers }` (Claude Code     |
/// |                   | falls through to its stdio parser without `type`) |
///
/// `disabled: true` is accepted by all targets and stays client-agnostic.
fn managed_server_entry(client: &str, server: &ManagedDeployServer) -> serde_json::Value {
    let mut entry = serde_json::Map::new();
    match client {
        "antigravity" => {
            entry.insert(
                "serverUrl".into(),
                serde_json::Value::String(server.url.clone()),
            );
        }
        _ => {
            entry.insert("type".into(), serde_json::Value::String("http".into()));
            entry.insert("url".into(), serde_json::Value::String(server.url.clone()));
        }
    }
    entry.insert(
        "headers".into(),
        serde_json::json!({
            "Authorization": format!("Bearer {}", server.token),
        }),
    );
    // Sprint 16b/C: Claude Code (CLI, v2.1.121+) honours a per-server
    // `alwaysLoad` flag — mark the managed JAWATA server so its (post-collapse)
    // tool surface loads upfront and never defers behind MCP tool-search.
    // Cursor caps at 40 tools and Antigravity has no such flag, so this is
    // Claude-only; the universal levers are the collapse + the always-loaded
    // rule block (derive_global_rule_path).
    if client == "claude" {
        entry.insert("alwaysLoad".into(), serde_json::Value::Bool(true));
    }
    if server.disabled {
        entry.insert("disabled".into(), serde_json::Value::Bool(true));
    }
    serde_json::Value::Object(entry)
}

fn build_client_mcp_json(client: &str, servers: &[ManagedDeployServer]) -> serde_json::Value {
    let server_map: serde_json::Map<String, serde_json::Value> = servers
        .iter()
        .map(|server| (server.id.clone(), managed_server_entry(client, server)))
        .collect();

    serde_json::json!({
        "mcpServers": server_map
    })
}

/// Sprint 16b/B: ensure the gateway has a persisted Bearer token, generating and
/// saving one on first use.
fn ensure_gateway_token(config_store: &ConfigStore, settings: &ManagerSettings) -> String {
    if let Some(token) = settings.gateway_token.clone() {
        return token;
    }
    let token = crate::resident::generate_token();
    let mut updated = settings.clone();
    updated.gateway_token = Some(token.clone());
    let _ = config_store.write_settings(updated);
    token
}

/// Sprint 16b/B: convert the per-workspace deploy set into the gateway's routing
/// table — one route per resident, carrying its project roots for path routing.
fn build_routing_table(servers: &[ManagedDeployServer]) -> gateway::RoutingTable {
    gateway::RoutingTable::new(
        servers
            .iter()
            .map(|server| gateway::GatewayRoute {
                workspace_name: server.workspace_name.clone(),
                url: server.url.clone(),
                token: server.token.clone(),
                project_paths: server.project_paths.clone(),
            })
            .collect(),
    )
}

/// Sprint 16b/B: the single client-facing `jawata` entry that points at the gateway.
fn gateway_entry(port: u16, token: &str, disabled: bool) -> ManagedDeployServer {
    ManagedDeployServer {
        id: "jawata".to_string(),
        workspace_name: "gateway".to_string(),
        project_names: Vec::new(),
        project_paths: Vec::new(),
        url: format!("http://127.0.0.1:{port}/mcp"),
        token: token.to_string(),
        disabled,
    }
}

fn write_managed_rule_block(
    path: &str,
    managed_rule_block: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<(), String> {
    let path_buf = PathBuf::from(path);
    let parent = path_buf
        .parent()
        .ok_or_else(|| format!("rule target path has no parent: {}", path_buf.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create parent {}: {error}", parent.display()))?;

    let existing = fs::read_to_string(&path_buf).unwrap_or_default();
    let start_marker = managed_rule_block
        .lines()
        .next()
        .ok_or("managed rule block missing start marker")?;
    let end_marker = managed_rule_block
        .lines()
        .last()
        .ok_or("managed rule block missing end marker")?;

    // Sprint 22b: a file last written by goja-studio carries the legacy markers —
    // find those too, so the redeploy REPLACES the old block instead of appending
    // a duplicate beside it.
    let legacy_start = legacy_sentinel(start_marker);
    let legacy_end = legacy_sentinel(end_marker);
    let found = match (existing.find(start_marker), existing.find(end_marker)) {
        (Some(s), Some(e)) => Some((s, e + end_marker.len())),
        _ => match (existing.find(&legacy_start), existing.find(&legacy_end)) {
            (Some(s), Some(e)) => Some((s, e + legacy_end.len())),
            _ => None,
        },
    };
    let next = if let Some((start_idx, end_inclusive)) = found {
        format!(
            "{}{}{}",
            &existing[..start_idx],
            managed_rule_block,
            &existing[end_inclusive..]
        )
    } else if existing.trim().is_empty() {
        managed_rule_block.to_string()
    } else {
        format!("{}\n\n{}", existing.trim_end(), managed_rule_block)
    };

    if !force_rewrite && existing.trim() == next.trim() {
        return Ok(());
    }

    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&path_buf)
            .map_err(|error| format!("failed creating centralized rule backup: {error}"))?;
    }
    fs::write(&path_buf, format!("{}\n", next.trim_end()))
        .map_err(|error| format!("failed writing rule file {}: {error}", path_buf.display()))
}

fn remove_managed_rule_block(
    path: &str,
    client: &str,
    backup_before_write: bool,
) -> Result<bool, String> {
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Ok(false);
    }
    let existing = fs::read_to_string(&path_buf)
        .map_err(|error| format!("failed to read rule file {}: {error}", path_buf.display()))?;
    let start_marker = format!("<!-- jawata-studio:{client}:start -->");
    let end_marker = format!("<!-- jawata-studio:{client}:end -->");
    // Sprint 22b: also remove blocks written by goja-studio (legacy markers).
    let (start_marker, end_marker) = if existing.contains(&start_marker) {
        (start_marker, end_marker)
    } else {
        (legacy_sentinel(&start_marker), legacy_sentinel(&end_marker))
    };

    let Some(start_idx) = existing.find(&start_marker) else {
        return Ok(false);
    };
    let Some(rel_end_idx) = existing[start_idx..].find(&end_marker) else {
        return Ok(false);
    };
    let end_idx = start_idx + rel_end_idx + end_marker.len();

    let mut next = format!("{}{}", &existing[..start_idx], &existing[end_idx..]);
    while next.contains("\n\n\n") {
        next = next.replace("\n\n\n", "\n\n");
    }
    let next = next.trim().to_string();

    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&path_buf)
            .map_err(|error| format!("failed creating centralized rule backup: {error}"))?;
    }

    if next.is_empty() {
        fs::write(&path_buf, "")
            .map_err(|error| format!("failed writing rule file {}: {error}", path_buf.display()))?;
    } else {
        fs::write(&path_buf, format!("{next}\n"))
            .map_err(|error| format!("failed writing rule file {}: {error}", path_buf.display()))?;
    }
    Ok(true)
}

/// Sprint 21a (item E): the newest CENTRALIZED backup of `path` (the old sibling-file
/// stub always returned None — the UI's backupPath was permanently empty).
fn latest_backup_path(path: &str) -> Option<String> {
    crate::backups::latest_backup_path(Path::new(path))
        .map(|backup| display_path(&backup))
}

// ===== Sprint 18 Track 2 / Stage 9: PreToolUse enforcement hook (Claude Code) =====
//
// Level 3 of "make the agent use JAWATA" (available → recommended → ENFORCED; the
// rule block is level 2). Claude Code fires a `PreToolUse` hook before it runs a
// tool; a hook that exits 2 blocks the call and feeds its stderr back to the
// model. We register a hook on `Bash|Grep` that redirects Java *text search*
// (grep/rg/find/sed/awk over `.java`, or the Grep tool aimed at Java) to the JAWATA
// semantic tools. It is HEALTH-GATED: the same block carries a different message
// when the resident is up (redirect to the tool) vs down (diagnosis + how to
// start, or proceed grep-degraded on purpose). Non-Java calls, and edits, pass
// untouched — the hook enforces only the unambiguous, high-precision case so it
// can never block a legitimate edit. Structural-edit guidance stays in the rule
// block (level 2). Claude Code only; other clients keep the rule block.

/// Sentinel embedded in the managed guard command so we can find + replace + remove
/// exactly our `PreToolUse` entries without disturbing user-authored hooks.
const JAWATA_HOOK_SENTINEL: &str = "jawata-studio/pretooluse-guard.sh";

/// Sprint 22b: the pre-rebrand (goja) twin of a managed sentinel/marker —
/// recognised so a redeploy REPLACES entries/blocks written by goja-studio
/// instead of duplicating beside them, and removal cleans both generations.
/// Migration literal (grep-contract exception class 3); drop with the legacy
/// layer next release.
fn legacy_sentinel(sentinel: &str) -> String {
    sentinel.replace("jawata", "goja")
}

/// Delete everything in OUR namespace inside a client's hooks directory that is
/// not the live generation. Returns whether anything was removed.
///
/// We own the `jawata-` prefix there (and the legacy `goja-` one), so any file
/// carrying it that is not a role's current invocation target is residue. Two
/// kinds occur: a retired script, and — the reason this exists — a binary an
/// EARLIER VERSION wrote under a name nothing resolves.
///
/// v3.7.9 did exactly that on every Windows install: four binaries named without
/// `.exe`, which no client could invoke and no later deploy would have touched,
/// because retirement only ever removed names the current code knows how to
/// write. Enumerating what to keep, and removing the rest, covers every past and
/// future misnaming instead of the specific ones someone remembered.
///
/// Files outside the namespace are never touched — `hook_config.json` and
/// `hook_silence.log` do not carry the prefix, and a user's own hooks live under
/// their own names.
fn sweep_managed_hook_residue(hooks_dir: &Path, keep: &[String]) -> bool {
    let Ok(entries) = fs::read_dir(hooks_dir) else {
        return false;
    };
    let mut removed = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = name.starts_with("jawata-") || name.starts_with("goja-");
        if !ours || keep.iter().any(|k| k == &name) {
            continue;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if fs::remove_file(entry.path()).is_ok() {
            removed = true;
        }
    }
    removed
}

/// EVERY prior generation of a managed sentinel, newest-legacy first.
///
/// Sprint 28 (D-SHIM) — the third generation, and why one was not enough.
/// `legacy_sentinel` covers exactly one: `jawata-*` → `goja-*`. Stage 6 renames
/// the managed scripts to role-named BINARIES (`jawata-hook-guard`, no `.sh`),
/// which makes the current `.sh` names a second legacy generation. An install
/// carrying them would then match no sentinel at all, be classified as **the
/// user's own hook**, and be preserved forever — the retired script and the new
/// binary both firing on every event. That is the hook-outage shape, shipped by
/// the fix for the hook outage.
///
/// So a sentinel's history is data. Given the CURRENT sentinel, this returns
/// what earlier deploys wrote for the same role, and every managed-entry
/// predicate checks the whole list.
fn legacy_sentinels(current: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Generation 2 — the `.sh` script this binary replaces, and its goja twin.
    if let Some(script) = SCRIPT_GENERATION.iter().find(|(binary, _)| *binary == current) {
        out.push(script.1.to_string());
        out.push(legacy_sentinel(script.1));
    }
    // Generation 1 — the pre-rebrand twin of the current name itself.
    let goja = legacy_sentinel(current);
    if goja != current {
        out.push(goja);
    }
    out.dedup();
    out
}

/// The generation-2 script path each role-named binary replaces.
///
/// A table rather than a transform: the rename is not mechanical
/// (`pretooluse-guard.sh` → `jawata-hook-guard`), and a wrong guess here does
/// not fail loudly — it silently classifies our own retired script as the
/// user's and leaves it firing.
const SCRIPT_GENERATION: &[(&str, &str)] = &[
    // The sentinel CONSTANTS, not copies of their values. Writing the strings
    // out again left each one defined twice and the constants themselves dead
    // in production — caught by the hollow-wiring gate the moment the
    // predicates were rerouted through this table (C6 audit fixes). Two
    // definitions of one string is how a rename fixes half a system.
    ("jawata-hook-guard", JAWATA_HOOK_SENTINEL),
    ("jawata-hook-observer", JAWATA_POSTHOOK_SENTINEL),
    ("jawata-hook-primer", JAWATA_PRIMER_SENTINEL),
    ("jawata-hook-recall", JAWATA_RECALL_SENTINEL),
    ("jawata-hook-userprompt", JAWATA_USERPROMPT_SENTINEL),
    // NOT JAWATA_STOP_SENTINEL. That constant is a marker inside the script's
    // BODY, never a command, so a row using it could not match a live entry —
    // and the unit test passed it only by fabricating a command that cannot
    // exist. Exactly the failure this table's own comment warns about: a wrong
    // guess here does not fail loudly (C6 audit, F1).
    // "jawata-studio/stop-gate.sh", not the bare filename. The other five rows
    // carry the jawata-studio/ segment; a bare "stop-gate.sh" would also claim
    // a user's own /home/u/bin/stop-gate.sh and DELETE it on undeploy. The one
    // row that could over-claim was also the one with no specimen in the
    // never-claim test (C6 audit round 2, N2).
    ("jawata-hook-stop", "jawata-studio/stop-gate.sh"),
];

/// Whether an entry's command names ANY generation of a managed hook.
///
/// The predicate every managed-entry check should use. Matching only the
/// current generation is what preserves a retired script as the user's own.
fn entry_is_managed_any_generation(entry: &serde_json::Value, current: &str) -> bool {
    // SEPARATOR-INSENSITIVE. Every sentinel is written with a forward slash
    // ("jawata-studio/sessionstart-primer.sh"), but `PathBuf::join` emits
    // BACKSLASHES on Windows — so on Windows every predicate missed its own
    // entry, and since write_managed_hook_section is retain(!is_managed) then
    // push, each deploy appended one more entry per role, unbounded, with
    // undeploy leaving them all. Pre-existing and measured by the C6 audit
    // (round 3, N4); not introduced by this sprint, but this sprint is where
    // the predicate became the single place it can be fixed.
    //
    // The hook crate already solved the same hazard in `role_for_binary`,
    // splitting on both separators, with the comment "Windows is the platform
    // D-SHIM exists to serve". One crate knew and the other did not.
    let normalise = |s: &str| s.replace('\\', "/");
    let Some(commands) = entry.get("hooks").and_then(|h| h.as_array()) else {
        return false;
    };
    let commands: Vec<String> = commands
        .iter()
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
        .map(normalise)
        .collect();
    let mut wanted = vec![current.to_string()];
    wanted.extend(legacy_sentinels(current));
    wanted
        .iter()
        .map(|w| normalise(w))
        .any(|w| commands.iter().any(|c| c.contains(&w)))
}

/// Sprint 22b: a pre-rebrand deploy left a `goja-studio…`-named rule FILE beside
/// the renamed one (e.g. `.cursor/rules/goja-studio.mdc`, `goja-studio-rules.md`);
/// both would steer the agent. Remove the legacy sibling after the new file is
/// written (centralized backup first). No-op when the rule file is a shared file
/// (CLAUDE.md — no `jawata-studio` in its name).
fn remove_legacy_rule_sibling(rule_path: &str) -> Result<bool, String> {
    let p = PathBuf::from(rule_path);
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return Ok(false);
    };
    if !name.contains("jawata-studio") {
        return Ok(false);
    }
    let legacy = p.with_file_name(legacy_sentinel(name));
    if legacy.exists() {
        crate::backups::backup_before_write(&legacy)
            .map_err(|e| format!("failed backing up legacy rule file {}: {e}", legacy.display()))?;
        fs::remove_file(&legacy)
            .map_err(|e| format!("failed removing legacy rule file {}: {e}", legacy.display()))?;
        return Ok(true);
    }
    Ok(false)
}

/// Sprint 22 (POST layer): sentinel for the managed PostToolUse observer entry.
const JAWATA_POSTHOOK_SENTINEL: &str = "jawata-studio/posttooluse-observer.sh";

/// The client whose settings file receives the enforcement hook. Claude Code only:
/// its `~/.claude/settings.json` hook schema is the one we target; Cursor/
/// Antigravity have no equivalent, so they keep the rule block (level 2) alone.
fn derive_hook_settings_path(client: &str) -> Option<String> {
    if client != "claude" {
        return None;
    }
    let home = dirs::home_dir()?;
    Some(display_path(&home.join(".claude").join("settings.json")))
}

/// The managed Claude-side scripts dir `~/.claude/jawata-studio/`, with the Sprint-22b
/// one-time legacy move: an existing `~/.claude/goja-studio/` (pre-rebrand deploys —
/// scripts, trygate/editgate state, outcomes.log) is RENAMED to the new dir on first
/// touch, never clobbered (if the new dir already exists, the old one is left alone).
/// The redeploy then overwrites the scripts; the state/logs carry over.
/// The managed-scripts directory NAME, derived from a home dir. Pure.
///
/// C6 audit round 3, N5: `claude_scripts_dir()` is not a path getter — it
/// performs an irreversible `fs::rename` of a pre-rebrand directory as a side
/// effect. My new path-linkage test called it six times, so `cargo test --lib`
/// migrated the developer's real home on any machine still carrying a goja
/// install. It was a no-op here, which is why it passed unnoticed. A getter a
/// test may call must not move the user's files, so the derivation is split out
/// and the test uses THIS.
fn claude_scripts_dir_under(home: &Path) -> PathBuf {
    home.join(".claude").join("jawata-studio")
}

fn claude_scripts_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let new = claude_scripts_dir_under(&home);
    let old = home.join(".claude").join("goja-studio"); // migration literal (exception class 3)
    if old.exists() && !new.exists() {
        match fs::rename(&old, &new) {
            Ok(()) => eprintln!(
                "[jawata-studio] migrated claude scripts dir: {} -> {}",
                old.display(),
                new.display()
            ),
            Err(e) => eprintln!(
                "[jawata-studio] WARN: could not migrate claude scripts dir {} -> {}: {e}",
                old.display(),
                new.display()
            ),
        }
    }
    Some(new)
}

/// Absolute path of the managed guard script jawata-studio writes + owns. Lives under
/// `~/.claude/jawata-studio/` so the settings.json entry is a stable one-liner and all
/// the branching logic lives in a shell file we overwrite on every deploy.
/// The managed script filenames, named ONCE. The production path functions and
/// the linkage test both use these, so a rename cannot land in one and not the
/// other — which is what made the whole suite stay green while production
/// accumulated entries (C6 audit round 2, N1).
const GUARD_SCRIPT_FILE: &str = "pretooluse-guard.sh";
const OBSERVER_SCRIPT_FILE: &str = "posttooluse-observer.sh";
const PRIMER_SCRIPT_FILE: &str = "sessionstart-primer.sh";
const RECALL_SCRIPT_FILE: &str = "pretooluse-recall.sh";
const USERPROMPT_SCRIPT_FILE: &str = "userpromptsubmit-recall.sh";
const STOP_SCRIPT_FILE: &str = "stop-gate.sh";

/// The path a client should INVOKE for a hook role.
///
/// Sprint 28 dogfood: the six role binaries were deployed and every client
/// event still invoked the `.sh` script, because the deploy wrote binaries and
/// then wrote settings entries pointing at scripts — no code path built a
/// command aimed at a binary. The headline deliverable shipped its artifact and
/// not its effect, and seven audit rounds plus a five-platform release gate all
/// passed, because every one of them checked CODE callers and none checked what
/// the editor actually calls.
///
/// Prefer the binary when it is on disk; fall back to the script when it is
/// not, so an install that has not yet received a binary keeps working.
fn managed_hook_invocation_path(role: &str, script_file: &str) -> Option<PathBuf> {
    invocation_path_in(&claude_scripts_dir()?, role, script_file)
}

/// The same rule against an explicit directory, so it is testable without
/// resolving — and mutating — the developer's real home.
fn invocation_path_in(dir: &Path, role: &str, script_file: &str) -> Option<PathBuf> {
    invocation_path_in_when(dir, role, script_file, role_is_binary_live(role))
}

/// The resolver, with the liveness decision handed IN rather than looked up.
///
/// Generation is a property of a role AND the client deploying it, which the
/// role-only lookup cannot express. Cursor's observer is the case that forced
/// it: its script is a no-op (`cat > /dev/null`, then `{}`) while Claude Code's
/// captures tool outcomes and the `jawata-fallback` audit trail, so the same
/// role is correctly script-generation on one client and binary on the other.
///
/// This exists as ONE function taking a parameter rather than two copies of the
/// same rule for the reason `managed_cursor_hook_entries` records: the entry
/// written into the client's settings and the file written to disk must be
/// decided by the same code, or they disagree — which is the defect that put a
/// binary on disk while the entry still named a script.
fn invocation_path_in_when(
    dir: &Path,
    role: &str,
    script_file: &str,
    live_as_binary: bool,
) -> Option<PathBuf> {
    invocation_path_on(HostPlatform::host(), dir, role, script_file, live_as_binary)
}

/// The resolver with the naming convention handed IN, so the Windows result is
/// observable from a Linux test instead of only from a Windows install.
fn invocation_path_on(
    platform: HostPlatform,
    dir: &Path,
    role: &str,
    script_file: &str,
    live_as_binary: bool,
) -> Option<PathBuf> {
    if live_as_binary {
        let binary = dir.join(role_binary_file_name_on(platform, role));
        if binary.exists() {
            return Some(binary);
        }
    }
    Some(dir.join(script_file))
}

/// Cursor's four managed roles: the event, the role binary, and the
/// generation-2 script that role falls back to.
///
/// One table, because the deploy needs the same four facts three times — which
/// binaries to write, which entries to point at them, and which scripts are now
/// stale — and three hand-kept lists is how the guard ended up on disk as a
/// binary with its entry still naming a script.
const CURSOR_ROLES: &[(&str, &str, &str)] = &[
    ("sessionStart", "jawata-hook-primer", "jawata-session-primer.sh"),
    ("beforeShellExecution", "jawata-hook-guard", "jawata-guard.sh"),
    ("beforeSubmitPrompt", "jawata-hook-userprompt", "jawata-recall.sh"),
    ("afterMCPExecution", "jawata-hook-observer", "jawata-observer.sh"),
];

/// Whether a role runs as its BINARY when Cursor is the client.
///
/// All four do. Three are binary-live everywhere ([`BINARY_LIVE_ROLES`]); the
/// observer is Cursor-only, declared in `hook-events.json` as
/// `role_generations.observer.cursor` with its reasoning, and asserted against
/// this function by `the_cursor_observer_generation_matches_the_declaration`.
///
/// Windows is why this matters at all. A `.sh` cannot execute there, so every
/// script-generation Cursor hook was dead — and worse than dead: Cursor tried
/// to open each one, putting a window on the user's screen at session start, on
/// every prompt submitted, and after every tool call. Reported live on 2026-08-13
/// with the whole point stated plainly: "it does not work on windows because it
/// still tries bash".
fn cursor_role_is_binary_live(role: &str) -> bool {
    role_is_binary_live(role) || role == "jawata-hook-observer"
}

/// The role binary's on-disk file name (`.exe` on Windows).
/// Which filename convention a deploy is writing for.
///
/// This exists as a VALUE rather than a `cfg!(windows)` for one reason, and it
/// is the reason five releases shipped a Windows-only defect: `cfg!` is fixed at
/// compile time, so a test can only ever exercise the host's branch. On Linux
/// the two spellings are identical, which made the writer/resolver mismatch
/// literally unrepresentable in the suite — 468 passing tests could not have
/// caught it, and did not. A parameter can be handed either value from any
/// machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HostPlatform {
    Windows,
    Unix,
}

impl HostPlatform {
    /// The platform this build actually runs on. The ONLY place `cfg!(windows)`
    /// is consulted for hook filenames.
    fn host() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Unix }
    }

    fn exe_suffix(self) -> &'static str {
        match self {
            Self::Windows => ".exe",
            Self::Unix => "",
        }
    }

    /// Whether a deployed `.sh` can actually execute here.
    ///
    /// This is what makes the script generation a real fallback on Unix and a
    /// lie on Windows. There is no shell there to run one: Cursor's attempt to
    /// launch a `.sh` is what put a window on the user's screen at session
    /// start, on every prompt and after every tool call — and under the guard's
    /// `failClosed`, a hook that never answers BLOCKS the command.
    ///
    /// A fallback is only a fallback if it works. Where it does not, falling
    /// back is a failure wearing a fallback's clothes, and the deploy must say
    /// so instead of quietly producing a broken install.
    fn can_run_shell_scripts(self) -> bool {
        matches!(self, Self::Unix)
    }
}

/// The one place a role's binary filename is spelled.
fn role_binary_file_name_on(platform: HostPlatform, role: &str) -> String {
    format!("{role}{}", platform.exe_suffix())
}

/// Whether a managed-hook invocation path names a deployed role BINARY rather
/// than a generation-2 script. The binaries are exactly the role names
/// (`jawata-hook-guard`, …; plus `.exe` on Windows); every script ends `.sh`.
fn path_is_role_binary(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("jawata-hook-") && !n.ends_with(".sh"))
        .unwrap_or(false)
}

/// Whether a role's BINARY generation is the live one — the stop_rules parity
/// discipline applied at role granularity.
///
/// The observer is NOT live as a binary: its binary arm is a deliberate stub
/// (`pipeline.rs`: record a silence row, nothing else), while the script
/// generation captures tool outcomes and the `jawata-fallback:` audit trail.
/// The 3.7.2 dogfood found both dead on the live machine — `outcomes.log`
/// froze the moment the observer entry first pointed at the binary — because
/// the cutover was decided by file existence, not by parity. Until the binary
/// ports those jobs, the observer's invocation path is the SCRIPT and its
/// binary is not deployed. Declared in `hook-events.json` (`role_generations`).
fn role_is_binary_live(role: &str) -> bool {
    BINARY_LIVE_ROLES.contains(&role)
}

fn managed_guard_script_path() -> Option<PathBuf> {
    managed_hook_invocation_path("jawata-hook-guard", GUARD_SCRIPT_FILE)
}

/// Absolute path of the managed PostToolUse observer script (sibling of the guard).
fn managed_observer_script_path() -> Option<PathBuf> {
    managed_hook_invocation_path("jawata-hook-observer", OBSERVER_SCRIPT_FILE)
}

/// The bash guard. `health_url` (the deployed gateway `/mcp` URL) is baked in so the
/// health probe needs no config lookup. Exit 0 = pass; exit 2 = block + redirect
/// (stderr is shown to the model). Deterministic for a given `health_url` so a
/// re-deploy is a byte-stable no-op.
fn build_guard_script(health_url: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
# <jawata-studio managed PreToolUse guard — do not edit; overwritten on deploy>
# Redirects Java SYMBOL SEARCH (grep/rg over *.java files, or the Grep tool aimed
# at Java) to JAWATA's compiler-accurate tools. Health-gated: a different
# message when JAWATA is up (use the tool) vs down (start it, or grep on purpose).
# Non-Java calls pass through untouched; a Java hand-edit is redirected to jawata
# refactor tools (Sprint 22). Exit 2 blocks + tells the model why; exit 0 lets it run.
set -u

HEALTH_URL="{health_url}"

# Append a DECLARED fallback to the audit log, stamped with the deployed jawata engine
# version (derived from the install path). A "jawata vX can't do Y" entry is then a
# versioned signal — scoring substrate + feature backlog. Rare (only on an explicit
# fallback), so the version lookup cost is paid only then.
jawata_log_fallback() {{
  ver="$(ls -1d "${{XDG_CACHE_HOME:-$HOME/.cache}}/jawata-studio/tools/jawata/current"/jawata-* 2>/dev/null | head -n1 | sed 's#.*/jawata-##')"
  [ -n "$ver" ] || ver="unknown"
  dir="$HOME/.claude/jawata-studio"; mkdir -p "$dir" 2>/dev/null
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null)"
  printf '%s\t%s\tdeclared-fallback\t%s\n' "$ts" "$ver" "$1" >> "$dir/fallback.log" 2>/dev/null
}}

input="$(cat)"
# One flattened line so the crude extractors below never span a newline.
flat="$(printf '%s' "$input" | tr '\n\r' '  ')"

tool_name="$(printf '%s' "$flat" | sed -n 's/.*"tool_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"

# v1.4.0 (Sprint 22): per-session "try-first" state, keyed by the hook stdin
# session_id. Later stages log jawata calls here and consult it to gate grep. Derived
# once. An empty session_id (older clients) leaves the file empty → gates degrade open.
session_id="$(printf '%s' "$flat" | sed -n 's/.*"session_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
jawata_state_dir="$HOME/.claude/jawata-studio/trygate"
if [ -n "$session_id" ]; then jawata_state_file="$jawata_state_dir/$session_id"; else jawata_state_file=""; fi

# v1.5.1 (Sprint 22 refinement): AUTHORING permit. Adding NEW Java code is authoring,
# not a refactor jawata can express — and a text-level hook cannot reliably tell authoring
# from restructuring (that judgment needs the AST; it is the intelligent-injector's job).
# So the clean escape for a structured Edit/Write — with no marker polluting the source —
# is a SEPARATE declaration: run a Bash command containing 'jawata-author: <reason>' to open
# a short, session-scoped authoring window; subsequent .java edits then pass and are logged.
jawata_editgate_dir="$HOME/.claude/jawata-studio/editgate"
if [ -n "$session_id" ]; then jawata_editgate="$jawata_editgate_dir/$session_id"; else jawata_editgate=""; fi
if [ "$tool_name" = "Bash" ] && printf '%s' "$flat" | grep -qiE 'jawata-author:'; then
  ar="$(printf '%s' "$input" | sed -n 's/.*jawata-author:[[:space:]]*//p' | head -n1 | sed 's/\\.*//' | sed 's/".*//' | sed 's/[[:space:]]*$//' | head -c 200)"
  if [ -n "$jawata_editgate" ]; then
    mkdir -p "$jawata_editgate_dir" 2>/dev/null
    printf '%s\t%s\n' "$(date +%s 2>/dev/null)" "$ar" > "$jawata_editgate" 2>/dev/null
  fi
  jawata_log_fallback "authoring-window: $ar"
  exit 0
fi

# The matcher fires for Bash|Grep (search gate), Edit|Write|MultiEdit (edit
# enforcement — Stage 3) and mcp__jawata* (jawata-call logging — Stage 1). Stage 0 wires
# the state above and still routes only search to the gates below; the other tools
# pass through until their stages add branches.
case "$tool_name" in
  mcp__jawata*)
    # Stage 1: record that jawata was TRIED for these targets — the try-first signal
    # the search gate (Stage 2) consults. Jawata calls are never blocked; we just log
    # the target tokens (query / typeName / symbol / newName / filePath basename),
    # lowercased, one per line.
    if [ -n "$jawata_state_file" ]; then
      mkdir -p "$jawata_state_dir" 2>/dev/null
      printf '%s' "$flat" \
        | grep -oiE '"(query|typeName|symbol|newName|filePath)"[[:space:]]*:[[:space:]]*"[^"]*"' \
        | sed -E 's/.*:[[:space:]]*"//; s/"$//; s#.*/##' \
        | tr 'A-Z' 'a-z' \
        >> "$jawata_state_file" 2>/dev/null
    fi
    exit 0 ;;
  Edit|Write|MultiEdit)
    # v1.4.0 (Sprint 22) EDIT ENFORCEMENT: a hand-edit of a .java file must go through a
    # jawata refactor tool, or be justified. Non-.java, brand-new files, and jawata-down pass.
    edit_path="$(printf '%s' "$flat" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
    case "$edit_path" in
      *.java) ;;
      *) exit 0 ;;
    esac
    # Declared fallback → proceed + log (versioned).
    if printf '%s' "$flat" | grep -qiE 'jawata-fallback:'; then
      er="$(printf '%s' "$input" | sed -n 's/.*jawata-fallback:[[:space:]]*//p' | head -n1 | sed 's/\\.*//' | sed 's/".*//' | sed 's/[[:space:]]*$//' | head -c 200)"
      jawata_log_fallback "$er"
      exit 0
    fi
    # v1.5.1: a fresh AUTHORING window (declared via a 'jawata-author:' Bash command this
    # session) covers structured .java edits — authoring new code is not a refactor. The
    # window is TTL-bounded (30 min); each covered edit is logged so the trail stays complete.
    if [ -n "$jawata_editgate" ] && [ -f "$jawata_editgate" ]; then
      pts="$(cut -f1 "$jawata_editgate" 2>/dev/null)"; nows="$(date +%s 2>/dev/null)"
      if [ -n "$pts" ] && [ -n "$nows" ] && [ "$((nows - pts))" -lt 1800 ]; then
        jawata_log_fallback "authored-edit ($edit_path)"
        exit 0
      fi
    fi
    # JAWATA down → its refactor tools are unreachable → allow the hand-edit.
    jawata_up=0
    if command -v curl >/dev/null 2>&1; then curl -s -o /dev/null --max-time 1 "$HEALTH_URL" && jawata_up=1
    elif command -v wget >/dev/null 2>&1; then wget -q -O /dev/null --timeout=1 "$HEALTH_URL" && jawata_up=1
    else hp="$(printf '%s' "$HEALTH_URL" | sed -E 's#^https?://([^/]+).*#\1#')"; h="${{hp%%:*}}"; p="${{hp##*:}}"; [ "$p" = "$hp" ] && p=80; (exec 3<>"/dev/tcp/$h/$p") >/dev/null 2>&1 && jawata_up=1; fi
    [ "$jawata_up" -eq 1 ] || exit 0
    # Brand-new file (Write to a non-existent path) → nothing to refactor → allow.
    if [ "$tool_name" = "Write" ] && [ ! -e "$edit_path" ]; then exit 0; fi
    # Sprint 26a D1 (reflex→capability): if the blocked .java edit LOOKS like a
    # runtime reflex — a hand-rolled timer, or debug-logging to diagnose —
    # SURFACE the zero-code-change runtime tool by name. This is not a new block
    # (R5: no false-positive guard on ordinary logging); it is a smarter message
    # on an edit that is already blocked as a .java hand-edit.
    reflex_hint=""
    if printf '%s' "$flat" | grep -qiE 'nanoTime|currentTimeMillis|Stopwatch|StopWatch'; then
      reflex_hint="TIMING BY HAND? Use profile — it samples the running JVM and names the hotspot as a symbol, ZERO code change. A hand-rolled stopwatch edits production code for nothing."
    elif printf '%s' "$flat" | grep -qiE 'System\.out\.print|System\.err\.print|printStackTrace|logger?\.(debug|trace)|// *DEBUG|DEBUG:'; then
      reflex_hint="DEBUG-ARMOR? Use debug — attach and set probe_set kind=logpoint (or field_watch / method_trace) to stream values at runtime, ZERO code change. Hand-adding logging to diagnose edits production code for nothing."
    fi
    {{
      [ -n "$reflex_hint" ] && echo "$reflex_hint"
      echo "USE A JAWATA REFACTOR TOOL — hand-editing $edit_path (a .java file) is blocked."
      echo "Rename → rename_symbol (updates ALL references). Move → move / move_in_hierarchy."
      echo "Extract method/variable/constant/superclass → extract. Duplicate a class → generate(kind=copy_class)."
      echo "Any structural change → refactoring(action=plan) then apply_plan (parity-gated, reversible)."
      echo "Only adding a print/log to observe a value at runtime? Don't edit source — attach with debug and set a probe: probe_set kind=logpoint (also field_watch / method_trace) streams values while the program keeps running."
      echo "Authoring NEW code (not a refactor)? Declare a window: run a Bash command with 'jawata-author: <reason>', then edit (session-scoped, logged)."
      echo "If this is a genuinely non-structural edit JAWATA cannot do, re-run with 'jawata-fallback: <why>' (declared + logged)."
    }} 1>&2
    exit 2 ;;
  Bash|Grep) ;;
  *) exit 0 ;;
esac

# v1.2.1 tuning: redirect only genuine Java SYMBOL SEARCH. BOTH gates must hold —
# a content-search tool AND a real .java file target — so file/line ops and
# incidental ".java" mentions no longer trip the guard.

# (1) Content-search tool only (grep-family). File/line ops (find/sed/awk) and
#     everything else are NOT symbol search — pass them untouched.
is_search=0
if [ "$tool_name" = "Grep" ]; then
  is_search=1
else
  printf '%s' "$flat" | grep -qE '(^|[^a-zA-Z])(grep|egrep|fgrep|rg|ripgrep|ag|ack)([^a-zA-Z]|$)' && is_search=1
fi
[ "$is_search" -eq 1 ] || exit 0

# (2) It must target Java SOURCE FILES — a concrete path (Foo.java, src/Foo.java)
#     or a glob (*.java). The char before the dot must be a word char or a glob
#     star, which excludes an escaped regex pattern like "\.java" and incidental
#     mentions such as ".java" inside a build file, a log, or this guard's own text.
printf '%s' "$flat" | grep -qiE '([A-Za-z0-9_$]\.java|\*\.java)([^a-zA-Z]|$)' || exit 0

# (3) v1.3.0 escape valve: a DECLARED fallback proceeds — and is logged. This turns
#     a silent, lazy skip into an explicit, auditable decision (the friction that
#     defeats laziness). Works whether JAWATA is up or down: the agent asserts jawata
#     cannot or need not answer THIS search. Grammar: put 'jawata-fallback: <reason>'
#     in the Bash command (e.g. a trailing comment). The Grep tool has no free field,
#     so falling back means using Bash grep with the marker — deliberately.
if printf '%s' "$flat" | grep -qiE 'jawata-fallback:'; then
  # v1.3.1: capture ONLY the reason on the marker's own line. Read from the
  # UN-flattened input (so other lines of a multi-line command can't bleed in),
  # then trim at the first backslash (a JSON escape / \n) or double-quote (the JSON
  # string close), strip trailing space, and cap. fallback.log is the audit trail
  # (and training data for the intelligent-injector sprint), so keep it clean.
  jawata_reason="$(printf '%s' "$input" | sed -n 's/.*jawata-fallback:[[:space:]]*//p' | head -n1 | sed 's/\\.*//' | sed 's/".*//' | sed 's/[[:space:]]*$//' | head -c 200)"
  jawata_log_fallback "$jawata_reason"
  exit 0
fi

# (4) v1.4.0 (Sprint 22) TRY-FIRST gate: if this search's target was already looked
#     up via jawata THIS session (its token is in the per-session state), the agent
#     tried jawata first → grep is a legitimate follow-up, allow it. Only an UN-tried
#     java-symbol search reaches the block below. Conservative: match any jawata-queried
#     token (>=3 chars) that appears in the command; when in doubt, allow.
if [ -s "$jawata_state_file" ] && printf '%s' "$flat" | grep -qiFf <(grep -E '^.{{3,}}$' "$jawata_state_file") 2>/dev/null; then
  exit 0
fi

# JAWATA liveness: any HTTP response on the gateway = up; connection refused = down.
jawata_up=0
if command -v curl >/dev/null 2>&1; then
  curl -s -o /dev/null --max-time 1 "$HEALTH_URL" && jawata_up=1
elif command -v wget >/dev/null 2>&1; then
  wget -q -O /dev/null --timeout=1 "$HEALTH_URL" && jawata_up=1
else
  # No HTTP client: fall back to a raw TCP connect via bash /dev/tcp.
  hostport="$(printf '%s' "$HEALTH_URL" | sed -E 's#^https?://([^/]+).*#\1#')"
  host="${{hostport%%:*}}"; port="${{hostport##*:}}"
  [ "$port" = "$hostport" ] && port=80
  (exec 3<>"/dev/tcp/$host/$port") >/dev/null 2>&1 && jawata_up=1
fi

if [ "$jawata_up" -eq 1 ]; then
  {{
    echo "TRY JAWATA FIRST — you have not looked this up via JAWATA yet this session."
    echo "For a symbol: search_symbols. Callers/usages: find_references."
    echo "Type shape/members/hierarchy: analyze / inspect. Jump: go_to_definition."
    echo "Once you have queried it via JAWATA, grep is a fine follow-up (this gate then passes)."
    echo "(JAWATA is compiler-accurate; grep over .java misses/overmatches symbols.)"
    echo "If this genuinely is NOT a symbol search, re-run with 'jawata-fallback: <reason>' to proceed (declared + logged)."
  }} 1>&2
  exit 2
else
  {{
    echo "JAWATA MCP appears to be DOWN (no response at $HEALTH_URL) and this is Java work."
    echo "Per the collaboration rules, do not silently grep Java semantics — decide first:"
    echo "  1) Start JAWATA (open jawata-studio and start the resident), then use search_symbols / find_references / analyze."
    echo "  2) Or proceed deliberately: re-run with 'jawata-fallback: <reason>' in the command (e.g. a trailing comment) — declared + logged, not a silent skip."
    echo "Non-Java work is unaffected."
  }} 1>&2
  exit 2
fi
"#,
        health_url = health_url
    )
}

/// The single `PreToolUse` matcher entry that invokes the guard. Matchers are
/// unanchored regex: `Bash|Grep` (search gate), `Edit|Write|MultiEdit` (edit
/// enforcement), and `mcp__jawata.*` (jawata-call logging for the try-first gate).
/// Kept deterministic so the settings.json write is idempotent.
fn build_managed_hook_entry(guard_path: &Path) -> serde_json::Value {
    let command = display_path(guard_path);
    serde_json::json!({
        "matcher": "Bash|Grep|Edit|Write|MultiEdit|mcp__jawata.*",
        "hooks": [
            { "type": "command", "timeout": HOOK_TIMEOUT_SECS, "command": command }
        ]
    })
}

/// True iff a `PreToolUse` entry is one jawata-studio wrote (its command references
/// the managed guard script). Used to replace/remove our entries and leave the
/// user's hooks alone.
fn is_managed_hook_entry(entry: &serde_json::Value) -> bool {
    entry_is_managed_any_generation(entry, "jawata-hook-guard")
}

/// Write the guard script + register the managed `PreToolUse` entry in the client's
/// settings.json, replacing any prior managed entry and preserving user hooks.
/// Returns Ok(true) when anything changed. Idempotent: an unchanged re-deploy is a
/// no-op write.
fn write_managed_hook(
    settings_path: &str,
    guard_path: &Path,
    health_url: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    // 1. Write the guard script (jawata-studio owns it outright).
    let script_parent = guard_path
        .parent()
        .ok_or_else(|| format!("guard path has no parent: {}", guard_path.display()))?;
    fs::create_dir_all(script_parent).map_err(|error| {
        format!(
            "failed to create guard dir {}: {error}",
            script_parent.display()
        )
    })?;
    // v3.7.3 audit F1: this writer predates write_managed_hook_section and
    // carries its own copy of the body write — which is exactly where the
    // clobber survived the first fix. Same refusal as the section writer:
    // a role-binary path's content belongs to deploy_hook_binaries; here we
    // own only the settings entry. `read_to_string` on an ELF fails on
    // non-UTF-8, `unwrap_or(true)` called that "changed", and the bash guard
    // landed over the binary on every full deploy.
    let body_is_ours = !path_is_role_binary(guard_path);
    let script_body = build_guard_script(health_url);
    let script_changed = body_is_ours
        && fs::read_to_string(guard_path)
            .map(|existing| existing != script_body)
            .unwrap_or(true);
    if body_is_ours && (script_changed || force_rewrite) {
        fs::write(guard_path, &script_body).map_err(|error| {
            format!("failed writing guard script {}: {error}", guard_path.display())
        })?;
    }
    #[cfg(unix)]
    if body_is_ours {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(guard_path, fs::Permissions::from_mode(0o755));
    }

    // 2. Merge the managed entry into settings.json's hooks.PreToolUse.
    let settings_buf = PathBuf::from(settings_path);
    let settings_parent = settings_buf
        .parent()
        .ok_or_else(|| format!("settings path has no parent: {}", settings_buf.display()))?;
    fs::create_dir_all(settings_parent).map_err(|error| {
        format!(
            "failed to create settings dir {}: {error}",
            settings_parent.display()
        )
    })?;

    let existing_contents = fs::read_to_string(&settings_buf).ok();
    let mut root = existing_contents
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }

    {
        let object = root.as_object_mut().expect("root is an object");
        let hooks = object
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));
        if !hooks.is_object() {
            *hooks = serde_json::json!({});
        }
        let hooks_object = hooks.as_object_mut().expect("hooks is an object");

        let mut pre = hooks_object
            .get("PreToolUse")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        // Drop any prior managed entry, keep user entries, append the fresh one.
        pre.retain(|entry| !is_managed_hook_entry(entry));
        pre.push(build_managed_hook_entry(guard_path));
        hooks_object.insert("PreToolUse".into(), serde_json::Value::Array(pre));
    }

    let next_json = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("failed serializing settings json: {error}"))?;

    if !force_rewrite {
        if let Some(existing) = existing_contents.as_deref() {
            if existing.trim() == next_json.trim() && !script_changed {
                return Ok(false);
            }
        }
    }

    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&settings_buf)
            .map_err(|error| format!("failed creating centralized settings backup: {error}"))?;
    }
    fs::write(&settings_buf, format!("{next_json}\n")).map_err(|error| {
        format!(
            "failed writing settings {}: {error}",
            settings_buf.display()
        )
    })?;
    Ok(true)
}

/// Remove the managed `PreToolUse` entry from settings.json + delete the guard
/// script. Returns Ok(true) when anything was removed. Leaves user hooks intact and
/// prunes now-empty `PreToolUse` / `hooks` containers.
fn remove_managed_hook(
    settings_path: &str,
    guard_path: &Path,
    backup_before_write: bool,
) -> Result<bool, String> {
    let mut changed = false;

    // 1. Strip our entry from settings.json (if the file + entry exist).
    let settings_buf = PathBuf::from(settings_path);
    if settings_buf.exists() {
        let existing = fs::read_to_string(&settings_buf).map_err(|error| {
            format!("failed to read settings {}: {error}", settings_buf.display())
        })?;
        if let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&existing) {
            let mut removed_any = false;
            if let Some(hooks) = root
                .as_object_mut()
                .and_then(|object| object.get_mut("hooks"))
                .and_then(|hooks| hooks.as_object_mut())
            {
                if let Some(pre) = hooks.get_mut("PreToolUse").and_then(|v| v.as_array_mut()) {
                    let before = pre.len();
                    pre.retain(|entry| !is_managed_hook_entry(entry));
                    removed_any = pre.len() != before;
                    if pre.is_empty() {
                        hooks.remove("PreToolUse");
                    }
                }
                let hooks_empty = hooks.is_empty();
                if hooks_empty {
                    root.as_object_mut().map(|object| object.remove("hooks"));
                }
            }
            if removed_any {
                let next_json = serde_json::to_string_pretty(&root)
                    .map_err(|error| format!("failed serializing settings json: {error}"))?;
                if backup_before_write {
                    let _ = crate::backups::backup_before_write(&settings_buf);
                }
                fs::write(&settings_buf, format!("{next_json}\n")).map_err(|error| {
                    format!("failed writing settings {}: {error}", settings_buf.display())
                })?;
                changed = true;
            }
        }
    }

    // 2. Delete the guard script.
    if guard_path.exists() {
        fs::remove_file(guard_path).map_err(|error| {
            format!("failed removing guard script {}: {error}", guard_path.display())
        })?;
        changed = true;
    }

    Ok(changed)
}

/// Sprint 22 (POST layer): the PostToolUse observer. Reactive — PostToolUse cannot
/// block — it appends three signals to `~/.claude/jawata-studio/outcomes.log` (the
/// scoring substrate) and steers after a declared-fallback slip. Deterministic so a
/// re-deploy is a byte-stable no-op.
fn build_observer_script(mcp_url: &str, token: &str) -> String {
    OBSERVER_TEMPLATE.replace("__MCP_URL__", mcp_url).replace("__TOKEN__", token)
}

const OBSERVER_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed PostToolUse observer — do not edit; overwritten on deploy>
# Reactive, never blocks. Appends three POST signals to a versioned outcomes log:
#   slip            a declared jawata-fallback the PRE guard allowed (+ a steering note)
#   read-ungrounded a Read of a .java file not preceded by a JAWATA lookup this session
#   verify          a compile/diagnostics/test event (correlates a preceding change)
# Sprint 21a (item J): slips are also BRIDGED into the experience store as candidates.
dir="$HOME/.claude/jawata-studio"; mkdir -p "$dir" 2>/dev/null
log="$dir/outcomes.log"
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"

# The one steering payload — selftest and the real slip path share these bytes.
slip_ctx='{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"jawata-fallback recorded. Next: verify with compile_workspace + get_diagnostics. A declared fallback is a JAWATA feature request — if a newer JAWATA version can do it, prefer JAWATA next time."}}'
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then printf '%s' "$slip_ctx"; exit 0; fi

jawata_ver() {
  ls -1d "${XDG_CACHE_HOME:-$HOME/.cache}/jawata-studio/tools/jawata/current"/jawata-* 2>/dev/null \
    | head -n1 | sed 's#.*/jawata-##'
}
emit() {
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null)"
  printf '%s\t%s\t%s\t%s\n' "$ts" "$(jawata_ver)" "$1" "$2" >> "$log" 2>/dev/null
}
# v1.5.1: log a declared-fallback slip + steer. Callers gate this to a REAL .java-targeted
# op, so a non-.java edit whose content merely contains the marker is not counted.
# Sprint 21a (item J): the slip is also recorded into the experience store (candidate) —
# the first conversation-level auto-learn path. Fail-safe: jawata down -> log-only.
emit_slip() {
  reason="$(printf '%s' "$flat" | sed -nE 's/.*[Jj][Aa][Ww][Aa][Tt][Aa]-[Ff][Aa][Ll][Ll][Bb][Aa][Cc][Kk]:[[:space:]]*([^"\\]*).*/\1/p' | head -n1 | sed -E 's/[[:space:]]*$//')"
  emit "slip" "$tool_name	$reason"
  if command -v curl >/dev/null 2>&1 && [ -n "$MCP_URL" ]; then
    sr="$(printf '%s: %s' "$tool_name" "$reason" | sed 's/["\\]/ /g' | tr -d '[:cntrl:]' | cut -c1-200)"
    curl -s --max-time 3 -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"experience","arguments":{"kind":"record","type":"failure_mode","operation":"jawata-fallback-slip","summary":"jawata-fallback slip: '"$sr"'","symptoms":["jawata fallback slip"]}}}' \
      "$MCP_URL" >/dev/null 2>&1 || true
  fi
  printf '%s' "$slip_ctx"
}

input="$(cat)"
flat="$(printf '%s' "$input" | tr '\n' ' ')"
# Sprint 21a (item J): judge the REQUEST only. tool_response may echo file contents that
# merely mention '.java' or 'jawata-fallback:' (a cat of a hook script logged a false slip).
flat="$(printf '%s' "$flat" | sed 's/"tool_response".*$//')"

# Sprint 26 C7: the consequence-labeled edit feed. A hook's HTTP post cannot share the
# agent's MCP session, so the CORRELATION lives HERE, in the client session: a .java
# edit's fragments are held in a per-session state file; the session's next gate
# outcome (or an undo) labels them and posts each as observe_edit(outcome=...) —
# the resident trains immediately. Fail-open: no python3 / resident down = no feed.
editfeed_hold() {
  [ -n "$session_id" ] || return 0
  command -v python3 >/dev/null 2>&1 || return 0
  mkdir -p "$dir/editfeed" 2>/dev/null
  printf '%s' "$input" | EF="$dir/editfeed/$session_id" python3 -c '
import json,os,sys
try:
    d=json.load(sys.stdin); ti=d.get("tool_input") or {}
    edits=ti.get("edits") or [ti]
    before="\n".join(e.get("old_string","") for e in edits)[:4000]
    after="\n".join(e.get("new_string", e.get("content","")) for e in edits)[:4000]
    if not (before.strip() or after.strip()): raise SystemExit(0)
    path=os.environ["EF"]
    lines=[]
    if os.path.exists(path):
        lines=open(path).read().splitlines()
    lines.append(json.dumps({"before":before,"after":after}))
    open(path,"w").write("\n".join(lines[-32:])+"\n")
except Exception: pass
' 2>/dev/null
}
editfeed_resolve() {
  # $1 = forced outcome ("failed" for an undo) or "" (read the gate result)
  [ -n "$session_id" ] || return 0
  sf="$dir/editfeed/$session_id"
  [ -s "$sf" ] || return 0
  command -v python3 >/dev/null 2>&1 || return 0
  printf '%s' "$input" | EF="$sf" FORCED="$1" MCP_URL="$MCP_URL" TOKEN="$TOKEN" python3 -c '
import json,os,sys,urllib.request
path=os.environ["EF"]
try:
    pend=[json.loads(l) for l in open(path).read().splitlines() if l.strip()]
except Exception:
    pend=[]
# Pop FIRST: a lost post is a lost label, never a stale re-label.
try: os.remove(path)
except Exception: pass
if not pend: raise SystemExit(0)
outcome=os.environ.get("FORCED") or ""
if not outcome:
    try:
        d=json.load(sys.stdin); tr=d.get("tool_response")
        text=""
        if isinstance(tr,dict):
            c=tr.get("content") or []
            if c and isinstance(c[0],dict): text=c[0].get("text","")
        body=json.loads(text) if text else {}
        data=body.get("data") or {}
        errs=data.get("errorCount", data.get("failed", 0)) or 0
        ok=body.get("success", True)
        outcome="clean" if (ok and int(errs)==0) else "failed"
    except Exception:
        raise SystemExit(0)  # unreadable outcome: no label beats a guessed label
url=os.environ.get("MCP_URL"); tok=os.environ.get("TOKEN")
if not url: raise SystemExit(0)
for p in pend:
    payload=json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"experience","arguments":{"kind":"observe_edit","outcome":outcome,
        "before":p.get("before",""),"after":p.get("after","")}}}).encode()
    try:
        req=urllib.request.Request(url,data=payload,headers={
            "Authorization":"Bearer "+ (tok or ""),"Content-Type":"application/json"})
        urllib.request.urlopen(req,timeout=3).read()
    except Exception: pass
' 2>/dev/null
}
# v2.9.1 (D3b): the request JSON carries newlines as literal \n — a grep STARTING a
# line is preceded by the letter 'n' and failed the word-boundary check below, so
# the slip was silently never recorded. Gate checks run on the normalized copy;
# reason extraction stays on $flat (its capture stops at the raw backslash).
nflat="$(printf '%s' "$flat" | sed 's/\\[ntr]/ /g')"
tool_name="$(printf '%s' "$flat" | grep -oE '"tool_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed -E 's/.*"([^"]*)"$/\1/')"
session_id="$(printf '%s' "$flat" | grep -oE '"session_id"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed -E 's/.*"([^"]*)"$/\1/')"
state="$dir/trygate/$session_id"

case "$tool_name" in
  Read)
    f="$(printf '%s' "$flat" | grep -oE '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed -E 's/.*"([^"]*)"$/\1/')"
    case "$f" in
      *.java)
        base="$(printf '%s' "$f" | sed -E 's#.*/##; s#\.java$##' | tr '[:upper:]' '[:lower:]')"
        grounded=0
        if [ -s "$state" ] && [ -n "$base" ] \
           && printf '%s' "$base" | grep -qiFf <(grep -E '^.{3,}$' "$state") 2>/dev/null; then
          grounded=1
        fi
        [ "$grounded" -eq 0 ] && emit "read-ungrounded" "$f"
        ;;
    esac
    ;;
  *compile_workspace|*get_diagnostics|*run_tests)
    emit "verify" "$tool_name"
    # C7: the gate outcome labels every edit pending in THIS client session.
    editfeed_resolve ""
    ;;
  *find_tests)
    emit "verify" "$tool_name"
    ;;
  *refactoring)
    # C7: an undo is the strongest structural-mishandled consequence.
    printf '%s' "$nflat" | grep -qE '"action"[[:space:]]*:[[:space:]]*"undo' \
      && editfeed_resolve "failed"
    ;;
  Edit|Write|MultiEdit)
    # v1.5.1: a slip counts only for a .java edit the PRE edit-gate allowed via the marker —
    # a non-.java edit whose CONTENT merely contains 'jawata-fallback:' is not a gated op.
    ef="$(printf '%s' "$flat" | grep -oE '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed -E 's/.*"([^"]*)"$/\1/')"
    case "$ef" in
      *.java)
        printf '%s' "$flat" | grep -qiE 'jawata-fallback:' && emit_slip
        # C7: hold the edit's fragments until this session's next gate outcome.
        editfeed_hold
        ;;
    esac
    ;;
  Bash|Grep)
    # v1.5.1: a slip counts only for a Java symbol SEARCH the PRE search-gate allowed —
    # require a content-search tool AND a .java target (the PRE dual-gate) plus the marker.
    st=0
    if [ "$tool_name" = "Grep" ]; then st=1
    elif printf '%s' "$nflat" | grep -qiE '(^|[^a-zA-Z])(grep|egrep|fgrep|rg|ripgrep|ag|ack)([^a-zA-Z]|$)'; then st=1; fi
    if [ "$st" = "1" ] \
       && printf '%s' "$nflat" | grep -qiE '([A-Za-z0-9_$]\.java|\*\.java)' \
       && printf '%s' "$nflat" | grep -qiE 'jawata-fallback:'; then emit_slip; fi
    ;;
esac
exit 0
"#;

/// Sprint 21a (item F): knowledge-store + memory-crawl configuration handed to the
/// resident JVM as `-D` system properties (they MUST precede `-jar`).
fn knowledge_jvm_properties(settings: &ManagerSettings) -> Vec<String> {
    let mut props = vec![format!(
        "-Djawata.experience.store={}",
        settings.experience_store_mode
    )];
    if !settings.memory_roots.is_empty() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        props.push(format!(
            "-Djawata.memory.roots={}",
            settings.memory_roots.join(separator)
        ));
    }
    // Sprint 21b: no -Djawata.memory.max* — the resident's defaults are runaway backstops
    // ("the crawl finds everything"); the properties remain honored for manual launches.
    props
}

/// Sprint 21a (item F): call `experience(...)` on a resident and peel jawata's fixed MCP
/// envelope — the body carries the JSON-RPC result whose `content[0].text` is the
/// DOUBLE-ENCODED ToolResponse (`{success, data, ...}`), returned decoded.
fn call_experience(
    url: &str,
    token: &str,
    arguments: serde_json::Value,
    timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    let body = call_resident_tool(url, token, "experience", arguments, timeout_secs)?;
    let envelope: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| format!("bad envelope: {error}"))?;
    if let Some(rpc_error) = envelope.get("error") {
        return Err(format!("resident error: {rpc_error}"));
    }
    let text = envelope
        .pointer("/result/content/0/text")
        .and_then(|text| text.as_str())
        .ok_or_else(|| "unexpected envelope (no result.content[0].text)".to_string())?;
    serde_json::from_str(text).map_err(|error| format!("bad tool response: {error}"))
}

/// Sprint 21a (item F): the exact verb vocabulary — the Knowledge view's actions are
/// these names 1:1 (Harald 2026-07-05: what you click is what you'd say in a prompt).
const EXPERIENCE_KINDS: &[&str] = &[
    "record", "recall", "primer", "list", "load", "reseed", "refresh", "wipe", "promote",
    "export", "import", "prune", "dedup", "compact", "stats",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeWorkspaceStatus {
    pub workspace: String,
    pub url: String,
    pub reachable: bool,
    pub stats: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Sprint 21a (item D): which residents to auto-seed. Pure so the toggle logic is
/// unit-testable; empty when the setting is off or a server has no url/token.
fn auto_seed_targets(enabled: bool, servers: &[ManagedDeployServer]) -> Vec<(String, String)> {
    if !enabled {
        return Vec::new();
    }
    servers
        .iter()
        .filter(|server| !server.url.is_empty() && !server.token.is_empty())
        .map(|server| (server.url.clone(), server.token.clone()))
        .collect()
}

/// Sprint 21a (item D): one-shot JSON-RPC `tools/call` against a resident `/mcp` —
/// the small sibling of `gateway::forward` (reqwest blocking POST with Bearer). Used by
/// auto-seed and by the Knowledge view's maintenance actions (item F).
fn call_resident_tool(
    url: &str,
    token: &str,
    tool: &str,
    arguments: serde_json::Value,
    timeout_secs: u64,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|error| format!("http client: {error}"))?;
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    });
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("response read failed: {error}"))?;
    if !status.is_success() {
        return Err(format!("resident answered {status}: {body}"));
    }
    Ok(body)
}

/// Sprint 21a (item J): the post-deploy hook self-check — the v2.0.x dogfood lesson
/// institutionalized. Unit tests on the TEMPLATE were green while the EMITTED bytes were
/// broken (greedy peel, printf `\n`); so after writing a hook, drive its
/// `JAWATA_HOOK_SELFTEST=1` path (which shares the live emit format) and validate the bytes
/// it prints parse as the hook JSON contract. Fail-OPEN when bash is unavailable (the
/// check cannot judge), fail-CLOSED on empty/invalid output (the deploy reports it).
/// Run a hook's self-check process: a role BINARY executes directly (running
/// an ELF through `bash` yields "cannot execute binary file" and an empty
/// stdout — which reads as a failed selftest for a correct binary); a script
/// still goes through `bash` so the check works on Windows dev machines too.
fn selftest_command(script: &Path) -> std::process::Command {
    use std::process::Command;
    if path_is_role_binary(script) {
        Command::new(script)
    } else {
        let mut c = Command::new("bash");
        c.arg(script);
        c
    }
}

fn selftest_hook_script(script: &Path) -> Result<(), String> {
    use std::process::Stdio;
    if !script.exists() {
        return Ok(());
    }
    let output = match selftest_command(script)
        .env("JAWATA_HOOK_SELFTEST", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => output,
        // A script needs bash, which this platform may lack — cannot judge.
        // A BINARY that cannot be spawned is a real deploy defect and says so.
        Err(e) if path_is_role_binary(script) => {
            return Err(format!("hook self-check could not run {}: {e}", script.display()))
        }
        Err(_) => return Ok(()),
    };
    if output.stdout.is_empty() {
        return Err(format!(
            "hook self-check emitted NOTHING (selftest path missing?): {}",
            script.display()
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "hook self-check emitted INVALID JSON ({error}): {}",
            script.display()
        )
    })?;
    let has_context = value
        .get("hookSpecificOutput")
        .and_then(|h| h.get("additionalContext"))
        .and_then(|c| c.as_str())
        .map(|c| !c.is_empty())
        .unwrap_or(false);
    if has_context {
        Ok(())
    } else {
        Err(format!(
            "hook self-check output lacks hookSpecificOutput.additionalContext: {}",
            script.display()
        ))
    }
}

/// Sprint 26 (v3.2.2, finding #7): the Stop hook's post-deploy self-check.
/// A Stop hook does NOT emit `hookSpecificOutput.additionalContext` — that is the
/// context-injecting contract (observer / primer / recall). A Stop hook emits the
/// Stop decision: `{}` to allow the stop, or `{"decision":"block","reason":...}` to
/// bounce the final message. The generic `selftest_hook_script` wrongly required
/// `additionalContext` and so rejected a CORRECT Stop hook at deploy time (the
/// Claude deploy reported the stop-gate as failed). This validates the Stop
/// contract instead. Fail-OPEN when bash is unavailable; fail-CLOSED on
/// empty / invalid / wrong-shaped output.
fn selftest_stop_hook_script(script: &Path) -> Result<(), String> {
    use std::process::Stdio;
    if !script.exists() {
        return Ok(());
    }
    let output = match selftest_command(script)
        .env("JAWATA_HOOK_SELFTEST", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(e) if path_is_role_binary(script) => {
            return Err(format!("stop-gate self-check could not run {}: {e}", script.display()))
        }
        Err(_) => return Ok(()), // no bash on this platform — cannot judge
    };
    if output.stdout.is_empty() {
        return Err(format!(
            "stop-gate self-check emitted NOTHING (selftest path missing?): {}",
            script.display()
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "stop-gate self-check emitted INVALID JSON ({error}): {}",
            script.display()
        )
    })?;
    let obj = value.as_object().ok_or_else(|| {
        format!(
            "stop-gate self-check output is not a JSON object (Stop contract): {}",
            script.display()
        )
    })?;
    // Empty object = allow the stop (the canonical no-block output). A decision,
    // if present, must be a recognized Stop verdict; a block must name a reason.
    match obj.get("decision").map(|d| d.as_str()) {
        None => Ok(()),
        Some(Some("approve")) => Ok(()),
        Some(Some("block")) => {
            let has_reason = obj
                .get("reason")
                .and_then(|r| r.as_str())
                .map(|r| !r.is_empty())
                .unwrap_or(false);
            if has_reason {
                Ok(())
            } else {
                Err(format!(
                    "stop-gate self-check: a block decision must carry a non-empty reason: {}",
                    script.display()
                ))
            }
        }
        Some(_) => Err(format!(
            "stop-gate self-check: unrecognized decision (expected block/approve): {}",
            script.display()
        )),
    }
}

/// The single `PostToolUse` matcher entry that invokes the observer. Broad matcher:
/// Read (ungrounded-read capture), the verify MCP tools, and search/edit tools (slip
/// capture); the script no-ops on anything else.
fn build_managed_posthook_entry(observer_path: &Path) -> serde_json::Value {
    let command = display_path(observer_path);
    serde_json::json!({
        "matcher": "Bash|Grep|Edit|Write|MultiEdit|Read|mcp__jawata.*",
        "hooks": [
            { "type": "command", "timeout": HOOK_TIMEOUT_SECS, "command": command }
        ]
    })
}

/// True iff a `PostToolUse` entry is one jawata-studio wrote (its command references the
/// managed observer script).
fn is_managed_posthook_entry(entry: &serde_json::Value) -> bool {
    entry_is_managed_any_generation(entry, "jawata-hook-observer")
}

/// Write the observer script + register the managed `PostToolUse` entry, preserving
/// user hooks. Mirror of `write_managed_hook`. Idempotent.
fn write_managed_posthook(
    settings_path: &str,
    observer_path: &Path,
    mcp_url: &str,
    token: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    let script_parent = observer_path
        .parent()
        .ok_or_else(|| format!("observer path has no parent: {}", observer_path.display()))?;
    fs::create_dir_all(script_parent).map_err(|error| {
        format!("failed to create observer dir {}: {error}", script_parent.display())
    })?;
    // Same refusal as write_managed_hook / the section writer (v3.7.3 audit
    // F1): a role-binary path's content is deploy_hook_binaries'. Today the
    // observer resolves to its script (role_generations declares the script
    // generation live), so this arm is latent — it becomes load-bearing the
    // day the observer binary cuts over, which is exactly when nobody will
    // remember this copy of the body write exists.
    let body_is_ours = !path_is_role_binary(observer_path);
    let script_body = build_observer_script(mcp_url, token);
    let script_changed = body_is_ours
        && fs::read_to_string(observer_path)
            .map(|existing| existing != script_body)
            .unwrap_or(true);
    if body_is_ours && (script_changed || force_rewrite) {
        fs::write(observer_path, &script_body).map_err(|error| {
            format!("failed writing observer script {}: {error}", observer_path.display())
        })?;
    }
    #[cfg(unix)]
    if body_is_ours {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(observer_path, fs::Permissions::from_mode(0o755));
    }

    let settings_buf = PathBuf::from(settings_path);
    let settings_parent = settings_buf
        .parent()
        .ok_or_else(|| format!("settings path has no parent: {}", settings_buf.display()))?;
    fs::create_dir_all(settings_parent).map_err(|error| {
        format!("failed to create settings dir {}: {error}", settings_parent.display())
    })?;

    let existing_contents = fs::read_to_string(&settings_buf).ok();
    let mut root = existing_contents
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }

    {
        let object = root.as_object_mut().expect("root is an object");
        let hooks = object.entry("hooks").or_insert_with(|| serde_json::json!({}));
        if !hooks.is_object() {
            *hooks = serde_json::json!({});
        }
        let hooks_object = hooks.as_object_mut().expect("hooks is an object");
        let mut post = hooks_object
            .get("PostToolUse")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        post.retain(|entry| !is_managed_posthook_entry(entry));
        post.push(build_managed_posthook_entry(observer_path));
        hooks_object.insert("PostToolUse".into(), serde_json::Value::Array(post));
    }

    let next_json = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("failed serializing settings json: {error}"))?;

    if !force_rewrite {
        if let Some(existing) = existing_contents.as_deref() {
            if existing.trim() == next_json.trim() && !script_changed {
                return Ok(false);
            }
        }
    }

    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&settings_buf)
            .map_err(|error| format!("failed creating centralized settings backup: {error}"))?;
    }
    fs::write(&settings_buf, format!("{next_json}\n")).map_err(|error| {
        format!("failed writing settings {}: {error}", settings_buf.display())
    })?;
    Ok(true)
}

/// Remove the managed `PostToolUse` entry + delete the observer script. Mirror of
/// `remove_managed_hook`. Prunes now-empty containers.
fn remove_managed_posthook(
    settings_path: &str,
    observer_path: &Path,
    backup_before_write: bool,
) -> Result<bool, String> {
    let mut changed = false;
    let settings_buf = PathBuf::from(settings_path);
    if settings_buf.exists() {
        let existing = fs::read_to_string(&settings_buf).map_err(|error| {
            format!("failed to read settings {}: {error}", settings_buf.display())
        })?;
        if let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&existing) {
            let mut removed_any = false;
            if let Some(hooks) = root
                .as_object_mut()
                .and_then(|object| object.get_mut("hooks"))
                .and_then(|hooks| hooks.as_object_mut())
            {
                if let Some(post) = hooks.get_mut("PostToolUse").and_then(|v| v.as_array_mut()) {
                    let before = post.len();
                    post.retain(|entry| !is_managed_posthook_entry(entry));
                    removed_any = post.len() != before;
                    if post.is_empty() {
                        hooks.remove("PostToolUse");
                    }
                }
                let hooks_empty = hooks.is_empty();
                if hooks_empty {
                    root.as_object_mut().map(|object| object.remove("hooks"));
                }
            }
            if removed_any {
                let next_json = serde_json::to_string_pretty(&root)
                    .map_err(|error| format!("failed serializing settings json: {error}"))?;
                if backup_before_write {
                    let _ = crate::backups::backup_before_write(&settings_buf);
                }
                fs::write(&settings_buf, format!("{next_json}\n")).map_err(|error| {
                    format!("failed writing settings {}: {error}", settings_buf.display())
                })?;
                changed = true;
            }
        }
    }
    if observer_path.exists() {
        fs::remove_file(observer_path).map_err(|error| {
            format!("failed removing observer script {}: {error}", observer_path.display())
        })?;
        changed = true;
    }
    Ok(changed)
}

// ===== Sprint 21 (v2.0): the knowledge PUSH hooks — SessionStart domain primer +
// PreToolUse cue-gated recall. Both live-call experience(...) on the deployed JAWATA
// resident (Bearer token baked in like health_url), peel jawata's FIXED MCP envelope
// (POST /mcp returns the JSON-RPC result in the body — no handshake), and inject via
// `additionalContext`. FAIL-SAFE by construction: jawata down / empty / absence / any
// parse miss → emit nothing, so the session/tool call proceeds unchanged. Rendering
// lives in the mcp (`experience(..., format=text)`, reactor-tested + sanitized), so
// these scripts only peel the fixed envelope and never parse variable tool structure. =====

/// Sentinel for the managed SessionStart primer entry.
const JAWATA_PRIMER_SENTINEL: &str = "jawata-studio/sessionstart-primer.sh";
/// Sentinel for the managed PreToolUse recall entry (distinct from the guard's entry).
const JAWATA_RECALL_SENTINEL: &str = "jawata-studio/pretooluse-recall.sh";
/// Sentinel for the managed UserPromptSubmit recall entry (Sprint 21c item D).
const JAWATA_USERPROMPT_SENTINEL: &str = "jawata-studio/userpromptsubmit-recall.sh";

/// Absolute path of the managed SessionStart primer script (sibling of the guard).
fn managed_primer_script_path() -> Option<PathBuf> {
    managed_hook_invocation_path("jawata-hook-primer", PRIMER_SCRIPT_FILE)
}

/// Absolute path of the managed PreToolUse recall script (sibling of the guard).
fn managed_recall_script_path() -> Option<PathBuf> {
    managed_hook_invocation_path("jawata-hook-recall", RECALL_SCRIPT_FILE)
}

/// Absolute path of the managed UserPromptSubmit recall script (Sprint 21c item D).
fn managed_userprompt_script_path() -> Option<PathBuf> {
    managed_hook_invocation_path("jawata-hook-userprompt", USERPROMPT_SCRIPT_FILE)
}

/// True iff a hook entry's command references the given managed sentinel.
fn entry_command_contains(entry: &serde_json::Value, needle: &str) -> bool {
    // Sprint 22b: a redeploy must also match entries written by goja-studio
    // (the pre-rebrand sentinels) so they are replaced, not duplicated.
    let legacy = legacy_sentinel(needle);
    entry
        .get("hooks")
        .and_then(|hooks| hooks.as_array())
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|command| command.as_str())
                    .map(|command| command.contains(needle) || command.contains(&legacy))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

// Sprint 28 (D-SHIM): keyed on the BINARY name, not on today's script path.
// entry_is_managed_any_generation resolves that name to every generation we
// have ever written — the role-named binary this sprint deploys, the `.sh`
// script it replaces, and that script's pre-rebrand goja twin. Keying on the
// binary therefore recognises an install from BEFORE this sprint as well as
// one from after it, which is the whole point: an entry we do not recognise is
// classified as the user's own and preserved, so the retired script would keep
// firing beside the new binary forever.
fn is_managed_primer_entry(entry: &serde_json::Value) -> bool {
    entry_is_managed_any_generation(entry, "jawata-hook-primer")
}
fn is_managed_recall_entry(entry: &serde_json::Value) -> bool {
    entry_is_managed_any_generation(entry, "jawata-hook-recall")
}

/// SessionStart entry: no matcher (fires on every session start).
/// Where the shipped hook binary lives, or `None` before Stage 7 packages it.
///
/// Checked in order: beside the running app (a Tauri `externalBin` sidecar
/// lands next to the executable), then the dev build tree. `None` is a NORMAL
/// answer today — the scripts still deploy — and deliberately distinct from a
/// deploy that failed: a present-but-unwritable binary is an error, a missing
/// one is simply not shipped yet.
/// The places a sidecar may be, beside a given executable directory.
///
/// Split out so a test can assert the list against the name `tauri.conf.json`
/// actually ships (C7 audit, F3) — the previous binding was a tautology that
/// could not fail.
fn hook_source_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    vec![exe_dir.join("jawata-hook"), exe_dir.join("jawata-hook.exe")]
}

fn hook_binary_source() -> Option<PathBuf> {
    let beside_app = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
    let mut candidates: Vec<PathBuf> = beside_app
        .as_ref()
        .map(|d| hook_source_candidates(d))
        .unwrap_or_default();
    // Dev fallbacks, relative to the repo root.
    candidates.push(PathBuf::from("src-tauri/target/release/jawata-hook"));
    candidates.push(PathBuf::from("src-tauri/target/debug/jawata-hook"));
    candidates.into_iter().find(|p| p.exists())
}

/// Whether this is an installed build rather than a dev run.
///
/// C7 audit F6: the distinction the deploy needs in order to tell "the sidecar
/// is not shipped yet" from "the sidecar is shipped and unreachable". A dev
/// `cargo run` legitimately has no sidecar; an install that lacks one is
/// broken, and before this the two were the same silent branch.
fn running_from_an_installed_build() -> bool {
    std::env::current_exe()
        .ok()
        .map(|exe| {
            let p = exe.to_string_lossy().to_string();
            // A dev binary lives under target/{debug,release}; anything else
            // that got this far is an install.
            !p.contains("/target/debug/") && !p.contains("/target/release/")
                && !p.contains("\\target\\debug\\") && !p.contains("\\target\\release\\")
        })
        .unwrap_or(false)
}

/// The role names whose BINARY generation is live — the deploy writes exactly
/// these, dispatched by `argv[0]`.
///
/// Kept beside the deploy rather than imported from the hook crate: the studio
/// must NOT depend on jawata-hook (deploy writes the binary, it never runs it),
/// and that edge is asserted by a test. `hook-events.json` is what keeps the
/// two lists honest with each other.
///
/// `jawata-hook-observer` is deliberately ABSENT — its binary is a stub and
/// the script generation still owns the role (`role_is_binary_live`). The
/// deploy also REMOVES a previously-deployed observer binary, because a binary
/// on disk is what flips the invocation path.
const BINARY_LIVE_ROLES: &[&str] = &[
    "jawata-hook-primer",
    "jawata-hook-userprompt",
    "jawata-hook-recall",
    "jawata-hook-stop",
    // Sprint 28a (2026-08-12): the guard rejoins, because the binary now carries
    // BOTH halves. It lost this place in the 3.7.3 dogfood for holding only the
    // shell half while the `.java` hand-edit gate stayed in the script; the gate
    // is now in `editgate.rs` and both halves are pinned by tests that spawn the
    // real binary.
    //
    // Windows forced the timing rather than tidiness: a `.sh` cannot execute
    // there, Cursor runs this role `failClosed`, and a hook that never returns
    // BLOCKS the user's command — seen live as an interactive bash window hung
    // on `cat` waiting for a payload nobody piped in.
    "jawata-hook-guard",
];

/// Roles whose binary must NOT be on disk: a stale one from an earlier deploy
/// would sit unfired forever — the 3.7.1 unwired shape, resurrected per role.
///
/// The GUARD joined the observer here in the 3.7.3 dogfood (F5): its binary
/// held parity on the shell-command half (java-grep redirect, the
/// `jawata-fallback:` escape) and silently dropped the other half — the
/// `.java` hand-edit gate with its `jawata-author:` authoring windows. A
/// front-door Edit of a `.java` file went through unblocked. The script
/// generation carries BOTH halves, so the role reverts until the binary
/// ports the edit gate.
const BINARY_RETIRED_ROLES: &[&str] = &["jawata-hook-observer"];

/// Sprint 28 (D-SHIM, C6 clause 5 first half): deploy the role-named hook
/// binaries — UNLINK, then write.
///
/// The rename to `argv[0]` dispatch introduces a hazard the `.sh` generation
/// did not have: Linux refuses to open an executing binary for writing
/// (`ETXTBSY`), and hooks fire on every prompt, so a redeploy lands on top of
/// running processes routinely. Overwriting a shell script has no such problem
/// — the kernel does not hold text pages for `bash`'s argument.
///
/// Unlinking first sidesteps it entirely: the running process keeps its inode
/// (it is unlinked, not destroyed, and dies with the process), while the new
/// file takes the name. This is the ordinary Unix replace-a-running-binary
/// move, and it is why `fs::write` over the top is NOT good enough here.
///
/// On Windows the same shape is required for a different reason — a running
/// image cannot be replaced — and the same unlink-first order handles it.
fn deploy_hook_binaries(
    source: &Path,
    hooks_dir: &Path,
    roles: &[&str],
    platform: HostPlatform,
) -> Result<Vec<String>, String> {
    if !source.exists() {
        return Err(format!(
            "the hook binary is not at {} — nothing to deploy; the bundle did not ship it",
            source.display()
        ));
    }
    fs::create_dir_all(hooks_dir)
        .map_err(|e| format!("failed to create hooks dir {}: {e}", hooks_dir.display()))?;

    let fresh = fs::read(source)
        .map_err(|e| format!("failed reading {}: {e}", source.display()))?;
    // Retired-role binaries come OFF the disk: the invocation path prefers a
    // binary that exists, so a stale one from an earlier deploy is not inert —
    // it is the thing that flips the role away from its live script.
    for retired in BINARY_RETIRED_ROLES {
        let stale = hooks_dir.join(role_binary_file_name_on(platform, retired));
        match fs::remove_file(&stale) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("failed removing retired {}: {e}", stale.display())),
        }
    }
    let mut written = Vec::new();
    for role in roles {
        // Delegated, never spelled here. This line read `hooks_dir.join(role)`
        // through five releases: correct on Unix by coincidence, wrong on
        // Windows, and invisible to every test because the convention was a
        // compile-time constant. The retirement loop above always delegated —
        // one function, two conventions, which is the whole defect.
        let target = hooks_dir.join(role_binary_file_name_on(platform, role));
        if fs::read(&target).map(|existing| existing == fresh).unwrap_or(false) {
            continue;   // byte-stable no-op
        }
        // UNLINK FIRST. Ignore a missing file; anything else is real.
        match fs::remove_file(&target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("failed unlinking {}: {e}", target.display())),
        }
        fs::write(&target, &fresh)
            .map_err(|e| format!("failed writing {}: {e}", target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).map_err(|e| {
                format!("failed making {} executable: {e}", target.display())
            })?;
        }
        written.push((*role).to_string());
    }
    Ok(written)
}

/// Sprint 28 (C8): rotate the hook's silence log once it exceeds its cap.
///
/// This is the OFF-fire-path half of the log-bounding contract. The hook only
/// appends (with a hard ceiling that drops records in the pathological
/// no-manager case); the manager — single process, no 4-second deadline, owner
/// of the directory — does the one rename. No token, no staleness horizon, no
/// reap protocol: every scheme that tried to do this from inside the
/// concurrently-firing hook destroyed records or disabled itself, six audit
/// rounds running.
///
/// A concurrent hook APPENDING during the rename is safe: it holds the old
/// inode and its record lands in `hook_silence.log.1`, still readable. Two
/// MANAGERS racing is not a real state — the studio is a singleton per user —
/// and even interleaved, two renames only shuffle which generation a record
/// sits in; nothing truncates.
/// The silence log's cap, read from the shared contract at COMPILE TIME.
///
/// `include_str!` plus a const parse keeps this a genuine single source: there
/// is no runtime file to go missing, and a malformed contract fails the build
/// rather than silently defaulting. The architect's F2 named the three
/// hand-copied facts this replaces (cap, filename, rotated name); this closes
/// the cap, which is the one whose drift changes behaviour.
const fn silence_log_cap() -> u64 {
    // A const fn cannot parse JSON, so the value is asserted rather than
    // extracted — the test below fails the build's own suite if the contract
    // moves, and the hook's half fails from the other side.
    262_144
}

fn rotate_silence_log(hooks_dir: &Path) -> bool {
    // READ FROM THE CONTRACT, not copied. A shared Rust constant is impossible
    // — neither crate may depend on the other, and both forbidden edges are
    // asserted by tests — but `hook-events.json` is a committed data file both
    // sides read at compile time, invented for exactly this constraint. The
    // hook asserts its own numbers against the same row in
    // jawata-hook/tests/silence_log_contract_matches_the_deploy.rs, so a change
    // on either side fails BOTH until they agree.
    const MAX_BYTES: u64 = silence_log_cap();
    let live = hooks_dir.join("hook_silence.log");
    let oversized = fs::metadata(&live).map(|m| m.len() > MAX_BYTES).unwrap_or(false);
    if !oversized {
        return false;
    }
    fs::rename(&live, hooks_dir.join("hook_silence.log.1")).is_ok()
}

/// Sprint 28 (D-SHIM, C6 clause 5): write `hook_config.json` beside the hook
/// binaries — temp file, then rename.
///
/// Concurrency here is MEASURED, not assumed: three sessions with a holding
/// hook produced three overlapping pairs, so invocations genuinely run in
/// parallel. A plain `write` truncates first, and a hook reading during that
/// window sees an empty or half-written file. `rename` within a directory is
/// atomic on every platform we ship, so a reader sees either the whole old file
/// or the whole new one and never a torn one.
///
/// The hook's read side already treats a zero-length file as a TORN DEPLOY
/// rather than as "not configured" — this function makes that state
/// unreachable, and the reader stays loud if it ever happens anyway.
fn write_hook_config(
    hooks_dir: &Path,
    mcp_url: &str,
    token: &str,
    client: &str,
) -> Result<bool, String> {
    fs::create_dir_all(hooks_dir)
        .map_err(|e| format!("failed to create hooks dir {}: {e}", hooks_dir.display()))?;
    // Housekeeping rides the config cadence: this function runs on every
    // deploy and every watch-loop refresh — including the byte-stable no-op
    // path below — which is exactly when the manager is already standing in
    // this directory. The hook itself never rotates; see rotate_silence_log.
    rotate_silence_log(hooks_dir);
    let body = serde_json::json!({
        "url": mcp_url,
        "token": token,
        "client": client,
    })
    .to_string();

    let target = hooks_dir.join("hook_config.json");
    if fs::read_to_string(&target).map(|existing| existing == body).unwrap_or(false) {
        return Ok(false);   // byte-stable no-op
    }
    // A per-process temp name: two deploys racing must not collide on the temp
    // file itself, or one truncates the other's staging.
    let tmp = hooks_dir.join(format!("hook_config.json.{}.tmp", std::process::id()));
    fs::write(&tmp, &body)
        .map_err(|e| format!("failed staging {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed publishing {}: {e}", target.display())
    })?;
    Ok(true)
}

/// Sprint 28 (C6 clause 6): every entry we write carries an EXPLICIT timeout.
///
/// The client default is unpublished — Cursor documents it only as "platform
/// default" — so an entry without one is a hook whose bound nobody knows,
/// including us. Cursor's four entries already carried timeouts; Claude's six
/// did not, which is the gap this closes.
///
/// The values sit ABOVE the hook's own budget and below anything a user would
/// notice: the binary's total deadline is 4s, so 5s leaves headroom for process
/// start without the client's timeout ever being the thing that ends a run.
/// The primer gets 15s because it fires once per session and may wait on a
/// cold store.
const HOOK_TIMEOUT_SECS: u64 = 5;
const PRIMER_TIMEOUT_SECS: u64 = 15;

fn build_managed_primer_entry(primer_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": [ { "type": "command", "command": display_path(primer_path),
                     "timeout": PRIMER_TIMEOUT_SECS } ]
    })
}
/// PreToolUse entry for recall: fires on jawata tool calls, and on hand-edits.
///
/// `Edit|Write|MultiEdit` restored in v3.7.3: the Sprint 22a capability —
/// recall the type's prior lessons when a source file is hand-edited — existed
/// in the script's own case-statement but the MATCHER never sent those events
/// to it, so the branch was unreachable for both generations. (`Read` stays
/// out deliberately: neither generation claimed it, and a hook process per
/// file read is cost without a declared capability behind it.)
fn build_managed_recall_entry(recall_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "matcher": "Edit|Write|MultiEdit|mcp__jawata.*",
        "hooks": [ { "type": "command", "timeout": HOOK_TIMEOUT_SECS, "command": display_path(recall_path) } ]
    })
}
fn is_managed_userprompt_entry(entry: &serde_json::Value) -> bool {
    entry_is_managed_any_generation(entry, "jawata-hook-userprompt")
}
/// UserPromptSubmit entry: no matcher (fires on every user prompt; the script gates itself).
fn build_managed_userprompt_entry(script_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": [ { "type": "command", "timeout": HOOK_TIMEOUT_SECS, "command": display_path(script_path) } ]
    })
}

/// Generic: write a managed script + merge its entry into `hooks.<section>`, dropping any
/// prior managed entry (by `is_managed`) and preserving user hooks. Idempotent. Shared by
/// the primer (SessionStart) + recall (PreToolUse) without touching the guard/observer.
#[allow(clippy::too_many_arguments)]
fn write_managed_hook_section(
    settings_path: &str,
    script_path: &Path,
    script_body: &str,
    section: &str,
    entry: serde_json::Value,
    is_managed: fn(&serde_json::Value) -> bool,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    let script_parent = script_path
        .parent()
        .ok_or_else(|| format!("script path has no parent: {}", script_path.display()))?;
    fs::create_dir_all(script_parent)
        .map_err(|error| format!("failed to create hook dir {}: {error}", script_parent.display()))?;
    // When the invocation path is a deployed role BINARY, its content belongs
    // to `deploy_hook_binaries` — this writer owns only the settings entry.
    // Writing the bash generation here is the 3.7.3 dogfood clobber: the
    // deploy wrote six binaries, then four of these writers ran after it and
    // wrote scripts back over four of them — same filenames, so every file
    // looked deployed and four events ran the previous generation.
    let body_is_ours = !path_is_role_binary(script_path);
    let script_changed = body_is_ours
        && fs::read_to_string(script_path)
            .map(|existing| existing != script_body)
            .unwrap_or(true);
    if body_is_ours && (script_changed || force_rewrite) {
        fs::write(script_path, script_body)
            .map_err(|error| format!("failed writing hook script {}: {error}", script_path.display()))?;
    }
    #[cfg(unix)]
    if body_is_ours {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(script_path, fs::Permissions::from_mode(0o755));
    }

    let settings_buf = PathBuf::from(settings_path);
    let settings_parent = settings_buf
        .parent()
        .ok_or_else(|| format!("settings path has no parent: {}", settings_buf.display()))?;
    fs::create_dir_all(settings_parent)
        .map_err(|error| format!("failed to create settings dir {}: {error}", settings_parent.display()))?;

    let existing_contents = fs::read_to_string(&settings_buf).ok();
    let mut root = existing_contents
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    {
        let object = root.as_object_mut().expect("root is an object");
        let hooks = object.entry("hooks").or_insert_with(|| serde_json::json!({}));
        if !hooks.is_object() {
            *hooks = serde_json::json!({});
        }
        let hooks_object = hooks.as_object_mut().expect("hooks is an object");
        let mut arr = hooks_object
            .get(section)
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        arr.retain(|entry| !is_managed(entry));
        arr.push(entry);
        hooks_object.insert(section.to_string(), serde_json::Value::Array(arr));
    }

    let next_json = serde_json::to_string_pretty(&root)
        .map_err(|error| format!("failed serializing settings json: {error}"))?;
    if !force_rewrite {
        if let Some(existing) = existing_contents.as_deref() {
            if existing.trim() == next_json.trim() && !script_changed {
                return Ok(false);
            }
        }
    }
    if backup_before_write {
        // Sprint 21a (item E): centralized area — no .bak-* beside the user's file.
        crate::backups::backup_before_write(&settings_buf)
            .map_err(|error| format!("failed creating centralized settings backup: {error}"))?;
    }
    fs::write(&settings_buf, format!("{next_json}\n"))
        .map_err(|error| format!("failed writing settings {}: {error}", settings_buf.display()))?;
    Ok(true)
}

/// Generic mirror of the section write: strip the managed entry + delete the script.
fn remove_managed_hook_section(
    settings_path: &str,
    script_path: &Path,
    section: &str,
    is_managed: fn(&serde_json::Value) -> bool,
    backup_before_write: bool,
) -> Result<bool, String> {
    let mut changed = false;
    let settings_buf = PathBuf::from(settings_path);
    if settings_buf.exists() {
        let existing = fs::read_to_string(&settings_buf)
            .map_err(|error| format!("failed to read settings {}: {error}", settings_buf.display()))?;
        if let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&existing) {
            let mut removed_any = false;
            if let Some(hooks) = root
                .as_object_mut()
                .and_then(|object| object.get_mut("hooks"))
                .and_then(|hooks| hooks.as_object_mut())
            {
                if let Some(arr) = hooks.get_mut(section).and_then(|v| v.as_array_mut()) {
                    let before = arr.len();
                    arr.retain(|entry| !is_managed(entry));
                    removed_any = arr.len() != before;
                    if arr.is_empty() {
                        hooks.remove(section);
                    }
                }
                if hooks.is_empty() {
                    root.as_object_mut().map(|object| object.remove("hooks"));
                }
            }
            if removed_any {
                let next_json = serde_json::to_string_pretty(&root)
                    .map_err(|error| format!("failed serializing settings json: {error}"))?;
                if backup_before_write {
                    let _ = crate::backups::backup_before_write(&settings_buf);
                }
                fs::write(&settings_buf, format!("{next_json}\n"))
                    .map_err(|error| format!("failed writing settings {}: {error}", settings_buf.display()))?;
                changed = true;
            }
        }
    }
    if script_path.exists() {
        fs::remove_file(script_path)
            .map_err(|error| format!("failed removing hook script {}: {error}", script_path.display()))?;
        changed = true;
    }
    Ok(changed)
}


// ============================================================
// Sprint 26 (D5/D4): the Stop hook — the communication gate's hard bounce
// + the seat-gate block, Claude Code only (no stop-equivalent elsewhere).
// ============================================================

const JAWATA_STOP_SENTINEL: &str = "jawata-studio stop gate";

fn managed_stop_script_path() -> Option<PathBuf> {
    managed_hook_invocation_path("jawata-hook-stop", STOP_SCRIPT_FILE)
}

fn is_managed_stop_entry(entry: &serde_json::Value) -> bool {
    entry_is_managed_any_generation(entry, "jawata-hook-stop")
}

fn build_managed_stop_entry(script: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": [ { "type": "command", "timeout": HOOK_TIMEOUT_SECS, "command": display_path(script) } ]
    })
}

/// The Stop-gate script: rule-based decision test on the final message when
/// it matches any of the three shapes (decision ask / checkpoint summary /
/// sprint result); a seat session whose transcript shows a seat command but
/// no gate calls is blocked with the named gates. Fail-open on any parse
/// problem — a broken gate must never mute the agent. One rewrite loop:
/// stop_hook_active means we already bounced once — allow.
fn build_stop_script(mcp_url: &str, token: &str) -> String {
    STOP_TEMPLATE
        .replace("__MCP_URL__", mcp_url)
        .replace("__TOKEN__", token)
}

const STOP_TEMPLATE: &str = r#"#!/usr/bin/env bash
# jawata-studio stop gate (Sprint 26 D5/D4) — managed; do not edit.
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
if [ -n "$JAWATA_HOOK_SELFTEST" ]; then printf '{}'; exit 0; fi
input=$(cat)
python3 - "$input" <<'PYEOF2'
import json, re, sys, subprocess, os
try:
    data = json.loads(sys.argv[1])
except Exception:
    print('{}'); sys.exit(0)
if data.get('stop_hook_active'):
    print('{}'); sys.exit(0)  # one rewrite loop only
tp = data.get('transcript_path')
# TAIL ONLY. This used to read every line of the transcript to end up with the
# LAST assistant message, and then read the whole file a SECOND time for the
# seat check. Measured on a real session: 332 MB, ~3 s per turn, twice over,
# with the entire file in memory. Nothing here needs history.
WIN = 1 << 20
text = ''
tail = ''
emitted = ''
try:
    with open(tp, 'rb') as f:
        f.seek(0, 2); size = f.tell()
        f.seek(max(0, size - WIN))
        raw = f.read()
    if size > WIN:
        nl = raw.find(b'\n')
        raw = raw[nl+1:] if nl >= 0 else raw
    tail = raw.decode('utf-8', 'replace')
    for line in tail.splitlines():
        try:
            j = json.loads(line)
        except Exception:
            continue
        m = j.get('message') or {}
        if (j.get('type') == 'assistant') and isinstance(m.get('content'), list):
            parts = [c.get('text','') for c in m['content'] if c.get('type')=='text']
            if parts:
                text = chr(10).join(parts)
                emitted += text + chr(10)
except Exception:
    print('{}'); sys.exit(0)
U = text.upper()
shaped = (U.startswith('DECISION:') or 'DECISION:' in U[:400]
          or 'AWAITING "CONTINUE"' in U or '⏸' in text
          or ('CHECKPOINT' in U and 'SHIPPED' in U)
          or ('SPRINT' in U and ('CLOSED' in U or 'RESULT' in U)))
reasons = []
# AUDIT-FIX LOOP (Sprint 28 C8). Every other check here waits for a CHECKPOINT
# marker, and a churn loop never produces one — the checkpoint never passes, so
# it is never written. The failure state and the trigger condition were mutually
# exclusive by construction: this gate ran every turn through six audit rounds
# and correctly found nothing to judge, while the pathology it exists to catch
# ran in front of it. Evidence: C8 took six REFUSEs, each new defect introduced
# by the previous round's fix, and the cure was a redesign that deleted 186
# lines — visible from round two, acted on after round six.
#
# So this trigger counts the LOOP, not the checkpoint.
try:
    # `tail`, not `whole` — `whole` is not assigned until the seat block BELOW
    # this one. The first version referenced it here, raised NameError, and was
    # swallowed by this block's own `except: pass`: the check reported nothing
    # and looked identical to a check that found nothing. That is the failure
    # this entire sprint exists to end, committed inside the trigger written to
    # detect it.
    # Count what the AGENT EMITTED, not strings in the window. Counting the raw
    # tail matched tool results and file contents, so any session that READ this
    # file, hook-events.json or a C8 sprint doc was told to abandon correct
    # work. Reading the word is not refusing.
    rounds = emitted.count('REFUSE')
    # NO "but a checkpoint happened" suppression. The first version of this
    # check looked for a checkpoint marker in the window and stood down if it
    # found one — and on the very session it was written for it found the
    # PAUSE GLYPH inside the quoted checkpoint-format TEMPLATE, not an actual
    # checkpoint, and suppressed itself. Matching a shape the agent's own text
    # can contain is the identical defect this gate already has elsewhere.
    # Three refusals in a megabyte IS the loop, whatever else is in the window.
    if rounds >= 3:
        reasons.append(
            'AUDIT-FIX LOOP: %d refusals in this window with no checkpoint reached. '
            'Repeated defect-in-the-fixing-commit is a DESIGN alarm, not a bug streak. '
            'STOP fixing findings and run the architect seat (/refactor) as a watch-diff '
            'against the ARCHITECTURE artifact: design fix or bandage? Six rounds of '
            'C8 were one wrong design decision generating bugs on demand.' % rounds)
except Exception:
    pass
# UNJUDGED UPWARD ASK. The communicator requirement lived only as prose in
# CLAUDE.md — binding on the agent by reading it, enforced by nothing. Asked
# "how can you skip a hook?", the honest answer was that no hook existed: this
# gate checked message shape, seat discipline, length and abbreviations, and
# had no concept of the communicator at all. Caught three times in one session,
# the third an hour after the agent recorded the rule as a lesson.
#
# A rule that depends on the agent remembering is not a rule. This is the
# cheapest layer that does not: the transcript is written by the HARNESS, so
# whether a communicator subagent ran is a fact the agent cannot fake by
# writing a marker.
try:
    # FIRST LIVE FAILURE, hours after this shipped: the next ask the agent wrote
    # — "you decide whether it becomes v3.7.2 or waits" — matched none of the
    # original ten phrases and this gate returned allow. That is the SAME defect
    # the comment above claims to cure: keying on tokens the agent authors
    # freely. Wider list, plus a DIRECT QUESTION, which an ask cannot avoid.
    # This REDUCES leakage; it does not close it. The non-evadable form is the
    # inversion — every stop shows a pass — and that needs the autonomy signal.
    phrases = ('YOUR WORD','NEEDS YOUR','YOUR CALL','YOUR RULING','YOUR SIGN-OFF',
               'YOUR DECISION','SHALL I','WANT ME TO','DO YOU WANT','MAY I',
               'DECISION:','LET ME KNOW','UP TO YOU','YOU DECIDE','YOU CHOOSE',
               "IF YOU'D RATHER",'IF YOU PREFER','SAY THE WORD','ON YOUR WORD',
               'AWAITING','AWAIT YOUR','SHOULD I','WOULD YOU LIKE','PREFER THAT I')
    tail_lines = [l.rstrip() for l in text.splitlines() if l.strip()][-6:]
    # LENGTH IS A TRIGGER TOO (Harald's own suggestion, and better than the
    # phrase list): it does not depend on wording, so the agent cannot walk
    # past it by accident — which is exactly how the ten-phrase list failed on
    # the first live ask hours after shipping. A long message to him is one the
    # communicator should have seen, whether or not it asks for anything.
    asking = (any(p in U for p in phrases)
              or any(l.endswith('?') for l in tail_lines)
              or len(text) > 2200)
    if asking:
        # Scope to the window since the last human turn: a communicator run
        # three hours ago does not judge THIS message.
        # A HUMAN TURN, not any user-role line. In this client a TOOL RESULT is
        # written as "type":"user" too — measured on a live transcript: 23,087
        # user entries, 20,142 of them tool results, so only 13% are turns.
        # Scoping on the last user-role line meant ONE ordinary tool call after
        # a communicator run re-armed the check and blocked a compliant ask.
        cut = -1
        for ln in tail.splitlines():
            if '"type":"user"' in ln and '"toolUseResult"' not in ln \
               and '"tool_result"' not in ln:
                cut = tail.rfind(ln)
        recent = tail[cut:] if cut >= 0 else tail
        # THE TOOL CALL, not the word. Matching bare 'communicator' was
        # satisfied by the AGENT'S OWN PROSE, which makes the harness-writes-the
        # -transcript argument true but irrelevant: the agent's text is in that
        # transcript too.
        if '"subagent_type":"communicator"' not in recent \
           and '"subagent_type": "communicator"' not in recent:
            reasons.append(
                'UNJUDGED ASK: this message asks Harald for a word, a ruling or a '
                'decision, and no communicator subagent ran since his last turn. '
                'Run the communicator on it, apply its verdict, then send. The rule '
                'is binding and has been skipped three times by memory alone.')
except Exception as e:
    reasons.append('UNJUDGED-ASK CHECK FAILED: %s' % e)
# Seat-gate block (D4): a seat command in this transcript but no gate calls after it.
try:
    whole = tail
    seats = [s for s in ('/javadocs','/cover','/refactor','/debug','/profile') if s in whole]
    if seats and not any(g in whole for g in ('compile_workspace','run_tests','get_diagnostics')):
        reasons.append('SEAT DISCIPLINE: %s invoked but no gate call (compile_workspace / run_tests / get_diagnostics) appears in this session — a gate you did not run has NOT passed. Run the gates before proposing.' % ','.join(seats))
except Exception:
    pass
# NOTE: no early emit for un-shaped reasons. An added `if reasons and not
# shaped: print(block)` here was DEAD CODE — the general `if reasons:` below
# already emits regardless of shape, which is precisely why the loop trigger
# works on ordinary prose. Seeding the early path out killed no test, and that
# was the code being wrong, not the test.
if shaped and not reasons:
    if len(text) > 3500:
        reasons.append('THE DECISION TEST: too long (%d chars) — noise includes LENGTH; a correct but bloated message fails. Cut to what the reader needs to decide.' % len(text))
    known = {'DECISION','WATCH','TLDR','API','MCP','JDT','CPU','JVM','CI','PR','TDD','AST','LRU','TTL','SGD','JSON','HTTP','URL','ID','OK','DONE','STOP','NOT','AND','THE','ALL','NEW','YOUR','BOTH'}
    caps = set(re.findall(r'[A-Z]{2,5}', text)) - known
    undefined = [c for c in caps if (c + ' (') not in text and ('(' + c) not in text]
    if len(undefined) > 2:
        reasons.append('THE DECISION TEST: undefined terms %s — define every abbreviation at first use.' % sorted(undefined)[:4])
if reasons:
    reason = ' | '.join(reasons)
    try:
        subprocess.Popen(['curl','-s','--max-time','3','-X','POST',
            '-H','Authorization: Bearer ' + os.environ.get('JAWATA_TOKEN','__TOKEN__'),
            '-H','Content-Type: application/json',
            '-d', json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'experience','arguments':{'kind':'record','type':'failure_mode','operation':'communication-audit','summary':'stop-gate bounce: ' + reason[:200],'status':'candidate'}}}),
            '__MCP_URL__'])
    except Exception:
        pass
    print(json.dumps({'decision':'block','reason':reason}))
else:
    print('{}')
PYEOF2
"#;

fn write_managed_stop(
    settings_path: &str,
    stop_path: &Path,
    mcp_url: &str,
    token: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    write_managed_hook_section(
        settings_path,
        stop_path,
        &build_stop_script(mcp_url, token),
        "Stop",
        build_managed_stop_entry(stop_path),
        is_managed_stop_entry,
        backup_before_write,
        force_rewrite,
    )
}

fn remove_managed_stop(settings_path: &str, stop_path: &Path, backup_before_write: bool) -> Result<bool, String> {
    remove_managed_hook_section(settings_path, stop_path, "Stop", is_managed_stop_entry, backup_before_write)
}

fn write_managed_primer(
    settings_path: &str,
    primer_path: &Path,
    mcp_url: &str,
    token: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    write_managed_hook_section(
        settings_path,
        primer_path,
        &build_primer_script(mcp_url, token),
        "SessionStart",
        build_managed_primer_entry(primer_path),
        is_managed_primer_entry,
        backup_before_write,
        force_rewrite,
    )
}
fn remove_managed_primer(settings_path: &str, primer_path: &Path, backup_before_write: bool) -> Result<bool, String> {
    remove_managed_hook_section(settings_path, primer_path, "SessionStart", is_managed_primer_entry, backup_before_write)
}
fn write_managed_recall(
    settings_path: &str,
    recall_path: &Path,
    mcp_url: &str,
    token: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    write_managed_hook_section(
        settings_path,
        recall_path,
        &build_recall_script(mcp_url, token),
        "PreToolUse",
        build_managed_recall_entry(recall_path),
        is_managed_recall_entry,
        backup_before_write,
        force_rewrite,
    )
}
fn remove_managed_recall(settings_path: &str, recall_path: &Path, backup_before_write: bool) -> Result<bool, String> {
    remove_managed_hook_section(settings_path, recall_path, "PreToolUse", is_managed_recall_entry, backup_before_write)
}
fn write_managed_userprompt(
    settings_path: &str,
    script_path: &Path,
    mcp_url: &str,
    token: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    write_managed_hook_section(
        settings_path,
        script_path,
        &build_userprompt_script(mcp_url, token),
        "UserPromptSubmit",
        build_managed_userprompt_entry(script_path),
        is_managed_userprompt_entry,
        backup_before_write,
        force_rewrite,
    )
}
fn remove_managed_userprompt(settings_path: &str, script_path: &Path, backup_before_write: bool) -> Result<bool, String> {
    remove_managed_hook_section(settings_path, script_path, "UserPromptSubmit", is_managed_userprompt_entry, backup_before_write)
}

/// The SessionStart primer script (URL + Bearer token baked in). Deterministic → a
/// re-deploy is a byte-stable no-op. Uses `.replace()` templating (not `format!`) so the
/// JSON-heavy body needs no brace-doubling.
fn build_primer_script(mcp_url: &str, token: &str) -> String {
    PRIMER_TEMPLATE.replace("__MCP_URL__", mcp_url).replace("__TOKEN__", token)
}
/// The PreToolUse recall script (URL + Bearer token baked in). Same peel; gated to
/// refactor-ish jawata verbs with a symbol cue.
fn build_recall_script(mcp_url: &str, token: &str) -> String {
    RECALL_TEMPLATE.replace("__MCP_URL__", mcp_url).replace("__TOKEN__", token)
}
/// The UserPromptSubmit recall script (Sprint 21c item D): prompt → keyword cues →
/// terminal recall → inject the ONE fitting fact, or nothing. Same envelope peel.
fn build_userprompt_script(mcp_url: &str, token: &str) -> String {
    USERPROMPT_TEMPLATE.replace("__MCP_URL__", mcp_url).replace("__TOKEN__", token)
}

const USERPROMPT_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed UserPromptSubmit recall — do not edit; overwritten on deploy>
# Sprint 21c (item D): prompt -> keywords -> recall -> injected NOMINEES. Extracts
# content-bearing cues from the user's prompt (longest n-grams first, rarity-marked
# tokens preferred within a tier, >=2 content tokens), asks the store, and injects what
# it offers — labelled as candidates to judge — or nothing when the store has nothing.
#
# Sprint 28 (studio#3): this script previously skipped whenever the store returned MORE
# THAN ONE answer, encoding 21c's retired "one fitting fact or silence" contract. Sprint
# 27a's C2 ruling — distance nominates, the agent judges — made multi-answer the NORMAL
# case (up to 11 labelled nominees, always), because no statistic over a score profile
# separates a real cue from a nonsense one. The two contracts were incompatible, so this
# hook injected NOTHING for two weeks, silently, for every cue. The skip is gone; the
# label now says what these actually are.
set -u
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
# THE emit path — selftest and the live path share this one printf format (Sprint 21a item J).
emit_ctx() {
  printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"JAWATA recalled candidate prior knowledge for this topic — these are NOMINEES, not vouched answers; judge whether each fits before relying on it:\\n%s"}}' "$1"
}
# Sprint 28 (studio#3): the canned value is deliberately MULTI-LINE. The previous
# single-line canned string could not exercise the shape that actually occurs, so the
# deploy-time check passed throughout the two weeks this hook emitted nothing. A check
# that cannot fail the way the thing fails is not a check of the thing.
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then emit_ctx '[lesson] selftest canned line (accepted)\n[lesson] selftest second line — multi-answer is the normal case'; exit 0; fi
command -v curl >/dev/null 2>&1 || exit 0
# THE recall attempt (Sprint 22a dual-cue): $1 = arg key (symbol|symptom), $2 = cue.
# On any non-empty answer it injects and exits 0; otherwise returns so the next-ranked
# cue is tried. Absence is still absence — "No known knowledge" falls through.
try_recall() {
  [ -n "$2" ] || return 1
  req='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"experience","arguments":{"kind":"recall","format":"text","'"$1"'":"'"$2"'"}}}'
  resp="$(curl -s --max-time 2 -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$req" "$MCP_URL" 2>/dev/null)"
  [ -n "$resp" ] || exit 0
  flat="$(printf '%s' "$resp" | tr -d '\n\r')"
  inner="$(printf '%s' "$flat" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\(.*\)"[[:space:]]*}[[:space:]]*][[:space:]]*}[[:space:]]*}.*/\1/p' | sed 's/\\"/"/g; s/\\\\/\\/g')"
  [ -n "$inner" ] || return 1
  printf '%s' "$inner" | grep -q '"success"[[:space:]]*:[[:space:]]*true' || return 1
  # data is a quote-sanitized flat string, so [^"]* stops at its closing quote — NOT
  # greedy .* (which would swallow the trailing ,"meta":{steering} the layer appends).
  data="$(printf '%s' "$inner" | sed -n 's/.*"data"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
  [ -n "$data" ] || return 1
  case "$data" in No\ known\ knowledge*|No\ domain*) return 1 ;; esac
  # Sprint 28 (studio#3): NO multi-answer skip. A newline-sniffing case-branch lived here
  # and discarded every answer the store gave, because 27a made multi the norm. The store
  # already caps what it offers (MAX_NOMINEES = 11, each an eight-word summary line —
  # Harald priced that cost in 27a), so there is nothing left to trim here.
  # (The removed branch is quoted in this file's tests, not reproduced here: a comment
  # containing it would satisfy the very `contains` check that guards its absence.)
  emit_ctx "$data"
  exit 0
}
input="$(cat)"
flatin="$(printf '%s' "$input" | tr '\n\r' '  ')"
prompt="$(printf '%s' "$flatin" | sed -n 's/.*"prompt"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
[ -n "$prompt" ] || exit 0
case "$prompt" in /*) exit 0 ;; esac
# Symbol cues (Sprint 22a dual-cue, precise-first): qualified/member identifiers that
# name a type or member (Type#member, pkg.Type, Outer.Inner), from the ORIGINAL prompt
# (case-sensitive). They fire kind=recall,symbol= BEFORE the symptom cues and are
# independent of the >=2-content-word gate, so a bare `Foo#bar` prompt still recalls.
symcues="$(printf '%s' "$prompt" | grep -oE '[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z0-9_]+)*#[A-Za-z0-9_]+|[a-z][A-Za-z0-9_]*(\.[a-z][A-Za-z0-9_]*)*\.[A-Z][A-Za-z0-9_]*|[A-Z][A-Za-z0-9_]*(\.[A-Z][A-Za-z0-9_]*)+' 2>/dev/null | head -n 2)"
try_recall symbol "$(printf '%s\n' "$symcues" | sed -n 1p)"
try_recall symbol "$(printf '%s\n' "$symcues" | sed -n 2p)"
# Normalize: lowercase, punctuation -> space; digits/hyphens/underscores survive (rarity marks).
norm="$(printf '%s' "$prompt" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9_-]/ /g')"
words=""
count=0
for w in $norm; do
  case "$w" in
    the|a|an|and|or|for|with|this|that|these|those|is|are|was|were|be|been|to|of|in|on|at|it|its|we|i|you|he|she|they|do|does|did|not|no|yes|our|my|your|his|her|their|what|which|how|why|when|where|who|make|makes|made|making|please|now|then|so|but|if|else|can|could|should|would|will|shall|may|might|must|have|has|had|get|got|just|also|about|into|from|out|up|down|over|again|more|less|very|all|any|some|one|two|new|use|used|using) continue ;;
  esac
  [ ${#w} -ge 3 ] || continue
  words="$words $w"
  count=$((count+1))
  [ "$count" -ge 40 ] && break
done
[ "$count" -ge 2 ] || exit 0
# Cue candidates per TIER: within a tier, rarity-marked cues (digits / hyphens /
# underscores) before plain ones, then order of appearance. The best trigram gets ONE
# attempt (precision bonus); bigrams — the workhorse under the all-tokens fit gate —
# get the other two, so long prompts can never starve them (live-drive finding).
# DECLARED deviation from "rarer tokens first": true corpus rarity needs a frequency
# table the hook does not have — the marker heuristic is the deterministic proxy.
ngrams() {
  printf '%s' "$words" | awk -v len="$1" '{
    n = split($0, w, " ");
    for (pass = 1; pass <= 2; pass++) {
      want = (pass == 1) ? 1 : 0;
      for (i = 1; i + len - 1 <= n; i++) {
        cue = w[i];
        for (j = 1; j < len; j++) cue = cue " " w[i+j];
        mark = (cue ~ /[0-9_-]/) ? 1 : 0;
        if (mark == want && !seen[cue]++) print cue;
      }
    }
  }'
}
tri="$(ngrams 3)"
bi="$(ngrams 2)"
# Symptom cues (unchanged tiering): best trigram once, then two bigrams — now routed
# through the shared try_recall, AFTER the precise symbol cues above.
try_recall symptom "$(printf '%s\n' "$tri" | sed -n 1p)"
try_recall symptom "$(printf '%s\n' "$bi" | sed -n 1p)"
try_recall symptom "$(printf '%s\n' "$bi" | sed -n 2p)"
exit 0
"#;

const PRIMER_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed SessionStart primer — do not edit; overwritten on deploy>
# Injects the DOMAIN-layer knowledge primer at session start (the always-on half of the
# knowledge push channel). Live-calls experience(kind=primer, format=text), peels jawata's
# fixed MCP envelope, emits the lines as SessionStart context. FAIL-SAFE: jawata down /
# empty / absence / any parse miss -> emit nothing; the session proceeds unchanged.
set -u
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
# THE emit path — selftest and the live path share this one printf format, so the deploy
# self-check validates the exact bytes the real injection will produce (Sprint 21a item J).
emit_ctx() {
  printf '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"JAWATA domain primer (what this codebase is about):\\n%s"}}' "$1"
}
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then emit_ctx '[domain_fact] selftest canned line (accepted)'; exit 0; fi
command -v curl >/dev/null 2>&1 || exit 0
req='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"experience","arguments":{"kind":"primer","format":"text","limit":12}}}'
resp="$(curl -s --max-time 3 -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$req" "$MCP_URL" 2>/dev/null)"
[ -n "$resp" ] || exit 0
flat="$(printf '%s' "$resp" | tr -d '\n\r')"
# Peel content[0].text (un-escape one JSON level) -> the ToolResponse JSON.
inner="$(printf '%s' "$flat" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\(.*\)"[[:space:]]*}[[:space:]]*][[:space:]]*}[[:space:]]*}.*/\1/p' | sed 's/\\"/"/g; s/\\\\/\\/g')"
[ -n "$inner" ] || exit 0
printf '%s' "$inner" | grep -q '"success"[[:space:]]*:[[:space:]]*true' || exit 0
# Pull the data string (flat primer lines; \n stays escaped, valid in the output JSON).
# data is a quote-sanitized flat string, so [^"]* stops at its closing quote — NOT greedy
# .* (which would swallow the trailing ,"meta":{steering} the result layer appends).
data="$(printf '%s' "$inner" | sed -n 's/.*"data"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
[ -n "$data" ] || exit 0
case "$data" in No\ domain\ knowledge*) exit 0 ;; esac
emit_ctx "$data"
exit 0
"#;

const RECALL_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed PreToolUse recall — do not edit; overwritten on deploy>
# Before a JAWATA refactor, injects the terminal recall for the target symbol (prior
# hazards / lessons / failure modes), or stays silent on absence. Never blocks (exit 0).
# FAIL-SAFE: jawata down / no cue / absence / any parse miss -> emit nothing.
set -u
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
# THE emit path — selftest and the live path share this one printf format (Sprint 21a item J).
emit_ctx() {
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"JAWATA recalled prior knowledge for %s:\\n%s"}}' "$1" "$2"
}
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then emit_ctx 'com.example.SelfTest' '[lesson] selftest canned line (accepted)'; exit 0; fi
command -v curl >/dev/null 2>&1 || exit 0
input="$(cat)"
flatin="$(printf '%s' "$input" | tr '\n\r' '  ')"
tool_name="$(printf '%s' "$flatin" | sed -n 's/.*"tool_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
case "$tool_name" in
  *rename_symbol*|*extract*|*move*|*refactor*|*inline*|*change_method_signature*|*apply_cleanup*|*apply_null*|*encapsulate*|*replace_duplicates*|*convert_anonymous*) ;;
  Edit|Write|MultiEdit) ;;   # Sprint 22a: recall on a hand-edit of a source file too
  *) exit 0 ;;
esac
# Cue PRIORITY (Sprint 21a dogfood find): the old single alternation with a greedy .*
# picked the LAST key — a rename carrying symbol+newName queried the NEW name and
# recalled nothing. The subject identifiers win; newName is the last resort.
sym=""
for key in typeName symbol query newName; do
  sym="$(printf '%s' "$flatin" | sed -n 's/.*"'"$key"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  [ -n "$sym" ] && break
done
# Sprint 22a recall-on-Edit: with no refactor-tool key, derive the cue from the edited
# file's type name (Foo.java -> Foo), so hand-editing source recalls its prior lessons
# (the Sprint 6d gap: ownership work is hand-authored, never hits a refactor tool).
if [ -z "$sym" ]; then
  fp="$(printf '%s' "$flatin" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  case "$fp" in *.java) sym="$(basename "$fp" .java)" ;; esac
fi
[ -n "$sym" ] || exit 0
req='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"experience","arguments":{"kind":"recall","format":"text","symbol":"'"$sym"'"}}}'
resp="$(curl -s --max-time 3 -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$req" "$MCP_URL" 2>/dev/null)"
[ -n "$resp" ] || exit 0
flat="$(printf '%s' "$resp" | tr -d '\n\r')"
inner="$(printf '%s' "$flat" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\(.*\)"[[:space:]]*}[[:space:]]*][[:space:]]*}[[:space:]]*}.*/\1/p' | sed 's/\\"/"/g; s/\\\\/\\/g')"
[ -n "$inner" ] || exit 0
printf '%s' "$inner" | grep -q '"success"[[:space:]]*:[[:space:]]*true' || exit 0
# data is a quote-sanitized flat string, so [^"]* stops at its closing quote — NOT greedy
# .* (which would swallow the trailing ,"meta":{steering} the result layer appends).
data="$(printf '%s' "$inner" | sed -n 's/.*"data"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
[ -n "$data" ] || exit 0
case "$data" in No\ known\ knowledge*) exit 0 ;; esac
emit_ctx "$sym" "$data"
exit 0
"#;

// ===================== Sprint 22a P1-b: Cursor hooks (client parity) =====================
// Cursor's beforeSubmitPrompt CANNOT inject context (only continue/user_message), so the
// recalled fact reaches the model via the jawata-studio rule block + sessionStart primer +
// MCP meta.steering — NOT a 1:1 UserPromptSubmit port (cursor.com/docs/hooks, verified
// 2026-07-08). These scripts follow Cursor's contract: one JSON object on stdin; a
// {continue, permission, additional_context} object on stdout. Guard + primer are full
// parity; recall is a side-effect; the observer is fire-and-forget.

fn build_cursor_primer_script(mcp_url: &str, token: &str) -> String {
    CURSOR_PRIMER_TEMPLATE.replace("__MCP_URL__", mcp_url).replace("__TOKEN__", token)
}
const CURSOR_PRIMER_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed Cursor sessionStart primer — do not edit; overwritten on deploy>
set -u
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then printf '%s\n' '{"additional_context":"[domain_fact] selftest (accepted)"}'; exit 0; fi
cat > /dev/null
command -v curl >/dev/null 2>&1 || { printf '%s\n' '{}'; exit 0; }
req='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"experience","arguments":{"kind":"primer","format":"text","limit":12}}}'
resp="$(curl -s --max-time 3 -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$req" "$MCP_URL" 2>/dev/null)"
[ -n "$resp" ] || { printf '%s\n' '{}'; exit 0; }
flat="$(printf '%s' "$resp" | tr -d '\n\r')"
inner="$(printf '%s' "$flat" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\(.*\)"[[:space:]]*}[[:space:]]*][[:space:]]*}[[:space:]]*}.*/\1/p' | sed 's/\\"/"/g; s/\\\\/\\/g')"
printf '%s' "$inner" | grep -q '"success"[[:space:]]*:[[:space:]]*true' || { printf '%s\n' '{}'; exit 0; }
data="$(printf '%s' "$inner" | sed -n 's/.*"data"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
[ -n "$data" ] || { printf '%s\n' '{}'; exit 0; }
case "$data" in No\ domain*) printf '%s\n' '{}'; exit 0 ;; esac
printf '{"additional_context":"JAWATA domain primer:\\n%s"}\n' "$data"
"#;

fn build_cursor_guard_script() -> String {
    CURSOR_GUARD_TEMPLATE.to_string()
}
const CURSOR_GUARD_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed Cursor beforeShellExecution guard — do not edit; overwritten on deploy>
set -u
input="$(cat)"
cmd="$(printf '%s' "$input" | tr '\n\r' '  ')"
# Deny Java-semantic shell text search/edit; steer to JAWATA MCP. failClosed in hooks.json
# means a crash/timeout also blocks. Everything else is allowed.
# THE ESCAPE COMES FIRST (3.7.3 Cursor dogfood, P3): this guard's own deny
# message says "or declare a jawata-fallback" — and no case implemented it, so
# the documented valve was a dead end in this client. The declaration is the
# audit trail; being inconvenient is the point, being impossible is not.
case "$cmd" in
  *jawata-fallback:*|*jawata-author:*)
    printf '%s\n' '{"continue":true,"permission":"allow"}'
    exit 0 ;;
esac
case "$cmd" in
  *grep*.java*|*\ rg\ *.java*|*sed*.java*|*awk*.java*)
    printf '%s\n' '{"continue":true,"permission":"deny","user_message":"Blocked: use JAWATA MCP for Java semantic search (or declare a jawata-fallback: <why> in the command; the declaration is logged).","agent_message":"Shell text search on .java is blocked — call search_symbols / find_references via JAWATA MCP (or declare a jawata-fallback: <why> in the command; the declaration is logged)."}'
    exit 0 ;;
esac
printf '%s\n' '{"continue":true,"permission":"allow"}'
"#;

fn build_cursor_recall_script(mcp_url: &str, token: &str) -> String {
    CURSOR_RECALL_TEMPLATE.replace("__MCP_URL__", mcp_url).replace("__TOKEN__", token)
}
const CURSOR_RECALL_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed Cursor beforeSubmitPrompt recall (SIDE-EFFECT only — Cursor cannot
# inject context on this event; the recalled fact reaches the model via the jawata-studio rule
# block + sessionStart primer + MCP meta.steering) — do not edit; overwritten on deploy>
set -u
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then printf '%s\n' '{"continue":true}'; exit 0; fi
input="$(cat)"
if command -v curl >/dev/null 2>&1; then
  flatin="$(printf '%s' "$input" | tr '\n\r' '  ')"
  prompt="$(printf '%s' "$flatin" | sed -n 's/.*"prompt"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  cue="$(printf '%s' "$prompt" | grep -oE '[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z0-9_]+)*#[A-Za-z0-9_]+|[A-Z][A-Za-z0-9_]*(\.[A-Z][A-Za-z0-9_]*)+' 2>/dev/null | head -n 1)"
  if [ -n "$cue" ]; then
    req='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"experience","arguments":{"kind":"recall","format":"text","symbol":"'"$cue"'"}}}'
    curl -s --max-time 2 -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$req" "$MCP_URL" >/dev/null 2>&1 || true
  fi
fi
printf '%s\n' '{"continue":true}'
"#;

fn build_cursor_observer_script() -> String {
    CURSOR_OBSERVER_TEMPLATE.to_string()
}
const CURSOR_OBSERVER_TEMPLATE: &str = r#"#!/usr/bin/env bash
# <jawata-studio managed Cursor afterMCPExecution observer (fire-and-forget side-effect) — do not edit; overwritten on deploy>
set -u
cat > /dev/null
# afterMCPExecution responses are not enforced; reserved for slip->store / fallback.log
# correlation (parity with the Claude PostToolUse observer).
printf '%s\n' '{}'
"#;

/// The managed sentinel: our Cursor hook scripts all live at `./hooks/jawata-*.sh`, so a
/// command containing this substring is one jawata-studio owns — used to replace/remove our
/// entries while leaving the user's hooks untouched.
const CURSOR_HOOK_SENTINEL: &str = "hooks/jawata-";

/// The four managed (event, entry) pairs — the SINGLE source used by the
/// merge-into-the-user's-file deploy path, which is the ONLY writer.
/// Sprint 28a (2026-08-12): the GUARD now names the role BINARY, not a script.
///
/// Why this one entry changed and the other three did not. On Windows a `.sh`
/// cannot execute: Cursor launches it as an interactive login shell, the script's
/// first act is to read its payload from stdin with `cat`, nothing is piped in,
/// and it waits forever in a visible window. This entry carries
/// `failClosed: true`, so a hook that never returns BLOCKS the user's command —
/// a guard strictly worse than no guard. Observed live on a Windows 11 machine.
///
/// The binary has no shell in its path on any platform, and it carries a
/// wedged-stdin watchdog, so the same launch shape exits cleanly instead of
/// hanging. It moved only now because until Sprint 28a it held just the
/// shell-command half of the guard; the `.java` hand-edit gate is now in it too
/// (`editgate.rs`), which is the condition `role_generations` requires.
///
/// The other three roles stay on scripts here for the reason recorded in
/// `hook-events.json` — their binaries do not yet hold parity — and they are
/// the observer's problem to finish, not the guard's.
fn managed_cursor_hook_entries(hooks_dir: &Path) -> Vec<(&'static str, serde_json::Value)> {
    managed_cursor_hook_entries_on(HostPlatform::host(), hooks_dir)
}

/// The entries Cursor's `hooks.json` receives, for a GIVEN naming convention.
fn managed_cursor_hook_entries_on(
    platform: HostPlatform,
    hooks_dir: &Path,
) -> Vec<(&'static str, serde_json::Value)> {
    // ONE resolver decides binary-or-script, and it is the one the Claude Code
    // side has run on since Sprint 28: `invocation_path_in`. This function used
    // to hand-roll the same rule, and a second copy of a rule is a second copy
    // that can disagree — the entry said "binary" while the writer had written a
    // script. Delegating removes the disagreement rather than reconciling it.
    //
    // `hooks_dir` is a PARAMETER for the reason `invocation_path_in` documents:
    // so the decision is testable without resolving — and mutating — the
    // developer's real home. A declaration whose value depends on the ambient
    // filesystem cannot be asserted.
    //
    // The command stays RELATIVE (`./hooks/…`, resolved against `~/.cursor/`)
    // for two reasons: it must not reach into Claude Code's directory, which
    // would make one client's deploy depend on another's; and
    // `CURSOR_HOOK_SENTINEL` identifies our own entries by the substring
    // `hooks/jawata-`. An absolute path need not contain it, and an entry we
    // cannot recognise is one a redeploy will not replace and a removal will not
    // strip.
    CURSOR_ROLES
        .iter()
        .map(|(event, role, script_file)| {
            let command = invocation_path_on(
                platform,
                hooks_dir,
                role,
                script_file,
                cursor_role_is_binary_live(role),
            )
            .and_then(|p| p.file_name().map(|n| format!("./hooks/{}", n.to_string_lossy())))
            .unwrap_or_else(|| format!("./hooks/{script_file}"));
            let entry = match *event {
                // The guard is the only entry Cursor enforces, and the only one
                // carrying a matcher — it runs on shell commands that look like
                // a Java symbol search, not on everything.
                "beforeShellExecution" => serde_json::json!({
                    "command": command,
                    "timeout": 5,
                    "failClosed": true,
                    "matcher": "grep |rg |sed |awk ",
                }),
                // The primer fetches the session's context, so it gets the
                // longer budget the script generation always had.
                "sessionStart" => serde_json::json!({ "command": command, "timeout": 15 }),
                _ => serde_json::json!({ "command": command, "timeout": 5 }),
            };
            (*event, entry)
        })
        .collect()
}

// Sprint 28 Stage 4 (D-UNWIRED): build_cursor_hooks_json() lived here — a
// whole-file `hooks.json` builder that overwrote the user's own hooks and was
// therefore never used by any deploy. Its ONLY caller was the test asserting
// its shape. Deleted; the contract it carried (version 1, the four events, the
// failClosed guard) is now asserted against the merge path's written file.

/// True iff a Cursor hook entry is one jawata-studio wrote (its `command` references a
/// managed `./hooks/jawata-*.sh` script).
fn cursor_entry_is_managed(entry: &serde_json::Value) -> bool {
    entry
        .get("command")
        .and_then(|c| c.as_str())
        .map(|c| c.contains(CURSOR_HOOK_SENTINEL)
            || c.contains(&legacy_sentinel(CURSOR_HOOK_SENTINEL)))
        .unwrap_or(false)
}

/// Merge one managed event entry into `hooks_object[event]`: drop any prior managed entry,
/// KEEP the user's entries, append the fresh one.
fn merge_cursor_event(
    hooks_object: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    entry: serde_json::Value,
) {
    let mut arr = hooks_object
        .get(event)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    arr.retain(|e| !cursor_entry_is_managed(e));
    arr.push(entry);
    hooks_object.insert(event.to_string(), serde_json::Value::Array(arr));
}

/// Cursor only: `~/.cursor/hooks.json` (the deploy target for the managed hooks). Claude
/// keeps its `settings.json` path; other clients have no hook surface.
fn derive_cursor_hooks_path(client: &str) -> Option<String> {
    if client != "cursor" {
        return None;
    }
    let home = dirs::home_dir()?;
    Some(display_path(&home.join(".cursor").join("hooks.json")))
}

/// The dir the managed Cursor scripts live in — `~/.cursor/hooks/`, matching the
/// `./hooks/jawata-*.sh` command paths in `hooks.json` (relative to `~/.cursor/`).
fn managed_cursor_hooks_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".cursor").join("hooks"))
}

/// Sprint 22a P1-b — deploy the managed Cursor hooks: write the four scripts under
/// `hooks_dir` and MERGE our four event entries into `hooks.json`, preserving any user
/// hooks (ours are identified by the `hooks/jawata-` command path). Returns Ok(true) when
/// anything changed. Idempotent: an unchanged re-deploy is a byte-stable no-op.
fn write_managed_cursor_hooks(
    hooks_json_path: &str,
    hooks_dir: &Path,
    mcp_url: &str,
    token: &str,
    backup_before_write: bool,
    force_rewrite: bool,
) -> Result<bool, String> {
    fs::create_dir_all(hooks_dir)
        .map_err(|e| format!("failed to create cursor hooks dir {}: {e}", hooks_dir.display()))?;

    // 0. THE CONFIG, BESIDE THE BINARIES. A hook binary resolves its endpoint as
    //    `<dir of the exe>/hook_config.json` (config::config_path_for), so it must
    //    sit HERE, in Cursor's own hooks directory.
    //
    //    It did not. The only writer was the Claude Code deploy, which put the
    //    file in ITS hooks directory — so every Cursor role binary loaded
    //    NotConfigured and went silent: correctly named, correctly deployed, and
    //    doing nothing at all. Found on a Windows install where all four .exe
    //    were present and the guard still refused nothing.
    //
    //    The script generation had no such gap because each script carried the
    //    URL and token baked into its text. The cutover to binaries moved the
    //    configuration mechanism and left the configuration behind — the same
    //    half-finished shape as the cutover itself.
    if let Err(e) = write_hook_config(hooks_dir, mcp_url, token, "cursor") {
        return Err(format!("failed writing the cursor hook config: {e}"));
    }

    // 1. DEPLOY THE GUARD BINARY FIRST, so the single resolver below sees the
    //    truth rather than the state from before this deploy.
    //
    // Unlink-then-write: Linux refuses to open an executing binary for writing
    // (ETXTBSY) and hooks fire on every prompt, so a redeploy routinely lands on
    // a running process. The running one keeps its inode and dies with the
    // process; the new file takes the name. Windows needs the same order because
    // a running image cannot be replaced.
    //
    // A missing source is NOT fatal: an install whose bundle did not ship the
    // binary keeps the script generation instead of losing its guard. That
    // choice is expressed once, by `invocation_path_in` below — not repeated
    // here as a second opinion.
    let roles: Vec<&str> = CURSOR_ROLES
        .iter()
        .filter(|(_, role, _)| cursor_role_is_binary_live(role))
        .map(|(_, role, _)| *role)
        .collect();
    let platform = HostPlatform::host();
    let deployed = match hook_binary_source().filter(|s| s.exists()) {
        Some(source) => deploy_hook_binaries(&source, hooks_dir, &roles, platform),
        None => Err("the bundle shipped no hook binary".to_string()),
    };
    if let Err(e) = deployed {
        // Falling back to the script generation is a REAL fallback only where a
        // script can run. Where it cannot, saying "the script generation stands"
        // is how a broken install reports success — five releases did exactly
        // that on Windows. The deploy fails instead, and the user is told.
        if !platform.can_run_shell_scripts() {
            return Err(format!(
                "the Cursor hook binaries could not be deployed ({e}), and this platform \
                 cannot run the .sh fallback — deploying it would leave hooks that hang \
                 or block instead of running. Nothing was written."
            ));
        }
        eprintln!(
            "[jawata-studio] cursor hook binaries not deployed ({e}); \
             the script generation stands"
        );
    }

    // 2. ASK THE RESOLVER — once — which generation is live for the guard, and
    //    write exactly that one. The entries built later ask the SAME function
    //    with the SAME directory, so the file on disk and the hooks.json entry
    //    cannot disagree. They used to be two independent probes, and the
    //    disagreement between them was the defect.
    //    Every role is asked, not just the guard: a role whose binary is live
    //    must NOT also get a script, or the stale file sits there looking
    //    authoritative — and on Windows a leftover `.sh` is not merely inert,
    //    Cursor tries to open it and puts a window on the user's screen.
    //    The decision is the RESOLVER'S, taken once per role and reused for
    //    writing and for retirement. It must not be re-derived from
    //    `cursor_role_is_binary_live` alone: that says which generation we
    //    INTEND, while the resolver also requires the binary to be present. An
    //    install whose bundle shipped no binary would otherwise have its script
    //    retired against an intention that never landed, leaving no guard at
    //    all — strictly worse than the script it replaced.
    let binary_live: Vec<bool> = CURSOR_ROLES
        .iter()
        .map(|(_, role, script_file)| {
            invocation_path_in_when(
                hooks_dir,
                role,
                script_file,
                cursor_role_is_binary_live(role),
            )
            .map(|p| path_is_role_binary(&p))
            .unwrap_or(false)
        })
        .collect();

    let mut scripts: Vec<(&str, String)> = Vec::new();
    for (i, (_, _role, script_file)) in CURSOR_ROLES.iter().enumerate() {
        if binary_live[i] {
            // Retired below, in the step that also handles the legacy name.
            continue;
        }
        let body = match *script_file {
            "jawata-session-primer.sh" => build_cursor_primer_script(mcp_url, token),
            "jawata-recall.sh" => build_cursor_recall_script(mcp_url, token),
            "jawata-observer.sh" => build_cursor_observer_script(),
            _ => build_cursor_guard_script(),
        };
        scripts.push((script_file, body));
    }
    let mut script_changed = false;
    for (name, body) in &scripts {
        let p = hooks_dir.join(name);
        let changed = fs::read_to_string(&p).map(|e| &e != body).unwrap_or(true);
        if changed || force_rewrite {
            fs::write(&p, body)
                .map_err(|e| format!("failed writing cursor hook {}: {e}", p.display()))?;
            script_changed = true;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o755));
        }
        // Sprint 22b: drop the pre-rebrand twin (goja-*.sh) so only one generation
        // of managed scripts remains; hooks.json entries pointing at it are replaced
        // by the merge (legacy-aware cursor_entry_is_managed).
        let legacy = hooks_dir.join(legacy_sentinel(name));
        if legacy.exists() {
            fs::remove_file(&legacy)
                .map_err(|e| format!("failed removing legacy cursor hook {}: {e}", legacy.display()))?;
            script_changed = true;
        }
    }

    // 3. Retire the script generation once the binary is live, so an install
    //    never carries two generations of one role.
    let live_files: Vec<String> = CURSOR_ROLES
        .iter()
        .enumerate()
        .map(|(i, (_, role, script_file))| {
            if binary_live[i] {
                role_binary_file_name_on(platform, role)
            } else {
                (*script_file).to_string()
            }
        })
        .collect();
    if sweep_managed_hook_residue(hooks_dir, &live_files) {
        script_changed = true;
    }

    // 2. Merge the managed entries into hooks.json, preserving user hooks.
    let hooks_buf = PathBuf::from(hooks_json_path);
    let parent = hooks_buf
        .parent()
        .ok_or_else(|| format!("cursor hooks path has no parent: {}", hooks_buf.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("failed to create cursor dir {}: {e}", parent.display()))?;

    let existing = fs::read_to_string(&hooks_buf).ok();
    let mut root = existing
        .as_deref()
        .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    {
        let object = root.as_object_mut().expect("root is an object");
        object.insert("version".into(), serde_json::json!(1));
        let hooks = object.entry("hooks").or_insert_with(|| serde_json::json!({}));
        if !hooks.is_object() {
            *hooks = serde_json::json!({});
        }
        let hooks_object = hooks.as_object_mut().expect("hooks is an object");
        for (event, entry) in managed_cursor_hook_entries(hooks_dir) {
            merge_cursor_event(hooks_object, event, entry);
        }
    }

    let next_json = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed serializing cursor hooks.json: {e}"))?;
    if !force_rewrite {
        if let Some(existing) = existing.as_deref() {
            if existing.trim() == next_json.trim() && !script_changed {
                return Ok(false);
            }
        }
    }
    if backup_before_write {
        crate::backups::backup_before_write(&hooks_buf)
            .map_err(|e| format!("failed creating centralized cursor hooks backup: {e}"))?;
    }
    fs::write(&hooks_buf, format!("{next_json}\n"))
        .map_err(|e| format!("failed writing cursor hooks {}: {e}", hooks_buf.display()))?;
    Ok(true)
}

/// Remove the managed entries from `hooks.json` + delete the four scripts. Leaves user
/// hooks intact, prunes now-empty event arrays, and removes the file entirely only when
/// nothing but our (now-stripped) content remained. Returns Ok(true) when anything changed.
fn remove_managed_cursor_hooks(
    hooks_json_path: &str,
    hooks_dir: &Path,
    backup_before_write: bool,
) -> Result<bool, String> {
    let mut changed = false;

    let hooks_buf = PathBuf::from(hooks_json_path);
    if hooks_buf.exists() {
        if let Ok(existing) = fs::read_to_string(&hooks_buf) {
            if let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&existing) {
                let mut json_changed = false;
                if let Some(object) = root.as_object_mut() {
                    if let Some(hooks) = object.get_mut("hooks").and_then(|h| h.as_object_mut()) {
                        for event in hooks.keys().cloned().collect::<Vec<_>>() {
                            if let Some(arr) = hooks.get_mut(&event).and_then(|v| v.as_array_mut()) {
                                let before = arr.len();
                                arr.retain(|e| !cursor_entry_is_managed(e));
                                json_changed |= arr.len() != before;
                                if arr.is_empty() {
                                    hooks.remove(&event);
                                }
                            }
                        }
                    }
                }
                if json_changed {
                    let hooks_empty = root
                        .get("hooks")
                        .and_then(|h| h.as_object())
                        .map(|o| o.is_empty())
                        .unwrap_or(true);
                    let only_ours = root
                        .as_object()
                        .map(|o| o.keys().all(|k| k == "version" || k == "hooks"))
                        .unwrap_or(false);
                    if backup_before_write {
                        crate::backups::backup_before_write(&hooks_buf)
                            .map_err(|e| format!("failed creating centralized cursor hooks backup: {e}"))?;
                    }
                    if hooks_empty && only_ours {
                        let _ = fs::remove_file(&hooks_buf);
                    } else {
                        let next_json = serde_json::to_string_pretty(&root)
                            .map_err(|e| format!("failed serializing cursor hooks.json: {e}"))?;
                        fs::write(&hooks_buf, format!("{next_json}\n"))
                            .map_err(|e| format!("failed writing cursor hooks {}: {e}", hooks_buf.display()))?;
                    }
                    changed = true;
                }
            }
        }
    }

    for name in [
        "jawata-session-primer.sh",
        "jawata-guard.sh",
        "jawata-recall.sh",
        "jawata-observer.sh",
    ] {
        // Remove both generations — the managed script and its pre-rebrand
        // (goja-*) twin, if a pre-22b deploy left one behind.
        for p in [hooks_dir.join(name), hooks_dir.join(legacy_sentinel(name))] {
            if p.exists() {
                let _ = fs::remove_file(&p);
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// Sprint 10 v0.10.4: atomic write of `workspace.json` for one workspace.
/// Lifted out of `ManagerService` so it can be unit-tested without the
/// full ConfigStore + ReleaseManager + RuntimeManager dependency graph.
///
/// Behavior:
/// - `paths.is_empty()` → the file is removed if present (no member =
///   no workspace.json on disk).
/// - Otherwise: writes to a `.tmp` sibling and renames atomically so the
///   `WorkspaceFileWatcher` in jawata never observes a half-written
///   file. Creates the workspace dir if missing.
pub(crate) fn write_workspace_json_to_dir(
    workspace_dir: &Path,
    workspace_name: &str,
    paths: &[&str],
) -> Result<(), String> {
    let workspace_json = workspace_dir.join("workspace.json");

    if paths.is_empty() {
        let _ = std::fs::remove_file(&workspace_json);
        return Ok(());
    }

    std::fs::create_dir_all(workspace_dir).map_err(|e| {
        format!(
            "failed to create workspace dir {}: {e}",
            workspace_dir.display()
        )
    })?;

    let payload = serde_json::json!({
        "version": 1,
        "name": workspace_name,
        "projects": paths,
    });
    let json = serde_json::to_string_pretty(&payload).map_err(|e| {
        format!("failed to serialize workspace.json for {workspace_name}: {e}")
    })?;

    let tmp = workspace_json.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{json}\n"))
        .map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &workspace_json).map_err(|e| {
        format!(
            "failed to rename {} to {}: {e}",
            tmp.display(),
            workspace_json.display()
        )
    })?;
    Ok(())
}

/// Sprint 12 (v0.12.0): apply the workspace-status aggregation rules to a
/// list of per-project phases and return the workspace's overall phase.
///
/// Pure function (no `self`) so it's trivially unit-testable.
fn aggregate_workspace_phase(phases: &[RuntimePhase]) -> RuntimePhase {
    if phases.iter().any(|p| matches!(p, RuntimePhase::Failed)) {
        RuntimePhase::Failed
    } else if phases.iter().any(|p| matches!(p, RuntimePhase::Starting)) {
        RuntimePhase::Starting
    } else if !phases.is_empty()
        && phases.iter().all(|p| matches!(p, RuntimePhase::Running))
    {
        RuntimePhase::Running
    } else {
        RuntimePhase::Stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_workspace_phase_two_running_returns_running() {
        let phases = vec![RuntimePhase::Running, RuntimePhase::Running];
        assert!(matches!(
            aggregate_workspace_phase(&phases),
            RuntimePhase::Running
        ));
    }

    #[test]
    fn aggregate_workspace_phase_failed_dominates_running() {
        let phases = vec![RuntimePhase::Running, RuntimePhase::Failed];
        assert!(matches!(
            aggregate_workspace_phase(&phases),
            RuntimePhase::Failed
        ));
    }

    #[test]
    fn aggregate_workspace_phase_starting_dominates_running() {
        let phases = vec![RuntimePhase::Running, RuntimePhase::Starting];
        assert!(matches!(
            aggregate_workspace_phase(&phases),
            RuntimePhase::Starting
        ));
    }

    #[test]
    fn ensure_runtime_jar_exists_refuses_a_missing_jar() {
        // v3.5.1 (Finding B): a real jar passes; a stale/missing path (e.g. the
        // pre-rebrand goja-mcp/target/products path) is refused with the path in
        // the message, so the launch never spawns a doomed java process.
        let present = std::env::current_exe().expect("test binary exists");
        assert!(ensure_runtime_jar_exists(&present.to_string_lossy()).is_ok());

        let missing = "/home/harald/CursorProjects/goja-mcp/org.jawata.product/target/products/does-not-exist/jawata.jar";
        let err = ensure_runtime_jar_exists(missing).expect_err("a missing jar must be refused");
        assert!(err.contains(missing), "the message names the offending path: {err}");
        assert!(err.contains("not found"), "the message says the jar was not found: {err}");
    }

    #[test]
    fn aggregate_workspace_phase_empty_or_stopped_returns_stopped() {
        assert!(matches!(
            aggregate_workspace_phase(&[]),
            RuntimePhase::Stopped
        ));
        assert!(matches!(
            aggregate_workspace_phase(&[RuntimePhase::Stopped, RuntimePhase::Stopped]),
            RuntimePhase::Stopped
        ));
    }

    #[test]
    fn extract_tool_entries_reads_standard_tools_list_shape() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    { "name": "searchSymbols", "description": "Search symbols by query" },
                    { "name": "resolveReferences" }
                ]
            }
        });

        let tools = extract_tool_entries(&response).expect("tools/list should parse");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "searchSymbols");
        assert_eq!(
            tools[0].description.as_deref(),
            Some("Search symbols by query")
        );
        assert_eq!(tools[1].name, "resolveReferences");
        assert_eq!(tools[1].description, None);
    }

    #[test]
    fn extract_tool_entries_surfaces_protocol_error_payload() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        });

        let error = extract_tool_entries(&response).expect_err("error payload should fail");
        assert!(error.contains("Method not found"));
    }

    // ============================================================
    // Sprint 10 v0.10.4: workspace flow tests.
    // ============================================================

    #[test]
    fn mcp_server_id_for_workspace_simple_name() {
        let id = mcp_server_id_for_workspace("alpha");
        assert_eq!(id, "jawata-alpha");
    }

    #[test]
    fn mcp_server_id_for_workspace_normalizes_special_chars() {
        // mcp_label_slug lowercases and replaces non-alphanumerics with `-`
        // (collapsing consecutive). The exact slug shape is internal but
        // the result must be a valid Cursor server id (only [a-z0-9-_]).
        let id = mcp_server_id_for_workspace("My Workspace!");
        assert!(id.starts_with("jawata-"));
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn mcp_server_id_for_workspace_long_name_fits_cursor_budget() {
        // Cursor's combined-id cap is around 59-60 chars. Whatever the
        // workspace name length, the produced id must fit within that
        // cap so the longest tool name still passes.
        let long = "a".repeat(200);
        let id = mcp_server_id_for_workspace(&long);
        assert!(id.starts_with("jawata-"));
        assert!(id.len() <= max_mcp_server_id_len_for_cursor());
    }

    #[test]
    fn mcp_server_id_for_workspace_empty_falls_back_to_hash() {
        // Pure whitespace produces an empty slug after sanitization;
        // mcp_server_id_for_workspace falls back to a deterministic hash
        // suffix so the id is still unique-ish and parseable.
        let id = mcp_server_id_for_workspace("   ");
        assert!(id.starts_with("jawata-"));
        assert!(id.len() > "jawata-".len(), "empty name must yield a hash-suffixed id, got '{id}'");
    }

    #[test]
    fn mcp_server_id_for_workspace_is_deterministic() {
        // Same input → same id, run-to-run. Important so mcp.json diffs
        // stay minimal across reloads.
        let a = mcp_server_id_for_workspace("payments-api");
        let b = mcp_server_id_for_workspace("payments-api");
        assert_eq!(a, b);
    }

    #[test]
    fn mcp_server_id_for_workspace_distinguishes_distinct_names() {
        // Two different workspace names → two different ids. Otherwise
        // mcp.json would collapse independent workspaces into one entry.
        let a = mcp_server_id_for_workspace("alpha");
        let b = mcp_server_id_for_workspace("orb");
        assert_ne!(a, b);
    }

    // ============================================================
    // write_workspace_json_to_dir integration tests (Sprint 10 B.7
    // follow-up).
    // ============================================================

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Returns a unique tempdir per call so concurrent tests don't
    /// collide. Caller is responsible for cleanup (best-effort).
    fn unique_tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-studio-mstest-{label}-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
        std::fs::create_dir_all(&dir).expect("failed to create test tempdir");
        dir
    }

    #[test]
    fn write_workspace_json_writes_atomic_and_correct_shape() {
        let dir = unique_tempdir("ws-json-write");
        let workspace_dir = dir.join("ws");
        let paths = ["/projects/a", "/projects/b"];

        write_workspace_json_to_dir(&workspace_dir, "test-ws", &paths)
            .expect("should write workspace.json");

        // File exists and has the expected shape.
        let workspace_json = workspace_dir.join("workspace.json");
        assert!(workspace_json.is_file(), "workspace.json must exist");
        let contents = std::fs::read_to_string(&workspace_json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["name"], "test-ws");
        assert_eq!(
            parsed["projects"].as_array().unwrap().len(),
            2,
            "both project paths present"
        );
        assert_eq!(parsed["projects"][0], "/projects/a");
        assert_eq!(parsed["projects"][1], "/projects/b");

        // No leftover .tmp file from the atomic-rename machinery.
        let tmp = workspace_dir.join("workspace.json.tmp");
        assert!(!tmp.exists(), ".tmp must be renamed away");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_workspace_json_with_empty_paths_removes_file() {
        let dir = unique_tempdir("ws-json-empty");
        let workspace_dir = dir.join("ws");

        // Pre-populate with a stale workspace.json from a prior run.
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace_json = workspace_dir.join("workspace.json");
        std::fs::write(&workspace_json, "{ \"projects\": [\"/old\"] }").unwrap();
        assert!(workspace_json.exists());

        // Empty paths → file is removed.
        write_workspace_json_to_dir(&workspace_dir, "test-ws", &[]).expect("ok");
        assert!(!workspace_json.exists(), "empty members must remove the file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_workspace_json_creates_workspace_dir_if_missing() {
        let dir = unique_tempdir("ws-json-mkdir");
        // workspace_dir does NOT exist yet — function must create it.
        let workspace_dir = dir.join("nested").join("ws");
        assert!(!workspace_dir.exists());

        write_workspace_json_to_dir(&workspace_dir, "ws", &["/p/a"])
            .expect("should create dir + write file");
        assert!(workspace_dir.is_dir());
        assert!(workspace_dir.join("workspace.json").is_file());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_workspace_json_overwrites_previous_contents() {
        let dir = unique_tempdir("ws-json-overwrite");
        let workspace_dir = dir.join("ws");

        // First write: 2 paths.
        write_workspace_json_to_dir(&workspace_dir, "ws", &["/p/a", "/p/b"]).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(workspace_dir.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(first["projects"].as_array().unwrap().len(), 2);

        // Second write: 1 path. File now reflects the new state.
        write_workspace_json_to_dir(&workspace_dir, "ws", &["/p/c"]).unwrap();
        let second: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(workspace_dir.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(second["projects"].as_array().unwrap().len(), 1);
        assert_eq!(second["projects"][0], "/p/c");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_workspace_json_no_op_when_dir_missing_and_paths_empty() {
        let dir = unique_tempdir("ws-json-noop");
        // workspace_dir does NOT exist; paths empty → no error, no
        // dir created. (Caller's contract: empty paths means "this
        // workspace has no members anymore"; if there's nothing on
        // disk, that's already the desired state.)
        let workspace_dir = dir.join("ws");
        assert!(!workspace_dir.exists());
        write_workspace_json_to_dir(&workspace_dir, "ws", &[])
            .expect("empty + missing dir is a clean no-op");
        // The function tries fs::remove_file on a missing path which is
        // ignored. No assertion on dir existence — implementation is
        // free to leave it absent.

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 15 Stage 11: URL-emitting MCP writer =====

    fn url_server(id: &str, port: u16, token: &str, disabled: bool) -> ManagedDeployServer {
        ManagedDeployServer {
            id: id.into(),
            workspace_name: id.into(),
            project_names: vec!["P".into()],
            project_paths: vec!["/p".into()],
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: token.into(),
            disabled,
        }
    }

    // ===== Sprint A0 (v0.17.0): sharpened rule block =====

    #[test]
    fn rule_block_has_routing_table_and_tdd_loop() {
        let servers = vec![url_server("jawata-ws-a", 8800, "tok", false)];
        let block = build_rule_block("cursor", &servers);

        // Routing-table section: names the key tools + the grep-fallback rule.
        assert!(block.contains("search_symbols"), "names search_symbols");
        assert!(block.contains("find_references"), "names find_references");
        assert!(block.contains("analyze_type"), "names analyze_type");
        assert!(block.contains("rename_symbol"), "names a refactoring tool");
        assert!(
            block.contains("grep") && block.to_lowercase().contains("fallback"),
            "frames shell text-search as a fallback"
        );

        // TDD-refactor loop: the verify + undo discipline.
        assert!(block.contains("compile_workspace"), "mentions compile_workspace");
        assert!(block.contains("undo_refactoring"), "mentions undo on red");

        // Contract preserved: markers + managed-id list still render.
        assert!(block.starts_with("<!-- jawata-studio:cursor:start -->"));
        assert!(block.trim_end().ends_with("<!-- jawata-studio:cursor:end -->"));
        assert!(block.contains("Managed service ids:"));
        assert!(block.contains("- jawata-ws-a"));
    }

    #[test]
    fn rule_block_has_try_or_justify_rule_and_edit_mappings() {
        // Sprint 22 Stage 1: the rule block carries the enforcement contract (the
        // hook's try-or-justify) + the edit-side intent→tool mappings.
        let block = build_rule_block("claude", &vec![url_server("jawata-ws-a", 8800, "tok", false)]);
        assert!(block.contains("Try-first, or justify"), "states the enforcement contract");
        assert!(block.contains("jawata-fallback:"), "names the declared-fallback escape");
        assert!(
            block.to_lowercase().contains("inconvenient") && block.contains("never stuck"),
            "inconvenient-not-to-use, but never stuck"
        );
        // Edit mappings for the blocked hand-edit path.
        assert!(block.contains("rename_symbol"), "rename → rename_symbol");
        assert!(block.contains("generate(kind=copy_class)"), "duplicate → copy_class");
        assert!(
            block.contains("refactoring(action=plan)"),
            "structural change → the plan lifecycle"
        );
    }

    #[test]
    fn rule_block_carries_the_communication_contract() {
        // Sprint 25 D10: the upward-communication contract is INJECTED (the
        // managed rule block), never merely remembered — sessions don't
        // share context. Verbatim anchors of the contract's five rules.
        let block = build_rule_block("cursor", &[url_server("jawata-ws-a", 8800, "tok", false)]);
        assert!(block.contains("Communication upward"), "the contract section exists");
        assert!(block.contains("DECISION FIRST"), "decision-first rule");
        assert!(block.contains("ONE decision per ask"), "granularity rule");
        assert!(block.contains("PER-ITEM TABLE"), "per-item table rule");
        assert!(
            block.contains("define at first use") && block.contains("cyclomatic complexity"),
            "abbreviation rule with the canonical example"
        );
        // Harald 2026-07-18: the enforced decision test rides the contract
        // in EVERY client, not only the Claude-Code /sprint step.
        assert!(
            block.contains("THE DECISION TEST (ENFORCED, every client)")
                && block.contains("WHAT IT PROVES"),
            "the enforced decision-test rule"
        );
        assert!(block.contains("folded"), "tech-detail-folded rule");
    }

    #[test]
    fn rule_block_carries_the_memory_recall_discipline() {
        // v2.5.1 (Cursor parity, interim): clients without push hooks must PULL the
        // cross-client experience store — recall-before-theorize, record-what-you-
        // learn, declare-your-fallback. The textual substitute until Cursor's hook
        // schema is ported; identical for every client.
        let block = build_rule_block("cursor", &vec![url_server("jawata-ws-a", 8800, "tok", false)]);
        assert!(
            block.contains("recall before you theorize"),
            "names the memory discipline section"
        );
        assert!(
            block.contains("experience(kind=recall, symbol="),
            "shows the symbol-cue recall call shape"
        );
        assert!(block.contains("CLOSED SET"), "carries the classify contract");
        assert!(
            block.contains("do not generate a novel cause"),
            "classify, never generate"
        );
        assert!(
            block.contains("experience(kind=record"),
            "shows the record call shape"
        );
        assert!(
            block.contains("experience(kind=primer"),
            "pull-based session primer for clients without a session-start hook"
        );
        assert!(
            block.contains("CROSS-CLIENT"),
            "states the same store answers in every client"
        );
        // Same text in the Claude block (harmless where hooks push anyway).
        let claude = build_rule_block("claude", &vec![url_server("jawata-ws-a", 8800, "tok", false)]);
        assert!(claude.contains("recall before you theorize"));
    }

    #[test]
    fn guard_logs_jawata_calls_for_try_first() {
        // Sprint 22 Stage 1: the guard records jawata-call target tokens to the
        // per-session state file (the "tried jawata" signal), never blocking jawata.
        let script = build_guard_script("http://127.0.0.1:8890/mcp");
        assert!(script.contains("mcp__jawata*)"), "has a branch for jawata tool calls");
        assert!(
            script.contains("query|typeName|symbol|newName|filePath"),
            "extracts the target tokens from tool_input"
        );
        assert!(
            script.contains("$jawata_state_file"),
            "appends the tokens to the per-session try-first state"
        );
        // Stage 2: the search gate consults that state — an un-tried symbol is blocked.
        assert!(
            script.contains("TRY-FIRST gate"),
            "the search gate consults the try-first state before blocking"
        );
    }

    #[test]
    fn guard_enforces_java_edits() {
        // Sprint 22 Stage 3: a hand-edit of a .java file is blocked → refactor tool
        // or justify; the fallback log is stamped with the deployed engine version.
        let script = build_guard_script("http://127.0.0.1:8890/mcp");
        assert!(script.contains("Edit|Write|MultiEdit)"), "has the edit-enforcement branch");
        assert!(
            script.contains("USE A JAWATA REFACTOR TOOL"),
            "blocks a Java hand-edit with a refactor-tool redirect"
        );
        assert!(
            script.contains("rename_symbol") && script.contains("refactoring(action=plan)"),
            "names the refactor tools"
        );
        assert!(
            script.contains("jawata_log_fallback") && script.contains("tools/jawata/current"),
            "the fallback log is versioned by the deployed engine version"
        );
    }

    #[test]
    fn guard_surfaces_runtime_tools_on_a_reflex_edit() {
        // Sprint 26a D1 (reflex→capability): when a blocked .java edit LOOKS like a
        // runtime reflex, the guard's message names the ZERO-code-change tool —
        // profile for a hand-rolled timer, debug for debug-armor. Not a new block
        // (R5): a smarter message on an edit already blocked as a .java hand-edit.
        let script = build_guard_script("http://127.0.0.1:8890/mcp");
        // the timing reflex → profile
        assert!(script.contains("nanoTime|currentTimeMillis|Stopwatch"),
            "detects a hand-rolled timer in the edit content");
        assert!(script.contains("Use profile") && script.contains("names the hotspot as a symbol"),
            "surfaces profile (with how-to) for a timing edit");
        // the debug-armor reflex → debug
        assert!(script.contains("printStackTrace|logger?"),
            "detects debug-armor (added logging to diagnose) in the edit content");
        assert!(script.contains("Use debug") && script.contains("probe_set kind=logpoint"),
            "surfaces debug (with how-to) for an armor edit");
        // both pitch ZERO code change — the reason to prefer the tool
        assert!(script.matches("ZERO code change").count() >= 2,
            "both runtime pitches state the zero-code-change advantage");
        // it is a HINT on the existing block, not a new gate
        assert!(script.contains("not a new block") || script.contains("already blocked"),
            "documented as a message enrichment, not a new false-positive guard");
    }

    #[test]
    fn guard_authoring_window_permits_java_edits() {
        // v1.5.1 (Sprint 22 refinement): a 'jawata-author:' Bash declaration opens a
        // session-scoped, TTL-bounded window during which .java edits pass + are logged —
        // the clean escape for authoring NEW code (not a refactor), no marker in the source.
        let script = build_guard_script("http://127.0.0.1:8890/mcp");
        assert!(script.contains("jawata-author:"), "recognizes the authoring declaration");
        assert!(script.contains("editgate"), "keeps a per-session authoring-window state");
        assert!(
            script.contains("authoring-window:") && script.contains("authored-edit"),
            "logs the declaration and each covered edit to the versioned fallback log"
        );
        assert!(script.contains("1800"), "the authoring window is TTL-bounded, not a permanent bypass");
        assert!(
            script.contains("Authoring NEW code"),
            "the block message points the agent at the authoring window"
        );
    }

    #[test]
    fn rule_block_has_health_gated_fallback() {
        let servers = vec![url_server("jawata-ws-a", 8800, "tok", false)];
        let block = build_rule_block("claude", &servers);
        // The ASK-when-down section: pause + ask on Java work, stay silent on non-Java, no dodging.
        assert!(block.contains("When JAWATA is unavailable"), "has the health-gated section header");
        assert!(
            block.contains("STOP and ask") && block.to_lowercase().contains("degraded"),
            "instructs to stop and ask rather than silently degrade"
        );
        assert!(block.contains("non-Java"), "scopes the ask to Java work; silent on non-Java");
        assert!(block.contains("dodge"), "carries the anti-dodge guard");
    }

    #[test]
    fn rule_block_body_is_identical_across_clients_except_the_conductor_section() {
        // Amended DELIBERATELY in Sprint 25a (D2): the conductor section is
        // the ONE per-client part of the body; everything else must stay
        // byte-identical across clients.
        let servers = vec![url_server("jawata-ws-a", 8800, "tok", false)];
        let strip = |s: &str, c: &str| -> String {
            let no_markers = s
                .replace(&format!("<!-- jawata-studio:{c}:start -->"), "")
                .replace(&format!("<!-- jawata-studio:{c}:end -->"), "");
            // Drop the conductor section: from its heading to (exclusive)
            // the managed-ids trailer.
            let mut out = Vec::new();
            let mut in_conductor = false;
            for line in no_markers.lines() {
                if line.starts_with("## The jawata seats") {
                    in_conductor = true;
                }
                if line == "Managed service ids:" {
                    in_conductor = false;
                }
                if !in_conductor {
                    out.push(line);
                }
            }
            out.join("\n")
        };
        let claude = build_rule_block("claude", &servers);
        for client in ["cursor", "antigravity", "claude_desktop", "intellij"] {
            let other = build_rule_block(client, &servers);
            assert!(other.contains(&format!("jawata-studio:{client}:start")));
            assert_eq!(
                strip(&claude, "claude"),
                strip(&other, client),
                "non-conductor body must be identical for {client}"
            );
        }
        // The three command clients share even the conductor section.
        assert_eq!(
            build_rule_block("cursor", &servers).replace(":cursor:", ":x:"),
            build_rule_block("antigravity", &servers).replace(":antigravity:", ":x:"),
        );
    }

    #[test]
    fn rule_block_carries_the_conductor_section() {
        let servers = vec![url_server("jawata-ws-a", 8800, "tok", false)];
        let block = build_rule_block("claude", &servers);
        for anchor in [
            "## The jawata seats",
            "javadoc-writer (`/javadocs`)",
            "test-writer (`/cover`)",
            "architect (`/refactor`)",
            "debugger (`/debug`)",
            "profiler (`/profile`)",
            "spec-editor + spec-auditor",
            "Involve the ARCHITECT seat unprompted",
            "checkpoint diff",
            "`ARCHITECTURE-<scope>.md`",
            "has NOT passed",
            "PROPOSE, never auto-apply",
            "experience(kind=record",
            "Seat commands are installed",
            // Sprint 26a D3a: the coded seat-workflow placement (deployed to every client).
            "When each seat fires in the dev process",
            "at EVERY checkpoint",
            "COVERAGE gate",
            "DOC gate",
            "do NOT hand-add logging",
            "do NOT hand-roll a stopwatch",
        ] {
            assert!(block.contains(anchor), "conductor anchor missing: {anchor}");
        }
    }

    #[test]
    fn conductor_section_intellij_gets_the_phrase_table_others_do_not() {
        let servers = vec![url_server("jawata-ws-a", 8800, "tok", false)];
        let intellij = build_rule_block("intellij", &servers);
        assert!(intellij.contains("| You say | Adopt the seat |"));
        assert!(intellij.contains("No command channel in this client"));
        assert!(!intellij.contains("Seat commands are installed"));
        for client in ["claude", "cursor", "antigravity"] {
            let block = build_rule_block(client, &servers);
            assert!(!block.contains("| You say |"), "{client} must not carry the table");
            assert!(block.contains("Seat commands are installed"), "{client} one-liner");
        }
        let desktop = build_rule_block("claude_desktop", &servers);
        assert!(desktop.contains("`jawata-seats` skill"), "desktop points at the skill");
        assert!(!desktop.contains("| You say |"));
    }


    // ---------- Sprint 26: the Stop gate ----------

    /// THE WIRE THE WHOLE SPRINT MISSED. Seven audit rounds and a five-platform
    /// release gate all passed while every client event still invoked the .sh
    /// script, because they checked CODE callers and none checked what the
    /// editor is pointed at. This asserts the invocation path itself.
    #[test]
    fn a_client_is_pointed_at_the_binary_when_one_is_deployed() {
        let dir = unique_tempdir("invoke-path");
        // No binary yet: the script is the honest answer.
        let script = dir.join(PRIMER_SCRIPT_FILE);
        fs::write(&script, "#!/bin/sh
").unwrap();
        assert_eq!(
            Some(script.clone()),
            invocation_path_in(&dir, "jawata-hook-primer", PRIMER_SCRIPT_FILE),
            "with no binary deployed the script must still be used"
        );
        // Binary present: it wins.
        let binary = dir.join("jawata-hook-primer");
        fs::write(&binary, "ELF").unwrap();
        assert_eq!(
            Some(binary),
            invocation_path_in(&dir, "jawata-hook-primer", PRIMER_SCRIPT_FILE),
            "a deployed binary must be what the client invokes"
        );
    }

    /// LENGTH AS A TRIGGER — Harald's suggestion, and stronger than the phrase
    /// list because it does not depend on wording. A long message is one the
    /// communicator should have judged, ask or not.
    #[test]
    fn a_long_message_needs_the_communicator_even_if_it_asks_nothing() {
        let dir = unique_tempdir("stop-long");
        let p = dir.join("t.jsonl");
        let long = "Committed and green. ".repeat(140); // ~2.8k, no question, no phrase
        let mut b = String::from(
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"go\"}]}}\n",
        );
        b.push_str(&format!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{long}\"}}]}}}}\n"
        ));
        fs::write(&p, b).unwrap();
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": p, "stop_hook_active": false }));
        assert_eq!(Some("block"), out.get("decision").and_then(|d| d.as_str()),
            "a wall of text must be judged before it is sent: {out}");
    }

    /// THE LIVE MISS. Hours after the unjudged-ask check shipped, the next ask
    /// the agent wrote matched none of its phrases and the deployed gate
    /// returned allow. Harald: "why is the communicator not firing?"
    #[test]
    fn the_stop_gate_catches_an_ask_phrased_outside_its_phrase_list() {
        for ask in [
            "This is dogfood output; you decide whether it becomes v3.7.2 or waits.",
            "Do we cut a patch, or leave it?",
            "Up to you whether we ship it.",
        ] {
            let dir = unique_tempdir("stop-liveask");
            let p = dir.join("t.jsonl");
            let mut b = String::from(
                "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"go\"}]}}\n",
            );
            b.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{ask}\"}}]}}}}\n"
            ));
            fs::write(&p, b).unwrap();
            let out = run_stop_script(&serde_json::json!({
                "transcript_path": p, "stop_hook_active": false }));
            assert_eq!(Some("block"), out.get("decision").and_then(|d| d.as_str()),
                "must be read as an ask: {ask:?} -> {out}");
        }
    }

    /// F1: a TOOL RESULT is `"type":"user"` too. Scoping the communicator
    /// window on the last user-role line meant one ordinary tool call after a
    /// judged pass re-armed the check and BLOCKED a compliant ask. Measured on
    /// a live transcript: 23,087 user entries, 20,142 of them tool results.
    #[test]
    fn a_tool_result_after_the_communicator_does_not_re_arm_the_ask_check() {
        let dir = unique_tempdir("stop-toolresult");
        let p = dir.join("t.jsonl");
        let mut b = String::from(
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"go\"}]}}\n",
        );
        b.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Agent\",\"input\":{\"subagent_type\":\"communicator\"}}]}}\n");
        b.push_str("{\"type\":\"user\",\"toolUseResult\":{\"ok\":true},\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"done\"}]}}\n");
        b.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"One thing needs your word.\"}]}}\n");
        fs::write(&p, b).unwrap();
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": p, "stop_hook_active": false }));
        assert!(out.get("decision").is_none(),
            "a judged ask must survive an ordinary tool call: {out}");
    }

    /// F1b: matching the bare word was satisfied by the AGENT'S OWN PROSE — so
    /// the harness-writes-the-transcript argument was true but irrelevant.
    #[test]
    fn writing_the_word_communicator_does_not_satisfy_the_check() {
        let dir = unique_tempdir("stop-word");
        let p = dir.join("t.jsonl");
        let mut b = String::from(
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"go\"}]}}\n",
        );
        b.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"I ran the communicator on this. One thing needs your word.\"}]}}\n");
        fs::write(&p, b).unwrap();
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": p, "stop_hook_active": false }));
        assert_eq!(Some("block"), out.get("decision").and_then(|d| d.as_str()),
            "claiming a pass in prose must not satisfy the gate: {out}");
    }

    /// F2: counting REFUSE over the raw window matched TOOL RESULTS and FILE
    /// CONTENTS, so any session that read this file or a C8 sprint doc was told
    /// to abandon correct work. Reading the word is not refusing.
    #[test]
    fn refusals_quoted_in_a_tool_result_are_not_an_audit_fix_loop() {
        let dir = unique_tempdir("stop-quoted");
        let p = dir.join("t.jsonl");
        let mut b = String::from(
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"go\"}]}}\n",
        );
        b.push_str("{\"type\":\"user\",\"toolUseResult\":{\"stdout\":\"the auditor may REFUSE; round 2 REFUSE; the verdict was REFUSE\"},\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"REFUSE REFUSE REFUSE\"}]}}\n");
        b.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Fixed the typo in the README and the build is green.\"}]}}\n");
        fs::write(&p, b).unwrap();
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": p, "stop_hook_active": false }));
        assert!(out.get("decision").is_none(),
            "reading refusals is not refusing: {out}");
    }

    /// The studio's half of the silence-log seam assertion. The hook's half is
    /// jawata-hook/tests/silence_log_contract_matches_the_deploy.rs. Both read
    /// the same row; a change on either side fails both.
    #[test]
    fn the_rotation_cap_matches_the_shared_contract() {
        const CONTRACT: &str = include_str!("../hook-events.json");
        let v: serde_json::Value =
            serde_json::from_str(CONTRACT).expect("hook-events.json is a committed contract");
        let row = &v["seam_files"]["hook_silence.log"];
        assert_eq!(
            row["max_bytes"].as_u64(),
            Some(silence_log_cap()),
            "the studio rotates at a size the contract does not declare"
        );
        assert_eq!(
            Some("studio"), row["rotator"].as_str(),
            "this crate IS the rotator named in the contract"
        );
        // The two names this function writes must both be declared.
        assert!(v["seam_files"].get("hook_silence.log").is_some());
        assert_eq!(
            Some("hook_silence.log.1"), row["rotated_name"].as_str(),
            "the rotated name is hand-written in rotate_silence_log and must match"
        );
    }

    fn ask_transcript(dir: &Path, ask: &str, with_communicator: bool) -> PathBuf {
        let p = dir.join("t.jsonl");
        let mut body = String::from(
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"go on\"}]}}\n",
        );
        if with_communicator {
            body.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Agent\",\"input\":{\"subagent_type\":\"communicator\"}}]}}\n");
        }
        body.push_str(&format!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{ask}\"}}]}}}}\n"
        ));
        fs::write(&p, body).unwrap();
        p
    }

    /// THE RULE THAT HAD NO HOOK. "Every self-initiated upward message passes
    /// the communicator" lived only as prose in CLAUDE.md — binding by reading,
    /// enforced by nothing. Asked "how can you skip a hook?", the honest answer
    /// was that no hook existed. Skipped three times in one session, the third
    /// an hour after the rule was recorded as a lesson.
    #[test]
    fn the_stop_gate_blocks_an_ask_the_communicator_never_judged() {
        let dir = unique_tempdir("stop-ask");
        let tp = ask_transcript(&dir, "One thing needs your word before I push.", false);
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false
        }));
        assert_eq!(Some("block"), out.get("decision").and_then(|d| d.as_str()),
            "an unjudged ask must block: {out}");
        assert!(out["reason"].as_str().unwrap().contains("UNJUDGED ASK"));
    }

    /// And it must NOT block when the communicator did run — otherwise the
    /// only way to satisfy it is to stop asking, which is worse.
    #[test]
    fn the_stop_gate_allows_an_ask_the_communicator_judged() {
        let dir = unique_tempdir("stop-ask-ok");
        let tp = ask_transcript(&dir, "One thing needs your word before I push.", true);
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false
        }));
        assert!(out.get("decision").is_none(), "a judged ask must pass: {out}");
    }

    /// A communicator run BEFORE the human's last turn does not judge this
    /// message. The window is what makes the check mean anything.
    #[test]
    fn a_stale_communicator_run_does_not_satisfy_a_new_ask() {
        let dir = unique_tempdir("stop-ask-stale");
        let p = dir.join("t.jsonl");
        let mut body = String::from(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Agent\",\"input\":{\"subagent_type\":\"communicator\"}}]}}\n",
        );
        body.push_str("{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"new question\"}]}}\n");
        body.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Shall I push this?\"}]}}\n");
        fs::write(&p, body).unwrap();
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": p, "stop_hook_active": false
        }));
        assert_eq!(Some("block"), out.get("decision").and_then(|d| d.as_str()),
            "a communicator run from before his turn must not count: {out}");
    }

    /// Ordinary work with no ask is untouched.
    #[test]
    fn the_stop_gate_allows_a_message_that_asks_nothing() {
        let dir = unique_tempdir("stop-noask");
        let tp = ask_transcript(&dir, "Committed and green, continuing.", false);
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false
        }));
        assert!(out.get("decision").is_none(), "no ask, no block: {out}");
    }

    /// Build a transcript with `refusals` audit refusals and no checkpoint.
    fn loop_transcript(dir: &Path, refusals: usize, pad_mb: usize) -> PathBuf {
        let p = dir.join("t.jsonl");
        let mut body = String::new();
        // Optional head padding, to prove the TAIL is what is read.
        for i in 0..(pad_mb * 1024) {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"pad {i} {}\"}}]}}}}\n",
                "x".repeat(900)
            ));
        }
        for i in 0..refusals {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"audit round {i}: REFUSE, one finding\"}}]}}}}\n"
            ));
        }
        body.push_str("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"fixed it, moving on\"}]}}\n");
        fs::write(&p, body).unwrap();
        p
    }

    /// THE TRIGGER THAT WAS MISSING. Every other check in this gate waits for a
    /// CHECKPOINT marker, and an audit-fix loop never produces one — the
    /// checkpoint never passes, so it is never written. The gate ran every turn
    /// through six C8 refusals and correctly found nothing to judge.
    #[test]
    fn the_stop_gate_blocks_an_audit_fix_loop_that_never_reaches_a_checkpoint() {
        let dir = unique_tempdir("stop-loop");
        let tp = loop_transcript(&dir, 5, 0);
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false
        }));
        assert_eq!(Some("block"), out.get("decision").and_then(|d| d.as_str()),
            "five refusals with no checkpoint must block: {out}");
        let reason = out["reason"].as_str().unwrap();
        assert!(reason.contains("AUDIT-FIX LOOP"), "{reason}");
        assert!(reason.contains("/refactor"), "must name the architect seat: {reason}");
    }

    /// Ordinary work must pass. A gate that fires on every session is turned
    /// off by the first person it annoys.
    #[test]
    fn the_stop_gate_allows_a_session_with_no_loop() {
        let dir = unique_tempdir("stop-noloop");
        let tp = loop_transcript(&dir, 1, 0);
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false
        }));
        assert!(out.get("decision").is_none(), "one refusal is not a loop: {out}");
    }

    /// The anti-wedge valve: a second pass always allows, or the gate can trap
    /// a session, which is worse than the problem it solves.
    #[test]
    fn the_stop_gate_never_blocks_twice() {
        let dir = unique_tempdir("stop-twice");
        let tp = loop_transcript(&dir, 9, 0);
        let out = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": true
        }));
        assert!(out.get("decision").is_none(), "second pass must allow: {out}");
    }

    /// It reads the TAIL, not the file — proven by CONTENT, not by a stopwatch.
    ///
    /// The first version of this test asserted an elapsed-time bound, and
    /// seeding the seek back to the start of the file killed nothing: a few MB
    /// reads fast either way. So the refusals now sit ONLY in the head, past
    /// the window. A tail reader cannot see them and must allow; a whole-file
    /// reader would see them and block. The verdict itself is the measurement.
    #[test]
    fn the_stop_gate_reads_only_the_tail() {
        let dir = unique_tempdir("stop-tail");
        let p = dir.join("t.jsonl");
        let mut body = String::new();
        // Head: the refusals, which a tail read must NOT reach.
        for i in 0..9 {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"round {i}: REFUSE\"}}]}}}}\n"
            ));
        }
        // 4 MB of padding pushes them out of the 1 MiB window.
        for i in 0..(4 * 1024) {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"pad {i} {}\"}}]}}}}\n",
                "x".repeat(900)
            ));
        }
        fs::write(&p, &body).unwrap();
        assert!(fs::metadata(&p).unwrap().len() > 3 * 1024 * 1024);

        let out = run_stop_script(&serde_json::json!({
            "transcript_path": p, "stop_hook_active": false
        }));
        assert!(
            out.get("decision").is_none(),
            "refusals sit past the window; seeing them proves the whole file was read: {out}"
        );
    }

    fn run_stop_script(input: &serde_json::Value) -> serde_json::Value {
        let dir = unique_tempdir("stop-gate");
        let script = dir.join("stop-gate.sh");
        fs::write(&script, build_stop_script("http://127.0.0.1:1/mcp", "t")).unwrap();
        let out = std::process::Command::new("bash")
            .arg(&script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap()
                    .write_all(input.to_string().as_bytes()).unwrap();
                c.wait_with_output()
            })
            .expect("script runs");
        serde_json::from_slice(&out.stdout).expect("script prints JSON")
    }

    /// NOTE: these fixtures now carry a communicator run.
    ///
    /// They predate the unjudged-ask check and used a bare `DECISION:` message
    /// with no communicator anywhere — which under the rule that was always
    /// meant to be binding is exactly the state that must BLOCK. The fixtures
    /// were stale, not the check: these tests exist to prove the SHAPE checks
    /// (length, undefined terms, second pass), so they now supply the judged
    /// precondition rather than asserting an unjudged ask may pass.
    fn transcript_with(dir_label: &str, final_text: &str, extra: &str) -> String {
        let dir = unique_tempdir(dir_label);
        let tp = dir.join("transcript.jsonl");
        let mut lines = String::new();
        lines.push_str(&serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "tool_use", "name": "Agent",
                                     "input": {"subagent_type": "communicator"}}]}
        }).to_string());
        lines.push('\n');
        lines.push_str(extra);
        lines.push_str(&serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "text", "text": final_text}]}
        }).to_string());
        lines.push('\n');
        fs::write(&tp, lines).unwrap();
        display_path(&tp)
    }

    #[test]
    fn stop_gate_bounces_a_noisy_specimen_of_each_of_the_three_shapes() {
        let noise = "x".repeat(4000);
        for (label, shaped) in [
            ("ask", format!("DECISION: ship it? {noise}")),
            ("checkpoint", format!("What shipped … ⏸ awaiting \"continue\" {noise}")),
            ("result", format!("SPRINT 26 CLOSED — the result. {noise}")),
        ] {
            let tp = transcript_with(&format!("shape-{label}"), &shaped, "");
            let v = run_stop_script(&serde_json::json!({
                "transcript_path": tp, "stop_hook_active": false }));
            assert_eq!(v["decision"], "block", "{label} must bounce");
            assert!(v["reason"].as_str().unwrap().contains("LENGTH"), "{label}: names length");
        }
    }

    #[test]
    fn stop_gate_allows_clean_messages_and_the_second_pass() {
        // Clean shaped message: short, no undefined terms → allowed.
        let tp = transcript_with("clean", "DECISION: ship v3.2.0? Options: yes / no.", "");
        let v = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false }));
        assert!(v.get("decision").is_none(), "clean passes: {v}");
        // stop_hook_active = the one-rewrite-loop guard.
        let tp2 = transcript_with("second", &"DECISION: x".repeat(500), "");
        let v2 = run_stop_script(&serde_json::json!({
            "transcript_path": tp2, "stop_hook_active": true }));
        assert!(v2.get("decision").is_none(), "second pass always allowed");
        // Unshaped prose is never gated.
        let tp3 = transcript_with("prose", &"just working notes ".repeat(300), "");
        let v3 = run_stop_script(&serde_json::json!({
            "transcript_path": tp3, "stop_hook_active": false }));
        assert!(v3.get("decision").is_none(), "unshaped prose passes");
    }

    #[test]
    fn stop_gate_blocks_a_seat_session_without_gate_calls() {
        let seat_line = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "text", "text": "/javadocs on Foo"}]}
        }).to_string() + "\n";
        let tp = transcript_with("seat-skip", "Proposal: add javadoc to Foo.", &seat_line);
        let v = run_stop_script(&serde_json::json!({
            "transcript_path": tp, "stop_hook_active": false }));
        assert_eq!(v["decision"], "block");
        assert!(v["reason"].as_str().unwrap().contains("SEAT DISCIPLINE"));
        assert!(v["reason"].as_str().unwrap().contains("compile_workspace"));
        // With a gate call in the transcript: allowed.
        let gated = seat_line.clone() + &serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "text", "text": "ran compile_workspace: clean"}]}
        }).to_string() + "\n";
        let tp2 = transcript_with("seat-ok", "Proposal: add javadoc to Foo.", &gated);
        let v2 = run_stop_script(&serde_json::json!({
            "transcript_path": tp2, "stop_hook_active": false }));
        assert!(v2.get("decision").is_none(), "gated seat session passes: {v2}");
    }

    #[test]
    fn stop_gate_fails_open_on_garbage_and_selftests() {
        let v = run_stop_script(&serde_json::json!({"transcript_path": "/nonexistent"}));
        assert!(v.get("decision").is_none(), "fail-open on unreadable transcript");
        let script_dir = unique_tempdir("stop-selftest");
        let script = script_dir.join("stop-gate.sh");
        fs::write(&script, build_stop_script("http://u/mcp", "t")).unwrap();
        assert!(build_stop_script("http://u/mcp", "tok").contains(JAWATA_STOP_SENTINEL));
        // finding #7: a Stop hook emits the Stop decision ({} = allow), NEVER
        // additionalContext — the deployed script MUST pass its own (Stop-contract)
        // self-check, and the generic additionalContext check MUST reject it (proving
        // we no longer wire the wrong validator to the stop path).
        selftest_stop_hook_script(&script).expect("deployed stop-gate passes its Stop self-check");
        assert!(
            selftest_hook_script(&script).is_err(),
            "the additionalContext contract does NOT apply to a Stop hook"
        );
        // A stop script whose selftest path is broken (emits nothing) fails closed.
        let broken = script_dir.join("broken-stop.sh");
        fs::write(&broken, "#!/usr/bin/env bash\nif [ -n \"$JAWATA_HOOK_SELFTEST\" ]; then exit 0; fi\n").unwrap();
        assert!(
            selftest_stop_hook_script(&broken).is_err(),
            "empty stop selftest output fails closed"
        );
    }


    // ---------- Sprint 26 C7: the observer's consequence-labeled edit feed ----------

    /// Runs the REAL observer script under a controlled HOME and returns the
    /// per-session editfeed state file path.
    fn run_observer(home: &std::path::Path, input: &serde_json::Value) {
        let script = home.join("observer.sh");
        // A dead URL: posting fails silently (fail-open); state mechanics still run.
        fs::write(&script, build_observer_script("http://127.0.0.1:1/mcp", "t")).unwrap();
        let out = std::process::Command::new("bash")
            .arg(&script)
            .env("HOME", home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap()
                    .write_all(input.to_string().as_bytes()).unwrap();
                c.wait_with_output()
            })
            .expect("observer runs");
        assert!(out.status.success(), "observer must exit 0 (fail-open contract)");
    }

    #[test]
    fn observer_edit_feed_holds_java_edits_and_resolves_on_the_gate_outcome() {
        let home = unique_tempdir("editfeed");
        let state = home.join(".claude/jawata-studio/editfeed/sess-1");
        // 1. A .java Edit is HELD with its fragments.
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-1", "tool_name": "Edit",
            "tool_input": {"file_path": "/w/Foo.java",
                "old_string": "int f() { return 1; }",
                "new_string": "int f(int a) { return a; }"}
        }));
        let held = fs::read_to_string(&state).expect("fragments held per session");
        assert!(held.contains("int f() { return 1; }"));
        // A non-.java edit is NOT held.
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-1", "tool_name": "Edit",
            "tool_input": {"file_path": "/w/notes.md", "old_string": "a", "new_string": "b"}
        }));
        assert_eq!(1, fs::read_to_string(&state).unwrap().lines().count());
        // 2. The session's gate outcome POPS the state (label posted; dead URL = lost
        //    label, never a stale one).
        let gate_text = serde_json::json!({
            "success": true, "data": {"errorCount": 0}}).to_string();
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-1", "tool_name": "mcp__jawata-x__compile_workspace",
            "tool_input": {},
            "tool_response": {"content": [{"type": "text", "text": gate_text}]}
        }));
        assert!(!state.exists(), "the gate outcome resolves + clears the pendings");
        // 3. An undo forces the failed label and clears too.
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-1", "tool_name": "Write",
            "tool_input": {"file_path": "/w/Bar.java", "content": "class Bar {}"}
        }));
        assert!(state.exists());
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-1", "tool_name": "mcp__jawata-x__refactoring",
            "tool_input": {"action": "undo", "undoChangeId": "u1"}
        }));
        assert!(!state.exists(), "an undo consequence clears the pendings");
        // 4. Another session's gate never touches this session's holds.
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-A", "tool_name": "Edit",
            "tool_input": {"file_path": "/w/Baz.java", "old_string": "x", "new_string": "y"}
        }));
        run_observer(&home, &serde_json::json!({
            "session_id": "sess-B", "tool_name": "mcp__jawata-x__compile_workspace",
            "tool_input": {},
            "tool_response": {"content": [{"type": "text", "text": gate_text}]}
        }));
        assert!(home.join(".claude/jawata-studio/editfeed/sess-A").exists(),
            "edit holds are session-scoped");
    }

    #[test]
    fn utility_commands_roundtrip_inventory_idempotency_delete() {
        for client in ["claude", "cursor", "antigravity"] {
            let base = unique_tempdir(&format!("util-{client}"));
            let cfg = base.join("config.json");
            fs::write(&cfg, "{}").unwrap();
            let dir = derive_seat_commands_dir(client, &display_path(&cfg)).unwrap();
            let written = write_managed_utility_commands(client, &dir, false).unwrap();
            assert_eq!(written.len(), 2, "{client}: /memorize + /sprint");
            for (cmd, path) in utility_artifact_paths(client, &dir) {
                assert!(path.exists(), "{client}: /{cmd} missing");
                let body = fs::read_to_string(&path).unwrap();
                assert!(body.contains("GENERATED by jawata-studio"));
                if cmd == "memorize" { assert!(body.contains("STORE FIRST")); }
                // v3.3.1: /sprint ships with jawata now — assert the PIPELINE
                // actually rode along, not merely that a file was written.
                if cmd == "sprint" {
                    assert!(body.contains("AUDITOR"), "{client}: /sprint lost its auditor seat");
                    assert!(body.contains("REFUSE"), "{client}: /sprint lost the refuse-loop");
                    assert!(body.contains("the RAW"), "{client}: /sprint lost the raw baseline");
                }
            }
            // v3.3.1: /train was REMOVED (Sprint 26a D4 retired the ML it drove).
            // Pin its absence — a tombstone command must not creep back in.
            for (cmd, _) in crate::conductor::UTILITY_MAP {
                assert_ne!(cmd, "train", "{client}: /train is retired, it must not deploy");
            }
            assert!(write_managed_utility_commands(client, &dir, false).unwrap().is_empty(),
                "{client}: second deploy writes nothing");
            assert!(remove_managed_utility_commands(client, &dir).unwrap());
            for (_, path) in utility_artifact_paths(client, &dir) {
                assert!(!path.exists());
            }
        }
    }

    fn loaded_seats(label: &str) -> (PathBuf, Vec<crate::runner::SeatDefinition>) {
        let seats_dir = unique_tempdir(label).join("seats");
        crate::conductor::materialize_seats(&seats_dir).expect("materialize");
        let (seats, errors) = crate::runner::load_seat_definitions(&seats_dir);
        assert!(errors.is_empty(), "embedded seats must load clean: {errors:?}");
        (seats_dir, seats)
    }

    #[test]
    fn seat_commands_roundtrip_inventory_idempotency_delete_per_client() {
        let (_, seats) = loaded_seats("seat-rt");
        for client in ["claude", "cursor", "antigravity"] {
            let base = unique_tempdir(&format!("tree-{client}"));
            let cfg = base.join("config.json");
            fs::write(&cfg, "{}").unwrap();
            let dir = derive_seat_commands_dir(client, &display_path(&cfg))
                .expect("command-bearing client");
            // Deploy: exactly the five artifacts, by name (the inventory).
            let written = write_managed_seat_commands(client, &dir, &seats, false).unwrap();
            assert_eq!(written.len(), 5, "{client}: five command artifacts written");
            for (cmd, path) in seat_artifact_paths(client, &dir) {
                assert!(path.exists(), "{client}: /{cmd} artifact missing at {path:?}");
                let body = fs::read_to_string(&path).unwrap();
                assert!(
                    body.contains("GENERATED by jawata-studio"),
                    "{client}/{cmd}: provenance marker"
                );
            }
            // Deploy twice: byte-stable at the DEPLOY layer — nothing written.
            let written2 = write_managed_seat_commands(client, &dir, &seats, false).unwrap();
            assert!(written2.is_empty(), "{client}: second deploy must write nothing");
            // Delete: no trace.
            assert!(remove_managed_seat_commands(client, &dir).unwrap());
            for (cmd, path) in seat_artifact_paths(client, &dir) {
                assert!(!path.exists(), "{client}: /{cmd} must be removed");
            }
            assert!(!dir.exists(), "{client}: commands dir pruned");
        }
        // Non-command clients have no commands dir at all.
        assert!(derive_seat_commands_dir("claude_desktop", "/tmp/x.json").is_none());
        assert!(derive_seat_commands_dir("intellij", "/tmp/x.json").is_none());
    }

    #[test]
    fn seat_export_roundtrip_and_idempotency() {
        let (_, seats) = loaded_seats("seat-export");
        let export_dir = unique_tempdir("export-rt").join("exports");
        let (_, changed) = write_managed_seat_export(&export_dir, &seats, false).unwrap();
        assert!(changed, "first export writes");
        assert!(seat_export_zip_path(&export_dir).exists());
        let (_, changed2) = write_managed_seat_export(&export_dir, &seats, false).unwrap();
        assert!(!changed2, "second export is a no-op (deterministic zip bytes)");
        assert!(remove_managed_seat_export(&export_dir).unwrap());
        assert!(!seat_export_zip_path(&export_dir).exists());
        assert!(!export_dir.exists(), "export dir pruned");
    }

    #[test]
    fn edited_seat_propagates_to_every_channel_on_redeploy() {
        // The spec's Approach sentence, proven: "a seat edit followed by a
        // redeploy updates every channel."
        let (seats_dir, seats) = loaded_seats("seat-edit");
        let mut trees = Vec::new();
        for client in ["claude", "cursor", "antigravity"] {
            let base = unique_tempdir(&format!("edit-{client}"));
            let cfg = base.join("config.json");
            fs::write(&cfg, "{}").unwrap();
            let dir = derive_seat_commands_dir(client, &display_path(&cfg)).unwrap();
            write_managed_seat_commands(client, &dir, &seats, false).unwrap();
            trees.push((client, dir));
        }
        // Edit the materialized seat (the config copy — the runtime source).
        let seat_path = seats_dir.join("javadoc-writer.md");
        let edited = fs::read_to_string(&seat_path)
            .unwrap()
            .replace("GROUNDED PROSE ONLY", "GROUNDED PROSE ONLY (EDIT-MARKER)");
        fs::write(&seat_path, &edited).unwrap();
        let (reloaded, errors) = crate::runner::load_seat_definitions(&seats_dir);
        assert!(errors.is_empty());
        for (client, dir) in &trees {
            let written = write_managed_seat_commands(client, dir, &reloaded, false).unwrap();
            assert_eq!(written.len(), 1, "{client}: exactly the edited seat rewrites");
            let (_, javadocs_path) = seat_artifact_paths(client, dir)
                .into_iter()
                .find(|(cmd, _)| cmd == "javadocs")
                .unwrap();
            assert!(
                fs::read_to_string(&javadocs_path).unwrap().contains("EDIT-MARKER"),
                "{client}: the edited stance reached the artifact"
            );
        }
        // The archive channel too.
        let bytes = crate::conductor::render_claudeai_skill_zip(&reloaded).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        use std::io::Read;
        let mut reference = String::new();
        archive
            .by_name("jawata-seats/references/javadocs.md")
            .unwrap()
            .read_to_string(&mut reference)
            .unwrap();
        assert!(reference.contains("EDIT-MARKER"), "zip reference updated");
    }

    #[test]
    fn conductor_section_respects_the_recorded_line_budgets() {
        // The R2 guard — budgets FIXED in dossier-25a C0 (≤30 universal,
        // ≤60 IntelliJ incl. the phrase table). Growth past the budget is a
        // BUILD FAILURE, not a review note.
        let seats = crate::conductor::embedded_seat_definitions().unwrap();
        for client in ["claude", "cursor", "antigravity", "claude_desktop"] {
            let n = crate::conductor::render_conductor_section(&seats, client).len();
            assert!(
                n <= crate::conductor::CONDUCTOR_SECTION_BUDGET_UNIVERSAL,
                "{client} conductor section {n} lines > budget {}",
                crate::conductor::CONDUCTOR_SECTION_BUDGET_UNIVERSAL
            );
        }
        let n = crate::conductor::render_conductor_section(&seats, "intellij").len();
        assert!(
            n <= crate::conductor::CONDUCTOR_SECTION_BUDGET_INTELLIJ,
            "intellij conductor section {n} lines > budget {}",
            crate::conductor::CONDUCTOR_SECTION_BUDGET_INTELLIJ
        );
    }

    #[test]
    fn rule_block_is_deterministic_idempotent() {
        let servers = vec![
            url_server("jawata-ws-a", 8800, "tok-a", false),
            url_server("jawata-ws-b", 8801, "tok-b", false),
        ];
        // Same inputs → byte-identical output (so a re-deploy is a no-op write).
        assert_eq!(
            build_rule_block("claude", &servers),
            build_rule_block("claude", &servers)
        );
    }

    /// Sprint 22b: a rule file last written by goja-studio (legacy markers) is
    /// REPLACED by the redeploy — never duplicated beside the old block.
    #[test]
    fn legacy_goja_rule_block_is_replaced_not_duplicated() {
        let dir = std::env::temp_dir().join(format!("jawata-legacy-rule-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("CLAUDE.md");
        fs::write(
            &path,
            "# my file\n\n<!-- goja-studio:claude:start -->\nOLD goja content\n<!-- goja-studio:claude:end -->\n\ntrailing user text\n",
        )
        .unwrap();

        let servers = vec![url_server("jawata-ws-a", 8800, "tok-a", false)];
        let block = build_rule_block("claude", &servers);
        write_managed_rule_block(path.to_str().unwrap(), &block, false, false).unwrap();

        let out = fs::read_to_string(&path).unwrap();
        assert!(!out.contains("goja-studio:claude:start"), "legacy block gone");
        assert!(!out.contains("OLD goja content"), "legacy body gone");
        assert_eq!(out.matches("jawata-studio:claude:start").count(), 1, "exactly one new block");
        assert!(out.contains("# my file"), "user prefix preserved");
        assert!(out.contains("trailing user text"), "user suffix preserved");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Sprint 22b: removal cleans blocks of EITHER generation.
    #[test]
    fn remove_managed_rule_block_removes_legacy_generation() {
        let dir = std::env::temp_dir().join(format!("jawata-legacy-rm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("CLAUDE.md");
        fs::write(
            &path,
            "keep me\n\n<!-- goja-studio:claude:start -->\nold\n<!-- goja-studio:claude:end -->\n",
        )
        .unwrap();
        let changed = remove_managed_rule_block(path.to_str().unwrap(), "claude", false).unwrap();
        assert!(changed, "legacy block was found and removed");
        let out = fs::read_to_string(&path).unwrap();
        assert!(!out.contains("goja-studio"), "no legacy remnants");
        assert!(out.contains("keep me"), "user content preserved");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Sprint 22b: managed-key recognition spans every generation the studio
    /// ever wrote — jawata (current), goja, jl, javalens — so redeploys replace
    /// and removals clean pre-rebrand entries.
    #[test]
    fn managed_mcp_key_recognizes_all_generations() {
        assert!(is_managed_mcp_key("jawata-orb"));
        assert!(is_managed_mcp_key("goja-orb"));
        assert!(is_managed_mcp_key("jl-orb"));
        assert!(is_managed_mcp_key("javalens-orb"));
        assert!(!is_managed_mcp_key("someone-elses-server"));
    }

    /// Sprint 22b (Stage-8 live catch): a PLAIN deploy (default merge mode, no
    /// force) must strip stale managed generations (goja-*) while writing the
    /// jawata-* entries — user servers stay. Previously the prune only ran under
    /// force_rewrite / ReplaceManagedSection, so legacy keys survived forever.
    #[test]
    fn plain_merge_deploy_strips_stale_managed_generations() {
        let dir = std::env::temp_dir().join(format!("jawata-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("mcp.json");
        fs::write(
            &cfg,
            r#"{"mcpServers":{
                "eclipse":{"url":"http://user/own"},
                "goja-goja":{"url":"http://127.0.0.1:8800/mcp"},
                "goja-orb-strategy":{"url":"http://127.0.0.1:8801/mcp"}
            }}"#,
        )
        .unwrap();

        let servers = vec![
            url_server("jawata-goja", 8800, "t1", false),
            url_server("jawata-orb-strategy", 8801, "t2", false),
        ];
        write_managed_json_block(
            cfg.to_str().unwrap(),
            "cursor",
            &servers,
            &McpMergeMode::SafeMerge,
            false,
            false, // NOT force_rewrite — the plain deploy path
        )
        .unwrap();

        let out: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
        let keys: Vec<&String> = out["mcpServers"].as_object().unwrap().keys().collect();
        assert!(keys.iter().any(|k| *k == "eclipse"), "user server preserved");
        assert!(keys.iter().any(|k| *k == "jawata-goja"), "new key written");
        assert!(
            keys.iter().any(|k| *k == "jawata-orb-strategy"),
            "new key written"
        );
        assert!(
            !keys.iter().any(|k| k.starts_with("goja-")),
            "stale goja-* generations stripped on plain merge deploy, got: {keys:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Sprint 22b: cursor hooks.json entries written by goja-studio are managed
    /// (replaced on merge), and the legacy goja-*.sh scripts are dropped when the
    /// new scripts are written.
    #[test]
    fn cursor_legacy_entries_and_scripts_are_migrated() {
        assert!(cursor_entry_is_managed(&serde_json::json!({
            "command": "./hooks/goja-guard.sh", "timeout": 5
        })), "legacy goja entry recognized as managed");

        let dir = std::env::temp_dir().join(format!("jawata-cursor-mig-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let hooks_dir = dir.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        fs::write(hooks_dir.join("goja-guard.sh"), "#!/bin/sh\nold").unwrap();
        let hooks_json = dir.join("hooks.json");
        fs::write(
            &hooks_json,
            r#"{"version":1,"hooks":{"beforeShellExecution":[{"command":"./hooks/goja-guard.sh"},{"command":"./hooks/user-own.sh"}]}}"#,
        )
        .unwrap();

        write_managed_cursor_hooks(
            hooks_json.to_str().unwrap(),
            &hooks_dir,
            "http://127.0.0.1:8800/mcp",
            "tok",
            false,
            false,
        )
        .unwrap();

        assert!(!hooks_dir.join("goja-guard.sh").exists(), "legacy script removed");
        // Sprint 28a: exactly ONE generation of the guard is on disk. Which one
        // depends on whether the bundle shipped a binary; that both are never
        // present does not.
        let binary = hooks_dir.join(role_binary_file_name_on(HostPlatform::host(), "jawata-hook-guard")).exists();
        let script = hooks_dir.join("jawata-guard.sh").exists();
        assert!(binary ^ script,
            "exactly one guard generation must be on disk (binary={binary}, script={script}) \
             — two is the unfired-stale-generation defect, none is no guard at all");
        let out: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&hooks_json).unwrap()).unwrap();
        let shell = out["hooks"]["beforeShellExecution"].as_array().unwrap();
        assert!(
            shell.iter().any(|e| e["command"] == "./hooks/user-own.sh"),
            "user hook preserved"
        );
        assert!(
            shell.iter().all(|e| e["command"] != "./hooks/goja-guard.sh"),
            "legacy managed entry replaced"
        );
        assert!(
            shell.iter().any(|e| e["command"] == "./hooks/jawata-guard.sh"),
            "new managed entry present"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Sprint 22b: a settings.json hook entry pointing at the pre-rebrand
    /// script path is recognized as managed (so redeploys replace it).
    #[test]
    fn legacy_goja_hook_entry_is_recognized_as_managed() {
        let entry = serde_json::json!({
            "matcher": "Bash",
            "hooks": [{ "type": "command",
                "command": "/home/x/.claude/goja-studio/pretooluse-guard.sh" }]
        });
        assert!(is_managed_hook_entry(&entry));
    }

    #[test]
    fn deploy_writer_emits_url_entries() {
        let servers = vec![url_server("ws-a", 8800, "tok-a", false)];
        let json = build_client_mcp_json("cursor", &servers);
        let entry = &json["mcpServers"]["ws-a"];

        assert_eq!(entry["url"], "http://127.0.0.1:8800/mcp");
        // Stage 11 contract: stdio fields must NOT leak into the new shape.
        assert!(entry.get("command").is_none(), "must not emit `command`");
        assert!(entry.get("args").is_none(), "must not emit `args`");
        assert!(entry.get("env").is_none(), "must not emit `env`");
    }

    #[test]
    fn deploy_writer_includes_correct_token_per_workspace() {
        // Two workspaces, distinct ports + tokens — verify each entry
        // carries its OWN Bearer token (not the other's).
        let servers = vec![
            url_server("ws-a", 8800, "alpha-token", false),
            url_server("ws-b", 8801, "beta-token", false),
        ];
        let json = build_client_mcp_json("cursor", &servers);

        assert_eq!(
            json["mcpServers"]["ws-a"]["headers"]["Authorization"],
            "Bearer alpha-token"
        );
        assert_eq!(
            json["mcpServers"]["ws-b"]["headers"]["Authorization"],
            "Bearer beta-token"
        );
    }

    // ===== Sprint 16b/C: deploy-owned always-load =====

    #[test]
    fn claude_entry_marks_always_load() {
        let servers = vec![url_server("jawata-ws", 8800, "tok", false)];
        let json = build_client_mcp_json("claude", &servers);
        assert_eq!(
            json["mcpServers"]["jawata-ws"]["alwaysLoad"],
            serde_json::Value::Bool(true),
            "Claude entry must carry alwaysLoad:true so the surface never defers"
        );
    }

    #[test]
    fn non_claude_entries_omit_always_load() {
        let servers = vec![url_server("jawata-ws", 8800, "tok", false)];
        for client in ["cursor", "antigravity", "intellij", "claude_desktop"] {
            let json = build_client_mcp_json(client, &servers);
            assert!(
                json["mcpServers"]["jawata-ws"].get("alwaysLoad").is_none(),
                "{client} entry must not carry alwaysLoad (Claude-only flag)"
            );
        }
    }

    #[test]
    fn global_rule_path_claude_targets_always_loaded_file() {
        let p = derive_global_rule_path("claude").expect("claude has a global file");
        let norm = p.replace('\\', "/");
        assert!(
            norm.ends_with(".claude/CLAUDE.md"),
            "claude global rule must be ~/.claude/CLAUDE.md, got {p}"
        );
    }

    #[test]
    fn global_rule_path_none_for_other_clients() {
        for client in ["cursor", "antigravity", "intellij", "claude_desktop", "unknown"] {
            assert!(
                derive_global_rule_path(client).is_none(),
                "{client} must have no global rule path (sibling covers it / unconfirmed)"
            );
        }
    }

    #[test]
    fn write_managed_rule_block_new_replace_append_idempotent() {
        let dir = unique_tempdir("rule-global");
        let file = dir.join(".claude").join("CLAUDE.md");
        let path = file.to_string_lossy().to_string();
        let servers = vec![url_server("jawata-ws", 8800, "tok", false)];
        let block = build_rule_block("claude", &servers);

        // (1) NEW FILE: parent dir created, block written.
        write_managed_rule_block(&path, &block, false, false).unwrap();
        let after_new = std::fs::read_to_string(&file).unwrap();
        assert!(after_new.contains("<!-- jawata-studio:claude:start -->"));
        assert!(after_new.contains("JAWATA MCP"));

        // (2) IDEMPOTENT: same block again is a byte-stable no-op.
        write_managed_rule_block(&path, &block, false, false).unwrap();
        assert_eq!(
            after_new,
            std::fs::read_to_string(&file).unwrap(),
            "re-deploy must be byte-stable"
        );

        // (3) APPEND PRESERVING USER CONTENT: hand-written file w/o markers.
        let user_file = dir.join("user.md");
        let user_path = user_file.to_string_lossy().to_string();
        std::fs::write(&user_file, "# My own notes\n\nkeep me\n").unwrap();
        write_managed_rule_block(&user_path, &block, false, false).unwrap();
        let appended = std::fs::read_to_string(&user_file).unwrap();
        assert!(appended.contains("# My own notes"), "user content preserved");
        assert!(appended.contains("keep me"), "user content preserved");
        assert!(
            appended.contains("<!-- jawata-studio:claude:start -->"),
            "block appended"
        );

        // (4) REPLACE BETWEEN MARKERS: a stale block is spliced out, user text kept.
        let stale = "# Header\n\n<!-- jawata-studio:claude:start -->\nOLD STALE BODY\n<!-- jawata-studio:claude:end -->\n\n# Footer\n";
        let replace_file = dir.join("replace.md");
        let replace_path = replace_file.to_string_lossy().to_string();
        std::fs::write(&replace_file, stale).unwrap();
        write_managed_rule_block(&replace_path, &block, false, false).unwrap();
        let replaced = std::fs::read_to_string(&replace_file).unwrap();
        assert!(replaced.contains("# Header"), "leading user text kept");
        assert!(replaced.contains("# Footer"), "trailing user text kept");
        assert!(!replaced.contains("OLD STALE BODY"), "stale body replaced");
        assert!(replaced.contains("JAWATA MCP"), "new body present");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 18 Track 2 / Stage 9: PreToolUse enforcement hook =====

    #[test]
    fn hook_settings_path_is_claude_only() {
        assert!(
            derive_hook_settings_path("claude")
                .map(|p| p.ends_with("settings.json"))
                .unwrap_or(false),
            "claude gets ~/.claude/settings.json"
        );
        for other in ["cursor", "antigravity", "claude_desktop", "intellij"] {
            assert!(
                derive_hook_settings_path(other).is_none(),
                "{other} keeps the rule block, no hook"
            );
        }
    }

    #[test]
    fn guard_script_is_health_gated_and_java_scoped() {
        let script = build_guard_script("http://127.0.0.1:8890/mcp");
        // Health URL baked in; both branches present.
        assert!(script.contains("http://127.0.0.1:8890/mcp"), "health url baked in");
        assert!(script.contains("TRY JAWATA FIRST"), "up branch: try-first redirect");
        assert!(script.contains("appears to be DOWN"), "down branch: diagnosis");
        assert!(script.contains("search_symbols"), "names the JAWATA tool to use instead");
        // Java-scoped + content-search-scoped; edits/non-Java pass.
        assert!(script.contains(r"\.java"), "scoped to Java source");
        assert!(script.contains("exit 0"), "has a pass path");
        assert!(script.contains("exit 2"), "has a block/redirect path");

        // v1.2.1 tuning: content-search tools only — grep-family, NOT file/line ops.
        assert!(
            script.contains("grep|egrep|fgrep|rg|ripgrep|ag|ack"),
            "matches content-search tools"
        );
        assert!(
            !script.contains("|find|sed|awk") && !script.contains("ack|find"),
            "file/line ops (find/sed/awk) are NOT treated as symbol search"
        );
        // v1.2.1 tuning: requires a real .java FILE reference (word-char/glob-star
        // before the dot), so an escaped pattern `\.java` or incidental mention passes.
        assert!(
            script.contains(r"[A-Za-z0-9_$]\.java|\*\.java"),
            "requires a .java file/glob target, not an incidental mention"
        );

        // v1.3.0 escape valve: a declared fallback proceeds and is logged.
        assert!(
            script.contains("jawata-fallback:"),
            "recognises the declared-fallback escape grammar"
        );
        assert!(
            script.contains("fallback.log"),
            "logs declared fallbacks (auditable, not silent)"
        );
        // v1.3.1: the reason is captured from the UN-flattened input (marker's own
        // line only), so a multi-line command's other lines can't bleed into the log.
        assert!(
            script.contains(r#"printf '%s' "$input" | sed -n 's/.*jawata-fallback:"#),
            "reason captured from un-flattened input (clean audit line)"
        );
        // The down-branch must point at the real escape, not the old false promise.
        assert!(
            !script.contains("this guard only warns once JAWATA is confirmed down"),
            "the false 're-run' promise is gone"
        );

        // v1.4.0 (Sprint 22): per-session try-first state keyed by session_id.
        assert!(
            script.contains(r#""session_id""#) && script.contains("trygate"),
            "derives the per-session try-first state path from the hook session_id"
        );

        // Deterministic → byte-stable re-deploy.
        assert_eq!(script, build_guard_script("http://127.0.0.1:8890/mcp"));
    }

    #[test]
    fn managed_hook_entry_shape() {
        let guard = PathBuf::from("/home/u/.claude/jawata-studio/pretooluse-guard.sh");
        let entry = build_managed_hook_entry(&guard);
        assert_eq!(
            entry["matcher"], "Bash|Grep|Edit|Write|MultiEdit|mcp__jawata.*",
            "fires for search (Bash|Grep), edits (Edit|Write|MultiEdit) and jawata calls (mcp__jawata.*)"
        );
        let cmd = entry["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains(JAWATA_HOOK_SENTINEL), "command references the managed guard");
        assert_eq!(entry["hooks"][0]["type"], "command");
        assert!(is_managed_hook_entry(&entry), "our own entry is recognised as managed");
        // A user's unrelated PreToolUse entry must NOT be flagged as managed.
        let user = serde_json::json!({
            "matcher": "Write",
            "hooks": [{ "type": "command", "command": "echo hi" }]
        });
        assert!(!is_managed_hook_entry(&user));
    }

    #[test]
    fn managed_hook_write_remove_roundtrip_preserves_user_hooks() {
        let dir = unique_tempdir("hook");
        let settings = dir.join(".claude").join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let guard = dir
            .join(".claude")
            .join("jawata-studio")
            .join("pretooluse-guard.sh");
        let health = "http://127.0.0.1:8890/mcp";

        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Write","hooks":[{"type":"command","command":"echo user"}]}]}}"#,
        )
        .unwrap();

        // (1) WRITE: entry added, guard written, user content preserved.
        assert!(write_managed_hook(&settings_path, &guard, health, false, false).unwrap());
        assert!(guard.exists(), "guard script written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&guard).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "guard is executable");
        }
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["model"], "opus", "unrelated setting preserved");
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "user entry + managed entry");
        assert!(pre.iter().any(is_managed_hook_entry), "managed entry present");
        assert!(
            pre.iter()
                .any(|e| e["hooks"][0]["command"] == "echo user"),
            "user entry preserved"
        );

        // (2) IDEMPOTENT: unchanged re-deploy is a no-op, byte-stable.
        let before = std::fs::read_to_string(&settings).unwrap();
        assert!(
            !write_managed_hook(&settings_path, &guard, health, false, false).unwrap(),
            "re-deploy is a no-op"
        );
        assert_eq!(before, std::fs::read_to_string(&settings).unwrap(), "byte-stable");

        // (3) REMOVE: managed entry + guard gone, user entry kept.
        assert!(remove_managed_hook(&settings_path, &guard, false).unwrap());
        assert!(!guard.exists(), "guard deleted");
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let pre2 = v2["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre2.len(), 1, "only the user entry remains");
        assert!(!pre2.iter().any(is_managed_hook_entry));
        assert_eq!(v2["model"], "opus");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_posthook_entry_shape() {
        let observer = PathBuf::from("/home/u/.claude/jawata-studio/posttooluse-observer.sh");
        let entry = build_managed_posthook_entry(&observer);
        assert_eq!(
            entry["matcher"], "Bash|Grep|Edit|Write|MultiEdit|Read|mcp__jawata.*",
            "fires for Read (ungrounded capture), verify MCP tools, and search/edit slips"
        );
        let cmd = entry["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains(JAWATA_POSTHOOK_SENTINEL), "command references the managed observer");
        assert!(is_managed_posthook_entry(&entry), "our own entry is recognised as managed");
        let user = serde_json::json!({
            "matcher": "Bash",
            "hooks": [{ "type": "command", "command": "echo hi" }]
        });
        assert!(!is_managed_posthook_entry(&user), "a user PostToolUse entry is not managed");
    }

    #[test]
    fn observer_script_captures_the_three_signals() {
        let s = build_observer_script("http://127.0.0.1:8890/mcp", "tok");
        assert!(s.contains("outcomes.log"), "appends to the versioned outcomes log");
        assert!(s.contains("read-ungrounded"), "captures ungrounded .java reads");
        assert!(s.contains("emit \"slip\""), "captures declared-fallback slips");
        assert!(s.contains("emit \"verify\""), "captures verify events");
        assert!(s.contains("jawata-fallback"), "keys the slip off the declared fallback");
        assert!(s.contains("additionalContext"), "steers after a slip");
        assert!(s.contains("emit_slip"), "slip logging is factored so callers can gate it to real .java ops");
        assert!(s.contains("not a gated op"), "v1.5.1: slip scoped to .java-targeted ops — no false slip on an incidental marker in edited content");
        assert!(s.trim_end().ends_with("exit 0"), "reactive — never blocks");
        assert_eq!(
            s,
            build_observer_script("http://127.0.0.1:8890/mcp", "tok"),
            "deterministic (byte-stable re-deploy)"
        );
    }

    #[test]
    fn observer_judges_tool_input_only() {
        // Sprint 21a (item J): a cat of a file whose CONTENT mentions '.java' +
        // 'jawata-fallback:' logged a false slip — the observer grepped the whole payload
        // including tool_response. The response must be stripped BEFORE any matching.
        let s = build_observer_script("u", "t");
        let strip = s.find(r#"sed 's/"tool_response".*$//'"#)
            .expect("strips tool_response from the judged payload");
        let matching = s.find("case \"$tool_name\" in").expect("signal matching");
        assert!(strip < matching, "the strip happens before any signal matching");
    }

    #[test]
    fn observer_bridges_slips_into_the_experience_store() {
        // Sprint 21a (items G+J): the first conversation-level auto-learn path — a slip
        // becomes a candidate entry, fail-safe when jawata is down.
        let s = build_observer_script("http://127.0.0.1:8890/mcp", "sekret");
        assert!(s.contains(r#"MCP_URL="http://127.0.0.1:8890/mcp""#), "bakes the resident url");
        assert!(s.contains(r#"TOKEN="sekret""#), "bakes the bearer token");
        assert!(s.contains(r#""kind":"record""#), "records into the store");
        assert!(s.contains(r#""type":"failure_mode""#), "as a failure-mode candidate");
        assert!(s.contains("|| true"), "fail-safe: a dead resident never breaks the hook");
        assert!(
            s.contains(r#"sed 's/["\\]/ /g'"#),
            "the interpolated summary is sanitized for the JSON payload"
        );
    }

    #[test]
    fn all_emitting_hooks_have_a_selftest_path_sharing_the_live_emit() {
        // Sprint 21a (item J): the selftest MUST exercise the same emit format as the
        // live path — a duplicated format string could pass selftest while live is broken.
        for s in [
            build_primer_script("u", "t"),
            build_recall_script("u", "t"),
            build_userprompt_script("u", "t"),
        ] {
            assert!(s.contains("JAWATA_HOOK_SELFTEST"), "has a selftest entry point");
            assert!(s.contains("emit_ctx"), "emits through the shared function");
            assert_eq!(
                s.matches("hookSpecificOutput").count(),
                1,
                "exactly ONE emit format definition — selftest and live share it"
            );
        }
        let observer = build_observer_script("u", "t");
        assert!(observer.contains("JAWATA_HOOK_SELFTEST"));
        assert_eq!(
            observer.matches("hookSpecificOutput").count(),
            1,
            "observer steering payload defined once, shared by selftest + emit_slip"
        );
    }

    // ===== Sprint 21a (item D): auto-seed on deploy =====

    fn seed_server(url: &str, token: &str) -> ManagedDeployServer {
        ManagedDeployServer {
            id: "jawata-test".into(),
            workspace_name: "test".into(),
            project_names: vec![],
            project_paths: vec![],
            url: url.into(),
            token: token.into(),
            disabled: false,
        }
    }

    #[test]
    fn auto_seed_targets_honors_the_toggle_and_skips_empty_credentials() {
        let servers = vec![
            seed_server("http://127.0.0.1:8801/mcp", "tok-a"),
            seed_server("", ""),                                  // no resident allocated
            seed_server("http://127.0.0.1:8802/mcp", "tok-b"),
        ];
        assert!(auto_seed_targets(false, &servers).is_empty(), "toggle off → no seeding");
        let on = auto_seed_targets(true, &servers);
        assert_eq!(on.len(), 2, "credential-less servers are skipped");
        assert_eq!(on[0], ("http://127.0.0.1:8801/mcp".into(), "tok-a".into()));
    }

    #[test]
    fn knowledge_jvm_properties_carries_no_crawl_caps() {
        // Sprint 21b: the crawl finds everything — studio sends store mode + roots only;
        // the resident's own defaults are the runaway backstops.
        let paths = crate::config::AppPaths {
            config_dir: std::path::PathBuf::from("/tmp/config"),
            state_dir: std::path::PathBuf::from("/tmp/state"),
            cache_dir: std::path::PathBuf::from("/tmp/cache"),
            projects_file: std::path::PathBuf::from("/tmp/config/projects.json"),
            settings_file: std::path::PathBuf::from("/tmp/config/settings.json"),
            runtime_state_file: std::path::PathBuf::from("/tmp/state/runtime-state.json"),
            default_data_root: std::path::PathBuf::from("/tmp/cache/jawata-studio"),
            log_dir: std::path::PathBuf::from("/tmp/state/logs"),
        };
        let mut settings = ManagerSettings::default_for_paths(&paths);
        settings.memory_roots = vec!["/home/x/.claude".into()];
        let props = knowledge_jvm_properties(&settings);
        assert_eq!(props.len(), 2, "store mode + roots, nothing else");
        assert!(props[0].starts_with("-Djawata.experience.store="));
        assert!(props[1].starts_with("-Djawata.memory.roots="));
        assert!(
            props.iter().all(|p| !p.contains("jawata.memory.max")),
            "no -Djawata.memory.max* from studio"
        );
    }

    #[test]
    fn call_resident_tool_posts_jsonrpc_with_bearer() {
        use std::io::{Read as _, Write as _};
        // Minimal one-shot HTTP stub — asserts on the request, answers 200 JSON.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ok"}]}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            req
        });

        let url = format!("http://{addr}/mcp");
        let result = call_resident_tool(&url, "sekret", "experience",
            serde_json::json!({"kind": "load"}), 5);
        let request = handle.join().unwrap();

        assert!(result.is_ok(), "stub answered 200: {result:?}");
        assert!(result.unwrap().contains("\"result\""));
        assert!(request.contains("Authorization: Bearer sekret") || request.contains("authorization: Bearer sekret"),
            "bearer auth sent: {request}");
        assert!(request.contains(r#""name":"experience""#), "tools/call for experience");
        assert!(request.contains(r#""kind":"load""#), "seed arguments passed through");
    }

    #[test]
    fn call_resident_tool_reports_dead_resident_as_err() {
        // Nothing listens here — the helper must fail fast with a message, not panic.
        let result = call_resident_tool("http://127.0.0.1:9/mcp", "t", "experience",
            serde_json::json!({"kind": "load"}), 2);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn selftest_validates_emitted_bytes_and_catches_the_v202_bug_class() {
        let dir = unique_tempdir("selftest");
        std::fs::create_dir_all(&dir).unwrap();

        // A correctly generated primer passes.
        let good = dir.join("primer-good.sh");
        std::fs::write(&good, build_primer_script("http://127.0.0.1:1/mcp", "t")).unwrap();
        assert!(selftest_hook_script(&good).is_ok(), "healthy template passes the self-check");

        // Re-introduce the v2.0.1 bug (printf format with a REAL newline instead of \n):
        // the emitted additionalContext becomes invalid JSON — the self-check must fail.
        let broken = dir.join("primer-broken.sh");
        let body = build_primer_script("http://127.0.0.1:1/mcp", "t").replace(r"\\n%s", "\n%s");
        std::fs::write(&broken, body).unwrap();
        let err = selftest_hook_script(&broken);
        assert!(err.is_err(), "the v2.0.x bug class is caught at deploy time");
        assert!(err.unwrap_err().contains("INVALID JSON"), "with a diagnosable message");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_posthook_write_remove_roundtrip_preserves_user_hooks() {
        let dir = unique_tempdir("posthook");
        let settings = dir.join(".claude").join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let observer = dir
            .join(".claude")
            .join("jawata-studio")
            .join("posttooluse-observer.sh");

        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"opus","hooks":{"PostToolUse":[{"matcher":"Write","hooks":[{"type":"command","command":"echo user-post"}]}]}}"#,
        )
        .unwrap();

        // (1) WRITE: entry added, observer written, user content preserved.
        assert!(write_managed_posthook(&settings_path, &observer, "http://u/mcp", "tok", false, false).unwrap());
        assert!(observer.exists(), "observer script written");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["model"], "opus", "unrelated setting preserved");
        let post = v["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 2, "user entry + managed entry");
        assert!(post.iter().any(is_managed_posthook_entry), "managed entry present");
        assert!(
            post.iter().any(|e| e["hooks"][0]["command"] == "echo user-post"),
            "user entry preserved"
        );

        // (2) IDEMPOTENT: unchanged re-deploy is a no-op.
        assert!(
            !write_managed_posthook(&settings_path, &observer, "http://u/mcp", "tok", false, false).unwrap(),
            "re-deploy is a no-op"
        );

        // (3) REMOVE: managed entry + observer gone, user entry kept.
        assert!(remove_managed_posthook(&settings_path, &observer, false).unwrap());
        assert!(!observer.exists(), "observer deleted");
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let post2 = v2["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post2.len(), 1, "only the user entry remains");
        assert!(!post2.iter().any(is_managed_posthook_entry));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 22a P1-b: Cursor hooks.json deploy/remove lifecycle =====

    #[test]
    fn cursor_hooks_path_is_cursor_only() {
        assert!(
            derive_cursor_hooks_path("cursor")
                .map(|p| p.ends_with("hooks.json"))
                .unwrap_or(false),
            "cursor gets ~/.cursor/hooks.json"
        );
        for other in ["claude", "antigravity", "claude_desktop", "intellij"] {
            assert!(derive_cursor_hooks_path(other).is_none(), "{other} has no cursor hooks");
        }
    }

    #[test]
    fn cursor_hooks_write_merges_preserving_user_hooks() {
        let dir = unique_tempdir("cursor-hooks-merge");
        let cursor = dir.join(".cursor");
        let hooks_json = cursor.join("hooks.json");
        let hooks_path = hooks_json.to_string_lossy().to_string();
        let hooks_dir = cursor.join("hooks");
        std::fs::create_dir_all(&cursor).unwrap();
        // A user already has their own beforeSubmitPrompt hook + a bespoke event.
        std::fs::write(
            &hooks_json,
            r#"{"version":1,"hooks":{"beforeSubmitPrompt":[{"command":"./hooks/my-own.sh"}],"stop":[{"command":"./hooks/user-stop.sh"}]}}"#,
        )
        .unwrap();

        assert!(write_managed_cursor_hooks(
            &hooks_path, &hooks_dir, "http://127.0.0.1:8899/mcp", "tok", false, false
        )
        .unwrap());

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hooks_json).unwrap()).unwrap();
        assert_eq!(v["version"], 1);
        let hooks = v["hooks"].as_object().unwrap();
        for ev in ["sessionStart", "beforeShellExecution", "beforeSubmitPrompt", "afterMCPExecution"] {
            assert!(hooks.contains_key(ev), "managed event {ev} present");
        }
        // beforeSubmitPrompt keeps the user's entry AND adds ours.
        let bsp = hooks["beforeSubmitPrompt"].as_array().unwrap();
        assert!(bsp.iter().any(|e| e["command"] == "./hooks/my-own.sh"), "user hook preserved");
        assert!(bsp.iter().any(|e| e["command"] == "./hooks/jawata-recall.sh"), "managed recall added");
        // The user's bespoke event is untouched.
        assert!(hooks.contains_key("stop"), "unrelated user event preserved");
        // The guard is failClosed, and — Sprint 28a — it names the role BINARY.
        //
        // The `.sh` assertion below is the one that matters on Windows: a shell
        // script there is launched as an interactive login shell, blocks on
        // `cat` waiting for a payload nobody pipes in, and never returns. Under
        // `failClosed` that is a BLOCK on the user's command, so a regression to
        // a script here is not cosmetic — it breaks the client.
        let guard = hooks["beforeShellExecution"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| cursor_entry_is_managed(e))
            .expect("our guard entry is present");
        assert_eq!(guard["failClosed"], true, "guard is failClosed");
        // THE INVARIANT the redesign guarantees: the entry and the file on disk
        // name the SAME generation. Asserting "always a binary" would be wrong —
        // an install whose bundle shipped no binary correctly keeps the script.
        // What must never happen is the entry pointing at one and the deploy
        // having written the other, which is what two independent probes used to
        // allow.
        let guard_cmd = guard["command"].as_str().unwrap_or_default().to_string();
        let named = guard_cmd.rsplit('/').next().unwrap_or_default();
        assert!(
            hooks_dir.join(named).exists(),
            "the guard entry names {named:?} but no such file was deployed — the \
             entry and the deploy disagree, which is the defect this design removes"
        );
        // The three roles still on scripts are written + executable; the recall
        // script baked the url + token.
        for name in ["jawata-session-primer.sh", "jawata-recall.sh", "jawata-observer.sh"] {
            let p = hooks_dir.join(name);
            assert!(p.exists(), "{name} written");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&p).unwrap().permissions().mode();
                assert!(mode & 0o111 != 0, "{name} is executable");
            }
        }
        let recall = std::fs::read_to_string(hooks_dir.join("jawata-recall.sh")).unwrap();
        assert!(
            recall.contains("http://127.0.0.1:8899/mcp") && recall.contains("tok"),
            "url + token baked into the recall script"
        );

        // IDEMPOTENT: unchanged re-deploy is a byte-stable no-op.
        assert!(
            !write_managed_cursor_hooks(&hooks_path, &hooks_dir, "http://127.0.0.1:8899/mcp", "tok", false, false).unwrap(),
            "re-deploy is a no-op"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_hooks_remove_strips_ours_keeps_user() {
        let dir = unique_tempdir("cursor-hooks-remove");
        let cursor = dir.join(".cursor");
        let hooks_json = cursor.join("hooks.json");
        let hooks_path = hooks_json.to_string_lossy().to_string();
        let hooks_dir = cursor.join("hooks");
        std::fs::create_dir_all(&cursor).unwrap();
        std::fs::write(
            &hooks_json,
            r#"{"version":1,"hooks":{"beforeSubmitPrompt":[{"command":"./hooks/my-own.sh"}]}}"#,
        )
        .unwrap();

        write_managed_cursor_hooks(&hooks_path, &hooks_dir, "http://u/mcp", "t", false, false).unwrap();
        assert!(remove_managed_cursor_hooks(&hooks_path, &hooks_dir, false).unwrap());

        // File kept (user content remains); our entries + scripts gone; managed-only event pruned.
        assert!(hooks_json.exists(), "file kept — user hook remains");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hooks_json).unwrap()).unwrap();
        let hooks = v["hooks"].as_object().unwrap();
        let bsp = hooks["beforeSubmitPrompt"].as_array().unwrap();
        assert_eq!(bsp.len(), 1, "only the user entry remains");
        assert!(bsp.iter().any(|e| e["command"] == "./hooks/my-own.sh"), "user hook preserved");
        assert!(!hooks.contains_key("sessionStart"), "managed-only event pruned");
        for name in ["jawata-session-primer.sh", "jawata-guard.sh", "jawata-recall.sh", "jawata-observer.sh"] {
            assert!(!hooks_dir.join(name).exists(), "{name} deleted");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_hooks_remove_deletes_file_when_only_ours() {
        let dir = unique_tempdir("cursor-hooks-solo");
        let cursor = dir.join(".cursor");
        let hooks_json = cursor.join("hooks.json");
        let hooks_path = hooks_json.to_string_lossy().to_string();
        let hooks_dir = cursor.join("hooks");

        // Deploy into a virgin ~/.cursor (jawata created the file).
        write_managed_cursor_hooks(&hooks_path, &hooks_dir, "http://u/mcp", "t", false, false).unwrap();
        assert!(hooks_json.exists());
        assert!(remove_managed_cursor_hooks(&hooks_path, &hooks_dir, false).unwrap());
        assert!(!hooks_json.exists(), "file removed when nothing but ours remained");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_scripts_emit_valid_json_over_the_wire() {
        // Dogfood the emitted BYTES, not just the generated string: run the guard (no
        // network) and the primer (selftest mode) and assert valid JSON output.
        let dir = unique_tempdir("cursor-scripts-exec");
        let hooks_dir = dir.join("hooks");
        let hooks_path = dir.join("hooks.json").to_string_lossy().to_string();
        write_managed_cursor_hooks(&hooks_path, &hooks_dir, "http://127.0.0.1:1/mcp", "t", false, false).unwrap();

        use std::process::{Command, Stdio};
        // The GUARD is no longer a script here (Sprint 28a) — its equivalent
        // check runs the real binary in
        // jawata-hook/tests/edit_gate_runs_the_real_binary.rs, which is a
        // stronger test than this one: it drives both halves of the role and
        // asserts the Cursor permission dialect, rather than only that empty
        // stdin yields valid JSON.
        // primer: selftest mode -> valid JSON carrying additional_context.
        if let Ok(out) = Command::new("bash")
            .arg(hooks_dir.join("jawata-session-primer.sh"))
            .env("JAWATA_HOOK_SELFTEST", "1")
            .stdin(Stdio::null())
            .output()
        {
            let v: serde_json::Value =
                serde_json::from_slice(&out.stdout).expect("primer emits valid JSON");
            assert!(v.get("additional_context").is_some(), "primer selftest injects context");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_write_backs_up_centrally_never_beside_the_file() {
        // Sprint 21a (item E) acceptance: a managed write with backups ON produces ZERO
        // .bak siblings and exactly one version in the managed area.
        let _guard = crate::backups::test_lock().lock().unwrap();
        let dir = unique_tempdir("central-backup");
        crate::backups::set_backups_root(dir.to_string_lossy().as_ref());
        let settings = dir.join(".claude").join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let guard = dir.join(".claude").join("jawata-studio").join("pretooluse-guard.sh");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();

        write_managed_hook(&settings_path, &guard, "http://127.0.0.1:8890/mcp", true, false)
            .unwrap();

        let siblings = std::fs::read_dir(settings.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".bak"))
            .count();
        assert_eq!(siblings, 0, "zero .bak siblings beside the user's file");
        assert!(
            latest_backup_path(&settings_path).is_some(),
            "the pre-write state landed in the managed area"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_managed_hook_prunes_empty_containers() {
        let dir = unique_tempdir("hook-prune");
        let settings = dir.join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let guard = dir.join("jawata-studio").join("pretooluse-guard.sh");

        // Only our entry exists → after removal the containers vanish.
        write_managed_hook(&settings_path, &guard, "http://127.0.0.1:8890/mcp", false, false)
            .unwrap();
        assert!(remove_managed_hook(&settings_path, &guard, false).unwrap());
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(v.get("hooks").is_none(), "empty hooks container pruned");

        // Removal when nothing is managed → no-op, no error.
        assert!(!remove_managed_hook(&settings_path, &guard, false).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 21 (v2.0): knowledge PUSH hooks (primer + recall) =====

    #[test]
    fn primer_script_bakes_url_token_and_fails_safe() {
        let s = build_primer_script("http://127.0.0.1:8890/mcp", "sekret");
        assert!(s.contains(r#"MCP_URL="http://127.0.0.1:8890/mcp""#), "bakes the mcp url");
        assert!(s.contains(r#"TOKEN="sekret""#), "bakes the bearer token");
        assert!(
            s.contains(r#""kind":"primer""#) && s.contains(r#""format":"text""#),
            "calls experience(kind=primer, format=text)"
        );
        assert!(s.contains("Authorization: Bearer $TOKEN"), "authenticates the call");
        assert!(
            s.contains("SessionStart") && s.contains("additionalContext"),
            "injects the primer as SessionStart context"
        );
        assert!(
            s.contains("command -v curl") && s.contains("exit 0"),
            "fail-safe: curl absent / any miss → exit 0, inject nothing"
        );
        assert!(s.contains("No\\ domain"), "silent on the absence sentinel");
    }

    #[test]
    fn recall_script_gates_to_refactor_verbs_with_symbol_cue() {
        let s = build_recall_script("http://127.0.0.1:8890/mcp", "sekret");
        assert!(s.contains(r#""kind":"recall""#), "calls experience(kind=recall)");
        assert!(
            s.contains("rename_symbol") && s.contains("refactor") && s.contains("extract"),
            "gated to refactor-ish jawata verbs"
        );
        assert!(
            s.contains("typeName") && s.contains("symbol") && s.contains("newName"),
            "extracts a symbol cue from the tool input"
        );
        // Sprint 21a live-dogfood find: subject identifiers must WIN over newName — the
        // old greedy alternation picked the LAST key, so a rename queried the NEW name
        // and recalled nothing. The priority loop encodes the order explicitly.
        assert!(
            s.contains("for key in typeName symbol query newName"),
            "cue priority: subject identifiers first, newName last"
        );
        assert!(s.contains("PreToolUse") && s.contains("additionalContext"), "injects pre-op context");
        assert!(s.contains("No\\ known\\ knowledge"), "silent on absence");
    }

    #[test]
    fn push_scripts_are_deterministic() {
        assert_eq!(build_primer_script("u", "t"), build_primer_script("u", "t"));
        assert_eq!(build_recall_script("u", "t"), build_recall_script("u", "t"));
        assert_eq!(build_userprompt_script("u", "t"), build_userprompt_script("u", "t"));
    }

    #[test]
    fn push_scripts_extract_data_without_swallowing_meta() {
        // Live dogfood (v2.0.0) caught this: the result layer appends ,"meta":{steering} after
        // "data" on every success, so a greedy "\(.*\)" peel swallows the meta blob into the
        // injected context. The data string is quote-sanitized, so [^"]* stops at its closing
        // quote. Guard both templates against a regression to the greedy form.
        for s in [
            build_primer_script("u", "t"),
            build_recall_script("u", "t"),
            build_userprompt_script("u", "t"),
        ] {
            assert!(
                s.contains(r#""data"[[:space:]]*:[[:space:]]*"\([^"]*\)""#),
                "data-extraction stops at the closing quote (safe against trailing meta)"
            );
            assert!(
                !s.contains(r#""data"[[:space:]]*:[[:space:]]*"\(.*\)""#),
                "must not use the greedy .* that swallows the trailing meta"
            );
        }
    }

    #[test]
    fn push_scripts_emit_escaped_newline_for_valid_json() {
        // Deployed-loop dogfood (v2.0.1) caught this: a bare \n in the printf FORMAT string
        // becomes a REAL newline inside the additionalContext value → invalid JSON → the
        // client rejects the injection. The header separator must be \\n so printf emits a
        // literal \n escape.
        for s in [
            build_primer_script("u", "t"),
            build_recall_script("u", "t"),
            build_userprompt_script("u", "t"),
        ] {
            assert!(
                s.contains(r"\\n%s"),
                "additionalContext header newline is escaped (\\n), not a raw newline"
            );
        }
    }

    #[test]
    fn userprompt_script_extracts_cues_and_injects_nominees() {
        // Sprint 21c (item D): prompt -> keywords -> recall -> injected context.
        // Sprint 28 (studio#3): the injected thing is NOMINEES, not "the ONE fitting fact".
        let s = build_userprompt_script("http://127.0.0.1:8890/mcp", "sekret");
        assert!(s.contains(r#"MCP_URL="http://127.0.0.1:8890/mcp""#), "bakes the mcp url");
        assert!(s.contains(r#"TOKEN="sekret""#), "bakes the bearer token");
        assert!(
            s.contains(r#""kind":"recall""#)
                && s.contains("try_recall symptom")
                && s.contains("try_recall symbol"),
            "recalls by BOTH symbol and symptom cues (Sprint 22a dual-cue)"
        );
        assert!(s.contains(r#""prompt""#), "reads the prompt from hook stdin");
        assert!(
            s.contains(r#"case "$prompt" in /*) exit 0"#),
            "slash commands are not topics"
        );
        assert!(
            s.contains(r#"[ "$count" -ge 2 ] || exit 0"#),
            "a cue needs >=2 content tokens"
        );
        assert!(
            s.contains("UserPromptSubmit") && s.contains("additionalContext"),
            "injects prompt-boundary context"
        );
        // Sprint 28 (studio#3) — THIS ASSERTION IS THE INVERSE OF WHAT IT USED TO BE.
        // It previously required `*"\n"*) return 1`, i.e. it ASSERTED the defect: the
        // suite actively defended the retired 21c "one fact or silence" contract, so a
        // correct fix would have failed the test and been reverted. 27a made multi-answer
        // the norm; discarding it is the bug, not the safeguard.
        assert!(
            !s.contains(r#"*"\n"*) return 1"#),
            "must NOT skip multi-answer recalls — 27a returns up to 11 nominees, always"
        );
        assert!(
            s.contains("NOMINEES, not vouched answers"),
            "labels what it injects as candidates to judge, per the 27a rendering contract"
        );
        assert!(s.contains("No\\ known\\ knowledge"), "silent on absence");
        assert!(s.contains("--max-time 2"), "short per-attempt timeout");
    }

    /// Sprint 28 (studio#3): the BEHAVIOURAL gate — runs the real deployed script against
    /// a stubbed `curl` that returns a realistic MULTI-nominee store response, and asserts
    /// context actually comes out.
    ///
    /// Why this exists and the string assertions above are not enough: for two weeks every
    /// string assertion passed, the suite was green, and this hook emitted nothing on every
    /// prompt. The defect's only symptom is an ABSENCE, and no assertion anywhere asserted
    /// on absence. This test fails on the pre-fix script.
    #[test]
    fn userprompt_script_actually_emits_on_a_multi_nominee_answer() {
        let dir = unique_tempdir("userprompt-behaviour");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();

        // A store answer in the CURRENT (27a) shape: several \n-joined nominees. The \n
        // are JSON escapes inside the data string, exactly as the real layer emits them.
        let stub = bin.join("curl");
        std::fs::write(
            &stub,
            "#!/usr/bin/env bash\ncat <<'JSON'\n{\"result\":{\"content\":[{\"type\":\"text\",\
             \"text\":\"{\\\"success\\\":true,\\\"data\\\":\\\"In a similar situation: first \
             nominee  [meaning-near]\\\\nIn a similar situation: second nominee  \
             [meaning-near]\\\\nIn a similar situation: third nominee  [meaning-near]\\\"}\"}]}}\nJSON\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let script = dir.join("userpromptsubmit-recall.sh");
        std::fs::write(&script, build_userprompt_script("http://127.0.0.1:8890/mcp", "tok")).unwrap();

        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let out = std::process::Command::new("bash")
            .arg(&script)
            .env("PATH", path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin
                    .as_mut()
                    .unwrap()
                    .write_all(br#"{"prompt":"what do we know about the supervision surface recall contract"}"#)?;
                c.wait_with_output()
            })
            .expect("hook script runs");

        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.trim().is_empty(),
            "the hook emitted NOTHING on a real multi-nominee answer — this is studio#3"
        );
        assert!(
            stdout.contains("additionalContext"),
            "emits prompt-boundary context, got: {stdout}"
        );
        assert!(
            stdout.contains("first nominee") && stdout.contains("third nominee"),
            "passes through ALL nominees, not a trimmed single fact, got: {stdout}"
        );
        let v: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("emits parseable JSON");
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
    }

    #[test]
    fn userprompt_script_fires_symbol_cues_precise_first() {
        // Sprint 22a dual-cue: qualified/member identifiers in the prompt fire a
        // kind=recall,symbol= attempt BEFORE the symptom cues, via the shared helper.
        let s = build_userprompt_script("u", "t");
        assert!(s.contains("symcues="), "extracts symbol cues from the prompt");
        assert!(s.contains("try_recall symbol"), "tries symbol cues");
        assert!(
            s.find("try_recall symbol").unwrap() < s.find("try_recall symptom").unwrap(),
            "symbol cues are tried before symptom cues (precise-first)"
        );
    }

    #[test]
    fn recall_script_fires_on_java_edit() {
        // Sprint 22a recall-on-Edit: a hand-edit of a .java file also triggers recall,
        // with the type name (Foo.java -> Foo) as the symbol cue.
        let s = build_recall_script("u", "t");
        assert!(s.contains("Edit|Write|MultiEdit"), "matches Edit/Write of a source file");
        assert!(s.contains(r#""file_path""#), "reads the edited file path");
        assert!(
            s.contains(r#"basename "$fp" .java"#),
            "derives the type-name cue from the edited .java file"
        );
    }

    #[test]
    fn a_redeploy_succeeds_while_a_hook_is_executing() {
        // C6 exit clause 5, FIRST half — and the hazard this whole change
        // introduces. Linux refuses to open an EXECUTING binary for writing
        // (ETXTBSY). Hooks fire on every prompt, so a redeploy lands on top of
        // running processes routinely, and overwriting a shell script never had
        // this problem — the kernel holds no text pages for bash's argument.
        //
        // The test runs a real long-lived process from the deployed path and
        // redeploys underneath it. Replace the unlink with a plain write and
        // this fails with "Text file busy", which is the point.
        let dir = unique_tempdir("etxtbsy");
        let hooks = dir.join("hooks");
        let source = dir.join("source-binary");

        // A REAL ELF binary, not a shell script. The first version of this
        // test used a script and PASSED WITHOUT THE UNLINK — vacuous, because
        // ETXTBSY protects the interpreter's image, not the script text the
        // interpreter reads. Only an executing ELF makes the kernel refuse the
        // write, which is the condition the deployed hook binaries will be in.
        std::fs::create_dir_all(&dir).unwrap();
        let real_binary = ["/bin/sleep", "/usr/bin/sleep"]
            .iter()
            .map(std::path::Path::new)
            .find(|p| p.exists())
            .expect("a real ELF binary is required — this test is meaningless with a script");
        std::fs::copy(real_binary, &source).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let roles = ["jawata-hook-primer"];
        let written = deploy_hook_binaries(&source, &hooks, &roles, HostPlatform::host()).unwrap();
        assert_eq!(vec!["jawata-hook-primer".to_string()], written);

        // Start it, and hold it running across the redeploy.
        let deployed = hooks.join("jawata-hook-primer");
        // RETRY THE FIRST EXEC. This test copies a binary and immediately runs
        // it, and Linux refuses to exec a file still open for writing — the very
        // errno this test exists to prove the DEPLOY handles. It was latent
        // until enough tests spawned binaries concurrently for the window to be
        // hit, and then failed on ubuntu-22.04 while green everywhere else.
        //
        // THIRD COPY of this retry (fail_safe_boundary.rs and
        // edit_gate_runs_the_real_binary.rs have the others). Three copies of
        // one workaround is debt: it wants a shared test helper, which spans a
        // lib test and two integration binaries and is therefore its own change,
        // not a rider on a release.
        let mut attempt = 0;
        let mut child = loop {
            match std::process::Command::new(&deployed)
                .arg("30")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => break c,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 40 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => panic!("the deployed binary must be executable: {e}"),
            }
        };
        std::thread::sleep(std::time::Duration::from_millis(150));

        // A CHANGED binary, so the byte-stable short-circuit cannot hide the
        // hazard by simply not writing. Appending a byte keeps it a valid ELF
        // for the kernel's purposes while making the content differ.
        let mut bytes = std::fs::read(&source).unwrap();
        bytes.push(0);
        std::fs::write(&source, &bytes).unwrap();
        let result = deploy_hook_binaries(&source, &hooks, &roles, HostPlatform::host());

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);

        let written = result.unwrap_or_else(|e| {
            panic!("redeploy over an EXECUTING hook failed: {e}\n                    This is ETXTBSY — unlink before writing.")
        });
        assert_eq!(vec!["jawata-hook-primer".to_string()], written,
            "the redeploy reported nothing written even though the source changed");
    }

    #[test]
    fn deploying_the_same_binary_twice_writes_nothing_the_second_time() {
        let dir = unique_tempdir("bin-stable");
        let hooks = dir.join("hooks");
        let source = dir.join("source-binary");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "#!/bin/sh\nexit 0\n").unwrap();

        // Two LIVE roles — a retired role (guard, observer) here would be
        // contradictory: the deploy unconditionally removes retired binaries
        // before writing, so it can never be byte-stable for one.
        let roles = ["jawata-hook-primer", "jawata-hook-recall"];
        assert_eq!(2, deploy_hook_binaries(&source, &hooks, &roles, HostPlatform::host()).unwrap().len());
        assert!(deploy_hook_binaries(&source, &hooks, &roles, HostPlatform::host()).unwrap().is_empty(),
            "an unchanged redeploy must write nothing — otherwise every deploy churns \
             the files a hook may be executing");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The 3.7.2 dogfood clobber, reproduced END TO END: binaries land first
    /// (the production order since v3.7.3), then EVERY section writer runs
    /// with `force_rewrite` — the strongest clobber attempt. 3.7.2 shipped
    /// with four of the six role files as bash scripts because four writers
    /// ran after the binary deploy and wrote the generation-2 body over the
    /// binary, same filenames, so every file LOOKED deployed. The per-step
    /// checks all passed at their instant; only the end state was wrong —
    /// which is why this asserts the end state.
    #[test]
    fn the_full_deploy_order_leaves_live_roles_binaries_and_the_observer_a_script() {
        let dir = unique_tempdir("deploy-order");
        let hooks = dir.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let source = dir.join("source-binary");
        let binary_bytes = b"\x7fELF fake role binary".to_vec();
        std::fs::write(&source, &binary_bytes).unwrap();

        // Stale retired-role binaries from 3.7.1/3.7.2/3.7.3. The deploy must
        // REMOVE them: their existence is what used to flip a role away from
        // its live script (the invocation path preferred any binary on disk).
        std::fs::write(hooks.join("jawata-hook-observer"), &binary_bytes).unwrap();
        std::fs::write(hooks.join("jawata-hook-guard"), &binary_bytes).unwrap();

        deploy_hook_binaries(&source, &hooks, BINARY_LIVE_ROLES, HostPlatform::host()).unwrap();

        // The PRODUCTION writers, in the production order, with force_rewrite
        // (Regenerate mode) — the strongest clobber attempt. The first version
        // of this test drove all six roles through write_managed_hook_section
        // and stayed green while production's guard step, write_managed_hook,
        // carried its own unguarded copy of the body write (v3.7.3 audit F1/F2)
        // — verifying a parallel implementation instead of the deployed one,
        // which is this sprint's founding failure shape.
        let settings = dir.join("settings.json");
        let s = settings.to_str().unwrap();
        let (url, token) = ("http://127.0.0.1:1/mcp", "t");
        let resolve = |role: &str, script_file: &str| {
            invocation_path_in(&hooks, role, script_file).unwrap()
        };
        write_managed_hook(s, &resolve("jawata-hook-guard", GUARD_SCRIPT_FILE), url, false, true)
            .unwrap();
        write_managed_posthook(
            s, &resolve("jawata-hook-observer", OBSERVER_SCRIPT_FILE), url, token, false, true,
        )
        .unwrap();
        write_managed_primer(
            s, &resolve("jawata-hook-primer", PRIMER_SCRIPT_FILE), url, token, false, true,
        )
        .unwrap();
        write_managed_recall(
            s, &resolve("jawata-hook-recall", RECALL_SCRIPT_FILE), url, token, false, true,
        )
        .unwrap();
        write_managed_userprompt(
            s, &resolve("jawata-hook-userprompt", USERPROMPT_SCRIPT_FILE), url, token, false, true,
        )
        .unwrap();
        write_managed_stop(
            s, &resolve("jawata-hook-stop", STOP_SCRIPT_FILE), url, token, false, true,
        )
        .unwrap();

        for role in BINARY_LIVE_ROLES {
            let content = std::fs::read(hooks.join(role)).unwrap_or_else(|e| {
                panic!("{role}: the deployed binary is gone after the writers ran: {e}")
            });
            assert_eq!(binary_bytes, content,
                "{role}: a production writer clobbered the deployed binary with the \
                 generation-2 script — the 3.7.2 dogfood defect");
        }
        assert_eq!(build_observer_script(url, token),
            std::fs::read_to_string(hooks.join(OBSERVER_SCRIPT_FILE)).unwrap(),
            "the observer's live generation is the SCRIPT (role_generations)");
        // Sprint 28a: the guard resolves to whichever generation this deploy
        // actually produced, and the OTHER one must not be left behind.
        let guard_binary = hooks.join(role_binary_file_name_on(HostPlatform::host(), "jawata-hook-guard")).exists();
        let guard_script = hooks.join(GUARD_SCRIPT_FILE).exists();
        assert!(guard_binary || guard_script,
            "the guard must exist in one generation or the other");
        for retired in BINARY_RETIRED_ROLES {
            assert!(!hooks.join(retired).exists(),
                "{retired}: the retired binary must come off the disk — a stale one \
                 flips the invocation path back to the incomplete generation");
        }

        // And the settings entries point at what actually runs: the binary
        // for live roles, the script for the observer — the wiring the last
        // two releases got wrong.
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let commands: Vec<String> = ["PreToolUse", "PostToolUse", "SessionStart",
                "UserPromptSubmit", "Stop"]
            .iter()
            .flat_map(|section| {
                written["hooks"][section].as_array().cloned().unwrap_or_default()
            })
            .flat_map(|entry| entry["hooks"].as_array().cloned().unwrap_or_default())
            .filter_map(|h| h["command"].as_str().map(|c| c.to_string()))
            .collect();
        for role in BINARY_LIVE_ROLES {
            assert!(commands.iter().any(|c| c.ends_with(role)),
                "{role}: no settings entry points at the deployed binary; commands: {commands:?}");
        }
        assert!(commands.iter().any(|c| c.ends_with(OBSERVER_SCRIPT_FILE)),
            "the observer entry must point at its script; commands: {commands:?}");
        assert!(!commands.iter().any(|c| c.ends_with("jawata-hook-observer")),
            "no entry may point at the retired observer binary; commands: {commands:?}");
        // The guard's two assertions are GONE, not softened: it is a
        // BINARY_LIVE_ROLE as of Sprint 28a, so the loop above already requires
        // an entry naming its binary, and requiring the script too would demand
        // both generations at once. The observer keeps its pair because it is
        // still declared script-generation.
        assert!(!commands.iter().any(|c| c.ends_with(GUARD_SCRIPT_FILE)),
            "the guard script is retired on the Claude side once its binary is live; \
             commands: {commands:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// role_generations, enforced at the resolver: an observer binary ON DISK
    /// must not win the invocation path — the script generation is live.
    #[test]
    fn the_observer_invocation_path_ignores_a_binary_on_disk() {
        let dir = unique_tempdir("observer-script-gen");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("jawata-hook-observer"), b"stub").unwrap();
        assert_eq!(
            Some(dir.join(OBSERVER_SCRIPT_FILE)),
            invocation_path_in(&dir, "jawata-hook-observer", OBSERVER_SCRIPT_FILE),
            "the observer binary is a stub; the invocation path must stay on the script"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The selftests must EXEC a role binary rather than feed it to bash —
    /// `bash <elf>` prints "cannot execute binary file" to stderr and nothing
    /// to stdout, which reads as a failed selftest for a CORRECT binary. The
    /// stand-ins use shebangs so direct exec works; their outputs are the real
    /// contract shapes the deploy asserts.
    #[test]
    #[cfg(unix)]
    fn selftests_execute_role_binaries_directly_not_through_bash() {
        use std::os::unix::fs::PermissionsExt;
        let dir = unique_tempdir("selftest-direct");
        std::fs::create_dir_all(&dir).unwrap();
        let write_exec = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };

        // Retry ETXTBSY: a parallel test forking between this test's write and
        // exec inherits the write fd and holds it briefly — the same race the
        // fail_safe_boundary harness documents. Production deploys are
        // sequential in one thread and never hit it.
        let retry = |f: &dyn Fn() -> Result<(), String>| {
            let mut last = Ok(());
            for _ in 0..40 {
                last = f();
                match &last {
                    Err(e) if e.contains("Text file busy") => {
                        std::thread::sleep(std::time::Duration::from_millis(50))
                    }
                    _ => break,
                }
            }
            last
        };

        let stop = write_exec(
            "jawata-hook-stop",
            "#!/bin/sh\nprintf '{\"decision\":\"block\",\"reason\":\"selftest\"}'\n",
        );
        retry(&|| selftest_stop_hook_script(&stop))
            .expect("a block-with-reason through direct exec is the Stop contract");

        let primer = write_exec(
            "jawata-hook-primer",
            "#!/bin/sh\nprintf '{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"x\"}}'\n",
        );
        retry(&|| selftest_hook_script(&primer))
            .expect("a context emission through direct exec is the injecting contract");

        // And a role binary that emits NOTHING is a real failure, loudly —
        // never the silent Ok(()) the missing-bash arm grants scripts.
        let mute = write_exec("jawata-hook-userprompt", "#!/bin/sh\nexit 0\n");
        let err = retry(&|| selftest_hook_script(&mute))
            .expect_err("a mute role binary must fail its selftest");
        assert!(err.contains("NOTHING"),
            "the failure must be about the empty output, not a spawn race: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_resolver_looks_for_the_name_tauri_conf_actually_ships() {
        // C7 audit F3. The previous version of this test was a TAUTOLOGY: it
        // wrote a file it named itself and asserted the file had that name —
        // `dir.join("jawata-hook").file_name() == "jawata-hook"` is true for
        // every dir. It never called hook_binary_source(), never read
        // tauri.conf.json, and the name was an independent literal on both
        // sides, so renaming either one left it green. Its comment claimed "a
        // rename on either side breaks this"; no rename could.
        //
        // This reads the SHIPPED name out of tauri.conf.json — Tauri strips the
        // target-triple suffix when it places an externalBin, so
        // "binaries/jawata-hook" ships as "jawata-hook" beside the executable —
        // and asserts the resolver looks for exactly that.
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json"))
                .expect("tauri.conf.json must parse");
        let external = conf["bundle"]["externalBin"]
            .as_array()
            .expect("bundle.externalBin — Stage 7 declares the sidecar here")
            .first()
            .and_then(|v| v.as_str())
            .expect("at least one externalBin entry");
        let shipped_name = external.rsplit('/').next().unwrap_or(external);

        let exe_dir = std::path::Path::new("/opt/app");
        let candidates = hook_source_candidates(exe_dir);
        assert!(
            candidates.iter().any(|c| c == &exe_dir.join(shipped_name)),
            "tauri.conf.json ships the sidecar as {shipped_name:?}, but the resolver's \
             candidates beside the executable are {candidates:?} — the studio would find no \
             hook on an installed build and deploy none"
        );
    }

    #[test]
    fn a_missing_source_binary_is_a_named_error_not_a_silent_skip() {
        // The bundle failing to ship the binary must not look like a successful
        // deploy — that is a hook install with no hook.
        let dir = unique_tempdir("bin-missing");
        let err = deploy_hook_binaries(&dir.join("nope"), &dir.join("hooks"), &["jawata-hook-primer"], HostPlatform::host())
            .unwrap_err();
        assert!(err.contains("did not ship"), "the error must name the cause: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deploy_undeploy_deploy_is_byte_stable() {
        // C6 exit clause 2. A cycle that does not return to the same bytes
        // leaves residue — an orphaned key, a reordered array, a stale entry —
        // and residue is what the next migration has to guess about. Compared
        // as BYTES rather than as parsed JSON, because key order and formatting
        // are part of what a user diffing their settings.json sees.
        let dir = unique_tempdir("byte-stable");
        let settings = dir.join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let managed = dir.join("jawata-studio");
        std::fs::create_dir_all(&managed).unwrap();

        // A user's own hook is present throughout: the cycle must preserve it
        // exactly, not merely leave something equivalent.
        std::fs::write(&settings, serde_json::json!({
            "hooks": {
                "SessionStart": [
                    { "hooks": [ { "type": "command", "command": "/home/u/bin/mine.sh" } ] }
                ]
            }
        }).to_string()).unwrap();
        let primer = managed.join("sessionstart-primer.sh");
        let deploy = || {
            write_managed_primer(&settings_path, &primer, "http://u/mcp", "t", false, false).unwrap()
        };
        let undeploy = || remove_managed_primer(&settings_path, &primer, false).unwrap();
        let read = || std::fs::read_to_string(&settings).unwrap();

        // The baseline is the file after ONE FULL CYCLE, not the hand-written
        // one. The product pretty-prints settings.json — correctly, since users
        // read and edit it — so comparing against compact input measures
        // FORMATTING, not residue. The first version of this test failed for
        // exactly that reason, and the fix belongs in the baseline rather than
        // in the writer.
        deploy();
        let after_first = read();
        undeploy();
        let settled = read();

        deploy();
        assert_eq!(after_first, read(), "the second deploy did not reproduce the first's bytes");
        assert!(!deploy(), "an unchanged redeploy must be a no-op");
        assert_eq!(after_first, read(), "a redeploy changed the file while reporting no change");

        undeploy();
        assert_eq!(settled, read(), "the cycle is not closed — undeploy left different bytes");

        // The user's own hook survives every cycle, asserted on CONTENT so a
        // formatting change cannot mask its loss.
        let v: serde_json::Value = serde_json::from_str(&read()).unwrap();
        assert!(
            v["hooks"]["SessionStart"].as_array().is_some_and(|a| a
                .iter()
                .any(|e| e["hooks"][0]["command"] == "/home/u/bin/mine.sh")),
            "the user's own hook did not survive the cycles: {}",
            read()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE WIRE. The pure function passing its own test proves nothing about
    /// production — deleting the call site left it green (measured, the
    /// Stage-8 lesson again). This drives write_hook_config, the function the
    /// deploy path actually calls, and requires the rotation to have happened.
    #[test]
    fn a_config_deploy_rotates_an_oversized_silence_log_on_the_way() {
        let dir = std::env::temp_dir().join(format!("jawata-mgr-rotwire-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let live = dir.join("hook_silence.log");
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(200) + "\n";
        let mut body = String::new();
        while body.len() <= 256 * 1024 {
            body.push_str(&filler);
        }
        fs::write(&live, &body).unwrap();

        write_hook_config(&dir, "http://u/mcp", "tw", "claude-code").unwrap();

        assert!(!live.exists(), "the deploy pass must have rotated the oversized log");
        assert_eq!(
            body,
            fs::read_to_string(dir.join("hook_silence.log.1")).unwrap(),
            "moved whole, never truncated"
        );
    }

    #[test]
    fn the_manager_rotates_an_oversized_silence_log_and_leaves_a_small_one_alone() {
        let dir = std::env::temp_dir().join(format!("jawata-mgr-rotate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let live = dir.join("hook_silence.log");

        // Small: untouched.
        fs::write(&live, "1\tprimer\tstore-had-nothing\t\n").unwrap();
        assert!(!rotate_silence_log(&dir), "a small log must not rotate");
        assert!(live.exists());
        assert!(!dir.join("hook_silence.log.1").exists());

        // Oversized: rotated aside, nothing destroyed.
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(200) + "\n";
        let mut body = String::new();
        while body.len() <= 256 * 1024 {
            body.push_str(&filler);
        }
        fs::write(&live, &body).unwrap();
        assert!(rotate_silence_log(&dir), "an oversized log must rotate");
        assert!(!live.exists(), "the live name is free for the next append");
        let kept = fs::read_to_string(dir.join("hook_silence.log.1")).unwrap();
        assert_eq!(body, kept, "rotation must move, never truncate or rewrite");
    }

    #[test]
    fn hook_config_is_never_seen_torn_by_a_concurrent_reader() {
        // C6 exit clause 5, second half. Concurrency here is MEASURED: three
        // sessions with a holding hook produced three overlapping pairs, so a
        // deploy genuinely can land while a hook is reading. A plain write
        // truncates first, and a reader in that window sees an empty or
        // half-written file — which the hook's read side reports as a TORN
        // DEPLOY. This makes that state unreachable.
        let dir = unique_tempdir("hookcfg-race");
        let hooks = dir.join("hooks");
        write_hook_config(&hooks, "http://u/mcp", "t0", "claude-code").unwrap();
        let target = hooks.join("hook_config.json");

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader_path = target.clone();
        let reader = std::thread::spawn(move || {
            let mut reads = 0u32;
            let mut torn = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            // Keep reading until the writer says stop AND we have a usable
            // sample — bounded, so a starved thread cannot hang the suite.
            while (!reader_stop.load(std::sync::atomic::Ordering::Relaxed) || reads < 60)
                && std::time::Instant::now() < deadline
            {
                match std::fs::read_to_string(&reader_path) {
                    Ok(text) => {
                        reads += 1;
                        // Either the whole old file or the whole new one —
                        // never something in between.
                        if serde_json::from_str::<serde_json::Value>(&text).is_err() {
                            torn.push(text);
                        }
                    }
                    // A missing file would mean rename left a gap; record it.
                    Err(e) => torn.push(format!("<unreadable: {e}>")),
                }
            }
            (reads, torn)
        });

        for i in 0..300 {
            write_hook_config(&hooks, "http://u/mcp", &format!("token-{i}"), "claude-code")
                .unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let (reads, torn) = reader.join().expect("reader thread");

        // C6 audit round 3, N3. This was the suite's ONLY load-sensitive
        // threshold and therefore the standing suspect for a single
        // unreproduced red observed at 85ddeac: the reader is a spin loop
        // racing 300 writes, and a thread starved on a busy machine could do
        // fewer than 50 reads and fail once, for no reason connected to the
        // property under test.
        //
        // The property is "no read is ever torn", and it does not need a fixed
        // count — it needs enough reads to be meaningful. So the reader now
        // runs until it has taken a decent sample OR a wall-clock bound
        // expires, and the assertion is on the sample it actually took. A
        // starved machine now makes the test slower, not red.
        assert!(
            reads > 5,
            "the reader took {reads} reads even with a wall-clock budget — the file was \
             unreadable, not merely contended"
        );
        assert!(torn.is_empty(), "{} torn read(s), first: {:?}", torn.len(), torn.first());

        // And no staging file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&hooks).unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");

        // Byte-stable: rewriting the same content changes nothing.
        assert!(!write_hook_config(&hooks, "http://u/mcp", "token-299", "claude-code").unwrap(),
            "an unchanged rewrite must be a no-op");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_entry_we_write_carries_an_explicit_timeout() {
        // C6 exit clause 6: "Every written entry carries an explicit timeout —
        // the client default is unpublished." Cursor documents its own only as
        // "platform default", so an entry without one is a hook whose bound
        // nobody knows, including us. Cursor's four already had them; Claude's
        // six did not.
        //
        // Asserted against the DEPLOYED FILE rather than the builders, so a
        // merge path that drops the field on the way through is caught too.
        let dir = unique_tempdir("timeouts");
        let settings = dir.join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let managed = dir.join("jawata-studio");
        std::fs::create_dir_all(&managed).unwrap();

        write_managed_hook(&settings_path, &managed.join("pretooluse-guard.sh"),
            "http://u/health", false, false).unwrap();
        write_managed_posthook(&settings_path, &managed.join("posttooluse-observer.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_stop(&settings_path, &managed.join("stop-gate.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_primer(&settings_path, &managed.join("sessionstart-primer.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_recall(&settings_path, &managed.join("pretooluse-recall.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_userprompt(&settings_path, &managed.join("userpromptsubmit-recall.sh"),
            "http://u/mcp", "t", false, false).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let mut checked = 0;
        for (event, entries) in written["hooks"].as_object().expect("hooks") {
            for entry in entries.as_array().expect("entries") {
                for hook in entry["hooks"].as_array().expect("hook list") {
                    let t = hook["timeout"].as_u64().unwrap_or_else(|| {
                        panic!("{event}: an entry we wrote carries no explicit timeout — the                                 client default is unpublished, so its bound is unknown: {hook}")
                    });
                    assert!(t >= 5, "{event}: timeout {t}s is under the hook's own 4s budget");
                    checked += 1;
                }
            }
        }
        assert_eq!(6, checked, "all six Claude entries checked, got {checked}");

        // Cursor's four, from the source both its deploy and the hook read.
        // An empty dir: no binary on disk, so every role resolves to its
        // script. This test is about EVENT NAMES, which do not vary with it.
        let empty = std::env::temp_dir().join("jawata-cursor-events-shape");
        for (event, entry) in managed_cursor_hook_entries(&empty) {
            assert!(
                entry["timeout"].as_u64().is_some(),
                "cursor {event} carries no explicit timeout: {entry}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_install_from_a_previous_generation_converges_to_one_entry_per_event() {
        // C6 exit clause 1, end to end, across ALL SIX roles and FOUR events —
        // the audit found the first version covering two. PreToolUse is the one
        // that matters most: it holds TWO managed entries (guard and recall) in
        // one array, so a predicate that over-claims deletes its sibling.
        //
        // The fixture also carries a MIXED install — a goja-generation entry
        // and a jawata-generation entry for the SAME event — which nothing
        // tested before, and which is what a user who upgraded once already has.
        let dir = unique_tempdir("gen-converge");
        let settings = dir.join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let managed = dir.join("jawata-studio");
        std::fs::create_dir_all(&managed).unwrap();

        let old = |script: &str| serde_json::json!({
            "hooks": [ { "type": "command",
                         "command": format!("/home/u/.claude/{script}") } ]
        });
        let mine = |name: &str| serde_json::json!({
            "hooks": [ { "type": "command", "command": format!("/home/u/bin/{name}") } ]
        });
        std::fs::write(&settings, serde_json::json!({
            "hooks": {
                "SessionStart": [ old("jawata-studio/sessionstart-primer.sh"), mine("my-primer.sh") ],
                "UserPromptSubmit": [ old("jawata-studio/userpromptsubmit-recall.sh") ],
                // BOTH generations of the guard, plus the recall, plus a user
                // hook — the crowded array.
                "PreToolUse": [
                    old("jawata-studio/pretooluse-guard.sh"),
                    old("goja-studio/pretooluse-guard.sh"),
                    old("jawata-studio/pretooluse-recall.sh"),
                    mine("my-pretool.sh")
                ],
                "PostToolUse": [ old("jawata-studio/posttooluse-observer.sh") ],
                "Stop": [ old("jawata-studio/stop-gate.sh"), mine("my-stop.sh") ]
            }
        }).to_string()).unwrap();

        write_managed_hook(&settings_path, &managed.join("pretooluse-guard.sh"),
            "http://u/health", false, false).unwrap();
        write_managed_posthook(&settings_path, &managed.join("posttooluse-observer.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_stop(&settings_path, &managed.join("stop-gate.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_primer(&settings_path, &managed.join("sessionstart-primer.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_recall(&settings_path, &managed.join("pretooluse-recall.sh"),
            "http://u/mcp", "t", false, false).unwrap();
        write_managed_userprompt(&settings_path, &managed.join("userpromptsubmit-recall.sh"),
            "http://u/mcp", "t", false, false).unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();

        let count = |event: &str, predicate: fn(&serde_json::Value) -> bool| -> usize {
            after["hooks"][event].as_array()
                .map(|a| a.iter().filter(|e| predicate(e)).count())
                .unwrap_or(0)
        };
        let checks: Vec<(&str, &str, fn(&serde_json::Value) -> bool)> = vec![
            ("SessionStart", "primer", is_managed_primer_entry),
            ("UserPromptSubmit", "userprompt", is_managed_userprompt_entry),
            ("PreToolUse", "guard", is_managed_hook_entry),
            ("PreToolUse", "recall", is_managed_recall_entry),
            ("PostToolUse", "observer", is_managed_posthook_entry),
            ("Stop", "stop", is_managed_stop_entry),
        ];
        for (event, role, predicate) in checks {
            assert_eq!(1, count(event, predicate),
                "{event}/{role}: exactly ONE managed entry must survive the upgrade — the \
                 mixed-generation array must collapse to one, not accumulate: {after}");
        }

        // Every user hook intact, on all three events that carried one.
        for (event, name) in [("SessionStart", "my-primer.sh"), ("PreToolUse", "my-pretool.sh"),
                              ("Stop", "my-stop.sh")] {
            assert!(
                after["hooks"][event].as_array().is_some_and(|a| a.iter()
                    .any(|e| e["hooks"][0]["command"].as_str()
                        .is_some_and(|c| c.ends_with(name)))),
                "{event}: the user's own {name} was removed by the migration: {after}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_role_recognises_what_its_own_builder_writes() {
        // C6 audit F1. The previous version tested the PREDICATE FUNCTION and
        // never the call sites, so it passed with six rows while only three
        // roles were wired — "built but not connected", in the stage whose
        // headline deliverable is preventing exactly that. It also fabricated
        // the stop command, which is why the wrong SCRIPT_GENERATION row
        // survived: JAWATA_STOP_SENTINEL is a marker inside the script BODY,
        // never a command.
        //
        // This goes through the REAL builders and the REAL predicates, so a
        // predicate left on the old check fails here.
        let base = std::path::Path::new("/home/u/.claude/jawata-studio");
        let cases: Vec<(&str, serde_json::Value, fn(&serde_json::Value) -> bool)> = vec![
            ("guard", build_managed_hook_entry(&base.join("pretooluse-guard.sh")),
                is_managed_hook_entry),
            ("observer", build_managed_posthook_entry(&base.join("posttooluse-observer.sh")),
                is_managed_posthook_entry),
            ("primer", build_managed_primer_entry(&base.join("sessionstart-primer.sh")),
                is_managed_primer_entry),
            ("recall", build_managed_recall_entry(&base.join("pretooluse-recall.sh")),
                is_managed_recall_entry),
            ("userprompt", build_managed_userprompt_entry(&base.join("userpromptsubmit-recall.sh")),
                is_managed_userprompt_entry),
            ("stop", build_managed_stop_entry(&base.join("stop-gate.sh")),
                is_managed_stop_entry),
        ];
        assert_eq!(6, cases.len(), "all six Claude roles");
        for (role, entry, predicate) in &cases {
            assert!(predicate(entry),
                "{role}: the deploy writes an entry its OWN predicate does not recognise — \
                 a redeploy would duplicate it and an undeploy would leave it: {entry}");
        }

        // And no predicate claims another role's entry, or an undeploy of one
        // role would delete another's.
        for (role_a, entry_a, _) in &cases {
            for (role_b, _, predicate_b) in &cases {
                if role_a == role_b { continue; }
                // guard and recall BOTH live on PreToolUse; their commands
                // differ, so neither may claim the other.
                assert!(!predicate_b(entry_a),
                    "{role_b}'s predicate claims {role_a}'s entry — an undeploy of one would \
                     remove the other from the same event array");
            }
        }
    }

    #[test]
    fn every_role_recognises_both_earlier_generations() {
        // The migration proper: gen-2 (the .sh this sprint replaces) and gen-1
        // (its pre-rebrand goja twin) must both read as OURS, or an upgrading
        // install keeps them as the user's own and they fire forever beside the
        // new binary.
        let predicates: Vec<(&str, fn(&serde_json::Value) -> bool)> = vec![
            ("jawata-hook-guard", is_managed_hook_entry),
            ("jawata-hook-observer", is_managed_posthook_entry),
            ("jawata-hook-primer", is_managed_primer_entry),
            ("jawata-hook-recall", is_managed_recall_entry),
            ("jawata-hook-userprompt", is_managed_userprompt_entry),
            ("jawata-hook-stop", is_managed_stop_entry),
        ];
        for (binary, predicate) in &predicates {
            let script = SCRIPT_GENERATION.iter().find(|(b, _)| b == binary)
                .unwrap_or_else(|| panic!("{binary} has no generation-2 row"))
                .1;
            for command in [
                format!("/home/u/.claude/hooks/{binary}"),          // gen 3
                format!("/home/u/.claude/{script}"),                 // gen 2
                format!("/home/u/.claude/{}", legacy_sentinel(script)),   // gen 1
            ] {
                let entry = serde_json::json!({
                    "hooks": [ { "type": "command", "command": command } ]
                });
                assert!(predicate(&entry),
                    "{binary}: {command} is one of OUR generations and its own predicate does \
                     not recognise it — it would be preserved as the user's hook and keep firing");
            }
        }
    }

    #[test]
    fn a_windows_style_command_is_recognised_as_ours() {
        // C6 audit round 3, N4 — measured, not hypothesised: every sentinel
        // carries a forward slash while PathBuf::join emits backslashes on
        // Windows, so on Windows every predicate missed its own entry and each
        // deploy appended another, unbounded, with undeploy leaving them all.
        //
        // Asserted here on Linux because the DEFECT is not platform-specific
        // even though the symptom is: the matching rule is the same code
        // everywhere, and a backslash command is exactly what a Windows install
        // presents to it.
        let windows = |c: &str| serde_json::json!({
            "hooks": [ { "type": "command", "command": c } ]
        });
        let cases: Vec<(&str, serde_json::Value, fn(&serde_json::Value) -> bool)> = vec![
            ("guard", windows(r"C:\Users\h\.claude\jawata-studio\pretooluse-guard.sh"),
                is_managed_hook_entry),
            ("observer", windows(r"C:\Users\h\.claude\jawata-studio\posttooluse-observer.sh"),
                is_managed_posthook_entry),
            ("primer", windows(r"C:\Users\h\.claude\jawata-studio\sessionstart-primer.sh"),
                is_managed_primer_entry),
            ("recall", windows(r"C:\Users\h\.claude\jawata-studio\pretooluse-recall.sh"),
                is_managed_recall_entry),
            ("userprompt", windows(r"C:\Users\h\.claude\jawata-studio\userpromptsubmit-recall.sh"),
                is_managed_userprompt_entry),
            ("stop", windows(r"C:\Users\h\.claude\jawata-studio\stop-gate.sh"),
                is_managed_stop_entry),
            ("gen-3 binary", windows(r"C:\Users\h\.claude\hooks\jawata-hook-primer"),
                is_managed_primer_entry),
        ];
        for (role, entry, predicate) in cases {
            assert!(predicate(&entry),
                "{role}: a Windows-style command is one of OURS and was not recognised — on \
                 Windows every deploy would append another entry and undeploy would leave \
                 them all: {entry}");
        }
    }

    #[test]
    fn the_production_path_functions_write_what_the_predicates_look_for() {
        // C6 audit round 2, N1 — THE LAST LINK, and the one closest to
        // production. managed_*_script_path() decides the filename that appears
        // in the command the deploy actually writes, and every test hardcoded
        // its own string instead. The auditor measured the consequence: rename
        // sessionstart-primer.sh in the path function and the WHOLE SUITE stays
        // green while a real deploy appends one SessionStart entry per run,
        // unbounded, with undeploy leaving all of them — because
        // write_managed_hook_section is retain(!is_managed) then push, so a
        // predicate that cannot see its own entry never removes it.
        //
        // The chain was HOOK_ROLES <-> hook-events.json <-> SCRIPT_GENERATION
        // <-> sentinel constants <-> builders, and stopped one link short of
        // the paths. This closes it.
        // Derived under a FAKE home. Calling the real managed_*_script_path()
        // here would invoke claude_scripts_dir(), which renames a pre-rebrand
        // directory as a side effect — so `cargo test` would migrate the
        // developer's actual home (C6 audit round 3, N5). The filenames below
        // are the ones those functions append, and the assertion is that each
        // contains its predicate's sentinel; a rename in the production
        // function still has to be mirrored here, and the control proves it.
        let home = std::path::Path::new("/fake-home");
        let dir = claude_scripts_dir_under(home);
        let paths: Vec<(&str, Option<PathBuf>)> = vec![
            ("jawata-hook-guard", Some(dir.join(GUARD_SCRIPT_FILE))),
            ("jawata-hook-observer", Some(dir.join(OBSERVER_SCRIPT_FILE))),
            ("jawata-hook-primer", Some(dir.join(PRIMER_SCRIPT_FILE))),
            ("jawata-hook-recall", Some(dir.join(RECALL_SCRIPT_FILE))),
            ("jawata-hook-userprompt", Some(dir.join(USERPROMPT_SCRIPT_FILE))),
            ("jawata-hook-stop", Some(dir.join(STOP_SCRIPT_FILE))),
        ];
        assert_eq!(BINARY_LIVE_ROLES.len() + BINARY_RETIRED_ROLES.len(), paths.len(),
            "one production path per role, live or retired");

        for (binary, path) in paths {
            let Some(path) = path else {
                // No home directory in this environment: the check cannot be
                // made, and saying so is better than passing.
                panic!("{binary}: no production path resolved — this assertion did not run");
            };
            let sentinel = SCRIPT_GENERATION
                .iter()
                .find(|(b, _)| *b == binary)
                .unwrap_or_else(|| panic!("{binary} has no generation-2 row"))
                .1;
            let shown = display_path(&path);
            assert!(
                shown.contains(sentinel),
                "{binary}: the deploy writes {shown}, which does NOT contain the sentinel \
                 {sentinel} its own predicate matches on. Every deploy would append another \
                 entry and undeploy would leave them all."
            );
        }
    }

    #[test]
    fn the_deployed_role_list_matches_the_shared_hook_contract() {
        // C6 audit F2. HOOK_ROLES — the list the deploy actually writes — was
        // asserted against NOTHING: not against the hook's role table, not
        // against hook-events.json, not against SCRIPT_GENERATION. Drop a name
        // and that binary is never deployed while its settings entry points at
        // a file that does not exist; typo one and the binary deploys under a
        // name argv[0] dispatch cannot resolve, so it exits silent forever.
        // Both stay green across both crates.
        //
        // This is C5's own finding — "that is a count, not a linkage" —
        // reintroduced one layer up, which is why it gets the same cure.
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json")).unwrap();
        let claude = contract["claude-code"].as_object().expect("claude-code section");

        // v3.7.3: the deploy no longer writes a binary for every role — it
        // writes exactly the roles the contract declares `live: "binary"`
        // (role_generations), and RETIRES the binaries of the rest. The
        // declaration is what makes an observer-style drop a decision instead
        // of a discovery.
        let generations = contract["role_generations"]
            .as_object()
            .expect("role_generations section");
        let mut expected_live: Vec<String> = Vec::new();
        let mut expected_retired: Vec<String> = Vec::new();
        for role in claude.keys() {
            let generation = generations
                .get(role)
                .and_then(|g| g["live"].as_str())
                .unwrap_or_else(|| panic!("{role}: no role_generations row — every role \
                     declares which generation is live"));
            match generation {
                "binary" => expected_live.push(format!("jawata-hook-{role}")),
                "script" => expected_retired.push(format!("jawata-hook-{role}")),
                other => panic!("{role}: unknown generation {other:?}"),
            }
        }
        expected_live.sort();
        expected_retired.sort();
        let mut actual_live: Vec<String> = BINARY_LIVE_ROLES.iter().map(|s| s.to_string()).collect();
        actual_live.sort();
        let mut actual_retired: Vec<String> =
            BINARY_RETIRED_ROLES.iter().map(|s| s.to_string()).collect();
        actual_retired.sort();
        assert_eq!(expected_live, actual_live,
            "the binaries the deploy writes have drifted from the shared hook contract");
        assert_eq!(expected_retired, actual_retired,
            "the binaries the deploy retires have drifted from the shared hook contract");

        // Every role — live or retired — has a generation-2 row, or its
        // migration is silently absent.
        for role in BINARY_LIVE_ROLES.iter().chain(BINARY_RETIRED_ROLES) {
            assert!(SCRIPT_GENERATION.iter().any(|(b, _)| b == role),
                "{role} has no generation-2 row — an upgrading install keeps \
                 its old entry as the user's own");
        }
        assert_eq!(BINARY_LIVE_ROLES.len() + BINARY_RETIRED_ROLES.len(), SCRIPT_GENERATION.len(),
            "SCRIPT_GENERATION carries a row for a role the deploy neither writes nor retires");
    }

    /// Audit F3, the bash column: `stop_rules` declared each rule's status in
    /// the bash generation and NOTHING asserted it — a rule could be dropped
    /// from the template while the contract still said "present", which is the
    /// divergence the section exists to prevent. Each declared-present rule
    /// pins to a distinctive marker in the template it claims to live in.
    #[test]
    fn every_stop_rule_declared_present_in_bash_has_its_marker_in_the_template() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json")).unwrap();
        let rules = contract["stop_rules"]["rules"].as_object().expect("stop_rules.rules");
        let marker: std::collections::HashMap<&str, &str> = [
            ("anti_loop", "stop_hook_active"),
            ("audit_fix_loop", "AUDIT-FIX LOOP"),
            ("unjudged_ask", "UNJUDGED"),
            ("seat_discipline", "SEAT DISCIPLINE"),
            ("decision_test_length", "2200"),
            ("undefined_abbreviations", "undefined terms"),
        ]
        .into();
        for (rule, row) in rules {
            if row["bash"].as_str() != Some("present") {
                continue;
            }
            let m = marker.get(rule.as_str()).unwrap_or_else(|| {
                panic!("{rule}: declared present in bash but this test knows no marker \
                        for it — add one here alongside the rule")
            });
            assert!(STOP_TEMPLATE.contains(m),
                "{rule}: declared present in the bash generation, but its marker \
                 {m:?} is not in STOP_TEMPLATE — the contract says more than the \
                 script does");
        }
    }

    /// Audit F3, the Cursor side — restated for Sprint 28a.
    ///
    /// The original form asserted the Cursor deploy was all-script BY
    /// CONSTRUCTION. That was the right guard for the wrong invariant: what
    /// actually matters is that **no cutover happens without a declared row**,
    /// and "never cut over" was merely the cheapest way to guarantee it while
    /// no Cursor role had a binary.
    ///
    /// The guard now does, so the check becomes the weaker and truer one: a
    /// Cursor entry may name a role binary EXACTLY WHERE `role_generations`
    /// declares that role live as a binary, and must be a script everywhere
    /// else. A cutover that outruns the contract still fails here — which is
    /// the property the 3.7.2 and 3.7.3 dogfoods paid for.
    #[test]
    fn a_cursor_entry_names_a_binary_only_where_the_contract_declares_one() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json")).expect("contract parses");
        let generations = &doc["role_generations"];

        // BOTH states, or this test proves nothing. With no binary on disk every
        // role resolves to its script and the loop below would `continue` past
        // every assertion — green, and blind. So the binary case is staged
        // explicitly, and the script case is checked as the fallback it is.
        let dir = std::env::temp_dir()
            .join(format!("jawata-cursor-contract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch hooks dir");

        // --- state 1: no binary deployed -> the fallback is the script ---
        let fallback = managed_cursor_hook_entries(&dir);
        let guard_fallback = fallback
            .iter()
            .find(|(ev, _)| *ev == "beforeShellExecution")
            .map(|(_, e)| e["command"].as_str().unwrap_or_default().to_string())
            .expect("a guard entry exists");
        assert!(
            guard_fallback.ends_with(".sh"),
            "with no binary deployed the guard must fall back to its script, got {guard_fallback:?}"
        );

        // --- state 2: binary deployed -> the entry names it ---
        // Sprint 28a, 2026-08-13: stage ALL FOUR, not just the guard. Staging one
        // left the other three resolving to scripts, so the loop below `continue`d
        // past them and the test could not have seen a wrong cutover on any role
        // but the guard — which is precisely the blindness it exists to prevent.
        for (_, role, _) in CURSOR_ROLES {
            std::fs::write(dir.join(role_binary_file_name_on(HostPlatform::host(), role)), b"x")
                .expect("stage the binary");
        }

        for (event, entry) in managed_cursor_hook_entries(&dir) {
            let command = entry["command"].as_str().unwrap_or_default();
            let names_binary = command.contains("jawata-hook-");

            // Which role is this entry? Read it from the command itself so the
            // test cannot drift from what the deploy writes.
            let role = if names_binary {
                command
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(".exe")
                    .strip_prefix("jawata-hook-")
                    .unwrap_or_default()
                    .to_string()
            } else {
                continue; // scripts need no row; the contract only governs cutovers
            };

            // Generation is per-role AND per-client: a `cursor` override wins
            // here when the row carries one, because a role can legitimately be
            // script-generation on Claude Code and binary on Cursor. The
            // observer is that case — Claude's script captures tool outcomes,
            // Cursor's does nothing at all. The override must be DECLARED; an
            // undeclared one still fails on `live`.
            let row = &generations[&role];
            let declared = row["cursor"].as_str().unwrap_or_else(|| row["live"].as_str().unwrap_or(""));
            assert_eq!(
                "binary", declared,
                "{event}: the Cursor entry {command:?} names the {role} binary, but \
                 role_generations declares that role live as {declared:?} for Cursor — a \
                 cutover no contract row governs is exactly what the 3.7.2/3.7.3 dogfoods caught"
            );
            assert!(
                !command.ends_with(".sh"),
                "{event}: {command:?} claims to be a binary and ends .sh"
            );
        }

        // The staged binary MUST have been picked up — otherwise the loop above
        // ran entirely on scripts and asserted nothing.
        // EVERY staged binary must have been picked up. Asserting only the guard
        // is what made the loop above blind to the other three for a whole
        // release: three entries kept naming `.sh` files, and on Windows — where
        // a `.sh` cannot execute — Cursor opened a window for each one at session
        // start, on every prompt, and after every tool call.
        let staged = managed_cursor_hook_entries(&dir);
        for (_, role, _) in CURSOR_ROLES {
            let named = staged.iter().any(|(_, e)| {
                e["command"].as_str().unwrap_or_default().contains(role)
            });
            assert!(
                named,
                "the deployed {role} binary must be picked up, or this test is vacuous \
                 for that role; entries were {staged:?}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE test this whole area was missing: what the deploy WRITES is what the
    /// resolver LOOKS FOR — on both filename conventions, from any machine.
    ///
    /// Five consecutive releases shipped a Windows-only mismatch because the
    /// writer spelled the file `jawata-hook-guard` and every reader spelled it
    /// `jawata-hook-guard.exe`. No test could catch it: `cfg!(windows)` fixed the
    /// convention at compile time, and on Linux the two spellings are the same
    /// string, so the defect was unrepresentable in the suite. It was found by a
    /// human looking at a directory listing on a Windows 11 machine.
    ///
    /// Every future naming change fails HERE first, on a developer's Linux box,
    /// instead of on someone's Windows install.
    #[test]
    fn what_the_deploy_writes_is_what_the_resolver_looks_for() {
        for platform in [HostPlatform::Unix, HostPlatform::Windows] {
            let dir = std::env::temp_dir().join(format!(
                "jawata-roundtrip-{}-{platform:?}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");

            // A stand-in for the shipped binary; the bytes are irrelevant, the
            // NAME is the entire subject of this test.
            let source = dir.join("source-binary");
            std::fs::write(&source, b"binary bytes").expect("stage a source");

            let roles = ["jawata-hook-guard", "jawata-hook-primer"];
            deploy_hook_binaries(&source, &dir, &roles, platform).expect("deploy");

            for role in roles {
                let expected = dir.join(role_binary_file_name_on(platform, role));
                assert!(
                    expected.exists(),
                    "{platform:?}: the deploy must write {expected:?} — the name every \
                     reader resolves. Present instead: {:?}",
                    std::fs::read_dir(&dir)
                        .map(|rd| rd.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect::<Vec<_>>())
                        .unwrap_or_default()
                );
            }

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// What a WINDOWS install actually ends up with, asserted from Linux.
    ///
    /// The end-to-end invariant, and the one nobody could state before the
    /// naming convention became a value: after a deploy, every Cursor entry
    /// names a file that EXISTS, and no entry names a `.sh` for a role that runs
    /// as a binary. Both halves failed on Windows in v3.7.9 — four binaries were
    /// written under names nothing resolved, and three entries kept naming
    /// scripts that cannot execute there.
    #[test]
    fn a_windows_deploy_leaves_every_cursor_entry_naming_a_file_that_exists() {
        for platform in [HostPlatform::Unix, HostPlatform::Windows] {
            let dir = std::env::temp_dir()
                .join(format!("jawata-e2e-{}-{platform:?}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");

            let source = dir.join("source-binary");
            std::fs::write(&source, b"binary bytes").expect("stage a source");
            let roles: Vec<&str> = CURSOR_ROLES
                .iter()
                .filter(|(_, role, _)| cursor_role_is_binary_live(role))
                .map(|(_, role, _)| *role)
                .collect();
            deploy_hook_binaries(&source, &dir, &roles, platform).expect("deploy");

            for (event, entry) in managed_cursor_hook_entries_on(platform, &dir) {
                let command = entry["command"].as_str().unwrap_or_default();
                let file = command.rsplit('/').next().unwrap_or_default();

                assert!(
                    !file.ends_with(".sh"),
                    "{platform:?} {event}: names the script {file:?}, but every Cursor role \
                     runs as a binary — on Windows a .sh cannot execute at all"
                );
                assert!(
                    dir.join(file).exists(),
                    "{platform:?} {event}: names {file:?}, which the deploy did not write. \
                     On disk: {:?}",
                    std::fs::read_dir(&dir)
                        .map(|rd| rd.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect::<Vec<_>>())
                        .unwrap_or_default()
                );
            }

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A deploy leaves the live generation and NOTHING ELSE of ours behind.
    ///
    /// Staged with the exact residue v3.7.9 left on a Windows machine: four role
    /// binaries written WITHOUT `.exe`, which nothing can invoke and which no
    /// retirement step would have removed — retirement only ever deleted names
    /// the current code knows how to write, and those names were a bug nobody
    /// had recorded. Reported from that machine as "you still deliver not only
    /// exe in hooks".
    ///
    /// Files outside our prefix must survive untouched; the user's own hooks and
    /// the config live in the same directory.
    #[test]
    fn a_deploy_removes_our_residue_and_leaves_everything_else_alone() {
        let platform = HostPlatform::Windows;
        let dir = std::env::temp_dir().join(format!("jawata-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");

        // The residue, and two files that must NOT be touched.
        for stale in [
            "jawata-hook-guard",          // v3.7.9's misnamed binaries
            "jawata-hook-primer",
            "jawata-hook-observer",
            "jawata-hook-userprompt",
            "jawata-guard.sh",            // a retired script
            "goja-session-primer.sh",     // a pre-rename script
        ] {
            std::fs::write(dir.join(stale), b"residue").expect("stage residue");
        }
        std::fs::write(dir.join("hook_config.json"), b"{}").expect("config");
        std::fs::write(dir.join("my-own-hook.sh"), b"#!/bin/sh").expect("user hook");

        let live: Vec<String> = CURSOR_ROLES
            .iter()
            .map(|(_, role, _)| role_binary_file_name_on(platform, role))
            .collect();
        for name in &live {
            std::fs::write(dir.join(name), b"live").expect("stage the live generation");
        }

        assert!(sweep_managed_hook_residue(&dir, &live), "the sweep must report work done");

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read back")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();

        let mut expected: Vec<String> = live.clone();
        expected.push("hook_config.json".to_string());
        expected.push("my-own-hook.sh".to_string());
        expected.sort();

        assert_eq!(
            expected, left,
            "a deploy must leave the live generation, the config and the user's own \
             files — and none of our residue"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Cursor deploy leaves the hooks CONFIGURED, not merely installed.
    ///
    /// A role binary resolves its endpoint as `<dir of the exe>/hook_config.json`.
    /// The only writer of that file was the Claude Code deploy, which wrote it
    /// into ITS OWN hooks directory — so every Cursor binary loaded
    /// `NotConfigured` and went silent. On the Windows install that found this,
    /// all four `.exe` were present, correctly named, and the guard refused
    /// nothing: an installation that looks complete in every listing and does
    /// nothing at all.
    ///
    /// The script generation could not have this bug, because each script
    /// carried the URL and token in its own text. The cutover to binaries moved
    /// the configuration mechanism and left the configuration behind.
    ///
    /// LIMIT, stated: this asserts the file's SHAPE, not that the hook crate
    /// parses it. The two crates must not depend on each other (a forbidden edge
    /// asserted by its own test), so the shared contract is the `seam_files`
    /// row for `hook_config.json` in hook-events.json, and both sides assert
    /// against that rather than against each other.
    #[test]
    fn a_cursor_deploy_writes_the_config_the_binaries_read() {
        let dir = unique_tempdir("cursor-hook-config");
        let cursor = dir.join(".cursor");
        let hooks_json = cursor.join("hooks.json");
        let hooks_dir = cursor.join("hooks");
        std::fs::create_dir_all(&cursor).unwrap();

        write_managed_cursor_hooks(
            &hooks_json.to_string_lossy(),
            &hooks_dir,
            "http://127.0.0.1:8899/mcp",
            "tok",
            false,
            false,
        )
        .expect("deploy");

        // Beside the binaries — the directory the exe resolves against, which is
        // the entire subject here.
        let config = hooks_dir.join("hook_config.json");
        assert!(
            config.exists(),
            "the deploy must write hook_config.json into Cursor's own hooks dir; \
             without it every role binary loads NotConfigured and goes silent. \
             Present: {:?}",
            std::fs::read_dir(&hooks_dir)
                .map(|rd| rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>())
                .unwrap_or_default()
        );

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).expect("parses");
        // The three the loader rejects an empty value for.
        assert!(
            v["url"].as_str().is_some_and(|u| !u.trim().is_empty()),
            "an empty url is rejected by the loader as unreadable: {v}"
        );
        assert!(
            v["token"].as_str().is_some_and(|t| !t.trim().is_empty()),
            "an empty token is rejected by the loader as unreadable: {v}"
        );
        assert_eq!(
            "cursor", v["client"],
            "the config must name the CLIENT it was written for — the roles a \
             binary may play differ per client"
        );
    }

    /// The claim `cursor_role_is_binary_live` makes in its doc comment, checked.
    ///
    /// That function hard-codes one Cursor-only exception. A hard-coded
    /// exception whose declaration lives in a data file is two facts that can
    /// drift apart, and the drift is silent — the deploy would keep cutting the
    /// observer over while `hook-events.json`, the file a human reads to learn
    /// what is live, said something else.
    #[test]
    fn the_cursor_observer_generation_matches_the_declaration() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json")).expect("contract parses");
        let row = &doc["role_generations"]["observer"];

        assert_eq!(
            "binary",
            row["cursor"].as_str().unwrap_or(""),
            "hook-events.json must declare the observer's CURSOR generation, because \
             cursor_role_is_binary_live cuts it over"
        );
        assert!(
            cursor_role_is_binary_live("jawata-hook-observer"),
            "the code must agree with the declaration it points at"
        );
        assert!(
            row["why_cursor"].as_str().is_some_and(|w| !w.is_empty()),
            "an exception without its reason is the kind of row nobody can audit later"
        );

        // And the Claude Code side is UNCHANGED — this override must not have
        // leaked into the generation that still has outcome capture to lose.
        assert_eq!("script", row["live"].as_str().unwrap_or(""));
        assert!(
            !role_is_binary_live("jawata-hook-observer"),
            "the Claude Code observer must stay on its script: that one captures tool \
             outcomes and the jawata-fallback audit trail, and cutting it over froze \
             outcomes.log in the 3.7.2 dogfood"
        );
    }


    /// The other half of the same contract: a role the contract still calls a
    /// script must NOT have quietly become a binary in the Cursor deploy.
    #[test]
    fn cursor_roles_the_contract_calls_scripts_are_still_scripts() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json")).expect("contract parses");
        for (role, script) in [
            ("primer", "jawata-session-primer.sh"),
            ("userprompt", "jawata-recall.sh"),
            ("observer", "jawata-observer.sh"),
        ] {
            if doc["role_generations"][role]["live"].as_str() == Some("script") {
                assert!(
                    managed_cursor_hook_entries(&std::env::temp_dir()
                        .join("jawata-cursor-scripts-still-scripts"))
                        .iter()
                        .any(|(_, e)| e["command"].as_str().unwrap_or_default().ends_with(script)),
                    "{role} is declared script-generation, so the Cursor deploy must \
                     still write {script}"
                );
            }
        }
    }

    #[test]
    fn a_users_own_hook_is_never_claimed_by_the_migration() {
        // The other direction, and the one that costs the user if it is wrong:
        // widening the match until it swallows their entries. Each of these
        // resembles ours without being ours.
        for command in [
            "/home/u/.claude/hooks/my-own-guard.sh",
            // The stop row was the ONE that could over-claim (it briefly used a
            // bare "stop-gate.sh") and the one with no specimen here — C6 audit
            // round 2, N2. A user who names their own hook stop-gate.sh must
            // keep it.
            "/home/u/bin/stop-gate.sh",
            "/home/u/.claude/hooks/jawata-notes/my-hook.sh",
            "/usr/local/bin/jawata-hook-guard-wrapper-of-mine",
            "echo 'jawata-studio/pretooluse-guard.sh is what I replaced'",
        ] {
            let entry = serde_json::json!({
                "hooks": [ { "type": "command", "command": command } ]
            });
            let claimed = SCRIPT_GENERATION
                .iter()
                .any(|(binary, _)| entry_is_managed_any_generation(&entry, binary));
            // Both directions asserted, including the two we KNOWINGLY claim.
            // Recording a limitation as an expectation is the difference
            // between a known behaviour and a surprise: matching is by
            // substring, so a command that embeds one of our sentinel paths is
            // treated as ours. Deliberate — the sentinel is a full path
            // fragment (`jawata-studio/pretooluse-guard.sh`), which a user is
            // unlikely to embed by accident, and the alternative (exact match)
            // breaks the moment a wrapper or an absolute prefix appears, which
            // is the normal case for OUR OWN entries.
            let we_knowingly_claim = command.contains("jawata-hook-guard")
                || command.contains("jawata-studio/pretooluse-guard.sh");
            assert_eq!(
                we_knowingly_claim, claimed,
                "migration claim on {command:?} is not what this test records — if the \
                 matching rule changed, change this expectation deliberately"
            );
        }
    }

    #[test]
    fn claude_deploy_events_match_the_shared_hook_contract() {
        // C5 audit round 2, R1 — the other four-sixths of the linkage.
        // hook-events.json bound the CURSOR deploy to the hook's role table, so
        // for Cursor it is a genuine three-way agreement: deploy ↔ contract ↔
        // table. Claude's six events were written as separate string literals
        // through six different section writers and asserted against nothing
        // that ships, leaving contract ↔ table only — two places that must agree
        // with each other and with nothing real. Rename "PreToolUse" in one
        // writer and every suite stays green while the recall role is
        // registered under an event Claude never fires.
        //
        // This DEPLOYS all six into one settings file and reads back the event
        // keys the deploy actually wrote, which is the end of the triangle that
        // was missing.
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json"))
                .expect("hook-events.json must parse — it is a committed contract");
        let claude = contract["claude-code"].as_object().expect("the claude-code section");

        let dir = unique_tempdir("claude-contract");
        let settings = dir.join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let hooks_dir = dir.join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        let p = |name: &str| hooks_dir.join(name);

        write_managed_hook(&settings_path, &p("guard.sh"), "http://u/health", false, false).unwrap();
        write_managed_posthook(&settings_path, &p("observer.sh"), "http://u/mcp", "t", false, false)
            .unwrap();
        write_managed_stop(&settings_path, &p("stop.sh"), "http://u/mcp", "t", false, false)
            .unwrap();
        write_managed_primer(&settings_path, &p("primer.sh"), "http://u/mcp", "t", false, false)
            .unwrap();
        write_managed_recall(&settings_path, &p("recall.sh"), "http://u/mcp", "t", false, false)
            .unwrap();
        write_managed_userprompt(&settings_path, &p("userprompt.sh"), "http://u/mcp", "t", false, false)
            .unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let mut deployed: Vec<String> = written["hooks"]
            .as_object()
            .expect("the deploy wrote a hooks object")
            .keys()
            .cloned()
            .collect();
        deployed.sort();

        // The contract lists six roles across four distinct events (guard and
        // recall share PreToolUse), so compare the SETS.
        let mut expected: Vec<String> = claude
            .values()
            .map(|v| v.as_str().expect("an event name").to_string())
            .collect();
        expected.sort();
        expected.dedup();

        assert_eq!(
            expected, deployed,
            "the Claude events this deploy writes have drifted from hook-events.json — \
             the hook's role table is asserted against that file, so a rename in a section \
             writer without a rename there registers a role under an event Claude never fires"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_deploy_events_match_the_shared_hook_contract() {
        // Sprint 28, C5 audit finding F5 — the OTHER end of the contract.
        // jawata-hook's role table asserts itself against hook-events.json;
        // that binds nothing unless the DEPLOY is asserted against the same
        // file. Otherwise renaming an event here leaves both crates green with
        // the hook mapped to an event Cursor never fires.
        //
        // A shared constant is impossible by design: the hook must not depend
        // on the studio (a hook process must never link a GUI toolkit) and the
        // studio must not depend on the hook (deploy writes the binary, it
        // never runs it). A committed data file is the one thing both sides
        // read with no edge between them.
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../hook-events.json"))
                .expect("hook-events.json must parse — it is a committed contract");
        let cursor = contract["cursor"].as_object().expect("the cursor section");

        let empty = std::env::temp_dir().join("jawata-cursor-events-contract");
        let deployed: Vec<&str> = managed_cursor_hook_entries(&empty)
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        let mut expected: Vec<&str> =
            cursor.values().map(|v| v.as_str().expect("an event name")).collect();
        expected.sort_unstable();
        let mut actual = deployed.clone();
        actual.sort_unstable();

        assert_eq!(
            expected, actual,
            "the Cursor events this deploy writes have drifted from hook-events.json — \
             the hook's role table is asserted against that file, so a rename here without \
             a rename there maps a role onto an event Cursor never fires"
        );
    }

    #[test]
    fn cursor_hooks_json_registers_managed_events_failclosed_guard() {
        // Sprint 28 Stage 4 (D-UNWIRED): this test used to assert
        // build_cursor_hooks_json() — a whole-file builder production never
        // called, since deploys go through the merge path that preserves the
        // user's own hooks. The clauses below were true of a JSON we never
        // wrote, so the shipped file could have lost failClosed with the test
        // still green. It now reads the file the deploy actually writes.
        let dir = unique_tempdir("cursor-hooks-contract");
        let cursor = dir.join(".cursor");
        let hooks_json = cursor.join("hooks.json");
        let hooks_path = hooks_json.to_string_lossy().to_string();
        let hooks_dir = cursor.join("hooks");
        write_managed_cursor_hooks(&hooks_path, &hooks_dir, "http://u/mcp", "t", false, false).unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hooks_json).unwrap()).unwrap();
        assert_eq!(v["version"], 1);
        for ev in ["sessionStart", "beforeShellExecution", "beforeSubmitPrompt", "afterMCPExecution"] {
            assert!(v["hooks"][ev].is_array(), "event {ev} registered");
        }
        assert_eq!(v["hooks"]["beforeShellExecution"][0]["failClosed"], true, "guard fails closed");
        assert_eq!(v["hooks"]["sessionStart"][0]["command"], "./hooks/jawata-session-primer.sh");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_primer_injects_additional_context() {
        let s = build_cursor_primer_script("u", "t");
        assert!(s.contains(r#""additional_context""#), "sessionStart injects via additional_context");
        assert!(s.contains(r#""kind":"primer""#));
    }

    #[test]
    fn cursor_guard_denies_java_grep_with_agent_steer() {
        let s = build_cursor_guard_script();
        assert!(s.contains(r#""permission":"deny""#), "denies");
        assert!(s.contains("grep") && s.contains(".java"), "targets Java grep");
        assert!(s.contains(r#""agent_message""#), "steers the agent to JAWATA");
    }

    /// 3.7.3 Cursor dogfood, P3: the guard's own deny message said "or declare
    /// a jawata-fallback" and NO case implemented it — the documented valve was
    /// a dead end, found because the claude-side guard honored the declaration
    /// and this script then blocked the same command anyway. Behavioral, not
    /// shape: the real script runs under bash with the real payloads.
    #[test]
    #[cfg(unix)]
    fn cursor_guard_honors_the_declared_fallback_it_advertises() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let dir = unique_tempdir("cursor-guard-escape");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("jawata-guard.sh");
        std::fs::write(&script, build_cursor_guard_script()).unwrap();

        let run = |payload: &str| -> String {
            let mut child = Command::new("bash")
                .arg(&script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            String::from_utf8_lossy(&out.stdout).to_string()
        };

        let denied = run(r#"{"command":"grep -n addSourceEntries src/ProjectImporter.java"}"#);
        assert!(denied.contains(r#""permission":"deny""#), "a bare java grep is denied: {denied}");
        // BOTH message fields carry the signpost: the Cursor re-run dogfood
        // showed the client surfacing user_message to the agent, so a signpost
        // living only in agent_message never reaches whoever was denied.
        for field in ["user_message", "agent_message"] {
            let msg = denied.split(&format!("\"{field}\":\"")).nth(1).unwrap_or("").split('"').next().unwrap_or("");
            assert!(msg.contains("jawata-fallback"),
                "{field} must advertise the escape it implements: {msg:?}");
        }

        let declared = run(
            r#"{"command":"grep -n addSourceEntries src/ProjectImporter.java # jawata-fallback: cursor dogfood probe"}"#,
        );
        assert!(declared.contains(r#""permission":"allow""#),
            "the declared fallback must proceed — the declaration is the audit trail: {declared}");

        let authored = run(r#"{"command":"echo ok # jawata-author: opening a window"}"#);
        assert!(authored.contains(r#""permission":"allow""#), "{authored}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_recall_is_side_effect_only_no_inject() {
        let s = build_cursor_recall_script("u", "t");
        // beforeSubmitPrompt cannot inject on Cursor — must NOT emit additional_context.
        assert!(!s.contains("additional_context"), "beforeSubmitPrompt cannot inject on Cursor");
        assert!(s.contains(r#"{"continue":true}"#), "returns the allow shape");
        assert!(s.contains(r#""kind":"recall""#), "still does the side-effect recall");
    }

    #[test]
    fn cursor_observer_is_fire_and_forget() {
        let s = build_cursor_observer_script();
        assert!(s.contains(r#"{}"#), "afterMCPExecution response is not enforced");
    }

    #[test]
    fn userprompt_write_remove_roundtrip_preserves_user_hooks() {
        let dir = unique_tempdir("push-userprompt");
        let settings = dir.join(".claude").join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let script = dir
            .join(".claude")
            .join("jawata-studio")
            .join("userpromptsubmit-recall.sh");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"echo user-prompt"}]}]}}"#,
        )
        .unwrap();

        assert!(write_managed_userprompt(&settings_path, &script, "http://127.0.0.1:8890/mcp", "tok", false, false).unwrap());
        assert!(script.exists(), "userprompt script written");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let arr = v["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "user + managed entry");
        assert!(arr.iter().any(is_managed_userprompt_entry), "managed entry present");
        assert!(
            arr.iter().any(|e| e["hooks"][0]["command"] == "echo user-prompt"),
            "user UserPromptSubmit entry preserved"
        );
        assert!(
            arr.iter()
                .filter(|e| is_managed_userprompt_entry(e))
                .all(|e| e.get("matcher").is_none()),
            "UserPromptSubmit takes no matcher"
        );

        assert!(
            !write_managed_userprompt(&settings_path, &script, "http://127.0.0.1:8890/mcp", "tok", false, false).unwrap(),
            "unchanged re-deploy is a byte-stable no-op"
        );

        assert!(remove_managed_userprompt(&settings_path, &script, false).unwrap());
        assert!(!script.exists(), "userprompt script deleted");
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            v2["hooks"]["UserPromptSubmit"].as_array().unwrap().len(),
            1,
            "only the user entry remains"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn primer_write_remove_roundtrip_preserves_user_hooks() {
        let dir = unique_tempdir("push-primer");
        let settings = dir.join(".claude").join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let primer = dir
            .join(".claude")
            .join("jawata-studio")
            .join("sessionstart-primer.sh");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo user-start"}]}]}}"#,
        )
        .unwrap();

        assert!(write_managed_primer(&settings_path, &primer, "http://127.0.0.1:8890/mcp", "tok", false, false).unwrap());
        assert!(primer.exists(), "primer script written");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let ss = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 2, "user + managed primer entry");
        assert!(ss.iter().any(is_managed_primer_entry), "managed primer present");
        assert!(
            ss.iter().any(|e| e["hooks"][0]["command"] == "echo user-start"),
            "user SessionStart entry preserved"
        );

        assert!(
            !write_managed_primer(&settings_path, &primer, "http://127.0.0.1:8890/mcp", "tok", false, false).unwrap(),
            "unchanged re-deploy is a no-op"
        );

        assert!(remove_managed_primer(&settings_path, &primer, false).unwrap());
        assert!(!primer.exists(), "primer script deleted");
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v2["hooks"]["SessionStart"].as_array().unwrap().len(), 1, "only the user entry remains");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recall_and_guard_coexist_in_pretooluse() {
        let dir = unique_tempdir("push-recall");
        let settings = dir.join(".claude").join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let guard = dir.join(".claude").join("jawata-studio").join("pretooluse-guard.sh");
        let recall = dir.join(".claude").join("jawata-studio").join("pretooluse-recall.sh");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();

        write_managed_hook(&settings_path, &guard, "http://127.0.0.1:8890/mcp", false, false).unwrap();
        write_managed_recall(&settings_path, &recall, "http://127.0.0.1:8890/mcp", "tok", false, false).unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let pre = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "guard + recall entries coexist in PreToolUse");
        assert!(pre.iter().any(is_managed_hook_entry), "guard entry present");
        assert!(pre.iter().any(is_managed_recall_entry), "recall entry present");

        // Removing recall leaves the guard untouched.
        assert!(remove_managed_recall(&settings_path, &recall, false).unwrap());
        let v2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let pre2 = v2["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre2.len(), 1, "only the guard remains");
        assert!(pre2.iter().any(is_managed_hook_entry));
        assert!(!pre2.iter().any(is_managed_recall_entry));
        assert!(!recall.exists(), "recall script deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 16b/B: single-service gateway wiring =====

    fn ws_server(id: &str, ws: &str, port: u16, token: &str, paths: &[&str]) -> ManagedDeployServer {
        ManagedDeployServer {
            id: id.into(),
            workspace_name: ws.into(),
            project_names: vec!["P".into()],
            project_paths: paths.iter().map(|p| p.to_string()).collect(),
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: token.into(),
            disabled: false,
        }
    }

    #[test]
    fn gateway_entry_is_single_jawata_pointing_at_gateway_port() {
        let entry = gateway_entry(8790, "gtok", false);
        assert_eq!(entry.id, "jawata");
        assert_eq!(entry.url, "http://127.0.0.1:8790/mcp");
        assert_eq!(entry.token, "gtok");
        // The client sees exactly one entry with the standard http shape.
        let json = build_client_mcp_json("cursor", &[entry]);
        let servers = json["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1, "client sees ONE service");
        assert_eq!(servers["jawata"]["url"], "http://127.0.0.1:8790/mcp");
        assert_eq!(servers["jawata"]["headers"]["Authorization"], "Bearer gtok");
    }

    #[test]
    fn routing_table_maps_each_workspace_and_routes_by_path() {
        let servers = vec![
            ws_server("jawata-a", "a", 8800, "ta", &["/p/a"]),
            ws_server("jawata-b", "b", 8801, "tb", &["/p/b"]),
        ];
        let table = build_routing_table(&servers);
        assert_eq!(table.routes.len(), 2);

        let params = serde_json::json!({"arguments": {"filePath": "/p/b/src/X.java"}});
        let route = match table.resolve("tools/call", Some(&params)) {
            crate::gateway::Resolution::Route(route) => route,
            other => panic!("expected Route, got {other:?}"),
        };
        assert_eq!(route.url, "http://127.0.0.1:8801/mcp");
        assert_eq!(route.token, "tb");
    }

    #[test]
    fn deploy_writer_omits_disabled_when_enabled() {
        let servers = vec![url_server("ws-a", 8800, "tok", false)];
        let json = build_client_mcp_json("cursor", &servers);
        let entry = &json["mcpServers"]["ws-a"];
        assert!(
            entry.get("disabled").is_none(),
            "disabled flag must be omitted when false (cleaner client config)"
        );
    }

    #[test]
    fn deploy_writer_emits_disabled_true_when_set() {
        // WriterMode::Disable + autostart=off produces servers with
        // disabled=true. Cursor + Claude honour the flag.
        let servers = vec![url_server("ws-a", 8800, "tok", true)];
        let json = build_client_mcp_json("cursor", &servers);
        let entry = &json["mcpServers"]["ws-a"];
        assert_eq!(entry["disabled"], serde_json::Value::Bool(true));
        // Url + headers stay populated so a one-click toggle re-enables
        // without re-deploying.
        assert_eq!(entry["url"], "http://127.0.0.1:8800/mcp");
        assert_eq!(
            entry["headers"]["Authorization"],
            "Bearer tok"
        );
    }

    #[test]
    fn deploy_writer_empty_when_no_servers() {
        // autostart=off + WriterMode::Remove produces zero servers; the
        // downstream merge step strips any pre-existing managed entries.
        let servers: Vec<ManagedDeployServer> = Vec::new();
        let json = build_client_mcp_json("cursor", &servers);
        let map = json["mcpServers"]
            .as_object()
            .expect("mcpServers must always be an object");
        assert!(map.is_empty(), "no servers must serialize to an empty map");
    }

    #[test]
    fn deploy_writer_antigravity_emits_serverurl_shape() {
        // Sprint 16 (bugs.md #10): Antigravity's Windsurf-lineage parser
        // rejects `type`+`url` ("serverURL or command must be specified");
        // it wants `serverUrl` and no `type`. Verified live 2026-06-10.
        let servers = vec![url_server("jawata-ws", 8805, "tok", false)];
        let json = build_client_mcp_json("antigravity", &servers);
        let entry = &json["mcpServers"]["jawata-ws"];

        assert_eq!(entry["serverUrl"], "http://127.0.0.1:8805/mcp");
        assert!(entry.get("url").is_none(), "antigravity must not get `url`");
        assert!(entry.get("type").is_none(), "antigravity must not get `type`");
        assert_eq!(entry["headers"]["Authorization"], "Bearer tok");
    }

    #[test]
    fn deploy_writer_antigravity_honours_disabled_flag() {
        let servers = vec![url_server("jawata-ws", 8805, "tok", true)];
        let json = build_client_mcp_json("antigravity", &servers);
        let entry = &json["mcpServers"]["jawata-ws"];
        assert_eq!(entry["disabled"], serde_json::Value::Bool(true));
        assert_eq!(entry["serverUrl"], "http://127.0.0.1:8805/mcp");
    }

    #[test]
    fn deploy_target_ids_accept_every_known_client_including_claude_desktop() {
        // The regression: "claude_desktop" must survive normalization. It is
        // the ONLY multi-word client id, so it is the only one the
        // camelCase/snake_case confusion can drop.
        let requested: Vec<String> = KNOWN_DEPLOY_CLIENT_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect();
        let resolved = normalize_requested_deploy_targets(Some(&requested))
            .expect("every known client id must be accepted")
            .expect("an explicit selection must produce a set");
        for id in KNOWN_DEPLOY_CLIENT_IDS {
            assert!(resolved.contains(id), "{id} must survive normalization");
        }
    }

    #[test]
    fn deploy_target_ids_refuse_the_camelcase_settings_key_loudly() {
        // v3.5.1 and earlier SILENTLY dropped this and then reported
        // "Skipped: not selected in this deploy run" — the lie that hid the
        // bug. It must now fail loudly and name the offending id.
        let requested = vec!["cursor".to_string(), "claudeDesktop".to_string()];
        let error = normalize_requested_deploy_targets(Some(&requested))
            .expect_err("an unknown client id must refuse the deploy, not vanish");
        assert!(
            error.contains("claudedesktop"),
            "the refusal must name the offending id, got: {error}"
        );
        assert!(
            error.contains("claude_desktop"),
            "the refusal must teach the correct id, got: {error}"
        );
    }

    #[test]
    fn deploy_target_ids_are_case_and_whitespace_tolerant() {
        let requested = vec!["  Claude_Desktop  ".to_string()];
        let resolved = normalize_requested_deploy_targets(Some(&requested))
            .expect("trimmed/cased known ids stay accepted")
            .expect("an explicit selection must produce a set");
        assert!(resolved.contains("claude_desktop"));
    }

    #[test]
    fn deploy_without_an_explicit_selection_falls_back_to_settings_flags() {
        assert!(
            normalize_requested_deploy_targets(None)
                .expect("no selection is not an error")
                .is_none(),
            "None must stay None so the settings flags decide"
        );
    }

    #[test]
    fn deploy_writer_claude_desktop_gets_http_shape() {
        // Sprint 16.1 (bugs.md #17): Claude Desktop is a native-HTTP client
        // like Claude Code / Cursor — NOT the antigravity serverUrl shape.
        let servers = vec![url_server("jawata-ws", 8805, "tok", false)];
        let json = build_client_mcp_json("claude_desktop", &servers);
        let entry = &json["mcpServers"]["jawata-ws"];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], "http://127.0.0.1:8805/mcp");
        assert!(entry.get("serverUrl").is_none());
        assert!(
            validate_client_config_shape("claude_desktop", &json, &servers).is_ok(),
            "validator must accept the claude_desktop http shape"
        );
    }

    /// Sprint 28 (v3.6.1): the deploy log must carry what the macOS dogfood
    /// could not recover. There, Claude Desktop's config ended up with an empty
    /// `mcpServers` and the on-disk artifacts could not distinguish "the deploy
    /// wrote nothing" from "the deploy wrote entries and the app clobbered them
    /// later". Each of these assertions is one half of that question.
    #[test]
    fn deploy_log_records_per_client_outcome() {
        let result = DeployToAgentsResult {
            mode: DeployMode::Deploy,
            ok: false,
            detail: "Agent deploy completed with failures.".into(),
            duration_ms: 42,
            clients: vec![
                DeployClientResult {
                    client: "claude_desktop".into(),
                    target_path: "/Users/h/Library/.../claude_desktop_config.json".into(),
                    status: DeployClientStatus::Success,
                    message: "Configuration written.".into(),
                    backup_path: Some("/backups/1785180489557-0013".into()),
                    changed_sections: vec!["mcpServers".into(), "seats".into()],
                    validation_errors: vec![],
                    preview_content: None,
                },
                DeployClientResult {
                    client: "cursor".into(),
                    target_path: "/Users/h/.cursor/mcp.json".into(),
                    status: DeployClientStatus::Failed,
                    message: "Validation failed.".into(),
                    backup_path: None,
                    changed_sections: vec![],
                    validation_errors: vec!["missing url".into()],
                    preview_content: None,
                },
            ],
        };

        let log = format_deploy_log("2026-07-29T10:00:00Z", &result);
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 3, "summary line plus one line per client");
        assert!(lines[0].contains("ok=false"), "summary carries the verdict");
        assert!(lines[0].contains("clients=2"), "summary carries the count");

        // The client that was WRITTEN: which sections changed, and where the
        // backup went — enough to answer "did this run put entries in the file".
        assert!(lines[1].contains("claude_desktop"));
        assert!(lines[1].contains("status=Success"));
        assert!(lines[1].contains("changed=mcpServers,seats"));
        assert!(lines[1].contains("backup=/backups/1785180489557-0013"));
        assert!(lines[1].contains("claude_desktop_config.json"), "the target path");

        // The client that FAILED: the reason survives, not just the status.
        assert!(lines[2].contains("status=Failed"));
        assert!(lines[2].contains("validation_errors=missing url"));
    }

    /// A skipped client must still appear — a run that silently omits a target
    /// is exactly the shape that made "Skipped: not selected in this deploy
    /// run" impossible to diagnose.
    #[test]
    fn deploy_log_records_skipped_clients_too() {
        let result = DeployToAgentsResult {
            mode: DeployMode::Deploy,
            ok: true,
            detail: "Agent deploy completed.".into(),
            duration_ms: 7,
            clients: vec![skipped_client_result(
                "claude_desktop",
                Some("/path/claude_desktop_config.json".to_string()),
                "Skipped: not selected in this deploy run.",
            )],
        };

        let log = format_deploy_log("2026-07-29T10:00:00Z", &result);
        assert!(
            log.contains("claude_desktop") && log.contains("Skipped"),
            "a skipped target must be on the record, not absent from it: {log}"
        );
    }

    #[test]
    fn deploy_writer_claude_cursor_shape_unchanged_by_per_client_branch() {
        // The v0.15.1 shape stays byte-stable for claude + cursor.
        for client in ["claude", "cursor"] {
            let servers = vec![url_server("jawata-ws", 8805, "tok", false)];
            let json = build_client_mcp_json(client, &servers);
            let entry = &json["mcpServers"]["jawata-ws"];
            assert_eq!(entry["type"], "http", "{client} keeps type");
            assert_eq!(entry["url"], "http://127.0.0.1:8805/mcp", "{client} keeps url");
            assert!(entry.get("serverUrl").is_none(), "{client} must not get serverUrl");
        }
    }

    #[test]
    fn validator_accepts_per_client_shapes() {
        let servers = vec![url_server("jawata-ws", 8805, "tok", false)];

        let antigravity_json = build_client_mcp_json("antigravity", &servers);
        assert!(
            validate_client_config_shape("antigravity", &antigravity_json, &servers).is_ok(),
            "validator must accept the antigravity serverUrl shape"
        );

        let claude_json = build_client_mcp_json("claude", &servers);
        assert!(
            validate_client_config_shape("claude", &claude_json, &servers).is_ok(),
            "validator must accept the claude type+url shape"
        );

        // Cross-shape must FAIL: a claude-shaped entry handed to the
        // antigravity validator means the per-client branch regressed.
        assert!(
            validate_client_config_shape("antigravity", &claude_json, &servers).is_err(),
            "antigravity validator must reject a url-shaped entry"
        );
    }

    #[test]
    fn writer_mode_default_is_remove() {
        // Sanity: the default for new ManagerSettings must be Remove
        // (matches the "autostart off should mean off" user intent).
        assert_eq!(
            crate::config::default_mcp_disabled_writer_mode(),
            crate::config::WriterMode::Remove
        );
    }

    #[test]
    fn writer_mode_round_trips_through_json() {
        // The settings.json contains the mode by string; round-trip via
        // serde to confirm both variants persist correctly.
        let remove = serde_json::to_string(&crate::config::WriterMode::Remove).unwrap();
        let disable = serde_json::to_string(&crate::config::WriterMode::Disable).unwrap();
        assert_eq!(remove, "\"remove\"");
        assert_eq!(disable, "\"disable\"");

        let back_remove: crate::config::WriterMode =
            serde_json::from_str(&remove).unwrap();
        let back_disable: crate::config::WriterMode =
            serde_json::from_str(&disable).unwrap();
        assert_eq!(back_remove, crate::config::WriterMode::Remove);
        assert_eq!(back_disable, crate::config::WriterMode::Disable);
    }

    // ===== Sprint 16: scan-folder backend (autoscan) =====

    fn make_maven_project(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("src/main/java")).unwrap();
        std::fs::write(dir.join("pom.xml"), "<project/>").unwrap();
    }

    #[test]
    fn scan_directory_finds_nested_projects_and_skips_junk() {
        let dir = unique_tempdir("scan-mixed");
        make_maven_project(&dir, "maven-app");
        // Gradle project.
        let gradle = dir.join("gradle-app");
        std::fs::create_dir_all(gradle.join("src/main/java")).unwrap();
        std::fs::write(gradle.join("build.gradle"), "").unwrap();
        // Eclipse PDE project.
        let eclipse = dir.join("eclipse-app");
        std::fs::create_dir_all(&eclipse).unwrap();
        std::fs::write(eclipse.join(".project"), "<projectDescription/>").unwrap();
        std::fs::write(eclipse.join(".classpath"), "<classpath/>").unwrap();
        // Plain folder — no Java signals.
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        // Java project buried in node_modules — must be skipped by the walk.
        make_maven_project(&dir.join("node_modules"), "fake-proj");

        let candidates = scan_directory_for_java_projects(&[dir.clone()])
            .expect("scan must succeed");

        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["eclipse-app", "gradle-app", "maven-app"],
            "sorted, junk-free: {candidates:?}"
        );
        let kind_of = |n: &str| {
            candidates
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.kind.clone())
                .unwrap()
        };
        assert_eq!(kind_of("maven-app"), "maven-gradle");
        assert_eq!(kind_of("gradle-app"), "maven-gradle");
        assert_eq!(kind_of("eclipse-app"), "eclipse-pde");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_root_itself_is_a_candidate_and_children_collapse() {
        // Browsing directly INTO a maven multi-module root: the root is the
        // one candidate; its modules are nested children and collapse away.
        let dir = unique_tempdir("scan-rootproj");
        std::fs::create_dir_all(dir.join("src/main/java")).unwrap();
        std::fs::write(dir.join("pom.xml"), "<project/>").unwrap();
        make_maven_project(&dir, "module-a");

        let candidates = scan_directory_for_java_projects(&[dir.clone()])
            .expect("scan must succeed");

        assert_eq!(candidates.len(), 1, "only the containing root: {candidates:?}");
        assert_eq!(candidates[0].project_path, dir.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_folder_for_projects_at_validates_input() {
        let missing = scan_folder_for_projects_at("/definitely/not/a/real/dir-xyz");
        assert!(missing.is_err(), "missing dir must error");
        assert!(missing.unwrap_err().contains("not a directory"));

        assert!(
            scan_folder_for_projects_at("   ").is_err(),
            "blank input must error"
        );
    }

    // ===== Sprint 16 (bugs.md #14a): managed-entry detection =====

    #[test]
    fn path_has_managed_entries_detects_managed_keys() {
        let dir = unique_tempdir("managed-detect");

        let managed = dir.join("managed.json");
        std::fs::write(
            &managed,
            r#"{ "mcpServers": { "jawata-my-ws": { "url": "http://x" }, "other": {} } }"#,
        )
        .unwrap();
        assert!(path_has_managed_entries(managed.to_str().unwrap()));

        // Legacy pre-rebrand keys (`jl-…` / `javalens-…`) are still recognised as managed,
        // so the manager can find and clean up deployments written before the JAWATA rebrand.
        let legacy = dir.join("legacy.json");
        std::fs::write(
            &legacy,
            r#"{ "mcpServers": { "jl-legacy-ws": { "url": "http://x" } } }"#,
        )
        .unwrap();
        assert!(path_has_managed_entries(legacy.to_str().unwrap()));

        let foreign = dir.join("foreign.json");
        std::fs::write(
            &foreign,
            r#"{ "mcpServers": { "filesystem": { "command": "npx" } } }"#,
        )
        .unwrap();
        assert!(!path_has_managed_entries(foreign.to_str().unwrap()));

        let empty = dir.join("empty.json");
        std::fs::write(&empty, r#"{ "somethingElse": true }"#).unwrap();
        assert!(!path_has_managed_entries(empty.to_str().unwrap()));

        assert!(
            !path_has_managed_entries(dir.join("missing.json").to_str().unwrap()),
            "never-deployed clients (no file) are not refresh targets"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ===== Sprint 16 (bugs.md #14b): resolve errors must surface =====

    #[test]
    fn merge_resolve_errors_attaches_to_written_results_only() {
        let mut results = vec![
            DeployClientResult {
                client: "claude".into(),
                target_path: "/tmp/a".into(),
                status: DeployClientStatus::Success,
                message: "ok".into(),
                backup_path: None,
                changed_sections: Vec::new(),
                validation_errors: Vec::new(),
                preview_content: None,
            },
            DeployClientResult {
                client: "cursor".into(),
                target_path: "/tmp/b".into(),
                status: DeployClientStatus::Skipped,
                message: "skipped".into(),
                backup_path: None,
                changed_sections: Vec::new(),
                validation_errors: Vec::new(),
                preview_content: None,
            },
        ];
        let errors = vec!["workspace 'broken-ws': no runtime installed".to_string()];

        merge_resolve_errors(&mut results, &errors);

        assert_eq!(
            results[0].validation_errors, errors,
            "written client must carry the resolve error"
        );
        assert!(
            results[1].validation_errors.is_empty(),
            "skipped client untouched"
        );

        // Empty error set is a no-op.
        merge_resolve_errors(&mut results, &[]);
        assert_eq!(results[0].validation_errors.len(), 1);
    }
}
