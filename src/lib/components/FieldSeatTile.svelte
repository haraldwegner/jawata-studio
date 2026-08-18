<script lang="ts">
  // Sprint 28b (D10): the seat lane's tile-per-seat component. One tile ships —
  // `/report`; Sprint 28f fills the roster with the rest of the seats.
  //
  // The tile is PASSIVE apart from its two switches: it renders what the field
  // recording says and offers the two controls that turn jawata's speaking off.
  // Nothing here interrupts the user, and nothing here posts anything — the
  // seat itself shows the drafted body and the user's own `gh` does the filing.
  import { createEventDispatcher } from "svelte";
  import {
    fieldSetSilence,
    type FieldStatus,
    type FieldWorkspaceStatus
  } from "../api/tauri";

  export let entry: FieldWorkspaceStatus;
  export let disabled = false;

  const dispatch = createEventDispatcher<{ changed: FieldStatus }>();

  let busy = false;
  let error = "";

  $: lane = entry.lane;
  $: pile = entry.pile;
  $: topShapes = pile.shapes.slice(0, 5);
  $: worthReporting = pile.shapes.filter((s) => !s.posted && s.count >= 3);
  $: controlsDisabled = disabled || busy;

  async function setSwitch(nudges: boolean | null, silenced: boolean | null) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      dispatch("changed", await fieldSetSilence(entry.workspace, nudges, silenced));
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function onGoSilent(event: Event) {
    void setSwitch(null, (event.currentTarget as HTMLInputElement).checked);
  }

  function onNoNudges(event: Event) {
    // The checkbox reads "stop them", so the stored switch is its inverse.
    void setSwitch(!(event.currentTarget as HTMLInputElement).checked, null);
  }

  function whenShown(millis: number): string {
    if (!millis) return "never";
    return new Date(millis).toLocaleString();
  }
</script>

<article class="seat-tile">
  <header class="seat-tile-head">
    <h4><code>{lane.seat}</code></h4>
    <span class="seat-badge" class:seat-badge-quiet={worthReporting.length === 0}>
      {worthReporting.length} worth reporting
    </span>
  </header>
  <p class="muted seat-tile-intro">
    Turns this machine's local recording into a bug report you review and post from
    your own GitHub account — shapes only, never code or paths. Run it by typing
    <code>/report</code> in any client.
  </p>

  <section class="seat-tile-section">
    <h5>What is in the pile — {entry.workspace}</h5>
    {#if !pile.present}
      <p class="muted">
        Nothing recorded here yet. That is an absence, not a clean bill of health:
        it means no tool call has been observed in this workspace.
      </p>
    {:else}
      <p class="muted">
        {pile.totalEvents} recorded tool calls, {pile.failures} of them failed, across
        {pile.shapes.length} distinct failure shapes.
        {#if pile.unreadableLines > 0}
          {pile.unreadableLines} line(s) could not be read.
        {/if}
      </p>
      {#if topShapes.length > 0}
        <ul class="shape-list">
          {#each topShapes as shape (shape.shape)}
            <li>
              <code>{shape.shape}</code>
              <span class="shape-count">{shape.count}x</span>
              {#if shape.posted}<span class="shape-flag">filed</span>{/if}
              {#if lane.nudgedShapes.includes(shape.shape)}
                <span class="shape-flag shape-flag-soft">pointed at once</span>
              {/if}
              <span class="muted">{shape.clients.join(", ")}</span>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </section>

  <section class="seat-tile-section">
    <h5>Posted history</h5>
    {#if lane.postedShapes.length === 0}
      <p class="muted">Nothing filed from this machine yet.</p>
    {:else}
      <ul class="shape-list">
        {#each lane.postedShapes as shape (shape)}
          <li><code>{shape}</code> <span class="shape-flag">filed</span></li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="seat-tile-section">
    <h5>Reminders</h5>
    <p class="muted">
      Reminder state: <strong>{lane.reminderReason}</strong>.
      Shown {lane.remindersShown} time(s), last {whenShown(lane.lastRemindedAtMillis)};
      {lane.strikes} unanswered since you last used <code>/report</code>.
      {#if !lane.stateFilePresent}
        Both switches below are still at their defaults — you have not set either.
      {/if}
    </p>
  </section>

  <section class="seat-tile-section seat-switches">
    <h5>The two switches</h5>
    <p class="muted seat-distinction">
      These are two different things. The checkbox below stops the PERIODIC REMINDER
      the agent speaks at the start of a session. The switch under it stops the
      IN-SESSION POINTER — the single line that appears the third time one failure
      shape repeats. Turning one off leaves the other exactly as it is.
    </p>

    <label class="checkbox-row">
      <input
        type="checkbox"
        checked={lane.silenced}
        disabled={controlsDisabled}
        on:change={onGoSilent}
      />
      <span>
        Go silent about failures — stop the periodic reminder
        {#if lane.silenced}<em>(currently off by your choice)</em>{/if}
      </span>
    </label>

    <label class="checkbox-row">
      <input
        type="checkbox"
        checked={!lane.nudges}
        disabled={controlsDisabled}
        on:change={onNoNudges}
      />
      <span>
        No nudges — stop the in-session line pointing at <code>/report</code>
        {#if !lane.nudges}<em>(currently off by your choice)</em>{/if}
      </span>
    </label>

    {#if error}
      <p class="seat-error">{error}</p>
    {/if}
  </section>
</article>

<style>
  .seat-tile {
    border: 1px solid var(--border-color, #d7dbe3);
    border-radius: 8px;
    padding: 1rem 1.1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-width: 0;
  }
  .seat-tile-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .seat-tile-head h4 {
    margin: 0;
    font-size: 1.05rem;
  }
  .seat-badge {
    font-size: 0.8rem;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    background: #a8621a;
    color: #fff;
    white-space: nowrap;
  }
  .seat-badge-quiet {
    background: transparent;
    color: inherit;
    opacity: 0.6;
    border: 1px solid currentColor;
  }
  .seat-tile-intro {
    margin: 0;
  }
  .seat-tile-section {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .seat-tile-section h5 {
    margin: 0;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.75;
  }
  .seat-tile-section p {
    margin: 0;
  }
  .shape-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
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
  .shape-flag-soft {
    opacity: 0.55;
  }
  .seat-distinction {
    font-size: 0.85rem;
  }
  .seat-switches {
    gap: 0.5rem;
  }
  .seat-error {
    color: #a8621a;
    font-size: 0.85rem;
  }
</style>
