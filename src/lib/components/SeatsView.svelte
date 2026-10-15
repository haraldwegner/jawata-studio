<script lang="ts">
  // Sprint 28g (D19 / D20): the seats view — what a PERSON decides with, and nothing else.
  //
  // Harald, 2026-10-12, on the 4.4.0 view: *"it means that a human can look at it and take
  // decisions, change config"* · *"What could be interesting from human perspective in tools and
  // gates? This is not at users discretion at all."* · *"The explanations are to short, we have
  // an on/off switch so he must know"* · *"The brain is necessary and we can show already here,
  // because we want to select once we have ACP"* · *"The seats' name is not architect but
  // "/refactor" and this info is mandatory architect (/refactor)"* · *"We need the mouseovers."*
  //
  // So each seat shows: its name WITH how it is called · what it does, long enough to decide on
  // the switch · when it is used · its brain · the switch, and Reset when there is something to
  // reset. Every one of them has a mouseover saying what it means or what changing it does.
  // Tools, gates and the schedule are not shown: no person acts on them, and no schedule runs.
  //
  // On is offered only where it does something (a seat with a command); Clients only when two
  // INDEPENDENT clients exist — Cursor reads Claude's seat directory, so offering both would let
  // a seat vanish from both while Cursor read ticked. A change is written and the seat part of a
  // deploy runs at once, so a toggle never waits on a second button somewhere else (C11-4: only
  // the seat commands, not every client's MCP configuration).
  import { createEventDispatcher, onMount } from "svelte";
  import {
    brainProbe,
    brainsStatus,
    seatChange,
    seatsView,
    type BrainStatus,
    type ProbeResult,
    type SeatChange,
    type SeatRow,
    type SeatsView,
  } from "../api/tauri";

  export let disabled = false;

  const dispatch = createEventDispatcher<{ changed: void }>();

  let view: SeatsView | null = null;
  /** What every agent program on this machine says about itself — asked of the programs, never a
   * list kept here. */
  let brains: BrainStatus[] = [];
  let brainsChecking = false;
  /** Per seat: the test run in progress (true) or the last one's result. */
  let testing: Record<string, boolean> = {};
  let lastTest: Record<string, ProbeResult> = {};
  let loadError = "";
  let actionError = "";
  let busy = false;
  async function load() {
    try {
      view = await seatsView();
      void checkBrains(view);
      loadError = "";
    } catch (error) {
      loadError = String(error);
    }
  }

  /** Asks every agent program on this machine whether it is installed and logged in. Runs after
   * the list is shown, so a slow program never holds the page. */
  async function checkBrains(_current: SeatsView) {
    brainsChecking = true;
    try {
      brains = await brainsStatus([]);
    } catch (error) {
      actionError = `brain check: ${String(error)}`;
    } finally {
      brainsChecking = false;
    }
  }

  async function apply(seat: SeatRow, change: SeatChange) {
    if (busy) return;
    busy = true;
    actionError = "";
    try {
      view = await seatChange(seat.name, change);
      dispatch("changed");
    } catch (error) {
      actionError = `${seat.name}: ${String(error)}`;
    } finally {
      busy = false;
    }
  }

  /** One dropdown entry: a brain a hire can start and is installed here, with one of the models it
   * names for itself, or its own default. The value is `brain|model` (empty model = default). */
  interface BrainOption {
    value: string;
    label: string;
  }

  function loginNote(b: BrainStatus): string {
    if (b.login === "notLoggedIn") return " (not logged in)";
    if (b.login === "unknown") return " (login not checkable)";
    return "";
  }

  function brainOptions(seat: SeatRow): BrainOption[] {
    const options: BrainOption[] = [];
    for (const b of brains.filter((x) => x.hirable && x.installed)) {
      const note = loginNote(b);
      options.push({
        value: `${b.id}|`,
        label: `${b.label} — ${b.takesModel ? "its default model" : "chooses its own model"}${note}`,
      });
      if (b.takesModel) {
        for (const m of b.models ?? []) options.push({ value: `${b.id}|${m}`, label: `${b.label} — ${m}${note}` });
      }
    }
    // The saved choice is always shown, even when this machine cannot start it right now.
    const current = `${seat.brain}|${seat.model ?? ""}`;
    if (!options.some((o) => o.value === current)) {
      const b = brains.find((x) => x.id === seat.brain);
      options.unshift({
        value: current,
        label: `${b?.label ?? seat.brain} — ${seat.model ?? "its default model"} (not available on this machine)`,
      });
    }
    return options;
  }

  /** Test first, save only if the brain answered: a choice that cannot run is never stored. */
  async function chooseBrain(seat: SeatRow, select: HTMLSelectElement) {
    const saved = `${seat.brain}|${seat.model ?? ""}`;
    const [brain, model] = select.value.split("|");
    testing = { ...testing, [seat.name]: true };
    try {
      const result = await brainProbe(brain, model || null);
      lastTest = { ...lastTest, [seat.name]: result };
      if (result.ok) {
        await apply(seat, { setting: "brain", brain, model: model || null });
      } else {
        select.value = saved; // a choice that did not answer is not shown as made
      }
    } catch (error) {
      actionError = `${seat.name}: ${String(error)}`;
      select.value = saved;
    } finally {
      testing = { ...testing, [seat.name]: false };
    }
  }

  function brainLabel(id: string): string {
    return brains.find((b) => b.id === id)?.label ?? id;
  }

  /** How a person reaches the seat, shown beside its name: the command, or where it runs. */
  function invocation(seat: SeatRow): string {
    if (seat.command) return seat.command;
    return seat.pipeline ? "inside /sprint" : "hired by a seat";
  }

  function invocationHint(seat: SeatRow): string {
    if (seat.command) return `Type ${seat.command} in your agent's chat to run this seat.`;
    return seat.pipeline
      ? "Runs as one of the two steps of /sprint. It has no command of its own."
      : "Has no command. Another seat hires it for a job, gives it a brief, and checks what it returns.";
  }

  function onHint(seat: SeatRow): string {
    return `Ticked: ${seat.command} is available in your agents. Unticked: ${seat.command} is removed from them, at once. The seat itself is kept either way.`;
  }

  const FIRES_HINT = "When this seat is meant to be used: the point in your work at which an agent is sent to it.";
  const BRAIN_HINT = "The agent program, and the model, that a hire of this seat starts — logged in with your account on this machine. Choosing one first runs a one-word test through it (Claude Code: under a cent, measured) and is saved only if it answers. The list is read from the programs installed here.";
  const RESET_HINT = "Discards your changes to this seat and takes the version this studio ships. Your changed file is kept beside it as a backup.";
  const CLIENTS_HINT = "Which of your agents get this seat's command.";

  /** Whether the seat's command is written for this client right now. `null` means all. */
  function onClient(seat: SeatRow, clientId: string): boolean {
    return seat.clients === null || seat.clients.includes(clientId);
  }

  function toggleClient(seat: SeatRow, clientId: string, checked: boolean) {
    const all = (view?.clients ?? []).map((c) => c.id);
    const current = seat.clients ?? all;
    const next = checked
      ? all.filter((id) => id === clientId || current.includes(id))
      : current.filter((id) => id !== clientId);
    apply(seat, { setting: "clients", value: next });
  }

  onMount(load);
</script>

<!-- The app's own view skeleton, copied from the Memory and Reporting views rather than styled
     here: `runtime-settings-root` is the class that makes the middle of the page SCROLL (the
     shell row is fixed-height), and its absence left every seat below the first screen
     unreachable in 4.4.0. Each seat is a `settings-section` panel in the shared two-column
     `settings-grid`; seats are of similar height, so the pairs line up. -->
<section class="panel stack runtime-settings-root seats-root">
  <h2>Seats</h2>

  {#if loadError}
    <p class="project-error">{loadError}</p>
  {/if}
  {#if actionError}
    <p class="project-error">{actionError}</p>
  {/if}
  {#if view && view.errors.length > 0}
    {#each view.errors as error}
      <p class="project-error">{error}</p>
    {/each}
  {/if}

  {#if view}
    <div class="settings-grid">
      {#each view.seats as seat (seat.name)}
        <section class="panel stack settings-section" class:seat-off={!seat.enabled}>
          <div class="section-intro">
            <h3 title={invocationHint(seat)}>{seat.name} ({invocation(seat)})</h3>
            <p class="seat-purpose">{seat.purpose}</p>
          </div>

          {#if seat.deployable}
            <label class="checkbox-row" title={onHint(seat)}>
              <input
                checked={seat.enabled}
                disabled={disabled || busy}
                on:change={(e) =>
                  apply(seat, { setting: "enabled", value: e.currentTarget.checked })}
                type="checkbox"
              />
              <span>On</span>
            </label>
          {/if}

          <!-- The one local style: the shared sheet has no label/value list, and its global
               `dd` clamps every value to one line. -->
          <dl class="seat-facts">
            <dt title={FIRES_HINT}>When</dt>
            <dd title={FIRES_HINT}>{seat.fires}</dd>
            <dt title={BRAIN_HINT}>Brain</dt>
            <dd title={BRAIN_HINT}>
              {#if brains.length === 0}
                {brainsChecking ? "checking which brains this machine has…" : brainLabel(seat.brain)}
              {:else}
                {@const status = brains.find((b) => b.id === seat.brain)}
                <select
                  class="seat-brain"
                  value={`${seat.brain}|${seat.model ?? ""}`}
                  disabled={disabled || busy || testing[seat.name]}
                  on:change={(e) => chooseBrain(seat, e.currentTarget)}
                >
                  {#each brainOptions(seat) as option (option.value)}
                    <option value={option.value}>{option.label}</option>
                  {/each}
                </select>
                {#if seat.runsInAgent}
                  <span class="muted seat-note">Typed as {seat.command ?? "part of /sprint"}, it runs on your own agent; this is the brain a hire starts.</span>
                {/if}
                {#if testing[seat.name]}
                  <span class="seat-note">Testing — one word through the brain…</span>
                {:else if lastTest[seat.name]}
                  {@const t = lastTest[seat.name]}
                  {#if t.ok}
                    <span class="seat-note">
                      Tested: answered “{t.answer}” in {t.seconds.toFixed(1)} s{#if t.modelsUsed.length > 0} · {t.modelsUsed[0]}{/if}{#if t.costUsd !== null} · ${t.costUsd.toFixed(3)}{/if}
                    </span>
                  {:else}
                    <span class="project-error seat-note">{brainLabel(t.brain)} did not answer: {t.error}</span>
                    <span class="seat-note">To log in, run <code>{t.loginCommand}</code> in a terminal, then choose it again. The previous choice is kept.</span>
                  {/if}
                {:else if status && status.installed && status.login === "notLoggedIn"}
                  <span class="project-error seat-note">{status.label} is not logged in: {status.reason}</span>
                  <span class="seat-note">To log in, run <code>{status.loginCommand}</code> in a terminal, then open this tab again.</span>
                {:else if status && !status.installed}
                  <span class="project-error seat-note">{status.reason}</span>
                {/if}
              {/if}
            </dd>
            {#if seat.deployable && view.clients.length >= 2}
              <dt title={CLIENTS_HINT}>Clients</dt>
              <dd title={CLIENTS_HINT}>
                {#each view.clients as client (client.id)}
                  <label class="checkbox-row">
                    <input
                      checked={onClient(seat, client.id)}
                      disabled={disabled || busy}
                      on:change={(e) => toggleClient(seat, client.id, e.currentTarget.checked)}
                      type="checkbox"
                    />
                    <span>{client.label}</span>
                  </label>
                {/each}
              </dd>
            {/if}
          </dl>

          {#if seat.customised}
            <button
              disabled={disabled || busy}
              on:click={() => apply(seat, { setting: "reset" })}
              title={RESET_HINT}
              type="button"
            >
              Reset to shipped
            </button>
          {/if}
        </section>
      {/each}
    </div>
  {:else if !loadError}
    <p class="muted">Loading seats...</p>
  {/if}
</section>

<style>
  .seat-off {
    opacity: 0.6;
  }
  /* The purpose is what a person decides on, so it reads at body size, not as a muted aside. */
  .seat-purpose {
    margin: 0;
    color: #cbd5e1;
    font-size: 0.95rem;
    line-height: 1.45;
  }
  .seat-facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.35rem 0.9rem;
    margin: 0;
    font-size: 0.9rem;
  }
  .seat-facts dt {
    color: #94a3b8;
  }
  /* A login failure is something to act on, so it reads at the size of the line it sits in. */
  .seat-facts .project-error {
    font-size: inherit;
  }
  .seat-brain {
    width: 100%;
  }
  .seat-note {
    display: block;
    margin-top: 0.3rem;
  }
  .seat-facts dd {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: unset;
    overflow: visible;
  }
</style>
