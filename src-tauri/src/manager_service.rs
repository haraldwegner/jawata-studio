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

        Ok(DeployToAgentsResult {
            mode: input.mode,
            ok,
            detail,
            duration_ms: started_at.elapsed().as_millis(),
            clients: results,
        })
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
    // Keep it tight and scannable; a long rule gets ignored. Identical text for
    // every client (only the marker name differs) so the idempotent
    // marker-replace in write_managed_rule_block stays simple.
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
        "Managed service ids:".to_string(),
    ];
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
        let should_prune_managed =
            force_rewrite || matches!(merge_mode, McpMergeMode::ReplaceManagedSection);
        if should_prune_managed {
            existing_servers.retain(|key, _| !is_managed_mcp_key(key));
        }

        // Sprint 15 Stage 11: URL form replaces stdio command/args/env.
        // Sprint 16 (bugs.md #10): the entry shape is per-client — see
        // managed_server_entry for the schema table.
        for server in servers {
            existing_servers.insert(server.id.clone(), managed_server_entry(client, server));
        }

        if force_rewrite {
            existing_servers.retain(|key, _| {
                !is_managed_mcp_key(key) || incoming_ids.contains(key)
            });
        }

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
fn claude_scripts_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let new = home.join(".claude").join("jawata-studio");
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
fn managed_guard_script_path() -> Option<PathBuf> {
    Some(claude_scripts_dir()?.join("pretooluse-guard.sh"))
}

/// Absolute path of the managed PostToolUse observer script (sibling of the guard).
fn managed_observer_script_path() -> Option<PathBuf> {
    Some(claude_scripts_dir()?.join("posttooluse-observer.sh"))
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
    {{
      echo "USE A JAWATA REFACTOR TOOL — hand-editing $edit_path (a .java file) is blocked."
      echo "Rename → rename_symbol (updates ALL references). Move → move / move_in_hierarchy."
      echo "Extract method/variable/constant/superclass → extract. Duplicate a class → generate(kind=copy_class)."
      echo "Any structural change → refactoring(action=plan) then apply_plan (parity-gated, reversible)."
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
            { "type": "command", "command": command }
        ]
    })
}

/// True iff a `PreToolUse` entry is one jawata-studio wrote (its command references
/// the managed guard script). Used to replace/remove our entries and leave the
/// user's hooks alone.
fn is_managed_hook_entry(entry: &serde_json::Value) -> bool {
    entry
        .get("hooks")
        .and_then(|hooks| hooks.as_array())
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|command| command.as_str())
                    .map(|command| command.contains(JAWATA_HOOK_SENTINEL)
                        || command.contains(&legacy_sentinel(JAWATA_HOOK_SENTINEL)))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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
    let script_body = build_guard_script(health_url);
    let script_changed = fs::read_to_string(guard_path)
        .map(|existing| existing != script_body)
        .unwrap_or(true);
    if script_changed || force_rewrite {
        fs::write(guard_path, &script_body).map_err(|error| {
            format!("failed writing guard script {}: {error}", guard_path.display())
        })?;
    }
    #[cfg(unix)]
    {
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
  reason="$(printf '%s' "$flat" | sed -nE 's/.*[Gg][Oo][Jj][Aa]-[Ff][Aa][Ll][Ll][Bb][Aa][Cc][Kk]:[[:space:]]*([^"\\]*).*/\1/p' | head -n1 | sed -E 's/[[:space:]]*$//')"
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
  *compile_workspace|*get_diagnostics|*run_tests|*find_tests)
    emit "verify" "$tool_name"
    ;;
  Edit|Write|MultiEdit)
    # v1.5.1: a slip counts only for a .java edit the PRE edit-gate allowed via the marker —
    # a non-.java edit whose CONTENT merely contains 'jawata-fallback:' is not a gated op.
    ef="$(printf '%s' "$flat" | grep -oE '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed -E 's/.*"([^"]*)"$/\1/')"
    case "$ef" in
      *.java) printf '%s' "$flat" | grep -qiE 'jawata-fallback:' && emit_slip ;;
    esac
    ;;
  Bash|Grep)
    # v1.5.1: a slip counts only for a Java symbol SEARCH the PRE search-gate allowed —
    # require a content-search tool AND a .java target (the PRE dual-gate) plus the marker.
    st=0
    if [ "$tool_name" = "Grep" ]; then st=1
    elif printf '%s' "$flat" | grep -qiE '(^|[^a-zA-Z])(grep|egrep|fgrep|rg|ripgrep|ag|ack)([^a-zA-Z]|$)'; then st=1; fi
    if [ "$st" = "1" ] \
       && printf '%s' "$flat" | grep -qiE '([A-Za-z0-9_$]\.java|\*\.java)' \
       && printf '%s' "$flat" | grep -qiE 'jawata-fallback:'; then emit_slip; fi
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
fn selftest_hook_script(script: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    if !script.exists() {
        return Ok(());
    }
    let output = match Command::new("bash")
        .arg(script)
        .env("JAWATA_HOOK_SELFTEST", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(_) => return Ok(()),          // no bash on this platform — cannot judge
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

/// The single `PostToolUse` matcher entry that invokes the observer. Broad matcher:
/// Read (ungrounded-read capture), the verify MCP tools, and search/edit tools (slip
/// capture); the script no-ops on anything else.
fn build_managed_posthook_entry(observer_path: &Path) -> serde_json::Value {
    let command = display_path(observer_path);
    serde_json::json!({
        "matcher": "Bash|Grep|Edit|Write|MultiEdit|Read|mcp__jawata.*",
        "hooks": [
            { "type": "command", "command": command }
        ]
    })
}

/// True iff a `PostToolUse` entry is one jawata-studio wrote (its command references the
/// managed observer script).
fn is_managed_posthook_entry(entry: &serde_json::Value) -> bool {
    entry
        .get("hooks")
        .and_then(|hooks| hooks.as_array())
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|command| command.as_str())
                    .map(|command| command.contains(JAWATA_POSTHOOK_SENTINEL)
                        || command.contains(&legacy_sentinel(JAWATA_POSTHOOK_SENTINEL)))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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
    let script_body = build_observer_script(mcp_url, token);
    let script_changed = fs::read_to_string(observer_path)
        .map(|existing| existing != script_body)
        .unwrap_or(true);
    if script_changed || force_rewrite {
        fs::write(observer_path, &script_body).map_err(|error| {
            format!("failed writing observer script {}: {error}", observer_path.display())
        })?;
    }
    #[cfg(unix)]
    {
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
    Some(claude_scripts_dir()?.join("sessionstart-primer.sh"))
}

/// Absolute path of the managed PreToolUse recall script (sibling of the guard).
fn managed_recall_script_path() -> Option<PathBuf> {
    Some(claude_scripts_dir()?.join("pretooluse-recall.sh"))
}

/// Absolute path of the managed UserPromptSubmit recall script (Sprint 21c item D).
fn managed_userprompt_script_path() -> Option<PathBuf> {
    Some(claude_scripts_dir()?.join("userpromptsubmit-recall.sh"))
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

fn is_managed_primer_entry(entry: &serde_json::Value) -> bool {
    entry_command_contains(entry, JAWATA_PRIMER_SENTINEL)
}
fn is_managed_recall_entry(entry: &serde_json::Value) -> bool {
    entry_command_contains(entry, JAWATA_RECALL_SENTINEL)
}

/// SessionStart entry: no matcher (fires on every session start).
fn build_managed_primer_entry(primer_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": [ { "type": "command", "command": display_path(primer_path) } ]
    })
}
/// PreToolUse entry for recall: fires on jawata tool calls (the script gates to refactor verbs).
fn build_managed_recall_entry(recall_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "matcher": "mcp__jawata.*",
        "hooks": [ { "type": "command", "command": display_path(recall_path) } ]
    })
}
fn is_managed_userprompt_entry(entry: &serde_json::Value) -> bool {
    entry_command_contains(entry, JAWATA_USERPROMPT_SENTINEL)
}
/// UserPromptSubmit entry: no matcher (fires on every user prompt; the script gates itself).
fn build_managed_userprompt_entry(script_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": [ { "type": "command", "command": display_path(script_path) } ]
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
    let script_changed = fs::read_to_string(script_path)
        .map(|existing| existing != script_body)
        .unwrap_or(true);
    if script_changed || force_rewrite {
        fs::write(script_path, script_body)
            .map_err(|error| format!("failed writing hook script {}: {error}", script_path.display()))?;
    }
    #[cfg(unix)]
    {
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
# Sprint 21c (item D): prompt -> keywords -> recall -> injected FACT. Extracts content-
# bearing cues from the user's prompt (longest n-grams first, rarity-marked tokens
# preferred within a tier, >=2 content tokens), asks the store terminally, and injects
# the ONE fitting atomic fact — or nothing. Never a pile, never a guess, never blocks.
set -u
MCP_URL="__MCP_URL__"
TOKEN="__TOKEN__"
# THE emit path — selftest and the live path share this one printf format (Sprint 21a item J).
emit_ctx() {
  printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"JAWATA recalled a prior fact for this topic:\\n%s"}}' "$1"
}
if [ "${JAWATA_HOOK_SELFTEST:-}" = "1" ]; then emit_ctx '[lesson] selftest canned line (accepted)'; exit 0; fi
command -v curl >/dev/null 2>&1 || exit 0
# THE recall attempt (Sprint 22a dual-cue): $1 = arg key (symbol|symptom), $2 = cue.
# On a single fitting fact it injects and exits 0; otherwise returns so the next-
# ranked cue is tried. Single-fact-or-silence: any \n in data = 2+ facts = skip.
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
  # Terminal-or-absence: any \n in data = 2+ fitting facts = ambiguous -> next cue.
  case "$data" in *"\n"*) return 1 ;; esac
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
case "$cmd" in
  *grep*.java*|*\ rg\ *.java*|*sed*.java*|*awk*.java*)
    printf '%s\n' '{"continue":true,"permission":"deny","user_message":"Blocked: use JAWATA MCP for Java semantic search.","agent_message":"Shell text search on .java is blocked — call search_symbols / find_references via JAWATA MCP (or declare a jawata-fallback)."}'
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

/// The four managed (event, entry) pairs — the SINGLE source for both the standalone
/// `build_cursor_hooks_json` and the merge-into-the-user's-file path, so they never drift.
fn managed_cursor_hook_entries() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("sessionStart", serde_json::json!({ "command": "./hooks/jawata-session-primer.sh", "timeout": 15 })),
        ("beforeShellExecution", serde_json::json!({ "command": "./hooks/jawata-guard.sh", "timeout": 5, "failClosed": true, "matcher": "grep |rg |sed |awk " })),
        ("beforeSubmitPrompt", serde_json::json!({ "command": "./hooks/jawata-recall.sh", "timeout": 5 })),
        ("afterMCPExecution", serde_json::json!({ "command": "./hooks/jawata-observer.sh", "timeout": 5 })),
    ]
}

/// The managed Cursor `hooks.json` (version 1) registering the four managed scripts, with
/// command paths relative to `~/.cursor/` per the spec. The guard is `failClosed` so a
/// crash/timeout blocks the Java-grep rather than leaking it.
fn build_cursor_hooks_json() -> String {
    let mut hooks = serde_json::Map::new();
    for (event, entry) in managed_cursor_hook_entries() {
        hooks.insert(event.to_string(), serde_json::Value::Array(vec![entry]));
    }
    serde_json::json!({ "version": 1, "hooks": hooks }).to_string()
}

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
    // 1. Write the four managed scripts (jawata-studio owns them outright).
    fs::create_dir_all(hooks_dir)
        .map_err(|e| format!("failed to create cursor hooks dir {}: {e}", hooks_dir.display()))?;
    let scripts: [(&str, String); 4] = [
        ("jawata-session-primer.sh", build_cursor_primer_script(mcp_url, token)),
        ("jawata-guard.sh", build_cursor_guard_script()),
        ("jawata-recall.sh", build_cursor_recall_script(mcp_url, token)),
        ("jawata-observer.sh", build_cursor_observer_script()),
    ];
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
        for (event, entry) in managed_cursor_hook_entries() {
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
    fn rule_block_marker_name_is_per_client_but_body_is_identical() {
        let servers = vec![url_server("jawata-ws-a", 8800, "tok", false)];
        let cursor = build_rule_block("cursor", &servers);
        let claude = build_rule_block("claude", &servers);
        assert!(cursor.contains("jawata-studio:cursor:start"));
        assert!(claude.contains("jawata-studio:claude:start"));
        // Strip the client-specific markers; the guidance body must match.
        let strip = |s: &str, c: &str| {
            s.replace(&format!("<!-- jawata-studio:{c}:start -->"), "")
                .replace(&format!("<!-- jawata-studio:{c}:end -->"), "")
        };
        assert_eq!(strip(&cursor, "cursor"), strip(&claude, "claude"));
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
        assert!(hooks_dir.join("jawata-guard.sh").exists(), "new script written");
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
        // The guard is failClosed.
        let guard = hooks["beforeShellExecution"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["command"] == "./hooks/jawata-guard.sh")
            .unwrap();
        assert_eq!(guard["failClosed"], true, "guard is failClosed");
        // Scripts written + executable; the recall script baked the url + token.
        for name in ["jawata-session-primer.sh", "jawata-guard.sh", "jawata-recall.sh", "jawata-observer.sh"] {
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
        // guard: empty stdin -> allow -> valid JSON with permission=allow.
        if let Ok(out) = Command::new("bash")
            .arg(hooks_dir.join("jawata-guard.sh"))
            .stdin(Stdio::null())
            .output()
        {
            let v: serde_json::Value =
                serde_json::from_slice(&out.stdout).expect("guard emits valid JSON");
            assert_eq!(v["permission"], "allow", "an empty command is allowed");
        }
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
    fn userprompt_script_extracts_cues_and_injects_single_fact_only() {
        // Sprint 21c (item D): prompt -> keywords -> recall -> the ONE fitting fact.
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
        // Terminal-or-absence at the injection boundary: entry lines are \n-joined, so
        // any \n in the peeled data = 2+ fitting facts = ambiguous -> next cue, never a pile.
        assert!(
            s.contains(r#"*"\n"*) return 1"#),
            "multi-fact answers are ambiguous — try the next-ranked cue"
        );
        assert!(s.contains("No\\ known\\ knowledge"), "silent on absence");
        assert!(s.contains("--max-time 2"), "short per-attempt timeout");
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
    fn cursor_hooks_json_registers_managed_events_failclosed_guard() {
        let v: serde_json::Value = serde_json::from_str(&build_cursor_hooks_json()).unwrap();
        assert_eq!(v["version"], 1);
        for ev in ["sessionStart", "beforeShellExecution", "beforeSubmitPrompt", "afterMCPExecution"] {
            assert!(v["hooks"][ev].is_array(), "event {ev} registered");
        }
        assert_eq!(v["hooks"]["beforeShellExecution"][0]["failClosed"], true, "guard fails closed");
        assert_eq!(v["hooks"]["sessionStart"][0]["command"], "./hooks/jawata-session-primer.sh");
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
