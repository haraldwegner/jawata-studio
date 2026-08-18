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

  /** File reads only on the backend, so polling costs nothing worth counting. */
  const POLL_MILLIS = 5000;

  $: utilization = status?.utilization ?? null;
  $: recall = status?.recall ?? null;
  $: store = status?.store ?? null;
  // Stage 9: the classpath half. Loaded once on mount rather than polled — an
  // import's outcome changes when a workspace is (re)loaded, not second to
  // second, and this costs one health_check per resident.
  let resolution: ResolutionStatus[] = [];
  $: unresolvedProjects = resolution.flatMap((r: ResolutionStatus) =>
    r.projects.map((p: ProjectResolution) => ({ ...p, workspace: r.workspace })),
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
    <!-- The tab says "Reporting" — what you DO here. The header names what is
         ON the page — the recording itself. Repeating the tab word in the
         header is chrome, not hierarchy. -->
    <h2>Field recording</h2>
    <p class="muted">
      Shapes only — tool, kind, error code, counts, latency, client, version. No paths,
      no symbol names, no message text. Nothing leaves this machine unless you send it:
      type <code>/report</code> in any client to file one from your own GitHub account.
    </p>
    <label class="checkbox-row field-silence">
      <input
        type="checkbox"
        checked={allSilenced}
        disabled={disabled || silenceBusy || workspaces.length === 0}
        on:change={onGoSilent}
      />
      <span>
        Go silent about failures — stop the periodic reminder
        {#if allSilenced}<em>(currently off by your choice)</em>{/if}
      </span>
    </label>
    {#if silenceError}
      <p class="field-error">{silenceError}</p>
    {/if}
  </div>

  {#if loadError}
    <p class="field-error">Could not read the field recording: {loadError}</p>
  {/if}

  <div class="field-headline">
    <div class="field-stat">
      <span class="field-stat-value">{status?.badge ?? 0}</span>
      <span class="field-stat-label">shapes worth reporting</span>
    </div>
    <div class="field-stat">
      <span class="field-stat-value">{sharePercent ?? "—"}</span>
      <span class="field-stat-label">went through JAWATA</span>
    </div>
    <div class="field-stat">
      <span class="field-stat-value">{status?.deadChannels.length ?? 0}</span>
      <span class="field-stat-label">dead channels</span>
    </div>
    <div class="field-stat" class:field-stat-degraded={canaryHealth === "degraded"}>
      <span class="field-stat-value">{canaryHealth}</span>
      <span class="field-stat-label">residents {canaryWord}</span>
    </div>
    <div
      class="field-stat"
      class:field-stat-degraded={store?.health === "slow" || store?.health === "unavailable"}
    >
      <span class="field-stat-value">{store?.word ?? "not checked yet"}</span>
      <span class="field-stat-label">
        knowledge store{#if store && store.slowestMillis > 0} · {store.slowestMillis} ms{/if}
      </span>
    </div>
  </div>

  <div class="settings-grid">
    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>JAWATA vs the shell</h3>
        <p class="muted">
          {#if sharePercent}
            {status?.utilization.jawataCalls} tool calls went through JAWATA;
            {status?.utilization.shellFallbacks} pieces of Java work went to a shell text
            tool instead ({status?.utilization.slips} declared fallbacks,
            {status?.utilization.ungroundedReads} reads with no lookup behind them).
          {:else}
            Nothing observed yet — an empty denominator is not a perfect score.
          {/if}
        </p>
      </div>
      <p class="field-caveat">{status?.utilization.caveat ?? ""}</p>
      {#if utilization && !utilization.observerPresent}
        <p class="muted">
          No observer has written here, so this counts JAWATA's half and nothing against it.
        </p>
      {/if}
    </section>

    {#if unresolvedProjects.length > 0}
      <section class="panel stack settings-section">
        <div class="section-intro">
          <h3>Projects missing dependencies</h3>
          <p class="muted">
            These resolved fewer dependencies than they asked for, so their errors may be
            the classpath rather than the code.
          </p>
        </div>
        <ul class="shape-list">
          {#each unresolvedProjects as p (p.workspace + p.projectKey)}
            <li>
              <code>{p.projectKey}</code>
              <span class="shape-count">{p.unresolved} unresolved</span>
              <span class="muted">{p.workspace}</span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Recalled knowledge</h3>
        <p class="muted">
          {#if recall && recall.present}
            What the agent did with what it was given.
          {:else}
            Nothing observed yet.
          {/if}
        </p>
      </div>
      <ul class="recall-counts">
        <li><span>{recall?.applied ?? 0}</span> applied</li>
        <li><span>{recall?.rejected ?? 0}</span> judged and rejected</li>
        <li class:recall-bad={(recall?.skipped ?? 0) > 0}>
          <span>{recall?.skipped ?? 0}</span> taken and never answered
        </li>
        <li><span>{recall?.wouldBlock ?? 0}</span> would block</li>
        <li><span>{recall?.blocked ?? 0}</span> blocked</li>
        <li><span>{recall?.unavailable ?? 0}</span> store unavailable</li>
      </ul>
      <p class="field-caveat">{recall?.coverage ?? ""}</p>
      {#if store && (store.health === "slow" || store.health === "unavailable")}
        <p class="field-caveat">
          {store.worstWorkspace}: {store.why} (over {store.slowAboveMillis} ms)
        </p>
      {/if}
    </section>

    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Top failure shapes</h3>
        <p class="muted">
          Ranked by repeats. Three or more, unfiled, is what the count above counts.
        </p>
      </div>
      {#if allShapes.length === 0}
        <p class="muted">No failures recorded.</p>
      {:else}
        <ul class="shape-list">
          {#each allShapes as shape (shape.workspace + shape.shape)}
            <li>
              <code>{shape.shape}</code>
              <span class="shape-count">{shape.count}x</span>
              {#if shape.posted}<span class="shape-flag">filed</span>{/if}
              <span class="muted">{shape.workspace}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Channel reach</h3>
        <p class="muted">
          DEAD = the store answered and nothing reached the session. Quiet is not dead —
          nothing to say, or this client cannot inject, is listed separately.
        </p>
      </div>
      {#if (status?.silenceLogsRead.length ?? 0) === 0}
        <p class="muted">
          No hook has ever run here — a finding, not a healthy zero.
        </p>
      {:else}
        {#if status && status.deadChannels.length > 0}
          <p class="field-dead">Dead: {status.deadChannels.join(", ")}</p>
        {:else}
          <p class="muted">No dead channels.</p>
        {/if}
        {#if status && status.legitimatelyQuietChannels.length > 0}
          <p class="muted">
            Quiet by design: {status.legitimatelyQuietChannels.join(", ")}
          </p>
        {/if}
        <ul class="shape-list">
          {#each status?.channels ?? [] as channel (channel.role)}
            <li>
              <code>{channel.role}</code>
              <span class="muted">
                {channel.emitted} of {channel.fired} reached the session
              </span>
              {#if channel.dead}<span class="shape-flag">dead</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Resident canary</h3>
        <p class="muted">
          Each resident is asked one real recall and one real compiler question.
          Failing either turns this amber and tints the tray — nothing more.
        </p>
      </div>
      {#if (status?.canary.length ?? 0) === 0}
        <p class="muted">Not checked yet.</p>
      {:else}
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
      {/if}
    </section>
  </div>

  <!-- The seat lane left this page (Harald, 2026-08-18): seats get their own
       menu item in Sprint 28f. The go-silent switch and the /report mention
       moved into the header above; the no-nudges switch is parked until 28f,
       its on-disk state untouched. -->
</section>

<style>
  /* The control is a separate act from the explanation above it — it needs the
     breathing room that says so, rather than reading as the paragraph's last
     line. Matched below by the headline row, so the checkbox sits in its own
     band between the two. */
  .field-silence {
    margin-top: 0.9rem;
  }
  .field-headline {
    margin-top: 0.4rem;
    display: flex;
    flex-wrap: wrap;
    gap: 1.5rem;
  }
  .field-stat {
    display: flex;
    flex-direction: column;
    min-width: 9rem;
  }
  .field-stat-value {
    font-size: 1.6rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .field-stat-label {
    font-size: 0.85rem;
    opacity: 0.75;
  }
  .field-stat-degraded .field-stat-value {
    color: #a8621a;
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
