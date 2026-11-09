<script lang="ts">
  // Sprint 28h, D53 (l): the workflow as an org chart — who hires whom.
  //
  // His words: *"There should be a generatl lock/unlock toggle button. And as in a real tree we should be able to
  // shift etc."* · *"Org cahrt I would assume as well"* · *"stacked is fine"*. Locked by default, so it opens with
  // nothing to fill in. Unlocked, a seat dragged onto another moves with everything under it, a click on the role
  // changes the role and a right-click says where its failures go. A placement the engine would refuse is refused here
  // with the engine's own reason before anything is stored, so the chart is always one the engine will load.
  import { createEventDispatcher, onMount } from "svelte";
  import {
    seatChange,
    workflowView,
    type SeatChange,
    type WorkflowRow,
    type WorkflowView,
  } from "../api/tauri";

  export let disabled = false;

  const dispatch = createEventDispatcher<{ changed: void }>();

  let view: WorkflowView | null = null;
  let loadError = "";
  let actionError = "";
  let busy = false;
  let unlocked = false;
  let folded: Record<string, boolean> = {};
  let dragged: string | null = null;
  let over: string | null = null;
  let menu: { seat: WorkflowRow; x: number; y: number } | null = null;

  const TOP = "main";
  const ROLES = ["lead", "worker", "reviewer"];
  const ROLE_ORDER: Record<string, number> = { lead: 0, worker: 1, reviewer: 2 };
  const ROLE_HINT = "Lead decides, worker changes code, reviewer only reads.";

  async function load() {
    try {
      view = await workflowView();
      loadError = "";
    } catch (error) {
      loadError = String(error);
    }
  }

  async function apply(row: WorkflowRow, change: SeatChange) {
    busy = true;
    actionError = "";
    try {
      await seatChange(row.name, change);
      dispatch("changed");
    } catch (error) {
      // The engine's reasons, for a placement that would break the tree; nothing was stored and the chart snaps back.
      actionError = String(error);
    } finally {
      await load();
      busy = false;
    }
  }

  function place(row: WorkflowRow, patch: Partial<{ parent: string; role: string; escalatesTo: string | null }>) {
    const next = { parent: row.parent, role: row.role, escalatesTo: row.escalatesTo, ...patch };
    return apply(row, {
      setting: "workflow",
      parent: next.parent,
      role: next.role,
      escalatesTo: next.escalatesTo === "" ? null : next.escalatesTo,
    });
  }

  $: seats = (view?.rows ?? []).filter((row) => !row.noPlaceReason);
  $: helpers = (view?.rows ?? []).filter((row) => row.noPlaceReason && row.name !== "btw");
  $: anyPlaced = seats.some((row) => row.placed);

  function kids(parent: string, rows: WorkflowRow[], closed: Record<string, boolean>): WorkflowRow[] {
    if (closed[parent]) return [];
    return rows
      .filter((row) => row.parent === parent)
      .sort((a, b) => (ROLE_ORDER[a.role] ?? 9) - (ROLE_ORDER[b.role] ?? 9) || a.name.localeCompare(b.name));
  }

  /** Whether `name` has anyone under it, folded or not. */
  function hasKids(name: string, rows: WorkflowRow[]): boolean {
    return rows.some((row) => row.parent === name);
  }

  /** Seats with nothing under them go in one column below their parent, so only branches take width. */
  function parts(parent: string, rows: WorkflowRow[], closed: Record<string, boolean>) {
    const children = kids(parent, rows, closed);
    return { branches: children.filter((row) => hasKids(row.name, rows)), leaves: children.filter((row) => !hasKids(row.name, rows)) };
  }

  function drop(target: string) {
    over = null;
    const row = seats.find((seat) => seat.name === dragged);
    dragged = null;
    if (row && row.name !== target && row.parent !== target) {
      place(row, { parent: target });
    }
  }

  function boxTitle(row: WorkflowRow): string {
    return [
      row.name,
      row.command ? `you type ${row.command}` : "",
      row.escalatesTo ? `if it fails, goes to ${row.escalatesTo}` : "",
      row.placed ? "changed by you" : "",
      row.noFile ? "it has no seat file, so it cannot be moved" : "",
    ].filter(Boolean).join(", ");
  }

  function nextRole(row: WorkflowRow) {
    if (unlocked && !row.noFile) {
      place(row, { role: ROLES[(ROLES.indexOf(row.role) + 1) % ROLES.length] });
    }
  }

  function openMenu(event: MouseEvent, row: WorkflowRow) {
    event.preventDefault();
    if (unlocked && !row.noFile) {
      menu = { seat: row, x: event.clientX, y: event.clientY };
    }
  }

  async function backToDefault() {
    for (const row of seats.filter((seat) => seat.placed)) {
      await apply(row, { setting: "workflowDefault" });
    }
  }

  function others(row: WorkflowRow): string[] {
    return (view?.places ?? []).filter((name) => name !== row.name);
  }

  onMount(load);
</script>

<svelte:window on:click={() => (menu = null)} />

{#snippet box(row: WorkflowRow)}
  <div
    class="box"
    class:changed={row.placed}
    class:over={over === row.name}
    draggable={unlocked && !row.noFile}
    role="treeitem"
    aria-selected="false"
    tabindex="-1"
    title={boxTitle(row)}
    oncontextmenu={(e) => openMenu(e, row)}
    ondragstart={() => (dragged = row.name)}
    ondragover={(e) => { if (unlocked && dragged) { e.preventDefault(); over = row.name; } }}
    ondragleave={() => (over = null)}
    ondrop={(e) => { e.preventDefault(); drop(row.name); }}
  >
    <span class="n">{row.name}</span>
    <button class="r" type="button" disabled={!unlocked || row.noFile || disabled || busy} title={ROLE_HINT} onclick={() => nextRole(row)}>{row.role}</button>
    {#if hasKids(row.name, seats)}
      <button class="fold" type="button" title="Show or hide the seats under this one." onclick={() => (folded = { ...folded, [row.name]: !folded[row.name] })}>{folded[row.name] ? "▸" : "▾"}</button>
    {/if}
  </div>
{/snippet}

{#snippet children(parent: string)}
  {@const below = parts(parent, seats, folded)}
  {#if below.branches.length > 0 || below.leaves.length > 0}
    <ul>
      {#each below.branches as row (row.name)}
        <li>
          {@render box(row)}
          {@render children(row.name)}
        </li>
      {/each}
      {#if below.leaves.length > 0}
        <li>
          <div class="stack">
            {#each below.leaves as row (row.name)}
              {@render box(row)}
            {/each}
          </div>
        </li>
      {/if}
    </ul>
  {/if}
{/snippet}

<!-- The app's own view skeleton (see SeatsView): `runtime-settings-root` is what makes the middle of the page scroll. -->
<section class="panel stack runtime-settings-root workflow-root">
  <div class="wf-head">
    <h2 title="Who hires whom. A seat sits under the seat that hires it, and only that seat can hire it.">Workflow</h2>
    <div class="wf-actions">
      {#if unlocked && anyPlaced}
        <button class="secondary" type="button" disabled={disabled || busy} title="Returns every seat to the default tree." onclick={backToDefault}>Back to default</button>
      {/if}
      <button class="secondary" class:unlocked type="button" disabled={disabled} title="Locked: the tree can only be read. Unlocked: drag a seat onto another to move it with everything under it, click a role to change it, right-click a seat for where its failures go." onclick={() => (unlocked = !unlocked)}>{unlocked ? "Unlocked" : "Locked"}</button>
    </div>
  </div>

  {#if loadError}
    <p class="project-error">{loadError}</p>
  {/if}
  {#if actionError}
    <p class="project-error">{actionError}</p>
  {/if}
  {#if view}
    {#each view.errors as error}
      <p class="project-error">{error}</p>
    {/each}
    {#each view.refusals as refusal}
      <p class="project-error" title="The engine refuses a tree that breaks a rule and uses the default one in its place.">{refusal}</p>
    {/each}

    <div class="org" class:unlocked role="tree">
      <ul>
        <li>
          <div class="root-wrap">
            <div
              class="box"
              class:over={over === TOP}
              role="treeitem"
              aria-selected="false"
              tabindex="-1"
              title="The principal: the seat you talk to. Nobody hires it."
              ondragover={(e) => { if (unlocked && dragged) { e.preventDefault(); over = TOP; } }}
              ondragleave={() => (over = null)}
              ondrop={(e) => { e.preventDefault(); drop(TOP); }}
            >
              <span class="n">{TOP}</span>
            </div>
            {#if helpers.length > 0}
              <div class="helpers-box">
                <span class="lbl" title="They are not hired. Each helps the principal, and each says what starts it.">Helpers</span>
                {#each helpers as row (row.name)}
                  <span class="h" title={row.noPlaceReason}>{row.command ?? row.name}</span>
                {/each}
              </div>
            {/if}
          </div>
          {@render children(TOP)}
        </li>
      </ul>
    </div>
  {/if}

  {#if menu}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="wf-menu" role="menu" tabindex="-1" style={`left:${menu.x}px;top:${menu.y}px`} onclick={(e) => e.stopPropagation()}>
      <div title="Where a task this seat could not do goes next.">If {menu.seat.name} fails, goes to</div>
      <select value={menu.seat.escalatesTo ?? ""} disabled={disabled || busy} onchange={(e) => { const seat = menu?.seat; menu = null; if (seat) place(seat, { escalatesTo: e.currentTarget.value }); }}>
        <option value="">nowhere</option>
        {#each others(menu.seat) as name (name)}
          <option value={name}>{name}</option>
        {/each}
      </select>
      {#if menu.seat.placed}
        <button class="secondary" type="button" title="Returns this seat to where the default tree puts it." onclick={() => { const seat = menu?.seat; menu = null; if (seat) apply(seat, { setting: "workflowDefault" }); }}>Back to default</button>
      {/if}
    </div>
  {/if}
</section>

<style>
  .wf-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .wf-head h2 {
    margin: 0;
  }
  .wf-actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  button.unlocked {
    border-color: var(--accent);
  }
  .org {
    padding: 0.4rem 0;
  }
  .org ul {
    display: flex;
    justify-content: center;
    list-style: none;
    margin: 0;
    padding: 12px 0 0;
    position: relative;
  }
  .org li {
    position: relative;
    padding: 12px 6px 0;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .org li::before,
  .org li::after {
    content: "";
    position: absolute;
    top: 0;
    right: 50%;
    width: 50%;
    height: 12px;
    border-top: 1px solid var(--line);
  }
  .org li::after {
    right: auto;
    left: 50%;
    border-left: 1px solid var(--line);
  }
  .org li:only-child::before,
  .org li:only-child::after {
    display: none;
  }
  .org li:only-child {
    padding-top: 0;
  }
  .org li:first-child::before,
  .org li:last-child::after {
    border: 0 none;
  }
  .org li:last-child::before {
    border-right: 1px solid var(--line);
  }
  .org ul ul::before {
    content: "";
    position: absolute;
    top: 0;
    left: 50%;
    height: 12px;
    border-left: 1px solid var(--line);
  }
  .org > ul {
    padding-top: 0;
  }
  .org > ul > li {
    padding-top: 0;
  }
  .org > ul > li::before,
  .org > ul > li::after {
    display: none;
  }
  .box {
    position: relative;
    width: 172px;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: var(--panel);
    padding: 2px 16px 2px 7px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    line-height: 1.3;
  }
  .n {
    font-weight: 600;
    font-size: 11.5px;
    white-space: nowrap;
  }
  .r {
    border: 1px solid var(--line);
    border-radius: 999px;
    padding: 0 6px;
    font-size: 10px;
    background: transparent;
    color: inherit;
    cursor: default;
  }
  .org.unlocked .r:not(:disabled) {
    cursor: pointer;
  }
  .fold {
    position: absolute;
    top: 2px;
    right: 3px;
    border: 0;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    font-size: 10px;
    padding: 0;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .changed {
    border-color: var(--accent);
  }
  .org.unlocked .box[draggable="true"] {
    cursor: grab;
  }
  .over {
    outline: 2px solid var(--accent);
  }
  .root-wrap {
    position: relative;
  }
  .helpers-box {
    position: absolute;
    left: calc(100% + 12px);
    top: 0;
    display: flex;
    gap: 5px;
    align-items: center;
  }
  .h {
    border: 1px dashed var(--line);
    border-radius: 6px;
    padding: 2px 7px;
    font-size: 11px;
    white-space: nowrap;
    cursor: help;
  }
  .lbl {
    color: var(--muted);
    font-size: 11px;
  }
  .wf-menu {
    position: fixed;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 7px;
    font-size: 12px;
    z-index: 9;
  }
  .wf-menu select {
    display: block;
    margin: 4px 0 7px;
  }
</style>
