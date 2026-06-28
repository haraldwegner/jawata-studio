<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { createEventDispatcher, onMount } from "svelte";
  import {
    discoverWorkspaceProjects,
    importWorkspaceProjects,
    scanFolderForProjects,
    type AddProjectInput,
    type WorkspaceProjectCandidate
  } from "../api/tauri";

  export let disabled = false;
  /** Sprint 10 v0.10.4: the workspace this form's submit will add the
   * project to. Owned by the parent (App.svelte) and shared with the
   * Workspaces card on the left so that picking a workspace there
   * routes new projects (and imports) to it. Empty string = no
   * workspace selected yet. */
  export let activeWorkspaceName: string = "";

  const dispatch = createEventDispatcher<{
    submit: AddProjectInput;
    imported: void;
  }>();

  let name = "";
  let projectPath = "";
  let lastSuggestedName = "";
  let workspaceFile = "";
  let candidates: WorkspaceProjectCandidate[] = [];
  let selectedPaths: string[] = [];
  let importMessage = "";
  let isImporting = false;
  let lastDiscoveredFile = "";

  /* Sprint 16 autoscan: one checkbox under Project path. Checked, the
   * form flips to discovery — Browse autoscans the picked folder, the
   * submit button relabels to "Discover" (hand-typed paths / rescans),
   * and results unfold into the shared candidate list below. The
   * candidate/selection/import machinery is shared with the VSCode
   * workspace flow; candidateSource records which flow filled it so the
   * two never fight. */
  let autoscan = false;
  let isScanning = false;
  let scanMessage = "";
  let scannedFolder = "";
  let candidateSource: "" | "folder" | "workspace" = "";

  $: canDiscover =
    !disabled &&
    !isImporting &&
    workspaceFile.trim().length > 0 &&
    workspaceFile.trim() !== lastDiscoveredFile;

  $: canImportSelected =
    !disabled && !isImporting && selectedPaths.length > 0 && activeWorkspaceName.length > 0;

  $: canSubmit =
    name.trim().length > 0 &&
    projectPath.trim().length > 0 &&
    activeWorkspaceName.length > 0;

  // Sprint 16.1 (bugs.md #19): the current path is "already scanned" when
  // folder results are showing for exactly this path. Discover hides then;
  // it returns if the path is edited (≠ scannedFolder) or while scanning.
  $: currentPathScanned =
    candidateSource === "folder" &&
    candidates.length > 0 &&
    projectPath.trim() === scannedFolder;
  $: showDiscover = isScanning || !currentPathScanned;

  $: canScan =
    !disabled &&
    !isScanning &&
    !isImporting &&
    projectPath.trim().length > 0 &&
    activeWorkspaceName.length > 0;

  onMount(() => {
    /* no-op */
  });

  function inferNameFromPath(path: string): string {
    const trimmed = path.trim().replace(/[\\/]+$/, "");
    if (!trimmed) {
      return "";
    }

    const parts = trimmed.split(/[\\/]/);
    return parts[parts.length - 1] ?? "";
  }

  function maybeAdoptSuggestedName(projectFolderName: string) {
    if (!projectFolderName) {
      return;
    }

    if (!name.trim() || name.trim() === lastSuggestedName) {
      name = projectFolderName;
      lastSuggestedName = projectFolderName;
    }
  }

  async function chooseProjectFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: autoscan
        ? "Select folder to scan for Java projects"
        : "Select Java project folder"
    });

    if (typeof selected === "string") {
      projectPath = selected;
      if (autoscan) {
        // Autoscan-on-Browse: picking the folder IS the trigger in the
        // common case; the Discover button covers hand-typed paths.
        await scanProjectFolder();
      } else {
        maybeAdoptSuggestedName(inferNameFromPath(selected));
      }
    }
  }

  /* Sprint 16: scan the Project path folder for Java projects and unfold
   * the results into the shared candidate list. */
  async function scanProjectFolder() {
    const folder = projectPath.trim();
    if (!folder) {
      scanMessage = "Enter or browse a folder to scan first.";
      return;
    }
    isScanning = true;
    scanMessage = "";
    importMessage = "";
    candidates = [];
    selectedPaths = [];
    try {
      candidates = await scanFolderForProjects(folder);
      selectedPaths = candidates.map((candidate) => candidate.projectPath);
      candidateSource = "folder";
      scannedFolder = folder;
      if (candidates.length === 0) {
        scanMessage = `No Java projects found under ${folder}.`;
      }
    } catch (error) {
      scanMessage = String(error);
      candidateSource = "";
      scannedFolder = "";
    } finally {
      isScanning = false;
    }
  }

  function toggleAutoscan() {
    autoscan = !autoscan;
    scanMessage = "";
    if (!autoscan && candidateSource === "folder") {
      // Leaving autoscan mode: retire the folder-sourced results so the
      // single-project flow returns exactly to its pre-checkbox state.
      candidates = [];
      selectedPaths = [];
      candidateSource = "";
      scannedFolder = "";
    }
  }

  async function chooseWorkspaceFile() {
    const selected = await open({
      directory: false,
      multiple: false,
      title: "Select VSCode workspace file",
      filters: [{ name: "VSCode Workspace", extensions: ["code-workspace"] }]
    });
    if (typeof selected === "string") {
      workspaceFile = selected;
      // Reset the "already discovered this file" guard so Discover re-enables
      // even if the user re-picks the same file (e.g. dialog double-click).
      lastDiscoveredFile = "";
    }
  }

  async function discoverFromWorkspace() {
    importMessage = "";
    scanMessage = "";
    const path = workspaceFile.trim();
    if (!path) {
      importMessage = "Choose a .code-workspace file first.";
      return;
    }
    try {
      candidates = await discoverWorkspaceProjects(path);
      selectedPaths = candidates.map((candidate) => candidate.projectPath);
      candidateSource = "workspace";
      scannedFolder = "";
      lastDiscoveredFile = path;
      if (candidates.length === 0) {
        importMessage = "No Maven/Gradle or Eclipse/PDE Java projects found.";
      }
    } catch (error) {
      importMessage = String(error);
    }
  }

  function toggleCandidate(path: string) {
    if (selectedPaths.includes(path)) {
      selectedPaths = selectedPaths.filter((value) => value !== path);
    } else {
      selectedPaths = [...selectedPaths, path];
    }
  }

  async function importSelected() {
    const fromFolder = candidateSource === "folder";
    const source = fromFolder ? scannedFolder : workspaceFile.trim();
    if (!source || selectedPaths.length === 0) {
      importMessage = "Select at least one discovered project.";
      return;
    }
    isImporting = true;
    importMessage = "";
    scanMessage = "";
    try {
      const result = await importWorkspaceProjects({
        workspaceFile: fromFolder ? "" : source,
        scanFolder: fromFolder ? source : "",
        selectedPaths,
        workspaceName: activeWorkspaceName
      });
      let message = `Imported ${result.added.length} project(s).`;
      if (result.skipped.length > 0) {
        message += ` Skipped ${result.skipped.length}.`;
      }
      candidates = [];
      selectedPaths = [];
      candidateSource = "";
      scannedFolder = "";
      // Return both sources to their initial empty state so the buttons
      // grey out and the form is ready for the next operation.
      workspaceFile = "";
      lastDiscoveredFile = "";
      if (fromFolder) {
        projectPath = "";
        scanMessage = message;
      } else {
        importMessage = message;
      }
      dispatch("imported");
    } catch (error) {
      if (fromFolder) {
        scanMessage = String(error);
      } else {
        importMessage = String(error);
      }
    } finally {
      isImporting = false;
    }
  }

  function handleSubmit() {
    dispatch("submit", {
      name,
      projectPath,
      workspaceName: activeWorkspaceName
    });

    name = "";
    projectPath = "";
    // activeWorkspaceName persists across submits — owned by the parent
    // and shared with the Workspaces card. The user is likely adding
    // multiple projects to the same workspace.
  }
</script>

<form class="panel stack" on:submit|preventDefault={handleSubmit}>
  <section class="stack">
    <div class="section-intro">
      <h2>Register Project</h2>
      <p class="muted">
        {#if activeWorkspaceName}
          Adding to <strong>{activeWorkspaceName}</strong>. Pick a different workspace above to change.
        {:else}
          Pick a workspace above first.
        {/if}
      </p>
    </div>

    <label class="field">
      <span>Name</span>
      <input
        bind:value={name}
        disabled={disabled || !activeWorkspaceName || autoscan}
        placeholder={autoscan
          ? "Names come from discovered projects"
          : "Defaults to the selected folder name"}
        required={!autoscan}
        title={autoscan
          ? "Disabled while autoscan is on — each discovered project keeps its folder name."
          : "Display name shown in the Dashboard. Auto-fills from the folder if left blank."}
      />
    </label>

    <label class="field">
      <span>Project path</span>
      <div class="field-row">
        <input
          bind:value={projectPath}
          disabled={disabled || !activeWorkspaceName || isScanning}
          placeholder={autoscan ? "/folder/to/scan/recursively" : "/path/to/java/project"}
          required
          title={autoscan
            ? "Folder to scan recursively (depth ≤ 6) for Java projects."
            : "Absolute path to the Java project root (the folder containing pom.xml, build.gradle, or .project)."}
        />
        <button
          disabled={disabled || !activeWorkspaceName || isScanning}
          on:click={chooseProjectFolder}
          title={autoscan
            ? "Pick a folder — scanning starts immediately"
            : "Open a folder picker to choose the Java project root"}
          type="button"
        >Browse</button>
      </div>
    </label>

    <label class="checkbox-row">
      <input
        checked={autoscan}
        disabled={disabled || !activeWorkspaceName || isScanning || isImporting}
        on:change={toggleAutoscan}
        title="When checked, Browse scans the picked folder recursively for Java projects and lists them for import"
        type="checkbox"
      />
      <span>Recursive search (autoscan)</span>
    </label>

    {#if autoscan}
      {#if showDiscover}
        <!-- Sprint 16.1 (bugs.md #19): Browse auto-scans, so Discover is
             only shown when the current path hasn't been scanned yet (a
             hand-typed path, or one edited since the last scan). Once
             results are showing for this path the button is redundant
             and hides. -->
        <button
          class:primary={canScan}
          disabled={!canScan}
          on:click={scanProjectFolder}
          title="Scan the folder above recursively (depth ≤ 6) for Java projects"
          type="button"
        >{isScanning ? "Scanning…" : "Discover"}</button>
      {/if}
    {:else}
      <button
        class:primary={!disabled && canSubmit}
        disabled={disabled || !canSubmit}
        title="Add this project to the selected workspace"
        type="submit"
      >Save project</button>
    {/if}

    {#if candidateSource === "folder" && candidates.length > 0}
      <p class="muted">Found {candidates.length} project(s) (depth ≤ 6).</p>
      <div class="stack candidate-list">
        {#each candidates as candidate}
          <label class="checkbox-row" title={candidate.projectPath}>
            <input
              checked={selectedPaths.includes(candidate.projectPath)}
              disabled={disabled || isImporting}
              on:change={() => toggleCandidate(candidate.projectPath)}
              title="Include this project in the import"
              type="checkbox"
            />
            <span>{candidate.name} ({candidate.kind}) - {candidate.projectPath}</span>
          </label>
        {/each}
      </div>
      <button
        class:primary={canImportSelected}
        disabled={!canImportSelected}
        on:click={importSelected}
        title={`Add ${selectedPaths.length} project(s) to ${activeWorkspaceName || "the selected workspace"}`}
        type="button"
      >
        Import selected ({selectedPaths.length})
      </button>
    {/if}

    {#if scanMessage}
      <p class="muted">{scanMessage}</p>
    {/if}
  </section>

  <hr class="section-divider" />

  <section class="stack">
    <div class="section-intro">
      <h2>Import from VSCode Workspace</h2>
      <p class="muted">
        {#if activeWorkspaceName}
          Discover Maven/Gradle and Eclipse/PDE Java projects from a .code-workspace file. Selected projects join <strong>{activeWorkspaceName}</strong>.
        {:else}
          Pick a workspace above first.
        {/if}
      </p>
    </div>

    <label class="field">
      <span>.code-workspace file</span>
      <div class="field-row">
        <input
          bind:value={workspaceFile}
          disabled={disabled || isImporting}
          placeholder="/path/to/workspace.code-workspace"
          title="Path to a VSCode .code-workspace file describing the projects to import"
        />
        <button
          disabled={disabled || isImporting}
          on:click={chooseWorkspaceFile}
          title="Open a file picker to choose a .code-workspace file"
          type="button"
        >Browse</button>
      </div>
    </label>

    <button
      class:primary={canDiscover}
      disabled={!canDiscover}
      on:click={discoverFromWorkspace}
      title="Scan the chosen .code-workspace file for Java projects (Maven/Gradle/Eclipse)"
      type="button"
    >
      Discover
    </button>

    {#if candidateSource === "workspace" && candidates.length > 0}
      <div class="stack candidate-list">
        {#each candidates as candidate}
          <label class="checkbox-row" title={candidate.projectPath}>
            <input
              checked={selectedPaths.includes(candidate.projectPath)}
              disabled={disabled || isImporting}
              on:change={() => toggleCandidate(candidate.projectPath)}
              title="Include this project in the import"
              type="checkbox"
            />
            <span>{candidate.name} ({candidate.kind}) - {candidate.projectPath}</span>
          </label>
        {/each}
      </div>
      <button
        class:primary={canImportSelected}
        disabled={!canImportSelected}
        on:click={importSelected}
        title={`Add ${selectedPaths.length} project(s) to ${activeWorkspaceName || "the selected workspace"}`}
        type="button"
      >
        Import selected
      </button>
    {/if}

    {#if importMessage}
      <p class="muted">{importMessage}</p>
    {/if}
  </section>
</form>
