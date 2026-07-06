<script lang="ts">
  // Sprint 21a (item F): the Memory / Database view (Harald 2026-07-06: "memory", not
  // "knowledge", in every user-visible label).
  //
  // VOCABULARY PRINCIPLE (Harald, 2026-07-05): every action carries EXACTLY the
  // experience(kind=…) verb name — load / reseed / wipe / refresh / list / promote /
  // export / import / prune / dedup / compact — no UI synonyms. Each shows its prompt
  // phrase, so the UI and the conversation share one vocabulary.
  //
  // LAYOUT (Harald, 2026-07-06): mirrors the Settings page — the global panel/
  // settings-grid/section-intro/field/checkbox-row/hint classes from app.css, a
  // two-column grid of panels, and the same sticky save footer.
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

  $: interactionDisabled = disabled || saveState === "saving";
  $: isDirty =
    storeMode !== (settings.experienceStoreMode ?? "shared") ||
    memoryRootsText !== (settings.memoryRoots ?? []).join("\n") ||
    memoryRecursive !== (settings.memoryRecursive ?? false) ||
    memoryMaxDepth !== (settings.memoryMaxDepth ?? 5) ||
    memoryMaxFiles !== (settings.memoryMaxFiles ?? 200) ||
    memoryMaxBytes !== (settings.memoryMaxBytes ?? 2_000_000) ||
    autoSeedOnDeploy !== (settings.autoSeedOnDeploy ?? true) ||
    backupRetention !== (settings.backupRetention ?? 10);
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

  /** The verb rows, data-driven so the panel stays a tidy uniform list. */
  type Verb = {
    kind: string;
    label?: string;
    danger?: boolean;
    args?: () => Record<string, unknown>;
    confirm?: string;
    hint: string;
  };
  const maintenanceVerbs: Verb[] = [
    {
      kind: "load",
      args: () => ({ recursive: memoryRecursive }),
      hint: "say: “load my memory files” — seed from the configured roots"
    },
    {
      kind: "refresh",
      hint: "say: “refresh the store” — re-resolve Java pointers, flag stale"
    },
    {
      kind: "prune",
      args: () => ({ days: 30 }),
      hint: "say: “prune the store” — drop aged rejected/superseded (30 d)"
    },
    {
      kind: "dedup",
      hint: "say: “dedup the store” — reports duplicate groups"
    },
    {
      kind: "dedup",
      label: "dedup + merge",
      args: () => ({ confirm: true }),
      confirm:
        "dedup with confirm MERGES duplicate groups (best survives, rest superseded). Continue?",
      hint: "say: “dedup the store and merge”"
    },
    {
      kind: "compact",
      confirm: "compact briefly closes the store (attached residents reconnect). Continue?",
      hint: "say: “compact the store” — reclaim file space"
    },
    {
      kind: "reseed",
      danger: true,
      args: () => ({ confirm: true, recursive: memoryRecursive }),
      confirm:
        "reseed WIPES the whole store, then reloads from the default roots. Continue?",
      hint: "say: “reseed the store” — the explicit initial load (wipe + load)"
    },
    {
      kind: "wipe",
      danger: true,
      confirm: "wipe removes EVERY entry from this store. Continue?",
      hint: "say: “wipe the store”"
    }
  ];
</script>

<!-- runtime-settings-root = the app-wide "scrollable middle + sticky footer" scroll
     container (the same mechanism Settings and Dashboard use). -->
<section class="panel stack runtime-settings-root memory-root">
  <div>
    <h2>Memory / Database</h2>
    <p class="muted">
      Your memory store behind the GOJA push channel. Every action is the exact verb you
      can also use in a prompt — the UI and the conversation share one vocabulary.
    </p>
  </div>

  <div class="settings-grid">
    <!-- F.1: store overview — full width, it is a table -->
    <section class="panel stack settings-section knowledge-wide">
      <div class="section-intro">
        <h3>Store</h3>
        <p class="muted">One user-level store by default — select the workspace to act on.</p>
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
                  <td class="mono">{status.stats?.store?.file ?? status.error ?? "–"}</td>
                  <td>{formatBytes(status.stats?.store?.bytes)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      <div class="actions">
        <button type="button" on:click={refreshStatus} disabled={statusLoading || interactionDisabled}>
          {statusLoading ? "Loading…" : "Reload status"}
        </button>
        {#if busyVerb}
          <span class="hint">running “{busyVerb}”…</span>
        {/if}
      </div>
    </section>

    <!-- F.2 + F.5: maintenance verbs -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Maintenance</h3>
        <p class="muted">On workspace “{selected || "—"}”. Destructive verbs always confirm.</p>
      </div>
      <div class="verb-list">
        {#each maintenanceVerbs as verb}
          <div class="verb-row">
            <button
              type="button"
              class:danger={verb.danger}
              disabled={!!busyVerb || interactionDisabled}
              on:click={() => runVerb(verb.kind, verb.args ? verb.args() : {}, verb.confirm)}
            >
              {verb.label ?? verb.kind}
            </button>
            <span class="hint">{verb.hint}</span>
          </div>
        {/each}
      </div>
      <label class="field">
        <span>export — <span class="hint">say: “export the store to a file”</span></span>
        <div class="field-row">
          <input type="text" placeholder="file path (empty = show inline)" bind:value={exportPath} />
          <button type="button" disabled={!!busyVerb || interactionDisabled}
            on:click={() => runVerb("export", exportPath.trim() ? { path: exportPath.trim() } : {})}>
            export
          </button>
        </div>
      </label>
      <label class="field">
        <span>import — <span class="hint">say: “import the export file”</span></span>
        <div class="field-row">
          <input type="text" placeholder="export file path" bind:value={importPath} />
          <button type="button" disabled={!!busyVerb || interactionDisabled || !importPath.trim()}
            on:click={() => runVerb("import", { path: importPath.trim() })}>
            import
          </button>
        </div>
      </label>
    </section>

    <!-- F.5: curation -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Curation</h3>
        <p class="muted">
          Recall is terminal-single by design — curation is where you SEE the set and promote it.
        </p>
      </div>
      <label class="field">
        <span>status filter</span>
        <div class="field-row">
          <select bind:value={listStatusFilter}>
            <option value="candidate">candidate</option>
            <option value="accepted">accepted</option>
            <option value="rejected">rejected</option>
            <option value="superseded">superseded</option>
            <option value="">(all)</option>
          </select>
          <button type="button" disabled={!!busyVerb || interactionDisabled}
            on:click={() => runVerb("list", listStatusFilter ? { status: listStatusFilter, limit: 50 } : { limit: 50 })}>
            list
          </button>
        </div>
        <span class="hint">say: “list the {listStatusFilter || "stored"} entries”</span>
      </label>
      {#if listRows.length > 0}
        <div class="table-wrap">
          <table>
            <thead>
              <tr><th>Type</th><th>Status</th><th>Lang</th><th>Summary</th><th></th></tr>
            </thead>
            <tbody>
              {#each listRows as row (row.id)}
                <tr>
                  <td>{row.type ?? "–"}</td>
                  <td>{row.status ?? "–"}</td>
                  <td>{row.language ?? "java"}</td>
                  <td class="mono">{row.summary ?? ""}</td>
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
        </div>
      {:else}
        <p class="hint">Run “list” to browse entries for curation.</p>
      {/if}
    </section>

    <!-- F.3 + F.4: memory sources + store location -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Memory sources &amp; store location</h3>
        <p class="muted">
          Feeds load/reseed and auto-seed. Store mode, roots and caps reach residents at their
          next start; the verbs act immediately.
        </p>
      </div>
      <label class="field">
        <span>Store mode</span>
        <select bind:value={storeMode} disabled={interactionDisabled}>
          <option value="shared">shared — one user-level store (default)</option>
          <option value="workspace">workspace — per-workspace store</option>
          <option value="memory">memory — non-persistent</option>
        </select>
        <span class="hint">“shared” makes your knowledge recallable from every workspace.</span>
      </label>
      <label class="field">
        <span>Memory root folders/files (one per line)</span>
        <textarea rows="3" bind:value={memoryRootsText} disabled={interactionDisabled}
          placeholder="empty = the resident's layered CLAUDE.md defaults"></textarea>
        <span class="hint">e.g. /home/you/.claude/projects/&lt;project&gt;/memory</span>
      </label>
      <label class="checkbox-row" title="Directory roots are also walked recursively; the [[link]] graph is always followed">
        <input type="checkbox" bind:checked={memoryRecursive} disabled={interactionDisabled} />
        <span>Recursive — also walk subdirectories (link-following is always on)</span>
      </label>
      <label class="checkbox-row" title="After a successful deploy, run 'load' on every resident">
        <input type="checkbox" bind:checked={autoSeedOnDeploy} disabled={interactionDisabled} />
        <span>Auto-seed on deploy</span>
      </label>
      <label class="field">
        <span>Crawl caps</span>
        <div class="field-row caps-row">
          <label class="cap">
            <span class="hint">max depth</span>
            <input type="number" min="1" bind:value={memoryMaxDepth} disabled={interactionDisabled} />
          </label>
          <label class="cap">
            <span class="hint">max files</span>
            <input type="number" min="1" bind:value={memoryMaxFiles} disabled={interactionDisabled} />
          </label>
          <label class="cap">
            <span class="hint">max bytes</span>
            <input type="number" min="1024" step="1024" bind:value={memoryMaxBytes} disabled={interactionDisabled} />
          </label>
        </div>
        <span class="hint">Bounds for the link crawl; skipped sources are always reported.</span>
      </label>
    </section>

    <!-- F.5: backups -->
    <section class="panel stack settings-section knowledge-wide">
      <div class="section-intro">
        <h3>Backups</h3>
        <p class="muted">
          Deploys back up into <span class="mono">{settings.dataRoot}/backups</span> — never
          beside your files. The GC sweeps historically scattered .bak files in.
        </p>
      </div>
      <div class="backup-controls">
        <label class="field retention-field">
          <span>Retention (versions kept per file)</span>
          <input type="number" min="1" max="500" bind:value={backupRetention} disabled={interactionDisabled} />
        </label>
        <div class="actions">
          <button type="button" disabled={gcBusy || interactionDisabled} on:click={() => runGc(true)}>
            GC dry-run
          </button>
          <button type="button" class="danger"
            disabled={gcBusy || interactionDisabled || !gcReport || !gcReport.dryRun || gcReport.items.length === 0}
            on:click={() => runGc(false)}>
            GC apply
          </button>
        </div>
      </div>
      {#if gcReport}
        <p class="hint">
          {gcReport.dryRun ? "Plan" : "Done"}: {gcReport.items.length} recognized backup(s),
          {gcReport.moved} moved, {gcReport.unrecognizedSkipped} unrecognized left untouched
          ({gcReport.scannedDirs} dirs scanned).
        </p>
        {#if gcReport.items.length > 0}
          <div class="table-wrap">
            <table>
              <thead><tr><th>File</th><th>Action</th></tr></thead>
              <tbody>
                {#each gcReport.items as item}
                  <tr><td class="mono">{item.file}</td><td>{item.action}</td></tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      {/if}
    </section>

    {#if output}
      <section class="panel stack settings-section knowledge-wide">
        <div class="section-intro">
          <h3>Result of “{outputTitle}”</h3>
        </div>
        <pre>{output}</pre>
      </section>
    {/if}
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
    on:click={saveKnowledgeSettings}
    title="Persist the memory settings to disk"
    type="button"
  >
    Save settings
  </button>
</div>

<style>
  /* Only what app.css does not already provide: tables, verb rows, wide panels. */
  .knowledge-wide {
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
  .verb-list {
    display: grid;
    gap: 0.45rem;
  }
  .verb-row {
    display: grid;
    grid-template-columns: 8.5rem 1fr;
    align-items: center;
    gap: 0.6rem;
  }
  .verb-row button {
    width: 100%;
  }
  .row-actions {
    display: flex;
    gap: 0.4rem;
  }
  .caps-row {
    display: flex;
    gap: 0.9rem;
    flex-wrap: wrap;
  }
  .cap {
    display: grid;
    gap: 0.2rem;
  }
  .cap input {
    width: 7.5rem;
  }
  .backup-controls {
    display: flex;
    align-items: end;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .retention-field input {
    width: 8rem;
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
