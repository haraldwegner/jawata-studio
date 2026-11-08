<script lang="ts">
  // Sprint 28h, S18a (D51): the workflow view — who hires whom.
  //
  // Harald, 2026-11-03: *"There could be slight changes. But the core idea stays only the workers
  // work."* · 2026-11-06: *"There is a new view in studio!"*
  //
  // What a person decides here, and nothing else: which seat a seat reports to, what the seat is
  // (lead, worker, reviewer) and where a task it could not do goes next. Each is a choice with
  // a mouseover saying what it changes. A placement the engine would refuse is refused here with the
  // engine's own reason before anything is stored, so the tree on screen is always one the engine
  // will load. Who is working right now is the chat's view (28i), not this one.
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

  const ROLES: { value: string; label: string; hint: string }[] = [
    { value: "lead", label: "lead", hint: "Decides and instructs. A lead never changes code and holds no tool that does." },
    { value: "worker", label: "worker", hint: "Changes code through the engine's tools, as instructed. Only a worker may." },
    { value: "reviewer", label: "reviewer", hint: "Reads and proposes. Changes nothing." },
  ];

  const REPORTS_TO_HINT =
    "The seat that hires this one. Only that seat can hire it. Changing it moves the seat, and everything under it, in the tree.";
  const ROLE_HINT =
    "What the seat is: a lead decides, a worker changes code, a reviewer only reads. A seat that is not a worker may not hold a tool that changes code, and the engine refuses a tree that lets one.";
  const ESCALATES_HINT =
    "Where a task this seat could not do goes next, for example a developer that fails goes to the senior developer. Nowhere means the failure comes back to whoever hired it.";

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
      await load();
      dispatch("changed");
    } catch (error) {
      // The engine's reasons, for a placement that would break the tree; nothing was stored.
      actionError = String(error);
      await load();
    } finally {
      busy = false;
    }
  }

  function place(row: WorkflowRow, patch: Partial<{ parent: string; role: string; escalatesTo: string | null }>) {
    const next = {
      parent: row.parent,
      role: row.role,
      escalatesTo: row.escalatesTo,
      ...patch,
    };
    return apply(row, {
      setting: "workflow",
      parent: next.parent,
      role: next.role,
      escalatesTo: next.escalatesTo === "" ? null : next.escalatesTo,
    });
  }

  /** A seat outside the tree: the choices are held here until both a parent and a role are picked, because the
   *  engine refuses a parent without a role and a role without a parent, and one select cannot carry both. */
  let drafts: Record<string, { parent: string; role: string }> = {};

  function draftOf(row: WorkflowRow) {
    return drafts[row.name] ?? { parent: "", role: "" };
  }

  function setDraft(row: WorkflowRow, patch: Partial<{ parent: string; role: string }>) {
    drafts = { ...drafts, [row.name]: { ...draftOf(row), ...patch } };
  }

  async function placeDraft(row: WorkflowRow) {
    const draft = draftOf(row);
    await apply(row, { setting: "workflow", parent: draft.parent, role: draft.role, escalatesTo: null });
    if (!actionError) {
      const { [row.name]: _placed, ...rest } = drafts;
      drafts = rest;
    }
  }

  function others(row: WorkflowRow): string[] {
    return (view?.places ?? []).filter((name) => name !== row.name);
  }

  onMount(load);
</script>

<!-- The app's own view skeleton (see SeatsView): `runtime-settings-root` is what makes the middle
     of the page scroll, and the rows are `settings-section` panels in the shared grid. -->
<section class="panel stack runtime-settings-root workflow-root">
  <h2>Workflow</h2>
  {#if view}
    <p
      class="summary-line"
      title="Who hires whom. The tree starts as the signed one; a seat you place keeps your place until you take it back. Who is working right now is shown in the chat."
    >
      <strong>{view.rows.filter((row) => row.placed).length}</strong> of {view.rows.length} seats placed by you
    </p>
  {/if}

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
      <p class="project-error" title="The engine refuses a tree that breaks a rule and uses the signed one in its place.">
        {refusal}
      </p>
    {/each}

    <div class="settings-grid">
      {#each view.rows as row (row.name)}
        <section class="settings-section workflow-row">
          <h3 title="The seat, with what you type to call it.">
            {row.name}{#if row.command} <span class="seat-command">({row.command})</span>{/if}
          </h3>
          {#if row.noPlaceReason}
            <!-- The signed tree has no place for this seat. It says why, instead of two empty fields. -->
            <p class="hint workflow-no-place" title="The engine's signed tree does not place this seat. Choose a parent and a role below to put it in the tree.">
              No place in the tree. {row.noPlaceReason}
            </p>
            <label class="workflow-field" title={REPORTS_TO_HINT}>
              <span>Reports to</span>
              <select
                value={draftOf(row).parent}
                disabled={disabled || busy}
                on:change={(e) => setDraft(row, { parent: e.currentTarget.value })}
              >
                <option value="">(none)</option>
                {#each others(row) as name (name)}
                  <option value={name}>{name}</option>
                {/each}
              </select>
            </label>
            <label class="workflow-field" title={ROLE_HINT}>
              <span>Is a</span>
              <select
                value={draftOf(row).role}
                disabled={disabled || busy}
                on:change={(e) => setDraft(row, { role: e.currentTarget.value })}
              >
                <option value="">(none)</option>
                {#each ROLES as role (role.value)}
                  <option value={role.value} title={role.hint}>{role.label}</option>
                {/each}
              </select>
            </label>
            <button
              class="secondary"
              type="button"
              disabled={disabled || busy || !draftOf(row).parent || !draftOf(row).role}
              title="Put this seat in the tree under the chosen seat, as the chosen kind. The engine checks the result before it is stored."
              on:click={() => placeDraft(row)}
            >
              Put in the tree
            </button>
          {:else}
          <label class="workflow-field" title={REPORTS_TO_HINT}>
            <span>Reports to</span>
            <select
              value={row.parent}
              disabled={disabled || busy}
              on:change={(e) => place(row, { parent: e.currentTarget.value })}
            >
              {#each others(row) as name (name)}
                <option value={name}>{name}</option>
              {/each}
            </select>
          </label>
          <label class="workflow-field" title={ROLE_HINT}>
            <span>Is a</span>
            <select
              value={row.role}
              disabled={disabled || busy}
              on:change={(e) => place(row, { role: e.currentTarget.value })}
            >
              {#each ROLES as role (role.value)}
                <option value={role.value} title={role.hint}>{role.label}</option>
              {/each}
            </select>
          </label>
          <label class="workflow-field" title={ESCALATES_HINT}>
            <span>If it fails, goes to</span>
            <select
              value={row.escalatesTo ?? ""}
              disabled={disabled || busy}
              on:change={(e) => place(row, { escalatesTo: e.currentTarget.value })}
            >
              <option value="">nowhere</option>
              {#each others(row) as name (name)}
                <option value={name}>{name}</option>
              {/each}
            </select>
          </label>
          {/if}
          {#if row.placed}
            <button
              class="secondary"
              type="button"
              disabled={disabled || busy}
              title={row.signed
                ? `Back to the signed place: reports to ${row.signed.parent}, a ${row.signed.role}.`
                : "Remove your placement. The signed tree does not know this seat, so it leaves the tree."}
              on:click={() => apply(row, { setting: "workflowDefault" })}
            >
              Take back
            </button>
          {/if}
        </section>
      {/each}
    </div>
  {/if}
</section>

<style>
  /* The one local style: a labelled select per line, the shared sheet has no label/value row. */
  .workflow-field {
    display: grid;
    grid-template-columns: 9rem 1fr;
    align-items: center;
    gap: 0.5rem;
    margin: 0.25rem 0;
  }
  .workflow-field select {
    width: 100%;
  }
  .workflow-row h3 {
    margin-bottom: 0.4rem;
  }
</style>
