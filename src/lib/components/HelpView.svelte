<script lang="ts">
  import { onMount } from "svelte";
  import { marked } from "marked";
  import AboutModal from "./AboutModal.svelte";
  import helpContent from "../../assets/help.md?raw";

  let parsedContent = "";
  let showAboutModal = false;

  onMount(() => {
    parsedContent = marked.parse(helpContent) as string;
  });

  function openAboutModal() {
    showAboutModal = true;
  }

  function closeAboutModal() {
    showAboutModal = false;
  }
</script>

<section class="panel stack help-view-root">
  <div class="help-header">
    <h2>Help & Documentation</h2>
    <p class="muted">Learn how to configure and deploy goja-studio.</p>
  </div>

  <div class="markdown-body">
    {@html parsedContent}
  </div>
</section>

<div class="panel help-footer">
  <a href="https://github.com/haraldwegner/goja-studio/releases" target="_blank" rel="noopener noreferrer" class="muted" style="margin-right: auto; text-decoration: underline;">
    Check for goja-studio updates
  </a>
  <button class="primary about-button" on:click={openAboutModal} type="button">
    About
  </button>
</div>

{#if showAboutModal}
  <AboutModal on:close={closeAboutModal} />
{/if}

<style>
  .help-view-root {
    overflow: auto;
    min-height: 0;
  }

  .help-header {
    margin-bottom: 1.5rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid rgba(148, 163, 184, 0.15);
  }

  .markdown-body {
    flex: 1;
    color: #e2e8f0;
    line-height: 1.6;
    font-size: 0.95rem;
    max-width: 800px;
    margin: 0 auto;
    width: 100%;
  }

  /* Markdown Typography */
  :global(.markdown-body h1) {
    font-size: 1.8rem;
    font-weight: 600;
    margin-top: 0;
    margin-bottom: 1rem;
    color: #f8fafc;
  }

  :global(.markdown-body h2) {
    font-size: 1.4rem;
    font-weight: 600;
    margin-top: 2rem;
    margin-bottom: 0.75rem;
    color: #f8fafc;
    border-bottom: 1px solid rgba(148, 163, 184, 0.15);
    padding-bottom: 0.3rem;
  }

  :global(.markdown-body h3) {
    font-size: 1.2rem;
    font-weight: 600;
    margin-top: 1.5rem;
    margin-bottom: 0.5rem;
    color: #e2e8f0;
  }

  :global(.markdown-body p) {
    margin-top: 0;
    margin-bottom: 1rem;
  }

  :global(.markdown-body ul) {
    margin-top: 0;
    margin-bottom: 1rem;
    padding-left: 1.5rem;
  }

  :global(.markdown-body li) {
    margin-bottom: 0.25rem;
  }

  :global(.markdown-body strong) {
    font-weight: 600;
    color: #f8fafc;
  }

  :global(.markdown-body em) {
    font-style: italic;
  }

  :global(.markdown-body a) {
    color: #60a5fa;
    text-decoration: none;
  }

  :global(.markdown-body a:hover) {
    text-decoration: underline;
  }

  :global(.markdown-body code) {
    background: rgba(15, 23, 42, 0.6);
    padding: 0.2em 0.4em;
    border-radius: 4px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
    font-size: 0.85em;
    color: #cbd5e1;
  }

  :global(.markdown-body pre) {
    background: rgba(15, 23, 42, 0.8);
    padding: 1rem;
    border-radius: 8px;
    overflow-x: auto;
    margin-bottom: 1rem;
    border: 1px solid rgba(148, 163, 184, 0.15);
  }

  :global(.markdown-body pre code) {
    background: transparent;
    padding: 0;
    color: inherit;
  }

  :global(.markdown-body blockquote) {
    margin: 0 0 1rem 0;
    padding: 0.5rem 1rem;
    border-left: 4px solid #3b82f6;
    background: rgba(59, 130, 246, 0.1);
    color: #cbd5e1;
    border-radius: 0 4px 4px 0;
  }

  :global(.markdown-body table) {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: 1rem;
    font-size: 0.9rem;
  }

  :global(.markdown-body th),
  :global(.markdown-body td) {
    border: 1px solid rgba(148, 163, 184, 0.22);
    padding: 0.45rem 0.65rem;
    text-align: left;
    vertical-align: top;
  }

  :global(.markdown-body th) {
    background: rgba(15, 23, 42, 0.5);
    color: #f8fafc;
    font-weight: 600;
  }

  :global(.markdown-body hr) {
    border: none;
    border-top: 1px solid rgba(148, 163, 184, 0.2);
    margin: 1.5rem 0;
  }

  :global(.markdown-body img) {
    display: block;
    max-width: 100%;
    height: auto;
    margin: 1rem auto;
    border-radius: 8px;
    border: 1px solid rgba(148, 163, 184, 0.22);
    box-shadow: 0 4px 18px rgba(0, 0, 0, 0.22);
  }

  /* Footer */
  .help-footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    padding: 0.58rem 0.82rem;
    border-top: 1px solid rgba(148, 163, 184, 0.22);
  }

  .about-button {
    min-width: 9.25rem;
    transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
  }
</style>
