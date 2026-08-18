<script lang="ts">
  // Sprint 28b (D2 + D10 + D6): the field view.
  //
  // PASSIVE BY CONSTRUCTION. This view renders what jawata recorded locally and
  // offers the two switches on the /report tile. It mounts nothing over the
  // window, it opens nothing, and a failing canary changes a colour here and on
  // the tray icon rather than interrupting anyone. That is asserted from the
  // Rust side, not merely stated here: field_view.rs::interruption_scans reads
  // this file and fails the build's own suite if an interrupting surface ever
  // appears in it.
  //
  // LAYOUT: the app-wide panel / settings-grid / section-intro classes, as the
  // Memory view uses them.
  import { onDestroy, onMount } from "svelte";
  import FieldSeatTile from "./FieldSeatTile.svelte";
  import { fieldStatus, type FieldStatus } from "../api/tauri";

  export let disabled = false;

  let status: FieldStatus | null = null;
  let loading = false;
  let loadError = "";
  let timer: ReturnType<typeof setInterval> | null = null;

  /** File reads only on the backend, so polling costs nothing worth counting. */
  const POLL_MILLIS = 5000;

  $: utilization = status?.utilization ?? null;
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
  $: canaryWord =
    canaryHealth === "green"
      ? "answering"
      : canaryHealth === "degraded"
        ? "not answering"
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
    <h2>Field</h2>
    <p class="muted">
      What jawata recorded on THIS machine while you worked: shapes only — tool name,
      kind, error code, counts, latency bucket, client and version. No file paths, no
      symbol names, no message text, and nothing leaves this machine unless you run
      <code>/report</code> and post it yourself.
    </p>
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
            Nothing has been observed yet, so there is no share to show. An empty
            denominator is not a perfect score.
          {/if}
        </p>
      </div>
      <p class="field-caveat">{status?.utilization.caveat ?? ""}</p>
      {#if utilization && !utilization.observerPresent}
        <p class="muted">
          The observer has never written on this machine, so the number above counts
          JAWATA's own half and nothing against it.
        </p>
      {/if}
    </section>

    <section class="panel stack settings-section">
      <div class="section-intro">
        <h3>Top failure shapes</h3>
        <p class="muted">
          Ranked by how often they repeat. A shape at three or more that you have not
          filed is what the count at the top of this page is counting.
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
          A channel is DEAD when the store answered and nothing reached the session —
          the signature of the outage that went two weeks unseen. Quiet is not dead:
          a channel with nothing to say, or one this client cannot inject on, is
          listed separately.
        </p>
      </div>
      {#if (status?.silenceLogsRead.length ?? 0) === 0}
        <p class="muted">
          No hook has ever run on this machine — there is nothing to fold. That is a
          finding, not a healthy zero.
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
          Every few minutes each resident is asked one real recall and one real
          compiler question about a type every Java workspace can resolve. A resident
          that cannot answer both turns this amber and tints the tray icon — nothing
          more; you find out when you look.
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

  <section class="panel stack settings-section">
    <div class="section-intro">
      <h3>Seats</h3>
      <p class="muted">
        One tile per seat. <code>/report</code> ships here now; the rest of the roster
        arrives with Sprint 28f.
      </p>
    </div>
    {#if workspaces.length === 0}
      <p class="muted">
        No workspace has a resident yet, so there is no recording and no seat state.
      </p>
    {:else}
      <div class="seat-lane">
        {#each workspaces as entry (entry.workspace)}
          <FieldSeatTile
            {disabled}
            {entry}
            on:changed={(event) => (status = event.detail)}
          />
        {/each}
      </div>
    {/if}
  </section>
</section>

<style>
  .field-headline {
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
  .seat-lane {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(22rem, 1fr));
    gap: 1rem;
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
