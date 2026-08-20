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
  /** GitHub repo (owner/repo) for the managed JAWATA runtime release stream. */
  releaseRepo: string;
  lastReleaseCheck?: string | null;
  lastSeenLatestVersion?: string | null;
  /** Sprint 21a: memory-store preferences (Memory view). Sprint 21b: recursive +
   * crawl caps removed — the crawl finds everything (resident-side backstops). */
  autoSeedOnDeploy: boolean;
  /** `shared` (user-level store, default) | `workspace` | `memory` | explicit dir. */
  experienceStoreMode: string;
  memoryRoots: string[];
  /** Backup plumbing — config key only, no UI. */
  backupRetention: number;
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
  /** Sprint 28a (D1). */
  codex: McpClientPathEntry;
  copilotCli: McpClientPathEntry;
  vscode: McpClientPathEntry;
  grok: McpClientPathEntry;
}

/** Flags indicating which MCP clients to deploy to. */
export interface DeployTargetFlags {
  cursor: boolean;
  claude: boolean;
  /** Sprint 28a (D1). */
  codex: boolean;
  copilotCli: boolean;
  vscode: boolean;
  grok: boolean;
}

/** Source configuration for the JAWATA runtime. */
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
  /** Sprint 21a: Memory-view settings — optional so older saves preserve them. */
  autoSeedOnDeploy?: boolean | null;
  experienceStoreMode?: string | null;
  memoryRoots?: string[] | null;
  backupRetention?: number | null;
}

// ===== Sprint 21a (item F): Knowledge view =====

/** Per-workspace knowledge-store overview (resident stats or unreachable+error). */
export interface KnowledgeWorkspaceStatus {
  workspace: string;
  url: string;
  reachable: boolean;
  stats?: {
    total?: number;
    by_status?: Record<string, number>;
    by_language?: Record<string, number>;
    store?: { file?: string; bytes?: number };
  } | null;
  error?: string | null;
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
 * projects sharing a `workspaceName` reflect the same underlying jawata
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

// ===== Sprint 21a (item F): Knowledge view =====

/** Per-workspace knowledge-store overview (resident experience(kind=stats)). */
export function knowledgeStatus(): Promise<KnowledgeWorkspaceStatus[]> {
  return invoke("knowledge_status");
}

/** Run one experience(kind=…) verb on a workspace's resident. The UI action names ARE
 * the prompt vocabulary — load/reseed/wipe/refresh/list/promote/export/import/prune/
 * dedup/compact/stats. Returns the decoded ToolResponse ({success, data, ...}). */
export function experienceVerb(
  workspace: string,
  kind: string,
  args: Record<string, unknown> = {}
): Promise<{ success: boolean; data?: unknown; error?: unknown }> {
  return invoke("experience_verb", { workspace, kind, args });
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

/** Downloads or updates the JAWATA runtime. */
export function downloadOrUpdateJawata(): Promise<ManagerDashboard> {
  return invoke("download_or_update_jawata");
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

// ===== Sprint 28b (D2 / D6 / D10): the field view, the seat lane, the canary =====

/** One recurring failure shape: `<tool>/<kind>/<code>`. Shapes only — the pile
 * carries no paths, symbol names or message text, so neither does this. */
export interface FieldErrorShape {
  shape: string;
  tool: string;
  kind: string;
  code: string;
  count: number;
  /** Already filed. The nudge stops and the badge stops counting it. */
  posted: boolean;
  clients: string[];
  versions: string[];
  worstLatencyBucket: number;
}

/** What one workspace's `pile.jsonl` says. */
export interface FieldPileFold {
  /** False when nothing has been recorded here yet — NOT the same as zero failures. */
  present: boolean;
  contract?: number | null;
  totalEvents: number;
  failures: number;
  successes: number;
  /** Ranked by recurrence, highest first. */
  shapes: FieldErrorShape[];
  /** Unposted shapes that recurred at least three times. */
  badge: number;
  unreadableLines: number;
}

/** The `/report` tile's state: the two switches, the reminder's REASON, the history. */
export interface FieldSeatLaneState {
  seat: string;
  /** The in-session pointer switch. Distinct from `silenced`. */
  nudges: boolean;
  /** The periodic reminder's go-silent checkbox. */
  silenced: boolean;
  /** "off by your choice" | "on" — never inferred by the view. */
  reminderReason: string;
  strikes: number;
  remindersShown: number;
  lastRemindedAtMillis: number;
  nudgedShapes: string[];
  postedShapes: string[];
  /** False when neither switch has ever been touched — both are defaults, not choices. */
  stateFilePresent: boolean;
}

/** One hook channel's reach: what fired, what came out, and why it did not. */
export interface FieldChannelReach {
  role: string;
  fired: number;
  emitted: number;
  suppressed: Record<string, number>;
  /** The store answered and nothing came out. */
  dead: boolean;
  /** Nothing came out and every suppression was a legitimate absence. */
  legitimatelyQuiet: boolean;
}

/** Where the agent used JAWATA and where it reached for the shell instead. */
export interface FieldUtilization {
  jawataCalls: number;
  shellFallbacks: number;
  slips: number;
  ungroundedReads: number;
  /** null when nothing has been observed — an empty denominator is not 100 %. */
  percent?: number | null;
  /** R1: the denominator is hook-scoped. Always rendered WITH the number. */
  caveat: string;
  observerPresent: boolean;
}

/** One resident's canary reading: a real recall and a real compiler question. */
export interface FieldCanaryResult {
  workspace: string;
  url: string;
  recallOk: boolean;
  recallDetail: string;
  compilerOk: boolean;
  compilerDetail: string;
  green: boolean;
  /** The resident answered CORRECTLY, with PROJECT_LOADING — still importing. */
  loading: boolean;
  /** When the unbroken loading run began; null whenever it is not loading. */
  loadingSinceMillis: number | null;
  checkedAtMillis: number;
  /** How long the store's own answer took. Measured, not asserted. */
  recallMillis: number;
}

/**
 * "unknown" is never rendered as healthy — it means nothing has been probed yet.
 * "loading" is a cold start still importing: not green, and not an alarm (#16).
 */
export type FieldCanaryHealth = "unknown" | "green" | "loading" | "degraded";

/**
 * What the recall gate saw this install. `coverage` rides WITH the numbers for
 * the same reason `FieldUtilization.caveat` does: a zero here means NOT
 * OBSERVED (Cursor fires neither event; Windows cannot read the payload), and
 * a bare row of zeros would read as "the agent never ignored anything".
 */
export interface FieldRecallSignals {
  present: boolean;
  applied: number;
  rejected: number;
  /** OBSERVE mode: what the gate WOULD have held. Promotion is decided on it. */
  wouldBlock: number;
  blocked: number;
  skipped: number;
  unavailable: number;
  coverage: string;
}

/**
 * Stage 5a. `recallOk` is binary and the failure was not — 3459 seconds of a
 * read parked in a socket poll, with the port answering throughout.
 */
export type FieldStoreHealth = "unknown" | "healthy" | "slow" | "unavailable";

export interface FieldStoreHealthReport {
  health: FieldStoreHealth;
  /** The dashboard word for the variant — a separate binding from the variant. */
  word: string;
  worstWorkspace: string;
  slowestMillis: number;
  /** Derived from the hook's own budget, not chosen. */
  slowAboveMillis: number;
  /** Why the threshold is there. Rendered WITH the verdict. */
  why: string;
}

export interface FieldWorkspaceStatus {
  workspace: string;
  fieldDir: string;
  pile: FieldPileFold;
  lane: FieldSeatLaneState;
}

export interface FieldStatus {
  utilization: FieldUtilization;
  channels: FieldChannelReach[];
  deadChannels: string[];
  legitimatelyQuietChannels: string[];
  /** The silence logs that actually existed. Empty = no hook has ever run here. */
  silenceLogsRead: string[];
  workspaces: FieldWorkspaceStatus[];
  /** Machine-wide: unposted recurring shapes across every workspace. */
  badge: number;
  canary: FieldCanaryResult[];
  canaryHealth: FieldCanaryHealth;
  store: FieldStoreHealthReport;
  recall: FieldRecallSignals;
}


/**
 * Stage 9 (G6b): what an import could not resolve, per project.
 *
 * The studio used to DISCARD the whole `health_check` body — `residentAnswers`
 * was a bare boolean — so the engine's classpath honesty had no consumer at
 * all. `projects` lists only the NON-ZERO rows; empty means every project
 * resolved everything, which is a different fact from `reachable: false`.
 */
export interface ProjectResolution {
  projectKey: string;
  /** The project's path on disk, as the resident reports it — the exact join
   *  key for a consumer that knows projects by path rather than by key. */
  projectPath: string;
  unresolved: number;
  /** The refactoring guard, as the resident computes it — NOT derived here. */
  healthy: boolean;
  /** WHY it cannot be read, in the resident's own words. Null when healthy. */
  problem?: string | null;
  /** What the reader can DO about it, in the resident's own words. */
  remedy?: string | null;
}

export interface ResolutionStatus {
  workspace: string;
  url: string;
  reachable: boolean;
  projects: ProjectResolution[];
  projectCount: number;
  /** FALSE means at least one project cannot be READ — worse than unresolved
   *  dependencies, because every whole-workspace answer is then incomplete. */
  healthy: boolean;
  /** The resident's warning text; it names the consequence a colour cannot. */
  warning?: string | null;
  error?: string | null;
}

/** Whether a workspace can be READ, as its resident last reported it.
 *
 *  Separate from the service state on purpose: "the process is running" and
 *  "the workspace can be read" are two different facts, and the engine keeps
 *  them apart. */
export interface WorkspaceReadability {
  workspace: string;
  readable: boolean;
}

/** Per-workspace readability from the canary readings ALREADY TAKEN — a cached
 *  read, no probe. Safe to poll on the dashboard's own cadence. */
export function workspaceReadability(): Promise<WorkspaceReadability[]> {
  return invoke<WorkspaceReadability[]>("workspace_readability");
}

/** Per-workspace dependency-resolution report. One health_check per resident. */
export function resolutionStatus(): Promise<ResolutionStatus[]> {
  return invoke<ResolutionStatus[]>("resolution_status");
}

/** Read the field recording. File reads only — cheap enough to poll. */
export function fieldStatus(): Promise<FieldStatus> {
  return invoke("field_status");
}

/** Set one or both field switches for a workspace, atomically.
 *
 * THEY ARE TWO SWITCHES. `silenced` is the go-silent checkbox: it stops the
 * periodic reminder the agent speaks. `nudges` is the separate no-nudges
 * switch: it stops the one-line pointer at `/report` inside a running session.
 * Pass `null` for either to leave it exactly as it was — the state file has
 * three writers and setting one switch must never move the other. */
export function fieldSetSilence(
  workspace: string,
  nudges: boolean | null,
  silenced: boolean | null
): Promise<FieldStatus> {
  return invoke("field_set_silence", { workspace, nudges, silenced });
}
