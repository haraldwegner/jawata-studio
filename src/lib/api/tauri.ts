import { invoke } from "@tauri-apps/api/core";

/** Represents the current phase of a runtime instance. */
export type RuntimePhase = "stopped" | "starting" | "running" | "failed";
/** Policy for handling application updates. */
export type UpdatePolicy = "always" | "ask";
/** Mode for merging MCP settings into client configurations. */
export type McpMergeMode = "safeMerge" | "replaceManagedSection";
/** Mode for deploying MCP configuration to clients. */
export type DeployMode = "deploy" | "dryRun" | "preview" | "regenerate" | "delete";
/** Status kind for application release checks. */
export type ReleaseStatusKind =
  | "ready"
  | "missing"
  | "updateAvailable"
  | "checkFailed"
  | "checkingDisabled";

/** Paths and configuration used during application bootstrap. */
export interface BootstrapStatus {
  configDir: string;
  stateDir: string;
  cacheDir: string;
  projectsFile: string;
  settingsFile: string;
  runtimeStateFile: string;
  defaultDataRoot: string;
  logDir: string;
  transport: string;
  healthStrategy: string;
}

/** Global settings for the manager application. */
export interface ManagerSettings {
  version: number;
  updatePolicy: UpdatePolicy;
  autoCheckForUpdates: boolean;
  manualFallbackJarPath?: string | null;
  dataRoot: string;
  globalRuntimeSource: RuntimeSource;
  useSystemTray: boolean;
  /** Sprint 14 (v0.14.0): manager auto-launches at session login when true.
   * v0.14.1 (bugs.md #7) extends the semantic: also restores the workspaces
   * that were running at last shutdown. */
  autostartOnBoot: boolean;
  mcpClientPaths: McpClientPaths;
  mcpMergeMode: McpMergeMode;
  mcpBackupBeforeWrite: boolean;
  deployTargets: DeployTargetFlags;
  /** GitHub repo (owner/repo) for the managed GOJA runtime release stream. */
  releaseRepo: string;
  lastReleaseCheck?: string | null;
  lastSeenLatestVersion?: string | null;
}

/** Represents path configuration for a specific MCP client. */
export interface McpClientPathEntry {
  autoDetectedPath?: string | null;
  manualOverridePath?: string | null;
  effectivePath?: string | null;
}

/** Collection of paths for all supported MCP clients. */
export interface McpClientPaths {
  cursor: McpClientPathEntry;
  claude: McpClientPathEntry;
  claudeDesktop: McpClientPathEntry;
  antigravity: McpClientPathEntry;
  intellij: McpClientPathEntry;
}

/** Flags indicating which MCP clients to deploy to. */
export interface DeployTargetFlags {
  cursor: boolean;
  claude: boolean;
  claudeDesktop: boolean;
  antigravity: boolean;
  intellij: boolean;
}

/** Source configuration for the GOJA runtime. */
export type RuntimeSource =
  | {
      kind: "managed";
    }
  | {
      kind: "localJar";
      jarPath: string;
    };

/** Record of a registered project. */
export interface ProjectRecord {
  id: string;
  name: string;
  projectPath: string;
  /** Sprint 10 v0.10.4: logical workspace identifier. Multiple projects
   * sharing this name run as one MCP service. */
  workspaceName: string;
  /** Legacy v0.10.3 field. Kept on disk for one release cycle for
   * migration purposes; ignored at runtime. */
  assignedPort?: number;
}

/** Input for adding a new project. */
export interface AddProjectInput {
  name: string;
  projectPath: string;
  /** Sprint 10 v0.10.4: target workspace. Empty/missing → "workspace-default". */
  workspaceName: string;
}

/** Input for updating manager settings. */
export interface UpdateSettingsInput {
  updatePolicy: UpdatePolicy;
  autoCheckForUpdates: boolean;
  dataRoot: string;
  globalRuntimeSource: RuntimeSource;
  useSystemTray: boolean;
  /** Sprint 14 (v0.14.0): manager auto-launches at session login when true.
   * v0.14.1 (bugs.md #7) extends the semantic: also restores the workspaces
   * that were running at last shutdown. */
  autostartOnBoot: boolean;
  mcpClientPaths: McpClientPaths;
  mcpMergeMode: McpMergeMode;
  mcpBackupBeforeWrite: boolean;
  deployTargets: DeployTargetFlags;
  /** Optional override of the GitHub repo (owner/repo) for the runtime release stream. */
  releaseRepo?: string | null;
}

/** Record of an installed managed runtime. */
export interface ManagedRuntimeRecord {
  version: string;
  installDir: string;
  jarPath: string;
  assetName: string;
  installedAt: string;
}

/** Status of the current release and available updates. */
export interface ReleaseStatus {
  kind: ReleaseStatusKind;
  latestVersion?: string | null;
  defaultVersion?: string | null;
  checkedAt?: string | null;
  updateAvailable: boolean;
  detail: string;
}

/** Status of a specific project's runtime. Sprint 10 v0.10.4: multiple
 * projects sharing a `workspaceName` reflect the same underlying goja
 * process — same PID, same workspace dir. */
export interface RuntimeStatusRecord {
  projectId: string;
  phase: RuntimePhase;
  /** Sprint 10 v0.10.4: workspace this project belongs to. */
  workspaceName: string;
  transport: string;
  pid?: number | null;
  workspaceDir: string;
  logPath: string;
  runtimeLabel: string;
  resolvedJarPath: string;
  serviceMode: string;
  detail: string;
}

/** Comprehensive dashboard state for the manager application. */
export interface ManagerDashboard {
  bootstrap: BootstrapStatus;
  settings: ManagerSettings;
  releaseStatus: ReleaseStatus;
  installedRuntime?: ManagedRuntimeRecord | null;
  projects: ProjectRecord[];
  runtimeStatuses: Record<string, RuntimeStatusRecord>;
  /** Sprint 10 v0.10.4: a workspace name to pre-fill in the "Add project"
   * form. Surfaces an existing workspace if one exists; null/undefined
   * means the UI falls back to a fresh name. */
  suggestedWorkspaceName?: string | null;
  servicesInventory: ServicesInventory;
}

/** Inventory of available runtime services. */
export interface ServicesInventory {
  available: boolean;
  services: string[];
  detail: string;
}

/** Summary of a cleanup operation. */
export interface CleanupSummary {
  target: string;
  deletedFiles: number;
  deletedDirs: number;
  failedPaths: string[];
  detail: string;
}

/** Result of probing available services. */
export interface ServiceProbeResult {
  ok: boolean;
  services: ProbeServiceEntry[];
  detail: string;
  durationMs: number;
  rawProtocolError?: string | null;
}

/** Entry for a probed service. */
export interface ProbeServiceEntry {
  name: string;
  description?: string | null;
}

/** Status of a deployment to a specific client. */
export type DeployClientStatus = "success" | "skipped" | "failed";

/** Result of a deployment to a specific client. */
export interface DeployClientResult {
  client: string;
  targetPath: string;
  status: DeployClientStatus;
  message: string;
  backupPath?: string | null;
  changedSections: string[];
  validationErrors: string[];
  previewContent?: string | null;
}

/** Input for deploying MCP configuration to agents. */
export interface DeployToAgentsInput {
  mode: DeployMode;
  targetClients?: string[] | null;
}

/** Result of deploying MCP configuration to agents. */
export interface DeployToAgentsResult {
  mode: DeployMode;
  ok: boolean;
  detail: string;
  durationMs: number;
  clients: DeployClientResult[];
}

/** Context for the quit prompt dialog. */
export interface QuitPromptContext {
  runningServices: number;
  trayEnabled: boolean;
}

/** Action to take when quitting the application. */
export type QuitAction = "cancel" | "hideToTray" | "stopAndQuit" | "quit";

/** Sprint 10 v0.10.4: input for moving a project to a different workspace. */
export interface SetProjectWorkspaceInput {
  projectId: string;
  workspaceName: string;
}

/** Sprint 10 v0.10.4: input for renaming a workspace. */
export interface RenameWorkspaceInput {
  oldName: string;
  newName: string;
}

/** Sprint 10 v0.10.4: input for renaming a project's display name. */
export interface RenameProjectInput {
  projectId: string;
  name: string;
}

/** Candidate project found during workspace discovery. */
export interface WorkspaceProjectCandidate {
  name: string;
  projectPath: string;
  kind: string;
}

/** Input for importing projects from a workspace. */
export interface WorkspaceImportInput {
  /** `.code-workspace` source. Ignored when scanFolder is set. */
  workspaceFile: string;
  /** Sprint 16: autoscan source — the backend re-scans this folder and
   * imports the selected candidates from it. Takes precedence. */
  scanFolder?: string;
  selectedPaths: string[];
  /** Sprint 10 v0.10.4: target workspace for the imported projects.
   * Empty/missing → "workspace-default". */
  workspaceName: string;
}

/** Result of importing projects from a workspace. */
export interface WorkspaceImportResult {
  added: ProjectRecord[];
  skipped: string[];
}

/** Retrieves the current dashboard state. */
export function getDashboard(): Promise<ManagerDashboard> {
  return invoke("get_dashboard");
}

/** Adds a new project. */
export function addProject(input: AddProjectInput): Promise<ProjectRecord> {
  return invoke("add_project", { input });
}

/** Sprint 10 v0.10.4: move a project to a different workspace. Replaces
 * the legacy `updateProjectPort`. */
export function setProjectWorkspace(input: SetProjectWorkspaceInput): Promise<ManagerDashboard> {
  return invoke("set_project_workspace", { input });
}

/** Sprint 10 v0.10.4: rename a workspace. Updates every member project
 * record + workspace.json. */
export function renameWorkspace(input: RenameWorkspaceInput): Promise<ManagerDashboard> {
  return invoke("rename_workspace", { input });
}

/** Sprint 10 v0.10.4: delete a workspace entirely. Stops the workspace
 * process, deletes every member project record, and removes the JDT
 * data dir on disk. */
export function deleteWorkspace(workspaceName: string): Promise<ManagerDashboard> {
  return invoke("delete_workspace", { workspaceName });
}

/** Sprint 10 v0.10.4: rename a project's human-readable name. */
export function renameProject(input: RenameProjectInput): Promise<ManagerDashboard> {
  return invoke("rename_project", { input });
}

/** Deletes a project by its ID. */
export function deleteProject(projectId: string): Promise<ManagerDashboard> {
  return invoke("delete_project", { projectId });
}

/** Starts runtimes for all projects. */
export function startAllRuntimes(): Promise<ManagerDashboard> {
  return invoke("start_all_runtimes");
}

/** Stops runtimes for all projects. */
export function stopAllRuntimes(): Promise<ManagerDashboard> {
  return invoke("stop_all_runtimes");
}

/** Sprint 14 (v0.14.0): stops every workspace, waits for each to reach
 * Stopped/Failed (30 s deadline), then restarts them. */
export function reloadAllRuntimes(): Promise<ManagerDashboard> {
  return invoke("reload_all_runtimes");
}

/** Deletes all projects. */
export function deleteAllProjects(): Promise<ManagerDashboard> {
  return invoke("delete_all_projects");
}

/** Discovers project candidates within a workspace file. */
export function discoverWorkspaceProjects(workspaceFile: string): Promise<WorkspaceProjectCandidate[]> {
  return invoke("discover_workspace_projects", { workspaceFile });
}

/** Sprint 16: autoscan — recursively scan a folder (depth ≤ 6) for Java
 * projects, no `.code-workspace` seed needed. */
export function scanFolderForProjects(folder: string): Promise<WorkspaceProjectCandidate[]> {
  return invoke("scan_folder_for_projects", { folder });
}

/** Imports selected projects from a workspace. */
export function importWorkspaceProjects(input: WorkspaceImportInput): Promise<WorkspaceImportResult> {
  return invoke("import_workspace_projects", { input });
}

/** Updates the manager settings. */
export function updateSettings(input: UpdateSettingsInput): Promise<ManagerDashboard> {
  return invoke("update_settings", { input });
}

/** Sprint 14 (v0.14.0): toggle OS-level autostart-on-boot in one
 * round-trip — persists the setting AND reconciles the OS plugin's
 * autostart entry. */
export function setAutostartOnBoot(enabled: boolean): Promise<ManagerDashboard> {
  return invoke("set_autostart_on_boot", { enabled });
}

/** Redetects paths for MCP clients. */
export function redetectMcpClientPaths(): Promise<ManagerDashboard> {
  return invoke("redetect_mcp_client_paths");
}

/** Downloads or updates the GOJA runtime. */
export function downloadOrUpdateGoja(): Promise<ManagerDashboard> {
  return invoke("download_or_update_goja");
}

/** Starts the runtime for a specific project. */
export function startRuntime(projectId: string): Promise<RuntimeStatusRecord> {
  return invoke("start_runtime", { projectId });
}

/** Stops the runtime for a specific project. */
export function stopRuntime(projectId: string): Promise<RuntimeStatusRecord> {
  return invoke("stop_runtime", { projectId });
}

/** Retrieves the runtime status for a specific project. */
export function getRuntimeStatus(projectId: string): Promise<RuntimeStatusRecord> {
  return invoke("get_runtime_status", { projectId });
}

/** Retrieves the inventory of available services. */
export function getServicesInventory(): Promise<ServicesInventory> {
  return invoke("get_services_inventory");
}

/** Cleans up log files. */
export function cleanLogs(): Promise<CleanupSummary> {
  return invoke("clean_logs");
}

/** Cleans up workspace data. */
export function cleanWorkspaces(): Promise<CleanupSummary> {
  return invoke("clean_workspaces");
}

/** Cleans up generated data. */
export function cleanGeneratedData(): Promise<CleanupSummary> {
  return invoke("clean_generated_data");
}

/** Probes available services to check their status. */
export function probeServices(): Promise<ServiceProbeResult> {
  return invoke("probe_services");
}

/** Deploys MCP configuration to target agents. */
export function deployToAgents(input: DeployToAgentsInput): Promise<DeployToAgentsResult> {
  return invoke("deploy_to_agents", { input });
}

/** Retrieves context for the quit prompt. */
export function getQuitPromptContext(): Promise<QuitPromptContext> {
  return invoke("get_quit_prompt_context");
}

/** Performs the specified quit action. */
export function performQuitAction(action: QuitAction): Promise<void> {
  return invoke("perform_quit_action", { action });
}
