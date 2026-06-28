<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import ContextMenu from "./lib/components/ContextMenu.svelte";
  import ProjectForm from "./lib/components/ProjectForm.svelte";
  import ProjectList from "./lib/components/ProjectList.svelte";
  import WorkspaceList from "./lib/components/WorkspaceList.svelte";
  import RuntimeSettings from "./lib/components/RuntimeSettings.svelte";
  import HelpView from "./lib/components/HelpView.svelte";
  import { createAppStore } from "./lib/stores/app";
  import {
    type AddProjectInput,
    performQuitAction,
    type UpdateSettingsInput
  } from "./lib/api/tauri";
  import { formatManagerVersionForUi, getManagerAppVersion } from "./lib/appVersion";

  const appStore = createAppStore();
  let managerAppVersion = "";
  const MIN_LEFT_PANEL_WIDTH = 260;
  const MAX_LEFT_PANEL_WIDTH = 560;
  const MIN_RIGHT_PANEL_WIDTH = 420;
  const SPLITTER_WIDTH = 12;

  let currentView: "dashboard" | "settings" | "help" = "dashboard";
  /** Sprint 10 v0.10.4: which workspace the Register Project / Import
   * forms add to. Owned at App.svelte so the Workspaces card and the
   * forms stay in sync. Defaults to the first existing workspace if
   * one exists, else empty (prompts the user to create one). */
  let activeWorkspaceName: string = "";
  /** Workspaces that exist in the UI but have no projects yet —
   * typically just-created via "+ New workspace…". They aren't in
   * projects.json so we keep them pinned client-side for the session
   * so the user can switch back to them. They graduate to "real" once
   * a project is added; they vanish if explicitly deleted. */
  let pinnedEmptyWorkspaces = new Set<string>();
  let leftPanelWidth = 320;
  let isDraggingSplitter = false;
  let isCompactLayout = false;
  let dashboardLayoutEl: HTMLElement | null = null;
  let splitterPointerId: number | null = null;
  let dragStartX = 0;
  let dragStartWidth = 0;
  let lastDashboardWidth = 0;
  let unlistenQuitRequested: (() => void) | undefined;

  type ContextMenuItem = {
    label: string;
    onSelect: () => void;
    danger?: boolean;
    disabled?: boolean;
  };
  /** Global context menu (text-field Copy/Cut/Paste/Select all).
   * Per-component menus stopPropagation so they never reach this. */
  let globalContextMenu: { x: number; y: number; items: ContextMenuItem[] } | null = null;

  function isEditableTextTarget(el: EventTarget | null): el is HTMLInputElement | HTMLTextAreaElement {
    if (!(el instanceof HTMLElement)) return false;
    if (el.tagName === "TEXTAREA") return true;
    if (el.tagName === "INPUT") {
      const type = (el as HTMLInputElement).type;
      return type === "text" || type === "search" || type === "url" || type === "email" || type === "number" || type === "password" || type === "tel" || type === "";
    }
    return false;
  }

  function handleGlobalContextMenu(event: MouseEvent) {
    // Always suppress the default OS/browser menu (back/forward/inspect-element noise).
    event.preventDefault();
    const target = event.target;
    if (!isEditableTextTarget(target)) {
      // Not an editable field → no menu at all.
      globalContextMenu = null;
      return;
    }
    const input = target;
    const items: ContextMenuItem[] = [
      {
        label: "Cut",
        disabled: input.disabled || input.readOnly || input.selectionStart === input.selectionEnd,
        onSelect: () => document.execCommand("cut"),
      },
      {
        label: "Copy",
        disabled: input.selectionStart === input.selectionEnd,
        onSelect: () => document.execCommand("copy"),
      },
      {
        label: "Paste",
        disabled: input.disabled || input.readOnly,
        onSelect: () => document.execCommand("paste"),
      },
      {
        label: "Select all",
        onSelect: () => input.select(),
      },
    ];
    globalContextMenu = { x: event.clientX, y: event.clientY, items };
  }

  $: selectedProject = $appStore.projects?.find((project) => project.id === $appStore.selectedProjectId);
  $: workspacesWithProjects = Array.from(
    new Set(($appStore.projects ?? []).map((p) => p.workspaceName))
  ).sort();
  /** All workspaces the UI knows about — those with projects plus any
   * pinned empties (newly-created via the "+ New workspace…" inline
   * action). Sorted alphabetically, deduped. */
  $: knownWorkspaces = (() => {
    const all = new Set<string>(workspacesWithProjects);
    for (const name of pinnedEmptyWorkspaces) all.add(name);
    if (activeWorkspaceName) all.add(activeWorkspaceName);
    return Array.from(all).filter((s) => s.length > 0).sort();
  })();
  // Default the active workspace once we have data: prefer the manager's
  // suggestion (most recent), else the first existing, else stays empty.
  $: if (!activeWorkspaceName) {
    if ($appStore.suggestedWorkspaceName) {
      activeWorkspaceName = $appStore.suggestedWorkspaceName;
    } else if (workspacesWithProjects.length > 0) {
      activeWorkspaceName = workspacesWithProjects[0];
    }
  }
  // When projects.json picks up a newly-active workspace (the user
  // actually added a project to it), drop it from the pinned-empty set.
  $: for (const name of workspacesWithProjects) {
    if (pinnedEmptyWorkspaces.has(name)) {
      pinnedEmptyWorkspaces.delete(name);
      pinnedEmptyWorkspaces = pinnedEmptyWorkspaces;
    }
  }

  function activateWorkspace(name: string) {
    activeWorkspaceName = name;
    // If the workspace has no projects yet, pin it so it stays visible
    // when the user switches to a different workspace and back.
    if (!workspacesWithProjects.includes(name)) {
      pinnedEmptyWorkspaces.add(name);
      pinnedEmptyWorkspaces = pinnedEmptyWorkspaces;
    }
  }
  $: selectedStatus = selectedProject
    ? $appStore.runtimeStatuses?.[selectedProject.id]
    : undefined;
  $: runtimeSource = $appStore.settings?.globalRuntimeSource;
  $: runtimeSubtitle = (() => {
    const ver =
      managerAppVersion.length > 0
        ? formatManagerVersionForUi(managerAppVersion)
        : "…";
    const prefix = `goja-studio ${ver} | `;
    if (runtimeSource?.kind === "localJar") {
      const jarPath = runtimeSource.jarPath?.trim() ?? "";
      if (!jarPath) {
        return `${prefix}GOJA local JAR (path not set)`;
      }
      const jarName = jarPath.split(/[\\/]/).pop() ?? jarPath;
      return `${prefix}GOJA local JAR ${jarName}`;
    }

    if ($appStore.installedRuntime?.version) {
      return `${prefix}GOJA ${$appStore.installedRuntime.version}${
        $appStore.releaseStatus?.updateAvailable
          ? ` (update: ${$appStore.releaseStatus.latestVersion ?? "available"})`
          : ""
      }`;
    }
    return `${prefix}GOJA runtime not downloaded`;
  })();
  // Sprint 10 v0.10.4: workspace + project count summary, shown next to
  // the runtime subtitle so the user sees managed scope at a glance.
  $: workspaceSubtitle = (() => {
    const wsCount = knownWorkspaces.length;
    const projCount = $appStore.projects?.length ?? 0;
    if (wsCount === 0 && projCount === 0) return "no workspaces yet";
    return `${wsCount} workspace${wsCount === 1 ? "" : "s"} · ${projCount} project${projCount === 1 ? "" : "s"}`;
  })();
  $: runtimeSubtitleTitle =
    runtimeSource?.kind === "localJar" && runtimeSource.jarPath?.trim()
      ? runtimeSource.jarPath
      : runtimeSubtitle;

  onMount(() => {
    void getManagerAppVersion().then((v) => {
      managerAppVersion = v;
    });
    appStore.load();
    void subscribeQuitRequested();

    if (typeof window !== "undefined") {
      const mediaQuery = window.matchMedia("(max-width: 960px)");
      const updateCompact = () => {
        const wasCompact = isCompactLayout;
        isCompactLayout = mediaQuery.matches;
        if (wasCompact && !isCompactLayout) {
          lastDashboardWidth = getDashboardWidth();
          applyClampedWidth();
          return;
        }
        lastDashboardWidth = getDashboardWidth();
      };
      updateCompact();
      window.addEventListener("resize", handleWindowResize);
      mediaQuery.addEventListener("change", updateCompact);

      return () => {
        window.removeEventListener("resize", handleWindowResize);
        mediaQuery.removeEventListener("change", updateCompact);
      };
    }
  });

  onDestroy(() => {
    stopSplitterDrag();
    unlistenQuitRequested?.();
    unlistenQuitRequested = undefined;
  });

  interface QuitPromptPayload {
    source: "tray" | "window";
    runningServices: number;
    trayEnabled: boolean;
  }

  async function subscribeQuitRequested() {
    unlistenQuitRequested = await listen<QuitPromptPayload>(
      "goja://quit-requested",
      async (event) => {
        await handleQuitPrompt(event.payload);
      }
    );
  }

  async function handleQuitPrompt(payload: QuitPromptPayload) {
    const running = payload.runningServices;
    if (running <= 0) {
      if (confirm("Do you really want to shut down goja-studio?")) {
        await performQuitAction("quit");
      }
      return;
    }

    if (payload.source === "tray") {
      const stopAndQuit = confirm(
        `Quit will stop ${running} running MCP service(s). Press OK to stop services and quit.`
      );
      if (stopAndQuit) {
        await performQuitAction("stopAndQuit");
        return;
      }
      if (payload.trayEnabled && confirm("Keep services running and hide window to tray instead?")) {
        await performQuitAction("hideToTray");
      }
      return;
    }

    if (payload.trayEnabled) {
      if (confirm("MCP services are still running. Hide to tray and keep them running?")) {
        await performQuitAction("hideToTray");
        return;
      }
    }
    if (confirm(`Quit and stop ${running} running MCP service(s)?`)) {
      await performQuitAction("stopAndQuit");
    }
  }

  function handleProjectSubmit(event: CustomEvent<AddProjectInput>) {
    appStore.addProjectEntry(event.detail);
  }

  function handleSettingsSave(event: CustomEvent<UpdateSettingsInput>) {
    appStore.updateManagerSettings(event.detail);
  }

  function clampLeftPanelWidth(width: number): number {
    if (isCompactLayout) {
      return leftPanelWidth;
    }

    const layoutWidth = dashboardLayoutEl?.clientWidth ?? window.innerWidth;
    const maxAllowedByLayout = Math.max(
      MIN_LEFT_PANEL_WIDTH,
      layoutWidth - MIN_RIGHT_PANEL_WIDTH - SPLITTER_WIDTH
    );
    const maxWidth = Math.min(MAX_LEFT_PANEL_WIDTH, maxAllowedByLayout);
    return Math.max(MIN_LEFT_PANEL_WIDTH, Math.min(width, maxWidth));
  }

  function getDashboardWidth(): number {
    if (typeof window === "undefined") {
      return 0;
    }
    return dashboardLayoutEl?.clientWidth ?? window.innerWidth;
  }

  function applyClampedWidth() {
    if (isCompactLayout || currentView !== "dashboard") {
      return;
    }
    const clamped = clampLeftPanelWidth(leftPanelWidth);
    if (clamped !== leftPanelWidth) {
      leftPanelWidth = clamped;
    }
  }

  function stopSplitterDrag() {
    if (!isDraggingSplitter) {
      return;
    }
    isDraggingSplitter = false;
    splitterPointerId = null;
    window.removeEventListener("pointermove", handleSplitterPointerMove);
    window.removeEventListener("pointerup", handleSplitterPointerUp);
    window.removeEventListener("pointercancel", handleSplitterPointerUp);
  }

  function handleSplitterPointerMove(event: PointerEvent) {
    if (!isDraggingSplitter) {
      return;
    }
    const delta = event.clientX - dragStartX;
    leftPanelWidth = clampLeftPanelWidth(dragStartWidth + delta);
  }

  function handleSplitterPointerUp(event: PointerEvent) {
    if (splitterPointerId !== null && event.pointerId !== splitterPointerId) {
      return;
    }
    stopSplitterDrag();
    lastDashboardWidth = getDashboardWidth();
  }

  function handleSplitterPointerDown(event: PointerEvent) {
    if (isCompactLayout) {
      return;
    }
    event.preventDefault();
    splitterPointerId = event.pointerId;
    dragStartX = event.clientX;
    dragStartWidth = leftPanelWidth;
    isDraggingSplitter = true;
    leftPanelWidth = clampLeftPanelWidth(leftPanelWidth);
    window.addEventListener("pointermove", handleSplitterPointerMove);
    window.addEventListener("pointerup", handleSplitterPointerUp);
    window.addEventListener("pointercancel", handleSplitterPointerUp);
  }

  function handleWindowResize() {
    if (isCompactLayout || currentView !== "dashboard") {
      lastDashboardWidth = getDashboardWidth();
      return;
    }

    const nextWidth = getDashboardWidth();
    if (nextWidth <= 0) {
      return;
    }

    if (lastDashboardWidth <= 0) {
      lastDashboardWidth = nextWidth;
      applyClampedWidth();
      return;
    }

    const scaledWidth = (leftPanelWidth / lastDashboardWidth) * nextWidth;
    const clamped = clampLeftPanelWidth(scaledWidth);
    if (clamped !== leftPanelWidth) {
      leftPanelWidth = clamped;
    }
    lastDashboardWidth = nextWidth;
  }

  $: if (currentView === "dashboard") {
    applyClampedWidth();
    const width = getDashboardWidth();
    if (width > 0) {
      lastDashboardWidth = width;
    }
  }
</script>

<svelte:head>
  <title>goja-studio</title>
</svelte:head>

<svelte:window on:contextmenu={handleGlobalContextMenu} />

<main class="app-shell full-height-shell">
  <header class="hero panel">
    <div class="header-content">
      <div>
        <h1>goja-studio</h1>
        <p class="title-subline muted" title={runtimeSubtitleTitle}>{runtimeSubtitle}</p>
        <p class="title-subline muted">{workspaceSubtitle}</p>
      </div>
      <nav class="nav-tabs">
        <button
          class="tab {currentView === 'dashboard' ? 'active' : ''}"
          on:click={() => (currentView = 'dashboard')}
          type="button"
        >
          Dashboard
        </button>
        <button
          class="tab {currentView === 'settings' ? 'active' : ''}"
          on:click={() => (currentView = 'settings')}
          type="button"
        >
          Settings
        </button>
        <button
          class="tab {currentView === 'help' ? 'active' : ''}"
          on:click={() => (currentView = 'help')}
          type="button"
        >
          Help
        </button>
      </nav>
    </div>
  </header>

  {#if $appStore.error}
    <div class="banner error">
      <span>{$appStore.error}</span>
      <button on:click={() => appStore.clearError()} type="button">Dismiss</button>
    </div>
  {/if}

  {#if currentView === 'dashboard'}
    <section class="dashboard-main">
      <section class="dashboard-content">
        <section
          bind:this={dashboardLayoutEl}
          class={`layout dashboard-layout ${isDraggingSplitter ? "is-resizing" : ""}`}
          style={`--left-panel-width: ${leftPanelWidth}px;`}
        >
          <div class="dashboard-column dashboard-column-stack">
            <WorkspaceList
              activeWorkspaceName={activeWorkspaceName}
              disabled={$appStore.isBusy}
              knownWorkspaces={knownWorkspaces}
              onSelect={activateWorkspace}
              onRename={(oldName, newName) => appStore.renameWorkspaceEntry(oldName, newName)}
              onDelete={(name) => {
                appStore.deleteWorkspaceEntry(name);
                pinnedEmptyWorkspaces.delete(name);
                pinnedEmptyWorkspaces = pinnedEmptyWorkspaces;
                if (activeWorkspaceName === name) {
                  activeWorkspaceName = workspacesWithProjects.find((n) => n !== name) ?? "";
                }
              }}
              onProjectsDropped={(workspaceName, projectIds) => {
                for (const id of projectIds) {
                  appStore.setProjectWorkspaceEntry(id, workspaceName);
                }
              }}
              projects={$appStore.projects ?? []}
              runtimeStatuses={$appStore.runtimeStatuses ?? {}}
            />
            <ProjectForm
              activeWorkspaceName={activeWorkspaceName}
              disabled={$appStore.isBusy}
              on:submit={handleProjectSubmit}
              on:imported={() => appStore.load()}
            />
          </div>

          <div
            aria-hidden={isCompactLayout}
            class="dashboard-splitter"
            on:pointerdown={handleSplitterPointerDown}
            role="separator"
            tabindex="-1"
          ></div>

          <div class="dashboard-column">
            <ProjectList
              disabled={$appStore.isBusy}
              onRefresh={(projectId) => appStore.refreshProjectStatus(projectId)}
              onSelect={(projectId) => appStore.selectProject(projectId)}
              onStart={(projectId) => appStore.startProject(projectId)}
              onStartAll={() => appStore.startAllProjects()}
              onStop={(projectId) => appStore.stopProject(projectId)}
              onStopAll={() => appStore.stopAllProjects()}
              onReloadAll={() => appStore.reloadAllProjects()}
              onDelete={(projectId) => appStore.deleteProjectEntry(projectId)}
              onDeleteAll={async () => {
                // Sprint 10 v0.10.4: "Delete all" means everything —
                // every workspace's runtime, projects, JDT data dir,
                // and any pinned-empty workspaces. Iterates known
                // workspaces calling delete_workspace (which wipes
                // each one fully); then resets client-side pin state
                // and active workspace.
                for (const name of [...knownWorkspaces]) {
                  await appStore.deleteWorkspaceEntry(name);
                }
                pinnedEmptyWorkspaces.clear();
                pinnedEmptyWorkspaces = pinnedEmptyWorkspaces;
                activeWorkspaceName = "";
              }}
              onDeploy={(mode, targetClients) => appStore.deployToAgents(mode, targetClients)}
              onSetWorkspace={(projectId, workspaceName) => appStore.setProjectWorkspaceEntry(projectId, workspaceName)}
              onRenameProject={(projectId, name) => appStore.renameProjectEntry(projectId, name)}
              onRenameWorkspace={(oldName, newName) => appStore.renameWorkspaceEntry(oldName, newName)}
              onDeleteWorkspace={(name) => {
                appStore.deleteWorkspaceEntry(name);
                pinnedEmptyWorkspaces.delete(name);
                pinnedEmptyWorkspaces = pinnedEmptyWorkspaces;
                if (activeWorkspaceName === name) {
                  activeWorkspaceName = workspacesWithProjects.find((n) => n !== name) ?? "";
                }
              }}
              knownWorkspaces={knownWorkspaces}
              deployTargetDefaults={$appStore.settings?.deployTargets ?? { cursor: true, claude: true, claudeDesktop: true, antigravity: true, intellij: true }}
              deployBusy={$appStore.deployBusy ?? false}
              deployError={$appStore.deployError}
              lastDeployResult={$appStore.lastDeployResult}
              projects={$appStore.projects ?? []}
              projectErrors={$appStore.projectErrors ?? {}}
              runtimeStatuses={$appStore.runtimeStatuses ?? {}}
              selectedProjectId={$appStore.selectedProjectId}
            />
          </div>
        </section>
      </section>

      {#if ($appStore.projects ?? []).length > 0}
        <section class="panel detail-panel dashboard-footer-panel">
          <div class="detail-header">
            <h2>Selected Project Status</h2>
            {#if selectedProject}
              <button
                aria-label={`Refresh status for ${selectedProject.name}`}
                class="icon-refresh"
                on:click={() => appStore.refreshProjectStatus(selectedProject.id)}
                title="Refresh status"
                type="button"
              >
                ↻
              </button>
            {/if}
          </div>

          {#if selectedProject && selectedStatus}
            <dl class="detail-grid">
              <div>
                <dt>Name</dt>
                <dd>{selectedProject.name}</dd>
              </div>
              <div>
                <dt>Project path</dt>
                <dd title={selectedProject.projectPath}>{selectedProject.projectPath}</dd>
              </div>
              <div>
                <dt>Workspace</dt>
                <dd>{selectedProject.workspaceName}</dd>
              </div>
              <div>
                <dt>PID</dt>
                <dd>{selectedStatus.pid ?? "Not running"}</dd>
              </div>
              <div>
                <dt>Phase / Health</dt>
                <dd title={selectedStatus.detail}>
                  {selectedStatus.phase} - {selectedStatus.detail}
                </dd>
              </div>
            </dl>
          {:else}
            <p class="muted">Choose a project to inspect runtime and health details.</p>
          {/if}
        </section>
      {/if}
    </section>
  {:else if currentView === 'settings'}
    <section class="dashboard-main">
      {#if $appStore.settings}
        <RuntimeSettings
          bootstrap={$appStore.bootstrap}
          disabled={$appStore.isBusy}
          installedRuntime={$appStore.installedRuntime}
          lastCleanupSummary={$appStore.lastCleanupSummary}
          lastServiceProbe={$appStore.lastServiceProbe}
          projects={$appStore.projects ?? []}
          releaseStatus={$appStore.releaseStatus}
          runtimeStatuses={$appStore.runtimeStatuses ?? {}}
          serviceProbeBusy={$appStore.serviceProbeBusy ?? false}
          serviceProbeError={$appStore.serviceProbeError}
          saveMessage={$appStore.settingsSaveMessage}
          saveStatus={$appStore.settingsSaveStatus ?? "idle"}
          settings={$appStore.settings}
          on:cleanGeneratedData={() => appStore.cleanAllGeneratedData()}
          on:cleanLogs={() => appStore.cleanAllLogs()}
          on:cleanWorkspaces={() => appStore.cleanAllWorkspaces()}
          on:clearCleanupSummary={() => appStore.clearCleanupSummary()}
          on:clearServiceProbeError={() => appStore.clearServiceProbeError()}
          on:download={() => appStore.downloadLatestRuntime()}
          on:edited={() => appStore.markSettingsEdited()}
          on:probeServices={() => appStore.probeServices()}
          on:redetectMcpPaths={() => appStore.redetectMcpClientPaths()}
          on:refresh={() => appStore.load()}
          on:save={handleSettingsSave}
        />
      {:else}
        <section class="panel stack">
          <h2>Settings</h2>
          <p class="muted">Loading settings...</p>
        </section>
      {/if}
    </section>
  {:else if currentView === 'help'}
    <section class="dashboard-main">
      <HelpView />
    </section>
  {/if}
</main>

{#if globalContextMenu}
  <ContextMenu
    items={globalContextMenu.items}
    onClose={() => (globalContextMenu = null)}
    x={globalContextMenu.x}
    y={globalContextMenu.y}
  />
{/if}
