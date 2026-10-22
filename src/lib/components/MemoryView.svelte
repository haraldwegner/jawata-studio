<script lang="ts">
  import { closeOnOutside } from "../menu";
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
  /** False until the first status answer: an empty list before it means "not read yet", not "none". */
  let statusReadOnce = false;
  let selected = "";
  let busyAction = "";
  let outputTitle = "";
  let outputSummary: string[] = [];
  let outputRaw = "";

  // --- memory settings mirrors (saved via the normal settings round-trip) ---
  let storeMode = settings.experienceStoreMode ?? "shared";
  let memoryRoots: string[] = [...(settings.memoryRoots ?? [])];
  let autoSeedOnDeploy = settings.autoSeedOnDeploy ?? true;
  // Moved here from Settings (his walkthrough, 2026-10-21): both are decisions about the memory store.
  // An empty depth box is null and is sent as 0 (the resident's own default, stored as ABSENT, since the
  // resident would floor a literal 0 to 1). An empty folder box is the store's own story folder.
  // NUMBER, not string, and the type is load-bearing: Svelte coerces `bind:value` on an <input type="number">
  // through its own `to_number` (an empty box is null, anything else a number). Declared as a string it
  // type-checks and throws `.trim is not a function` on the first keystroke.
  let experienceBackupDepth: number | null = settings.experienceBackupDepth ?? null;
  let experienceStoriesDir = settings.experienceStoriesDir ?? "";
  let saveState: "idle" | "saving" | "saved" | "error" = "idle";
  let saveError = "";

  $: interactionDisabled = disabled || saveState === "saving";
  $: isDirty =
    storeMode !== (settings.experienceStoreMode ?? "shared") ||
    memoryRoots.join("\n") !== (settings.memoryRoots ?? []).join("\n") ||
    autoSeedOnDeploy !== (settings.autoSeedOnDeploy ?? true) ||
    (experienceBackupDepth ?? 0) !== (settings.experienceBackupDepth ?? 0) ||
    experienceStoriesDir.trim() !== (settings.experienceStoriesDir ?? "").trim();
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
    /**
     * Sprint 28f Stage 7: how many source units have been DESCRIBED, per bundle — the
     * cataloguer's progress, exactly as the resident groups it.
     *
     * Studio keeps no list of bundles, for the reason `lanes` gives one line up: a
     * hardcoded set here would be a second copy of a fact the engine owns, and a bundle
     * added or renamed upstream would show under its old name with a count of zero
     * forever. A bundle nobody has described does not appear at all.
     */
    described?: Record<string, number>;
    /** Bookkeeping writes the describing ledger LOST — absent when there are none. */
    describedLost?: number;
    /**
     * Sprint 28f Stage 7: the packages that HAVE an area summary written, as the resident
     * reports them.
     *
     * The complement is deliberately absent and the hint beside it says so: an area with
     * no row here has not been described yet, and ENUMERATING those needs the population
     * of packages, which only a loaded project knows. That is the same boundary
     * `described` above draws by carrying the numerator only.
     */
    describedAreas?: string[];
    /**
     * The resident's own statement that the list above was TRUNCATED — present only then.
     *
     * 2026-10-08. The complement clause the field above documents ("an area with no row
     * here has not been described yet") is true only when the read was complete, and the
     * resident caps it. When this is set the panel prints it INSTEAD of that clause: the
     * producer admits the filter and the view must not go on denying it.
     */
    describedAreasTruncated?: string;
    /** A sample of the rows the admission gate accepted — what was written, not how much. */
    acceptedSample?: { symbol?: string; summary: string }[];
    /** How many rows the sample was drawn FROM — absent when the resident could not say. */
    acceptedRows?: number;
    /**
     * Sprint 28g D27: the PATTERN CATALOGUE, as its own thing.
     *
     * His ruling: "They are the pattern templates for the architect and not just
     * experiences… The value should even be higher than our own experiences." The lane
     * split above cannot show that — a catalogue row carries a lane like any other row,
     * so its 189 entries were folded into those counts and the architect's template
     * library was invisible as a library.
     *
     * TWO POPULATIONS, TWO NUMBERS, and the hint says which is which. `entries` is what
     * this store HOLDS; the per-namespace figure for the upstream fork is what the
     * EXTRACTION seeded. They are close and they are not the same question, so a single
     * number would be wrong about one of them.
     *
     * Read from the resident, never listed here, for the reason `lanes` gives: a
     * namespace added upstream needs no change in this file.
     */
    catalogue?: {
      entries: number;
      byNamespace?: Record<string, number>;
      awaitingReview?: number;
      curesClean?: boolean;
      curesDeclared?: number;
      curesUnresolved?: number;
    };
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
          lanes: laneSplit(first?.stats),
          described: describedSplit(first?.stats),
          describedLost: lostDescribeWrites(first?.stats),
          describedAreas: describedAreas(first?.stats),
          describedAreasTruncated: describedAreasTruncated(first?.stats),
          acceptedSample: acceptedSample(first?.stats),
          acceptedRows: acceptedRows(first?.stats),
          catalogue: catalogueBlock(first?.stats)
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
      lanes: laneSplit(status.stats),
      described: describedSplit(status.stats),
      describedLost: lostDescribeWrites(status.stats),
      describedAreas: describedAreas(status.stats),
      describedAreasTruncated: describedAreasTruncated(status.stats),
      acceptedSample: acceptedSample(status.stats),
      acceptedRows: acceptedRows(status.stats),
      catalogue: catalogueBlock(status.stats)
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

  /**
   * Sprint 28g D27 — the catalogue block the resident reports, or undefined on an engine
   * that does not report one.
   *
   * Undefined rather than zeroes: a store whose engine cannot say how many patterns it
   * holds has not told us there are none, and rendering 0 would be this view inventing
   * an answer the engine declined to give.
   */
  function catalogueBlock(
    stats: KnowledgeWorkspaceStatus["stats"]
  ): StoreRow["catalogue"] | undefined {
    const raw = (stats as Record<string, unknown> | null | undefined)?.catalogue;
    if (!raw || typeof raw !== "object") return undefined;
    const source = raw as Record<string, unknown>;
    const entries = Number(source.entries);
    if (!Number.isFinite(entries)) return undefined;
    const ns = source.byNamespace;
    const byNamespace: Record<string, number> = {};
    if (ns && typeof ns === "object") {
      for (const [k, v] of Object.entries(ns as Record<string, unknown>)) {
        const n = Number(v);
        if (Number.isFinite(n)) byNamespace[k] = n;
      }
    }
    const cures = source.cures as Record<string, unknown> | undefined;
    const awaiting = Number(source.awaitingReview);
    return {
      entries,
      byNamespace: Object.keys(byNamespace).length > 0 ? byNamespace : undefined,
      awaitingReview: Number.isFinite(awaiting) ? awaiting : undefined,
      curesClean: typeof cures?.clean === "boolean" ? (cures.clean as boolean) : undefined,
      curesDeclared: Number.isFinite(Number(cures?.declared))
        ? Number(cures?.declared)
        : undefined,
      curesUnresolved: Number.isFinite(Number(cures?.unresolved))
        ? Number(cures?.unresolved)
        : undefined
    };
  }

  /** The `describing` block the resident reports, or undefined on an older engine. */
  function describingBlock(
    stats: KnowledgeWorkspaceStatus["stats"]
  ): Record<string, unknown> | undefined {
    const raw = (stats as Record<string, unknown> | null | undefined)?.describing;
    return raw && typeof raw === "object" ? (raw as Record<string, unknown>) : undefined;
  }

  /**
   * Sprint 28f Stage 7 — units described per bundle.
   *
   * THE NUMERATOR ONLY, and the hint below says so rather than the view inventing the
   * other half. How many units a bundle HOLDS is a question only a loaded project can
   * answer, and the resident's stats cannot see one; a percentage rendered here would be
   * a ratio made up at the point of display, which is the shape this product has paid
   * for before.
   */
  function describedSplit(
    stats: KnowledgeWorkspaceStatus["stats"]
  ): Record<string, number> | undefined {
    const raw = describingBlock(stats)?.describedPerBundle;
    if (!raw || typeof raw !== "object") return undefined;
    const source = raw as Record<string, unknown>;
    const out: Record<string, number> = {};
    // Same ordering rule as the lanes, and for the same reason: a bracketed or
    // lower-case placeholder like "unattributed" is a collector, and it reads last.
    for (const key of Object.keys(source).sort(
      (a, b) => Number(a === "unattributed") - Number(b === "unattributed")
    )) {
      const count = Number(source[key]);
      if (Number.isFinite(count)) out[key] = count;
    }
    return Object.keys(out).length > 0 ? out : undefined;
  }

  /**
   * Ledger writes that were LOST, or undefined when none were.
   *
   * Surfaced beside the counts rather than swallowed: progress computed over dropped rows
   * reads as "these units were never described" when the truth is "we failed to write it
   * down", and the two lead to opposite next actions.
   */
  function lostDescribeWrites(stats: KnowledgeWorkspaceStatus["stats"]): number | undefined {
    const raw = describingBlock(stats)?.lostBookkeepingWrites;
    const n = Number(raw);
    return Number.isFinite(n) && n > 0 ? n : undefined;
  }

  /**
   * Sprint 28f Stage 7 — the packages that HAVE an area summary, as the resident reports
   * them. Undefined on an older engine; EMPTY when the store holds no area row at all,
   * and the two are deliberately different: an empty list is the honest "nothing has been
   * summarised yet", which the block below says in words rather than rendering as a blank.
   *
   * Studio derives no complement from this. Which packages exist is a question only a
   * loaded project can answer, and a view that subtracted this list from a set it guessed
   * at would be inventing the half it cannot see — the same boundary `describedSplit`
   * keeps by carrying the numerator only.
   */
  function describedAreas(
    stats: KnowledgeWorkspaceStatus["stats"]
  ): string[] | undefined {
    const raw = describingBlock(stats)?.describedAreas;
    if (!Array.isArray(raw)) return undefined;
    return raw.filter((p): p is string => typeof p === "string" && p.length > 0);
  }

  /**
   * The resident's truncation note for the list above, when it set one.
   *
   * Undefined on an older engine, which is why the panel branches on presence rather than
   * on a boolean: an engine that does not publish this cannot be distinguished from one
   * whose read was complete, and the complement clause is what an absent note leaves
   * standing. That is the honest reading — the note is an ADMISSION, so its absence can
   * only mean "nothing was admitted", never "nothing was truncated".
   */
  function describedAreasTruncated(
    stats: KnowledgeWorkspaceStatus["stats"]
  ): string | undefined {
    const raw = describingBlock(stats)?.describedAreasTruncated;
    return typeof raw === "string" && raw.length > 0 ? raw : undefined;
  }

  /**
   * A sample of the rows the admission gate accepted — WHAT was written, beside the counts
   * that say how much. Undefined on an older engine or when nothing has been accepted.
   *
   * Listed by SYMBOL because that is what a row carries: a job entry has no bundle and no
   * package, so the resident cannot group the sample and neither can this.
   */
  function acceptedSample(
    stats: KnowledgeWorkspaceStatus["stats"]
  ): { symbol?: string; summary: string }[] | undefined {
    const raw = describingBlock(stats)?.acceptedSample;
    if (!Array.isArray(raw) || raw.length === 0) return undefined;
    const out: { symbol?: string; summary: string }[] = [];
    for (const row of raw) {
      if (!row || typeof row !== "object") continue;
      const r = row as Record<string, unknown>;
      const summary = typeof r.summary === "string" ? r.summary : "";
      if (!summary) continue;
      out.push({
        symbol: typeof r.symbol === "string" ? r.symbol : undefined,
        summary
      });
    }
    return out.length > 0 ? out : undefined;
  }

  /**
   * How many rows the sample was drawn FROM.
   *
   * The resident omits this when its own read was capped, rather than publishing a capped
   * figure as a total — so an absence here means "could not say", and the view shows no
   * number instead of showing a wrong one.
   */
  function acceptedRows(stats: KnowledgeWorkspaceStatus["stats"]): number | undefined {
    const raw = describingBlock(stats)?.acceptedRows;
    const n = Number(raw);
    return Number.isFinite(n) ? n : undefined;
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
      statusReadOnce = true;
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
        if (ruleCount !== undefined) {
          lines.push(
            `${ruleCount} rule(s), ${asCount(p.entries) ?? 0} entr(ies), ` +
              `${asCount(p.awaitingReview) ?? 0} awaiting review`
          );
        }
        break;
      }
      case "promote_rule":
        if (typeof p.id === "string") {
          const sources = Array.isArray(p.derived_from) ? p.derived_from.length : undefined;
          lines.push(
            `Promoted as v${p.rule_version ?? 1}` +
              (sources !== undefined ? `, drawn from ${sources} entr(ies)` : "") +
              ". The sources are NOT superseded — they go on answering as themselves."
          );
        }
        break;
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
  let entries: Array<Record<string, unknown>> = [];

  // Sprint 28f Stage 5 — promoting entries INTO a rule, which the stage's clause puts
  // here in as many words: "promote_rule(ids…) (creates version 1 linked to its sources,
  // FROM STUDIO)". Choosing which entries a rule is drawn from IS the act, and a screen
  // that already lists them is where a prompt is worst and this is best.
  let selectedSources = new Set<string>();
  let ruleSentence = "";

  $: awaitingReview = entries.filter((e) => e.status === "candidate").length;
  $: canPromoteRule = selectedSources.size > 0 && ruleSentence.trim().length > 0;

  // A curation list belongs to ONE store, for the same reason the version list does:
  // leaving the previous store's rows on screen beside buttons that would now act on a
  // different database is worse than showing nothing. The half-written rule goes with
  // them — a sentence drafted about one store's entries is not about another's.
  $: if (selected) {
    rules = [];
    entries = [];
    selectedSources = new Set();
    ruleSentence = "";
    curationShown = false;
  }

  /**
   * Svelte 4 reacts to ASSIGNMENT, not to mutation, so a Set that is added to in place
   * leaves the button disabled while the box is visibly ticked. A new Set each time.
   */
  function toggleSource(id: string, on: boolean) {
    const next = new Set(selectedSources);
    if (on) {
      next.add(id);
    } else {
      next.delete(id);
    }
    selectedSources = next;
  }

  async function promoteToRule() {
    if (!canPromoteRule) return;
    await runVerb("promote_rule", {
      ids: [...selectedSources],
      summary: ruleSentence.trim()
    });
    // Cleared only after the call, so a failure leaves the draft and the selection where
    // the person can see what was attempted and try again.
    ruleSentence = "";
    selectedSources = new Set();
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
      const [ruleReply, entryReply] = await Promise.all([
        experienceVerb(target, "list", { type: "rule", limit: 200 }),
        experienceVerb(target, "list", { limit: 200 })
      ]);
      rules = entriesOf(ruleReply.success ? ruleReply.data : null);
      // Rules are excluded from the entry list because they have their own list above
      // with their own action — a row appearing twice with two different buttons is the
      // mixed list this block exists to avoid. They remain selectable as SOURCES nowhere,
      // which is deliberate: a rule drawn from a rule is an amendment, a different verb.
      entries = entriesOf(entryReply.success ? entryReply.data : null).filter(
        (e) => e.type !== "rule"
      );
      curationShown = true;
      if (!quiet) {
        showResult("list", {
          rules: rules.length,
          entries: entries.length,
          awaitingReview: awaitingReview
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
    if (
      experienceBackupDepth !== null &&
      (!Number.isInteger(experienceBackupDepth) || experienceBackupDepth < 1 || experienceBackupDepth > 500)
    ) {
      saveState = "error";
      saveError = "Backup copies must be a whole number from 1 to 500, or empty for the engine's default.";
      return;
    }
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
        memoryRoots: [...memoryRoots],
        experienceBackupDepth: experienceBackupDepth ?? 0,
        experienceStoriesDir: experienceStoriesDir.trim()
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

  /** What each lane of the store is, as the mouseover on its count. The resident names the lanes; studio keeps
   * no list of them, so a lane not named here still shows, with the generic line. */
  const LANE_HINTS: Record<string, string> = {
    experience: "Something lived here: a lesson or a failure mode. It can be superseded by a later one.",
    domain: "A fact that simply holds: a contract, a convention, how something works.",
    patterns: "The architect's template library: design patterns it reaches for when a design question has no answer in what we have lived. Nothing in ordinary use removes one; a re-seed supersedes rows rather than deleting them.",
    rules: "Standing guidance. A rule is versioned and retired, never deleted.",
    code: "What each class member and package is for, written by the cataloguer.",
    "(none)": "Entries whose type no lane rule covers. They are counted here, not filed under a default, so the gap stays visible."
  };

  /** Closes the "More" menu after one of its buttons was pressed. */
  function closeMore(event: Event) {
    (event.currentTarget as HTMLElement).closest("details")?.removeAttribute("open");
  }

  $: describedUnits = selectedRow?.described
    ? Object.values(selectedRow.described).reduce((sum, count) => sum + count, 0)
    : 0;
</script>

<!-- runtime-settings-root = the app-wide "scrollable middle + sticky footer" scroll
     container (the same mechanism Settings and Dashboard use). -->
<section class="panel stack runtime-settings-root memory-root">
  <div>
    <h2>Memory</h2>
    {#if selectedRow}
      <p
        class="summary-line"
        title={storeMode === "shared"
          ? "One user-level store, shared by every workspace, so what is learned in one is recallable in all."
          : "One store per workspace."}
      >
        <strong>{selectedRow.total ?? "–"} entries</strong> · {formatBytes(selectedRow.bytes)} ·
        {storeMode === "shared" ? "shared by every workspace" : "one store per workspace"}
      </p>
      <!-- The two conditions the old view printed in the open and a fold must not bury (the S17 audit):
           the cataloguer's bookkeeping lost writes, and an areas list the resident cut. -->
      {#if selectedRow.describedLost}
        <p class="summary-line attention">
          <strong>{selectedRow.describedLost} cataloguer bookkeeping write(s) were LOST</strong> —
          those units will be offered again, so the cataloguer's figures may be lower than the work done.
        </p>
      {/if}
      {#if selectedRow.describedAreasTruncated}
        <p class="summary-line attention">
          <strong>The list of described areas is cut:</strong> {selectedRow.describedAreasTruncated}
        </p>
      {/if}
    {:else if storeRows.length === 0}
      <!-- Said once, and only after the store was actually asked: the seconds before the first
           answer read as "no workspaces" while three were running (his screenshot, 2026-10-22). -->
      <p class="summary-line">
        {statusReadOnce ? "No workspaces, or their services are not reachable." : "Reading the store…"}
      </p>
    {/if}
  </div>

  <div class="stack">
      {#if storeRows.length === 0}
        <!-- the summary line above already says why there is nothing here -->
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


    <div class="actions">
      <button
        type="button"
        disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
        on:click={runLoad}
        title={'Seed the store from your memory files — the auto-discovered instruction files plus your extra roots (docs folders and all). Runs on EVERY reachable workspace of this store. Idempotent: re-loading replaces, so this is also the re-initialize after a wipe. Say: "load my memory files"'}
      >
        Load
      </button>
      <button
        type="button"
        disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
        on:click={runBackup}
        title={'Take a copy of the whole store right now. Wipe, Clean up, Import and Restore each take their own automatically. Say: "back up the store"'}
      >
        Back up now
      </button>
      <button
        type="button"
        disabled={!!busyAction || interactionDisabled || !selectedRow?.targets.length}
        on:click={runCleanUp}
        title="One hygiene pass: prune aged rejected/superseded entries, merge duplicate groups and compact the store file. Each is also available by prompt."
      >
        Clean up
      </button>
        <details class="more-menu" use:closeOnOutside>
          <summary title="Less common actions">More ▾</summary>
          <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
          <div class="more-menu-body" on:click={closeMore}>
            <button
              type="button"
              disabled={statusLoading || interactionDisabled}
              on:click={refreshStatus}
              title="Re-read entry counts, store file and size from every resident (unreachable residents are retried automatically)"
            >
              {statusLoading ? "Loading…" : "Reload status"}
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
              title={'Delete everything in the selected store. Re-initialize afterwards with Load. Say: "wipe the store"'}
            >
              Wipe…
            </button>
          </div>
        </details>
      {#if autoRetrying && !busyAction}
        <span class="hint">resident starting — retrying automatically…</span>
      {/if}
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
              No rules yet. Tick the entries a rule is drawn from in the list below, write
              the sentence, and promote them.
            </p>
          {:else}
            <ul class="root-list">
              {#each rules as rule (rule.id)}
                <li>
                  <span>
                    <strong>v{rule.rule_version ?? 1}</strong>
                    {textOf(rule, "summary")}
                    <!-- No sources here, and that is measured rather than forgotten: links
                         live in their own table, which is why the EXPORT has to join them
                         in, so a `list` row cannot carry them. A rule's sources are shown
                         where the plan measures them — in the promote response, below —
                         and making them visible on a rule afterwards is an engine change
                         nobody has asked for. Recorded at C5 rather than half-built. -->
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

          <h4>Entries — {entries.length}, {awaitingReview} awaiting review</h4>
          {#if entries.length === 0}
            <p class="hint">No entries in this store yet — “Load” seeds it from your files.</p>
          {:else}
            <ul class="root-list">
              {#each entries as entry (entry.id)}
                <li>
                  <label class="source-pick" title="Select as a source for a new rule">
                    <input
                      type="checkbox"
                      checked={selectedSources.has(String(entry.id))}
                      disabled={!!busyAction || interactionDisabled}
                      on:change={(event) =>
                        toggleSource(String(entry.id), event.currentTarget.checked)}
                    />
                    <span>
                      <strong>{textOf(entry, "type")}</strong>
                      {textOf(entry, "summary")}
                    </span>
                  </label>
                  {#if entry.status === "candidate"}
                    <button
                      type="button"
                      disabled={!!busyAction || interactionDisabled}
                      on:click={() => promoteOneEntry(String(entry.id))}
                      title="Accept this entry — it is then offered as vouched-for knowledge rather than as a candidate."
                    >
                      Promote
                    </button>
                  {/if}
                </li>
              {/each}
            </ul>
            <p class="hint">
              Up to 200 (the order is the store's own, not a recency ranking). Promoting
              accepts ONE entry; there is deliberately no accept-all — vouching for
              something nobody read is what the review exists to prevent.
            </p>
          {/if}

          <!-- Sprint 28f Stage 5: "promote_rule(ids…) creates version 1 linked to its
               sources, FROM STUDIO". The sentence is required and is not generated: a
               rule is what the tick-boxes MEAN taken together, and distilling them is the
               whole promotion — a generated line would do the mechanics and deliver none
               of it. -->
          <div class="promote-rule">
            <label class="field">
              <span>Promote {selectedSources.size} selected into a rule</span>
              <input
                bind:value={ruleSentence}
                disabled={!!busyAction || interactionDisabled}
                placeholder="the rule itself — one sentence a reader can follow and act on"
                type="text"
              />
            </label>
            <button
              type="button"
              disabled={!canPromoteRule || !!busyAction || interactionDisabled}
              on:click={promoteToRule}
              title="Write a standing rule drawn from the ticked entries. They are NOT superseded — they go on answering as themselves, and the rule records which ones it came from."
            >
              Promote to rule
            </button>
          </div>
          <p class="hint">
            The sources keep answering as themselves; the rule records which they were. Tick
            at least one entry and write the sentence — neither is optional, because a rule
            with no sources is an assertion nobody can check and a generated sentence is not
            a rule. Amending one later is a prompt verb: the amended sentence IS the work.
          </p>
        </div>
      {/if}

      <!-- Results live right below the actions (Harald, 2026-07-06) — the right
           column grows, the sources column breathes. -->
      <div class="result-block">
        <h4>Status{outputTitle && outputTitle !== "status" ? ` of “${outputTitle}”` : ""}</h4>
        {#if outputSummary.length > 0}
          <ul class="result-lines">
            {#each outputSummary as line}
              <li>{line}</li>
            {/each}
          </ul>
        {:else if !outputRaw}
          <p class="hint">No action run yet — what the last action did appears here.</p>
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
  </div>

  <details class="fold" open>
    <summary title="What the store holds, how far the cataloguer has got, and where the pattern library stands.">What the store holds</summary>
    <div class="fold-body">
      {#if selectedRow?.lanes}
        <div class="lane-split">
          {#each Object.entries(selectedRow.lanes) as [lane, count] (lane)}
            <span
              class="lane"
              class:lane-unclassified={lane.startsWith("(")}
              title={LANE_HINTS[lane] ?? "A kind of entry the store keeps apart from the others."}
            >
              <strong>{count}</strong>
              {lane === "(none)" ? "unclassified" : lane}
            </span>
          {/each}
        </div>
      {/if}

      <!-- Sprint 28g D27, his ruling: the pattern library is shown as its own thing, as a share of the
           whole store. Kept as one line; the namespace split and the reason the counts differ sit in the
           mouseover. The denominator is the row's OWN total, never a literal (C13). -->
      {#if selectedRow?.catalogue}
        <p
          class="summary-line"
          title={`The architect's template library. The first figure is how many of this store's rows are patterns; a per-namespace figure is what that source last SEEDED, so the two drift apart when a pattern is added here or retired upstream. ${
            selectedRow.catalogue.byNamespace
              ? Object.entries(selectedRow.catalogue.byNamespace).map(([ns, count]) => `${ns}: ${count}`).join(" · ")
              : ""
          }`}
        >
          <strong>Pattern library:</strong>
          {selectedRow.catalogue.entries}{#if selectedRow.total !== undefined} of {selectedRow.total}{/if}
          rows{#if selectedRow.catalogue.curesDeclared !== undefined}
            · {selectedRow.catalogue.curesDeclared} cures{selectedRow.catalogue.curesUnresolved
              ? `, ${selectedRow.catalogue.curesUnresolved} unresolved`
              : ", all resolved"}{/if}
        </p>
      {/if}

      <!-- The complement clause ("any area not listed is not described") is only true when the read was
           not truncated (the C14 architect watch), so a truncation note is shown, never dropped. -->
      {#if selectedRow?.described || selectedRow?.describedAreas}
        <p
          class="summary-line"
          class:attention={!!selectedRow?.describedLost || !!selectedRow?.describedAreasTruncated}
          title={`Source units the cataloguer has described, per bundle: ${
            selectedRow?.described
              ? Object.entries(selectedRow.described).map(([bundle, count]) => `${bundle} ${count}`).join(", ")
              : "none"
          }. How many units a bundle holds only a loaded project can say, so no percentage is claimed. An area with a summary written is one the cataloguer recorded for a whole package; an area not listed is not described yet${
            selectedRow?.describedAreasTruncated ? ", EXCEPT that this list is cut: " + selectedRow.describedAreasTruncated : ""
          }. Which areas are still missing needs a loaded project: experience(kind=describe, action=next) answers it, one row per package in scope.`}
        >
          <strong>Cataloguer:</strong>
          {describedUnits} units described{#if selectedRow?.described}
            in {Object.keys(selectedRow.described).length} bundles{/if}
          {#if selectedRow?.describedAreas}
            · {selectedRow.describedAreas.length > 0
              ? `${selectedRow.describedAreas.length} areas summarised${selectedRow.describedAreasTruncated ? " (list cut)" : ""}`
              : "no area summarised yet"}
          {/if}
          {#if selectedRow?.describedLost}
            · <strong>{selectedRow.describedLost} writes LOST</strong>
          {/if}
        </p>
      {/if}
    </div>
  </details>

  {#if selectedRow?.acceptedSample && selectedRow.acceptedSample.length > 0}
    <details class="fold">
      <summary
        title={`A sample of what the cataloguer wrote and the admission gate accepted${
          selectedRow?.acceptedRows ? ` — ${selectedRow.acceptedSample.length} of ${selectedRow.acceptedRows} rows` : ""
        }. At most two rows share a class, so one busy class cannot crowd out the rest. Read these to judge the work.`}
      >
        What the cataloguer wrote ({selectedRow.acceptedSample.length}{#if selectedRow?.acceptedRows} of {selectedRow.acceptedRows}{/if})
      </summary>
      <div class="fold-body">
        <ul class="sample">
          {#each selectedRow.acceptedSample as row, i (row.symbol ?? i)}
            <li>
              {#if row.symbol}<code>{row.symbol}</code>{/if}
              <span>{row.summary}</span>
            </li>
          {/each}
        </ul>
      </div>
    </details>
  {/if}

  <details class="fold">
    <summary title="Where Load finds your memory files, which store it fills, and the store's own copies and story files.">Memory sources and store settings</summary>
    <div class="fold-body">
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
          <span
            class="hint"
            title="Found automatically: your layered CLAUDE.md files, every ~/.claude/projects/*/memory, .cursor/rules, .cursorrules, AGENTS.md and copilot-instructions.md. Your own documents (sprints, postmortems, ADRs) are NOT found automatically."
          >
            None — only the instruction files found automatically are loaded. Add your docs folders here.
          </span>
        {/if}
      </label>
      <label class="field">
        <span>Store mode</span>
        <select
          bind:value={storeMode}
          disabled={interactionDisabled}
          title="Where the experience store lives. Shared makes your knowledge recallable from every workspace."
        >
          <option value="shared">shared — one user-level store (default)</option>
          <option value="workspace">workspace — per-workspace store</option>
        </select>
      </label>
      <label
        class="checkbox-row"
        title="After every successful deploy, run Load on each resident so the knowledge is there from day one"
      >
        <input type="checkbox" bind:checked={autoSeedOnDeploy} disabled={interactionDisabled} />
        <span>Auto-seed on deploy</span>
      </label>
      <label class="field">
        <span>Memory-store backup copies</span>
        <input
          bind:value={experienceBackupDepth}
          disabled={interactionDisabled}
          min="1"
          max="500"
          placeholder="engine default"
          title="How many copies of the memory store each resident keeps before dropping the oldest. A copy is about 37 MB, so ten are about 370 MB before compression. Empty = the engine's own default. This is not the MCP config backup in Settings → Agents."
          type="number"
        />
      </label>
      <label class="field">
        <span>Story export folder</span>
        <input
          bind:value={experienceStoriesDir}
          disabled={interactionDisabled}
          placeholder="the store's own story folder"
          title="When you accept a piece of knowledge, the engine also writes it as a readable markdown file here, one file per entry. The store stays the source of truth; deleting the folder loses nothing. Empty = the store's own story folder. Type off to write no files."
          type="text"
        />
      </label>
    </div>
  </details>
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
  /* Sprint 28g D27: the catalogue total reads as the HEADLINE of its own strip, so a
     glance separates "what the library holds" from the per-source figures beside it.
     Weight only — no colour, because the strip already distinguishes blocks by position
     and a second channel here would compete with the unclassified marker. */
  .lane-catalogue {
    font-weight: 600;
  }
  /* Sprint 28f Stage 7: the sample is PROSE to be read, not a row of counts — so it is a
     list with the sentence given the room to be read, where the lane strip above is a
     glance. Rendering it as pills would make the one thing on this panel a person must
     actually read look like another tally. */
  .sample {
    list-style: none;
    margin: 0.5rem 0 0.35rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .sample li {
    font-size: 0.8rem;
    line-height: 1.35;
    padding-left: 0.6rem;
    border-left: 2px solid rgba(148, 163, 184, 0.28);
  }
  .sample code {
    display: block;
    opacity: 0.75;
    font-size: 0.75rem;
    overflow-wrap: anywhere;
  }
  .retired {
    opacity: 0.7;
    font-style: italic;
  }
  /* The checkbox and the entry it selects are ONE label, so the whole row is the hit
     target — a tick-box a person has to aim at is how the wrong entry gets sourced. */
  .source-pick {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    cursor: pointer;
    min-width: 0;
  }
  .source-pick input {
    flex: none;
  }
  .promote-rule {
    display: flex;
    align-items: flex-end;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }
  .promote-rule .field {
    flex: 1;
    min-width: 0;
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
