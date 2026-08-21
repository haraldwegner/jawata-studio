import { writable } from "svelte/store";
import { listen } from "@tauri-apps/api/event";
import {
  addProject,
  cleanGeneratedData,
  cleanLogs,
  cleanWorkspaces,
  deployToAgents as deployToAgentsApi,
  deleteAllProjects,
  deleteProject,
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
  renameProject as renameProjectApi,
  updateSettings,
  type AddProjectInput,
  type CleanupSummary,
  type DeployMode,
  type DeployToAgentsResult,
  type ManagerDashboard,
  type ServiceProbeResult,
  type RuntimeStatusRecord,
  type UpdateSettingsInput,
  workspaceReadability} from "../api/tauri";

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

  async function load() {
    update((state) => ({ ...state, isBusy: true, error: undefined }));

    try {
      syncDashboard(await getDashboard());
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
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function addProjectEntry(input: AddProjectInput) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));

    try {
      await addProject(input);
      syncDashboard(await getDashboard());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function setProjectWorkspaceEntry(projectId: string, workspaceName: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await setProjectWorkspaceApi({ projectId, workspaceName }));
      clearProjectError(projectId);
    } catch (error) {
      setProjectError(projectId, error);
    }
  }

  async function renameWorkspaceEntry(oldName: string, newName: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await renameWorkspaceApi({ oldName, newName }));
    } catch (error) {
      update((state) => ({ ...state, error: String(error) }));
    } finally {
      update((state) => ({ ...state, isBusy: false }));
    }
  }

  async function deleteWorkspaceEntry(workspaceName: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await deleteWorkspaceApi(workspaceName));
    } catch (error) {
      update((state) => ({ ...state, error: String(error) }));
    } finally {
      update((state) => ({ ...state, isBusy: false }));
    }
  }

  /** studio#28: set a workspace's resident heap ceiling, or clear it with null.
   * Syncs the whole dashboard so the launcher's view and the UI's cannot drift. */
  async function setWorkspaceHeapBound(workspaceName: string, maxHeapMb: number | null) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await setWorkspaceMaxHeapApi(workspaceName, maxHeapMb));
    } catch (error) {
      update((state) => ({ ...state, error: String(error) }));
    } finally {
      update((state) => ({ ...state, isBusy: false }));
    }
  }

  async function renameProjectEntry(projectId: string, name: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await renameProjectApi({ projectId, name }));
      clearProjectError(projectId);
    } catch (error) {
      setProjectError(projectId, error);
    } finally {
      update((state) => ({ ...state, isBusy: false }));
    }
  }

  async function updateManagerSettings(input: UpdateSettingsInput) {
    update((state) => ({
      ...state,
      isBusy: true,
      error: undefined,
      settingsSaveStatus: "saving",
      settingsSaveMessage: "Saving settings..."
    }));

    try {
      syncDashboard(await updateSettings(input));
      update((state) => ({
        ...state,
        settingsSaveStatus: "success",
        settingsSaveMessage: "New settings stored successfully."
      }));
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error),
        settingsSaveStatus: "error",
        settingsSaveMessage: `Failed to store settings: ${normalizeError(error)}`
      }));
    }
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
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await redetectMcpClientPathsApi());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function runCleanup(
    cleanupCall: () => Promise<CleanupSummary>
  ) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      const summary = await cleanupCall();
      const dashboard = await getDashboard();
      update((state) => ({
        ...state,
        ...dashboard,
        isBusy: false,
        error: undefined,
        lastCleanupSummary: summary
      }));
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
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
    update((state) => ({
      ...state,
      serviceProbeBusy: true,
      serviceProbeError: undefined
    }));

    try {
      const result = await probeServicesApi();
      update((state) => ({
        ...state,
        serviceProbeBusy: false,
        lastServiceProbe: result,
        // Failed probes already carry user-visible detail in lastServiceProbe.
        serviceProbeError: undefined
      }));
    } catch (error) {
      update((state) => ({
        ...state,
        serviceProbeBusy: false,
        serviceProbeError: normalizeError(error)
      }));
    }
  }

  async function deployToAgents(mode: DeployMode, targetClients?: string[]) {
    // Sprint 28a Stage 2b: the PREVIOUS run's summary is retired the moment a
    // new one starts. It used to survive, so a spinner ran above a summary
    // describing a different deploy.
    update((state) => ({
      ...state,
      deployBusy: true,
      deployError: undefined,
      lastDeployResult: undefined
    }));
    try {
      const result = await deployToAgentsApi({
        mode,
        targetClients
      });
      update((state) => ({
        ...state,
        deployBusy: false,
        deployError: undefined,
        lastDeployResult: result
      }));
    } catch (error) {
      // No `lastDeployResult` here on purpose — it was already cleared when
      // this run started. Before that, a failure left the previous run's
      // SUCCESS in place, and dismissing the error revealed it again as though
      // it described what had just happened.
      update((state) => ({
        ...state,
        deployBusy: false,
        deployError: normalizeError(error)
      }));
    }
  }

  async function deleteProjectEntry(projectId: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await deleteProject(projectId));
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function deleteAllProjectEntries() {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await deleteAllProjects());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function downloadLatestRuntime() {
    update((state) => ({ ...state, isBusy: true, error: undefined }));

    try {
      syncDashboard(await downloadOrUpdateJawata());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function startProject(projectId: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));

    try {
      const status = await startRuntime(projectId);
      mergeRuntimeStatus(projectId, status);
    } catch (error) {
      setProjectError(projectId, error);
    }
  }

  async function startAllProjects() {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await startAllRuntimes());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function stopAllProjects() {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await stopAllRuntimes());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  /** Sprint 14 (v0.14.0): stop every workspace, wait for each to settle,
   * then start them all again. Sequenced server-side; the UI sees one
   * round-trip that can take up to ~30 s. */
  async function reloadAllProjects() {
    update((state) => ({ ...state, isBusy: true, error: undefined }));
    try {
      syncDashboard(await reloadAllRuntimes());
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function stopProject(projectId: string) {
    update((state) => ({ ...state, isBusy: true, error: undefined }));

    try {
      const status = await stopRuntime(projectId);
      mergeRuntimeStatus(projectId, status);
      clearProjectError(projectId);
    } catch (error) {
      update((state) => ({
        ...state,
        isBusy: false,
        error: normalizeError(error)
      }));
    }
  }

  async function refreshProjectStatus(projectId: string) {
    try {
      const status = await getRuntimeStatus(projectId);
      update((state) => ({
        ...state,
        runtimeStatuses: {
          ...(state.runtimeStatuses ?? {}),
          [projectId]: status
        }
      }));
      clearProjectError(projectId);
    } catch (error) {
      update((state) => ({
        ...state,
        error: normalizeError(error)
      }));
    }
  }

  async function refreshAllProjectStatuses() {
    if (pollInFlight || typeof document === "undefined" || document.hidden) {
      return;
    }

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
          workspaceReadable: readable ?? state.workspaceReadable ?? {}
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
    addProjectEntry,
    setProjectWorkspaceEntry,
    renameWorkspaceEntry,
    deleteWorkspaceEntry,
    setWorkspaceHeapBound,
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
    refreshProjectStatus,
    selectProject,
    clearError,
    cleanAllLogs,
    cleanAllWorkspaces,
    cleanAllGeneratedData,
    clearCleanupSummary,
    probeServices,
    deployToAgents,
    clearServiceProbeError,
    clearDeployError
  };
}
