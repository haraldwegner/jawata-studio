//! The editor's folders decide which jawata workspace runs (Sprint 28h, Stage 21 (j)).
//!
//! The owner's words (2026-11-07): *"If I start a VScode with a workspace, then we need to start the jawata workspace
//! which includes this. And if we don't have one we need a new jawata workspace for this from the vscode workspace."*
//! and *"If there is one leave studio in the background, just take the enclosing one. If there is none at all, make one,
//! you can leave in the background. Only if there is a partly matching one open studio and ask for an update."*
//!
//! An editor extension starts the studio with `--background --editor-folder <abs path>` once per folder of the
//! editor's workspace, either as the first launch or as a second launch that the running studio's single-instance
//! callback receives. Both reach [`handle_editor_launch`]. From there:
//!
//! ```text
//! folders ──scan (as "Find projects" does)──► P_e   (the editor workspace's Java projects)
//! P_e vs every jawata workspace's P_w:
//!   P_e empty ........................ NONE_JAVA  nothing starts
//!   P_e ⊆ P_w for some W ............. ENCLOSING  start W if stopped, no window
//!   else, some W shares projects ..... PARTIAL    window + question, nothing changes until the answer
//!   else ............................. CREATE     new workspace from P_e, started, no window
//! ```
//!
//! The answer goes to the extension through a small file, the HANDOFF FILE `<config dir>/editor-workspaces/<id>.json`,
//! because the address and key in the extension's settings were written at deploy time for one resident, and a
//! workspace per folder has a resident per folder. The file says `ready` only when the resident answers.
//!
//! Everything the rule needs from the studio sits behind [`EditorHost`] and [`EditorUi`], so the whole decision is
//! run in tests against a fake, and the only line of this file that raises a window is [`AppUi::ask`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::manager_service::WorkspaceProjectCandidate;
use crate::AppState;

/// The argument an extension passes once per folder of its workspace.
pub const EDITOR_FOLDER_ARG: &str = "--editor-folder";
/// The folder inside the studio's configuration directory that holds the handoff files.
pub const HANDOFF_FOLDER: &str = "editor-workspaces";
/// Sent to the window whenever the list of open questions changes.
pub const PROMPTS_CHANGED_EVENT: &str = "jawata://editor-prompts-changed";

// ===== the arguments =====

/// The folders a launch carries: every value that follows `--editor-folder`, in order, as passed. The program name
/// (argument zero) is never read. A flag with no value, or whose next word is another flag, adds nothing: a folder is
/// an absolute path and cannot begin with `--`.
pub fn editor_folders<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().skip(1).map(|a| a.as_ref().to_string()).collect();
    let mut folders = Vec::new();
    let mut at = 0;
    while at < args.len() {
        if args[at] == EDITOR_FOLDER_ARG {
            if let Some(value) = args.get(at + 1).filter(|v| !v.is_empty() && !v.starts_with("--")) {
                folders.push(value.clone());
                at += 2;
                continue;
            }
        }
        at += 1;
    }
    folders
}

// ===== the handoff file =====

/// The id of an editor workspace: the lowercase hex SHA-256 of its folders exactly as passed on the command line,
/// sorted ascending by their UTF-8 bytes and joined with a single `\n` (no trailing newline). The extension computes
/// the same value to find its file.
pub fn handoff_id(folders: &[String]) -> String {
    let mut sorted: Vec<&str> = folders.iter().map(String::as_str).collect();
    sorted.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let digest = Sha256::digest(sorted.join("\n").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The folder the handoff files go in: `<the studio's configuration directory>/editor-workspaces`, absolute.
pub fn handoff_dir(config_dir: &Path) -> PathBuf {
    let joined = config_dir.join(HANDOFF_FOLDER);
    std::path::absolute(&joined).unwrap_or(joined)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandoffState {
    /// The resident of `workspace` answers at `url` with `key`.
    Ready,
    /// The studio window is open with a question; nothing has changed yet.
    Asking,
    /// The person answered "Not now".
    Declined,
    /// The editor's folders hold no Java project, so nothing was started.
    None,
    /// Something went wrong; `message` says what.
    Failed,
}

/// What the extension reads: `{folders, workspace, url, key, state, message}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    pub folders: Vec<String>,
    pub workspace: String,
    pub url: String,
    pub key: String,
    pub state: HandoffState,
    pub message: String,
}

impl Handoff {
    fn waiting(folders: &[String], workspace: &str, state: HandoffState, message: &str) -> Self {
        Handoff {
            folders: folders.to_vec(),
            workspace: workspace.to_string(),
            url: String::new(),
            key: String::new(),
            state,
            message: message.to_string(),
        }
    }
}

fn handoff_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Writes the file of `handoff` into `dir` (made on demand), atomically: the text goes to a temporary file in the same
/// folder, readable by its owner only, and is renamed over the final name, so a reader sees the old file or the new
/// one and never half of either, and a file that was left with wider rights is replaced and not rewritten in place.
pub fn write_handoff(dir: &Path, handoff: &Handoff) -> Result<PathBuf, String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let id = handoff_id(&handoff.folders);
    let target = handoff_path(dir, &id);
    let temp = dir.join(format!(".{id}.json.tmp-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let text = serde_json::to_string_pretty(handoff).map_err(|e| format!("could not write the handoff: {e}"))?;
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not write {}: {e}", temp.display()));
    }
    if let Err(e) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not place {}: {e}", target.display()));
    }
    Ok(target)
}

// ===== comparing projects =====

/// A project path as it is compared: canonical (symbolic links and `..` resolved), or the given path normalised when it
/// cannot be resolved; folded to lower case only where the file system is case-insensitive in practice.
pub fn project_key_with(path: &str, fold_case: bool) -> String {
    let given = Path::new(path.trim());
    let resolved = std::fs::canonicalize(given).unwrap_or_else(|_| given.components().collect::<PathBuf>());
    let text = resolved.to_string_lossy().into_owned();
    if fold_case {
        text.to_lowercase()
    } else {
        text
    }
}

/// [`project_key_with`] on this machine: case-insensitive on Windows and macOS, exact elsewhere.
pub fn project_key(path: &str) -> String {
    project_key_with(path, cfg!(windows) || cfg!(target_os = "macos"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// The editor's folders hold no Java project.
    NoneJava,
    /// A workspace holds every project of the editor's folders (the one with the fewest projects, then by name).
    Enclosing { workspace: String },
    /// No workspace holds them all; this one shares the most projects with them (then by name).
    Partial { workspace: String, overlap: usize },
    /// No workspace shares a project with them.
    Create,
}

/// Whether the project `key` is `ancestor` or lies inside it, by whole path components (`/proj/mod-a` is inside
/// `/proj`, `/proj2` is not).
pub fn is_within(key: &str, ancestor: &str) -> bool {
    Path::new(key).starts_with(Path::new(ancestor))
}

/// COVERED: a workspace covers an editor project when one of its own project paths equals it or is an ancestor of it.
/// A module of a project the workspace holds at its root is therefore held by the workspace, and the owner's words are
/// "the jawata workspace which includes this".
pub fn covered_by(workspace_projects: &BTreeSet<String>, key: &str) -> bool {
    workspace_projects.iter().any(|project| is_within(key, project))
}

/// THE rule. `editor` is the editor workspace's project set P_e, `workspaces` each jawata workspace's set P_w, all as
/// [`project_key`]s; a project of P_e is covered by a workspace as [`covered_by`] says. Pure.
pub fn match_workspaces(editor: &BTreeSet<String>, workspaces: &[(String, BTreeSet<String>)]) -> Match {
    if editor.is_empty() {
        return Match::NoneJava;
    }
    let covered = |projects: &BTreeSet<String>| editor.iter().filter(|key| covered_by(projects, key)).count();
    let enclosing = workspaces
        .iter()
        .filter(|(_, projects)| covered(projects) == editor.len())
        .min_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.0.cmp(&b.0)));
    if let Some((name, _)) = enclosing {
        return Match::Enclosing { workspace: name.clone() };
    }
    let partial = workspaces
        .iter()
        .map(|(name, projects)| (name, covered(projects)))
        .filter(|(_, overlap)| *overlap > 0)
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)));
    match partial {
        Some((name, overlap)) => Match::Partial { workspace: name.clone(), overlap },
        None => Match::Create,
    }
}

/// `base` made unique among `taken` by a numeric suffix (`name`, `name-2`, `name-3`, ...). Names are compared
/// ignoring case: a workspace is a folder on disk, and two that differ only in case collide on some systems.
pub fn unique_workspace_name(base: &str, taken: &[String]) -> String {
    let base = base.trim();
    let base = if base.is_empty() { "workspace" } else { base };
    let used = |name: &str| taken.iter().any(|t| t.to_lowercase() == name.to_lowercase());
    if !used(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !used(candidate))
        .expect("an unbounded range has a free name")
}

fn basename_of(folder: &str) -> String {
    Path::new(folder.trim())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ===== what the rule needs of the studio =====

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// The resident answers.
    Ready,
    /// It is on its way up (or has not been started).
    Waiting,
    /// It cannot come up; the reason.
    Failed(String),
}

/// How long to wait for a resident: the pause between looks and the time after which the wait is given up.
#[derive(Debug, Clone, Copy)]
pub struct Pacing {
    pub poll: Duration,
    pub give_up: Duration,
    /// How long a start that was refused for want of an installed engine waits for the engine to be installed, polling
    /// every `poll`, before the start is requested once more.
    pub install_wait: Duration,
}

impl Pacing {
    pub fn production() -> Self {
        Pacing { poll: Duration::from_millis(500), give_up: Duration::from_secs(300), install_wait: Duration::from_secs(120) }
    }
}

pub trait EditorHost {
    /// Where the handoff files go.
    fn handoff_dir(&self) -> PathBuf;
    /// Every jawata workspace with the paths of its projects.
    fn workspaces(&self) -> Vec<(String, Vec<String>)>;
    /// The Java projects under `folder`, as the dashboard's "Find projects" finds them.
    fn scan_folder(&self, folder: &str) -> Result<Vec<WorkspaceProjectCandidate>, String>;
    /// Registers projects in a workspace (made on the first); the projects that could not go in are named in the answer.
    fn add_projects(&self, workspace: &str, projects: &[WorkspaceProjectCandidate]) -> Result<Vec<String>, String>;
    /// Starts a workspace by the start path the studio already has.
    fn start(&self, workspace: &str) -> Result<(), String>;
    /// Whether the studio is set to run a managed engine that is not installed (yet): the one condition under which a
    /// start is refused for want of an engine, asked of the same installed-engine record the start reads, so that no
    /// refusal text has to be recognised. The studio downloads and installs the engine on its own at first start.
    fn engine_missing(&self) -> bool;
    fn readiness(&self, workspace: &str) -> Readiness;
    /// The resident's base address (what `JAWATA_URL` carries, no `/mcp`) and its bearer key.
    fn endpoint(&self, workspace: &str) -> Result<(String, String), String>;
}

pub trait EditorUi {
    /// Show the studio window with the question. The ONLY thing in the rule that raises a window.
    fn ask(&self, prompt: &UpdatePrompt);
    /// The list of open questions changed.
    fn prompts_changed(&self);
    /// A workspace was created, changed or started: the dashboard should reload.
    fn workspaces_changed(&self);
    /// The launch ended with no resident for the editor (`none`, `declined` or `failed`): bring back the services that were
    /// running at last close, as a plain `--background` launch does, because the extension still waits for an engine.
    /// Never raises a window.
    fn restore_last_running(&self);
}

// ===== the question =====

/// What the window shows for a partly matching workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePrompt {
    pub id: String,
    pub workspace: String,
    /// The projects the workspace already has of the editor's.
    pub existing: Vec<String>,
    /// The editor's projects the workspace does not have and that "Update workspace" adds.
    pub missing: Vec<String>,
    /// The editor's projects that belong to another workspace; they stay there.
    pub elsewhere: Vec<String>,
    pub folders: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub prompt: UpdatePrompt,
    candidates: Vec<WorkspaceProjectCandidate>,
}

/// The questions that are open. One per editor workspace (by id); a repeated launch replaces its own.
#[derive(Default)]
pub struct PromptBook {
    open: Mutex<Vec<Pending>>,
}

impl PromptBook {
    pub fn put(&self, pending: Pending) {
        let mut open = self.open.lock().expect("prompt book poisoned");
        open.retain(|p| p.prompt.id != pending.prompt.id);
        open.push(pending);
    }

    pub fn take(&self, id: &str) -> Option<Pending> {
        let mut open = self.open.lock().expect("prompt book poisoned");
        let at = open.iter().position(|p| p.prompt.id == id)?;
        Some(open.remove(at))
    }

    pub fn list(&self) -> Vec<UpdatePrompt> {
        self.open.lock().expect("prompt book poisoned").iter().map(|p| p.prompt.clone()).collect()
    }
}

/// The studio's one book of open questions.
pub fn book() -> &'static PromptBook {
    static BOOK: OnceLock<PromptBook> = OnceLock::new();
    BOOK.get_or_init(PromptBook::default)
}

// ===== the rule, run =====

fn write_state(dir: &Path, folders: &[String], workspace: &str, state: HandoffState, message: &str) {
    if let Err(error) = write_handoff(dir, &Handoff::waiting(folders, workspace, state, message)) {
        crate::studio_log::say(&format!("editor launch: {error}"));
    }
}

/// An end of the launch with no resident for the editor (`none`, `declined`, `failed`): the answer is written, and the
/// services that ran at last close are brought back, because the extension still waits for an engine and a launch that
/// carries folders does not restore them at start (Stage 21 (j) audit, B2). The ready and asking states never come here.
fn end_without_resident(ui: &dyn EditorUi, dir: &Path, folders: &[String], workspace: &str, state: HandoffState, message: &str) {
    write_state(dir, folders, workspace, state, message);
    ui.restore_last_running();
}

/// One launch at a time chooses a new workspace's name and registers it: two launches for different folders whose first
/// folder has the same name would otherwise both find the name free and add their projects to one workspace. Held for the
/// decision, the registration and the request to start; never while waiting for a resident.
static CREATE_LOCK: Mutex<()> = Mutex::new(());

/// The first request to start `workspace`: nothing when it already answers. Writes nothing; [`settle_start`] judges it.
fn request_start_once(host: &dyn EditorHost, workspace: &str) -> Result<(), String> {
    if host.readiness(workspace) != Readiness::Ready {
        host.start(workspace)?;
    }
    Ok(())
}

/// Judges the first request to start. `false` when the workspace could not be started (the failure is written).
///
/// A start refused because the managed engine is not installed yet is not a failure of this launch: a studio on a fresh
/// install downloads the engine by itself and has it a few seconds later. So the launch waits for the install, at most
/// `pacing.install_wait`, writing no answer meanwhile, and requests the start once more. Any other refusal is final.
fn settle_start(host: &dyn EditorHost, ui: &dyn EditorUi, folders: &[String], workspace: &str, pacing: Pacing, first: Result<(), String>) -> bool {
    let Err(refusal) = first else { return true };
    let dir = host.handoff_dir();
    if !host.engine_missing() {
        end_without_resident(ui, &dir, folders, workspace, HandoffState::Failed, &format!("Workspace {workspace} could not be started: {refusal}"));
        return false;
    }
    let mut waited = Duration::ZERO;
    while host.engine_missing() && waited < pacing.install_wait {
        std::thread::sleep(pacing.poll);
        waited += pacing.poll;
    }
    let installed = !host.engine_missing();
    match host.start(workspace) {
        Ok(()) => true,
        Err(error) => {
            let how = if installed {
                format!("the engine download finished, but the start was refused again: {error}")
            } else {
                format!("the engine download did not finish within {} seconds ({refusal})", pacing.install_wait.as_secs())
            };
            end_without_resident(ui, &dir, folders, workspace, HandoffState::Failed, &format!("Workspace {workspace} could not be started: {how}"));
            false
        }
    }
}

/// Starts `workspace` unless it already answers, waits until it does, and only then writes `ready`.
fn serve(host: &dyn EditorHost, ui: &dyn EditorUi, folders: &[String], workspace: &str, pacing: Pacing) {
    let first = request_start_once(host, workspace);
    if settle_start(host, ui, folders, workspace, pacing, first) {
        wait_until_ready(host, ui, folders, workspace, pacing);
    }
}

/// Waits until the resident of `workspace` answers and only then writes `ready`.
fn wait_until_ready(host: &dyn EditorHost, ui: &dyn EditorUi, folders: &[String], workspace: &str, pacing: Pacing) {
    let dir = host.handoff_dir();
    let mut waited = Duration::ZERO;
    loop {
        match host.readiness(workspace) {
            Readiness::Ready => break,
            Readiness::Failed(reason) => {
                end_without_resident(ui, &dir, folders, workspace, HandoffState::Failed, &format!("Workspace {workspace} did not come up: {reason}"));
                return;
            }
            Readiness::Waiting if waited >= pacing.give_up => {
                end_without_resident(
                    ui,
                    &dir,
                    folders,
                    workspace,
                    HandoffState::Failed,
                    &format!("Workspace {workspace} did not answer within {} seconds.", pacing.give_up.as_secs()),
                );
                return;
            }
            Readiness::Waiting => {
                std::thread::sleep(pacing.poll);
                waited += pacing.poll;
            }
        }
    }
    match host.endpoint(workspace) {
        Ok((url, key)) => {
            let ready = Handoff { folders: folders.to_vec(), workspace: workspace.to_string(), url, key, state: HandoffState::Ready, message: String::new() };
            if let Err(error) = write_handoff(&dir, &ready) {
                crate::studio_log::say(&format!("editor launch: {error}"));
            }
        }
        Err(error) => end_without_resident(
            ui,
            &dir,
            folders,
            workspace,
            HandoffState::Failed,
            &format!("Workspace {workspace} answers, but its address is not known: {error}"),
        ),
    }
}

/// What the editor's folders hold.
struct Discovered {
    /// The Java projects of the folders, each once, by key.
    found: BTreeMap<String, WorkspaceProjectCandidate>,
    /// The folders that were read and hold no project: one of them may lie inside a project a workspace holds.
    empty_folders: Vec<String>,
    /// The folders that could not be read, each with the reason.
    problems: Vec<String>,
}

fn discover(host: &dyn EditorHost, folders: &[String]) -> Discovered {
    let mut found = BTreeMap::new();
    let mut empty_folders = Vec::new();
    let mut problems = Vec::new();
    for folder in folders {
        match host.scan_folder(folder) {
            Ok(candidates) if candidates.is_empty() => empty_folders.push(folder.clone()),
            Ok(candidates) => {
                for candidate in candidates {
                    found.entry(project_key(&candidate.project_path)).or_insert(candidate);
                }
            }
            Err(error) => problems.push(format!("{folder}: {error}")),
        }
    }
    Discovered { found, empty_folders, problems }
}

/// A folder that holds no project of its own but lies inside (or is) a project a workspace holds counts as covered: it
/// stands in the editor's project set as itself. It is never added to a workspace, because a workspace covers it.
fn folder_standing_in(folder: &str) -> WorkspaceProjectCandidate {
    WorkspaceProjectCandidate { name: basename_of(folder), project_path: folder.to_string(), kind: "folder".into(), source_roots: Default::default() }
}

/// One editor launch, from the folders to the file the extension reads. Runs on a thread of its own: it waits for a
/// resident. Raises a window only by asking [`EditorUi::ask`], and asks only for a partly matching workspace.
pub fn run_editor_launch(host: &dyn EditorHost, ui: &dyn EditorUi, book: &PromptBook, folders: &[String], pacing: Pacing) {
    let dir = host.handoff_dir();
    // A file left by an earlier launch must not be taken for this launch's answer while it is still being decided.
    let _ = std::fs::remove_file(handoff_path(&dir, &handoff_id(folders)));
    let Discovered { mut found, empty_folders, problems } = discover(host, folders);
    for problem in &problems {
        crate::studio_log::say(&format!("editor launch: a folder could not be searched: {problem}"));
    }
    // From here to the request to start, the choice of a workspace and a new workspace's name are one step.
    let create_guard = CREATE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspaces = host.workspaces();
    let sets: Vec<(String, BTreeSet<String>)> = workspaces
        .iter()
        .map(|(name, paths)| (name.clone(), paths.iter().map(|p| project_key(p)).collect()))
        .collect();
    for folder in &empty_folders {
        let key = project_key(folder);
        if sets.iter().any(|(_, keys)| covered_by(keys, &key)) {
            found.entry(key).or_insert_with(|| folder_standing_in(folder));
        }
    }
    let editor: BTreeSet<String> = found.keys().cloned().collect();
    match match_workspaces(&editor, &sets) {
        Match::NoneJava => {
            drop(create_guard);
            let message = if problems.is_empty() {
                "The editor's folders hold no Java project, so no jawata workspace was started.".to_string()
            } else {
                format!("The editor's folders hold no Java project that could be read ({}), so no jawata workspace was started.", problems.join("; "))
            };
            end_without_resident(ui, &dir, folders, "", HandoffState::None, &message);
        }
        Match::Enclosing { workspace } => {
            drop(create_guard);
            serve(host, ui, folders, &workspace, pacing);
            ui.workspaces_changed();
        }
        Match::Create => {
            let taken: Vec<String> = workspaces.iter().map(|(name, _)| name.clone()).collect();
            let name = unique_workspace_name(&basename_of(folders.first().map(String::as_str).unwrap_or("")), &taken);
            let projects: Vec<WorkspaceProjectCandidate> = found.into_values().collect();
            match host.add_projects(&name, &projects) {
                Err(error) => {
                    drop(create_guard);
                    end_without_resident(ui, &dir, folders, &name, HandoffState::Failed, &format!("Workspace {name} could not be created: {error}"));
                }
                Ok(skipped) => {
                    for line in skipped {
                        crate::studio_log::say(&format!("editor launch: not registered in {name}: {line}"));
                    }
                    ui.workspaces_changed();
                    let first = request_start_once(host, &name);
                    drop(create_guard);
                    if settle_start(host, ui, folders, &name, pacing, first) {
                        wait_until_ready(host, ui, folders, &name, pacing);
                    }
                }
            }
        }
        Match::Partial { workspace, .. } => {
            drop(create_guard);
            let have = sets.iter().find(|(name, _)| *name == workspace).map(|(_, keys)| keys.clone()).unwrap_or_default();
            let mut existing = Vec::new();
            let mut free = Vec::new();
            let mut elsewhere = Vec::new();
            for (key, candidate) in found {
                if covered_by(&have, &key) {
                    existing.push(candidate.project_path);
                } else if let Some((owner, _)) = sets.iter().find(|(name, keys)| *name != workspace && covered_by(keys, &key)) {
                    elsewhere.push(format!("{} (in workspace {owner})", candidate.project_path));
                } else {
                    free.push(candidate);
                }
            }
            if free.is_empty() {
                // Everything the workspace lacks belongs to another workspace and stays there, so there is nothing to
                // update; the workspace that shares the most is the one the editor gets.
                crate::studio_log::say(&format!(
                    "editor launch: workspace {workspace} holds what can be had of the editor's projects; {} belong to other workspaces and were left there",
                    elsewhere.len()
                ));
                serve(host, ui, folders, &workspace, pacing);
                ui.workspaces_changed();
                return;
            }
            let prompt = UpdatePrompt {
                id: handoff_id(folders),
                workspace: workspace.clone(),
                existing,
                missing: free.iter().map(|c| c.project_path.clone()).collect(),
                elsewhere,
                folders: folders.to_vec(),
            };
            write_state(
                &dir,
                folders,
                &workspace,
                HandoffState::Asking,
                &format!("The studio is asking whether to add {} project(s) to workspace {workspace}; nothing has changed yet.", free.len()),
            );
            book.put(Pending { prompt: prompt.clone(), candidates: free });
            ui.ask(&prompt);
            ui.prompts_changed();
        }
    }
}

/// The person's answer to a question. `update` adds the missing projects to the workspace, starts it and writes
/// `ready`; otherwise `declined` is written and nothing else happens.
pub fn answer_update(host: &dyn EditorHost, ui: &dyn EditorUi, pending: Pending, update: bool, pacing: Pacing) {
    let dir = host.handoff_dir();
    let Pending { prompt, candidates } = pending;
    ui.prompts_changed();
    if !update {
        end_without_resident(ui, &dir, &prompt.folders, &prompt.workspace, HandoffState::Declined, &format!("Workspace {} was not updated; the person chose not to.", prompt.workspace));
        return;
    }
    match host.add_projects(&prompt.workspace, &candidates) {
        Err(error) => {
            end_without_resident(ui, &dir, &prompt.folders, &prompt.workspace, HandoffState::Failed, &format!("Workspace {} could not be updated: {error}", prompt.workspace))
        }
        Ok(skipped) => {
            for line in skipped {
                crate::studio_log::say(&format!("editor launch: not registered in {}: {line}", prompt.workspace));
            }
            ui.workspaces_changed();
            serve(host, ui, &prompt.folders, &prompt.workspace, pacing);
        }
    }
}

// ===== the real studio =====

impl EditorHost for crate::manager_service::ManagerService {
    fn handoff_dir(&self) -> PathBuf {
        handoff_dir(&self.studio_config_dir())
    }
    fn workspaces(&self) -> Vec<(String, Vec<String>)> {
        self.editor_workspace_projects()
    }
    fn scan_folder(&self, folder: &str) -> Result<Vec<WorkspaceProjectCandidate>, String> {
        self.scan_folder_for_projects(folder)
    }
    fn add_projects(&self, workspace: &str, projects: &[WorkspaceProjectCandidate]) -> Result<Vec<String>, String> {
        self.editor_add_projects(workspace, projects)
    }
    fn start(&self, workspace: &str) -> Result<(), String> {
        self.editor_start_workspace(workspace)
    }
    fn engine_missing(&self) -> bool {
        self.editor_engine_missing()
    }
    fn readiness(&self, workspace: &str) -> Readiness {
        self.editor_workspace_readiness(workspace)
    }
    fn endpoint(&self, workspace: &str) -> Result<(String, String), String> {
        self.editor_workspace_endpoint(workspace)
    }
}

/// The window, for the real studio.
pub struct AppUi<R: Runtime> {
    pub app: AppHandle<R>,
}

impl<R: Runtime> EditorUi for AppUi<R> {
    fn ask(&self, _prompt: &UpdatePrompt) {
        crate::show_main_window(&self.app);
    }
    fn prompts_changed(&self) {
        let _ = self.app.emit(PROMPTS_CHANGED_EVENT, ());
    }
    fn workspaces_changed(&self) {
        let _ = self.app.emit("jawata://settings-changed", ());
    }
    fn restore_last_running(&self) {
        let state = self.app.state::<AppState>();
        if let Err(error) = state.manager_service.restore_last_running_workspaces() {
            crate::studio_log::say(&format!("editor launch: the services of the last run could not be brought back: {error}"));
        }
    }
}

/// The ONE entry for both launches: the first one (the process's own arguments) and a second one (what the
/// single-instance callback receives). Returns whether the launch carried editor folders. The work runs on a thread of
/// its own, so neither the setup nor the callback waits for a resident.
pub fn handle_editor_launch<R: Runtime>(app: &AppHandle<R>, argv: &[String]) -> bool {
    let folders = editor_folders(argv);
    if folders.is_empty() {
        return false;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("editor-launch".into()).spawn(move || {
        let state = app.state::<AppState>();
        run_editor_launch(&state.manager_service, &AppUi { app: app.clone() }, book(), &folders, Pacing::production());
    });
    if let Err(error) = spawned {
        crate::studio_log::say(&format!("editor launch: could not start its thread: {error}"));
    }
    true
}

/// The person's answer, off the thread that received the click.
pub fn answer_in_background<R: Runtime>(app: &AppHandle<R>, id: &str, update: bool) -> Result<(), String> {
    let pending = book().take(id).ok_or_else(|| "this question is no longer open".to_string())?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("editor-answer".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            answer_update(&state.manager_service, &AppUi { app: app.clone() }, pending, update, Pacing::production());
        })
        .map(|_| ())
        .map_err(|e| format!("could not start the answer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    // ----- helpers -----

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(label: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!("jawata-editor-launch-{label}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ws(name: &str, items: &[&str]) -> (String, BTreeSet<String>) {
        (name.to_string(), set(items))
    }

    fn candidate(path: &str) -> WorkspaceProjectCandidate {
        WorkspaceProjectCandidate {
            name: Path::new(path).file_name().unwrap().to_string_lossy().into_owned(),
            project_path: path.to_string(),
            kind: "maven".into(),
            source_roots: Default::default(),
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ----- the arguments -----

    #[test]
    fn several_editor_folders_are_read_in_order_as_passed_from_a_first_launch() {
        let first = ["/opt/jawata-studio", "--background", "--editor-folder", "/a/b", "--editor-folder", "/c", "--editor-folder", "/a/b"];
        assert_eq!(editor_folders(first), strings(&["/a/b", "/c", "/a/b"]), "every one, in order, duplicates as passed");
        assert_eq!(editor_folders(["jawata-studio"]), Vec::<String>::new());
        assert_eq!(editor_folders(["jawata-studio", "--background"]), Vec::<String>::new(), "a launch without folders carries none");
    }

    #[test]
    fn a_second_launch_is_read_from_its_own_argv_the_way_the_callback_receives_it() {
        let argv: Vec<String> = ["jawata-studio", "--editor-folder", "/home/h/proj", "--background"].iter().map(|s| s.to_string()).collect();
        assert_eq!(editor_folders(&argv), strings(&["/home/h/proj"]), "the flag order does not matter and a String vector works");
    }

    #[test]
    fn a_flag_without_a_value_and_the_program_name_add_nothing() {
        assert_eq!(editor_folders(["jawata-studio", "--editor-folder"]), Vec::<String>::new());
        assert_eq!(editor_folders(["jawata-studio", "--editor-folder", "--background"]), Vec::<String>::new(), "a flag is not a folder");
        assert_eq!(editor_folders(["jawata-studio", "--editor-folder", ""]), Vec::<String>::new());
        assert_eq!(editor_folders(["--editor-folder", "/a"]), Vec::<String>::new(), "argument zero is the program");
        assert_eq!(editor_folders(["jawata-studio", "--editor-folder=/a"]), Vec::<String>::new(), "only the contract's spelling");
    }

    // ----- the id -----

    #[test]
    fn the_id_is_the_sha256_of_the_sorted_folders_joined_by_a_newline_and_matches_the_contract_vectors() {
        assert_eq!(handoff_id(&strings(&["/a/b", "/c"])), "5650d16fbb762aa37a51dc25636e82d35230e7da6a6d5e77c31dd3b4007de4be");
        assert_eq!(handoff_id(&strings(&["/home/h/proj"])), "1e7edaca38dbb9d07c23cc7e5f5de48c9460cf7c53bee8eb926d3dd184ee2209");
        assert_eq!(handoff_id(&strings(&["/c", "/a/b"])), handoff_id(&strings(&["/a/b", "/c"])), "sorted, so the order passed does not matter");
        assert_ne!(handoff_id(&strings(&["/a/b", "/c"])), handoff_id(&strings(&["/a/b/c"])), "a join is not a concatenation");
    }

    #[test]
    fn the_sort_is_by_utf8_bytes_and_the_folders_are_not_normalised() {
        // 'é' (0xC3 0xA9) sorts after 'z' (0x7A) by bytes, which is not the order of a locale collation
        let sorted = ["/z", "/é"];
        let hash = |text: &str| Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(handoff_id(&strings(&["/é", "/z"])), hash(&sorted.join("\n")));
        assert_ne!(handoff_id(&strings(&["/a/b/"])), handoff_id(&strings(&["/a/b"])), "folders exactly as passed");
    }

    // ----- comparing projects -----

    #[test]
    fn a_project_key_resolves_to_the_same_value_for_a_trailing_slash_and_for_a_missing_folder() {
        assert_eq!(project_key_with("/nonexistent/x/", false), project_key_with("/nonexistent/x", false));
        assert_eq!(project_key_with(" /nonexistent/x ", false), project_key_with("/nonexistent/x", false));
    }

    #[test]
    fn case_is_folded_only_when_the_file_system_asks_for_it() {
        assert_eq!(project_key_with("/Work/Proj", true), project_key_with("/work/PROJ", true));
        assert_ne!(project_key_with("/Work/Proj", false), project_key_with("/work/PROJ", false), "Linux keeps two names");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_and_its_target_are_one_project() {
        let s = Scratch::new("symlink");
        let real = s.0.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = s.0.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(project_key(&real.to_string_lossy()), project_key(&link.to_string_lossy()));
        assert_ne!(project_key(&real.to_string_lossy()), project_key(&s.0.to_string_lossy()), "the control: a different folder differs");
    }

    // ----- the rule -----

    #[test]
    fn a_workspace_that_holds_every_project_encloses_and_the_one_with_the_fewest_wins() {
        let editor = set(&["/p/a", "/p/b"]);
        let big = ws("big", &["/p/a", "/p/b", "/p/c", "/p/d"]);
        let small = ws("small", &["/p/a", "/p/b", "/p/c"]);
        assert_eq!(match_workspaces(&editor, &[big.clone(), small.clone()]), Match::Enclosing { workspace: "small".into() });
        assert_eq!(match_workspaces(&editor, &[small, big]), Match::Enclosing { workspace: "small".into() }, "not the first one listed");
    }

    #[test]
    fn two_enclosing_workspaces_of_the_same_size_are_told_apart_by_name() {
        let editor = set(&["/p/a"]);
        let m = ws("m", &["/p/a", "/p/x"]);
        let b = ws("b", &["/p/a", "/p/y"]);
        assert_eq!(match_workspaces(&editor, &[m.clone(), b.clone()]), Match::Enclosing { workspace: "b".into() });
        assert_eq!(match_workspaces(&editor, &[b, m]), Match::Enclosing { workspace: "b".into() });
    }

    #[test]
    fn a_whole_enclosing_workspace_beats_a_partial_one_with_more_in_common() {
        // the partial one shares two, the enclosing one shares all three of the editor's
        let editor = set(&["/p/a", "/p/b", "/p/c"]);
        let enclosing = ws("enclosing", &["/p/a", "/p/b", "/p/c", "/q/1", "/q/2", "/q/3", "/q/4"]);
        let partial = ws("partial", &["/p/a", "/p/b"]);
        assert_eq!(match_workspaces(&editor, &[partial, enclosing]), Match::Enclosing { workspace: "enclosing".into() });
    }

    #[test]
    fn a_folder_inside_a_symlinked_project_is_enclosed_by_the_workspace_that_holds_the_real_one() {
        #[cfg(unix)]
        {
            let s = Scratch::new("enclose-link");
            let real = s.0.join("real");
            std::fs::create_dir_all(&real).unwrap();
            let link = s.0.join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let editor: BTreeSet<String> = [project_key(&link.to_string_lossy())].into();
            let workspace = (String::from("w"), BTreeSet::from([project_key(&real.to_string_lossy())]));
            assert_eq!(match_workspaces(&editor, &[workspace]), Match::Enclosing { workspace: "w".into() }, "a subset by symlink");
        }
    }

    #[test]
    fn no_shared_project_means_create() {
        let editor = set(&["/p/a"]);
        assert_eq!(match_workspaces(&editor, &[ws("w", &["/q/1"]), ws("v", &["/q/2"])]), Match::Create);
        assert_eq!(match_workspaces(&editor, &[]), Match::Create, "no workspace at all");
    }

    #[test]
    fn a_module_under_a_workspace_root_is_covered_by_the_workspace_that_holds_the_root() {
        // the editor opened /proj/mod-a and /proj/mod-b; the workspace holds /proj
        let editor = set(&["/proj/mod-a", "/proj/mod-b"]);
        let root = ws("root", &["/proj"]);
        assert_eq!(match_workspaces(&editor, &[ws("other", &["/x"]), root.clone()]), Match::Enclosing { workspace: "root".into() });
        // the fewest projects still wins among the workspaces that cover it
        let bigger = ws("bigger", &["/proj", "/q/1", "/q/2"]);
        assert_eq!(match_workspaces(&editor, &[bigger.clone(), root.clone()]), Match::Enclosing { workspace: "root".into() });
        // and a path of its own counts as before
        assert_eq!(match_workspaces(&set(&["/proj"]), &[root]), Match::Enclosing { workspace: "root".into() });
    }

    #[test]
    fn a_folder_inside_a_project_is_covered_by_the_workspace_that_holds_the_project() {
        // the stand-in of a folder that holds no project of its own is the folder itself
        let editor = set(&["/proj/src/main/java"]);
        assert_eq!(match_workspaces(&editor, &[ws("root", &["/proj"])]), Match::Enclosing { workspace: "root".into() });
    }

    #[test]
    fn nothing_that_covers_the_editors_projects_still_creates_and_a_path_prefix_is_not_an_ancestor() {
        let editor = set(&["/proj/mod-a"]);
        // "/proj2" and "/pro" start like "/proj" as text, and are not its ancestors
        assert_eq!(match_workspaces(&editor, &[ws("a", &["/proj2"]), ws("b", &["/pro"])]), Match::Create);
        // a workspace project INSIDE the editor's project does not cover it
        assert_eq!(match_workspaces(&editor, &[ws("c", &["/proj/mod-a/sub"])]), Match::Create);
    }

    #[test]
    fn when_only_some_of_the_editors_projects_are_covered_the_match_is_partial() {
        let editor = set(&["/proj/mod-a", "/proj/mod-b", "/other/x"]);
        assert_eq!(
            match_workspaces(&editor, &[ws("root", &["/proj"]), ws("one", &["/other/x"])]),
            Match::Partial { workspace: "root".into(), overlap: 2 },
            "two covered by the root, one by the other: the larger cover wins"
        );
    }

    #[test]
    fn a_partial_match_takes_the_largest_overlap_and_then_the_name() {
        let editor = set(&["/p/a", "/p/b", "/p/c"]);
        let two = ws("two", &["/p/a", "/p/b", "/q/1"]);
        let one = ws("one", &["/p/a", "/q/2"]);
        assert_eq!(match_workspaces(&editor, &[one.clone(), two.clone()]), Match::Partial { workspace: "two".into(), overlap: 2 });
        let also_two = ws("also-two", &["/p/b", "/p/c", "/q/3"]);
        for order in [vec![two.clone(), also_two.clone(), one.clone()], vec![also_two, two, one]] {
            assert_eq!(
                match_workspaces(&editor, &order),
                Match::Partial { workspace: "also-two".into(), overlap: 2 },
                "a tie is settled by name, whichever is listed first"
            );
        }
    }

    #[test]
    fn an_editor_without_a_java_project_is_none_java_whatever_the_workspaces_hold() {
        assert_eq!(match_workspaces(&set(&[]), &[ws("w", &["/p/a"])]), Match::NoneJava);
        assert_eq!(match_workspaces(&set(&[]), &[]), Match::NoneJava);
        assert_eq!(match_workspaces(&set(&[]), &[ws("empty", &[])]), Match::NoneJava, "an empty set is not enclosed by everything");
    }

    #[test]
    fn a_new_workspace_is_named_after_the_first_folder_and_made_unique_by_a_number() {
        assert_eq!(basename_of("/home/h/shop/"), "shop");
        assert_eq!(unique_workspace_name("shop", &[]), "shop");
        assert_eq!(unique_workspace_name("shop", &strings(&["shop"])), "shop-2");
        assert_eq!(unique_workspace_name("shop", &strings(&["shop", "shop-2", "Shop-3"])), "shop-4", "case does not make a name free");
        assert_eq!(unique_workspace_name("", &[]), "workspace");
        assert_eq!(unique_workspace_name("  ", &strings(&["workspace"])), "workspace-2");
    }

    // ----- the handoff file -----

    fn sample(state: HandoffState) -> Handoff {
        Handoff {
            folders: strings(&["/a/b", "/c"]),
            workspace: "shop".into(),
            url: "http://127.0.0.1:7001".into(),
            key: "k-1".into(),
            state,
            message: String::new(),
        }
    }

    #[test]
    fn the_file_is_named_by_the_id_in_a_folder_made_on_demand_and_holds_the_six_fields() {
        let s = Scratch::new("file");
        let dir = s.0.join("not").join("yet").join(HANDOFF_FOLDER);
        let path = write_handoff(&dir, &sample(HandoffState::Ready)).unwrap();
        assert_eq!(path, dir.join("5650d16fbb762aa37a51dc25636e82d35230e7da6a6d5e77c31dd3b4007de4be.json"));
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"folders": ["/a/b", "/c"], "workspace": "shop", "url": "http://127.0.0.1:7001", "key": "k-1", "state": "ready", "message": ""})
        );
        for (state, word) in [
            (HandoffState::Ready, "ready"),
            (HandoffState::Asking, "asking"),
            (HandoffState::Declined, "declined"),
            (HandoffState::None, "none"),
            (HandoffState::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), serde_json::json!(word));
        }
    }

    #[test]
    fn every_state_change_rewrites_the_file_and_leaves_no_temporary_file_behind() {
        let s = Scratch::new("rewrite");
        write_handoff(&s.0, &Handoff::waiting(&strings(&["/a"]), "w", HandoffState::Asking, "asking")).unwrap();
        let path = write_handoff(&s.0, &Handoff { folders: strings(&["/a"]), ..sample(HandoffState::Ready) }).unwrap();
        let now: Handoff = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(now.state, HandoffState::Ready);
        assert_eq!(now.url, "http://127.0.0.1:7001");
        let names: Vec<String> = std::fs::read_dir(&s.0).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 1, "only the final file is left: {names:?}");
        assert!(!names[0].contains("tmp"), "{names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_by_its_owner_only_and_is_replaced_by_a_rename_not_written_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let s = Scratch::new("mode");
        let folders = strings(&["/a/b", "/c"]);
        let path = s.0.join(format!("{}.json", handoff_id(&folders)));
        // an older file with wider rights, and a second name for the same inode: a write in place would change both
        std::fs::write(&path, "OLD").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let alias = s.0.join("alias");
        std::fs::hard_link(&path, &alias).unwrap();

        write_handoff(&s.0, &sample(HandoffState::Ready)).unwrap();

        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the key is the owner's alone");
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"state\": \"ready\""));
        assert_eq!(std::fs::read_to_string(&alias).unwrap(), "OLD", "the old inode was not touched: the new file arrived by a rename");
        // and a file written from nothing has the same rights
        let fresh = Scratch::new("mode-fresh");
        let made = write_handoff(&fresh.0, &sample(HandoffState::Ready)).unwrap();
        assert_eq!(std::fs::metadata(&made).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // ----- the rule, run against a fake studio -----

    #[derive(Default)]
    struct FakeHost {
        dir: PathBuf,
        workspaces: RefCell<Vec<(String, Vec<String>)>>,
        scans: BTreeMap<String, Vec<WorkspaceProjectCandidate>>,
        calls: RefCell<Vec<String>>,
        /// The readiness answers, one per look; the last one repeats.
        script: RefCell<VecDeque<Readiness>>,
        /// What the handoff file said at each look (None = no file): the proof that `ready` is never early.
        seen_at_look: RefCell<Vec<Option<HandoffState>>>,
        start_fails: bool,
        add_fails: bool,
        /// A managed engine that is not installed: starts are refused until `install_after_looks` looks have been made at
        /// whether it is (usize::MAX = never).
        engine_missing: bool,
        install_after_looks: usize,
        looks: std::cell::Cell<usize>,
    }

    impl FakeHost {
        fn new(dir: &Path) -> Self {
            FakeHost { dir: dir.to_path_buf(), ..Default::default() }
        }
        fn with_workspace(self, name: &str, paths: &[&str]) -> Self {
            self.workspaces.borrow_mut().push((name.to_string(), strings(paths)));
            self
        }
        fn with_scan(mut self, folder: &str, projects: &[&str]) -> Self {
            self.scans.insert(folder.to_string(), projects.iter().map(|p| candidate(p)).collect());
            self
        }
        fn script(self, answers: Vec<Readiness>) -> Self {
            *self.script.borrow_mut() = answers.into();
            self
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        /// Records what the handoff file says right now (None = no file), at each look the studio makes.
        fn glance(&self) {
            let on_disk = std::fs::read_dir(&self.dir)
                .ok()
                .and_then(|mut rd| rd.find_map(|e| e.ok()).map(|e| e.path()))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| serde_json::from_str::<Handoff>(&t).unwrap().state);
            self.seen_at_look.borrow_mut().push(on_disk);
        }
        fn is_missing(&self) -> bool {
            self.engine_missing && self.looks.get() < self.install_after_looks
        }
        fn handoff(&self, folders: &[String]) -> Option<Handoff> {
            std::fs::read_to_string(handoff_path(&self.dir, &handoff_id(folders))).ok().map(|t| serde_json::from_str(&t).unwrap())
        }
    }

    impl EditorHost for FakeHost {
        fn handoff_dir(&self) -> PathBuf {
            self.dir.clone()
        }
        fn workspaces(&self) -> Vec<(String, Vec<String>)> {
            self.workspaces.borrow().clone()
        }
        fn scan_folder(&self, folder: &str) -> Result<Vec<WorkspaceProjectCandidate>, String> {
            self.scans.get(folder).cloned().ok_or_else(|| format!("not a directory: {folder}"))
        }
        fn add_projects(&self, workspace: &str, projects: &[WorkspaceProjectCandidate]) -> Result<Vec<String>, String> {
            self.calls.borrow_mut().push(format!("add {workspace} {}", projects.iter().map(|p| p.project_path.as_str()).collect::<Vec<_>>().join(",")));
            if self.add_fails {
                return Err("the project list is locked".into());
            }
            let mut all = self.workspaces.borrow_mut();
            let paths = projects.iter().map(|p| p.project_path.clone());
            match all.iter_mut().find(|(n, _)| n == workspace) {
                Some((_, existing)) => existing.extend(paths),
                None => all.push((workspace.to_string(), paths.collect())),
            }
            Ok(vec![])
        }
        fn engine_missing(&self) -> bool {
            self.looks.set(self.looks.get() + 1);
            self.glance();
            self.is_missing()
        }
        fn start(&self, workspace: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("start {workspace}"));
            if self.is_missing() {
                return Err("the managed runtime is absent".into());
            }
            if self.start_fails {
                Err("no engine installed".into())
            } else {
                Ok(())
            }
        }
        fn readiness(&self, _workspace: &str) -> Readiness {
            let mut script = self.script.borrow_mut();
            let answer = if script.len() > 1 { script.pop_front().unwrap() } else { script.front().cloned().unwrap_or(Readiness::Ready) };
            self.glance();
            answer
        }
        fn endpoint(&self, workspace: &str) -> Result<(String, String), String> {
            Ok(("http://127.0.0.1:7100".to_string(), format!("key-of-{workspace}")))
        }
    }

    #[derive(Default)]
    struct FakeUi {
        asked: RefCell<Vec<UpdatePrompt>>,
        prompts_changed: RefCell<u32>,
        workspaces_changed: RefCell<u32>,
        restored: RefCell<u32>,
    }
    impl EditorUi for FakeUi {
        fn ask(&self, prompt: &UpdatePrompt) {
            self.asked.borrow_mut().push(prompt.clone());
        }
        fn prompts_changed(&self) {
            *self.prompts_changed.borrow_mut() += 1;
        }
        fn workspaces_changed(&self) {
            *self.workspaces_changed.borrow_mut() += 1;
        }
        fn restore_last_running(&self) {
            *self.restored.borrow_mut() += 1;
        }
    }

    const QUICK: Pacing = Pacing { poll: Duration::from_millis(1), give_up: Duration::from_millis(30), install_wait: Duration::from_millis(40) };

    fn launch(host: &FakeHost, ui: &FakeUi, book: &PromptBook, folders: &[&str]) {
        run_editor_launch(host, ui, book, &strings(folders), QUICK);
    }

    #[test]
    fn an_enclosing_workspace_that_is_stopped_is_started_and_ready_is_written_only_when_it_answers() {
        let s = Scratch::new("enclosing");
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/p/a", "/p/b", "/p/c"])
            .with_workspace("other", &["/q/1"])
            .with_scan("/p", &["/p/a", "/p/b"])
            .script(vec![Readiness::Waiting, Readiness::Waiting, Readiness::Waiting, Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/p"]);

        assert_eq!(host.calls(), vec!["start shop"], "the workspace is started; nothing is added or created");
        let file = host.handoff(&strings(&["/p"])).expect("the file is written");
        assert_eq!(file.state, HandoffState::Ready);
        assert_eq!((file.workspace.as_str(), file.url.as_str(), file.key.as_str()), ("shop", "http://127.0.0.1:7100", "key-of-shop"));
        assert_eq!(file.message, "", "a ready file has an empty message");
        let looks = host.seen_at_look.borrow().clone();
        assert!(looks.len() >= 4, "the studio looked until the resident answered: {looks:?}");
        assert!(looks.iter().all(|state| *state != Some(HandoffState::Ready)), "no ready file existed while the resident was not answering: {looks:?}");
        assert!(ui.asked.borrow().is_empty(), "no window");
    }

    #[test]
    fn an_enclosing_workspace_that_already_answers_is_not_started_again() {
        let s = Scratch::new("running");
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        launch(&host, &ui, &book, &["/p"]);
        assert!(host.calls().is_empty(), "{:?}", host.calls());
        assert_eq!(host.handoff(&strings(&["/p"])).unwrap().state, HandoffState::Ready);
    }

    #[test]
    fn a_stale_file_from_an_earlier_launch_is_gone_before_the_new_answer_is_known() {
        let s = Scratch::new("stale");
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![Readiness::Waiting, Readiness::Ready]);
        write_handoff(&s.0, &Handoff { folders: strings(&["/p"]), ..sample(HandoffState::Ready) }).unwrap();
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        launch(&host, &ui, &book, &["/p"]);
        let looks = host.seen_at_look.borrow().clone();
        assert_eq!(looks.first(), Some(&None), "the old ready file was removed before the first look: {looks:?}");
    }

    #[test]
    fn no_workspace_at_all_creates_one_after_the_first_folder_and_starts_it_without_a_window() {
        let s = Scratch::new("create");
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/other/x"])
            .with_scan("/home/h/shop", &["/home/h/shop/api", "/home/h/shop/web"])
            .with_scan("/libs", &["/libs/util"])
            .script(vec![Readiness::Waiting, Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/home/h/shop", "/libs"]);

        assert_eq!(
            host.calls(),
            vec!["add shop-2 /home/h/shop/api,/home/h/shop/web,/libs/util", "start shop-2"],
            "named after the first folder, made unique, every project of every folder imported, then started"
        );
        let file = host.handoff(&strings(&["/home/h/shop", "/libs"])).unwrap();
        assert_eq!((file.state, file.workspace.as_str()), (HandoffState::Ready, "shop-2"));
        assert!(ui.asked.borrow().is_empty(), "no window");
        assert!(book.list().is_empty());
    }

    #[test]
    fn a_partly_matching_workspace_asks_and_changes_nothing_until_the_answer() {
        let s = Scratch::new("partial");
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/p/a", "/p/b", "/q/old"])
            .with_workspace("lib", &["/p/c"])
            .with_scan("/p", &["/p/a", "/p/b", "/p/c", "/p/d"])
            .script(vec![Readiness::Waiting, Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/p"]);

        assert!(host.calls().is_empty(), "nothing is added or started before the answer: {:?}", host.calls());
        let asked = ui.asked.borrow().clone();
        assert_eq!(asked.len(), 1, "the window opens once, with the question");
        let q = &asked[0];
        assert_eq!(q.workspace, "shop");
        assert_eq!(q.existing, strings(&["/p/a", "/p/b"]));
        assert_eq!(q.missing, strings(&["/p/d"]), "what is free is added");
        assert_eq!(q.elsewhere, strings(&["/p/c (in workspace lib)"]), "what another workspace holds stays there");
        assert_eq!(q.id, handoff_id(&strings(&["/p"])));
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!((file.state, file.workspace.as_str(), file.url.as_str()), (HandoffState::Asking, "shop", ""));
        assert!(!file.message.is_empty());
        assert_eq!(book.list(), vec![q.clone()]);

        // "Update workspace": the missing project is added, the workspace is started, ready is written when it answers
        let pending = book.take(&q.id).expect("the question is open");
        answer_update(&host, &ui, pending, true, QUICK);
        assert_eq!(host.calls(), vec!["add shop /p/d", "start shop"]);
        assert_eq!(host.handoff(&strings(&["/p"])).unwrap().state, HandoffState::Ready);
        assert!(book.list().is_empty());
    }

    #[test]
    fn not_now_writes_declined_and_changes_nothing() {
        let s = Scratch::new("declined");
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a", "/p/b"]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        launch(&host, &ui, &book, &["/p"]);
        assert_eq!(ui.asked.borrow().len(), 1, "control: the question was asked");
        let pending = book.take(&handoff_id(&strings(&["/p"]))).unwrap();

        answer_update(&host, &ui, pending, false, QUICK);

        assert!(host.calls().is_empty(), "{:?}", host.calls());
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!((file.state, file.workspace.as_str(), file.key.as_str()), (HandoffState::Declined, "shop", ""));
        assert!(book.take("anything").is_none() && book.list().is_empty());
    }

    #[test]
    fn a_partial_workspace_whose_missing_projects_all_belong_elsewhere_is_started_without_a_question() {
        let s = Scratch::new("elsewhere");
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/p/a"])
            .with_workspace("lib", &["/p/b"])
            .with_scan("/p", &["/p/a", "/p/b"])
            .script(vec![Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        launch(&host, &ui, &book, &["/p"]);
        assert!(ui.asked.borrow().is_empty(), "there is nothing to add, so nothing to ask");
        assert!(host.calls().is_empty(), "already answering");
        let file = host.handoff(&strings(&["/p"])).unwrap();
        // both share one project; the tie is settled by name
        assert_eq!((file.state, file.workspace.as_str()), (HandoffState::Ready, "lib"));
    }

    #[test]
    fn folders_without_a_java_project_start_nothing_and_say_none() {
        let s = Scratch::new("none");
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/docs", &[]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        launch(&host, &ui, &book, &["/docs", "/missing"]);
        assert!(host.calls().is_empty(), "{:?}", host.calls());
        let file = host.handoff(&strings(&["/docs", "/missing"])).unwrap();
        assert_eq!(file.state, HandoffState::None);
        assert!(file.message.contains("/missing"), "the folder that could not be read is named: {}", file.message);
        assert!(ui.asked.borrow().is_empty() && book.list().is_empty());
    }

    #[test]
    fn a_workspace_that_cannot_start_or_never_answers_is_a_failed_file_with_the_reason() {
        let s = Scratch::new("failed");
        let mut host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![Readiness::Waiting]);
        host.start_fails = true;
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        launch(&host, &ui, &book, &["/p"]);
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!(file.state, HandoffState::Failed);
        assert!(file.message.contains("no engine installed"), "{}", file.message);

        // never answers: the wait is given up, and the file says so
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![Readiness::Waiting]);
        launch(&host, &ui, &book, &["/p"]);
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!(file.state, HandoffState::Failed);
        assert!(file.message.contains("did not answer within"), "{}", file.message);
        assert_eq!(file.url, "", "no address is given for a resident that did not answer");

        // a resident that failed on the way up
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/p/a"])
            .with_scan("/p", &["/p/a"])
            .script(vec![Readiness::Waiting, Readiness::Failed("port in use".into())]);
        launch(&host, &ui, &book, &["/p"]);
        assert!(host.handoff(&strings(&["/p"])).unwrap().message.contains("port in use"));
    }

    #[test]
    fn modules_under_a_workspace_root_start_that_workspace_and_register_nothing() {
        let s = Scratch::new("module-under-root");
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/proj"])
            .with_scan("/proj/mod-a", &["/proj/mod-a"])
            .with_scan("/proj/mod-b", &["/proj/mod-b"])
            .script(vec![Readiness::Waiting, Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/proj/mod-a", "/proj/mod-b"]);

        assert_eq!(host.calls(), vec!["start shop"], "no second workspace is made for modules the first one holds");
        assert_eq!(host.handoff(&strings(&["/proj/mod-a", "/proj/mod-b"])).unwrap().state, HandoffState::Ready);
        assert!(ui.asked.borrow().is_empty());
    }

    #[test]
    fn a_folder_inside_a_project_with_no_project_of_its_own_is_covered_and_not_none() {
        let s = Scratch::new("inside-project");
        // the scan of /proj/src finds no project: it is read, and empty
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/proj"]).with_scan("/proj/src", &[]).script(vec![Readiness::Ready]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/proj/src"]);

        let file = host.handoff(&strings(&["/proj/src"])).unwrap();
        assert_eq!((file.state, file.workspace.as_str()), (HandoffState::Ready, "shop"), "{}", file.message);
        assert!(host.calls().is_empty(), "nothing is added: the workspace covers the folder; {:?}", host.calls());
        assert_eq!(*ui.restored.borrow(), 0);
        // the control: a folder that is inside no workspace project is still none
        let host = FakeHost::new(&s.0).with_workspace("shop", &["/proj"]).with_scan("/elsewhere/docs", &[]);
        launch(&host, &ui, &book, &["/elsewhere/docs"]);
        assert_eq!(host.handoff(&strings(&["/elsewhere/docs"])).unwrap().state, HandoffState::None);
    }

    #[test]
    fn a_folder_standing_in_a_held_project_is_listed_as_existing_and_never_offered_for_adding() {
        let s = Scratch::new("stand-in");
        let host = FakeHost::new(&s.0)
            .with_workspace("shop", &["/p/a"])
            .with_scan("/p", &["/p/a", "/p/b"])
            .with_scan("/p/a/src", &[]);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/p", "/p/a/src"]);

        let asked = ui.asked.borrow().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].existing, strings(&["/p/a", "/p/a/src"]));
        assert_eq!(asked[0].missing, strings(&["/p/b"]), "only a real project is offered");
        assert!(host.calls().is_empty());
    }

    #[test]
    fn every_end_without_a_resident_brings_the_last_run_back_once_and_no_other_end_does() {
        let s = Scratch::new("restore");
        let (book, never) = (PromptBook::default(), Readiness::Waiting);
        let restored_after = |host: FakeHost, folders: &[&str]| {
            let ui = FakeUi::default();
            launch(&host, &ui, &book, folders);
            let n = *ui.restored.borrow();
            n
        };
        // the ends that leave the extension without a resident: each restores exactly once
        assert_eq!(restored_after(FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/docs", &[]), &["/docs"]), 1, "none");
        let mut failed_start = FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![never.clone()]);
        failed_start.start_fails = true;
        assert_eq!(restored_after(failed_start, &["/p"]), 1, "failed: it could not be started");
        let silent = FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![never.clone()]);
        assert_eq!(restored_after(silent, &["/p"]), 1, "failed: it never answered");
        let broken = FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![never, Readiness::Failed("port in use".into())]);
        assert_eq!(restored_after(broken, &["/p"]), 1, "failed: it came up broken");
        let mut no_create = FakeHost::new(&s.0).with_scan("/p", &["/p/a"]);
        no_create.add_fails = true;
        assert_eq!(restored_after(no_create, &["/p"]), 1, "failed: the workspace could not be made");
        // the ends that give the extension a resident, or are still open: none restores
        assert_eq!(restored_after(FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a"]), &["/p"]), 0, "enclosing, ready");
        assert_eq!(restored_after(FakeHost::new(&s.0).with_scan("/p", &["/p/a"]), &["/p"]), 0, "created, ready");
        assert_eq!(restored_after(FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a", "/p/b"]), &["/p"]), 0, "asking is not an end");
    }

    #[test]
    fn the_answer_to_the_question_restores_on_not_now_and_on_a_failed_update_and_not_on_a_ready_one() {
        let s = Scratch::new("restore-answer");
        let book = PromptBook::default();
        let ask = |add_fails: bool| {
            let mut host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a", "/p/b"]);
            host.add_fails = add_fails;
            let ui = FakeUi::default();
            launch(&host, &ui, &book, &["/p"]);
            assert_eq!(*ui.restored.borrow(), 0, "control: asking restores nothing");
            let pending = book.take(&handoff_id(&strings(&["/p"]))).unwrap();
            (host, ui, pending)
        };
        let (host, ui, pending) = ask(false);
        answer_update(&host, &ui, pending, false, QUICK);
        assert_eq!(*ui.restored.borrow(), 1, "Not now (declined)");
        let (host, ui, pending) = ask(true);
        answer_update(&host, &ui, pending, true, QUICK);
        assert_eq!(*ui.restored.borrow(), 1, "the update failed");
        let (host, ui, pending) = ask(false);
        answer_update(&host, &ui, pending, true, QUICK);
        assert_eq!(*ui.restored.borrow(), 0, "the update came up ready");
    }

    // ----- an engine that is not installed yet -----

    /// A fresh install: the first start is refused because the managed engine is not there, and the studio has it a few
    /// looks later. The launch must wait for it, write nothing meanwhile, and ask for the start once more.
    fn fresh_install(s: &Scratch, installs_after: usize) -> FakeHost {
        let mut host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![
            Readiness::Waiting,
            Readiness::Waiting,
            Readiness::Ready,
        ]);
        host.engine_missing = true;
        host.install_after_looks = installs_after;
        host
    }

    #[test]
    fn a_start_refused_for_want_of_the_engine_waits_for_the_install_and_asks_again() {
        let s = Scratch::new("fresh-install");
        let host = fresh_install(&s, 3);
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/p"]);

        assert_eq!(host.calls(), vec!["start shop", "start shop"], "refused once, asked once more");
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!((file.state, file.workspace.as_str(), file.key.as_str()), (HandoffState::Ready, "shop", "key-of-shop"), "{}", file.message);
        let looks = host.seen_at_look.borrow().clone();
        assert!(looks.len() >= 5, "the install was polled and the resident watched: {looks:?}");
        assert!(looks.iter().all(Option::is_none), "no answer file existed at any look while the install was awaited: {looks:?}");
        assert_eq!(*ui.restored.borrow(), 0, "nothing failed");
        assert!(ui.asked.borrow().is_empty());
    }

    #[test]
    fn a_new_workspace_on_a_fresh_install_waits_for_the_engine_too() {
        let s = Scratch::new("fresh-install-create");
        let mut host = FakeHost::new(&s.0).with_scan("/home/h/shop", &["/home/h/shop/api"]).script(vec![Readiness::Waiting, Readiness::Ready]);
        host.engine_missing = true;
        host.install_after_looks = 2;
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/home/h/shop"]);

        assert_eq!(host.calls(), vec!["add shop /home/h/shop/api", "start shop", "start shop"]);
        assert_eq!(host.handoff(&strings(&["/home/h/shop"])).unwrap().state, HandoffState::Ready);
    }

    #[test]
    fn an_engine_that_never_arrives_ends_in_failed_within_the_bound_and_names_the_download() {
        let s = Scratch::new("never-installs");
        let host = fresh_install(&s, usize::MAX);
        let (ui, book) = (FakeUi::default(), PromptBook::default());
        let began = std::time::Instant::now();

        launch(&host, &ui, &book, &["/p"]);

        assert!(began.elapsed() < Duration::from_secs(2), "the bound is the injected one, not 120 s: {:?}", began.elapsed());
        assert_eq!(host.calls(), vec!["start shop", "start shop"], "one more request after the bound, and no more");
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!(file.state, HandoffState::Failed);
        assert!(file.message.contains("the engine download did not finish within"), "names the download and that it timed out: {}", file.message);
        assert_eq!(file.url, "");
        assert_eq!(*ui.restored.borrow(), 1, "a failed end brings the last run back, as before");
        let looks = host.seen_at_look.borrow().clone();
        assert!(looks.iter().all(Option::is_none), "nothing was written during the wait: {looks:?}");
    }

    #[test]
    fn an_engine_that_arrives_but_whose_start_is_refused_again_says_the_download_finished() {
        let s = Scratch::new("installs-then-refused");
        let mut host = fresh_install(&s, 2);
        host.start_fails = true; // once the engine is there, the start fails for another reason
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/p"]);

        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!(file.state, HandoffState::Failed);
        assert!(file.message.contains("the engine download finished, but the start was refused again"), "{}", file.message);
        assert_eq!(host.calls(), vec!["start shop", "start shop"]);
    }

    #[test]
    fn a_start_refused_for_any_other_reason_is_not_retried_and_does_not_wait() {
        let s = Scratch::new("other-refusal");
        let mut host = FakeHost::new(&s.0).with_workspace("shop", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![Readiness::Waiting]);
        host.start_fails = true; // the engine is installed: engine_missing is false
        let (ui, book) = (FakeUi::default(), PromptBook::default());

        launch(&host, &ui, &book, &["/p"]);

        assert_eq!(host.calls(), vec!["start shop"], "one request only");
        let file = host.handoff(&strings(&["/p"])).unwrap();
        assert_eq!(file.state, HandoffState::Failed);
        assert!(file.message.contains("no engine installed"), "the refusal's own words reach the file: {}", file.message);
        assert_eq!(host.looks.get(), 1, "the engine was asked about once, to decide, and never polled");
        assert_eq!(*ui.restored.borrow(), 1);
    }

    // ----- two launches at once -----

    /// A studio that two threads can use at once. Registering a project takes a moment, as it does on disk.
    struct SharedHost {
        dir: PathBuf,
        workspaces: Mutex<Vec<(String, Vec<String>)>>,
        scans: BTreeMap<String, Vec<WorkspaceProjectCandidate>>,
    }
    impl EditorHost for SharedHost {
        fn handoff_dir(&self) -> PathBuf {
            self.dir.clone()
        }
        fn workspaces(&self) -> Vec<(String, Vec<String>)> {
            self.workspaces.lock().unwrap().clone()
        }
        fn scan_folder(&self, folder: &str) -> Result<Vec<WorkspaceProjectCandidate>, String> {
            self.scans.get(folder).cloned().ok_or_else(|| format!("not a directory: {folder}"))
        }
        fn add_projects(&self, workspace: &str, projects: &[WorkspaceProjectCandidate]) -> Result<Vec<String>, String> {
            std::thread::sleep(Duration::from_millis(60));
            let mut all = self.workspaces.lock().unwrap();
            let paths = projects.iter().map(|p| p.project_path.clone());
            match all.iter_mut().find(|(n, _)| n == workspace) {
                Some((_, existing)) => existing.extend(paths),
                None => all.push((workspace.to_string(), paths.collect())),
            }
            Ok(vec![])
        }
        fn start(&self, _workspace: &str) -> Result<(), String> {
            Ok(())
        }
        fn engine_missing(&self) -> bool {
            false
        }
        fn readiness(&self, _workspace: &str) -> Readiness {
            Readiness::Ready
        }
        fn endpoint(&self, workspace: &str) -> Result<(String, String), String> {
            Ok(("http://127.0.0.1:7100".to_string(), format!("key-of-{workspace}")))
        }
    }

    #[test]
    fn two_launches_for_different_folders_with_the_same_first_name_get_two_workspaces() {
        let s = Scratch::new("simultaneous");
        let mut scans = BTreeMap::new();
        scans.insert("/a/shop".to_string(), vec![candidate("/a/shop/x")]);
        scans.insert("/b/shop".to_string(), vec![candidate("/b/shop/x")]);
        let host = SharedHost { dir: s.0.clone(), workspaces: Mutex::new(vec![]), scans };

        std::thread::scope(|scope| {
            for folder in ["/a/shop", "/b/shop"] {
                let host = &host;
                scope.spawn(move || {
                    let (ui, book) = (FakeUi::default(), PromptBook::default());
                    run_editor_launch(host, &ui, &book, &strings(&[folder]), QUICK);
                });
            }
        });

        let mut made: Vec<(String, Vec<String>)> = host.workspaces.lock().unwrap().clone();
        made.sort();
        assert_eq!(made.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["shop", "shop-2"], "two workspaces with distinct names: {made:?}");
        assert!(made.iter().all(|(_, projects)| projects.len() == 1), "each holds only its own project: {made:?}");
        assert_ne!(made[0].1, made[1].1, "and not the same one");
    }

    #[test]
    fn the_real_window_brings_the_last_run_back_through_the_service_and_raises_no_window_for_it() {
        let whole = include_str!("editor_launch.rs");
        let code = whole.split("#[cfg(test)]\nmod tests").next().unwrap();
        let real = code.split("fn restore_last_running(&self) {").nth(1).expect("the real ui restores").split("\n    }\n").next().unwrap();
        assert!(real.contains("manager_service.restore_last_running_workspaces()"), "{real}");
        assert!(!real.contains("show_main_window"), "the restore opens no window");
        // the ends that call it, and only those: the helper is the one place, and ready never goes through it
        assert_eq!(code.matches("ui.restore_last_running()").count(), 1);
        let helper = code.split("fn end_without_resident(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(helper.contains("ui.restore_last_running()"));
        // write_state is the definition, the helper's call and the Asking write: no none, declined or failed answer is written
        // without going through the helper
        assert_eq!(code.matches("write_state(").count(), 3);
    }

    #[test]
    fn the_create_lock_is_released_before_the_wait_for_a_resident() {
        let whole = include_str!("editor_launch.rs");
        let code = whole.split("#[cfg(test)]\nmod tests").next().unwrap();
        let create = code.split("Match::Create => {").nth(1).unwrap().split("Match::Partial {").next().unwrap();
        assert!(create.contains("request_start_once(host, &name)"), "the request to start is made under the lock");
        assert!(create.find("settle_start(").unwrap() < create.find("wait_until_ready(").unwrap());
        // the wait for an engine to be installed (inside settle_start) is outside the lock too
        let between = &create[create.find("request_start_once(").unwrap()..create.find("settle_start(").unwrap()];
        assert!(
            between.contains("drop(create_guard)"),
            "the lock is dropped between the request to start and the wait, so one launch waiting for a resident does not hold up another"
        );
        assert!(code.contains("static CREATE_LOCK: Mutex<()>"));
    }

    // ----- no window except by a question -----

    #[test]
    fn only_a_partly_matching_workspace_opens_the_window_and_the_other_three_cases_do_not() {
        let s = Scratch::new("window");
        let ui = FakeUi::default();
        let book = PromptBook::default();
        let cases: [(&str, FakeHost); 4] = [
            ("enclosing", FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a"])),
            ("create", FakeHost::new(&s.0).with_scan("/p", &["/p/a"])),
            ("none", FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &[])),
            ("failed start", { let mut h = FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a"]).script(vec![Readiness::Waiting]); h.start_fails = true; h }),
        ];
        for (label, host) in cases {
            launch(&host, &ui, &book, &["/p"]);
            assert!(ui.asked.borrow().is_empty(), "{label}: the window stays closed");
        }
        launch(&FakeHost::new(&s.0).with_workspace("w", &["/p/a"]).with_scan("/p", &["/p/a", "/p/b"]), &ui, &book, &["/p"]);
        assert_eq!(ui.asked.borrow().len(), 1, "a partial match is the one case that opens it");
    }

    /// The window parts cannot be run without a display, so the wiring is held in the source: of this file's own code (the
    /// text before its test module, so this test cannot read itself) exactly one place raises the window, and it is the
    /// real window's `ask`, which the rule calls only for a partly matching workspace.
    #[test]
    fn the_only_call_that_raises_the_window_is_the_real_ask_and_the_rule_reaches_it_only_for_a_partial_match() {
        let whole = include_str!("editor_launch.rs");
        let code = whole.split("#[cfg(test)]\nmod tests").next().expect("the code part");
        assert!(code.contains("pub fn handle_editor_launch") && !code.contains("mod tests"), "control: the region is the code and not the tests");
        assert_eq!(code.matches("show_main_window").count(), 1, "one call in the whole code");
        let ask = code.split("impl<R: Runtime> EditorUi for AppUi<R> {").nth(1).expect("the real ui");
        let ask = ask.split("fn prompts_changed").next().unwrap();
        assert!(ask.contains("show_main_window(&self.app)"), "and it is inside AppUi::ask");
        // inside the rule, `ui.ask(` appears once, in the Partial arm, after the question is written and put in the book
        let rule = code.split("pub fn run_editor_launch(").nth(1).unwrap().split("pub fn answer_update(").next().unwrap();
        assert_eq!(rule.matches("ui.ask(").count(), 1);
        let partial = rule.split("Match::Partial {").nth(1).unwrap();
        assert!(partial.contains("ui.ask(&prompt)"), "the ask is in the Partial arm");
        for arm in ["Match::NoneJava => {", "Match::Enclosing { workspace } => {", "Match::Create => {"] {
            let body = rule.split(arm).nth(1).unwrap().split("Match::").next().unwrap();
            assert!(!body.contains("ui.ask("), "{arm} never asks");
        }
        // a launch that carries folders is judged by its own arguments, not by the background flag
        assert!(!code.contains("is_background_launch"), "this file does not decide windows by the flag; the callback does");
    }
}
