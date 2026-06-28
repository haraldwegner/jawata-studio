<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { createEventDispatcher, onDestroy, tick } from "svelte";
  import type {
    BootstrapStatus,
    CleanupSummary,
    DeployTargetFlags,
    ManagedRuntimeRecord,
    McpClientPathEntry,
    McpClientPaths,
    McpMergeMode,
    ManagerSettings,
    ProjectRecord,
    ReleaseStatus,
    RuntimeStatusRecord,
    ServiceProbeResult,
    UpdateSettingsInput,
    UpdatePolicy
  } from "../api/tauri";

  export let settings: ManagerSettings | undefined;
  export let releaseStatus: ReleaseStatus | undefined;
  export let installedRuntime: ManagedRuntimeRecord | null | undefined = undefined;
  export let bootstrap: BootstrapStatus | undefined;
  export let lastCleanupSummary: CleanupSummary | undefined;
  export let lastServiceProbe: ServiceProbeResult | undefined;
  export let serviceProbeBusy = false;
  export let serviceProbeError: string | undefined;
  export let saveStatus: "idle" | "saving" | "success" | "error" = "idle";
  export let saveMessage: string | undefined;
  export let disabled = false;
  export let projects: ProjectRecord[] = [];
  export let runtimeStatuses: Record<string, RuntimeStatusRecord> = {};

  $: workspaceStats = (() => {
    const byName = new Map<string, { count: number; running: number }>();
    for (const p of projects) {
      const ws = p.workspaceName || "workspace-default";
      const cur = byName.get(ws) ?? { count: 0, running: 0 };
      cur.count += 1;
      if (runtimeStatuses[p.id]?.phase === "running") cur.running += 1;
      byName.set(ws, cur);
    }
    let runningWorkspaces = 0;
    for (const v of byName.values()) if (v.running > 0) runningWorkspaces += 1;
    return { workspaces: byName.size, runningWorkspaces, projects: projects.length };
  })();

  const dispatch = createEventDispatcher<{
    save: UpdateSettingsInput;
    edited: void;
    download: void;
    refresh: void;
    cleanLogs: void;
    cleanWorkspaces: void;
    cleanGeneratedData: void;
    clearCleanupSummary: void;
    probeServices: void;
    clearServiceProbeError: void;
    redetectMcpPaths: void;
  }>();

  const SUCCESS_FADE_MS = 3000;

  let updatePolicy: UpdatePolicy = "ask";
  let autoCheckForUpdates = true;
  let dataRoot = "";
  let useSystemTray = true;
  let autostartOnBoot = false;
  let runtimeKind: "managed" | "localJar" = "managed";
  let localJarPath = "";
  let releaseRepo = "";
  const DEFAULT_RELEASE_REPO = "haraldwegner/goja-mcp";
  const UPSTREAM_RELEASE_REPO = "pzalutski-pixel/goja-mcp";

  type ReleaseRepoChoice = "default" | "upstream" | "custom";
  let releaseRepoChoice: ReleaseRepoChoice = "default";
  let customRepoText = "";
  let releaseRepoChoiceHydrated = false;

  function deriveChoiceFromRepo(repo: string): { choice: ReleaseRepoChoice; custom: string } {
    if (!repo || repo === DEFAULT_RELEASE_REPO) return { choice: "default", custom: "" };
    if (repo === UPSTREAM_RELEASE_REPO) return { choice: "upstream", custom: "" };
    return { choice: "custom", custom: repo };
  }

  function repoFromChoice(choice: ReleaseRepoChoice, custom: string): string {
    switch (choice) {
      case "default": return DEFAULT_RELEASE_REPO;
      case "upstream": return UPSTREAM_RELEASE_REPO;
      case "custom": return custom.trim();
    }
  }

  // Switch source: save immediately, then download from the new source so the
  // user gets the new repo's latest jar in one click. Skipped while the
  // component is still hydrating from initial settings.
  async function applyReleaseSourceChange() {
    if (!releaseRepoChoiceHydrated) return;
    const next = repoFromChoice(releaseRepoChoice, customRepoText);
    if (releaseRepoChoice === "custom" && next.length === 0) {
      // Wait for the user to type a custom repo before saving.
      handleUserEdit();
      return;
    }
    if (next === releaseRepo) {
      handleUserEdit();
      return;
    }
    releaseRepo = next;
    handleUserEdit();
    // Build payload with the new repo, dispatch save, then trigger download.
    // The store's updateManagerSettings + downloadLatestRuntime serialize
    // through state.isBusy so they don't race.
    handleSave();
    dispatch("download");
  }
  let mcpMergeMode: McpMergeMode = "safeMerge";
  let mcpBackupBeforeWrite = true;
  let deployTargets: DeployTargetFlags = {
    cursor: true,
    claude: true,
    claudeDesktop: true,
    antigravity: true,
    intellij: true
  };
  let mcpClientPaths: McpClientPaths = {
    cursor: {},
    claude: {},
    claudeDesktop: {},
    antigravity: {},
    intellij: {}
  };

  let hasHydratedSettings = false;
  let persistedPayload = "";
  let currentPayload = "";
  let isDirty = false;
  let lastAppliedSettingsSnapshot = "";
  let pendingSavePayload: string | undefined;
  let stagedSettings: ManagerSettings | undefined;
  let stagedSettingsSnapshot = "";
  let suppressEditSignal = false;
  let visibleSaveStatus: "idle" | "saving" | "success" | "error" = "idle";
  let visibleSaveMessage = "";
  let footerStatusText = "";
  let awaitingSaveResult = false;
  let interactionDisabled = true;
  let successFadeTimer: ReturnType<typeof setTimeout> | undefined;

  function clearSuccessFadeTimer() {
    if (successFadeTimer) {
      clearTimeout(successFadeTimer);
      successFadeTimer = undefined;
    }
  }

  onDestroy(() => {
    clearSuccessFadeTimer();
  });

  function settingsToSaveInput(nextSettings: ManagerSettings): UpdateSettingsInput {
    return {
      updatePolicy: nextSettings.updatePolicy,
      autoCheckForUpdates: nextSettings.autoCheckForUpdates,
      dataRoot: nextSettings.dataRoot,
      globalRuntimeSource: nextSettings.globalRuntimeSource,
      useSystemTray: nextSettings.useSystemTray,
      autostartOnBoot: nextSettings.autostartOnBoot,
      mcpClientPaths: nextSettings.mcpClientPaths,
      mcpMergeMode: nextSettings.mcpMergeMode,
      mcpBackupBeforeWrite: nextSettings.mcpBackupBeforeWrite,
      deployTargets: nextSettings.deployTargets,
      releaseRepo: nextSettings.releaseRepo
    };
  }

  function normalizeManualOverridePath(value: string | null | undefined): string | null {
    const trimmed = value?.trim();
    return trimmed ? trimmed : null;
  }

  function normalizeMcpPathEntry(entry: McpClientPathEntry | undefined): McpClientPathEntry {
    const manualOverridePath = normalizeManualOverridePath(entry?.manualOverridePath);
    return {
      autoDetectedPath: null,
      manualOverridePath,
      effectivePath: null
    };
  }

  function normalizeMcpClientPaths(paths: McpClientPaths): McpClientPaths {
    return {
      cursor: normalizeMcpPathEntry(paths.cursor),
      claude: normalizeMcpPathEntry(paths.claude),
      claudeDesktop: normalizeMcpPathEntry(paths.claudeDesktop),
      antigravity: normalizeMcpPathEntry(paths.antigravity),
      intellij: normalizeMcpPathEntry(paths.intellij)
    };
  }

  function normalizeSaveInput(input: UpdateSettingsInput): UpdateSettingsInput {
    const trimmedRepo = input.releaseRepo?.trim();
    return {
      updatePolicy: input.updatePolicy,
      autoCheckForUpdates: input.autoCheckForUpdates,
      dataRoot: input.dataRoot.trim(),
      globalRuntimeSource:
        input.globalRuntimeSource.kind === "managed"
          ? { kind: "managed" }
          : { kind: "localJar", jarPath: (input.globalRuntimeSource as any).jarPath?.trim() ?? "" },
      useSystemTray: input.useSystemTray,
      autostartOnBoot: input.autostartOnBoot,
      mcpClientPaths: normalizeMcpClientPaths(input.mcpClientPaths),
      mcpMergeMode: input.mcpMergeMode,
      mcpBackupBeforeWrite: input.mcpBackupBeforeWrite,
      deployTargets: {
        cursor: input.deployTargets.cursor,
        claude: input.deployTargets.claude,
        claudeDesktop: input.deployTargets.claudeDesktop,
        antigravity: input.deployTargets.antigravity,
        intellij: input.deployTargets.intellij
      },
      releaseRepo: trimmedRepo && trimmedRepo.length > 0 ? trimmedRepo : null
    };
  }

  function serializeSaveInput(input: UpdateSettingsInput): string {
    return JSON.stringify(normalizeSaveInput(input));
  }

  function buildSaveInput(): UpdateSettingsInput {
    return {
      updatePolicy,
      autoCheckForUpdates,
      dataRoot,
      useSystemTray,
      autostartOnBoot,
      globalRuntimeSource:
        runtimeKind === "managed"
          ? {
              kind: "managed"
            }
          : {
              kind: "localJar",
              jarPath: localJarPath
            },
      mcpClientPaths,
      mcpMergeMode,
      mcpBackupBeforeWrite,
      deployTargets,
      releaseRepo: releaseRepo.trim().length > 0 ? releaseRepo.trim() : null
    };
  }

  function recomputeDirtyState() {
    if (!hasHydratedSettings) {
      currentPayload = "";
      isDirty = false;
      return;
    }
    currentPayload = serializeSaveInput(buildSaveInput());
    isDirty = currentPayload !== persistedPayload;
  }

  function clearVisibleSaveFeedback() {
    clearSuccessFadeTimer();
    visibleSaveStatus = "idle";
    visibleSaveMessage = "";
  }

  function handleUserEdit() {
    if (!hasHydratedSettings || suppressEditSignal) {
      return;
    }
    awaitingSaveResult = false;
    pendingSavePayload = undefined;
    stagedSettings = undefined;
    stagedSettingsSnapshot = "";
    clearVisibleSaveFeedback();
    dispatch("edited");
    recomputeDirtyState();
  }

  async function handleBoundEdit() {
    if (!hasHydratedSettings || suppressEditSignal) {
      return;
    }
    await tick();
    handleUserEdit();
  }

  function updateSetting(mutator: () => void) {
    mutator();
    handleUserEdit();
  }

  function applySettingsSnapshot(nextSettings: ManagerSettings, snapshot: string) {
    suppressEditSignal = true;
    updatePolicy = nextSettings.updatePolicy;
    autoCheckForUpdates = nextSettings.autoCheckForUpdates;
    dataRoot = nextSettings.dataRoot;
    useSystemTray = nextSettings.useSystemTray;
    autostartOnBoot = nextSettings.autostartOnBoot;
    runtimeKind = nextSettings.globalRuntimeSource.kind;
    mcpMergeMode = nextSettings.mcpMergeMode;
    mcpBackupBeforeWrite = nextSettings.mcpBackupBeforeWrite;
    mcpClientPaths = nextSettings.mcpClientPaths;
    deployTargets = nextSettings.deployTargets;
    releaseRepo = nextSettings.releaseRepo ?? "";
    {
      const derived = deriveChoiceFromRepo(releaseRepo);
      releaseRepoChoice = derived.choice;
      customRepoText = derived.custom;
      releaseRepoChoiceHydrated = true;
    }
    localJarPath =
      nextSettings.globalRuntimeSource.kind === "localJar" ? nextSettings.globalRuntimeSource.jarPath : "";
    hasHydratedSettings = true;
    lastAppliedSettingsSnapshot = snapshot;
    persistedPayload = serializeSaveInput(buildSaveInput());
    currentPayload = persistedPayload;
    isDirty = false;
    pendingSavePayload = undefined;
    suppressEditSignal = false;
  }

  $: if (settings) {
    const settingsSnapshot = JSON.stringify(settings);
    if (settingsSnapshot !== lastAppliedSettingsSnapshot) {
      const incomingPayload = serializeSaveInput(settingsToSaveInput(settings));
      const matchesPendingSave = Boolean(pendingSavePayload && incomingPayload === pendingSavePayload);
      if (!hasHydratedSettings || matchesPendingSave || !isDirty) {
        applySettingsSnapshot(settings, settingsSnapshot);
        stagedSettings = undefined;
        stagedSettingsSnapshot = "";
      } else {
        stagedSettings = settings;
        stagedSettingsSnapshot = settingsSnapshot;
      }
    }
  }

  $: if (
    hasHydratedSettings &&
    !isDirty &&
    stagedSettings &&
    stagedSettingsSnapshot &&
    stagedSettingsSnapshot !== lastAppliedSettingsSnapshot
  ) {
    applySettingsSnapshot(stagedSettings, stagedSettingsSnapshot);
    stagedSettings = undefined;
    stagedSettingsSnapshot = "";
  }

  $: {
    if (saveStatus === "saving" && hasHydratedSettings) {
      clearSuccessFadeTimer();
      awaitingSaveResult = true;
      visibleSaveStatus = "saving";
      visibleSaveMessage = saveMessage ?? "Saving settings...";
    } else if (saveStatus === "error") {
      clearSuccessFadeTimer();
      awaitingSaveResult = false;
      visibleSaveStatus = "error";
      visibleSaveMessage = saveMessage ?? "Failed to store settings.";
    } else if (saveStatus === "success" && hasHydratedSettings && awaitingSaveResult) {
      clearSuccessFadeTimer();
      awaitingSaveResult = false;
      if (!isDirty) {
        visibleSaveStatus = "success";
        visibleSaveMessage = saveMessage ?? "New settings stored successfully.";
        successFadeTimer = setTimeout(() => {
          visibleSaveStatus = "idle";
          visibleSaveMessage = "";
        }, SUCCESS_FADE_MS);
      } else {
        visibleSaveStatus = "idle";
        visibleSaveMessage = "";
      }
    }
  }

  $: footerStatusText = (() => {
    if (visibleSaveStatus === "saving") {
      return visibleSaveMessage || "Saving settings...";
    }
    if (visibleSaveStatus === "error") {
      return visibleSaveMessage || "Failed to store settings.";
    }
    if (visibleSaveStatus === "success" && !isDirty) {
      return visibleSaveMessage || "New settings stored successfully.";
    }
    return "";
  })();

  $: interactionDisabled = disabled || !hasHydratedSettings;

  async function chooseDataRoot() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Select Manager Data Root directory"
    });

    if (typeof selected === "string") {
      updateSetting(() => {
        dataRoot = selected;
      });
    }
  }

  async function chooseLocalJar() {
    const selected = await open({
      directory: false,
      multiple: false,
      title: "Select GOJA JAR",
      filters: [
        {
          name: "Java archive",
          extensions: ["jar"]
        }
      ]
    });

    if (typeof selected === "string") {
      updateSetting(() => {
        localJarPath = selected;
      });
    }
  }

  async function chooseMcpPath(client: keyof McpClientPaths) {
    const selected = await open({
      directory: false,
      multiple: false,
      title: `Select ${client} MCP config file`
    });
    if (typeof selected === "string") {
      setManualMcpPath(client, selected);
    }
  }

  function setManualMcpPath(client: keyof McpClientPaths, path: string) {
    updateSetting(() => {
      mcpClientPaths = {
        ...mcpClientPaths,
        [client]: {
          ...(mcpClientPaths[client] ?? {}),
          manualOverridePath: path
        }
      };
    });
  }

  function clearManualMcpPath(client: keyof McpClientPaths) {
    updateSetting(() => {
      mcpClientPaths = {
        ...mcpClientPaths,
        [client]: {
          ...(mcpClientPaths[client] ?? {}),
          manualOverridePath: null
        }
      };
    });
  }

  function setDeployTargetEnabled(client: keyof DeployTargetFlags, enabled: boolean) {
    updateSetting(() => {
      deployTargets = {
        ...deployTargets,
        [client]: enabled
      };
    });
  }

  function confirmAndDispatch(
    message: string,
    eventName: "cleanLogs" | "cleanWorkspaces" | "cleanGeneratedData"
  ) {
    if (confirm(message)) {
      dispatch(eventName);
    }
  }

  function effectivePath(entry: McpClientPathEntry | undefined): string {
    return entry?.effectivePath ?? entry?.manualOverridePath ?? entry?.autoDetectedPath ?? "not configured";
  }

  function mcpPathSource(entry: McpClientPathEntry | undefined): "manual" | "auto" | "missing" {
    const manual = entry?.manualOverridePath?.trim();
    if (manual) {
      return "manual";
    }
    return entry?.autoDetectedPath ? "auto" : "missing";
  }

  function mcpPathSourceLabel(entry: McpClientPathEntry | undefined): string {
    const source = mcpPathSource(entry);
    if (source === "manual") {
      return "manual";
    }
    if (source === "auto") {
      return "auto-detected";
    }
    return "not configured";
  }

  function shouldShowDetectedHint(entry: McpClientPathEntry | undefined): boolean {
    const autoPath = entry?.autoDetectedPath?.trim();
    const manualPath = entry?.manualOverridePath?.trim();
    return Boolean(autoPath && manualPath && autoPath !== manualPath);
  }

  function handleSave() {
    if (!hasHydratedSettings) {
      return;
    }
    const payload = buildSaveInput();
    pendingSavePayload = serializeSaveInput(payload);
    dispatch("save", payload);
  }
</script>

<section class="panel stack runtime-settings-root">
  <div>
    <h2>Settings</h2>
    <p class="muted">Configure GOJA runtime, machine controls, diagnostics, and MCP location metadata.</p>
  </div>

  <div class="settings-grid">
    <section class="panel stack settings-section runtime-section">
      <div class="section-intro">
        <h3>GOJA Runtime</h3>
        <p class="muted">
          Runtime source and update behavior.
          <a
            href="https://github.com/haraldwegner/goja-studio/releases"
            target="_blank"
            rel="noopener noreferrer"
            style="color: #60a5fa; text-decoration: underline; margin-left: 0.5rem;"
          >
            Check for goja-studio updates
          </a>
        </p>
      </div>

      <label class="field">
        <span>Release source</span>
        <select
          bind:value={releaseRepoChoice}
          disabled={interactionDisabled}
          on:change={applyReleaseSourceChange}
        >
          <option value="default">{DEFAULT_RELEASE_REPO} (recommended fork)</option>
          <option value="upstream">{UPSTREAM_RELEASE_REPO} (upstream)</option>
          <option value="custom">Custom…</option>
        </select>
        <span class="hint">
          Switching source saves and downloads the new repo's latest release.
        </span>
      </label>

      {#if releaseRepoChoice === "custom"}
        <label class="field">
          <span>Custom repo</span>
          <input
            bind:value={customRepoText}
            disabled={interactionDisabled}
            on:change={applyReleaseSourceChange}
            placeholder="owner/repo"
            spellcheck="false"
            autocomplete="off"
          />
        </label>
      {/if}

      <label class="field">
        <span>Global GOJA Source</span>
        <select bind:value={runtimeKind} disabled={interactionDisabled} on:change={handleBoundEdit}>
          <option disabled={!installedRuntime} value="managed">Managed runtime</option>
          <option value="localJar">Local JAR fallback</option>
        </select>
      </label>

      {#if runtimeKind === "managed"}
        <p class="hint">
          Active:
          {#if !installedRuntime}
            not installed
          {:else}
            <strong>v{installedRuntime.version}</strong>
          {/if}
          {#if releaseStatus?.latestVersion && releaseStatus.latestVersion !== installedRuntime?.version}
            &nbsp;&middot;&nbsp;
            <span class="muted">Latest: v{releaseStatus.latestVersion}{releaseStatus.updateAvailable ? " (update available)" : ""}</span>
          {/if}
        </p>
      {/if}

      {#if runtimeKind === "localJar"}
        <label class="field">
          <span>Local GOJA JAR path</span>
          <div class="field-row">
            <input
              bind:value={localJarPath}
              disabled={interactionDisabled}
              on:input={handleBoundEdit}
              placeholder="/path/to/goja.jar"
              required={runtimeKind === "localJar"}
              title="Absolute path to a local goja.jar (overrides the managed runtime)"
            />
            <button
              disabled={interactionDisabled}
              on:click={chooseLocalJar}
              title="Open a file picker to select a local goja.jar"
              type="button"
            >Browse</button>
          </div>
        </label>
      {/if}

      <label class="field">
        <span>Update policy</span>
        <select bind:value={updatePolicy} disabled={interactionDisabled} on:change={handleBoundEdit}>
          <option value="ask">Ask before updating</option>
          <option value="always">Always keep latest</option>
        </select>
      </label>

      <label class="checkbox-row">
        <input bind:checked={autoCheckForUpdates} disabled={interactionDisabled} on:change={handleBoundEdit} type="checkbox" />
        <span>Auto-check release source on dashboard load</span>
      </label>

      {#if updatePolicy === "ask" && releaseStatus?.updateAvailable}
        <div class="actions">
          <button
            disabled={interactionDisabled}
            on:click={() => dispatch("download")}
            title="Download and install the latest goja release from the configured source"
            type="button"
          >
            Download update v{releaseStatus.latestVersion}
          </button>
        </div>
      {/if}
    </section>

    <section class="panel stack settings-section services-section">
      <div class="section-intro">
        <h3>Exposed Services</h3>
        <p class="muted">Probe GOJA live to detect exposed MCP tools.</p>
      </div>
      {#if !lastServiceProbe && !serviceProbeBusy}
        <p class="hint">Click <strong>Test Services</strong> to run a live MCP handshake and list tools.</p>
      {/if}
      <div class="actions">
        <button
          disabled={disabled || serviceProbeBusy}
          on:click={() => dispatch("probeServices")}
          title="Spawn goja, run an MCP handshake, and list the exposed tools"
          type="button"
        >
          {serviceProbeBusy ? "Testing..." : "Test Services"}
        </button>
      </div>
      {#if lastServiceProbe}
        <div class={`probe-result ${lastServiceProbe.ok ? "ok" : "error"}`}>
          {#if lastServiceProbe.ok}
            <p><strong>Probe successful</strong></p>
            <p class="hint">Duration: {lastServiceProbe.durationMs} ms</p>
            <p class="hint">Found {lastServiceProbe.services.length} services</p>
          {:else}
            <p>{lastServiceProbe.detail}</p>
            <p class="hint">Duration: {lastServiceProbe.durationMs} ms</p>
          {/if}
          {#if lastServiceProbe.ok && lastServiceProbe.services.length > 0}
            <ul class="service-list compact service-list-scroll">
              {#each lastServiceProbe.services as service}
                <li class="service-item">
                  <strong title={service.name}>{service.name}</strong>
                  <span class="hint" title={service.description ?? "No description provided by GOJA."}>
                    {service.description ?? "No description provided by GOJA."}
                  </span>
                </li>
              {/each}
            </ul>
          {/if}
          {#if !lastServiceProbe.ok && lastServiceProbe.rawProtocolError}
            <p class="hint">Protocol error: {lastServiceProbe.rawProtocolError}</p>
          {/if}
        </div>
      {/if}
      {#if serviceProbeError && !lastServiceProbe}
        <div class="probe-result error">
          <p>{serviceProbeError}</p>
          <div class="actions">
            <button on:click={() => dispatch("clearServiceProbeError")} type="button">Dismiss</button>
          </div>
        </div>
      {/if}
    </section>

    <section class="panel stack settings-section machine-section">
      <div class="section-intro paired-section-intro">
        <h3>Machine Runtime Controls</h3>
        <p class="muted">Machine-local runtime paths and controls.</p>
      </div>
      <div class="machine-controls-grid">
        <section class="machine-control-card compact-card stack">
          <h4>System Settings</h4>
          <label class="field">
            <span>Manager data root</span>
            <div class="field-row">
              <input
                bind:value={dataRoot}
                disabled={interactionDisabled}
                on:input={handleBoundEdit}
                placeholder="/path/to/manager/data/root"
                required
                title="Folder where the manager stores per-workspace JDT indexes and logs"
              />
              <button
                disabled={interactionDisabled}
                on:click={chooseDataRoot}
                title="Open a folder picker for the manager data root"
                type="button"
              >Browse</button>
            </div>
          </label>
          <label class="checkbox-row compact" title="When enabled, closing the window keeps the manager running in the system tray">
            <input bind:checked={useSystemTray} disabled={interactionDisabled} on:change={handleBoundEdit} type="checkbox" />
            <span>Use system tray</span>
          </label>
          <label class="checkbox-row compact" title="Start the manager automatically at session login (Linux ~/.config/autostart, macOS LaunchAgent, Windows registry). v0.14.1: also restores the workspaces that were running at last shutdown — Quit preserves them, Stop and Quit clears them.">
            <input bind:checked={autostartOnBoot} disabled={interactionDisabled} on:change={handleBoundEdit} type="checkbox" />
            <span>Autostart on boot</span>
          </label>
        </section>

        <section class="machine-control-card diagnostics-card stack">
          <h4><strong>Diagnostics</strong></h4>
          <div class="bootstrap-grid compact">
            <div>
              <span class="label">Projects</span>
              <strong>{bootstrap?.projectsFile ?? "-"}</strong>
            </div>
            <div>
              <span class="label">Settings</span>
              <strong>{bootstrap?.settingsFile ?? "-"}</strong>
            </div>
            <div>
              <span class="label">State</span>
              <strong>{bootstrap?.stateDir ?? "-"}</strong>
            </div>
            <div>
              <span class="label">Data root</span>
              <strong>{bootstrap?.defaultDataRoot ?? "-"}</strong>
            </div>
            <div title="Number of workspaces (and how many have at least one running runtime)">
              <span class="label">Workspaces</span>
              <strong>{workspaceStats.workspaces} ({workspaceStats.runningWorkspaces} running)</strong>
            </div>
            <div title="Total number of registered projects across all workspaces">
              <span class="label">Project count</span>
              <strong>{workspaceStats.projects}</strong>
            </div>
          </div>

          <div class="actions compact">
            <button
              disabled={interactionDisabled}
              on:click={() =>
                confirmAndDispatch(
                  "Delete all manager runtime logs? This keeps project registrations and settings.",
                  "cleanLogs"
                )}
              title="Delete all manager runtime logs. Keeps registrations and settings."
              type="button"
            >
              Clean logs
            </button>
            <button
              disabled={interactionDisabled}
              on:click={() =>
                confirmAndDispatch(
                  "Delete all manager workspace/index caches? This keeps project registrations and settings.",
                  "cleanWorkspaces"
                )}
              title="Delete cached JDT workspace indexes. Keeps registrations and settings."
              type="button"
            >
              Clean workspaces
            </button>
            <button
              disabled={interactionDisabled}
              on:click={() =>
                confirmAndDispatch(
                  "Start from scratch by deleting generated logs + workspaces? Stop running runtimes first.",
                  "cleanGeneratedData"
                )}
              title="Delete logs and workspace caches in one step. Stop runtimes first."
              type="button"
            >
              Start from scratch
            </button>
          </div>

          {#if lastCleanupSummary}
            <div class="banner">
              <span>{lastCleanupSummary.detail} Files: {lastCleanupSummary.deletedFiles}, Dirs: {lastCleanupSummary.deletedDirs}</span>
              <button on:click={() => dispatch("clearCleanupSummary")} type="button">Dismiss</button>
            </div>
          {/if}
        </section>
      </div>
    </section>

    <section class="panel stack settings-section mcp-locations-section">
      <div class="section-intro paired-section-intro mcp-section-intro">
        <div>
          <h3>MCP Config Locations</h3>
          <p class="muted">Review detected config paths and set optional manual overrides.</p>
        </div>
        <button
          disabled={interactionDisabled}
          on:click={() => dispatch("redetectMcpPaths")}
          title="Re-scan the system for default MCP config files for each client"
          type="button"
        >
          Redetect defaults
        </button>
      </div>

      <div class="mcp-clients-scroll">
      {#each [
        ["cursor", "Cursor"],
        ["claude", "Claude Code"],
        ["claudeDesktop", "Claude Desktop"],
        ["antigravity", "Antigravity"],
        ["intellij", "IntelliJ"]
      ] as [clientKey, clientLabel]}
        {@const key = clientKey as keyof McpClientPaths}
        {@const entry = mcpClientPaths[key]}
        <div class="stack mcp-client-card compact">
          <div class="mcp-client-heading">
            <div class="mcp-client-title-row">
              <strong>{clientLabel}</strong>
              <div class="mcp-client-title-controls">
                <label class="checkbox-row compact mcp-deploy-checkbox" title="Include in deploy default">
                  <input
                    checked={deployTargets[key]}
                    disabled={interactionDisabled}
                    on:change={(event) =>
                      setDeployTargetEnabled(key as keyof DeployTargetFlags, (event.currentTarget as HTMLInputElement).checked)}
                    type="checkbox"
                  />
                  <span>Deploy</span>
                </label>
                <span class={`mcp-source-badge ${mcpPathSource(entry)}`}>{mcpPathSourceLabel(entry)}</span>
              </div>
            </div>
          </div>
          <p class="hint mcp-current-path" title={effectivePath(entry)}>
            <strong>Current:</strong> {effectivePath(entry)}
          </p>
          {#if shouldShowDetectedHint(entry)}
            <p class="hint" title={entry?.autoDetectedPath ?? undefined}>
              Detected default: {entry?.autoDetectedPath}
            </p>
          {/if}
          <div class="field-row">
            <input
              disabled={interactionDisabled}
              on:input={(event) => setManualMcpPath(key, (event.currentTarget as HTMLInputElement).value)}
              placeholder="Manual override path"
              title="Manual override for this client's MCP config file (leave empty to use the auto-detected default)"
              value={entry?.manualOverridePath ?? ""}
            />
            <button
              disabled={interactionDisabled}
              on:click={() => chooseMcpPath(key)}
              title="Open a file picker to choose this client's MCP config file"
              type="button"
            >Browse</button>
            <button
              disabled={interactionDisabled}
              on:click={() => clearManualMcpPath(key)}
              title="Clear the manual override and fall back to the auto-detected path"
              type="button"
            >Clear</button>
          </div>
        </div>
      {/each}
      </div>

      <label class="field">
        <span>Merge mode</span>
        <select bind:value={mcpMergeMode} disabled={interactionDisabled} on:change={handleBoundEdit}>
          <option value="safeMerge">Safe merge</option>
          <option value="replaceManagedSection">Replace managed section</option>
        </select>
      </label>

      <label class="checkbox-row" title="Write a .bak copy of each MCP config file before deploying changes">
        <input bind:checked={mcpBackupBeforeWrite} disabled={interactionDisabled} on:change={handleBoundEdit} type="checkbox" />
        <span>Create backup before MCP config write</span>
      </label>
    </section>
  </div>
</section>

<div class="panel settings-save-footer">
  <div class="settings-save-status-wrap">
    {#if footerStatusText}
      <span class={`settings-save-status ${visibleSaveStatus}`}>{footerStatusText}</span>
    {/if}
  </div>
  <button
    class:primary={isDirty && !interactionDisabled}
    class="save-settings-button"
    disabled={interactionDisabled || !isDirty}
    on:click={handleSave}
    title="Persist all settings changes to disk"
    type="button"
  >
    Save settings
  </button>
</div>
