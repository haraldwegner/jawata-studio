<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { confirmDestructive } from "../dialog";
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
    ReleaseStatus,
    ServiceProbeResult,
    UpdateSettingsInput,
    UpdatePolicy,
    WorkspaceRuntimeSetting
  } from "../api/tauri";

  export let settings: ManagerSettings | undefined;
  export let releaseStatus: ReleaseStatus | undefined;
  export let installedRuntime: ManagedRuntimeRecord | null | undefined = undefined;
  export let bootstrap: BootstrapStatus | undefined;
  export let lastCleanupSummary: CleanupSummary | undefined;
  export let lastServiceProbe: ServiceProbeResult | undefined;
  export let serviceProbeBusy = false;
  export let serviceProbeError: string | undefined;
  /** studio#28: each workspace's resident heap ceiling. */
  export let workspaceRuntimeSettings: WorkspaceRuntimeSetting[] = [];

  // studio#28. Only the shape check lives here — the call itself goes through
  // the store like every other operation, so one dashboard sync keeps the UI
  // and the launcher's own view of the bound from drifting apart.
  let heapError: Record<string, string> = {};

  function applyHeapBound(workspaceName: string, raw: string) {
    const trimmed = raw.trim();
    if (trimmed !== "" && !/^\d+$/.test(trimmed)) {
      heapError = { ...heapError, [workspaceName]: "Whole megabytes, or empty for the default." };
      return;
    }
    heapError = { ...heapError, [workspaceName]: "" };
    dispatch("setWorkspaceHeapBound", {
      workspaceName,
      maxHeapMb: trimmed === "" ? null : Number(trimmed)
    });
  }

  export let saveStatus: "idle" | "saving" | "success" | "error" = "idle";
  export let saveMessage: string | undefined;
  export let disabled = false;

  /** The data root cleanup ACTUALLY targets: the persisted setting, falling
   * back to the OS default only when nothing is set. Read from `settings`
   * rather than the `dataRoot` binding, because that binding tracks the input
   * box and would show an unsaved edit as if it were live. */
  $: effectiveDataRoot = settings?.dataRoot?.trim() || bootstrap?.defaultDataRoot || "";

  // S19-46: the opening sentence says what Settings decides (the engine, and which agents Deploy writes to);
  // the workspace and project counts live in the header only. Built in script so no space is lost at a block edge.
  $: settingsSentence = [
    !installedRuntime
      ? "jawata engine not installed"
      : releaseStatus?.kind === "updateHeld"
        ? // 4.4.14: a newer engine was tried before it replaced this one, and it is not the one this studio expects.
          `jawata engine v${releaseStatus.latestVersion} is not used: ${releaseStatus.detail}`
        : releaseStatus?.updateAvailable
          ? `jawata engine v${releaseStatus.latestVersion} is available (v${installedRuntime.version} installed)`
          : "",
    `deploying to ${Object.values(deployTargets).filter(Boolean).length} of ${Object.keys(deployTargets).length} agents`
  ]
    .filter((part) => part !== "")
    .join(" · ");

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
    /** studio#28: `maxHeapMb: null` clears the bound back to the default. */
    setWorkspaceHeapBound: { workspaceName: string; maxHeapMb: number | null };
    /** studio#29: turn the resident's debug agent on or off for this workspace. */
    setWorkspaceDebuggable: { workspaceName: string; debuggable: boolean };
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
  const DEFAULT_RELEASE_REPO = "haraldwegner/jawata-mcp";
  // Sprint 22b: the "(upstream)" release source is GONE — post-fork there is one
  // source (ours); the old pzalutski option pointed at a repo that no longer
  // exists and produced a 404 banner on Settings load. Persisted legacy values
  // self-heal to the default on the backend (LEGACY_DEFAULT_RELEASE_REPOS).

  type ReleaseRepoChoice = "default" | "custom";
  let releaseRepoChoice: ReleaseRepoChoice = "default";
  let customRepoText = "";
  let releaseRepoChoiceHydrated = false;

  function deriveChoiceFromRepo(repo: string): { choice: ReleaseRepoChoice; custom: string } {
    if (!repo || repo === DEFAULT_RELEASE_REPO) return { choice: "default", custom: "" };
    return { choice: "custom", custom: repo };
  }

  function repoFromChoice(choice: ReleaseRepoChoice, custom: string): string {
    switch (choice) {
      case "default": return DEFAULT_RELEASE_REPO;
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
    codex: true,
    copilotCli: true,
    vscode: true,
    grok: true
  };
  let mcpClientPaths: McpClientPaths = {
    cursor: {},
    claude: {},
    codex: {},
    copilotCli: {},
    vscode: {},
    grok: {}
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
      codex: normalizeMcpPathEntry(paths.codex),
      copilotCli: normalizeMcpPathEntry(paths.copilotCli),
      vscode: normalizeMcpPathEntry(paths.vscode),
      grok: normalizeMcpPathEntry(paths.grok)
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
        codex: input.deployTargets.codex,
        copilotCli: input.deployTargets.copilotCli,
        vscode: input.deployTargets.vscode,
        grok: input.deployTargets.grok
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
      title: "Select JAWATA JAR",
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

  async function confirmAndDispatch(
    message: string,
    eventName: "cleanLogs" | "cleanWorkspaces" | "cleanGeneratedData"
  ) {
    if (await confirmDestructive(message)) {
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
    <p
      class="summary-line"
      class:attention={!installedRuntime || (releaseStatus?.updateAvailable ?? false) || releaseStatus?.kind === "updateHeld"}
      title="The engine is what runs your workspaces. Deploy writes to the agents ticked under Agents."
    >
      {settingsSentence}
    </p>
    {#if updatePolicy === "ask" && releaseStatus?.updateAvailable}
      <div class="actions">
        <button
          disabled={interactionDisabled}
          on:click={() => dispatch("download")}
          title="Download and install the latest jawata release from the configured source"
          type="button"
        >
          Download update v{releaseStatus.latestVersion}
        </button>
      </div>
    {/if}
  </div>

  <details class="fold">
    <summary title="Where the jawata engine comes from, whether it updates itself, and a check that it answers.">Engine</summary>
    <div class="fold-body">
      <label class="field">
        <span>Update policy</span>
        <select bind:value={updatePolicy} disabled={interactionDisabled} on:change={handleBoundEdit}>
          <option value="ask">Ask before updating</option>
          <option value="always">Always keep latest</option>
        </select>
        <span class="hint">
          Ask offers a Download button when a newer engine exists; Always downloads and installs it by itself.
        </span>
      </label>

      <label class="checkbox-row" title="Looks for a newer engine each time the Dashboard opens, using the release source under Advanced.">
        <input bind:checked={autoCheckForUpdates} disabled={interactionDisabled} on:change={handleBoundEdit} type="checkbox" />
        <span>Auto-check release source on dashboard load</span>
      </label>

      <!-- S19-50: the one place for this link, beside the update settings (it was also in the Help footer). -->
      <p class="muted">
        <a
          href="https://github.com/haraldwegner/jawata-studio/releases"
          target="_blank"
          rel="noopener noreferrer"
          style="color: #60a5fa; text-decoration: underline;"
        >
          Check for jawata-studio updates
        </a>
      </p>

      <!-- S19-51: where the engine comes from matters to someone running a fork or a local build, not to the
           person who only decides whether to update. -->
      <details class="fold">
        <summary title="Rarely changed. Where the engine is downloaded from, or a local jar to use instead.">Advanced</summary>
        <div class="fold-body">
      <label class="field">
        <span>Release source</span>
        <select
          bind:value={releaseRepoChoice}
          disabled={interactionDisabled}
          on:change={applyReleaseSourceChange}
        >
          <option value="default">{DEFAULT_RELEASE_REPO} (default)</option>
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
        <span>Global JAWATA Source</span>
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
          <span>Local JAWATA JAR path</span>
          <div class="field-row">
            <input
              bind:value={localJarPath}
              disabled={interactionDisabled}
              on:input={handleBoundEdit}
              placeholder="/path/to/jawata.jar"
              required={runtimeKind === "localJar"}
              title="Absolute path to a local jawata.jar (overrides the managed runtime)"
            />
            <button
              disabled={interactionDisabled}
              on:click={chooseLocalJar}
              title="Open a file picker to select a local jawata.jar"
              type="button"
            >Browse</button>
          </div>
        </label>
      {/if}

        </div>
      </details>

      <div class="actions">
        <button
          disabled={disabled || serviceProbeBusy}
          on:click={() => dispatch("probeServices")}
          title="Start the engine, run an MCP handshake and count the tools it exposes. Use it when an agent says jawata is not reachable."
          type="button"
        >
          {serviceProbeBusy ? "Testing..." : "Test services"}
        </button>
        {#if lastServiceProbe}
          <span class={lastServiceProbe.ok ? "hint" : "project-error"}>
            {#if lastServiceProbe.ok}
              Answered: {lastServiceProbe.services.length} tools in {lastServiceProbe.durationMs} ms
            {:else}
              {lastServiceProbe.detail}{#if lastServiceProbe.rawProtocolError} ({lastServiceProbe.rawProtocolError}){/if}
            {/if}
          </span>
        {/if}
      </div>
      {#if serviceProbeError && !lastServiceProbe}
        <div class="probe-result error">
          <p>{serviceProbeError}</p>
          <div class="actions">
            <button on:click={() => dispatch("clearServiceProbeError")} type="button">Dismiss</button>
          </div>
        </div>
      {/if}
    </div>
  </details>

  <details class="fold">
    <summary title="Which of your agents Deploy writes to, and where each keeps its configuration.">Agents</summary>
    <div class="fold-body">
      <div class="actions">
        <button
          disabled={interactionDisabled}
          on:click={() => dispatch("redetectMcpPaths")}
          title="Re-scan the system for default MCP config files for each client"
          type="button"
        >
          Redetect defaults
        </button>
      </div>
      <!-- S19 dogfood (his walkthrough: "Settings confusing"): ONE table, one row per agent. The
           switch is the Sprint 28a 2b design (checked = offered in the deploy picker, pre-ticked;
           unchecked = skipped entirely); the path override opens per row, because it is changed
           only when a file is not where the agent normally keeps it. -->
      <table class="agents-table">
        <thead>
          <tr>
            <th title="Checked: offered in the dashboard deploy picker, pre-ticked. Unchecked: skipped entirely.">Deploy</th>
            <th>Agent</th>
            <th title="The file Deploy writes for this agent. Hover a path for where it came from.">Config file</th>
          </tr>
        </thead>
        <tbody>
          {#each [
            ["cursor", "Cursor"],
            ["claude", "Claude Code"],
            ["codex", "Codex"],
            ["copilotCli", "Copilot CLI"],
            ["vscode", "VS Code"],
            ["grok", "Grok"]
          ] as [clientKey, clientLabel]}
            {@const key = clientKey as keyof McpClientPaths}
            {@const entry = mcpClientPaths[key]}
            <tr class:mcp-client-off={!deployTargets[key as keyof DeployTargetFlags]}>
              <td>
                <input
                  aria-label={`Deploy to ${clientLabel}`}
                  bind:checked={deployTargets[key as keyof DeployTargetFlags]}
                  disabled={interactionDisabled}
                  on:change={handleBoundEdit}
                  title="Checked: offered in the dashboard deploy picker, pre-ticked. Unchecked: skipped entirely."
                  type="checkbox"
                />
              </td>
              <td><strong>{clientLabel}</strong></td>
              <td>
                <details class="agent-path">
                  <summary
                    class="mono"
                    title={`${mcpPathSourceLabel(entry)}${shouldShowDetectedHint(entry) ? ` · detected default: ${entry?.autoDetectedPath}` : ""}. Open to change it.`}
                  >
                    {effectivePath(entry)}
                    {#if mcpPathSource(entry) !== "auto"}
                      <span
                        class="mcp-source-badge {mcpPathSource(entry)}"
                        title={mcpPathSource(entry) === "manual"
                          ? "You set this file by hand; Clear goes back to the detected one."
                          : "No config file was found for this agent, so Deploy skips it until you name one."}
                      >{mcpPathSourceLabel(entry)}</span>
                    {/if}
                  </summary>
                  <div class="field-row">
                    <input
                      disabled={interactionDisabled}
                      on:input={(event) => setManualMcpPath(key, (event.currentTarget as HTMLInputElement).value)}
                      placeholder="another file for this agent"
                      title="Only if this agent keeps its configuration somewhere unusual. Empty = the detected default."
                      value={entry?.manualOverridePath ?? ""}
                    />
                    <button
                      disabled={interactionDisabled}
                      on:click={() => chooseMcpPath(key)}
                      title="Open a file picker to choose this agent's config file"
                      type="button"
                    >Browse</button>
                    <button
                      disabled={interactionDisabled}
                      on:click={() => clearManualMcpPath(key)}
                      title="Back to the detected default"
                      type="button"
                    >Clear</button>
                  </div>
                </details>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>

      <details class="fold">
        <summary title="Rarely changed. Safe merge touches only what the manager owns; the backup is a .bak copy written next to each config before a change.">Advanced</summary>
        <div class="fold-body">
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


        </div>
      </details>
    </div>
  </details>

  <details class="fold">
    <summary title="Where the manager keeps its data, how it starts, and the memory limit of each workspace.">This machine</summary>
    <div class="fold-body">
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

        <section class="machine-control-card compact-card stack">
          <h4>Workspace Runtime</h4>
          <p class="muted">
            Each workspace gets this heap ceiling. Leave a box empty to use the
            2048 MB default. Both settings apply the next time that workspace starts —
            neither a running JVM's ceiling nor its debug agent can be changed.
          </p>
          {#if workspaceRuntimeSettings.length === 0}
            <p class="muted">No workspaces yet.</p>
          {:else}
            {#each workspaceRuntimeSettings as row (row.workspaceName)}
              <label class="field">
                <span>{row.workspaceName}</span>
                <div class="field-row">
                  <input
                    disabled={interactionDisabled}
                    inputmode="numeric"
                    on:change={(event) =>
                      applyHeapBound(row.workspaceName, event.currentTarget.value)}
                    placeholder="2048 (default)"
                    title="Heap ceiling in megabytes for this workspace, applied as -Xmx at its next start"
                    value={row.maxHeapMb ?? ""}
                  />
                  <span class="muted">MB</span>
                </div>
                {#if heapError[row.workspaceName]}
                  <span class="error-text">{heapError[row.workspaceName]}</span>
                {:else}
                  <span class="muted">Starts with {row.effectiveMaxHeapMb} MB{row.maxHeapMb == null ? " (default)" : ""}.</span>
                {/if}
                <!-- studio#29: an open JDWP port runs arbitrary code in that
                     resident's JVM, so this is off unless the user asks, it
                     binds loopback only, and the label says both. -->
                <label class="checkbox-row compact">
                  <input
                    checked={row.debuggable}
                    disabled={interactionDisabled}
                    on:change={(event) =>
                      dispatch("setWorkspaceDebuggable", {
                        workspaceName: row.workspaceName,
                        debuggable: event.currentTarget.checked
                      })}
                    type="checkbox"
                  />
                  <span>
                    Allow debugging
                    {#if row.debuggable && row.debugPort != null}
                      <span class="muted">— 127.0.0.1:{row.debugPort} at next start</span>
                    {:else}
                      <span class="muted">— lets jawata's own debug and profile tools attach; loopback only</span>
                    {/if}
                  </span>
                </label>
              </label>
            {/each}
          {/if}
        </section>


      </div>
      <details class="fold">
        <summary title="Where the manager keeps its settings, state and data. Quote these when you report a problem.">Paths</summary>
        <div class="fold-body">
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
            <!-- Sprint 28a Stage 2b: this showed `defaultDataRoot`, the OS
                 default, while "Clean workspaces" empties the EFFECTIVE root.
                 Anyone who had ever changed the root was reading the name of a
                 folder nothing would touch — a diagnostics row that lies is
                 worse than no row, because it is consulted precisely when
                 something has gone wrong. -->
            <div title={effectiveDataRoot !== (bootstrap?.defaultDataRoot ?? "")
              ? `Overridden. The OS default is ${bootstrap?.defaultDataRoot ?? "-"}`
              : "The OS default for this platform"}>
              <span class="label">Data root</span>
              <strong>{effectiveDataRoot || "-"}</strong>
              {#if effectiveDataRoot && effectiveDataRoot !== (bootstrap?.defaultDataRoot ?? "")}
                <span class="hint">(overridden)</span>
              {/if}
            </div>
          </div>


        </div>
      </details>
      <details class="fold">
        <summary title="Delete generated logs and caches. Your settings and registered projects survive; stop running services first.">Maintenance</summary>
        <div class="fold-body">
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
              title="Deletes generated logs and workspace caches only — settings and registered projects survive. Stop runtimes first."
              type="button"
            >
              Clean logs & workspaces
            </button>
          </div>

          {#if lastCleanupSummary}
            <div class="banner">
              <span>{lastCleanupSummary.detail} Files: {lastCleanupSummary.deletedFiles}, Dirs: {lastCleanupSummary.deletedDirs}</span>
              <button on:click={() => dispatch("clearCleanupSummary")} type="button">Dismiss</button>
            </div>
          {/if}

        </div>
      </details>
    </div>
  </details>
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
