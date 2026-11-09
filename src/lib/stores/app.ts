import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import {
  cleanGeneratedData,
  cleanLogs,
  cleanWorkspaces,
  deployToAgents as deployToAgentsApi,
  deploySeats as deploySeatsApi,
  deleteAllProjects,
  deleteProject,
  eclipseWorkspaces as eclipseWorkspacesApi,
  forgetEclipseWorkspace as forgetEclipseWorkspaceApi,
  downloadOrUpdateJawata,
  getDashboard,
  getRuntimeStatus,
  probeServices as probeServicesApi,
  redetectMcpClientPaths as redetectMcpClientPathsApi,
  reloadAllRuntimes,
  startAllRuntimes,
  startRuntime,
  stopAllRuntimes,
  stopRuntime,
  setProjectWorkspace as setProjectWorkspaceApi,
  renameWorkspace as renameWorkspaceApi,
  deleteWorkspace as deleteWorkspaceApi,
  setWorkspaceMaxHeap as setWorkspaceMaxHeapApi,
  setWorkspaceDebuggable as setWorkspaceDebuggableApi,
  renameProject as renameProjectApi,
  updateSettings,
  type CleanupSummary,
  type DeployMode,
  type DeployToAgentsResult,
  type ManagerDashboard,
  type ServiceProbeResult,
  type RuntimeStatusRecord,
  type UpdateSettingsInput,
  type ProjectResolution,
  projectPathKey,
  resolutionStatus,
  workspaceReadability} from "../api/tauri";
import { BUDGET_MS, runBusy } from "../busy";

interface AppState extends Partial<ManagerDashboard> {
  selectedProjectId?: string;
  isBusy: boolean;
  error?: string;
  settingsSaveStatus?: "idle" | "saving" | "success" | "error";
  settingsSaveMessage?: string;
  projectErrors?: Record<string, string>;
  /** jawata-studio#24: per-workspace readability from the resident's own
   *  verdict. Absent means not yet observed, which is NOT the same as
   *  unreadable — a row only says so when the resident said so. */
  workspaceReadable?: Record<string, boolean>;
  /** jawata-studio#37: the resident's PER-PROJECT verdict, keyed by projectPath, holding
   *  only the projects it says it cannot read — so a row can say which one failed instead
   *  of leaving the reader with one workspace-level marker and five green rows. Absent
   *  means not yet observed, which is NOT the same as healthy. */
  projectUnhealthy?: Record<string, ProjectResolution>;
  lastCleanupSummary?: CleanupSummary;
  serviceProbeBusy?: boolean;
  serviceProbeError?: string;
  lastServiceProbe?: ServiceProbeResult;
  deployBusy?: boolean;
  deployError?: string;
  lastDeployResult?: DeployToAgentsResult;
}

const initialState: AppState = {
  projects: [],
  runtimeStatuses: {},
  workspaceReadable: {},
  projectUnhealthy: {},
  projectErrors: {},
  isBusy: false,
  settingsSaveStatus: "idle"
};

function normalizeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
}

/** Creates and returns the application state store. */
export function createAppStore() {
  const { subscribe, update } = writable<AppState>(initialState);
  const STATUS_POLL_INTERVAL_MS = 2500;
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let pollInFlight = false;
  let visibilityHandlerAttached = false;
  /** v0.14.1 (bugs.md #4): track whether the Tauri event listener for
   * backend-driven settings changes is registered. Idempotent — load()
   * can be called multiple times. */
  let settingsChangedListenerAttached = false;

  function syncDashboard(dashboard: ManagerDashboard) {
    update((state) => ({
      ...state,
      ...dashboard,
      projectErrors: Object.fromEntries(
        Object.entries(state.projectErrors ?? {}).filter(([projectId]) =>
          dashboard.projects.some((project) => project.id === projectId)
        )
      ),
      selectedProjectId:
        state.selectedProjectId && dashboard.projects.some((project) => project.id === state.selectedProjectId)
          ? state.selectedProjectId
          : dashboard.projects[0]?.id,
      isBusy: false,
      error: undefined
    }));
  }

  /** Sprint 28h Stage 21 (h)3: every action that holds `isBusy` goes through this. The call runs under a
   *  budget and the flag is released in `finally`, so it comes back after an answer, an error, and no answer
   *  at all (see busy.ts). `fail` defaults to showing the error in the banner. */
  function busyAction<T>(
    what: string,
    budgetMs: number,
    call: () => Promise<T>,
    succeed?: (value: T) => void,
    fail: (error: unknown) => void = (error) =>
      update((state) => ({ ...state, error: normalizeError(error) }))
  ) {
    return runBusy<T>({
      what,
      budgetMs,
      begin: () => update((state) => ({ ...state, isBusy: true, error: undefined })),
      call,
      succeed,
      fail,
      release: () => update((state) => ({ ...state, isBusy: false }))
    });
  }

  async function load() {
    await busyAction("Loading the dashboard", BUDGET_MS.quick, getDashboard, (dashboard) => {
      syncDashboard(dashboard);
      ensureStatusPolling();
      // v0.14.1 (bugs.md #4): listen for backend-driven settings writes
      // (e.g. tray "Autostart on boot" toggle) and reload the dashboard
      // so the Settings UI reflects the new value. Idempotent — guarded
      // by the flag above. Fire-and-forget; the listener lives for the
      // app process's lifetime.
      if (!settingsChangedListenerAttached) {
        settingsChangedListenerAttached = true;
        listen("jawata://settings-changed", async () => {
          try {
            syncDashboard(await getDashboard());
          } catch (e) {
            console.error("dashboard reload after settings-changed failed", e);
          }
        }).catch((e) => {
          console.error("failed to attach settings-changed listener", e);
          settingsChangedListenerAttached = false;
        });
      }
    });
  }

  async function setProjectWorkspaceEntry(projectId: string, workspaceName: string) {
    await busyAction(
      "Moving the project",
      BUDGET_MS.quick,
      () => setProjectWorkspaceApi({ projectId, workspaceName }),
      (dashboard) => {
        syncDashboard(dashboard);
        clearProjectError(projectId);
      },
      (error) => setProjectError(projectId, error)
    );
  }

  async function renameWorkspaceEntry(oldName: string, newName: string) {
    await busyAction("Renaming the workspace", BUDGET_MS.quick, () => renameWorkspaceApi({ oldName, newName }), syncDashboard);
  }

  async function deleteWorkspaceEntry(workspaceName: string) {
    await busyAction("Deleting the workspace", BUDGET_MS.fleet, () => deleteWorkspaceApi(workspaceName), syncDashboard);
  }

  /** studio#28: set a workspace's resident heap ceiling, or clear it with null.
   * Syncs the whole dashboard so the launcher's view and the UI's cannot drift. */
  async function setWorkspaceHeapBound(workspaceName: string, maxHeapMb: number | null) {
    await busyAction(
      "Saving the heap bound",
      BUDGET_MS.quick,
      () => setWorkspaceMaxHeapApi(workspaceName, maxHeapMb),
      syncDashboard
    );
  }

  /** studio#29: turn the debug agent on or off for one workspace's resident. */
  async function setWorkspaceDebuggable(workspaceName: string, debuggable: boolean) {
    await busyAction(
      "Saving the debug setting",
      BUDGET_MS.quick,
      () => setWorkspaceDebuggableApi(workspaceName, debuggable),
      syncDashboard
    );
  }

  async function renameProjectEntry(projectId: string, name: string) {
    await busyAction(
      "Renaming the project",
      BUDGET_MS.quick,
      () => renameProjectApi({ projectId, name }),
      (dashboard) => {
        syncDashboard(dashboard);
        clearProjectError(projectId);
      },
      (error) => setProjectError(projectId, error)
    );
  }

  async function updateManagerSettings(input: UpdateSettingsInput) {
    await runBusy({
      what: "Saving the settings",
      budgetMs: BUDGET_MS.quick,
      begin: () =>
        update((state) => ({
          ...state,
          isBusy: true,
          error: undefined,
          settingsSaveStatus: "saving",
          settingsSaveMessage: "Saving settings..."
        })),
      call: () => updateSettings(input),
      succeed: (dashboard) => {
        syncDashboard(dashboard);
        update((state) => ({
          ...state,
          settingsSaveStatus: "success",
          settingsSaveMessage: "New settings stored successfully."
        }));
      },
      fail: (error) =>
        update((state) => ({
          ...state,
          error: normalizeError(error),
          settingsSaveStatus: "error",
          settingsSaveMessage: `Failed to store settings: ${normalizeError(error)}`
        })),
      release: () => update((state) => ({ ...state, isBusy: false }))
    });
  }

  function markSettingsEdited() {
    update((state) => ({
      ...state,
      error: undefined,
      settingsSaveStatus: "idle",
      settingsSaveMessage: undefined
    }));
  }

  async function redetectMcpClientPaths() {
    await busyAction("Detecting the agents' config paths", BUDGET_MS.quick, redetectMcpClientPathsApi, syncDashboard);
  }

  async function runCleanup(
    cleanupCall: () => Promise<CleanupSummary>
  ) {
    await busyAction(
      "The cleanup",
      BUDGET_MS.fleet,
      async () => ({ summary: await cleanupCall(), dashboard: await getDashboard() }),
      ({ summary, dashboard }) =>
        update((state) => ({
          ...state,
          ...dashboard,
          error: undefined,
          lastCleanupSummary: summary
        }))
    );
  }

  async function cleanAllLogs() {
    await runCleanup(() => cleanLogs());
  }

  async function cleanAllWorkspaces() {
    await runCleanup(() => cleanWorkspaces());
  }

  async function cleanAllGeneratedData() {
    await runCleanup(() => cleanGeneratedData());
  }

  async function probeServices() {
    await runBusy({
      what: "The service probe",
      budgetMs: BUDGET_MS.fleet,
      begin: () =>
        update((state) => ({
          ...state,
          serviceProbeBusy: true,
          serviceProbeError: undefined
        })),
      call: probeServicesApi,
      // Failed probes already carry user-visible detail in lastServiceProbe.
      succeed: (result) =>
        update((state) => ({ ...state, lastServiceProbe: result, serviceProbeError: undefined })),
      fail: (error) => update((state) => ({ ...state, serviceProbeError: normalizeError(error) })),
      release: () => update((state) => ({ ...state, serviceProbeBusy: false }))
    });
  }

  /** The deploy flag has the same rule as `isBusy`: released in `finally` under a budget. */
  function deployAction(what: string, call: () => Promise<DeployToAgentsResult>) {
    // Sprint 28a Stage 2b: the PREVIOUS run's summary is retired the moment a
    // new one starts. It used to survive, so a spinner ran above a summary
    // describing a different deploy.
    return runBusy<DeployToAgentsResult>({
      what,
      budgetMs: BUDGET_MS.deploy,
      begin: () =>
        update((state) => ({
          ...state,
          deployBusy: true,
          deployError: undefined,
          lastDeployResult: undefined
        })),
      call,
      succeed: (result) =>
        update((state) => ({ ...state, deployError: undefined, lastDeployResult: result })),
      // No `lastDeployResult` here on purpose — it was already cleared when
      // this run started. Before that, a failure left the previous run's
      // SUCCESS in place, and dismissing the error revealed it again as though
      // it described what had just happened.
      fail: (error) => update((state) => ({ ...state, deployError: normalizeError(error) })),
      release: () => update((state) => ({ ...state, deployBusy: false }))
    });
  }

  async function deployToAgents(mode: DeployMode, targetClients?: string[]) {
    await deployAction("The deploy", () => deployToAgentsApi({ mode, targetClients }));
  }

  /** The seat part of a deploy only (C11-4): a Seats view change refreshes the command files and
   * nothing else, reported through the same summary a full deploy uses. */
  async function deploySeats() {
    await deployAction("The seat deploy", deploySeatsApi);
  }

  async function deleteProjectEntry(projectId: string) {
    await busyAction("Deleting the project", BUDGET_MS.fleet, () => deleteProject(projectId), syncDashboard);
  }

  async function deleteAllProjectEntries() {
    await busyAction("Deleting all projects", BUDGET_MS.fleet, deleteAllProjects, syncDashboard);
  }

  async function downloadLatestRuntime() {
    await busyAction("The engine download", BUDGET_MS.fleet, downloadOrUpdateJawata, syncDashboard);
  }

  async function startProject(projectId: string) {
    await busyAction(
      "Starting the service",
      BUDGET_MS.fleet,
      () => startRuntime(projectId),
      (status) => mergeRuntimeStatus(projectId, status),
      (error) => setProjectError(projectId, error)
    );
  }

  async function startAllProjects() {
    await busyAction("Starting all services", BUDGET_MS.fleet, startAllRuntimes, syncDashboard);
  }

  async function stopAllProjects() {
    await busyAction("Stopping all services", BUDGET_MS.fleet, stopAllRuntimes, syncDashboard);
  }

  /** Sprint 14 (v0.14.0): stop every workspace, wait for each to settle,
   * then start them all again. Sequenced server-side; the UI sees one
   * round-trip that can take up to ~30 s. */
  async function reloadAllProjects() {
    await busyAction("Reloading all services", BUDGET_MS.fleet, reloadAllRuntimes, syncDashboard);
  }

  async function stopProject(projectId: string) {
    await busyAction(
      "Stopping the service",
      BUDGET_MS.fleet,
      () => stopRuntime(projectId),
      (status) => {
        mergeRuntimeStatus(projectId, status);
        clearProjectError(projectId);
      }
    );
  }

  /** Stop listing an Eclipse workspace that is not running, then show the list without it. */
  async function forgetEclipseWorkspace(path: string) {
    try {
      await forgetEclipseWorkspaceApi(path);
    } catch (e) {
      console.error("forgetting an Eclipse workspace failed", e);
    }
    await refreshEclipseWorkspaces();
  }

  async function refreshEclipseWorkspaces() {
    try {
      const rows = await eclipseWorkspacesApi();
      update((state) => ({ ...state, eclipseWorkspaces: rows }));
    } catch (e) {
      // A failed poll leaves the previous answer standing, the discipline the polls below keep.
      console.error("eclipse workspaces poll failed", e);
    }
  }

  async function refreshAllProjectStatuses() {
    if (pollInFlight || typeof document === "undefined" || document.hidden) {
      return;
    }

    // 28k Stage 20: an Eclipse opened or closed shows up here, not only on the next dashboard load. It does not
    // wait for, or depend on, the headless projects below: a machine with no project still has Eclipse workspaces.
    void refreshEclipseWorkspaces();

    let projectIds: string[] = [];
    update((state) => {
      projectIds = (state.projects ?? []).map((project) => project.id);
      return state;
    });

    if (projectIds.length === 0) {
      return;
    }

    pollInFlight = true;
    try {
      // Use allSettled so a single project's status fetch failing (e.g. it was
      // deleted between snapshotting projectIds and the poll resolving) does
      // not cascade into a global error banner. Per-project failures are
      // silently dropped here; the dashboard refresh on the next mutation
      // will reconcile the project list.
      const results = await Promise.allSettled(
        projectIds.map(async (projectId) => ({
          projectId,
          status: await getRuntimeStatus(projectId)
        }))
      );

      // jawata-studio#24: the resident's own verdict on whether it can READ its
      // workspace, so a row cannot say RUNNING for a project whose directory is
      // gone. A cached read of the canary board — no probe, which is why it can
      // ride this 2.5-second poll at all. A failure here leaves the previous
      // answer standing rather than inventing "readable".
      let readable: Record<string, boolean> | undefined;
      try {
        readable = Object.fromEntries(
          (await workspaceReadability()).map((w) => [w.workspace, w.readable])
        );
      } catch (e) {
        console.error("workspace readability poll failed", e);
      }

      // jawata-studio#37: the same verdict PER PROJECT. The readability above is one
      // boolean per workspace, so a five-project workspace with one failure showed one
      // workspace-level "cannot be read" and five green `running` rows — four correct and
      // one a lie, indistinguishable. The per-project answer already existed and already
      // reached the frontend (fold_resolution reads the resident's own health_check and
      // publishes projectKey, projectPath, healthy, problem, remedy); it was rendered in
      // the Field view and the dashboard read a different source. This is the wiring.
      //
      // Keyed by projectPath, which ProjectResolution's own doc calls "the exact join key
      // for a consumer that knows projects by path rather than by key" — which is what a
      // dashboard row is.
      let unhealthy: Record<string, ProjectResolution> | undefined;
      try {
        unhealthy = {};
        for (const workspace of await resolutionStatus()) {
          for (const project of workspace.projects) {
            if (project.healthy === false) {
              unhealthy[projectPathKey(project.projectPath)] = project;
            }
          }
        }
      } catch (e) {
        console.error("per-project resolution poll failed", e);
        unhealthy = undefined;
      }

      update((state) => {
        const currentIds = new Set((state.projects ?? []).map((project) => project.id));
        const runtimeStatuses = { ...(state.runtimeStatuses ?? {}) };
        for (const result of results) {
          if (result.status === "fulfilled" && currentIds.has(result.value.projectId)) {
            runtimeStatuses[result.value.projectId] = result.value.status;
          }
        }
        return {
          ...state,
          runtimeStatuses,
          workspaceReadable: readable ?? state.workspaceReadable ?? {},
          // A failed poll leaves the previous answer standing rather than inventing
          // "everything is fine" — the same discipline the readability poll above uses.
          projectUnhealthy: unhealthy ?? state.projectUnhealthy ?? {}
        };
      });
    } finally {
      pollInFlight = false;
    }
  }

  function ensureStatusPolling() {
    if (!pollTimer) {
      pollTimer = setInterval(() => {
        void refreshAllProjectStatuses();
      }, STATUS_POLL_INTERVAL_MS);
    }

    if (!visibilityHandlerAttached && typeof document !== "undefined") {
      document.addEventListener("visibilitychange", () => {
        if (!document.hidden) {
          void refreshAllProjectStatuses();
        }
      });
      visibilityHandlerAttached = true;
    }
  }

  function mergeRuntimeStatus(projectId: string, status: RuntimeStatusRecord) {
    update((state) => {
      const projectErrors = { ...(state.projectErrors ?? {}) };
      delete projectErrors[projectId];
      return {
        ...state,
        projectErrors,
        runtimeStatuses: {
          ...(state.runtimeStatuses ?? {}),
          [projectId]: status
        },
        isBusy: false
      };
    });
  }

  function setProjectError(projectId: string, error: unknown) {
    update((state) => ({
      ...state,
      isBusy: false,
      projectErrors: {
        ...(state.projectErrors ?? {}),
        [projectId]: normalizeError(error)
      }
    }));
  }

  function clearProjectError(projectId: string) {
    update((state) => {
      if (!state.projectErrors?.[projectId]) {
        return state;
      }
      const projectErrors = { ...(state.projectErrors ?? {}) };
      delete projectErrors[projectId];
      return {
        ...state,
        projectErrors
      };
    });
  }

  function selectProject(projectId: string) {
    update((state) => ({
      ...state,
      selectedProjectId: projectId
    }));
  }

  function clearError() {
    update((state) => ({
      ...state,
      error: undefined
    }));
  }

  function clearCleanupSummary() {
    update((state) => ({
      ...state,
      lastCleanupSummary: undefined
    }));
  }

  function clearServiceProbeError() {
    update((state) => ({
      ...state,
      serviceProbeError: undefined
    }));
  }

  function clearDeployError() {
    update((state) => ({
      ...state,
      deployError: undefined
    }));
  }


  return {
    subscribe,
    load,
    setProjectWorkspaceEntry,
    forgetEclipseWorkspace,
    renameWorkspaceEntry,
    deleteWorkspaceEntry,
    setWorkspaceHeapBound,
    setWorkspaceDebuggable,
    renameProjectEntry,
    deleteProjectEntry,
    deleteAllProjectEntries,
    updateManagerSettings,
    markSettingsEdited,
    redetectMcpClientPaths,
    downloadLatestRuntime,
    startProject,
    startAllProjects,
    stopAllProjects,
    reloadAllProjects,
    stopProject,
    selectProject,
    clearError,
    cleanAllLogs,
    cleanAllWorkspaces,
    cleanAllGeneratedData,
    clearCleanupSummary,
    probeServices,
    deployToAgents,
    deploySeats,
    clearServiceProbeError,
    clearDeployError
  };
}
