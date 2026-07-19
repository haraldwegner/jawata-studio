<script lang="ts">
  import { confirmRequest } from "../dialog";

  // The single confirmation modal for the whole app (mounted once in App).
  // Driven by the confirmRequest store; see lib/dialog.ts.
  $: request = $confirmRequest;

  function settle(ok: boolean) {
    const current = $confirmRequest;
    confirmRequest.set(null);
    current?.resolve(ok);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!$confirmRequest) return;
    // Esc cancels. Deliberately NO Enter-to-confirm: these are destructive
    // actions, so OK must be an explicit click (no accidental delete).
    if (event.key === "Escape") {
      event.preventDefault();
      settle(false);
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

{#if request}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <div class="confirm-backdrop" on:click={() => settle(false)} role="presentation">
    <div
      class="confirm-card"
      on:click|stopPropagation
      role="alertdialog"
      aria-modal="true"
      aria-label={request.title}
      tabindex="-1"
    >
      <div class="confirm-header">
        <span class="warn-icon" aria-hidden="true">!</span>
        <h2>{request.title}</h2>
      </div>
      <p class="confirm-message">{request.message}</p>
      <div class="confirm-actions">
        <!-- svelte-ignore a11y-autofocus -->
        <button class="btn-cancel" on:click={() => settle(false)} autofocus>
          Cancel
        </button>
        <button class="btn-confirm" on:click={() => settle(true)}>OK</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .confirm-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(2, 6, 23, 0.75);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }

  .confirm-card {
    width: 100%;
    max-width: 460px;
    background: rgba(15, 23, 42, 0.97);
    border: 1px solid rgba(148, 163, 184, 0.2);
    border-radius: 12px;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.45);
    padding: 1.5rem;
  }

  .confirm-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
  }

  .confirm-header h2 {
    margin: 0;
    font-size: 1.15rem;
    color: #f8fafc;
  }

  .warn-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    flex: none;
    border-radius: 50%;
    background: rgba(251, 191, 36, 0.15);
    color: #fbbf24;
    font-weight: 700;
  }

  .confirm-message {
    margin: 0;
    line-height: 1.55;
    color: #cbd5e1;
    white-space: pre-line;
  }

  .confirm-actions {
    margin-top: 1.5rem;
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
  }

  .confirm-actions button {
    padding: 0.5rem 1.15rem;
    border-radius: 8px;
    font-size: 0.9rem;
    border: 1px solid transparent;
  }

  .btn-cancel {
    background: transparent;
    border-color: rgba(148, 163, 184, 0.3);
    color: #cbd5e1;
  }

  .btn-cancel:hover {
    background: rgba(148, 163, 184, 0.12);
    color: #f8fafc;
  }

  .btn-confirm {
    background: #2563eb;
    color: #ffffff;
  }

  .btn-confirm:hover {
    background: #1d4ed8;
  }
</style>
