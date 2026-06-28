<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  /** Anchored context menu. Pass screen-space x/y on open and a list
   * of items; the component handles outside-click + Escape to close.
   * Callers define their own item shape (label / onSelect / optional
   * danger + disabled flags); the type is re-declared at each callsite
   * because Svelte instance scripts can't export types — moving it to
   * `<script context="module">` would isolate it from the runtime
   * state. The duplication is two lines per consumer; cheap. */
  type Item = {
    label: string;
    onSelect: () => void;
    danger?: boolean;
    disabled?: boolean;
  };

  export let x: number;
  export let y: number;
  export let items: Item[];
  export let onClose: () => void;

  let menuEl: HTMLDivElement | null = null;

  function handleDocumentClick(event: MouseEvent) {
    if (menuEl && !menuEl.contains(event.target as Node)) {
      onClose();
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  }

  onMount(() => {
    // Defer document listener by one tick so the click that opened the
    // menu doesn't immediately close it.
    setTimeout(() => {
      document.addEventListener("mousedown", handleDocumentClick);
    }, 0);
    document.addEventListener("keydown", handleKeydown);
  });

  onDestroy(() => {
    document.removeEventListener("mousedown", handleDocumentClick);
    document.removeEventListener("keydown", handleKeydown);
  });

  function handleItemClick(item: Item) {
    if (item.disabled) return;
    item.onSelect();
    onClose();
  }
</script>

<div bind:this={menuEl} class="context-menu" style={`left: ${x}px; top: ${y}px;`} role="menu">
  {#each items as item}
    <button
      class:danger={item.danger}
      class="context-menu-item"
      disabled={item.disabled}
      on:click={() => handleItemClick(item)}
      role="menuitem"
      type="button"
    >
      {item.label}
    </button>
  {/each}
</div>
