<script lang="ts">
  // Sprint 21b: the Memory / Database view — GOALS in the UI, verbs at the prompt.
  // Review round 2 (Harald, 2026-07-06): sources BEFORE the store panel · no "memory"
  // store mode · a shared store renders as ONE row (grouped by store file) · export/
  // import are file-explorer dialogs in the action row · results render human-readable
  // (raw JSON behind <details>) · memory roots = folder/file pickers + removable list.
  //
  // LAYOUT: mirrors the Settings page — global panel/settings-grid/section-intro/field/
  // checkbox-row/hint classes from app.css, two-column grid, sticky save footer.
  import { createEventDispatcher, onMount } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
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
  let outputSummary: string[] = [];
  let outputRaw = "";

  // --- memory settings mirrors (saved via the normal settings round-trip) ---
  let storeMode = settings.experienceStoreMode ?? "shared";
  let memoryRoots: string[] = [...(settings.memoryRoots ?? [])];
  let autoSeedOnDeploy = settings.autoSeedOnDeploy ?? true;
  let saveState: "idle" | "saving" | "saved" | "error" = "idle";
  let saveError = "";

  $: interactionDisabled = disabled || saveState === "saving";
  $: isDirty =
    storeMode !== (settings.experienceStoreMode ?? "shared") ||
    memoryRoots.join("\n") !== (settings.memoryRoots ?? []).join("\n") ||
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

  /** One row per DISTINCT store file — a shared store is ONE store, not N workspace
   * rows showing the same file. Unreachable residents keep their own row. */
  type StoreRow = {
    key: string;
    label: string;
    file?: string;
    total?: number;
    bytes?: number;
    workspaces: string[];
    reachable: boolean;
    error?: string | null;
  };
  $: storeRows = groupByStoreFile(statuses);
  $: selectedRow = storeRows.find((row) => row.key === selected);

  function groupByStoreFile(list: KnowledgeWorkspaceStatus[]): StoreRow[] {
    const rows: StoreRow[] = [];
    for (const status of list) {
      const file = status.reachable ? status.stats?.store?.file : undefined;
      const existing = file ? rows.find((row) => row.file === file) : undefined;
      if (existing) {
        existing.workspaces.push(status.workspace);
        existing.label = `shared — ${existing.workspaces.join(", ")}`;
        continue;
      }
      rows.push({
        key: status.workspace,
        label: status.workspace + (status.reachable ? "" : " (unreachable)"),
        file,
        total: status.stats?.total ?? undefined,
        bytes: status.stats?.store?.bytes,
        workspaces: [status.workspace],
        reachable: status.reachable,
        error: status.error
      });
    }
    return rows;
  }

  onMount(() => {
    void refreshStatus();
  });

  async function refreshStatus() {
    statusLoading = true;
    try {
      statuses = await knowledgeStatus();
      const rows = groupByStoreFile(statuses);
      if (rows.length > 0 && !rows.some((row) => row.key === selected)) {
        selected = (rows.find((row) => row.reachable) ?? rows[0]).key;
      }
    } catch (error) {
      showResult("status", { error: String(error) });
    } finally {
      statusLoading = false;
    }
  }

  // --- human-readable result rendering (raw JSON stays behind <details>) --------------

  function asCount(value: unknown): number | undefined {
    return typeof value === "number" ? value : undefined;
  }

  function summarize(kind: string, payload: unknown): string[] {
    if (payload === null || typeof payload !== "object") return [];
    const p = payload as Record<string, unknown>;
    const lines: string[] = [];
    if (typeof p.error === "string") lines.push(`Error: ${p.error}`);
    switch (kind) {
      case "load": {
        const loaded = asCount(p.loaded);
        const files = asCount(p.files);
        if (loaded !== undefined) {
          lines.push(
            `Loaded ${loaded}${files !== undefined ? ` of ${files}` : ""} file(s)` +
              (Array.isArray(p.linked) || asCount(p.linked) !== undefined
                ? `, ${asCount(p.linked) ?? 0} reached via links`
                : "")
          );
        }
        if (Array.isArray(p.skipped) && p.skipped.length > 0) {
          lines.push(`${p.skipped.length} source(s) skipped — see raw response`);
        }
        break;
      }
      case "wipe":
      case "prune":
        if (asCount(p.removed) !== undefined) lines.push(`Removed ${p.removed} entr(ies)`);
        break;
      case "dedup":
        if (asCount(p.group_count) !== undefined) {
          lines.push(`${p.group_count} duplicate group(s), ${asCount(p.merged) ?? 0} merged`);
        }
        break;
      case "compact":
        if (p.compacted === true) {
          const before = asCount(p.bytes_before);
          const after = asCount(p.bytes_after);
          lines.push(
            before !== undefined && after !== undefined
              ? `Store compacted: ${formatBytes(before)} → ${formatBytes(after)}`
              : "Store compacted"
          );
        } else if (p.compacted === false) {
          lines.push(`Not compacted${typeof p.reason === "string" ? ` — ${p.reason}` : ""}`);
        }
        break;
      case "export": {
        const count = asCount(p.exported) ?? asCount(p.count);
        lines.push(
          `Exported${count !== undefined ? ` ${count} entr(ies)` : ""}` +
            (typeof p.path === "string" ? ` to ${p.path}` : "")
        );
        break;
      }
      case "import": {
        const count = asCount(p.imported) ?? asCount(p.count);
        if (count !== undefined) lines.push(`Imported ${count} entr(ies)`);
        if (asCount(p.skipped) !== undefined) lines.push(`${p.skipped} duplicate(s) skipped`);
        break;
      }
    }
    const refresh = p.refresh as Record<string, unknown> | undefined;
    if (refresh && Array.isArray(refresh.staled) && refresh.staled.length > 0) {
      lines.push(`${refresh.staled.length} stale Java pointer(s) flagged automatically`);
    }
    return lines;
  }

  function showResult(kind: string, payload: unknown) {
    outputTitle = kind;
    outputSummary = summarize(kind, payload);
    outputRaw = typeof payload === "string" ? payload : JSON.stringify(payload, null, 2);
  }

  async function runVerb(kind: string, args: Record<string, unknown> = {}, confirmText?: string) {
    if (!selectedRow || busyAction) return;
    if (confirmText && !window.confirm(confirmText)) return;
    busyAction = kind;
    showResult(kind, "…");
    try {
      const response = await experienceVerb(selectedRow.workspaces[0], kind, args);
      showResult(kind, response.success ? response.data : response);
      if (["load", "wipe", "import"].includes(kind)) {
        await refreshStatus();
      }
    } catch (error) {
      showResult(kind, { error: String(error) });
    } finally {
      busyAction = "";
    }
  }

  /** Sprint 21b: ONE hygiene action — prune, dedup-merge, compact, in that order. */
  async function runCleanUp() {
    if (!selectedRow || busyAction) return;
    const confirmText =
      "clean up runs three steps on the selected store:\n" +
      "• prune — drop rejected/superseded entries older than 30 days\n" +
      "• dedup + merge — duplicate groups merged (best survives, rest superseded)\n" +
      "• compact — reclaim file space (attached residents reconnect)\n\nContinue?";
    if (!window.confirm(confirmText)) return;
    busyAction = "clean up";
    outputTitle = "clean up";
    outputSummary = [];
    outputRaw = "…";
    const report: Record<string, unknown> = {};
    const lines: string[] = [];
    try {
      for (const [step, args] of [
        ["prune", { days: 30 }],
        ["dedup", { confirm: true }],
        ["compact", {}]
      ] as const) {
        const response = await experienceVerb(selectedRow.workspaces[0], step, args);
        const payload = response.success ? response.data : response;
        report[step] = payload;
        lines.push(...summarize(step, payload));
        outputSummary = [...lines];
        outputRaw = JSON.stringify(report, null, 2);
      }
      await refreshStatus();
    } catch (error) {
      report["error"] = String(error);
      outputRaw = JSON.stringify(report, null, 2);
      outputSummary = [...lines, `Error: ${String(error)}`];
    } finally {
      busyAction = "";
    }
  }

  // --- export / import via the OS file dialogs ----------------------------------------

  async function runExport() {
    if (!selectedRow || busyAction) return;
    const path = await save({
      title: "Export memory store",
      defaultPath: "goja-memory-export.json",
      filters: [{ name: "JSON", extensions: ["json"] }]
    });
    if (!path) return;
    await runVerb("export", { path });
  }

  async function runImport() {
    if (!selectedRow || busyAction) return;
    const path = await open({
      title: "Import memory export",
      multiple: false,
      directory: false,
      filters: [{ name: "JSON", extensions: ["json"] }]
    });
    if (!path || typeof path !== "string") return;
    await runVerb("import", { path });
  }

  // --- memory roots: pickers + removable list ------------------------------------------

  async function addRootFolder() {
    const picked = await open({ title: "Add memory root folder", directory: true, multiple: true });
    if (!picked) return;
    for (const dir of Array.isArray(picked) ? picked : [picked]) {
      if (typeof dir === "string" && !memoryRoots.includes(dir)) {
        memoryRoots = [...memoryRoots, dir];
      }
    }
  }

  async function addRootFile() {
    const picked = await open({ title: "Add memory file", directory: false, multiple: true });
    if (!picked) return;
    for (const file of Array.isArray(picked) ? picked : [picked]) {
      if (typeof file === "string" && !memoryRoots.includes(file)) {
        memoryRoots = [...memoryRoots, file];
      }
    }
  }

  function removeRoot(root: string) {
    memoryRoots = memoryRoots.filter((entry) => entry !== root);
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
        memoryRoots: [...memoryRoots]
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
    <!-- Memory sources FIRST — they feed the store below. -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Memory sources</h3>
        <p class="muted">
          Where load finds your memory files. Claude, Cursor &amp; co. locations are
          auto-discovered — add extra roots only for anything beyond the conventions.
        </p>
      </div>
      <label class="field">
        <span>Extra memory roots</span>
        <div class="actions">
          <button
            type="button"
            disabled={interactionDisabled}
            on:click={addRootFolder}
            title="Pick folder(s) to crawl in addition to the auto-discovered locations"
          >
            Add folder…
          </button>
          <button
            type="button"
            disabled={interactionDisabled}
            on:click={addRootFile}
            title="Pick individual memory file(s)"
          >
            Add file…
          </button>
        </div>
        {#if memoryRoots.length > 0}
          <ul class="root-list">
            {#each memoryRoots as root (root)}
              <li title={root}>
                <span class="mono">{root}</span>
                <button
                  type="button"
                  class="danger"
                  disabled={interactionDisabled}
                  on:click={() => removeRoot(root)}
                  title="Remove this root (does not delete anything on disk)"
                >
                  ✕
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <span class="hint">
            None — load uses the auto-discovered set: layered CLAUDE.md, every
            ~/.claude/projects/*/memory, .cursor/rules, .cursorrules, AGENTS.md,
            copilot-instructions.md.
          </span>
        {/if}
      </label>
      <label class="field">
        <span>Store mode</span>
        <select
          bind:value={storeMode}
          disabled={interactionDisabled}
          title="Where the experience store lives — shared is one store for all your workspaces"
        >
          <option value="shared">shared — one user-level store (default)</option>
          <option value="workspace">workspace — per-workspace store</option>
        </select>
        <span class="hint">“shared” makes your knowledge recallable from every workspace.</span>
      </label>
      <label
        class="checkbox-row"
        title="After every successful deploy, run load on each resident so the push channel has content from day one"
      >
        <input type="checkbox" bind:checked={autoSeedOnDeploy} disabled={interactionDisabled} />
        <span>Auto-seed on deploy</span>
      </label>
    </section>

    <!-- Store & Maintenance: actions apply to the selected store. -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Store &amp; Maintenance</h3>
        <p class="muted">
          {storeRows.length === 1
            ? "One user-level store — all workspaces share it."
            : "Select the store to act on."}
        </p>
      </div>
      {#if storeRows.length === 0}
        <p class="hint">No workspaces (or residents unreachable).</p>
      {:else}
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                {#if storeRows.length > 1}<th></th>{/if}
                <th>Store</th>
                <th>Entries</th>
                <th>Size</th>
              </tr>
            </thead>
            <tbody>
              {#each storeRows as row (row.key)}
                <tr
                  class:unreachable={!row.reachable}
                  title={row.file ?? row.error ?? "Resident unreachable"}
                >
                  {#if storeRows.length > 1}
                    <td>
                      <input
                        type="radio"
                        name="memory-store"
                        value={row.key}
                        bind:group={selected}
                        title="Select this store for the actions below"
                      />
                    </td>
                  {/if}
                  <td>{row.label}</td>
                  <td>{row.total ?? "–"}</td>
                  <td>{formatBytes(row.bytes)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if selectedRow?.file}
          <p class="hint mono" title="The selected store's database file">{selectedRow.file}</p>
        {/if}
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
          disabled={!!busyAction || interactionDisabled || !selectedRow}
          on:click={() => runVerb("load", {})}
          title={'Seed the store from your memory files — auto-discovered Claude/Cursor & co. locations plus the extra roots. Idempotent: re-loading replaces, so this is also the re-initialize after a wipe. Say: "load my memory files"'}
        >
          load
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow}
          on:click={runCleanUp}
          title="One hygiene pass: prune aged rejected/superseded entries + merge duplicate groups + compact the store file. Runs prune, dedup and compact — each also available by prompt."
        >
          clean up
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow}
          on:click={runExport}
          title={'Write the whole store to a portable JSON file — opens the save dialog. Say: "export the store to a file"'}
        >
          export…
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow}
          on:click={runImport}
          title={'Re-ingest a previously exported JSON file (deduplicated by id) — opens the file picker. Say: "import the export file"'}
        >
          import…
        </button>
        <button
          type="button"
          class="danger"
          disabled={!!busyAction || interactionDisabled || !selectedRow}
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
    </section>

    <!-- Result: wide, human-readable; raw JSON behind details. -->
    <section class="panel stack settings-section memory-wide">
      <div class="section-intro">
        <h3>Result{outputTitle ? ` of “${outputTitle}”` : ""}</h3>
      </div>
      {#if outputSummary.length > 0}
        <ul class="result-lines">
          {#each outputSummary as line}
            <li>{line}</li>
          {/each}
        </ul>
      {:else if !outputRaw}
        <p class="hint">No action run yet — results appear here.</p>
      {/if}
      {#if outputRaw && outputRaw !== "…"}
        <details>
          <summary title="The unmodified response of the last action">raw response</summary>
          <pre>{outputRaw}</pre>
        </details>
      {:else if outputRaw === "…"}
        <p class="hint">running…</p>
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
  /* Only what app.css does not already provide: tables, the roots list, the pre. */
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
  tr.unreachable {
    opacity: 0.55;
  }
  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    word-break: break-all;
  }
  .root-list {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
    display: grid;
    gap: 0.35rem;
  }
  .root-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    padding: 0.35rem 0.55rem;
    border: 1px solid rgba(96, 165, 250, 0.25);
    background: rgba(59, 130, 246, 0.08);
    border-radius: 8px;
  }
  .root-list button {
    padding: 0 0.45rem;
  }
  .result-lines {
    margin: 0;
    padding-left: 1.1rem;
  }
  .result-lines li {
    padding: 0.1rem 0;
  }
  button.danger {
    color: #f87171;
  }
  details summary {
    cursor: pointer;
    opacity: 0.75;
    font-size: 0.85rem;
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
