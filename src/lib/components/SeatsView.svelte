<script lang="ts">
  // Sprint 28g (D19 / D20): the seats view.
  //
  // WHAT IS ON THIS PAGE, and the sentence that asked for each part (C11's exit clause):
  //   "every deployed seat listed with what it is for, when it fires, its tools, gates and
  //    model"                                  -> the rows
  //   "four settings changeable per seat (on/off, clients, brain, reset-to-shipped)"
  //                                            -> the controls on each row, EXCEPT the three
  //                                               cases below, each declared rather than hidden
  // Nothing else is here. There is no explanatory paragraph: the UX seat's census asks which
  // sentence of the spec asked for each block, and a hint nobody requested has no answer.
  // Two blocks that census REFUSED, and that were removed: a `/command` label and a
  // "customised" tag. The clause asks for neither, and the Reset button - shown only when
  // there is something to reset - already says the second.
  //
  // Three controls the clause names are NOT all shown, and why:
  //   brain   - shown as text. Nothing in this app consumes a seat's model (the hosted runner
  //             has no production caller; the hire path reads the seat file), so an editable
  //             field would echo the edit back and change nothing.
  //   On      - only for a seat that has a command. A driven seat writes none; "off" removes
  //             nothing.
  //   Clients - only when there are two or more INDEPENDENT clients. Cursor reads Claude's seat
  //             directory, so offering it beside Claude would let a seat be dropped from both
  //             while the view showed Cursor ticked.
  //
  // A change is written, then the dashboard's own Deploy is asked for — so a toggle is not a
  // control that does nothing until the user remembers to press something elsewhere.
  import { createEventDispatcher, onMount } from "svelte";
  import { seatChange, seatsView, type SeatChange, type SeatRow, type SeatsView } from "../api/tauri";

  export let disabled = false;

  const dispatch = createEventDispatcher<{ changed: void }>();

  let view: SeatsView | null = null;
  let loadError = "";
  let actionError = "";
  let busy = false;
  async function load() {
    try {
      view = await seatsView();
      loadError = "";
    } catch (error) {
      loadError = String(error);
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
            <h3>{seat.name}</h3>
            <p class="muted">{seat.purpose}</p>
          </div>

          {#if seat.deployable}
            <label class="checkbox-row">
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
               `dd` clamps every value to one line, which would cut a seat's tool list. -->
          <dl class="seat-facts">
            <dt>Fires</dt>
            <dd>{seat.fires}{#if seat.schedule} · schedule {seat.schedule}{/if}</dd>
            <dt>Tools</dt>
            <dd>{seat.tools.length > 0 ? seat.tools.join(", ") : "none"}</dd>
            <dt>Gates</dt>
            <dd>{seat.gates.length > 0 ? seat.gates.join(", ") : "none"}</dd>
            <dt>Brain</dt>
            <dd>{seat.model}</dd>
            {#if seat.deployable && view.clients.length >= 2}
              <dt>Clients</dt>
              <dd>
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
  .seat-facts dd {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: unset;
    overflow: visible;
  }
</style>
