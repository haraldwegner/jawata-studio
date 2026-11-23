<script lang="ts">
  // 28h z57. One small bar: the version each RUNNING engine reports about itself — each workspace is its own engine process — and a line
  // of its own only where one differs from the installed version. The header's "JAWATA 4.6.x" is the version on disk; this is what the
  // processes say, which differs when an engine was started before an update finished and keeps running the old one.
  import { onDestroy, onMount } from "svelte";
  import { engineVersions } from "../api/tauri";
  import { engineBar, type EngineVersionRow } from "../engineBar";

  /** The installed version, or null for a hand-picked jar (whose version the studio cannot know). */
  export let installed: string | null = null;

  const REFRESH_MS = 20_000;
  let rows: EngineVersionRow[] = [];
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refresh() {
    try {
      rows = await engineVersions();
    } catch {
      // The bar is information only: a failed read leaves the last answer, and a window with no engine running shows nothing.
    }
  }

  onMount(() => {
    void refresh();
    timer = setInterval(() => void refresh(), REFRESH_MS);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  $: bar = engineBar(rows, installed);
</script>

{#if bar.summary}
  <footer
    class="engine-bar muted"
    title="The version each running engine reports about itself (every workspace is its own engine process). The header shows the installed version; they differ when an engine was started before an update finished."
  >
    <span>{bar.summary}</span>
    {#each bar.mismatches as line (line)}
      <span class="engine-bar-warn">{line}</span>
    {/each}
  </footer>
{/if}

<style>
  .engine-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 1rem;
    font-size: 0.75rem;
    padding: 0.1rem 0.25rem;
  }
  .engine-bar-warn {
    color: var(--danger, #c0392b);
  }
</style>
