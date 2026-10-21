<script lang="ts">
  // Sprint 28b (D2 + D10 + D6): the field view.
  //
  // PASSIVE BY CONSTRUCTION. This view renders what jawata recorded locally and
  // offers the go-silent switch in its header. It mounts nothing over the
  // window, it opens nothing, and a failing canary changes a colour here and on
  // the tray icon rather than interrupting anyone. That is asserted from the
  // Rust side, not merely stated here: field_view.rs::interruption_scans reads
  // this file and fails the build's own suite if an interrupting surface ever
  // appears in it.
  //
  // Harald's ruling (2026-08-18): the SEAT LANE left this page — a Seats menu
  // item is Sprint 28f's work. What survived it, moved into the header: the
  // /report mention and the go-silent control. The no-nudges switch is parked
  // until 28f (its on-disk state persists; a user who set it keeps it).
  //
  // LAYOUT: the app-wide panel / settings-grid / section-intro classes, as the
  // Memory view uses them.
  import { onDestroy, onMount } from "svelte";
  import {
    fieldSetSilence,
    fieldStatus,
    resolutionStatus,
    type FieldStatus,
    type ProjectResolution,
    type ResolutionStatus,
  } from "../api/tauri";

  export let disabled = false;

  let status: FieldStatus | null = null;
  let loading = false;
  let loadError = "";
  let timer: ReturnType<typeof setInterval> | null = null;

  /** Reads on the backend, so polling costs nothing worth counting.
   *
   * studio#22 made it not quite reads-only: a workspace with no cached copy of
   * the machine's go-silent switch is given one here. That write happens ONCE
   * per such workspace and never for one that already has a value, so the
   * steady state of this poll is still reads — and `add_project` is where a new
   * workspace actually inherits, which is why this stays a backstop rather than
   * becoming the reason for the cadence. */
  const POLL_MILLIS = 5000;

  $: utilization = status?.utilization ?? null;
  $: recall = status?.recall ?? null;
  // D4 — WAS ANYTHING OBSERVED AT ALL. Every counter below is read against this
  // rather than defaulted to 0: on a machine where no observer has ever written,
  // a zero is an absence of measurement and printing it as a number states a
  // result nobody measured.
  // Sprint 28f: the observed/un-observed guard was a derived boolean here and is now the
  // expression itself at each site. TypeScript cannot narrow `recall` through a derived
  // name, so the guard the READER sees and the guard the COMPILER sees have to be the
  // same one — and `npm run check` was red for seven errors until they were.
  $: store = status?.store ?? null;
  // Stage 9: the classpath half. Loaded once on mount rather than polled — an
  // import's outcome changes when a workspace is (re)loaded, not second to
  // second, and this costs one health_check per resident.
  let resolution: ResolutionStatus[] = [];
  $: unresolvedProjects = resolution.flatMap((r: ResolutionStatus) =>
    r.projects.map((p: ProjectResolution) => ({ ...p, workspace: r.workspace })),
  );
  // A workspace that cannot be READ is a different and worse fact than one
  // missing dependencies: every whole-workspace answer is incomplete and
  // refactorings are refused. The resident says so; this only carries it.
  $: unreadableWorkspaces = resolution.filter(
    (r: ResolutionStatus) => r.reachable && r.healthy === false,
  );
  $: sharePercent =
    utilization && utilization.percent !== null && utilization.percent !== undefined
      ? `${utilization.percent}%`
      : null;
  $: workspaces = status?.workspaces ?? [];
  $: allShapes = workspaces
    .flatMap((w) => w.pile.shapes.map((s) => ({ ...s, workspace: w.workspace })))
    .sort((a, b) => b.count - a.count)
    .slice(0, 8);
  $: canaryHealth = status?.canaryHealth ?? "unknown";
  // GO SILENT IS PER MACHINE. Harald's ruling, 2026-08-18: "I report tool
  // failures for jawata. Why should I want to report for one workspace and not
  // for the other?" — so the per-workspace split is an implementation detail of
  // where the state file lives, never a distinction the user is shown. There is
  // no indeterminate state to render: the box reflects whether reminders are
  // off, and setting it settles every workspace.
  $: allSilenced = workspaces.length > 0 && workspaces.every((w) => w.lane.silenced);
  let silenceBusy = false;
  let silenceError = "";

  /** Fan the existing per-workspace command out to every workspace. */
  async function onGoSilent(event: Event) {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    silenceBusy = true;
    silenceError = "";
    try {
      for (const w of workspaces) {
        status = await fieldSetSilence(w.workspace, null, checked);
      }
    } catch (error) {
      silenceError = String(error);
    } finally {
      silenceBusy = false;
    }
  }
  // The ONE sentence the view opens on: what, if anything, needs a look. Each condition is a figure the
  // five-number strip used to show; the strip is gone and its figures are folded into this.
  $: attention = [
    (status?.badge ?? 0) > 0 ? `${status?.badge} failure shape${status?.badge === 1 ? "" : "s"} worth reporting` : "",
    (status?.deadChannels.length ?? 0) > 0 ? `dead channel${status?.deadChannels.length === 1 ? "" : "s"}: ${status?.deadChannels.join(", ")}` : "",
    canaryHealth === "degraded" ? "a resident is not answering" : "",
    store?.health === "slow" ? "the knowledge store is slow" : "",
    store?.health === "unavailable" ? "the knowledge store is not answering" : "",
    unresolvedProjects.length > 0 ? `${unresolvedProjects.length} project${unresolvedProjects.length === 1 ? "" : "s"} need attention` : "",
  ].filter((entry) => entry !== "");
  $: shownShapes = allShapes.slice(0, 3);
  $: canaryWord =
    canaryHealth === "green"
      ? "answering"
      : canaryHealth === "degraded"
        ? "not answering"
        : canaryHealth === "loading"
          ? "starting up"
          : "not checked yet";

  async function refresh() {
    if (loading) return;
    loading = true;
    try {
      status = await fieldStatus();
      loadError = "";
    } catch (error) {
      loadError = String(error);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void refresh();
    timer = setInterval(() => void refresh(), POLL_MILLIS);
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
  });
</script>

<section class="panel stack runtime-settings-root field-root">
  <div>
    <h2>Field recording</h2>
    <p class="muted">
      Shapes only — no paths, symbol names or message text. Nothing leaves this machine unless you
      type <code>/report</code> in any client to file one from your own GitHub account.
    </p>
    {#if loadError}
      <p class="field-error">Could not read the field recording: {loadError}</p>
    {:else if !status}
      <p class="summary-line">Reading the recording…</p>
    {:else if attention.length === 0}
      <p
        class="summary-line"
        title={`${status.utilization.caveat ?? ""}`}
      >
        <strong>Nothing needs a look.</strong>
        {#if sharePercent}
          <span title={`${status.utilization.jawataCalls} tool calls went through JAWATA; ${status.utilization.shellFallbacks} pieces of Java work went to a shell text tool instead (${status.utilization.slips} declared fallbacks, ${status.utilization.ungroundedReads} reads with no lookup behind them).`}>
            {sharePercent} of Java work went through JAWATA ·
          </span>
        {/if}
        knowledge store {store?.word ?? "not checked yet"}{#if store && store.slowestMillis > 0} ({store.slowestMillis} ms){/if} · residents {canaryWord}
      </p>
    {:else}
      <p class="summary-line attention">
        <strong>Needs a look:</strong> {attention.join(" · ")}
      </p>
    {/if}
    <label class="checkbox-row field-silence" title="Stops the periodic reminder that failures are piling up. The recording itself keeps running and this page still shows everything.">
      <input
        type="checkbox"
        checked={allSilenced}
        disabled={disabled || silenceBusy || workspaces.length === 0}
        on:change={onGoSilent}
      />
      <span>
        Go silent about failures
        {#if allSilenced}<em>(currently off by your choice)</em>{/if}
      </span>
    </label>
    {#if silenceError}
      <p class="field-error">{silenceError}</p>
    {/if}
  </div>

  {#if unresolvedProjects.length > 0}
    <details class="fold" open>
      <summary title="A project here either resolved fewer dependencies than it asked for, so its errors may be the classpath rather than the code, or cannot be read at all.">
        Projects needing attention ({unresolvedProjects.length})
      </summary>
      <div class="fold-body">
        {#each unreadableWorkspaces as w (w.workspace)}
          {#if w.warning}
            <p class="muted">{w.warning}</p>
          {/if}
        {/each}
        <ul class="shape-list">
          {#each unresolvedProjects as p (p.workspace + p.projectKey)}
            <li>
              <code>{p.projectKey}</code>
              <span class="muted">{p.projectPath}</span>
              {#if p.healthy === false}
                <span class="shape-count">cannot be read</span>
              {:else}
                <span class="shape-count">{p.unresolved} unresolved</span>
              {/if}
              <span class="muted">{p.workspace}</span>
              {#if p.problem}
                <span class="muted">{p.problem}</span>
              {/if}
              {#if p.remedy}
                <span class="muted">{p.remedy}</span>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    </details>
  {/if}

  {#if canaryHealth === "degraded"}
    <details class="fold" open>
      <summary title="Each resident is asked one real recall and one real compiler question. Failing either turns this amber and tints the tray.">
        Residents not answering
      </summary>
      <div class="fold-body">
        <ul class="shape-list">
          {#each status?.canary ?? [] as probe (probe.workspace)}
            <li class:field-dead={!probe.green}>
              <code>{probe.workspace}</code>
              <span class="muted">
                store: {probe.recallDetail} · compiler: {probe.compilerDetail}
              </span>
            </li>
          {/each}
        </ul>
      </div>
    </details>
  {/if}

  <details class="fold">
    <summary title="The figures behind the sentence above.">Details</summary>
    <div class="fold-body">
      <section class="stack">
        <h3 title="What the agent did with the knowledge it was handed. 'Taken and never answered' means the knowledge store was asked and its answer was not used.">Recalled knowledge</h3>
        <!--
          Each figure carries its own state. "Nothing observed yet" in a heading was not enough: a list
          of zeros reads as a measurement. D4 asks for the figure shown as observed or labelled
          un-observed AT THE POINT OF DISPLAY.
        -->
        <ul class="recall-counts">
          <li>
            {#if recall && recall.present}<span>{recall.applied}</span>{:else}<span
                class="recall-unobserved">not observed</span>{/if} applied
          </li>
          <li>
            {#if recall && recall.present}<span>{recall.rejected}</span>{:else}<span
                class="recall-unobserved">not observed</span>{/if} judged and rejected
          </li>
          <li class:recall-bad={!!recall && recall.present && recall.skipped > 0}>
            {#if recall && recall.present}<span>{recall.skipped}</span>{:else}<span
                class="recall-unobserved">not observed</span>{/if} taken and never answered
          </li>
          <li>
            {#if recall && recall.present}<span>{recall.wouldBlock}</span>{:else}<span
                class="recall-unobserved">not observed</span>{/if} would block
          </li>
          <li>
            {#if recall && recall.present}<span>{recall.blocked}</span>{:else}<span
                class="recall-unobserved">not observed</span>{/if} blocked
          </li>
          <li>
            {#if recall && recall.present}<span>{recall.unavailable}</span>{:else}<span
                class="recall-unobserved">not observed</span>{/if} store unavailable
          </li>
        </ul>
        <p class="field-caveat">{recall?.coverage ?? ""}</p>
        {#if store && (store.health === "slow" || store.health === "unavailable")}
          <p class="field-caveat">
            {store.worstWorkspace}: {store.why} (over {store.slowAboveMillis} ms)
          </p>
        {/if}
      </section>

      <section class="stack">
        <h3 title="Ranked by repeats. Three or more repeats that nobody has filed are what the count in the sentence above counts.">Top failure shapes</h3>
        {#if allShapes.length === 0}
          <p class="muted">No failures recorded.</p>
        {:else}
          <ul class="shape-list">
            {#each shownShapes as shape (shape.workspace + shape.shape)}
              <li>
                <code>{shape.shape}</code>
                <span class="shape-count">{shape.count}x</span>
                {#if shape.posted}<span class="shape-flag">filed</span>{/if}
                <span class="muted">{shape.workspace}</span>
              </li>
            {/each}
          </ul>
          {#if allShapes.length > shownShapes.length}
            <details class="fold">
              <summary>{allShapes.length - shownShapes.length} more</summary>
              <div class="fold-body">
                <ul class="shape-list">
                  {#each allShapes.slice(shownShapes.length) as shape (shape.workspace + shape.shape)}
                    <li>
                      <code>{shape.shape}</code>
                      <span class="shape-count">{shape.count}x</span>
                      {#if shape.posted}<span class="shape-flag">filed</span>{/if}
                      <span class="muted">{shape.workspace}</span>
                    </li>
                  {/each}
                </ul>
              </div>
            </details>
          {/if}
        {/if}
      </section>

      <section class="stack">
        <h3 title="DEAD means the store answered and nothing reached the session. Quiet is not dead: a channel with nothing to say, or on a client that cannot inject text, is not counted.">Channel reach</h3>
        {#if (status?.silenceLogsRead.length ?? 0) === 0}
          <p class="muted">No hook has ever run here — a finding, not a healthy zero.</p>
        {:else}
          {#if status && status.deadChannels.length > 0}
            <p class="field-dead">Dead: {status.deadChannels.join(", ")}</p>
          {:else}
            <p class="muted">No dead channels.</p>
          {/if}
          {#if status && status.legitimatelyQuietChannels.length > 0}
            <p class="muted">Quiet by design: {status.legitimatelyQuietChannels.join(", ")}</p>
          {/if}
          <details class="fold">
            <summary title="How often each hook ran and how often it put text into the session. A hook that runs and says nothing is not dead: it had nothing to say.">
              Every channel ({status?.channels.length ?? 0})
            </summary>
            <div class="fold-body">
              <ul class="shape-list">
                {#each status?.channels ?? [] as channel (channel.role)}
                  <li>
                    <code>{channel.role}</code>
                    <span class="muted">
                      ran {channel.fired} times, put text into the session {channel.emitted} times
                    </span>
                    {#if channel.dead}<span class="shape-flag">dead</span>{/if}
                  </li>
                {/each}
              </ul>
            </div>
          </details>
        {/if}
      </section>
    </div>
  </details>
</section>

<style>
  /* The control is a separate act from the explanation above it — it needs the
     breathing room that says so, rather than reading as the paragraph's last
     line. Matched below by the headline row, so the checkbox sits in its own
     band between the two. */
  .field-silence {
    margin-top: 0.9rem;
  }
  .recall-counts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 1.1rem;
    font-size: 0.9rem;
  }
  .recall-counts span {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  /* The one number that means something went wrong. */
  .recall-counts .recall-bad span {
    color: var(--color-warning, #d08700);
  }
  /* Not a number: it must not inherit the figure's weight, or an absence of
     measurement reads at a glance as a measured result. */
  .recall-counts span.recall-unobserved {
    font-weight: 400;
    font-style: italic;
    opacity: 0.7;
  }

  .field-caveat {
    font-size: 0.85rem;
    opacity: 0.85;
    border-left: 3px solid #a8621a;
    padding-left: 0.6rem;
    margin: 0;
  }
  .field-error,
  .field-dead {
    color: #a8621a;
  }
  .shape-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.9rem;
  }
  .shape-list li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
  }
  .shape-count {
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
  }
  .shape-flag {
    font-size: 0.75rem;
    padding: 0 0.35rem;
    border-radius: 4px;
    border: 1px solid currentColor;
    opacity: 0.8;
  }
</style>
