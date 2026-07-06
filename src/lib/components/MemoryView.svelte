<script lang="ts">
  // Sprint 21b: the Memory / Database view — GOALS in the UI, verbs at the prompt.
  // Harald rejected the 21a verb-catalog pane ("overshooting... spam"): the view now has
  // exactly two panels + a result pane, five actions (load / export / import / clean up /
  // wipe), no curation, no backups, no crawl knobs. Single-verb buttons keep the verb
  // name; every control carries a mouseover with its prompt phrase where one exists.
  //
  // LAYOUT (Harald, 2026-07-06): mirrors the Settings page — the global panel/
  // settings-grid/section-intro/field/checkbox-row/hint classes from app.css, a
  // two-column grid of panels, and the same sticky save footer.
  import { createEventDispatcher, onMount } from "svelte";
  import {
    experienceVerb,
    knowledgeStatus,
    updateSettings,
    type KnowledgeWorkspaceStatus,
    type ManagerSettings,
    type UpdateSettingsInput
  } from "../api/tauri";

  export let settings: ManagerSettings;
  export let disabled = false;

  const dispatch = createEventDispatcher<{ refresh: void }>();

  let statuses: KnowledgeWorkspaceStatus[] = [];
  let statusLoading = false;
  let selected = "";
  let busyAction = "";
  let outputTitle = "";
  let output = "";

  let exportPath = "";
  let importPath = "";

  // --- memory settings mirrors (saved via the normal settings round-trip) ---
  let storeMode = settings.experienceStoreMode ?? "shared";
  let memoryRootsText = (settings.memoryRoots ?? []).join("\n");
  let autoSeedOnDeploy = settings.autoSeedOnDeploy ?? true;
  let saveState: "idle" | "saving" | "saved" | "error" = "idle";
  let saveError = "";

  $: interactionDisabled = disabled || saveState === "saving";
  $: isDirty =
    storeMode !== (settings.experienceStoreMode ?? "shared") ||
    memoryRootsText !== (settings.memoryRoots ?? []).join("\n") ||
    autoSeedOnDeploy !== (settings.autoSeedOnDeploy ?? true);
  $: footerStatusText =
    saveState === "saving"
      ? "Saving…"
      : saveState === "saved"
        ? "Memory settings saved."
        : saveState === "error"
          ? saveError
          : isDirty
            ? "Unsaved memory settings."
            : "";

  onMount(() => {
    void refreshStatus();
  });

  async function refreshStatus() {
    statusLoading = true;
    try {
      statuses = await knowledgeStatus();
      if (!selected && statuses.length > 0) {
        selected = statuses[0].workspace;
      }
    } catch (error) {
      outputTitle = "status";
      output = String(error);
    } finally {
      statusLoading = false;
    }
  }

  async function runVerb(
    kind: string,
    args: Record<string, unknown> = {},
    confirmText?: string
  ) {
    if (!selected || busyAction) return;
    if (confirmText && !window.confirm(confirmText)) return;
    busyAction = kind;
    outputTitle = kind;
    output = "…";
    try {
      const response = await experienceVerb(selected, kind, args);
      const payload = response.success ? response.data : response;
      output = JSON.stringify(payload, null, 2);
      if (["load", "wipe", "import"].includes(kind)) {
        await refreshStatus();
      }
    } catch (error) {
      output = String(error);
    } finally {
      busyAction = "";
    }
  }

  /** Sprint 21b: ONE hygiene action — prune, dedup-merge, compact, in that order.
   * (prune = drop aged rejected/superseded rows · dedup = merge duplicate groups, best
   * survives, rest superseded — recoverable until pruned · compact = shrink the file.) */
  async function runCleanUp() {
    if (!selected || busyAction) return;
    const confirmText =
      "clean up runs three steps on the selected store:\n" +
      "• prune — drop rejected/superseded entries older than 30 days\n" +
      "• dedup + merge — duplicate groups merged (best survives, rest superseded)\n" +
      "• compact — reclaim file space (attached residents reconnect)\n\nContinue?";
    if (!window.confirm(confirmText)) return;
    busyAction = "clean up";
    outputTitle = "clean up";
    output = "…";
    const report: Record<string, unknown> = {};
    try {
      for (const [step, args] of [
        ["prune", { days: 30 }],
        ["dedup", { confirm: true }],
        ["compact", {}]
      ] as const) {
        const response = await experienceVerb(selected, step, args);
        report[step] = response.success ? response.data : response;
        output = JSON.stringify(report, null, 2);
      }
      await refreshStatus();
    } catch (error) {
      report["error"] = String(error);
      output = JSON.stringify(report, null, 2);
    } finally {
      busyAction = "";
    }
  }

  async function saveMemorySettings() {
    saveState = "saving";
    saveError = "";
    try {
      const input: UpdateSettingsInput = {
        updatePolicy: settings.updatePolicy,
        autoCheckForUpdates: settings.autoCheckForUpdates,
        dataRoot: settings.dataRoot,
        globalRuntimeSource: settings.globalRuntimeSource,
        useSystemTray: settings.useSystemTray,
        autostartOnBoot: settings.autostartOnBoot,
        mcpClientPaths: settings.mcpClientPaths,
        mcpMergeMode: settings.mcpMergeMode,
        mcpBackupBeforeWrite: settings.mcpBackupBeforeWrite,
        deployTargets: settings.deployTargets,
        releaseRepo: settings.releaseRepo,
        autoSeedOnDeploy,
        experienceStoreMode: storeMode.trim() || "shared",
        memoryRoots: memoryRootsText
          .split("\n")
          .map((root) => root.trim())
          .filter((root) => root.length > 0)
      };
      await updateSettings(input);
      saveState = "saved";
      dispatch("refresh");
    } catch (error) {
      saveState = "error";
      saveError = String(error);
    }
  }

  function formatBytes(bytes?: number): string {
    if (bytes === undefined || bytes === null || bytes < 0) return "–";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<!-- runtime-settings-root = the app-wide "scrollable middle + sticky footer" scroll
     container (the same mechanism Settings and Dashboard use). -->
<section class="panel stack runtime-settings-root memory-root">
  <div>
    <h2>Memory / Database</h2>
    <p class="muted">
      Your memory store behind the GOJA push channel. Everything else — refresh, curation,
      backups — runs automatically or by prompt.
    </p>
  </div>

  <div class="settings-grid">
    <!-- Store & Maintenance: ONE panel — actions apply to the selected workspace. -->
    <section class="panel stack settings-section memory-wide">
      <div class="section-intro">
        <h3>Store &amp; Maintenance</h3>
        <p class="muted">
          One user-level store by default — select the workspace to act on.
        </p>
      </div>
      {#if statuses.length === 0}
        <p class="hint">No workspaces (or residents unreachable).</p>
      {:else}
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th></th>
                <th>Workspace</th>
                <th>Entries</th>
                <th>Store file</th>
                <th>Size</th>
              </tr>
            </thead>
            <tbody>
              {#each statuses as status}
                <tr
                  class:unreachable={!status.reachable}
                  title={status.reachable
                    ? `Act on the store as seen by the “${status.workspace}” resident`
                    : (status.error ?? "Resident unreachable")}
                >
                  <td>
                    <input
                      type="radio"
                      name="memory-workspace"
                      value={status.workspace}
                      bind:group={selected}
                      title="Select this workspace for the actions below"
                    />
                  </td>
                  <td>{status.workspace}{status.reachable ? "" : " (unreachable)"}</td>
                  <td>{status.stats?.total ?? "–"}</td>
                  <td class="mono" title={status.stats?.store?.file ?? undefined}>
                    {status.stats?.store?.file ?? status.error ?? "–"}
                  </td>
                  <td>{formatBytes(status.stats?.store?.bytes)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      <div class="actions">
        <button
          type="button"
          disabled={statusLoading || interactionDisabled}
          on:click={refreshStatus}
          title="Re-read entry counts, store file and size from every resident"
        >
          {statusLoading ? "Loading…" : "Reload status"}
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selected}
          on:click={() => runVerb("load", {})}
          title={'Seed the store from your memory files — the layered CLAUDE.md set, Claude memory folders and the roots configured below. Idempotent: re-loading replaces, so this is also the re-initialize after a wipe. Say: "load my memory files"'}
        >
          load
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selected}
          on:click={runCleanUp}
          title="One hygiene pass: prune aged rejected/superseded entries + merge duplicate groups + compact the store file. Runs prune, dedup and compact — each also available by prompt."
        >
          clean up
        </button>
        <button
          type="button"
          class="danger"
          disabled={!!busyAction || interactionDisabled || !selected}
          on:click={() =>
            runVerb("wipe", {}, "wipe removes EVERY entry from this store. Continue?")}
          title={'Delete everything in the selected store. Re-initialize afterwards with load. Say: "wipe the store"'}
        >
          wipe
        </button>
        {#if busyAction}
          <span class="hint">running “{busyAction}”…</span>
        {/if}
      </div>
      <label class="field">
        <span>export</span>
        <div class="field-row">
          <input
            type="text"
            placeholder="file path (empty = show inline)"
            bind:value={exportPath}
            title="Absolute path for the export file; leave empty to show the JSON in the result pane"
          />
          <button
            type="button"
            disabled={!!busyAction || interactionDisabled || !selected}
            on:click={() =>
              runVerb("export", exportPath.trim() ? { path: exportPath.trim() } : {})}
            title={'Write the whole store to portable JSON. Say: "export the store to a file"'}
          >
            export
          </button>
        </div>
      </label>
      <label class="field">
        <span>import</span>
        <div class="field-row">
          <input
            type="text"
            placeholder="export file path"
            bind:value={importPath}
            title="Path of a previously exported JSON file"
          />
          <button
            type="button"
            disabled={!!busyAction || interactionDisabled || !selected || !importPath.trim()}
            on:click={() => runVerb("import", { path: importPath.trim() })}
            title={'Re-ingest an export file (deduplicated by id, provenance preserved). Say: "import the export file"'}
          >
            import
          </button>
        </div>
      </label>
    </section>

    <!-- Memory sources: the genuinely user-specific settings. -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Memory sources</h3>
        <p class="muted">
          Feeds load and auto-seed. Store mode and roots reach residents at their next
          start; the actions above act immediately.
        </p>
      </div>
      <label class="field">
        <span>Store mode</span>
        <select
          bind:value={storeMode}
          disabled={interactionDisabled}
          title="Where the experience store lives — shared is one store for all your workspaces"
        >
          <option value="shared">shared — one user-level store (default)</option>
          <option value="workspace">workspace — per-workspace store</option>
          <option value="memory">memory — non-persistent</option>
        </select>
        <span class="hint">“shared” makes your knowledge recallable from every workspace.</span>
      </label>
      <label class="field">
        <span>Memory root folders/files (one per line)</span>
        <textarea
          rows="3"
          bind:value={memoryRootsText}
          disabled={interactionDisabled}
          placeholder="empty = the resident's layered CLAUDE.md defaults"
          title="Extra folders or files load crawls, on top of the layered CLAUDE.md set and Claude memory folders. Everything reachable is ingested — subfolders and [[links]] included."
        ></textarea>
        <span class="hint">e.g. /home/you/.claude/projects/&lt;project&gt;/memory</span>
      </label>
      <label
        class="checkbox-row"
        title="After every successful deploy, run load on each resident so the push channel has content from day one"
      >
        <input type="checkbox" bind:checked={autoSeedOnDeploy} disabled={interactionDisabled} />
        <span>Auto-seed on deploy</span>
      </label>
    </section>

    <!-- Result: the output home for the actions — pairs with Memory sources. -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Result{outputTitle ? ` of “${outputTitle}”` : ""}</h3>
      </div>
      {#if output}
        <pre title="Raw response of the last action">{output}</pre>
      {:else}
        <p class="hint">No action run yet — results appear here.</p>
      {/if}
    </section>
  </div>
</section>

<div class="panel settings-save-footer">
  <div class="settings-save-status-wrap">
    {#if footerStatusText}
      <span class={`settings-save-status ${saveState}`}>{footerStatusText}</span>
    {/if}
  </div>
  <button
    class:primary={isDirty && !interactionDisabled}
    class="save-settings-button"
    disabled={interactionDisabled || !isDirty}
    on:click={saveMemorySettings}
    title="Persist the memory settings to disk"
    type="button"
  >
    Save settings
  </button>
</div>

<style>
  /* Only what app.css does not already provide: tables + the wide panel + the pre. */
  .memory-wide {
    grid-column: 1 / -1;
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.3rem 0.55rem;
    border-bottom: 1px solid rgba(148, 163, 184, 0.18);
    white-space: nowrap;
  }
  td.mono {
    white-space: normal;
  }
  tr.unreachable {
    opacity: 0.55;
  }
  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    word-break: break-all;
  }
  button.danger {
    color: #f87171;
  }
  pre {
    max-height: 300px;
    overflow: auto;
    font-size: 0.75rem;
    background: rgba(148, 163, 184, 0.08);
    padding: 0.5rem;
    border-radius: 8px;
  }
</style>
