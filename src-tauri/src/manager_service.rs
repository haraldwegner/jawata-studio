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
struct ManagedDeployServer {
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
                    eprintln!("[goja-studio] gateway listening on 127.0.0.1:{}", handle.port)
                }
                Err(error) => eprintln!("[goja-studio] gateway failed to start: {error}"),
            }
        }

        Self {
            config_store,
            release_manager,
            runtime_manager,
            routing_table,
        }
    }

    /// Loads the current manager dashboard state.
    pub fn load_dashboard(&self) -> Result<ManagerDashboard, String> {
        self.build_dashboard(true)
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
    /// `workspace.json` so any running goja for that workspace picks
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
    /// running goja processes drop / pick up the project via the
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
    /// goja subprocess for the workspace, deletes the JDT data dir,
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
    /// `workspace.json` so the running goja drops the project via the
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
    /// before spawning any goja process. Multiple projects sharing
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
    /// If the `release_repo` value changed, triggers a fresh release-status
    /// re-poll so the dashboard immediately reflects the new repo's latest
    /// release rather than showing the cached status from the previous repo.
    pub fn update_settings(&self, input: UpdateSettingsInput) -> Result<ManagerDashboard, String> {
        let previous_repo = self.config_store.get_settings().release_repo.clone();
        let updated = self.config_store.update_settings(input)?;
        let release_repo_changed = updated.release_repo != previous_repo;
        self.build_dashboard(release_repo_changed)
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
        let projects = self.config_store.list_projects();
        let (servers, resolve_errors) = self.build_deploy_servers(&settings, &projects);

        // Sprint 16b/B: with the gateway on, refresh its routing table and write
        // ONE `goja` entry to clients instead of N per-workspace entries. Off by
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
        let requested_targets: Option<HashSet<String>> =
            input.target_clients.as_ref().map(|targets| {
                targets
                    .iter()
                    .map(|target| target.trim().to_ascii_lowercase())
                    .filter(|target| {
                        matches!(
                            target.as_str(),
                            "cursor" | "claude" | "claude_desktop" | "antigravity" | "intellij"
                        )
                    })
                    .collect()
            });

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

        Ok(DeployToAgentsResult {
            mode: input.mode,
            ok,
            detail,
            duration_ms: started_at.elapsed().as_millis(),
            clients: results,
        })
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

    /// Downloads or updates the GOJA runtime.
    pub fn download_or_update_goja(&self) -> Result<ManagerDashboard, String> {
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
    /// goja picks up the full workspace member list.
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

        // Tell goja to drop this project: rewrite workspace.json
        // without it (the file watcher in goja will call removeProject
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
        // Eclipse JDT data dir + one goja process.
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
                    .ok_or_else(|| "No managed GOJA runtime is installed. Download the latest release first.".to_string())?;

                Ok(RuntimeReference {
                    project_id: project.id.clone(),
                    workspace_name: project.workspace_name.clone(),
                    workspace_dir,
                    runtime_label: format!("Managed GOJA {}", runtime.version),
                    resolved_jar_path: runtime.jar_path.clone(),
                    resident_port: workspace_state.resident_port,
                    resident_token: workspace_state.resident_token,
                })
            }
            RuntimeSource::LocalJar { jar_path } => Ok(RuntimeReference {
                project_id: project.id.clone(),
                workspace_name: project.workspace_name.clone(),
                workspace_dir,
                runtime_label: "Local GOJA JAR".into(),
                resolved_jar_path: jar_path.clone(),
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
    /// member list. Running goja processes pick up the change via
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
    /// hold goja-managed entries, so deployed configs track workspace
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
                "[goja-studio] auto-refresh of deployed configs completed \
                 with failures: {}",
                result.detail
            ),
            Err(error) => eprintln!(
                "[goja-studio] auto-refresh of deployed configs failed: {error}"
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

        if matches!(mode, DeployMode::Preview) {
            return DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Success,
                message: "Preview generated.".into(),
                backup_path: None,
                changed_sections: vec!["mcpConfig".into(), "rules".into()],
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
                changed_sections: vec!["mcpConfig".into(), "rules".into()],
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
                    message: "No managed GOJA deploy sections found.".into(),
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
                message: "Delete successful. Removed managed GOJA deploy sections.".into(),
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

        if errors.is_empty() {
            DeployClientResult {
                client: client.to_string(),
                target_path: path,
                status: DeployClientStatus::Success,
                message: "Deploy successful.".into(),
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
                    "No managed GOJA runtime is installed. Download latest first.".to_string()
                })?;
                ProbeRuntime {
                    jar_path: runtime.jar_path,
                    runtime_label: format!("Managed GOJA {}", runtime.version),
                }
            }
            RuntimeSource::LocalJar { jar_path } => ProbeRuntime {
                jar_path: jar_path.clone(),
                runtime_label: "Local GOJA JAR".into(),
            },
        };

        if !PathBuf::from(&runtime.jar_path).exists() {
            return Err(format!(
                "Configured GOJA JAR does not exist: {}",
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
                    format!("Failed to start GOJA probe process: {error}"),
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
                        "name": "goja-studio",
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
                "received non-JSON output from GOJA stdout: {trimmed}"
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
        "cursor" => display_path(&parent.join("rules").join("goja-studio.mdc")),
        "claude" => display_path(&parent.join("CLAUDE.md")),
        "antigravity" => display_path(&parent.join("AGENTS.md")),
        "intellij" => display_path(&parent.join("goja-studio-rules.md")),
        _ => display_path(&parent.join("goja-studio-rules.md")),
    }
}

/// Sprint 16b/C: the client's GLOBAL / always-loaded instruction file — the one
/// loaded into every session regardless of cwd. The deploy writes the managed
/// rule block here IN ADDITION to the config-sibling (`derive_rule_path`) so the
/// "use GOJA, not grep" rule survives MCP schema deferral.
///
/// - `claude` → `~/.claude/CLAUDE.md`. The sibling for Claude Code is `~/CLAUDE.md`
///   (next to `~/.claude.json`), which is NOT always-loaded; `~/.claude/CLAUDE.md`
///   is. This is the gap the rebrand left stale.
/// - `cursor` → `None`: the default Cursor sibling is already `~/.cursor/rules/
///   goja-studio.mdc` (a global rules dir), so the sibling already covers it.
/// - `antigravity` / others → `None`: no confirmed always-loaded global file;
///   don't guess a path. Revisit if/when one is confirmed.
fn derive_global_rule_path(client: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    match client {
        "claude" => Some(display_path(&home.join(".claude").join("CLAUDE.md"))),
        _ => None,
    }
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
    // delivery vehicle for "use goja, not grep, for Java" — the prior
    // one-line policy was too vague to change agent behaviour. Two imperative
    // sections: a Java→goja routing table, then the TDD-refactor loop.
    // Keep it tight and scannable; a long rule gets ignored. Identical text for
    // every client (only the marker name differs) so the idempotent
    // marker-replace in write_managed_rule_block stays simple.
    let mut lines = vec![
        format!("<!-- goja-studio:{client}:start -->"),
        "## GOJA MCP — use it for Java, before shell text tools".to_string(),
        String::new(),
        "These workspaces are served by GOJA MCP (compiler-accurate, JDT-backed). For \
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
        "Shell text search is a FALLBACK only — when GOJA is unavailable, or for \
         non-Java / non-semantic matches (build files, configs, comments, log strings)."
            .to_string(),
        String::new(),
        "## Refactor in small, verified steps".to_string(),
        String::new(),
        "1. Confirm a green baseline (`compile_workspace`; run the relevant tests).".to_string(),
        "2. Apply ONE refactoring via a GOJA tool (it returns a diff + `undoChangeId`)."
            .to_string(),
        "3. Re-check: `compile_workspace` + run the tests again.".to_string(),
        "4. Green → keep going. Red → `undo_refactoring` and rethink. One step at a time."
            .to_string(),
        String::new(),
        "Managed service ids:".to_string(),
    ];
    for server in servers {
        lines.push(format!("- {}", server.id));
    }
    lines.push(format!("<!-- goja-studio:{client}:end -->"));
    lines.join("\n")
}

/// Cursor enforces `len(server_id) + 1 + len(tool_name) <= 59` (reports as "exceeds 60 characters").
/// Antigravity is limited by a separate ~100 *services* / tool-budget; no shared constant here.
const CURSOR_MCP_COMBINED_MAX: usize = 59;
/// Upper bound on a single goja-mcp tool name length (e.g. `get_call_hierarchy_outgoing` ~ 28; keep buffer for future tools).
const GOJA_TOOL_NAME_BUDGET: usize = 32;

fn max_mcp_server_id_len_for_cursor() -> usize {
    CURSOR_MCP_COMBINED_MAX
        .saturating_sub(1) // ":"
        .saturating_sub(GOJA_TOOL_NAME_BUDGET)
}

/// Sprint 10 v0.10.4: MCP service ID derived from the workspace name.
/// Format: `goja-<sanitized-workspace-name>`, capped at the Cursor server-id
/// budget. Single-workspace mode means each MCP service represents one
/// logical workspace, not one project.
fn mcp_server_id_for_workspace(workspace_name: &str) -> String {
    let max_id = max_mcp_server_id_len_for_cursor();
    let prefix = "goja-";
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

/// Keys for MCP servers written by goja-studio: `goja-…`, plus legacy `jl-…` /
/// `javalens-…` recognised for cleanup/migration of pre-rebrand deploys.
fn is_goja_managed_mcp_key(key: &str) -> bool {
    key.starts_with("goja-") || key.starts_with("jl-") || key.starts_with("javalens-")
}

/// Sprint 16 (bugs.md #14a): true when the client's MCP config file already
/// carries at least one goja-managed server entry — the marker that the
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
        .map(|servers| servers.keys().any(|key| is_goja_managed_mcp_key(key)))
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

    // Merge managed GOJA servers into the client's real MCP schema.
    // Clients load "mcpServers", not our internal gojaManager metadata.
    if let Some(object) = next_value.as_object_mut() {
        let mut existing_servers = object
            .get("mcpServers")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();

        let incoming_ids: HashSet<String> =
            servers.iter().map(|server| server.id.clone()).collect();
        let should_prune_managed =
            force_rewrite || matches!(merge_mode, McpMergeMode::ReplaceManagedSection);
        if should_prune_managed {
            existing_servers.retain(|key, _| !is_goja_managed_mcp_key(key));
        }

        // Sprint 15 Stage 11: URL form replaces stdio command/args/env.
        // Sprint 16 (bugs.md #10): the entry shape is per-client — see
        // managed_server_entry for the schema table.
        for server in servers {
            existing_servers.insert(server.id.clone(), managed_server_entry(client, server));
        }

        if force_rewrite {
            existing_servers.retain(|key, _| {
                !is_goja_managed_mcp_key(key) || incoming_ids.contains(key)
            });
        }

        object.insert(
            "mcpServers".into(),
            serde_json::Value::Object(existing_servers),
        );
        // Remove legacy payload from earlier deploy versions.
        object.remove("gojaManager");
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

    if backup_before_write && path_buf.exists() {
        let backup_path = format!("{path}.bak-{}", crate::config::current_timestamp_string());
        fs::copy(&path_buf, &backup_path).map_err(|error| {
            format!(
                "failed creating backup {} from {}: {error}",
                backup_path,
                path_buf.display()
            )
        })?;
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
        existing_servers.retain(|key, _| !is_goja_managed_mcp_key(key));
        changed |= existing_servers.len() != previous_len;
        object.insert(
            "mcpServers".into(),
            serde_json::Value::Object(existing_servers),
        );
        changed |= object.remove("gojaManager").is_some();
    }

    if !changed {
        return Ok(false);
    }

    if backup_before_write && path_buf.exists() {
        let backup_path = format!("{path}.bak-{}", crate::config::current_timestamp_string());
        fs::copy(&path_buf, &backup_path).map_err(|error| {
            format!(
                "failed creating backup {} from {}: {error}",
                backup_path,
                path_buf.display()
            )
        })?;
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
    // `alwaysLoad` flag — mark the managed GOJA server so its (post-collapse)
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

/// Sprint 16b/B: the single client-facing `goja` entry that points at the gateway.
fn gateway_entry(port: u16, token: &str, disabled: bool) -> ManagedDeployServer {
    ManagedDeployServer {
        id: "goja".to_string(),
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

    let next = if let (Some(start_idx), Some(end_idx)) =
        (existing.find(start_marker), existing.find(end_marker))
    {
        let end_inclusive = end_idx + end_marker.len();
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

    if backup_before_write && path_buf.exists() {
        let backup_path = format!("{path}.bak-{}", crate::config::current_timestamp_string());
        fs::copy(&path_buf, &backup_path).map_err(|error| {
            format!(
                "failed creating rule backup {} from {}: {error}",
                backup_path,
                path_buf.display()
            )
        })?;
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
    let start_marker = format!("<!-- goja-studio:{client}:start -->");
    let end_marker = format!("<!-- goja-studio:{client}:end -->");

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

    if backup_before_write && path_buf.exists() {
        let backup_path = format!("{path}.bak-{}", crate::config::current_timestamp_string());
        fs::copy(&path_buf, &backup_path).map_err(|error| {
            format!(
                "failed creating rule backup {} from {}: {error}",
                backup_path,
                path_buf.display()
            )
        })?;
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

fn latest_backup_path(_path: &str) -> Option<String> {
    None
}

/// Sprint 10 v0.10.4: atomic write of `workspace.json` for one workspace.
/// Lifted out of `ManagerService` so it can be unit-tested without the
/// full ConfigStore + ReleaseManager + RuntimeManager dependency graph.
///
/// Behavior:
/// - `paths.is_empty()` → the file is removed if present (no member =
///   no workspace.json on disk).
/// - Otherwise: writes to a `.tmp` sibling and renames atomically so the
///   `WorkspaceFileWatcher` in goja never observes a half-written
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
        assert_eq!(id, "goja-alpha");
    }

    #[test]
    fn mcp_server_id_for_workspace_normalizes_special_chars() {
        // mcp_label_slug lowercases and replaces non-alphanumerics with `-`
        // (collapsing consecutive). The exact slug shape is internal but
        // the result must be a valid Cursor server id (only [a-z0-9-_]).
        let id = mcp_server_id_for_workspace("My Workspace!");
        assert!(id.starts_with("goja-"));
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn mcp_server_id_for_workspace_long_name_fits_cursor_budget() {
        // Cursor's combined-id cap is around 59-60 chars. Whatever the
        // workspace name length, the produced id must fit within that
        // cap so the longest tool name still passes.
        let long = "a".repeat(200);
        let id = mcp_server_id_for_workspace(&long);
        assert!(id.starts_with("goja-"));
        assert!(id.len() <= max_mcp_server_id_len_for_cursor());
    }

    #[test]
    fn mcp_server_id_for_workspace_empty_falls_back_to_hash() {
        // Pure whitespace produces an empty slug after sanitization;
        // mcp_server_id_for_workspace falls back to a deterministic hash
        // suffix so the id is still unique-ish and parseable.
        let id = mcp_server_id_for_workspace("   ");
        assert!(id.starts_with("goja-"));
        assert!(id.len() > "goja-".len(), "empty name must yield a hash-suffixed id, got '{id}'");
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
            "goja-studio-mstest-{label}-{}-{}-{}",
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
        let servers = vec![url_server("jl-ws-a", 8800, "tok", false)];
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
        assert!(block.starts_with("<!-- goja-studio:cursor:start -->"));
        assert!(block.trim_end().ends_with("<!-- goja-studio:cursor:end -->"));
        assert!(block.contains("Managed service ids:"));
        assert!(block.contains("- jl-ws-a"));
    }

    #[test]
    fn rule_block_marker_name_is_per_client_but_body_is_identical() {
        let servers = vec![url_server("jl-ws-a", 8800, "tok", false)];
        let cursor = build_rule_block("cursor", &servers);
        let claude = build_rule_block("claude", &servers);
        assert!(cursor.contains("goja-studio:cursor:start"));
        assert!(claude.contains("goja-studio:claude:start"));
        // Strip the client-specific markers; the guidance body must match.
        let strip = |s: &str, c: &str| {
            s.replace(&format!("<!-- goja-studio:{c}:start -->"), "")
                .replace(&format!("<!-- goja-studio:{c}:end -->"), "")
        };
        assert_eq!(strip(&cursor, "cursor"), strip(&claude, "claude"));
    }

    #[test]
    fn rule_block_is_deterministic_idempotent() {
        let servers = vec![
            url_server("jl-ws-a", 8800, "tok-a", false),
            url_server("jl-ws-b", 8801, "tok-b", false),
        ];
        // Same inputs → byte-identical output (so a re-deploy is a no-op write).
        assert_eq!(
            build_rule_block("claude", &servers),
            build_rule_block("claude", &servers)
        );
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
        let servers = vec![url_server("goja-ws", 8800, "tok", false)];
        let json = build_client_mcp_json("claude", &servers);
        assert_eq!(
            json["mcpServers"]["goja-ws"]["alwaysLoad"],
            serde_json::Value::Bool(true),
            "Claude entry must carry alwaysLoad:true so the surface never defers"
        );
    }

    #[test]
    fn non_claude_entries_omit_always_load() {
        let servers = vec![url_server("goja-ws", 8800, "tok", false)];
        for client in ["cursor", "antigravity", "intellij", "claude_desktop"] {
            let json = build_client_mcp_json(client, &servers);
            assert!(
                json["mcpServers"]["goja-ws"].get("alwaysLoad").is_none(),
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
        let servers = vec![url_server("goja-ws", 8800, "tok", false)];
        let block = build_rule_block("claude", &servers);

        // (1) NEW FILE: parent dir created, block written.
        write_managed_rule_block(&path, &block, false, false).unwrap();
        let after_new = std::fs::read_to_string(&file).unwrap();
        assert!(after_new.contains("<!-- goja-studio:claude:start -->"));
        assert!(after_new.contains("GOJA MCP"));

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
            appended.contains("<!-- goja-studio:claude:start -->"),
            "block appended"
        );

        // (4) REPLACE BETWEEN MARKERS: a stale block is spliced out, user text kept.
        let stale = "# Header\n\n<!-- goja-studio:claude:start -->\nOLD STALE BODY\n<!-- goja-studio:claude:end -->\n\n# Footer\n";
        let replace_file = dir.join("replace.md");
        let replace_path = replace_file.to_string_lossy().to_string();
        std::fs::write(&replace_file, stale).unwrap();
        write_managed_rule_block(&replace_path, &block, false, false).unwrap();
        let replaced = std::fs::read_to_string(&replace_file).unwrap();
        assert!(replaced.contains("# Header"), "leading user text kept");
        assert!(replaced.contains("# Footer"), "trailing user text kept");
        assert!(!replaced.contains("OLD STALE BODY"), "stale body replaced");
        assert!(replaced.contains("GOJA MCP"), "new body present");

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
    fn gateway_entry_is_single_goja_pointing_at_gateway_port() {
        let entry = gateway_entry(8790, "gtok", false);
        assert_eq!(entry.id, "goja");
        assert_eq!(entry.url, "http://127.0.0.1:8790/mcp");
        assert_eq!(entry.token, "gtok");
        // The client sees exactly one entry with the standard http shape.
        let json = build_client_mcp_json("cursor", &[entry]);
        let servers = json["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1, "client sees ONE service");
        assert_eq!(servers["goja"]["url"], "http://127.0.0.1:8790/mcp");
        assert_eq!(servers["goja"]["headers"]["Authorization"], "Bearer gtok");
    }

    #[test]
    fn routing_table_maps_each_workspace_and_routes_by_path() {
        let servers = vec![
            ws_server("goja-a", "a", 8800, "ta", &["/p/a"]),
            ws_server("goja-b", "b", 8801, "tb", &["/p/b"]),
        ];
        let table = build_routing_table(&servers);
        assert_eq!(table.routes.len(), 2);

        let params = serde_json::json!({"arguments": {"filePath": "/p/b/src/X.java"}});
        let route = table.resolve("tools/call", Some(&params)).unwrap();
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
        let servers = vec![url_server("jl-ws", 8805, "tok", false)];
        let json = build_client_mcp_json("antigravity", &servers);
        let entry = &json["mcpServers"]["jl-ws"];

        assert_eq!(entry["serverUrl"], "http://127.0.0.1:8805/mcp");
        assert!(entry.get("url").is_none(), "antigravity must not get `url`");
        assert!(entry.get("type").is_none(), "antigravity must not get `type`");
        assert_eq!(entry["headers"]["Authorization"], "Bearer tok");
    }

    #[test]
    fn deploy_writer_antigravity_honours_disabled_flag() {
        let servers = vec![url_server("jl-ws", 8805, "tok", true)];
        let json = build_client_mcp_json("antigravity", &servers);
        let entry = &json["mcpServers"]["jl-ws"];
        assert_eq!(entry["disabled"], serde_json::Value::Bool(true));
        assert_eq!(entry["serverUrl"], "http://127.0.0.1:8805/mcp");
    }

    #[test]
    fn deploy_writer_claude_desktop_gets_http_shape() {
        // Sprint 16.1 (bugs.md #17): Claude Desktop is a native-HTTP client
        // like Claude Code / Cursor — NOT the antigravity serverUrl shape.
        let servers = vec![url_server("jl-ws", 8805, "tok", false)];
        let json = build_client_mcp_json("claude_desktop", &servers);
        let entry = &json["mcpServers"]["jl-ws"];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], "http://127.0.0.1:8805/mcp");
        assert!(entry.get("serverUrl").is_none());
        assert!(
            validate_client_config_shape("claude_desktop", &json, &servers).is_ok(),
            "validator must accept the claude_desktop http shape"
        );
    }

    #[test]
    fn deploy_writer_claude_cursor_shape_unchanged_by_per_client_branch() {
        // The v0.15.1 shape stays byte-stable for claude + cursor.
        for client in ["claude", "cursor"] {
            let servers = vec![url_server("jl-ws", 8805, "tok", false)];
            let json = build_client_mcp_json(client, &servers);
            let entry = &json["mcpServers"]["jl-ws"];
            assert_eq!(entry["type"], "http", "{client} keeps type");
            assert_eq!(entry["url"], "http://127.0.0.1:8805/mcp", "{client} keeps url");
            assert!(entry.get("serverUrl").is_none(), "{client} must not get serverUrl");
        }
    }

    #[test]
    fn validator_accepts_per_client_shapes() {
        let servers = vec![url_server("jl-ws", 8805, "tok", false)];

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
            r#"{ "mcpServers": { "jl-my-ws": { "url": "http://x" }, "other": {} } }"#,
        )
        .unwrap();
        assert!(path_has_managed_entries(managed.to_str().unwrap()));

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
