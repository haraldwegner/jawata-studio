<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { createEventDispatcher, onMount } from "svelte";
  import {
    addProject,
    discoverWorkspaceProjects,
    importWorkspaceProjects,
    scanFolderForProjects,
    type WorkspaceProjectCandidate
  } from "../api/tauri";

  /** Open the panel while there is nothing registered yet; fold it once there is. */
  export let projectCount = 0;
  /** False until the project list has been read; before that the count is 0 whatever is registered. */
  export let ready = true;
  // Decided ONCE, as soon as the list is known. Binding `open` to the live count would fold the panel the
  // moment the first project is added, cutting off the result of an import that is still being reported.
  // BOUND, not a one-way `open={…}`: a one-way value is written back on the element's next
  // update, so picking a .code-workspace file (which changes the Discover button) folded the
  // panel shut under him (2026-10-22). Bound, the panel stays the way he left it.
  let foldOpen = false;
  let decided = false;
  $: if (!decided && ready) {
    foldOpen = projectCount === 0;
    decided = true;
  }
  export let disabled = false;
  /** Sprint 10 v0.10.4: the workspace this form's submit will add the
   * project to. Owned by the parent (App.svelte) and shared with the
   * Workspaces card on the left so that picking a workspace there
   * routes new projects (and imports) to it. Empty string = no
   * workspace selected yet. */
  export let activeWorkspaceName: string = "";

  const dispatch = createEventDispatcher<{
    imported: void;
  }>();

  let projectPath = "";
  let workspaceFile = "";
  let candidates: WorkspaceProjectCandidate[] = [];
  let selectedPaths: string[] = [];
  let importMessage = "";
  let isImporting = false;
  let lastDiscoveredFile = "";

  /* Sprint 16 autoscan, ALWAYS ON since the S19 dogfood (2026-10-21): Browse searches the picked
   * folder and the results unfold into the candidate list, which is shared with the
   * .code-workspace flow; candidateSource records which flow filled it so the two never fight. */
  let isScanning = false;
  /** studio#27: true while a .code-workspace is being walked for projects. */
  let isDiscovering = false;
  let scanMessage = "";
  let scannedFolder = "";
  let candidateSource: "" | "folder" | "workspace" = "";

  // studio#27: `isDiscovering` was missing entirely, which is the whole bug.
  // A .code-workspace naming ~200 modules takes 15-30s to walk, and during
  // that time the button stayed enabled and unlabelled — so there was no sign
  // anything was happening, and a second click started a second walk.
  $: canDiscover =
    !disabled &&
    !isImporting &&
    !isDiscovering &&
    workspaceFile.trim().length > 0 &&
    workspaceFile.trim() !== lastDiscoveredFile;

  $: canImportSelected =
    !disabled && !isImporting && selectedPaths.length > 0 && activeWorkspaceName.length > 0;

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

  async function chooseProjectFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Select a Java project, or a folder that holds several"
    });

    if (typeof selected === "string") {
      projectPath = selected;
      await scanProjectFolder();
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
        scanMessage = `No Maven, Gradle or Eclipse project found under ${folder}.`;
      }
    } catch (error) {
      scanMessage = String(error);
      candidateSource = "";
      scannedFolder = "";
    } finally {
      isScanning = false;
    }
  }

  /* The search finds only projects with a build or Eclipse file. A plain source folder is still a
   * project jawata can load, so when the search comes back empty the folder itself can be added. */
  $: canRegisterFolder =
    candidateSource === "folder" &&
    candidates.length === 0 &&
    projectPath.trim() === scannedFolder &&
    !disabled &&
    !isImporting &&
    activeWorkspaceName.length > 0;

  async function registerFolderAsProject() {
    const folder = scannedFolder;
    const name = folder.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || folder;
    isImporting = true;
    try {
      await addProject({ name, projectPath: folder, workspaceName: activeWorkspaceName });
      scanMessage = `Added ${name}.`;
      projectPath = "";
      candidateSource = "";
      scannedFolder = "";
      dispatch("imported");
    } catch (error) {
      scanMessage = String(error);
    } finally {
      isImporting = false;
    }
  }

  function onEnter(event: KeyboardEvent, action: () => void) {
    if (event.key === "Enter") {
      event.preventDefault();
      action();
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
    isDiscovering = true;
    // studio#27: say something within the first frame. A workspace naming ~200
    // modules takes 15-30s to walk, and an unchanged screen for that long is
    // indistinguishable from a hung application — which is what it was
    // reported as.
    scanMessage = "Reading the workspace file and looking for Java projects…";
    try {
      candidates = await discoverWorkspaceProjects(path);
      selectedPaths = candidates.map((candidate) => candidate.projectPath);
      candidateSource = "workspace";
      scannedFolder = "";
      lastDiscoveredFile = path;
      scanMessage =
        candidates.length === 0
          ? ""
          : `Found ${candidates.length} project(s).`;
      if (candidates.length === 0) {
        importMessage = "No Maven/Gradle or Eclipse/PDE Java projects found.";
      }
    } catch (error) {
      scanMessage = "";
      importMessage = String(error);
    } finally {
      isDiscovering = false;
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

</script>

<form class="panel stack" on:submit|preventDefault={scanProjectFolder}>
<details class="fold" bind:open={foldOpen}>
<summary title="Pick a folder: the Java project in it, or every project under it, is listed for you to import. Or import the projects of a .code-workspace file. Open while you have no project yet.">Add projects</summary>
<div class="fold-body">
  <section class="stack">
    <div class="section-intro">
      <h3>From a folder</h3>
      <p class="muted">
        {#if activeWorkspaceName}
          Adding to <strong>{activeWorkspaceName}</strong>. Pick a different workspace above to change.
        {:else}
          Pick a workspace above first.
        {/if}
      </p>
    </div>

    <label class="field">
      <span>Folder</span>
      <div class="field-row">
        <input
          bind:value={projectPath}
          disabled={disabled || !activeWorkspaceName || isScanning}
          on:keydown={(event) => onEnter(event, () => canScan && scanProjectFolder())}
          placeholder="/a/java/project, or a folder that holds several"
          required
          title="A Java project's folder, or a folder that holds several: it is searched (depth up to 6) and every Maven, Gradle or Eclipse project in it is listed, the folder itself included. Each keeps its folder name; rename one later with a right-click."
        />
        <button
          disabled={disabled || !activeWorkspaceName || isScanning}
          on:click={chooseProjectFolder}
          title="Pick a folder; the search starts at once"
          type="button"
        >Browse</button>
      </div>
    </label>

    {#if showDiscover}
      <!-- Sprint 16.1 (bugs.md #19): Browse searches at once, so this shows only for a
           hand-typed path, or one edited since the last search. -->
      <button
        class:primary={canScan}
        disabled={!canScan}
        on:click={scanProjectFolder}
        title="Search the folder above (depth up to 6) for Java projects"
        type="button"
      >{isScanning ? "Searching…" : "Find projects"}</button>
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

    {#if canRegisterFolder}
      <button
        on:click={registerFolderAsProject}
        title={`Add ${scannedFolder} to ${activeWorkspaceName} as one project, although it has no Maven, Gradle or Eclipse file. jawata loads its Java sources as they are.`}
        type="button"
      >Add this folder as a project</button>
    {/if}
  </section>

  <hr class="section-divider" />

  <section class="stack">
    <div class="section-intro">
      <h3>From a .code-workspace file</h3>
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
          on:keydown={(event) => onEnter(event, () => canDiscover && discoverFromWorkspace())}
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
      {isDiscovering ? "Discovering…" : "Discover"}
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
</div>
</details>
</form>
