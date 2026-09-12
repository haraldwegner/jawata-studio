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
  import { confirmDestructive } from "../dialog";
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

  /** ONE store ⇒ ONE row. In shared mode ALL workspaces are the same store — a single
   * row with the location and the workspace list (stats from any reachable resident);
   * unreachable residents don't split the store. In workspace mode: one row each. */
  type StoreRow = {
    key: string;
    file?: string;
    total?: number;
    bytes?: number;
    /** Display list — every workspace, unreachable ones marked. */
    workspaceLabels: string;
    /** Action targets — reachable residents only. */
    targets: string[];
    reachable: boolean;
    error?: string | null;
    /**
     * Sprint 28f Stage 5: the lane split, EXACTLY as the resident groups it.
     *
     * Studio keeps no list of what the lanes are. A hardcoded four here would be a second
     * copy of a vocabulary the engine owns, and the drift is the silent kind: a renamed
     * lane would go on showing under its old name with a count of zero forever. The cost
     * is stated in the hint beside it — a lane with no entries does not appear at all.
     */
    lanes?: Record<string, number>;
  };
  $: storeRows = buildStoreRows(statuses, storeMode);
  // Self-healing selection: switching store mode regroups the rows and can orphan the key.
  $: if (storeRows.length > 0 && !storeRows.some((row) => row.key === selected)) {
    selected = (storeRows.find((row) => row.reachable) ?? storeRows[0]).key;
  }
  $: selectedRow = storeRows.find((row) => row.key === selected);

  // A version list belongs to ONE store. Switching the selection must clear it rather
  // than leave the previous store's copies on screen beside a Restore button that would
  // now act on a different database — showing a list that is not of the selected store
  // is worse than showing none.
  $: if (selected) {
    backupNames = [];
    backupDepth = 0;
    backupsShown = false;
  }

  function buildStoreRows(list: KnowledgeWorkspaceStatus[], mode: string): StoreRow[] {
    if (list.length === 0) return [];
    if (mode !== "workspace") {
      const reachable = list.filter((status) => status.reachable && status.stats);
      const first = reachable[0];
      return [
        {
          key: "shared",
          file: first?.stats?.store?.file ?? undefined,
          total: first?.stats?.total ?? undefined,
          bytes: first?.stats?.store?.bytes,
          workspaceLabels: list
            .map((s) => s.workspace + (s.reachable ? "" : " (unreachable)"))
            .join(", "),
          targets: reachable.map((s) => s.workspace),
          reachable: reachable.length > 0,
          error: reachable.length === 0 ? "No resident reachable — retrying…" : null,
          lanes: laneSplit(first?.stats)
        }
      ];
    }
    return list.map((status) => ({
      key: status.workspace,
      file: status.stats?.store?.file ?? undefined,
      total: status.stats?.total ?? undefined,
      bytes: status.stats?.store?.bytes,
      workspaceLabels: status.workspace + (status.reachable ? "" : " (unreachable)"),
      targets: status.reachable ? [status.workspace] : [],
      reachable: status.reachable,
      error: status.error,
      lanes: laneSplit(status.stats)
    }));
  }

  /**
   * Sprint 28f Stage 5 — the lane split the resident reports, or undefined when it has
   * none (an older engine, or a store with no entries).
   *
   * Studio holds no list of lane NAMES — see `StoreRow.lanes`. The ONE ordering applied is
   * a rule about the KEY's shape, not about the vocabulary: a bracketed group like
   * "(none)" collects entries the engine could not classify, and it reads last. A lane
   * added or renamed upstream needs no change here.
   */
  function laneSplit(stats: KnowledgeWorkspaceStatus["stats"]): Record<string, number> | undefined {
    const raw = (stats as Record<string, unknown> | null | undefined)?.by_lane;
    if (!raw || typeof raw !== "object") return undefined;
    const source = raw as Record<string, unknown>;
    const out: Record<string, number> = {};
    // Stable sort, so the engine's own order survives inside each group.
    for (const key of Object.keys(source).sort(
      (a, b) => Number(a.startsWith("(")) - Number(b.startsWith("("))
    )) {
      const count = Number(source[key]);
      if (Number.isFinite(count)) out[key] = count;
    }
    return Object.keys(out).length > 0 ? out : undefined;
  }

  // Auto-reload while residents are unreachable: a freshly (re)started resident needs
  // ~30 s of OSGi/JDT boot before its HTTP port answers, so the view keeps polling for
  // as long as anything is unreachable AND the view is mounted — an expiring window
  // just re-creates the stale-"unreachable" trap (Harald, 2026-07-06).
  //
  // The retry BACKS OFF: the probe abandons its request at a client timeout without
  // cancelling the resident's work, so a fixed interval can pile up server-side jobs a
  // client cannot see. Backing off bounds how fast that can happen. It is hygiene, not
  // a bound on cost — that belongs on the resident, whose answer must be cheap.
  const RETRY_MIN_MS = 5000;
  const RETRY_MAX_MS = 60000;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let retryDelayMs = RETRY_MIN_MS;
  let autoRetrying = false;

  onMount(() => {
    void refreshStatus();
    return () => {
      if (retryTimer) clearTimeout(retryTimer);
    };
  });

  function scheduleAutoReload() {
    if (retryTimer) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
    autoRetrying = statuses.length === 0 || statuses.some((s) => !s.reachable);
    if (!autoRetrying) {
      retryDelayMs = RETRY_MIN_MS;   // reachable again: the next outage starts fast
      return;
    }
    const delay = retryDelayMs;
    retryDelayMs = Math.min(retryDelayMs * 2, RETRY_MAX_MS);
    retryTimer = setTimeout(() => {
      void refreshStatus();
    }, delay);
  }

  async function refreshStatus() {
    statusLoading = true;
    try {
      statuses = await knowledgeStatus();   // selection self-heals reactively
    } catch (error) {
      showResult("status", { error: String(error) });
    } finally {
      statusLoading = false;
      scheduleAutoReload();
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
        const unchanged = asCount(p.unchanged);
        if (loaded !== undefined) {
          lines.push(
            `Loaded ${loaded}${files !== undefined ? ` of ${files}` : ""} file(s)` +
              (unchanged ? `, ${unchanged} unchanged (skipped)` : "") +
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
      // Sprint 28f Stage 5 — the two curation verbs. Both answer "nothing changed" the
      // same way, with a flag rather than an error, and both are rendered so that a
      // no-op cannot read as a success: a rule that was already retired and one that has
      // just been retired are different facts about the store.
      case "retire_rule":
        if (p.retired === true) {
          lines.push("Retired — it stops being offered as guidance, and stays readable.");
        } else if (p.retired === false) {
          lines.push(
            "Nothing changed" + (typeof p.note === "string" ? ` — ${p.note}` : "") + "."
          );
        }
        break;
      case "promote":
        if (p.changed === true) {
          lines.push(`Promoted to ${p.status ?? "accepted"}.`);
        } else if (p.changed === false) {
          lines.push(
            `Nothing changed — it is already ${p.status ?? "accepted"}, or the entry is gone.`
          );
        }
        break;
      case "list": {
        const ruleCount = asCount(p.rules);
        const awaiting = asCount(p.awaitingReview);
        if (ruleCount !== undefined) {
          lines.push(`${ruleCount} rule(s), ${awaiting ?? 0} entr(ies) awaiting review`);
        }
        break;
      }
      case "backup":
        if (typeof p.backup === "string") {
          lines.push(
            `Copy taken: ${p.backup}` +
              (asCount(p.kept) !== undefined
                ? ` — ${p.kept} of ${asCount(p.depth) ?? "?"} kept`
                : "")
          );
        }
        break;
      case "restore": {
        // The same verb answers two different questions — asked with no name it
        // LISTS, asked with one it restores — so both shapes are summarized here.
        if (Array.isArray(p.backups)) {
          lines.push(
            p.backups.length === 0
              ? "No copies yet."
              : `${p.backups.length} version(s), newest first — ${asCount(p.depth) ?? "?"} kept`
          );
        }
        if (typeof p.restored === "string") {
          lines.push(`Restored from ${p.restored}`);
          if (asCount(p.rows) !== undefined) {
            lines.push(`The store now holds ${p.rows} entr(ies)`);
          }
        }
        break;
      }
    }
    const refresh = p.refresh as Record<string, unknown> | undefined;
    if (refresh && Array.isArray(refresh.staled) && refresh.staled.length > 0) {
      lines.push(`${refresh.staled.length} stale Java pointer(s) flagged automatically`);
    }
    // Sprint 28f D2: a destructive verb returns the copy it took in its OWN response,
    // and this is where a person sees it. The null branch is not decoration — "no copy
    // was taken" and "a copy was taken and nobody mentioned it" must not read alike,
    // which is the one case where the safety net is absent and it matters most.
    if (kind !== "backup") {
      if (typeof p.backup === "string") {
        lines.push(`A copy of the previous state was kept: ${p.backup}`);
      } else if (p.backup === null && typeof p.backupNote === "string") {
        lines.push(`No copy was taken — ${p.backupNote}`);
      }
    }
    return lines;
  }

  function showResult(kind: string, payload: unknown) {
    outputTitle = kind;
    outputSummary = summarize(kind, payload);
    outputRaw = typeof payload === "string" ? payload : JSON.stringify(payload, null, 2);
  }

  async function runVerb(kind: string, args: Record<string, unknown> = {}, confirmText?: string) {
    if (!selectedRow || selectedRow.targets.length === 0 || busyAction) return;
    if (confirmText && !(await confirmDestructive(confirmText))) return;
    busyAction = kind;
    showResult(kind, "…");
    try {
      const response = await experienceVerb(selectedRow.targets[0], kind, args);
      showResult(kind, response.success ? response.data : response);
      if (["load", "wipe", "import"].includes(kind)) {
        await refreshStatus();
      }
      // A destructive verb has just left a new copy, so a list already on screen is
      // stale — and a stale list is worse than none here, because the entry it is
      // missing is the newest one, which is the one worth going back to.
      if (backupsShown) await readBackups(true);
      // And the curation lists, for the same reason and a wider one: retire and promote
      // change a row that is on screen, while wipe, import, load and restore can change
      // every row. One re-read after any verb is cheaper than reasoning per verb about
      // which of them could have invalidated the list.
      if (curationShown) await readCuration(true);
    } catch (error) {
      showResult(kind, { error: String(error) });
    } finally {
      busyAction = "";
    }
  }

  /** load is per-resident-ADDITIVE: each workspace's resident discovers its own project
   * locations (project CLAUDE.md, .cursor/rules, …) into the ONE shared store — so a
   * shared-store load must run on EVERY reachable resident, not just one. All other
   * actions operate on the single database and need one resident only. */
  async function runLoad() {
    if (!selectedRow || selectedRow.targets.length === 0 || busyAction) return;
    busyAction = "load";
    outputTitle = "load";
    outputSummary = [];
    outputRaw = "…";
    const report: Record<string, unknown> = {};
    const lines: string[] = [];
    const multi = selectedRow.targets.length > 1;
    try {
      for (const workspace of selectedRow.targets) {
        const response = await experienceVerb(workspace, "load", {});
        const payload = response.success ? response.data : response;
        report[workspace] = payload;
        const sub = summarize("load", payload);
        lines.push(...(multi ? sub.map((line) => `${workspace}: ${line}`) : sub));
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

  /** Sprint 21b: ONE hygiene action — prune, dedup-merge, compact, in that order. */
  async function runCleanUp() {
    if (!selectedRow || selectedRow.targets.length === 0 || busyAction) return;
    const confirmText =
      "clean up runs three steps on the selected store:\n" +
      "• prune — drop rejected/superseded entries older than 30 days\n" +
      "• dedup + merge — duplicate groups merged (best survives, rest superseded)\n" +
      "• compact — reclaim file space (attached residents reconnect)\n\nContinue?";
    if (!(await confirmDestructive(confirmText))) return;
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
        const response = await experienceVerb(selectedRow.targets[0], step, args);
        const payload = response.success ? response.data : response;
        report[step] = payload;
        lines.push(...summarize(step, payload));
        outputSummary = [...lines];
        outputRaw = JSON.stringify(report, null, 2);
      }
      await refreshStatus();
      if (backupsShown) await readBackups(true);   // prune left a copy
      if (curationShown) await readCuration(true); // and dedup/prune moved rows
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
      defaultPath: "jawata-memory-export.json",
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

  // --- versions: take a copy now, list the copies, put one back ------------------------
  //
  // Sprint 28f D2. The destructive verbs each take their own copy server-side, so the
  // safety net exists whether or not anyone opens this view. What the view adds is the
  // half a prompt is worst at — SEEING which versions exist and choosing between them:
  // "Restore is a click in studio, by version."

  let backupNames: string[] = [];
  let backupDepth = 0;
  let backupsShown = false;

  /** Read the version list off the resident.
   *
   * The engine's restore verb LISTS when it is handed no name — deliberately, so that a
   * verb's no-argument form is never the destructive reading of its own name — so this
   * is the SAME verb the Restore buttons run, asked without a choice.
   *
   * `quiet` leaves the result panel showing whatever the caller just did: the version
   * list is a picker, not the outcome of an action. */
  async function readBackups(quiet: boolean) {
    if (!selectedRow || selectedRow.targets.length === 0) return;
    try {
      const response = await experienceVerb(selectedRow.targets[0], "restore", {});
      const payload = response.success ? response.data : response;
      const p = (payload ?? {}) as Record<string, unknown>;
      backupNames = Array.isArray(p.backups) ? p.backups.map(String) : [];
      backupDepth = asCount(p.depth) ?? 0;
      backupsShown = true;
      if (!quiet) showResult("restore", payload);
    } catch (error) {
      backupNames = [];
      backupsShown = true;
      if (!quiet) showResult("restore", { error: String(error) });
    }
  }

  async function showBackups() {
    if (!selectedRow || selectedRow.targets.length === 0 || busyAction) return;
    busyAction = "restore";
    showResult("restore", "…");
    try {
      await readBackups(false);
    } finally {
      busyAction = "";
    }
  }

  /** Take a copy NOW — before something this product does not know about. The
   * destructive verbs need no help; a hand edit, an upgrade or a machine move does. */
  async function runBackup() {
    await runVerb("backup");
    if (backupsShown) await readBackups(true);
  }

  /** Put ONE version back.
   *
   * Destructive by construction — every entry is replaced by the ones that version
   * holds — so it confirms first, and the question NAMES the version rather than asking
   * about "the backup". It also says that the current state is copied first, because a
   * user who does not know that will not dare press the button at all. */
  async function restoreBackup(name: string) {
    if (!selectedRow || selectedRow.targets.length === 0 || busyAction) return;
    const confirmed = await confirmDestructive(
      `Restore “${name}”?\n\n` +
        "Every entry in this store is replaced by the ones that version holds — anything" +
        " written since it was taken is gone from the store.\n\n" +
        "A copy of the CURRENT state is taken first, so this is itself undoable."
    );
    if (!confirmed) return;
    busyAction = "restore";
    showResult("restore", "…");
    try {
      // `confirm` is the ENGINE's gate, not a second dialog: the resident refuses a
      // named restore without it, because a confirmation the caller performs is one a
      // caller can skip. The dialog above is what earns passing it.
      const response = await experienceVerb(selectedRow.targets[0], "restore", {
        name,
        confirm: true
      });
      showResult("restore", response.success ? response.data : response);
      await refreshStatus();
      await readBackups(true);
    } catch (error) {
      showResult("restore", { error: String(error) });
    } finally {
      busyAction = "";
    }
  }

  // --- curation: two lanes, two lifecycles, as clicks ----------------------------------
  //
  // Sprint 28f Stage 5. The lane counts above say what the store HOLDS; this is where a
  // person acts on it. The two acts are deliberately not one button:
  //
  //   PROMOTE takes an entry nobody has vouched for and accepts it. It is curation.
  //   RETIRE says a standing RULE stopped applying. It is neither rejecting (the rule was
  //     not wrong) nor superseding (nothing replaced it), which is exactly why the engine
  //     gives it its own verb and a DATE rather than a status — and why it gets its own
  //     button here instead of being folded into a status dropdown.
  //
  // Read on demand, like the versions list: a store with thousands of entries should not
  // pay for two more queries on every status refresh.
  let curationShown = false;
  let rules: Array<Record<string, unknown>> = [];
  let candidates: Array<Record<string, unknown>> = [];

  // A curation list belongs to ONE store, for the same reason the version list does:
  // leaving the previous store's rows on screen beside buttons that would now act on a
  // different database is worse than showing nothing.
  $: if (selected) {
    rules = [];
    candidates = [];
    curationShown = false;
  }

  function entriesOf(payload: unknown): Array<Record<string, unknown>> {
    if (payload && typeof payload === "object") {
      const list = (payload as Record<string, unknown>).entries;
      if (Array.isArray(list)) return list as Array<Record<string, unknown>>;
    }
    return [];
  }

  function textOf(row: Record<string, unknown>, key: string): string {
    const value = row[key];
    return typeof value === "string" ? value : "";
  }

  /** The day half of an ISO timestamp — a rule stopped applying on a DAY, not at a ms. */
  function onDay(iso: unknown): string {
    return typeof iso === "string" && iso.length >= 10 ? iso.slice(0, 10) : "";
  }

  /**
   * The rules lane and the entries awaiting review.
   *
   * The rules are asked for by TYPE rather than by lane, and that is exact rather than a
   * workaround: the rules lane IS the `rule` type, one to one. Lane is the right filter
   * where a lane spans several types, and `list` does not take one — recorded rather than
   * worked around, since asking by type answers this view's question precisely.
   */
  async function readCuration(quiet = false) {
    if (!selectedRow || selectedRow.targets.length === 0) return;
    if (!quiet && busyAction) return;
    if (!quiet) busyAction = "list";
    try {
      const target = selectedRow.targets[0];
      const [ruleReply, candidateReply] = await Promise.all([
        experienceVerb(target, "list", { type: "rule", limit: 200 }),
        experienceVerb(target, "list", { status: "candidate", limit: 200 })
      ]);
      rules = entriesOf(ruleReply.success ? ruleReply.data : null);
      candidates = entriesOf(candidateReply.success ? candidateReply.data : null);
      curationShown = true;
      if (!quiet) {
        showResult("list", {
          rules: rules.length,
          awaitingReview: candidates.length
        });
      }
    } catch (error) {
      if (!quiet) showResult("list", { error: String(error) });
    } finally {
      if (!quiet) busyAction = "";
    }
  }

  async function retireOneRule(id: string, summary: string) {
    const confirmed = await confirmDestructive(
      "Retire this rule?\n\n" +
        summary +
        "\n\nIt stops being offered as guidance from now on. It is NOT deleted and NOT " +
        "marked wrong — it stays readable, with the date it stopped applying.\n\nContinue?"
    );
    if (!confirmed) return;
    // The list refresh is runVerb's, for every verb at once — see its comment.
    await runVerb("retire_rule", { id });
  }

  async function promoteOneEntry(id: string) {
    await runVerb("promote", { id });
  }

  // --- memory roots: pickers + removable list ------------------------------------------

  /** One Add… button (Harald): the OS dialog cannot offer files AND folders in a single
   * picker, and with the recursive crawl + skip-unchanged, a file's parent folder is
   * equivalent to the file — so folders are the one mode that covers everything. */
  async function addRoot() {
    const picked = await open({ title: "Add memory root folder", directory: true, multiple: true });
    if (!picked) return;
    for (const dir of Array.isArray(picked) ? picked : [picked]) {
      if (typeof dir === "string" && !memoryRoots.includes(dir)) {
        memoryRoots = [...memoryRoots, dir];
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
      Your memory store behind the JAWATA push channel. Everything else — refresh, curation,
      backups — runs automatically or by prompt.
    </p>
  </div>

  <div class="settings-grid">
    <!-- Memory sources FIRST — they feed the store below. -->
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Memory sources</h3>
        <p class="muted">
          Where load finds your memory files. Auto-discovered: agent INSTRUCTION files
          (CLAUDE.md, Claude project memory, Cursor/Copilot rules, AGENTS.md) — not your
          documents. Add your knowledge folders (docs/, sprints, postmortems, ADRs) as
          extra roots: every .md becomes recallable, anchored to the code it names.
        </p>
      </div>
      <label class="field">
        <span>Extra memory roots</span>
        <div class="actions">
          <button
            type="button"
            disabled={interactionDisabled}
            on:click={addRoot}
            title="Pick folder(s) to crawl in addition to the auto-discovered locations — subfolders and [[links]] are followed, unchanged files are skipped, so a folder also covers any single file in it"
          >
            Add…
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
            None — load uses only the auto-discovered instruction set: layered CLAUDE.md,
            every ~/.claude/projects/*/memory, .cursor/rules, .cursorrules, AGENTS.md,
            copilot-instructions.md. Your own docs (sprints, postmortems) are NOT found
            automatically — add their folders here.
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
                <th>Workspaces</th>
                <th>Entries</th>
                <th>Size</th>
              </tr>
            </thead>
            <tbody>
              {#each storeRows as row (row.key)}
                <tr
                  class:unreachable={!row.reachable}
                  title={row.error ?? row.file ?? undefined}
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
                  <td class="mono" title={row.file ?? undefined}>
                    {row.file ?? row.error ?? "–"}
                  </td>
                  <td>{row.workspaceLabels}</td>
                  <td>{row.total ?? "–"}</td>
                  <td>{formatBytes(row.bytes)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <!-- Sprint 28f Stage 5: the lane split — what the store HOLDS, before what to do
           about it. Rendered from the resident's own grouping; studio keeps no list of
           lane names, which is why a lane with no entries is absent rather than zero. -->
      {#if selectedRow?.lanes}
        <div class="lane-split">
          {#each Object.entries(selectedRow.lanes) as [lane, count] (lane)}
            <span class="lane" class:lane-unclassified={lane.startsWith("(")}>
              <strong>{count}</strong>
              {lane === "(none)" ? "unclassified" : lane}
            </span>
          {/each}
        </div>
        <p class="hint">
          Each lane is a lifecycle the store keeps apart: an experience was lived and can be
          superseded, a domain fact simply holds, a rule is versioned and retired. A lane
          with no entries is not listed. “Unclassified” is entries whose type no lane rule
          covers — counted here rather than filed under a default, so the gap is visible.
        </p>
      {/if}
      <div class="actions">
        <button
          type="button"
          disabled={statusLoading || interactionDisabled}
          on:click={refreshStatus}
          title="Re-read entry counts, store file and size from every resident (unreachable residents are retried automatically)"
        >
          {statusLoading ? "Loading…" : "Reload status"}
        </button>
        {#if autoRetrying && !busyAction}
          <span class="hint">resident starting — retrying automatically…</span>
        {/if}
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={runLoad}
          title={'Seed the store from your memory files — the auto-discovered instruction files plus your extra roots (docs folders and all). Runs on EVERY reachable workspace of this store (each contributes its own project locations). Idempotent: re-loading replaces, so this is also the re-initialize after a wipe. Say: "load my memory files"'}
        >
          Load
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={runCleanUp}
          title="One hygiene pass: prune aged rejected/superseded entries + merge duplicate groups + compact the store file. Runs prune, dedup and compact — each also available by prompt."
        >
          Clean up
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={runExport}
          title={'Write the whole store to a portable JSON file — opens the save dialog. Say: "export the store to a file"'}
        >
          Export…
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={runImport}
          title={'Re-ingest a previously exported JSON file (deduplicated by id) — opens the file picker. Say: "import the export file"'}
        >
          Import…
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={runBackup}
          title={'Take a copy of the whole store right now — before something this product does not know about. Wipe, Clean up, Import and Restore each take their own automatically. Say: "back up the store"'}
        >
          Back up now
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={showBackups}
          title={'List the kept versions, newest first, and put one back. Say: "restore the store"'}
        >
          {backupsShown ? "Refresh versions" : "Restore…"}
        </button>
        <button
          type="button"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={() => readCuration()}
          title={'The standing RULES, and the entries nobody has vouched for yet — each with the one action its lane\'s lifecycle allows. Say: "list the rules" / "promote that entry"'}
        >
          {curationShown ? "Refresh curation" : "Curation…"}
        </button>
        <button
          type="button"
          class="danger"
          disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
          on:click={() =>
            runVerb("wipe", {}, "wipe removes EVERY entry from this store. Continue?")}
          title={'Delete everything in the selected store. Re-initialize afterwards with load. Say: "wipe the store"'}
        >
          Wipe
        </button>
        {#if busyAction}
          <span class="hint">running “{busyAction}”…</span>
        {/if}
      </div>

      <!-- Sprint 28f D2: the versions, above the result panel, because choosing one IS
           the action here and the panel below reports what it did. -->
      {#if backupsShown}
        <div class="result-block">
          <h4>
            Versions{backupDepth
              ? ` — ${backupNames.length} kept, ${backupDepth} deep`
              : ""}
          </h4>
          {#if backupNames.length === 0}
            <p class="hint">
              No copies yet. One is taken automatically before Wipe, Clean up, Import and
              Restore — and “Back up now” takes one on demand.
            </p>
          {:else}
            <ul class="root-list">
              {#each backupNames as name (name)}
                <li>
                  <span class="mono">{name}</span>
                  <button
                    type="button"
                    class="danger"
                    disabled={!!busyAction || interactionDisabled}
                    on:click={() => restoreBackup(name)}
                    title="Replace every entry in this store with the ones this version holds"
                  >
                    Restore
                  </button>
                </li>
              {/each}
            </ul>
            <p class="hint">
              Newest first. A name is when the copy was taken and the action it preceded, so
              “…-wipe.zip” is the state as it stood just before that wipe.
            </p>
          {/if}
        </div>
      {/if}

      <!-- Sprint 28f Stage 5: two lanes, two lifecycles, one action each. Kept as two
           lists rather than one ranked pile for the reason the sweep's own lanes are:
           accepting an unvouched entry and retiring a standing rule are different acts,
           and a mixed list is one nobody can rule on. -->
      {#if curationShown}
        <div class="result-block">
          <h4>Rules — {rules.length}</h4>
          {#if rules.length === 0}
            <p class="hint">
              No rules yet. A rule is distilled FROM entries at the prompt — “promote these
              into a rule” — because choosing which entries it is drawn from is the act, and
              there is nothing here to select them with.
            </p>
          {:else}
            <ul class="root-list">
              {#each rules as rule (rule.id)}
                <li>
                  <span>
                    <strong>v{rule.rule_version ?? 1}</strong>
                    {textOf(rule, "summary")}
                    {#if rule.retired_at}
                      <em class="retired">— stopped applying {onDay(rule.retired_at)}</em>
                    {/if}
                  </span>
                  {#if !rule.retired_at}
                    <button
                      type="button"
                      disabled={!!busyAction || interactionDisabled}
                      on:click={() =>
                        retireOneRule(String(rule.id), textOf(rule, "summary"))}
                      title="It stops being offered as guidance from now on. Not deleted and not marked wrong — it stays readable, with the date it stopped applying."
                    >
                      Retire
                    </button>
                  {/if}
                </li>
              {/each}
            </ul>
            <p class="hint">
              A retired rule keeps its row and its date, which is what answers “what did this
              say, and until when”. Amending one — a new version, the old kept — is a prompt
              verb: the amended sentence is the work, and nothing here can write it.
            </p>
          {/if}

          <h4>Awaiting review — {candidates.length}</h4>
          {#if candidates.length === 0}
            <p class="hint">Nothing is waiting: every entry has been vouched for.</p>
          {:else}
            <ul class="root-list">
              {#each candidates as entry (entry.id)}
                <li>
                  <span>
                    <strong>{textOf(entry, "type")}</strong>
                    {textOf(entry, "summary")}
                  </span>
                  <button
                    type="button"
                    disabled={!!busyAction || interactionDisabled}
                    on:click={() => promoteOneEntry(String(entry.id))}
                    title="Accept this entry — it is then offered as vouched-for knowledge rather than as a candidate."
                  >
                    Promote
                  </button>
                </li>
              {/each}
            </ul>
            <p class="hint">
              Capped at 200 each. Promoting accepts one entry; there is no accept-all,
              deliberately — vouching for something nobody read is what the review exists to
              prevent.
            </p>
          {/if}
        </div>
      {/if}

      <!-- Results live right below the actions (Harald, 2026-07-06) — the right
           column grows, the sources column breathes. -->
      <div class="result-block">
        <h4>Result{outputTitle ? ` of “${outputTitle}”` : ""}</h4>
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
      </div>
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

  /* Sprint 28f Stage 5: the lane split reads as a row of counts, not a second table —
     it is one number per lane and a table would out-weigh the store table above it. */
  .lane-split {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin: 0.5rem 0 0.35rem;
  }
  .lane {
    padding: 0.15rem 0.5rem;
    border: 1px solid rgba(148, 163, 184, 0.28);
    border-radius: 999px;
    font-size: 0.8rem;
    white-space: nowrap;
  }
  .lane strong {
    margin-right: 0.25rem;
  }
  /* Unclassified is a GAP, not a lane — it is dimmed so it does not read as a fifth
     lifecycle, and it is never hidden, because hiding it restores the catch-all. */
  .lane-unclassified {
    opacity: 0.7;
    border-style: dashed;
  }
  .retired {
    opacity: 0.7;
    font-style: italic;
  }
  .result-block {
    margin-top: 0.4rem;
    padding-top: 0.6rem;
    border-top: 1px solid rgba(148, 163, 184, 0.18);
  }
  .result-block h4 {
    margin: 0 0 0.35rem;
    font-size: 0.95rem;
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
