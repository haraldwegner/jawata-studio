<script lang="ts">
  // Sprint 21a (item F): the Knowledge / Database view.
  //
  // VOCABULARY PRINCIPLE (Harald, 2026-07-05): every action here carries EXACTLY the
  // experience(kind=…) verb name — load / reseed / wipe / refresh / list / promote /
  // export / import / prune / dedup / compact — no UI synonyms. What you click is what
  // you'd say in a prompt; each action shows its prompt phrase as a hint.
  import { createEventDispatcher, onMount } from "svelte";
  import {
    backupsGc,
    experienceVerb,
    knowledgeStatus,
    updateSettings,
    type GcReport,
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
  let busyVerb = "";
  let outputTitle = "";
  let output = "";

  type ListRow = {
    id: string;
    type?: string;
    status?: string;
    language?: string;
    symbol?: string;
    summary?: string;
  };
  let listRows: ListRow[] = [];
  let listStatusFilter = "candidate";
  let exportPath = "";
  let importPath = "";
  let gcReport: GcReport | null = null;
  let gcBusy = false;

  // --- knowledge settings mirrors (saved via the normal settings round-trip) ---
  let storeMode = settings.experienceStoreMode ?? "shared";
  let memoryRootsText = (settings.memoryRoots ?? []).join("\n");
  let memoryRecursive = settings.memoryRecursive ?? false;
  let memoryMaxDepth = settings.memoryMaxDepth ?? 5;
  let memoryMaxFiles = settings.memoryMaxFiles ?? 200;
  let memoryMaxBytes = settings.memoryMaxBytes ?? 2_000_000;
  let autoSeedOnDeploy = settings.autoSeedOnDeploy ?? true;
  let backupRetention = settings.backupRetention ?? 10;
  let saveState: "idle" | "saving" | "saved" | "error" = "idle";
  let saveError = "";

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
    if (!selected || busyVerb) return;
    if (confirmText && !window.confirm(confirmText)) return;
    busyVerb = kind;
    outputTitle = kind;
    output = "…";
    try {
      const response = await experienceVerb(selected, kind, args);
      const payload = response.success ? response.data : response;
      output = JSON.stringify(payload, null, 2);
      if (kind === "list" && response.success) {
        listRows = ((response.data as { entries?: ListRow[] })?.entries ?? []) as ListRow[];
      }
      if (["load", "reseed", "wipe", "import", "prune", "dedup", "compact", "promote"].includes(kind)) {
        await refreshStatus();
      }
    } catch (error) {
      output = String(error);
    } finally {
      busyVerb = "";
    }
  }

  async function promoteRow(id: string, status: string) {
    await runVerb("promote", { id, status });
    await runVerb("list", { status: listStatusFilter, limit: 50 });
  }

  async function runGc(dryRun: boolean) {
    gcBusy = true;
    try {
      gcReport = await backupsGc(dryRun);
    } catch (error) {
      outputTitle = "backup GC";
      output = String(error);
    } finally {
      gcBusy = false;
    }
  }

  async function saveKnowledgeSettings() {
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
          .filter((root) => root.length > 0),
        memoryRecursive,
        memoryMaxDepth,
        memoryMaxFiles,
        memoryMaxBytes,
        backupRetention
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

  function statusCount(status: KnowledgeWorkspaceStatus, key: string): number {
    return status.stats?.by_status?.[key] ?? 0;
  }
</script>

<section class="panel stack knowledge">
  <h2>Knowledge / Database</h2>
  <p class="muted">
    The experience store behind the GOJA push channel. Every action below is the exact
    verb you can also use in a prompt — the UI and the conversation share one vocabulary.
  </p>

  <!-- F.1: storage overview -->
  <section class="sub-panel">
    <div class="row-head">
      <h3>Store</h3>
      <button type="button" on:click={refreshStatus} disabled={statusLoading || disabled}>
        {statusLoading ? "Loading…" : "Reload status"}
      </button>
    </div>
    {#if statuses.length === 0}
      <p class="muted">No workspaces (or residents unreachable).</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th></th>
            <th>Workspace</th>
            <th>Entries</th>
            <th>Candidates</th>
            <th>Accepted</th>
            <th>Store file</th>
            <th>Size</th>
          </tr>
        </thead>
        <tbody>
          {#each statuses as status}
            <tr class:unreachable={!status.reachable}>
              <td>
                <input
                  type="radio"
                  name="knowledge-workspace"
                  value={status.workspace}
                  bind:group={selected}
                />
              </td>
              <td>{status.workspace}{status.reachable ? "" : " (unreachable)"}</td>
              <td>{status.stats?.total ?? "–"}</td>
              <td>{statusCount(status, "candidate")}</td>
              <td>{statusCount(status, "accepted")}</td>
              <td class="path">{status.stats?.store?.file ?? status.error ?? "–"}</td>
              <td>{formatBytes(status.stats?.store?.bytes)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <!-- F.2 + F.5: the verb actions (the prompt vocabulary, 1:1) -->
  <section class="sub-panel">
    <h3>Actions <span class="muted">on workspace “{selected || "—"}”</span></h3>
    <div class="verbs">
      <div class="verb">
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("load", { recursive: memoryRecursive })}>load</button>
        <span class="hint">say: “load my memory files” — seed from the configured roots</span>
      </div>
      <div class="verb">
        <button type="button" class="danger" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("reseed", { confirm: true, recursive: memoryRecursive },
            "reseed WIPES the whole store, then reloads from the default roots. Continue?")}>reseed</button>
        <span class="hint">say: “reseed the store” — the explicit initial load (wipe + load)</span>
      </div>
      <div class="verb">
        <button type="button" class="danger" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("wipe", {}, "wipe removes EVERY entry from this store. Continue?")}>wipe</button>
        <span class="hint">say: “wipe the store”</span>
      </div>
      <div class="verb">
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("refresh")}>refresh</button>
        <span class="hint">say: “refresh the store” — re-resolve Java pointers, flag stale</span>
      </div>
      <div class="verb">
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("prune", { days: 30 })}>prune</button>
        <span class="hint">say: “prune the store” — drop aged rejected/superseded (30 d)</span>
      </div>
      <div class="verb">
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("dedup")}>dedup</button>
        <span class="hint">say: “dedup the store” — reports groups; confirm merges</span>
      </div>
      <div class="verb">
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("dedup", { confirm: true },
            "dedup with confirm MERGES duplicate groups (best survives, rest superseded). Continue?")}>dedup + merge</button>
        <span class="hint">say: “dedup the store and merge”</span>
      </div>
      <div class="verb">
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("compact", {},
            "compact briefly closes the store (attached residents reconnect). Continue?")}>compact</button>
        <span class="hint">say: “compact the store” — reclaim file space</span>
      </div>
      <div class="verb wide">
        <input type="text" placeholder="export file path (empty = show inline)" bind:value={exportPath} />
        <button type="button" disabled={!!busyVerb || disabled}
          on:click={() => runVerb("export", exportPath.trim() ? { path: exportPath.trim() } : {})}>export</button>
        <span class="hint">say: “export the store to a file”</span>
      </div>
      <div class="verb wide">
        <input type="text" placeholder="import file path" bind:value={importPath} />
        <button type="button" disabled={!!busyVerb || disabled || !importPath.trim()}
          on:click={() => runVerb("import", { path: importPath.trim() })}>import</button>
        <span class="hint">say: “import the export file”</span>
      </div>
    </div>
    {#if busyVerb}
      <p class="muted">running “{busyVerb}”…</p>
    {/if}
  </section>

  <!-- F.5: curation — list + promote/reject -->
  <section class="sub-panel">
    <div class="row-head">
      <h3>Curation</h3>
      <label>
        status
        <select bind:value={listStatusFilter}>
          <option value="candidate">candidate</option>
          <option value="accepted">accepted</option>
          <option value="rejected">rejected</option>
          <option value="superseded">superseded</option>
          <option value="">(all)</option>
        </select>
      </label>
      <button type="button" disabled={!!busyVerb || disabled}
        on:click={() => runVerb("list", listStatusFilter ? { status: listStatusFilter, limit: 50 } : { limit: 50 })}>
        list
      </button>
      <span class="hint">say: “list the {listStatusFilter || "stored"} entries”</span>
    </div>
    {#if listRows.length > 0}
      <table>
        <thead>
          <tr><th>Type</th><th>Status</th><th>Lang</th><th>Summary</th><th>Symbol</th><th>promote</th></tr>
        </thead>
        <tbody>
          {#each listRows as row (row.id)}
            <tr>
              <td>{row.type ?? "–"}</td>
              <td>{row.status ?? "–"}</td>
              <td>{row.language ?? "java"}</td>
              <td class="summary">{row.summary ?? ""}</td>
              <td class="path">{row.symbol ?? ""}</td>
              <td class="row-actions">
                <button type="button" disabled={!!busyVerb} title='say: "promote this entry"'
                  on:click={() => promoteRow(row.id, "accepted")}>promote</button>
                <button type="button" class="danger" disabled={!!busyVerb} title='say: "reject this entry"'
                  on:click={() => promoteRow(row.id, "rejected")}>reject</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted">Run “list” to browse entries for curation.</p>
    {/if}
  </section>

  <!-- F.3 + F.4: memory roots + crawl config + store mode -->
  <section class="sub-panel">
    <h3>Memory sources &amp; store location</h3>
    <div class="settings-grid">
      <label>
        store mode
        <select bind:value={storeMode}>
          <option value="shared">shared — one user-level store (default)</option>
          <option value="workspace">workspace — per-workspace store</option>
          <option value="memory">memory — non-persistent</option>
        </select>
      </label>
      <label class="stacked">
        memory root folders/files (one per line; passed to load/reseed)
        <textarea rows="3" bind:value={memoryRootsText}
          placeholder={"e.g. /home/you/.claude/projects/…/memory\n(empty = the resident's layered CLAUDE.md defaults)"}></textarea>
      </label>
      <label>
        <input type="checkbox" bind:checked={memoryRecursive} />
        recursive — also walk subdirectories (link-following is always on)
      </label>
      <div class="caps">
        <label>max depth <input type="number" min="1" bind:value={memoryMaxDepth} /></label>
        <label>max files <input type="number" min="1" bind:value={memoryMaxFiles} /></label>
        <label>max bytes <input type="number" min="1024" step="1024" bind:value={memoryMaxBytes} /></label>
      </div>
      <label>
        <input type="checkbox" bind:checked={autoSeedOnDeploy} />
        auto-seed on deploy — run “load” on every resident after a successful deploy
      </label>
      <p class="muted">
        Store mode, roots and caps reach the resident at its next start (they ride the
        JVM launch); load/reseed above act immediately.
      </p>
    </div>
  </section>

  <!-- F.5: backups -->
  <section class="sub-panel">
    <h3>Backups</h3>
    <div class="settings-grid">
      <label>
        retention (versions kept per file)
        <input type="number" min="1" max="500" bind:value={backupRetention} />
      </label>
      <p class="muted">
        Managed area: <span class="path">{settings.dataRoot}/backups</span> — deploys never
        write .bak files beside your own files anymore.
      </p>
      <div class="row-head">
        <button type="button" disabled={gcBusy || disabled} on:click={() => runGc(true)}>
          GC dry-run
        </button>
        <button type="button" class="danger"
          disabled={gcBusy || disabled || !gcReport || !gcReport.dryRun || gcReport.items.length === 0}
          on:click={() => runGc(false)}>
          GC apply
        </button>
        <span class="hint">sweep old scattered .bak files into the managed area</span>
      </div>
      {#if gcReport}
        <p class="muted">
          {gcReport.dryRun ? "Plan" : "Done"}: {gcReport.items.length} recognized backup(s),
          {gcReport.moved} moved, {gcReport.unrecognizedSkipped} unrecognized left untouched
          ({gcReport.scannedDirs} dirs scanned).
        </p>
        {#if gcReport.items.length > 0}
          <table>
            <thead><tr><th>File</th><th>Action</th></tr></thead>
            <tbody>
              {#each gcReport.items as item}
                <tr><td class="path">{item.file}</td><td>{item.action}</td></tr>
              {/each}
            </tbody>
          </table>
        {/if}
      {/if}
    </div>
  </section>

  <div class="row-head">
    <button type="button" disabled={saveState === "saving" || disabled} on:click={saveKnowledgeSettings}>
      {saveState === "saving" ? "Saving…" : "Save knowledge settings"}
    </button>
    {#if saveState === "saved"}<span class="muted">Saved.</span>{/if}
    {#if saveState === "error"}<span class="error-text">{saveError}</span>{/if}
  </div>

  {#if output}
    <section class="sub-panel">
      <h3>Result of “{outputTitle}”</h3>
      <pre>{output}</pre>
    </section>
  {/if}
</section>

<style>
  .knowledge {
    gap: 1rem;
  }
  .sub-panel {
    border: 1px solid var(--border-color, rgba(127, 127, 127, 0.3));
    border-radius: 8px;
    padding: 0.75rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .row-head {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.25rem 0.5rem;
    border-bottom: 1px solid rgba(127, 127, 127, 0.2);
  }
  tr.unreachable {
    opacity: 0.55;
  }
  .path,
  .summary {
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    word-break: break-all;
  }
  .verbs {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 0.5rem 1rem;
  }
  .verb {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .verb.wide {
    grid-column: 1 / -1;
  }
  .verb.wide input[type="text"] {
    flex: 1;
    min-width: 200px;
  }
  .hint {
    font-size: 0.75rem;
    opacity: 0.65;
    font-style: italic;
  }
  .settings-grid {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .settings-grid label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .settings-grid label.stacked {
    flex-direction: column;
    align-items: stretch;
  }
  .caps {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .caps input {
    width: 7rem;
  }
  button.danger {
    color: #c0392b;
  }
  .error-text {
    color: #c0392b;
  }
  pre {
    max-height: 300px;
    overflow: auto;
    font-size: 0.75rem;
    background: rgba(127, 127, 127, 0.08);
    padding: 0.5rem;
    border-radius: 6px;
  }
  .row-actions {
    display: flex;
    gap: 0.4rem;
  }
</style>
