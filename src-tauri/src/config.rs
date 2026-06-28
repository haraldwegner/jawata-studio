use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const APP_NAME: &str = "goja-studio";
const PROJECTS_FILE_NAME: &str = "projects.json";
const SETTINGS_FILE_NAME: &str = "settings.json";
const RUNTIME_STATE_FILE_NAME: &str = "runtime-state.json";

/// Initial configuration and state paths required for the application to start.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStatus {
    pub config_dir: String,
    pub state_dir: String,
    pub cache_dir: String,
    pub projects_file: String,
    pub settings_file: String,
    pub runtime_state_file: String,
    pub default_data_root: String,
    pub log_dir: String,
    pub transport: String,
    pub health_strategy: String,
}

/// Policy determining how application updates should be handled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePolicy {
    Always,
    Ask,
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self::Ask
    }
}

fn default_data_root() -> String {
    String::new()
}

fn default_global_runtime_source() -> RuntimeSource {
    RuntimeSource::Managed
}

fn default_use_system_tray() -> bool {
    true
}

/// Sprint 14 (v0.14.0): manager does NOT auto-launch at login by
/// default — the user opts in via Settings > "Autostart on boot" or
/// the tray checkable. Avoids surprising power users who don't want a
/// tray-resident process they didn't approve.
fn default_autostart_on_boot() -> bool {
    false
}

fn default_mcp_merge_mode() -> McpMergeMode {
    McpMergeMode::SafeMerge
}

fn default_mcp_backup_before_write() -> bool {
    true
}

fn default_deploy_targets() -> DeployTargetFlags {
    DeployTargetFlags::default()
}

/// Strategy for merging MCP configuration changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpMergeMode {
    SafeMerge,
    ReplaceManagedSection,
}

impl Default for McpMergeMode {
    fn default() -> Self {
        McpMergeMode::SafeMerge
    }
}

/// Configuration for a specific MCP client's configuration file path.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpClientPathEntry {
    #[serde(default)]
    pub auto_detected_path: Option<String>,
    #[serde(default)]
    pub manual_override_path: Option<String>,
    #[serde(default)]
    pub effective_path: Option<String>,
}

/// Collection of paths to various MCP client configuration files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpClientPaths {
    #[serde(default)]
    pub cursor: McpClientPathEntry,
    /// Claude Code (CLI) — `~/.claude.json`.
    #[serde(default)]
    pub claude: McpClientPathEntry,
    /// Sprint 16.1 (bugs.md #17): Claude Desktop (GUI app) —
    /// `<config-dir>/Claude/claude_desktop_config.json`. A distinct client
    /// from Claude Code: different file, different process.
    #[serde(default)]
    pub claude_desktop: McpClientPathEntry,
    #[serde(default)]
    pub antigravity: McpClientPathEntry,
    #[serde(default)]
    pub intellij: McpClientPathEntry,
}

/// Flags indicating which MCP clients should receive deployments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployTargetFlags {
    #[serde(default = "default_enabled_flag")]
    pub cursor: bool,
    #[serde(default = "default_enabled_flag")]
    pub claude: bool,
    #[serde(default = "default_enabled_flag")]
    pub claude_desktop: bool,
    #[serde(default = "default_enabled_flag")]
    pub antigravity: bool,
    #[serde(default = "default_enabled_flag")]
    pub intellij: bool,
}

fn default_enabled_flag() -> bool {
    true
}

impl Default for DeployTargetFlags {
    fn default() -> Self {
        Self {
            cursor: true,
            claude: true,
            claude_desktop: true,
            antigravity: true,
            intellij: true,
        }
    }
}

fn default_mcp_client_paths() -> McpClientPaths {
    detect_default_mcp_client_paths()
}

/// Global settings for the GOJA manager application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSettings {
    pub version: u32,
    pub update_policy: UpdatePolicy,
    pub auto_check_for_updates: bool,
    pub manual_fallback_jar_path: Option<String>,
    #[serde(default = "default_data_root")]
    pub data_root: String,
    #[serde(default = "default_global_runtime_source")]
    pub global_runtime_source: RuntimeSource,
    #[serde(default = "default_use_system_tray")]
    pub use_system_tray: bool,
    /// Sprint 14 (v0.14.0): if true, the manager registers itself with
    /// the OS so it auto-launches at session login (Linux:
    /// ~/.config/autostart/*.desktop, macOS: LaunchAgent, Windows:
    /// registry Run key). Surfaced in Settings AND as a tray checkable.
    #[serde(default = "default_autostart_on_boot")]
    pub autostart_on_boot: bool,
    #[serde(default = "default_mcp_client_paths")]
    pub mcp_client_paths: McpClientPaths,
    #[serde(default = "default_mcp_merge_mode")]
    pub mcp_merge_mode: McpMergeMode,
    #[serde(default = "default_mcp_backup_before_write")]
    pub mcp_backup_before_write: bool,
    #[serde(default = "default_deploy_targets")]
    pub deploy_targets: DeployTargetFlags,
    /// Sprint 15 Stage 11: how the MCP-config writer behaves when
    /// `autostart_on_boot` is OFF. `Remove` (default) strips the managed
    /// servers from `~/.cursor/mcp.json` / `~/.claude.json` entirely;
    /// `Disable` writes them with `disabled: true` for users who want
    /// visible-but-inert entries they can toggle on with one click.
    #[serde(default = "default_mcp_disabled_writer_mode")]
    pub mcp_disabled_writer_mode: WriterMode,
    #[serde(default = "default_release_repo")]
    pub release_repo: String,
    pub last_release_check: Option<String>,
    pub last_seen_latest_version: Option<String>,
    /// Sprint 16b/B: single-service gateway. When enabled, the deploy writes ONE
    /// `goja` MCP entry (the gateway) instead of N per-workspace entries, and the
    /// in-process gateway routes each call to the owning resident. Default OFF —
    /// the per-workspace deploy is unchanged until this is turned on.
    #[serde(default = "default_gateway_enabled")]
    pub gateway_enabled: bool,
    /// Stable port the gateway binds on 127.0.0.1 (below the resident range).
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,
    /// Stable Bearer token for the gateway entry; generated once on first enable.
    #[serde(default)]
    pub gateway_token: Option<String>,
}

/// Sprint 15 Stage 11: governs the MCP-config writer's behaviour when
/// `autostart_on_boot` is OFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriterMode {
    /// Strip managed goja entries from `~/.cursor/mcp.json` etc.
    /// Clients see no entry at all — autostart=off ↔ no MCP server.
    Remove,
    /// Write the entries with `disabled: true`. Cursor + Claude both
    /// honour this flag; the entry stays visible in the client's MCP
    /// list but is inert until the user re-enables.
    Disable,
}

pub fn default_mcp_disabled_writer_mode() -> WriterMode {
    WriterMode::Remove
}

/// Default GitHub repo for the managed GOJA runtime release stream.
///
/// Sprint 15 Stage 12: GitHub username rename hw1964 → haraldwegner
/// (2026-06-07). The redirect doesn't reliably follow at the
/// `releases/latest` API endpoint for anonymous polls, so an explicit
/// migration is necessary — see `OLD_DEFAULT_RELEASE_REPO` below.
///
/// Earlier history: hw1964/javalens-mcp was the fork that shipped the
/// source-resolution fixes (pom.xml <sourceDirectory>, Eclipse .classpath)
/// vs the original upstream `pzalutski-pixel/javalens-mcp`.
///
/// Override per-user by editing settings.json or via the
/// `GOJA_RELEASE_REPO` env var. Format: "<owner>/<repo>".
pub fn default_release_repo() -> String {
    "haraldwegner/goja-mcp".to_string()
}

/// Sprint 16b/B: gateway is OFF by default — per-workspace deploy is unchanged
/// until the user opts in.
fn default_gateway_enabled() -> bool {
    false
}

/// Sprint 16b/B: gateway port. 8790 sits just below the resident range
/// (8800–8999) so it never collides with a per-workspace resident.
fn default_gateway_port() -> u16 {
    8790
}

/// Legacy values rewritten to the current default on read.
/// - `pzalutski-pixel/javalens-mcp` — the pre-v0.10.0 upstream default.
/// - `hw1964/javalens-mcp` — the pre-v0.15.0 fork default (before the
///   2026-06-07 GitHub username rename to haraldwegner). The GitHub
///   redirect on `releases/latest` is unreliable for anonymous polls, so
///   the manager rewrites stale config to the new URL on startup.
const LEGACY_DEFAULT_RELEASE_REPOS: &[&str] = &[
    "pzalutski-pixel/javalens-mcp",
    "hw1964/javalens-mcp",
    "haraldwegner/javalens-mcp",
];

impl ManagerSettings {
    pub(crate) fn default_for_paths(paths: &AppPaths) -> Self {
        Self {
            version: 1,
            update_policy: UpdatePolicy::Ask,
            auto_check_for_updates: true,
            manual_fallback_jar_path: None,
            data_root: display_path(&paths.default_data_root),
            global_runtime_source: RuntimeSource::Managed,
            use_system_tray: default_use_system_tray(),
            autostart_on_boot: default_autostart_on_boot(),
            mcp_client_paths: detect_default_mcp_client_paths(),
            mcp_merge_mode: default_mcp_merge_mode(),
            mcp_backup_before_write: default_mcp_backup_before_write(),
            deploy_targets: default_deploy_targets(),
            mcp_disabled_writer_mode: default_mcp_disabled_writer_mode(),
            release_repo: default_release_repo(),
            last_release_check: None,
            last_seen_latest_version: None,
            gateway_enabled: default_gateway_enabled(),
            gateway_port: default_gateway_port(),
            gateway_token: None,
        }
    }

    pub fn tools_dir(&self) -> PathBuf {
        PathBuf::from(&self.data_root)
            .join("tools")
            .join("goja")
    }

    pub fn workspace_root(&self) -> PathBuf {
        PathBuf::from(&self.data_root).join("workspaces")
    }
}

/// Source of the GOJA runtime environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RuntimeSource {
    Managed,
    LocalJar { jar_path: String },
}

impl RuntimeSource {
    pub fn label(&self) -> String {
        match self {
            RuntimeSource::Managed => "Managed GOJA (Latest)".into(),
            RuntimeSource::LocalJar { jar_path } => format!("Local JAR ({jar_path})"),
        }
    }
}

/// Information about a registered Java project.
///
/// Sprint 10 v0.10.4: `workspace_name` identifies the logical workspace this
/// project belongs to. Multiple projects sharing a `workspace_name` run as
/// one MCP service (one goja process per workspace). The legacy
/// `assigned_port` field is preserved on disk for one release cycle to
/// support migration from v0.10.3-format projects.json — at runtime it is
/// ignored. To be removed in Sprint 11.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub project_path: String,
    /// Logical workspace this project belongs to. Required from v0.10.4 on.
    /// On v0.10.3 → v0.10.4 migration the manager derives this from
    /// `assigned_port` if missing (e.g. `"workspace-11100"`).
    #[serde(default)]
    pub workspace_name: String,
    /// Legacy v0.10.3 field. Kept on disk for one release cycle to support
    /// migration of existing projects.json files. Removed in Sprint 11.
    #[serde(default)]
    pub assigned_port: u16,
}

/// Input data for registering a new project.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddProjectInput {
    pub name: String,
    pub project_path: String,
    /// The logical workspace to add this project to. If empty/missing,
    /// a default `"workspace-default"` is used.
    #[serde(default)]
    pub workspace_name: String,
}

/// Input data for updating the manager settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettingsInput {
    pub update_policy: UpdatePolicy,
    pub auto_check_for_updates: bool,
    pub data_root: String,
    pub global_runtime_source: RuntimeSource,
    pub use_system_tray: bool,
    pub autostart_on_boot: bool,
    pub mcp_client_paths: McpClientPaths,
    pub mcp_merge_mode: McpMergeMode,
    pub mcp_backup_before_write: bool,
    pub deploy_targets: DeployTargetFlags,
    /// Optional: when omitted, current settings.release_repo is preserved.
    /// Lets older frontend builds save settings without resetting this field.
    #[serde(default)]
    pub release_repo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProjectsFile {
    version: u32,
    projects: Vec<ProjectRecord>,
    /// Sprint 15 Stage 9: per-workspace resident-JVM bookkeeping
    /// (`(port, token)` pairs persisted across manager restarts).
    /// `#[serde(default)]` so v0.14.x-format projects.json files load
    /// without migration noise.
    #[serde(default)]
    workspaces: Vec<crate::resident::WorkspaceState>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyProjectRecord {
    id: String,
    name: String,
    project_path: String,
    javalens_jar_path: String,
    workspace_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyProjectsFile {
    version: Option<u32>,
    projects: Vec<LegacyProjectRecord>,
}

/// Core filesystem paths used by the application.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub projects_file: PathBuf,
    pub settings_file: PathBuf,
    pub runtime_state_file: PathBuf,
    pub default_data_root: PathBuf,
    pub log_dir: PathBuf,
}

/// The pre-rebrand application directory name. Sprint 16a migrates
/// `<base>/javalens-manager` -> `<base>/goja-studio` on first launch so an
/// existing user's workspaces/settings survive the rebrand.
const OLD_APP_NAME: &str = "javalens-manager";

/// Best-effort one-time move of `<base>/javalens-manager` ->
/// `<base>/goja-studio`. Logs on move/error; never blocks startup.
fn migrate_legacy_app_dir(base: &Path, label: &str) {
    let old = base.join(OLD_APP_NAME);
    let new = base.join(APP_NAME);
    match migrate_dir_if_needed(&old, &new) {
        Ok(true) => eprintln!(
            "[goja-studio] migrated {label} dir: {} -> {}",
            old.display(),
            new.display()
        ),
        Ok(false) => {}
        Err(e) => eprintln!(
            "[goja-studio] WARN: could not migrate {label} dir {} -> {}: {e} (continuing with a fresh dir)",
            old.display(),
            new.display()
        ),
    }
}

/// Rename `old` -> `new` iff `old` exists and `new` does not (never clobber).
/// Returns whether a move happened. Unit-tested with temp dirs.
fn migrate_dir_if_needed(old: &Path, new: &Path) -> std::io::Result<bool> {
    if old.exists() && !new.exists() {
        if let Some(parent) = new.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(old, new)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

impl AppPaths {
    pub fn detect() -> Result<Self, String> {
        let home_dir = dirs::home_dir().ok_or("Could not determine user home directory")?;
        let config_base = dirs::config_dir().unwrap_or_else(|| home_dir.join(".config"));
        let state_base = dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| home_dir.join(".local").join("state"));
        let cache_base = dirs::cache_dir().unwrap_or_else(|| home_dir.join(".cache"));

        // Sprint 16a rebrand: one-time dir migration BEFORE the new dirs are
        // created (ensure_dirs runs after detect), so existing data carries over.
        migrate_legacy_app_dir(&config_base, "config");
        migrate_legacy_app_dir(&state_base, "state");
        migrate_legacy_app_dir(&cache_base, "cache");

        let config_dir = config_base.join(APP_NAME);
        let state_dir = state_base.join(APP_NAME);
        let cache_dir = cache_base.join(APP_NAME);

        Ok(Self {
            projects_file: config_dir.join(PROJECTS_FILE_NAME),
            settings_file: config_dir.join(SETTINGS_FILE_NAME),
            runtime_state_file: state_dir.join(RUNTIME_STATE_FILE_NAME),
            default_data_root: cache_dir.clone(),
            log_dir: state_dir.join("logs"),
            config_dir,
            state_dir,
            cache_dir,
        })
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        for dir in [
            &self.config_dir,
            &self.state_dir,
            &self.cache_dir,
            &self.default_data_root,
            &self.log_dir,
        ] {
            fs::create_dir_all(dir)
                .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
        }

        Ok(())
    }

    pub fn bootstrap_status(&self) -> BootstrapStatus {
        BootstrapStatus {
            config_dir: display_path(&self.config_dir),
            state_dir: display_path(&self.state_dir),
            cache_dir: display_path(&self.cache_dir),
            projects_file: display_path(&self.projects_file),
            settings_file: display_path(&self.settings_file),
            runtime_state_file: display_path(&self.runtime_state_file),
            default_data_root: display_path(&self.default_data_root),
            log_dir: display_path(&self.log_dir),
            transport: "stdio".into(),
            health_strategy: "process-liveness-first".into(),
        }
    }
}

/// Thread-safe storage for application configuration and state.
pub struct ConfigStore {
    paths: AppPaths,
    projects: Mutex<ProjectsFile>,
    settings: Mutex<ManagerSettings>,
}

impl ConfigStore {
    pub fn new() -> Result<Self, String> {
        let paths = AppPaths::detect()?;
        paths.ensure_dirs()?;

        let projects = if paths.projects_file.exists() {
            read_projects(&paths.projects_file)?
        } else {
            let default = ProjectsFile {
                version: 1,
                projects: Vec::new(),
                workspaces: Vec::new(),
            };
            write_json(&paths.projects_file, &default)?;
            default
        };

        let settings = if paths.settings_file.exists() {
            read_settings(&paths.settings_file, &paths)?
        } else {
            let default = ManagerSettings::default_for_paths(&paths);
            write_json(&paths.settings_file, &default)?;
            default
        };

        Ok(Self {
            paths,
            projects: Mutex::new(projects),
            settings: Mutex::new(settings),
        })
    }

    pub fn paths(&self) -> AppPaths {
        self.paths.clone()
    }

    pub fn bootstrap_status(&self) -> BootstrapStatus {
        self.paths.bootstrap_status()
    }

    pub fn list_projects(&self) -> Vec<ProjectRecord> {
        self.projects
            .lock()
            .expect("projects mutex poisoned")
            .projects
            .clone()
    }

    pub fn get_project(&self, project_id: &str) -> Option<ProjectRecord> {
        self.projects
            .lock()
            .expect("projects mutex poisoned")
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
    }

    pub fn add_project(&self, input: AddProjectInput) -> Result<ProjectRecord, String> {
        validate_non_empty("name", &input.name)?;
        validate_non_empty("projectPath", &input.project_path)?;

        // Sprint 10 v0.10.4: workspace_name is the grouping identifier.
        // Empty input falls back to "workspace-default".
        let workspace_name = sanitize_workspace_name(&input.workspace_name);

        let project_slug = slugify(&input.name);
        let project_id = format!("{project_slug}-{}", current_timestamp_millis());

        let project = ProjectRecord {
            id: project_id,
            name: input.name.trim().to_string(),
            project_path: input.project_path.trim().to_string(),
            workspace_name,
            assigned_port: 0,
        };

        let mut projects = self.projects.lock().expect("projects mutex poisoned");

        if projects
            .projects
            .iter()
            .any(|existing| existing.project_path == project.project_path)
        {
            return Err("A project with the same project path is already registered".into());
        }

        projects.projects.push(project.clone());
        write_json(&self.paths.projects_file, &*projects)?;

        Ok(project)
    }

    /// Sprint 10 v0.10.4: move a project to a different workspace.
    /// Replaces the legacy `update_project_port` (port concept removed).
    pub fn set_project_workspace(
        &self,
        project_id: &str,
        workspace_name: String,
    ) -> Result<ProjectRecord, String> {
        let sanitized = sanitize_workspace_name(&workspace_name);
        let mut projects = self.projects.lock().expect("projects mutex poisoned");
        let project = projects
            .projects
            .iter_mut()
            .find(|project| project.id == project_id)
            .ok_or_else(|| format!("Unknown project id: {project_id}"))?;
        project.workspace_name = sanitized;
        let updated = project.clone();
        write_json(&self.paths.projects_file, &*projects)?;
        Ok(updated)
    }

    /// Sprint 10 v0.10.4: rename a project's human-readable name.
    /// Does NOT change `id`, `project_path`, or `workspace_name` —
    /// only the `name` field that the dashboard renders.
    pub fn rename_project(
        &self,
        project_id: &str,
        new_name: String,
    ) -> Result<ProjectRecord, String> {
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err("Project name must not be empty".into());
        }
        let mut projects = self.projects.lock().expect("projects mutex poisoned");
        let project = projects
            .projects
            .iter_mut()
            .find(|p| p.id == project_id)
            .ok_or_else(|| format!("Unknown project id: {project_id}"))?;
        project.name = trimmed.to_string();
        let updated = project.clone();
        write_json(&self.paths.projects_file, &*projects)?;
        Ok(updated)
    }

    /// Sprint 10 v0.10.4: rename a workspace. Updates every ProjectRecord
    /// whose `workspace_name` matches `old_name` to `new_name`. The caller
    /// is responsible for moving the JDT data dir on disk and updating
    /// mcp.json (workspace name appears in the MCP service ID).
    pub fn rename_workspace(
        &self,
        old_name: &str,
        new_name: String,
    ) -> Result<usize, String> {
        let sanitized = sanitize_workspace_name(&new_name);
        if sanitized == old_name {
            return Ok(0);
        }
        let mut projects = self.projects.lock().expect("projects mutex poisoned");
        let mut count = 0usize;
        for project in projects.projects.iter_mut() {
            if project.workspace_name == old_name {
                project.workspace_name = sanitized.clone();
                count += 1;
            }
        }

        // Sprint 16 (bugs.md #11): migrate the resident (port, token) entry
        // in the same transaction so deployed endpoints stay valid across
        // the rename. If the target name already has allocated state, the
        // old name's entry is dropped instead — exactly one entry per
        // workspace, never a duplicate orphan.
        let target_has_state = projects
            .workspaces
            .iter()
            .any(|w| w.workspace_name == sanitized);
        let mut state_changed = false;
        if target_has_state {
            let before = projects.workspaces.len();
            projects.workspaces.retain(|w| w.workspace_name != old_name);
            state_changed = projects.workspaces.len() != before;
        } else if let Some(entry) = projects
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_name == old_name)
        {
            entry.workspace_name = sanitized.clone();
            state_changed = true;
        }

        if count > 0 || state_changed {
            write_json(&self.paths.projects_file, &*projects)?;
        }
        Ok(count)
    }

    pub fn delete_project(&self, project_id: &str) -> Result<ProjectRecord, String> {
        let mut projects = self.projects.lock().expect("projects mutex poisoned");
        let index = projects
            .projects
            .iter()
            .position(|project| project.id == project_id)
            .ok_or_else(|| format!("Unknown project id: {project_id}"))?;
        let removed = projects.projects.remove(index);
        write_json(&self.paths.projects_file, &*projects)?;
        Ok(removed)
    }

    /// Sprint 10 v0.10.4: distinct workspace names currently in use across
    /// all loaded projects, sorted. Replaces the legacy `used_ports`.
    pub fn workspace_names_in_use(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .projects
            .lock()
            .expect("projects mutex poisoned")
            .projects
            .iter()
            .map(|project| project.workspace_name.clone())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    // ===== Sprint 15 Stage 9: per-workspace resident-JVM state =====

    /// Returns the existing `(port, token)` for the workspace if one has
    /// been allocated, else `None`. Stage 10's `ResidentService::start`
    /// uses this to look up an already-assigned pair before deciding to
    /// allocate.
    pub fn get_workspace_state(
        &self,
        workspace_name: &str,
    ) -> Option<crate::resident::WorkspaceState> {
        self.projects
            .lock()
            .expect("projects mutex poisoned")
            .workspaces
            .iter()
            .find(|w| w.workspace_name == workspace_name)
            .cloned()
    }

    /// Returns every persisted workspace `(port, token)` pair. Used by the
    /// Stage 11 deploy writer to emit URL endpoints for ALL deployed
    /// workspaces and by the Stage 10 lifecycle to spawn residents on
    /// manager start.
    pub fn list_workspace_states(&self) -> Vec<crate::resident::WorkspaceState> {
        self.projects
            .lock()
            .expect("projects mutex poisoned")
            .workspaces
            .clone()
    }

    /// Returns the existing state for `workspace_name`, allocating a
    /// fresh `(port, token)` and persisting if none exists yet. The
    /// allocator skips ports already taken by other workspaces AND
    /// probes that the candidate is currently bindable on `127.0.0.1`.
    pub fn get_or_allocate_workspace_state(
        &self,
        workspace_name: &str,
    ) -> Result<crate::resident::WorkspaceState, String> {
        let mut projects = self.projects.lock().expect("projects mutex poisoned");

        if let Some(existing) = projects
            .workspaces
            .iter()
            .find(|w| w.workspace_name == workspace_name)
        {
            return Ok(existing.clone());
        }

        let taken: std::collections::HashSet<u16> = projects
            .workspaces
            .iter()
            .map(|w| w.resident_port)
            .collect();

        let allocator = crate::resident::PortAllocator::new();
        let port = allocator.allocate(&taken)?;
        let token = crate::resident::generate_token();
        let state =
            crate::resident::WorkspaceState::new(workspace_name.to_string(), port, token);

        projects.workspaces.push(state.clone());
        write_json(&self.paths.projects_file, &*projects)?;

        Ok(state)
    }

    /// Releases the persisted state for a workspace (e.g. after the last
    /// project in it is removed and the resident JVM is stopped). Frees
    /// the port for re-allocation. Returns the removed state, if any.
    pub fn release_workspace_state(
        &self,
        workspace_name: &str,
    ) -> Result<Option<crate::resident::WorkspaceState>, String> {
        let mut projects = self.projects.lock().expect("projects mutex poisoned");
        let index = projects
            .workspaces
            .iter()
            .position(|w| w.workspace_name == workspace_name);
        let Some(index) = index else {
            return Ok(None);
        };
        let removed = projects.workspaces.remove(index);
        write_json(&self.paths.projects_file, &*projects)?;
        Ok(Some(removed))
    }

    pub fn get_settings(&self) -> ManagerSettings {
        self.settings
            .lock()
            .expect("settings mutex poisoned")
            .clone()
    }

    pub fn update_settings(&self, input: UpdateSettingsInput) -> Result<ManagerSettings, String> {
        let mut settings = self.settings.lock().expect("settings mutex poisoned");
        settings.update_policy = input.update_policy;
        settings.auto_check_for_updates = input.auto_check_for_updates;

        if input.data_root.trim().is_empty() {
            return Err("dataRoot must not be empty".into());
        }
        settings.data_root = input.data_root.trim().to_string();

        validate_runtime_source(&input.global_runtime_source)?;
        settings.global_runtime_source = input.global_runtime_source;
        settings.use_system_tray = input.use_system_tray;
        settings.autostart_on_boot = input.autostart_on_boot;
        settings.mcp_client_paths = sanitize_mcp_client_paths(input.mcp_client_paths);
        settings.mcp_merge_mode = input.mcp_merge_mode;
        settings.mcp_backup_before_write = input.mcp_backup_before_write;
        settings.deploy_targets = sanitize_deploy_target_flags(input.deploy_targets);
        if let Some(release_repo) = input.release_repo {
            settings.release_repo = sanitize_release_repo(release_repo)?;
        }

        write_json(&self.paths.settings_file, &*settings)?;
        Ok(settings.clone())
    }

    /// Sprint 14 (v0.14.0): minimal setter for `autostart_on_boot`. The
    /// Tauri command also calls into tauri-plugin-autostart to
    /// reconcile OS-level state — this just persists the bool.
    pub fn set_autostart_on_boot(&self, enabled: bool) -> Result<ManagerSettings, String> {
        let mut settings = self.settings.lock().expect("settings mutex poisoned");
        settings.autostart_on_boot = enabled;
        write_json(&self.paths.settings_file, &*settings)?;
        Ok(settings.clone())
    }

    pub fn redetect_mcp_client_paths(&self) -> Result<ManagerSettings, String> {
        let mut settings = self.settings.lock().expect("settings mutex poisoned");
        settings.mcp_client_paths = merge_detected_mcp_paths(settings.mcp_client_paths.clone());
        write_json(&self.paths.settings_file, &*settings)?;
        Ok(settings.clone())
    }

    pub fn write_settings(&self, settings: ManagerSettings) -> Result<ManagerSettings, String> {
        let mut guard = self.settings.lock().expect("settings mutex poisoned");
        *guard = settings.clone();
        write_json(&self.paths.settings_file, &settings)?;
        Ok(settings)
    }
}

fn validate_non_empty(field_name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field_name} must not be empty"));
    }

    Ok(())
}

fn validate_runtime_source(runtime_source: &RuntimeSource) -> Result<(), String> {
    match runtime_source {
        RuntimeSource::Managed => Ok(()),
        RuntimeSource::LocalJar { jar_path } => {
            validate_non_empty("runtimeSource.jarPath", jar_path)
        }
    }
}

/// Sprint 10 v0.10.4: sanitize a workspace name. Empty/whitespace input
/// becomes the special-case `"workspace-default"`; otherwise the input is
/// trimmed (no other transformation — workspace names are user-visible
/// labels and may contain spaces, dashes, etc. — slug-quality enforcement
/// happens at MCP-service-ID derivation time, not here).
pub fn sanitize_workspace_name(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        "workspace-default".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Validate and normalize a "<owner>/<repo>" GitHub release source string.
/// Empty input falls back to the default upstream repo.
fn sanitize_release_repo(input: String) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(default_release_repo());
    }
    if trimmed.matches('/').count() != 1
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
    {
        return Err(format!(
            "releaseRepo must be of the form '<owner>/<repo>'; got '{trimmed}'"
        ));
    }
    Ok(trimmed.to_string())
}

/// Effective GitHub repo for the managed runtime release stream.
/// GOJA_RELEASE_REPO env var wins over the per-user setting.
pub fn effective_release_repo(settings: &ManagerSettings) -> String {
    std::env::var("GOJA_RELEASE_REPO")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| settings.release_repo.clone())
}

fn read_projects(path: &Path) -> Result<ProjectsFile, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;

    if let Ok(mut projects) = serde_json::from_str::<ProjectsFile>(&contents) {
        // Sprint 10 v0.10.4 migration: derive workspace_name from the
        // legacy assigned_port for v0.10.3-format projects.json files.
        // Existing v0.10.4 records keep their workspace_name.
        let mut migrated = false;
        for project in projects.projects.iter_mut() {
            if project.workspace_name.trim().is_empty() {
                project.workspace_name = if project.assigned_port > 0 {
                    format!("workspace-{}", project.assigned_port)
                } else {
                    "workspace-default".to_string()
                };
                migrated = true;
            }
        }
        // Sprint 10 v0.10.4 cleanup: dedupe by project.id. Earlier
        // versions could occasionally end up with two records sharing
        // the same id after manual edits or migration corner cases. The
        // grouped Dashboard view keys its {#each} blocks by id and
        // Svelte fails hard on duplicate keys; deduplicating on read
        // keeps both the UI and the stored file clean. First occurrence
        // of an id wins.
        let dropped = dedupe_projects_by_id(&mut projects.projects);
        if dropped > 0 {
            migrated = true;
        }
        // Sprint 16 (bugs.md #11 migration): prune `workspaces[]` entries
        // that name no current workspace — (port, token) damage left by
        // pre-v0.16.0 rename/delete leaks. The file is backed up before
        // the first pruning writeback; a clean file is left untouched.
        let live_names: std::collections::HashSet<String> = projects
            .projects
            .iter()
            .map(|p| p.workspace_name.clone())
            .collect();
        let orphans: Vec<crate::resident::WorkspaceState> = projects
            .workspaces
            .iter()
            .filter(|w| !live_names.contains(&w.workspace_name))
            .cloned()
            .collect();
        if !orphans.is_empty() {
            let backup = format!(
                "{}.bak-{}",
                path.display(),
                current_timestamp_string()
            );
            match fs::copy(path, &backup) {
                Ok(_) => {
                    for orphan in &orphans {
                        eprintln!(
                            "[goja-studio] pruning orphaned workspace state \
                             '{}' (port {}) — names no current workspace",
                            orphan.workspace_name, orphan.resident_port
                        );
                    }
                    projects
                        .workspaces
                        .retain(|w| live_names.contains(&w.workspace_name));
                    migrated = true;
                }
                Err(error) => {
                    // No backup → no prune. Keep the data; retry next load.
                    eprintln!(
                        "[goja-studio] skipping orphan prune — backup to {backup} \
                         failed: {error}"
                    );
                }
            }
        }
        if migrated {
            // Best-effort writeback so the next read sees clean data.
            let _ = write_json(path, &projects);
        }
        return Ok(projects);
    }

    let legacy = serde_json::from_str::<LegacyProjectsFile>(&contents)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let mut projects = ProjectsFile {
        version: legacy.version.unwrap_or(1),
        projects: legacy
            .projects
            .into_iter()
            .map(|legacy_project| ProjectRecord {
                id: legacy_project.id,
                name: legacy_project.name,
                project_path: legacy_project.project_path,
                workspace_name: "workspace-default".to_string(),
                assigned_port: 0,
            })
            .collect(),
        // Legacy projects.json files predate Stage 9; resident state is
        // allocated lazily the first time a workspace is started.
        workspaces: Vec::new(),
    };
    dedupe_projects_by_id(&mut projects.projects);
    let _ = write_json(path, &projects);
    Ok(projects)
}

/// Sprint 10 v0.10.4: drop ProjectRecord entries whose `id` has already
/// been seen earlier in the slice. First occurrence wins. Returns the
/// number of entries removed.
fn dedupe_projects_by_id(projects: &mut Vec<ProjectRecord>) -> usize {
    let original_len = projects.len();
    let mut seen = std::collections::HashSet::new();
    projects.retain(|p| seen.insert(p.id.clone()));
    original_len - projects.len()
}

fn read_settings(path: &Path, paths: &AppPaths) -> Result<ManagerSettings, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;

    let mut settings: ManagerSettings = serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if settings.data_root.trim().is_empty() {
        settings.data_root = display_path(&paths.default_data_root);
    }
    // One-shot migration: settings.json files written by v0.10.0 carry the
    // legacy upstream repo as the default value. Now that the fork is the
    // shipped default, transparently rewrite that legacy value to the new
    // default so users get our source-resolution fixes without needing to
    // edit settings.json by hand. Explicit user choices (anything other
    // than the legacy default) are preserved.
    if LEGACY_DEFAULT_RELEASE_REPOS
        .iter()
        .any(|legacy| settings.release_repo == *legacy)
    {
        settings.release_repo = default_release_repo();
    }
    settings.mcp_client_paths = merge_detected_mcp_paths(settings.mcp_client_paths);
    Ok(settings)
}

fn detect_default_mcp_client_paths() -> McpClientPaths {
    let home = dirs::home_dir();
    let detect = |candidates: &[PathBuf]| -> Option<String> {
        candidates
            .iter()
            .find(|path| path.exists())
            .map(|path| display_path(path))
            .or_else(|| candidates.first().map(|path| display_path(path)))
    };

    let build = |parts: &[&str]| -> Option<PathBuf> {
        home.as_ref()
            .map(|h| parts.iter().fold(h.clone(), |acc, part| acc.join(part)))
    };

    // Sprint 16.1 (bugs.md #17): Claude Desktop stores its config under the
    // OS config dir — `%APPDATA%` on Windows, `~/Library/Application Support`
    // on macOS, `~/.config` on Linux — so use config_dir, not home.
    let config_dir = dirs::config_dir();
    let build_config = |parts: &[&str]| -> Option<PathBuf> {
        config_dir
            .as_ref()
            .map(|c| parts.iter().fold(c.clone(), |acc, part| acc.join(part)))
    };

    let cursor_candidates: Vec<PathBuf> = [
        [".cursor", "mcp.json"].as_slice(),
        [".config", "Cursor", "mcp.json"].as_slice(),
    ]
    .iter()
    .filter_map(|parts| build(parts))
    .collect();

    // Sprint 16.1 (bugs.md #18): Claude Code reads its global MCP servers
    // from `~/.claude.json` (= `%USERPROFILE%\.claude.json` on Windows).
    // `~/.claude/mcp.json` is not a real Claude Code path — it was the
    // first candidate, so a fresh box where `.claude.json` didn't exist
    // yet fell back to it and the entry was written where Claude never
    // reads. `.claude.json` now leads.
    let claude_candidates: Vec<PathBuf> = [
        [".claude.json"].as_slice(),
        [".claude", "mcp.json"].as_slice(),
    ]
    .iter()
    .filter_map(|parts| build(parts))
    .collect();

    let claude_desktop_candidates: Vec<PathBuf> = [["Claude", "claude_desktop_config.json"]
        .as_slice()]
    .iter()
    .filter_map(|parts| build_config(parts))
    .collect();

    let antigravity_candidates: Vec<PathBuf> = [
        [".gemini", "antigravity", "mcp_config.json"].as_slice(),
        [".config", "Antigravity", "User", "mcp.json"].as_slice(),
        [".antigravity", "mcp.json"].as_slice(),
        [".config", "antigravity", "mcp.json"].as_slice(),
    ]
    .iter()
    .filter_map(|parts| build(parts))
    .collect();

    let intellij_candidates: Vec<PathBuf> = [
        [".config", "JetBrains", "IntelliJIdea", "mcp.json"].as_slice(),
        [".IntelliJIdea", "config", "options", "mcp.json"].as_slice(),
    ]
    .iter()
    .filter_map(|parts| build(parts))
    .collect();

    let make_entry = |candidates: &[PathBuf]| McpClientPathEntry {
        auto_detected_path: detect(candidates),
        manual_override_path: None,
        effective_path: detect(candidates),
    };

    McpClientPaths {
        cursor: make_entry(&cursor_candidates),
        claude: make_entry(&claude_candidates),
        claude_desktop: make_entry(&claude_desktop_candidates),
        antigravity: make_entry(&antigravity_candidates),
        intellij: make_entry(&intellij_candidates),
    }
}

fn merge_detected_mcp_paths(paths: McpClientPaths) -> McpClientPaths {
    let defaults = detect_default_mcp_client_paths();
    McpClientPaths {
        cursor: merge_mcp_path_entry(paths.cursor, defaults.cursor),
        claude: merge_mcp_path_entry(paths.claude, defaults.claude),
        claude_desktop: merge_mcp_path_entry(paths.claude_desktop, defaults.claude_desktop),
        antigravity: merge_mcp_path_entry(paths.antigravity, defaults.antigravity),
        intellij: merge_mcp_path_entry(paths.intellij, defaults.intellij),
    }
}

fn merge_mcp_path_entry(
    mut current: McpClientPathEntry,
    detected: McpClientPathEntry,
) -> McpClientPathEntry {
    current.auto_detected_path = detected.auto_detected_path;
    let manual = current
        .manual_override_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    current.manual_override_path = manual.clone();
    current.effective_path = manual.or_else(|| current.auto_detected_path.clone());
    current
}

fn sanitize_mcp_client_paths(paths: McpClientPaths) -> McpClientPaths {
    merge_detected_mcp_paths(paths)
}

fn sanitize_deploy_target_flags(flags: DeployTargetFlags) -> DeployTargetFlags {
    flags
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if path.exists() {
        let backup_path = path.with_extension(format!("json.bak.{}", current_timestamp_millis()));
        if let Err(error) = fs::copy(path, &backup_path) {
            eprintln!(
                "Warning: failed to create backup of {}: {error}",
                path.display()
            );
        }
    }

    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn current_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis()
}

/// Returns the current UNIX timestamp in milliseconds as a string.
pub fn current_timestamp_string() -> String {
    current_timestamp_millis().to_string()
}

/// Converts a path to a string, using lossy conversion if necessary.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let slug = slug.trim_matches('-');

    if slug.is_empty() {
        "project".into()
    } else {
        slug.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes_display_names() {
        assert_eq!(slugify("Example Service"), "example-service");
        assert_eq!(slugify("Repo::Manager"), "repo-manager");
        assert_eq!(slugify("###"), "project");
    }

    #[test]
    fn bootstrap_status_uses_stdio_transport() {
        let paths = AppPaths {
            config_dir: PathBuf::from("/tmp/config"),
            state_dir: PathBuf::from("/tmp/state"),
            cache_dir: PathBuf::from("/tmp/cache"),
            projects_file: PathBuf::from("/tmp/config/projects.json"),
            settings_file: PathBuf::from("/tmp/config/settings.json"),
            runtime_state_file: PathBuf::from("/tmp/state/runtime-state.json"),
            default_data_root: PathBuf::from("/tmp/cache/goja-studio"),
            log_dir: PathBuf::from("/tmp/state/logs"),
        };

        let bootstrap = paths.bootstrap_status();
        assert_eq!(bootstrap.transport, "stdio");
        assert_eq!(bootstrap.health_strategy, "process-liveness-first");
        assert!(bootstrap.settings_file.ends_with("settings.json"));
        assert!(bootstrap.default_data_root.ends_with("goja-studio"));
    }

    #[test]
    fn legacy_project_shape_is_upgraded_to_local_runtime_source() {
        let legacy = r#"{
          "version": 1,
          "projects": [
            {
              "id": "legacy-1",
              "name": "Legacy",
              "projectPath": "/tmp/project",
              "javalensJarPath": "/tmp/goja.jar",
              "workspaceDir": "/tmp/workspace"
            }
          ]
        }"#;

        let path = PathBuf::from("/tmp/legacy-projects.json");
        fs::write(&path, legacy).expect("failed to write test file");
        let parsed = read_projects(&path).expect("failed to parse legacy projects");
        let _ = fs::remove_file(&path);

        assert_eq!(parsed.projects.len(), 1);
        assert_eq!(parsed.projects[0].id, "legacy-1");
    }

    #[test]
    fn settings_defaults_use_ask_policy_and_auto_checks() {
        let paths = AppPaths {
            config_dir: PathBuf::from("/tmp/config"),
            state_dir: PathBuf::from("/tmp/state"),
            cache_dir: PathBuf::from("/tmp/cache"),
            projects_file: PathBuf::from("/tmp/config/projects.json"),
            settings_file: PathBuf::from("/tmp/config/settings.json"),
            runtime_state_file: PathBuf::from("/tmp/state/runtime-state.json"),
            default_data_root: PathBuf::from("/tmp/cache/goja-studio"),
            log_dir: PathBuf::from("/tmp/state/logs"),
        };

        let settings = ManagerSettings::default_for_paths(&paths);
        assert_eq!(settings.update_policy, UpdatePolicy::Ask);
        assert!(settings.auto_check_for_updates);
        assert_eq!(settings.data_root, "/tmp/cache/goja-studio");
    }

    // ============================================================
    // Sprint 10 v0.10.4: workspace_name + migration tests.
    // ============================================================

    use std::sync::atomic::{AtomicU64, Ordering};

    /// Returns a unique tempdir path per call, so concurrent tests don't
    /// step on each other's projects.json / settings.json files.
    fn unique_tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "goja-studio-test-{label}-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
        fs::create_dir_all(&dir).expect("failed to create test tempdir");
        dir
    }

    fn paths_in(dir: &Path) -> AppPaths {
        let p = AppPaths {
            config_dir: dir.to_path_buf(),
            state_dir: dir.to_path_buf(),
            cache_dir: dir.to_path_buf(),
            projects_file: dir.join("projects.json"),
            settings_file: dir.join("settings.json"),
            runtime_state_file: dir.join("runtime-state.json"),
            default_data_root: dir.to_path_buf(),
            log_dir: dir.join("logs"),
        };
        fs::create_dir_all(&p.log_dir).unwrap();
        p
    }

    #[test]
    fn migrate_dir_if_needed_moves_only_when_target_absent() {
        let root = unique_tempdir("rebrand-migrate");

        // Branch 1: old present, new absent -> move (data carries over).
        let old1 = root.join("c1").join(OLD_APP_NAME);
        let new1 = root.join("c1").join(APP_NAME);
        fs::create_dir_all(old1.join("workspaces")).unwrap();
        fs::write(old1.join("settings.json"), b"{}").unwrap();
        assert!(migrate_dir_if_needed(&old1, &new1).unwrap(), "should move");
        assert!(new1.join("settings.json").exists(), "settings carried over");
        assert!(new1.join("workspaces").is_dir(), "subdirs carried over");
        assert!(!old1.exists(), "old dir moved away");

        // Branch 2: new already present -> no clobber, no move.
        let old2 = root.join("c2").join(OLD_APP_NAME);
        let new2 = root.join("c2").join(APP_NAME);
        fs::create_dir_all(&old2).unwrap();
        fs::create_dir_all(&new2).unwrap();
        fs::write(new2.join("keep.txt"), b"new").unwrap();
        assert!(!migrate_dir_if_needed(&old2, &new2).unwrap(), "must not move");
        assert!(old2.exists(), "old left intact for manual review");
        assert_eq!(fs::read(new2.join("keep.txt")).unwrap(), b"new", "new untouched");

        // Branch 3: both absent -> noop, fresh start.
        let old3 = root.join("c3").join(OLD_APP_NAME);
        let new3 = root.join("c3").join(APP_NAME);
        assert!(!migrate_dir_if_needed(&old3, &new3).unwrap(), "nothing to move");
        assert!(!new3.exists(), "no dir conjured");
    }

    #[test]
    fn sanitize_workspace_name_empty_returns_default() {
        assert_eq!(sanitize_workspace_name(""), "workspace-default");
        assert_eq!(sanitize_workspace_name("   "), "workspace-default");
        assert_eq!(sanitize_workspace_name("\t\n"), "workspace-default");
    }

    #[test]
    fn sanitize_workspace_name_trims_but_preserves_inner_chars() {
        // Workspace names are user-visible labels; we only trim outer
        // whitespace. Spaces, dashes, mixed case all survive — slug
        // hygiene happens at MCP-service-ID derivation time, not here.
        assert_eq!(sanitize_workspace_name("  alpha  "), "alpha");
        assert_eq!(sanitize_workspace_name("My Workspace"), "My Workspace");
        assert_eq!(sanitize_workspace_name("workspace-11100"), "workspace-11100");
    }

    #[test]
    fn read_projects_migrates_assigned_port_to_workspace_name() {
        let dir = unique_tempdir("migrate-port");
        let path = dir.join("projects.json");

        // v0.10.3-shape projects.json: assignedPort set, workspaceName missing.
        let v0_10_3 = r#"{
          "version": 1,
          "projects": [
            {
              "id": "p1",
              "name": "Service A",
              "projectPath": "/projects/a",
              "assignedPort": 11100
            },
            {
              "id": "p2",
              "name": "Service B",
              "projectPath": "/projects/b",
              "assignedPort": 11102
            }
          ]
        }"#;
        fs::write(&path, v0_10_3).unwrap();

        let parsed = read_projects(&path).expect("migration must succeed");
        assert_eq!(parsed.projects.len(), 2);
        assert_eq!(parsed.projects[0].workspace_name, "workspace-11100");
        assert_eq!(parsed.projects[1].workspace_name, "workspace-11102");

        // Migrated data is written back so the next read is clean.
        let reread = read_projects(&path).unwrap();
        assert_eq!(reread.projects[0].workspace_name, "workspace-11100");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_projects_zero_port_falls_back_to_workspace_default() {
        let dir = unique_tempdir("zero-port");
        let path = dir.join("projects.json");

        // No assignedPort, no workspaceName — pre-port-allocation legacy.
        let raw = r#"{
          "version": 1,
          "projects": [
            { "id": "p1", "name": "Foo", "projectPath": "/projects/foo" }
          ]
        }"#;
        fs::write(&path, raw).unwrap();

        let parsed = read_projects(&path).unwrap();
        assert_eq!(parsed.projects[0].workspace_name, "workspace-default");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_projects_dedupes_records_with_duplicate_ids() {
        // Two records with the same id (corruption from manual edits,
        // imports, etc.). read_projects must drop the second occurrence
        // — Svelte's keyed {#each} block would otherwise crash with
        // each_key_duplicate. First occurrence wins.
        let dir = unique_tempdir("dedupe");
        let path = dir.join("projects.json");

        let raw = r#"{
          "version": 1,
          "projects": [
            { "id": "p1", "name": "First",  "projectPath": "/p/a", "workspaceName": "ws" },
            { "id": "p1", "name": "Dup",    "projectPath": "/p/b", "workspaceName": "ws" },
            { "id": "p2", "name": "Second", "projectPath": "/p/c", "workspaceName": "ws" }
          ]
        }"#;
        fs::write(&path, raw).unwrap();

        let parsed = read_projects(&path).unwrap();
        assert_eq!(parsed.projects.len(), 2, "duplicate id must be dropped");
        // First occurrence wins: the "First" record stays, "Dup" is gone.
        assert_eq!(parsed.projects[0].name, "First");
        assert_eq!(parsed.projects[1].name, "Second");

        // Cleaned data is written back so the next read is fully clean.
        let reread = read_projects(&path).unwrap();
        assert_eq!(reread.projects.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_projects_keeps_existing_workspace_name() {
        let dir = unique_tempdir("keep-ws");
        let path = dir.join("projects.json");

        // Already-migrated v0.10.4 record. Migration must not overwrite.
        let raw = r#"{
          "version": 1,
          "projects": [
            {
              "id": "p1",
              "name": "X",
              "projectPath": "/projects/x",
              "workspaceName": "alpha",
              "assignedPort": 11100
            }
          ]
        }"#;
        fs::write(&path, raw).unwrap();

        let parsed = read_projects(&path).unwrap();
        assert_eq!(parsed.projects[0].workspace_name, "alpha");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_store_add_project_uses_workspace_name() {
        let dir = unique_tempdir("add");
        let paths = paths_in(&dir);
        // Write empty projects.json + settings.json so ConfigStore::new path works
        // — but we'll bypass that and build the store directly to avoid the
        // detect()-based AppPaths the public constructor uses.
        let store = ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(ProjectsFile { version: 1, projects: Vec::new(), workspaces: Vec::new() }),
            settings: Mutex::new(ManagerSettings::default_for_paths(&paths)),
        };

        let project = store
            .add_project(AddProjectInput {
                name: "Alpha".into(),
                project_path: "/projects/alpha".into(),
                workspace_name: "alpha".into(),
            })
            .expect("add_project should succeed");

        assert_eq!(project.workspace_name, "alpha");
        assert_eq!(project.assigned_port, 0);

        // Empty workspace_name → "workspace-default".
        let p2 = store
            .add_project(AddProjectInput {
                name: "Beta".into(),
                project_path: "/projects/beta".into(),
                workspace_name: String::new(),
            })
            .unwrap();
        assert_eq!(p2.workspace_name, "workspace-default");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_store_set_project_workspace_moves_record() {
        let dir = unique_tempdir("set-ws");
        let paths = paths_in(&dir);
        let store = ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(ProjectsFile { version: 1, projects: Vec::new(), workspaces: Vec::new() }),
            settings: Mutex::new(ManagerSettings::default_for_paths(&paths)),
        };

        let project = store
            .add_project(AddProjectInput {
                name: "Alpha".into(),
                project_path: "/projects/alpha".into(),
                workspace_name: "alpha".into(),
            })
            .unwrap();

        let updated = store
            .set_project_workspace(&project.id, "beta".into())
            .expect("rename should succeed");

        assert_eq!(updated.workspace_name, "beta");
        // Persisted to disk.
        let reread = read_projects(&paths.projects_file).unwrap();
        assert_eq!(reread.projects[0].workspace_name, "beta");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_store_rename_workspace_bulk_updates_matching_records() {
        let dir = unique_tempdir("rename-ws");
        let paths = paths_in(&dir);
        let store = ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(ProjectsFile { version: 1, projects: Vec::new(), workspaces: Vec::new() }),
            settings: Mutex::new(ManagerSettings::default_for_paths(&paths)),
        };

        store.add_project(AddProjectInput {
            name: "A".into(),
            project_path: "/p/a".into(),
            workspace_name: "alpha".into(),
        }).unwrap();
        store.add_project(AddProjectInput {
            name: "B".into(),
            project_path: "/p/b".into(),
            workspace_name: "alpha".into(),
        }).unwrap();
        store.add_project(AddProjectInput {
            name: "C".into(),
            project_path: "/p/c".into(),
            workspace_name: "beta".into(),
        }).unwrap();

        let count = store
            .rename_workspace("alpha", "alpha-2".into())
            .expect("rename should succeed");
        assert_eq!(count, 2);

        let projects = store.list_projects();
        let by_name: std::collections::HashMap<String, String> = projects
            .iter()
            .map(|p| (p.name.clone(), p.workspace_name.clone()))
            .collect();
        assert_eq!(by_name.get("A").unwrap(), "alpha-2");
        assert_eq!(by_name.get("B").unwrap(), "alpha-2");
        assert_eq!(by_name.get("C").unwrap(), "beta");

        // Renaming to the same name is a no-op (returns 0 changes).
        let count2 = store.rename_workspace("beta", "beta".into()).unwrap();
        assert_eq!(count2, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    /// Sprint 14 (v0.14.0): the new ConfigStore::set_autostart_on_boot
    /// setter persists to disk and round-trips through read_settings.
    #[test]
    fn config_store_set_autostart_on_boot_persists_to_disk() {
        let dir = unique_tempdir("autostart");
        let paths = paths_in(&dir);
        let store = ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(ProjectsFile { version: 1, projects: Vec::new(), workspaces: Vec::new() }),
            settings: Mutex::new(ManagerSettings::default_for_paths(&paths)),
        };

        // Default is opt-in (false).
        assert!(!store.get_settings().autostart_on_boot);

        let updated = store
            .set_autostart_on_boot(true)
            .expect("set must succeed");
        assert!(updated.autostart_on_boot);

        // Reload from disk to confirm persistence survived the round-trip.
        let reread = read_settings(&paths.settings_file, &paths).unwrap();
        assert!(reread.autostart_on_boot);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_store_workspace_names_in_use_returns_distinct_sorted() {
        let dir = unique_tempdir("names-in-use");
        let paths = paths_in(&dir);
        let store = ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(ProjectsFile { version: 1, projects: Vec::new(), workspaces: Vec::new() }),
            settings: Mutex::new(ManagerSettings::default_for_paths(&paths)),
        };

        // Three projects across two workspaces; alphabetical "alpha" before "beta".
        store.add_project(AddProjectInput {
            name: "X".into(),
            project_path: "/p/x".into(),
            workspace_name: "beta".into(),
        }).unwrap();
        store.add_project(AddProjectInput {
            name: "Y".into(),
            project_path: "/p/y".into(),
            workspace_name: "alpha".into(),
        }).unwrap();
        store.add_project(AddProjectInput {
            name: "Z".into(),
            project_path: "/p/z".into(),
            workspace_name: "alpha".into(),
        }).unwrap();

        let names = store.workspace_names_in_use();
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    // ===== Sprint 15 Stage 9: workspace state allocation =====

    fn store_with_empty_state(dir: &Path) -> ConfigStore {
        let paths = paths_in(dir);
        ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(ProjectsFile {
                version: 1,
                projects: Vec::new(),
                workspaces: Vec::new(),
            }),
            settings: Mutex::new(ManagerSettings::default_for_paths(&paths)),
        }
    }

    #[test]
    fn get_or_allocate_assigns_distinct_ports_per_workspace() {
        let dir = unique_tempdir("alloc-distinct");
        let store = store_with_empty_state(&dir);

        let a = store
            .get_or_allocate_workspace_state("alpha")
            .expect("alpha");
        let b = store
            .get_or_allocate_workspace_state("beta")
            .expect("beta");

        assert_ne!(a.resident_port, b.resident_port, "ports must differ");
        assert_ne!(a.resident_token, b.resident_token, "tokens must differ");
        assert_eq!(a.workspace_name, "alpha");
        assert_eq!(b.workspace_name, "beta");
        assert!(
            (crate::resident::DEFAULT_PORT_RANGE_START
                ..=crate::resident::DEFAULT_PORT_RANGE_END)
                .contains(&a.resident_port)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_or_allocate_returns_existing_state_on_repeat_call() {
        let dir = unique_tempdir("alloc-repeat");
        let store = store_with_empty_state(&dir);

        let first = store
            .get_or_allocate_workspace_state("alpha")
            .expect("first");
        let second = store
            .get_or_allocate_workspace_state("alpha")
            .expect("second");

        assert_eq!(first, second, "subsequent call must return the same state");
        assert_eq!(store.list_workspace_states().len(), 1, "no duplicate");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_state_persists_across_restart() {
        // Persistence smoke: allocate via one store, build a fresh store
        // pointing at the same projects.json, and verify the allocated
        // (port, token) survives.
        let dir = unique_tempdir("alloc-persist");
        let paths = paths_in(&dir);
        // Seed projects.json so ConfigStore::new succeeds.
        write_json(
            &paths.projects_file,
            &ProjectsFile { version: 1, projects: Vec::new(), workspaces: Vec::new() },
        )
        .expect("seed projects.json");
        write_json(
            &paths.settings_file,
            &ManagerSettings::default_for_paths(&paths),
        )
        .expect("seed settings.json");

        let allocated = {
            let store = ConfigStore {
                paths: paths.clone(),
                projects: Mutex::new(
                    read_projects(&paths.projects_file).expect("read"),
                ),
                settings: Mutex::new(read_settings(&paths.settings_file, &paths).expect("settings")),
            };
            // Sprint 16: workspace state only persists while a project
            // names the workspace — the orphan prune on load enforces it.
            store
                .add_project(AddProjectInput {
                    name: "Alpha".into(),
                    project_path: "/projects/alpha".into(),
                    workspace_name: "alpha".into(),
                })
                .expect("seed project");
            store
                .get_or_allocate_workspace_state("alpha")
                .expect("alloc")
        };

        // Fresh store reading the same files.
        let restarted = ConfigStore {
            paths: paths.clone(),
            projects: Mutex::new(read_projects(&paths.projects_file).expect("re-read")),
            settings: Mutex::new(
                read_settings(&paths.settings_file, &paths).expect("settings"),
            ),
        };
        let after_restart = restarted
            .get_workspace_state("alpha")
            .expect("state should survive restart");
        assert_eq!(after_restart, allocated);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_frees_port_for_reuse() {
        let dir = unique_tempdir("alloc-release");
        let store = store_with_empty_state(&dir);

        // Pre-fill the range with only 3 ports to force collisions.
        // (We can't easily inject a custom PortAllocator into ConfigStore
        // without further plumbing, so we just check the release semantic.)
        let first = store
            .get_or_allocate_workspace_state("alpha")
            .expect("alpha");
        let released = store
            .release_workspace_state("alpha")
            .expect("release ok");
        assert_eq!(released.as_ref(), Some(&first));

        assert!(
            store.get_workspace_state("alpha").is_none(),
            "alpha must be gone after release"
        );

        // Re-allocating alpha must succeed; the port MAY be the same
        // (lowest-free-port allocator) since the previous one is now free.
        let realloc = store
            .get_or_allocate_workspace_state("alpha")
            .expect("realloc");
        assert_eq!(realloc.workspace_name, "alpha");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_returns_none_for_unknown_workspace() {
        let dir = unique_tempdir("alloc-release-none");
        let store = store_with_empty_state(&dir);
        assert!(store
            .release_workspace_state("nonexistent")
            .expect("release ok")
            .is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    // ===== Sprint 16 (bugs.md #11): rename migrates resident state =====

    #[test]
    fn rename_workspace_migrates_resident_state() {
        let dir = unique_tempdir("rename-migrate");
        let store = store_with_empty_state(&dir);
        store
            .add_project(AddProjectInput {
                name: "Alpha".into(),
                project_path: "/projects/alpha".into(),
                workspace_name: "old-ws".into(),
            })
            .unwrap();
        let allocated = store.get_or_allocate_workspace_state("old-ws").unwrap();

        store.rename_workspace("old-ws", "new-ws".into()).unwrap();

        let migrated = store
            .get_workspace_state("new-ws")
            .expect("state must follow the rename");
        assert_eq!(migrated.resident_port, allocated.resident_port, "same port");
        assert_eq!(migrated.resident_token, allocated.resident_token, "same token");
        assert!(
            store.get_workspace_state("old-ws").is_none(),
            "no orphan under the old name"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_workspace_into_existing_state_drops_stale_entry() {
        // If the target name somehow already has allocated state, the old
        // name's entry is released rather than left as a duplicate orphan.
        let dir = unique_tempdir("rename-collide");
        let store = store_with_empty_state(&dir);
        store
            .add_project(AddProjectInput {
                name: "Alpha".into(),
                project_path: "/projects/alpha".into(),
                workspace_name: "old-ws".into(),
            })
            .unwrap();
        let _old = store.get_or_allocate_workspace_state("old-ws").unwrap();
        let existing_new = store.get_or_allocate_workspace_state("new-ws").unwrap();

        store.rename_workspace("old-ws", "new-ws".into()).unwrap();

        let states = store.list_workspace_states();
        assert_eq!(states.len(), 1, "exactly one entry survives: {states:?}");
        assert_eq!(states[0].workspace_name, "new-ws");
        assert_eq!(states[0].resident_port, existing_new.resident_port,
            "the pre-existing target allocation wins");
        let _ = fs::remove_dir_all(&dir);
    }

    // ===== Sprint 16 (bugs.md #11 migration): orphan prune on load =====

    fn projects_json_with_orphans(dir: &Path) -> PathBuf {
        let path = dir.join("projects.json");
        let raw = r#"{
          "version": 1,
          "projects": [
            {
              "id": "p1",
              "name": "Alpha",
              "projectPath": "/projects/alpha",
              "workspaceName": "live-ws",
              "assignedPort": 0
            }
          ],
          "workspaces": [
            { "workspaceName": "live-ws", "residentPort": 8805, "residentToken": "tok-live" },
            { "workspaceName": "ghost-a", "residentPort": 8800, "residentToken": "tok-a" },
            { "workspaceName": "ghost-b", "residentPort": 8801, "residentToken": "tok-b" }
          ]
        }"#;
        fs::write(&path, raw).unwrap();
        path
    }

    fn backup_count(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("projects.json.bak-")
            })
            .count()
    }

    #[test]
    fn read_projects_prunes_orphaned_workspace_states() {
        let dir = unique_tempdir("prune-orphans");
        let path = projects_json_with_orphans(&dir);

        let parsed = read_projects(&path).unwrap();

        assert_eq!(parsed.workspaces.len(), 1, "only the live entry survives");
        assert_eq!(parsed.workspaces[0].workspace_name, "live-ws");
        assert_eq!(parsed.workspaces[0].resident_port, 8805);
        assert_eq!(backup_count(&dir), 1, "pruning write must back up first");

        // The pruned file persists — a second read finds nothing to prune
        // and creates no second backup (idempotent).
        let reread = read_projects(&path).unwrap();
        assert_eq!(reread.workspaces.len(), 1);
        assert_eq!(backup_count(&dir), 1, "no second backup on clean re-read");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_projects_prune_keeps_all_when_no_orphans() {
        let dir = unique_tempdir("prune-noop");
        let path = dir.join("projects.json");
        let raw = r#"{
          "version": 1,
          "projects": [
            {
              "id": "p1",
              "name": "Alpha",
              "projectPath": "/projects/alpha",
              "workspaceName": "live-ws",
              "assignedPort": 0
            }
          ],
          "workspaces": [
            { "workspaceName": "live-ws", "residentPort": 8805, "residentToken": "tok-live" }
          ]
        }"#;
        fs::write(&path, raw).unwrap();

        let parsed = read_projects(&path).unwrap();
        assert_eq!(parsed.workspaces.len(), 1);
        assert_eq!(backup_count(&dir), 0, "no orphans → no backup, no rewrite");
        let _ = fs::remove_dir_all(&dir);
    }
}
