<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { answerEditorUpdate, editorUpdatePrompts, type EditorUpdatePrompt } from "../api/tauri";
  import { describePrompt, nextPrompt } from "../editorPrompt";

  // The question the studio asks when an editor's folders match a jawata workspace only partly
  // (Sprint 28h Stage 21 (j)). Mounted once in App. The studio changes nothing until a button is pressed.
  let open: EditorUpdatePrompt[] = [];
  let answering = false;
  let failure = "";
  let unlisten: (() => void) | undefined;

  $: prompt = nextPrompt(open);
  $: view = prompt ? describePrompt(prompt) : null;

  async function refresh() {
    try {
      open = await editorUpdatePrompts();
    } catch (error) {
      failure = String(error);
    }
  }

  async function answer(update: boolean) {
    if (!prompt || answering) return;
    answering = true;
    failure = "";
    try {
      await answerEditorUpdate(prompt.id, update);
    } catch (error) {
      failure = String(error);
    } finally {
      answering = false;
      await refresh();
    }
  }

  let destroyed = false;

  /**
   * The listener is registered BEFORE the first read, so a question that changes between the two is not lost: the
   * event refreshes the list, and the read that follows sees the same list or a newer one. (A launch can ask before
   * this window has loaded, so the questions already open are read too.)
   */
  async function follow() {
    try {
      const off = await listen("jawata://editor-prompts-changed", () => void refresh());
      if (destroyed) {
        off();
      } else {
        unlisten = off;
      }
    } catch (error) {
      failure = String(error);
    }
    await refresh();
  }

  onMount(() => {
    void follow();
  });

  onDestroy(() => {
    destroyed = true;
    unlisten?.();
  });
</script>

{#if prompt && view}
  <div class="editor-backdrop" role="presentation">
    <div class="editor-card" role="alertdialog" aria-modal="true" aria-label={view.title} tabindex="-1">
      <h2>{view.title}</h2>
      <p class="editor-question">{view.question}</p>
      {#each view.sections as section}
        <div class="editor-section">
          <h3>{section.heading}</h3>
          <ul>
            {#each section.paths as path}
              <li><code>{path}</code></li>
            {/each}
          </ul>
        </div>
      {/each}
      {#if failure}
        <p class="editor-failure">{failure}</p>
      {/if}
      <div class="editor-actions">
        <button class="btn-decline" on:click={() => answer(false)} disabled={answering}>{view.decline}</button>
        <button class="btn-update" on:click={() => answer(true)} disabled={answering}>{view.update}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .editor-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(var(--shade-rgb), 0.75);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2100;
  }

  .editor-card {
    width: 100%;
    max-width: 560px;
    max-height: 80vh;
    overflow: auto;
    background: rgba(var(--panel-rgb), 0.97);
    border: 1px solid rgba(var(--line-rgb), 0.2);
    border-radius: 12px;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.45);
    padding: 1.5rem;
  }

  h2 {
    margin: 0 0 0.75rem;
    font-size: 1.15rem;
    color: var(--text-strong);
  }

  h3 {
    margin: 0.9rem 0 0.3rem;
    font-size: 0.85rem;
    color: var(--muted);
    font-weight: 600;
  }

  .editor-question {
    margin: 0;
    line-height: 1.55;
    color: var(--text-soft);
  }

  ul {
    margin: 0;
    padding-left: 1.1rem;
    color: var(--text-soft);
    font-size: 0.85rem;
  }

  .editor-failure {
    margin: 0.9rem 0 0;
    color: var(--danger-soft);
    font-size: 0.85rem;
  }

  .editor-actions {
    margin-top: 1.5rem;
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
  }

  .editor-actions button {
    padding: 0.5rem 1.15rem;
    border-radius: 8px;
    font-size: 0.9rem;
    border: 1px solid transparent;
  }

  .btn-decline {
    background: transparent;
    border-color: rgba(var(--line-rgb), 0.3);
    color: var(--text-soft);
  }

  .btn-decline:hover:not(:disabled) {
    background: rgba(var(--line-rgb), 0.12);
    color: var(--text-strong);
  }

  .btn-update {
    background: var(--accent-strong);
    color: var(--text-strong);
  }

  .btn-update:hover:not(:disabled) {
    background: var(--accent-deep);
  }

  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
