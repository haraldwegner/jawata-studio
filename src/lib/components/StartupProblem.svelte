<script lang="ts">
  // Drawn instead of the app when the studio could not read its own files; see src-tauri/src/startup_problem.rs.
  // Nothing to press: the person repairs or moves the file and starts the studio again.
  export let problem: string;
</script>

<main class="app-shell">
  <section class="panel" role="alert">
    <h1>jawata-studio could not read its settings</h1>
    <p class="reason">{problem}</p>
    <p>Nothing was changed: the file is as you left it, and nothing was deployed or started.</p>
    <p>
      Repair the file named above, or move it out of its folder so that jawata-studio writes a new one with default
      values. Then close this window and start jawata-studio again.
    </p>
  </section>
</main>

<style>
  .reason {
    font-family: ui-monospace, monospace;
    color: var(--danger-soft);
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
