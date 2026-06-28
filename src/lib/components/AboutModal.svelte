<script lang="ts">
  import { createEventDispatcher, onMount } from "svelte";
  import { formatManagerVersionForUi, getManagerAppVersion } from "../appVersion";

  const dispatch = createEventDispatcher<{
    close: void;
  }>();

  let appVersion = "";

  onMount(() => {
    void getManagerAppVersion().then((v) => {
      appVersion = v;
    });
  });

  function handleClose() {
    dispatch("close");
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      handleClose();
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="modal-backdrop" on:click={handleClose} role="presentation">
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <div class="modal-content panel stack" on:click|stopPropagation role="dialog" aria-modal="true" tabindex="-1">
    <div class="modal-header">
      <h2>About goja-studio</h2>
      <button class="close-button" on:click={handleClose} aria-label="Close modal">×</button>
    </div>

    <div class="modal-body stack">
      <div class="about-hero">
        <div class="app-title">goja-studio</div>
        <div class="app-version muted">
          {#if appVersion}
            Version {formatManagerVersionForUi(appVersion)}
          {:else}
            Loading version…
          {/if}
        </div>
      </div>

      <div class="about-section">
        <h3>Author</h3>
        <p>Created by <strong>Harald Wegner</strong>.</p>
      </div>

      <div class="about-section">
        <h3>License</h3>
        <p>Released under the <strong>MIT License</strong>.</p>
      </div>

      <div class="about-section">
        <h3>Credits & Acknowledgements</h3>
        <ul class="credits-list">
          <li>
            <strong>goja-mcp</strong><br />
            Explicit credit to <strong>P. Zalutski</strong> for the upstream GOJA MCP project.
          </li>
          <li>
            <strong>Open Source Dependencies</strong><br />
            Built with <a href="https://tauri.app" target="_blank" rel="noopener noreferrer">Tauri</a>,
            <a href="https://svelte.dev" target="_blank" rel="noopener noreferrer">Svelte</a>,
            and <a href="https://rust-lang.org" target="_blank" rel="noopener noreferrer">Rust</a>.
          </li>
        </ul>
      </div>
    </div>

    <div class="modal-footer">
      <button class="primary" on:click={handleClose}>Close</button>
    </div>
  </div>
</div>

<style>
  .modal-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(2, 6, 23, 0.75);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal-content {
    width: 100%;
    max-width: 480px;
    max-height: 90vh;
    overflow-y: auto;
    background: rgba(15, 23, 42, 0.95);
    border: 1px solid rgba(148, 163, 184, 0.2);
    border-radius: 12px;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.4);
    padding: 1.5rem;
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid rgba(148, 163, 184, 0.1);
  }

  .modal-header h2 {
    margin: 0;
    font-size: 1.25rem;
  }

  .close-button {
    background: transparent;
    border: none;
    font-size: 1.5rem;
    line-height: 1;
    padding: 0 0.5rem;
    color: #94a3b8;
    cursor: pointer;
  }

  .close-button:hover {
    color: #f8fafc;
  }

  .about-hero {
    text-align: center;
    padding: 1.5rem 0;
    background: rgba(30, 41, 59, 0.5);
    border-radius: 8px;
    margin-bottom: 1rem;
  }

  .app-title {
    font-size: 1.5rem;
    font-weight: 600;
    color: #f8fafc;
    margin-bottom: 0.25rem;
  }

  .app-version {
    font-size: 0.875rem;
  }

  .about-section {
    margin-bottom: 1.25rem;
  }

  .about-section h3 {
    font-size: 0.875rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #94a3b8;
    margin-bottom: 0.5rem;
  }

  .about-section p {
    margin: 0;
    line-height: 1.5;
  }

  .credits-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .credits-list li {
    margin-bottom: 0.75rem;
    line-height: 1.4;
  }

  .credits-list li:last-child {
    margin-bottom: 0;
  }

  .credits-list a {
    color: #60a5fa;
    text-decoration: none;
  }

  .credits-list a:hover {
    text-decoration: underline;
  }

  .modal-footer {
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid rgba(148, 163, 184, 0.1);
    display: flex;
    justify-content: flex-end;
  }
</style>
