//! The VS Code half of "Deploy to agents" (Sprint 28h, Stage 21 (c)).
//!
//! Deploying to VS Code does four things, and "Remove from agents" takes out exactly those:
//!
//! 1. installs two extensions with `<editor> --install-extension <file.vsix> --force`: the chat
//!    (`jawata-chat-<v>.vsix`) and jawata Java (`jawata-java-<v>.vsix`), from the assets of the latest
//!    release of `haraldwegner/jawata-chat`, or from a local folder named by `JAWATA_STUDIO_VSIX_DIR`. Every editor build
//!    on PATH (`code`, `codium`, `code-oss`, D56 (c)) is served through its own command; the folder its extensions land in is
//!    read from the build's own `product.json` (`~/<dataFolderName>/extensions`), and two builds sharing a folder are served once;
//! 2. writes the editor settings the extensions read: `jawataJava.server` (the language-server launch,
//!    `--lsp`, the resident's address and key in the environment, never in the arguments) and
//!    `jawataChat.studio` / `jawataJava.studio`, how an extension starts this studio when the engine's
//!    service does not answer: `{command: <this studio>, args: ["--background"], handoffDir: <folder>}` (no window, only the
//!    tray; the extension adds one `--editor-folder <path>` per folder of its workspace, and reads the answer, the matched
//!    jawata workspace's address and key, from `<handoffDir>/<id>.json`, see `editor_launch`). (`jawataChat.agent` is the
//!    existing chat row's, `agent_registration`.) Text edits, so comments and every other key stay;
//! 3. removes Microsoft's Project Manager for Java (`vscjava.vscode-java-dependency`) once jawata Java is in, so the Java
//!    Projects view shows once (D54 (d)); then removes Red Hat's Java extension (`redhat.java`) and every installed extension
//!    that needs it (its `extensionDependencies` or `extensionPack`, followed through), each before what it needs, because
//!    jawata Java brings its own Testing view and debugger (D56 (b)); and writes `chat.disableAIFeatures` so that Copilot is gone
//!    from VS Code (D53 (p));
//! 4. leaves Cursor alone: only editor commands found on PATH are run, and one that lives in a Cursor folder is refused.
//!
//! Everything that touches the machine takes an [`EditorEnv`] (the PATH, the local `.vsix` folder, the release
//! address, this studio's own path), so a test runs the whole thing against a temp folder, a fake `code` script
//! and a local HTTP server and never reaches the real editor, the network or the real settings.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent_registration::{
    finish, is_ours_launched_with, parse_object, read_root, render, rows, Launch, RowAction, RowOutcome, RowState, RowTarget, CHAT_ROW,
};
use crate::jsonc_edit;

pub const CHAT_EXTENSION_ID: &str = "jawata.jawata-chat";
pub const JAVA_EXTENSION_ID: &str = "jawata.jawata-java";
/// The extension that competes with jawata Java for the same files. The deploy removes it, with every extension that needs it,
/// once jawata Java is in (D56 (b)), and never installs it.
pub const RIVAL_EXTENSION_ID: &str = "redhat.java";
/// Microsoft's Project Manager for Java. It contributes a second "Java Projects" view beside the one jawata Java brings, so every project is
/// listed twice (D54 (d), his words: "We have Java projetcs twice"). The deploy removes it once jawata Java is in.
pub const PROJECT_MANAGER_EXTENSION_ID: &str = "vscjava.vscode-java-dependency";
/// The repository whose latest release carries the two `.vsix` files.
pub const EXTENSIONS_REPO: &str = "haraldwegner/jawata-chat";
/// A folder of local `.vsix` files, used instead of the network (tests, offline use).
pub const VSIX_DIR_ENV: &str = "JAWATA_STUDIO_VSIX_DIR";

/// VS Code's own switch for Copilot's chat and suggestions (D53 (p)).
pub const AI_OFF_KEY: &str = "chat.disableAIFeatures";
/// Written beside it when the deploy set it, so Remove takes back only what the deploy wrote.
pub const AI_OFF_BY_DEPLOY_KEY: &str = "jawataChat.copilotOffByDeploy";

pub const JAVA_SERVER_KEY: &str = "jawataJava.server";
pub const CHAT_STUDIO_KEY: &str = "jawataChat.studio";
pub const JAVA_STUDIO_KEY: &str = "jawataJava.studio";
/// The settings this module writes (the chat row writes `jawataChat.agent`).
pub const EDITOR_KEYS: [&str; 3] = [JAVA_SERVER_KEY, CHAT_STUDIO_KEY, JAVA_STUDIO_KEY];

/// The result ids this part reports under, beside the roster's `vscode` and the chat row `jawata-chat`.
pub const EXTENSIONS_ROW: &str = "vscode-extensions";
pub const SETTINGS_ROW: &str = "vscode-settings";

/// What the machine looks like to this module. Production reads the process; a test supplies its own.
#[derive(Debug, Clone)]
pub struct EditorEnv {
    /// The `PATH` in which `code` is looked for.
    pub path_var: Option<OsString>,
    /// `JAWATA_STUDIO_VSIX_DIR`: local `.vsix` files used instead of the network.
    pub vsix_dir: Option<PathBuf>,
    /// The releases API address of the extensions' repository; `None` means no network is used.
    pub release_api: Option<String>,
    /// How an extension starts this studio.
    pub studio: Option<PathBuf>,
    /// The home folder the editors keep their extensions under (`<home>/<dataFolderName>/extensions`).
    pub home: Option<PathBuf>,
    /// How long one `code` call may take.
    pub step_timeout: Duration,
}

impl EditorEnv {
    pub fn from_process() -> Self {
        let home = dirs::home_dir();
        EditorEnv {
            path_var: std::env::var_os("PATH"),
            vsix_dir: std::env::var_os(VSIX_DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from),
            release_api: Some(format!("https://api.github.com/repos/{EXTENSIONS_REPO}/releases/latest")),
            studio: studio_command(
                std::env::var("APPIMAGE").ok().as_deref(),
                home.as_deref(),
                std::env::current_exe().ok().as_deref(),
                &|p| p.is_file(),
            ),
            home,
            step_timeout: Duration::from_secs(180),
        }
    }

    /// An environment that cannot reach the real machine: no `code` on PATH, no folder, no network.
    #[cfg(test)]
    pub fn sandbox() -> Self {
        EditorEnv {
            path_var: None,
            vsix_dir: None,
            release_api: None,
            studio: Some(PathBuf::from("/sandbox/jawata-studio")),
            home: None,
            step_timeout: Duration::from_secs(20),
        }
    }
}

/// The program an extension runs to start this studio. The stable installed path is preferred over
/// whatever `current_exe` says: an AppImage runs from a mount that vanishes when it exits (`APPIMAGE` names
/// the file), and the installer puts the file at `~/.local/bin/jawata-studio`.
pub fn studio_command(
    appimage: Option<&str>,
    home: Option<&Path>,
    current_exe: Option<&Path>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(image) = appimage.filter(|a| !a.is_empty()).map(PathBuf::from) {
        if exists(&image) {
            return Some(image);
        }
    }
    let running_from_a_build = current_exe
        .map(|exe| {
            let p = exe.to_string_lossy();
            p.contains("/target/debug/") || p.contains("/target/release/") || p.contains("\\target\\debug\\") || p.contains("\\target\\release\\")
        })
        .unwrap_or(false);
    if !running_from_a_build {
        if let Some(installed) = home.map(|h| h.join(".local").join("bin").join("jawata-studio")) {
            // The installer's file is only the stable path when this very process is not running from elsewhere
            // on purpose (a dev build), so it is consulted only after the AppImage and before a mount path.
            if exists(&installed) && current_exe.map_or(true, |exe| looks_like_a_mount(exe)) {
                return Some(installed);
            }
        }
    }
    current_exe.map(Path::to_path_buf)
}

fn looks_like_a_mount(exe: &Path) -> bool {
    let p = exe.to_string_lossy();
    p.contains("/.mount_") || p.starts_with("/tmp/")
}

// ===== finding the editor builds =====

/// The editor commands the deploy looks for (D56 (c)): VS Code, VSCodium and Code - OSS. It does not choose between them: each build
/// found installs and removes through its own command.
pub const EDITOR_COMMANDS: [&str; 3] = ["code", "codium", "code-oss"];

fn command_files(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.cmd"), format!("{name}.exe"), format!("{name}.bat")]
    } else {
        vec![name.to_string()]
    }
}

fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn in_a_cursor_folder(path: &Path) -> bool {
    path.components().any(|c| {
        let part = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        part.trim_start_matches('.') == "cursor"
    })
}

/// One editor build found on PATH.
#[derive(Debug, Clone)]
pub struct EditorBuild {
    /// The command a person types: `code`, `codium` or `code-oss`.
    pub name: &'static str,
    /// Where it was found on PATH; every call goes through this file.
    pub command: PathBuf,
    /// The program behind it, links followed.
    pub program: PathBuf,
    /// The folder its extensions land in, `<home>/<dataFolderName>/extensions` from the build's own `product.json`, or why that
    /// could not be read.
    pub extensions_dir: Result<PathBuf, String>,
}

#[derive(Debug, Clone, Default)]
pub struct EditorSearch {
    pub builds: Vec<EditorBuild>,
    /// Commands on PATH that live in a Cursor folder. Cursor is never driven.
    pub cursors: Vec<PathBuf>,
}

/// Every editor command of [`EDITOR_COMMANDS`] on PATH, the first of each name, as the shell would run it.
pub fn find_editors(path_var: Option<&OsStr>, home: Option<&Path>) -> EditorSearch {
    let mut search = EditorSearch::default();
    let Some(path_var) = path_var else { return search };
    for name in EDITOR_COMMANDS {
        let found = std::env::split_paths(path_var)
            .find_map(|dir| command_files(name).into_iter().map(|file| dir.join(file)).find(|c| is_runnable(c)));
        let Some(command) = found else { continue };
        let program = std::fs::canonicalize(&command).unwrap_or_else(|_| command.clone());
        if in_a_cursor_folder(&command) || in_a_cursor_folder(&program) {
            search.cursors.push(command);
            continue;
        }
        let extensions_dir = data_folder_name(&program).and_then(|folder| match home {
            Some(home) => Ok(home.join(folder).join("extensions")),
            None => Err("this machine has no home folder".to_string()),
        });
        search.builds.push(EditorBuild { name, command, program, extensions_dir });
    }
    search
}

/// The `dataFolderName` of the build whose program is `program`, read from its own `product.json`: next to the program's folder
/// or one above it, directly or under `resources/app` (VS Code, VSCodium and Code - OSS on Linux and Windows keep it at
/// `<install>/resources/app/product.json` beside `<install>/bin`, macOS at `.../Resources/app/product.json` above `bin`). On Linux
/// VS Code names `.vscode`, VSCodium and Code - OSS `.vscode-oss`. When it cannot be read, the reason, never a guess.
pub fn data_folder_name(program: &Path) -> Result<String, String> {
    let bin = program.parent().ok_or_else(|| format!("{} has no folder", program.display()))?;
    let places: Vec<PathBuf> = [Some(bin), bin.parent()]
        .into_iter()
        .flatten()
        .flat_map(|dir| [dir.join("product.json"), dir.join("resources").join("app").join("product.json")])
        .collect();
    let Some((place, text)) = places.iter().find_map(|p| std::fs::read_to_string(p).ok().map(|t| (p, t))) else {
        return Err(format!("no product.json was found next to {}", program.display()));
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("{} could not be read: {e}", place.display()))?;
    let name = value
        .get("dataFolderName")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} names no dataFolderName", place.display()))?;
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(name.to_string()),
        _ => Err(format!("{} names the dataFolderName {name:?}, which is not a folder name", place.display())),
    }
}

#[derive(Debug, Clone)]
pub struct CodeOutput {
    pub ok: bool,
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `code <args>` and waits at most `timeout`; a call that does not finish is killed and reported.
pub fn run_code(code: &Path, args: &[&str], timeout: Duration) -> Result<CodeOutput, String> {
    let mut attempts = 0;
    let mut child = loop {
        let spawned = std::process::Command::new(code)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();
        match spawned {
            Ok(child) => break child,
            // "Text file busy": the launcher script was written a moment ago and another thread's fork still holds
            // it open for writing. It lets go within milliseconds.
            Err(e) if e.raw_os_error() == Some(26) && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("could not start {}: {e}", code.display())),
        }
    };
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut s) = stream {
                let mut bytes = Vec::new();
                let _ = s.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let name = code.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                return Err(format!("`{name} {}` did not finish within {} s and was stopped", args.join(" "), timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("could not wait for {}: {e}", code.display())),
        }
    };
    Ok(CodeOutput {
        ok: status.success(),
        exit: status.code(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

// ===== the files =====

/// The version in `<prefix>-<version>.vsix`, when the name is exactly that. `jawata-chat-view-1.0.0.vsix` is not
/// a chat: the rest after the prefix must be a version and nothing else.
pub fn vsix_version(file_name: &str, prefix: &str) -> Option<semver::Version> {
    let rest = file_name.strip_prefix(prefix)?.strip_prefix('-')?;
    let version = rest.strip_suffix(".vsix")?;
    semver::Version::parse(version).ok()
}

/// The newest of `names` that is a `.vsix` of `prefix`.
pub fn newest_vsix<'a>(names: impl IntoIterator<Item = &'a str>, prefix: &str) -> Option<&'a str> {
    names
        .into_iter()
        .filter_map(|name| vsix_version(name, prefix).map(|v| (v, name)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, name)| name)
}

#[derive(Debug, Default)]
pub struct Vsixes {
    pub chat: Option<PathBuf>,
    pub java: Option<PathBuf>,
    /// What could not be had, each a sentence for the result.
    pub problems: Vec<String>,
    /// Where the files came from, for the result.
    pub source: String,
}

fn from_folder(dir: &Path) -> Vsixes {
    let mut found = Vsixes { source: format!("the folder {} ({VSIX_DIR_ENV})", dir.display()), ..Default::default() };
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    if names.is_empty() && !dir.is_dir() {
        found.problems.push(format!("{VSIX_DIR_ENV} names {}, which is not a folder", dir.display()));
        return found;
    }
    found.chat = newest_vsix(names.iter().map(String::as_str), "jawata-chat").map(|n| dir.join(n));
    found.java = newest_vsix(names.iter().map(String::as_str), "jawata-java").map(|n| dir.join(n));
    if found.chat.is_none() {
        found.problems.push(format!("{} holds no jawata-chat-<version>.vsix", dir.display()));
    }
    if found.java.is_none() {
        found.problems.push(format!("{} holds no jawata-java-<version>.vsix", dir.display()));
    }
    found
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn http_client(total: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("jawata-studio/vscode-deploy")
        .connect_timeout(Duration::from_secs(10))
        .timeout(total)
        .build()
        .map_err(|e| format!("could not create an HTTP client: {e}"))
}

/// Brings one asset into `cache` under its own name, checking the size GitHub states. A file already there with
/// that size is kept (a second deploy downloads nothing); a half file from an interrupted run is continued by
/// [`crate::release_manager::fetch_resumable`]; a file of the wrong size is removed and reported, never installed.
fn download_asset(client: &reqwest::blocking::Client, asset: &Asset, cache: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(cache).map_err(|e| format!("could not create {}: {e}", cache.display()))?;
    let target = cache.join(&asset.name);
    if asset.size > 0 && std::fs::metadata(&target).map(|m| m.len()).ok() == Some(asset.size) {
        return Ok(target);
    }
    let partial = cache.join(format!(".partial-{}", asset.name));
    let got = crate::release_manager::fetch_resumable(client, &asset.browser_download_url, &partial)
        .map_err(|e| format!("download of {} failed: {e}", asset.name))?;
    if got == 0 || (asset.size > 0 && got != asset.size) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "download of {} came to {got} bytes, but the release says {} bytes; the file was removed and not installed",
            asset.name, asset.size
        ));
    }
    let _ = std::fs::remove_file(&target);
    std::fs::rename(&partial, &target).map_err(|e| format!("could not place {}: {e}", target.display()))?;
    Ok(target)
}

fn from_release(api: &str, cache: &Path) -> Vsixes {
    let mut found = Vsixes { source: format!("the latest release of {EXTENSIONS_REPO}"), ..Default::default() };
    let release: Result<Release, String> = http_client(Duration::from_secs(20)).and_then(|client| {
        let response = client
            .get(api)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|e| format!("could not reach the releases of {EXTENSIONS_REPO}: {}", crate::release_manager::describe_error_chain(&e)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(format!("{EXTENSIONS_REPO} has no published release yet, so there are no extensions to install"));
        }
        response
            .error_for_status()
            .map_err(|e| format!("the releases of {EXTENSIONS_REPO} answered an error: {e}"))?
            .json::<Release>()
            .map_err(|e| format!("the latest release of {EXTENSIONS_REPO} could not be read: {e}"))
    });
    let release = match release {
        Ok(r) => r,
        Err(e) => {
            found.problems.push(e);
            return found;
        }
    };
    found.source = format!("the release {} of {EXTENSIONS_REPO}", release.tag_name);
    let download = match http_client(Duration::from_secs(600)) {
        Ok(c) => c,
        Err(e) => {
            found.problems.push(e);
            return found;
        }
    };
    for (prefix, slot) in [("jawata-chat", &mut found.chat), ("jawata-java", &mut found.java)] {
        let wanted = newest_vsix(release.assets.iter().map(|a| a.name.as_str()), prefix);
        match wanted.and_then(|name| release.assets.iter().find(|a| a.name == name)) {
            None => found.problems.push(format!(
                "the release {} of {EXTENSIONS_REPO} has no {prefix}-<version>.vsix asset",
                release.tag_name
            )),
            Some(asset) => match download_asset(&download, asset, cache) {
                Ok(path) => *slot = Some(path),
                Err(e) => found.problems.push(e),
            },
        }
    }
    found
}

/// The two files: from the local folder when `JAWATA_STUDIO_VSIX_DIR` names one (never mixed with the network),
/// otherwise from the latest release.
pub fn obtain_vsixes(env: &EditorEnv, cache: &Path) -> Vsixes {
    if let Some(dir) = &env.vsix_dir {
        return from_folder(dir);
    }
    match &env.release_api {
        Some(api) => from_release(api, cache),
        None => Vsixes {
            source: "no source".into(),
            problems: vec!["no source for the extensions is configured".into()],
            ..Default::default()
        },
    }
}

// ===== the outcome =====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Skipped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ExtensionsReport {
    pub outcome: Outcome,
    /// One line in plain words: per editor command, what was installed or removed and in which folder.
    pub message: String,
    pub changed: Vec<String>,
    pub errors: Vec<String>,
    /// The first editor command used.
    pub code: Option<PathBuf>,
}

impl ExtensionsReport {
    fn skipped(message: String, code: Option<PathBuf>) -> Self {
        ExtensionsReport { outcome: Outcome::Skipped, message, changed: vec![], errors: vec![], code }
    }
}

const NO_EDITOR: &str = "Editor extensions: none installed or removed, because there is no `code` command on PATH, and no `codium` or \
`code-oss` either. Install the command (in VS Code's Command Palette: Shell Command: Install 'code' command in PATH) and deploy again.";

/// The editor builds to drive, or the report that there are none.
fn locate(env: &EditorEnv) -> Result<EditorSearch, ExtensionsReport> {
    let search = find_editors(env.path_var.as_deref(), env.home.as_deref());
    if !search.builds.is_empty() {
        return Ok(search);
    }
    match search.cursors.first() {
        None => Err(ExtensionsReport::skipped(NO_EDITOR.to_string(), None)),
        Some(cursor) => Err(ExtensionsReport::skipped(
            format!(
                "Editor extensions: none installed or removed. The `code` on PATH is {}, which is Cursor's; Cursor is never driven.",
                cursor.display()
            ),
            None,
        )),
    }
}

/// The builds grouped by the folder their extensions land in: two builds sharing one folder are one installation, done through the
/// first one's command. A build whose folder is unknown is its own group (unless it is the very same program as another).
fn by_folder(builds: &[EditorBuild]) -> Vec<Vec<&EditorBuild>> {
    let mut groups: Vec<(PathBuf, Vec<&EditorBuild>)> = Vec::new();
    for build in builds {
        let key = build.extensions_dir.clone().unwrap_or_else(|_| build.program.clone());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(build),
            None => groups.push((key, vec![build])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

/// An installed extension as `--list-extensions --show-versions` names it: `publisher.name@version`, the id in lower case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub version: Option<String>,
}

fn listed(build: &EditorBuild, env: &EditorEnv) -> Result<Vec<Installed>, String> {
    let out = run_code(&build.command, &["--list-extensions", "--show-versions"], env.step_timeout)?;
    if !out.ok {
        return Err(format!("`{}` could not list its extensions (it said: {})", build.name, said(&out)));
    }
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| match line.split_once('@') {
            Some((id, version)) => Installed { id: id.to_ascii_lowercase(), version: Some(version.trim().to_string()) },
            None => Installed { id: line.to_ascii_lowercase(), version: None },
        })
        .collect())
}

fn said(out: &CodeOutput) -> String {
    let text = out.stderr.trim();
    if text.is_empty() {
        format!("nothing, and stopped with exit code {}", out.exit.map_or("none".to_string(), |c| c.to_string()))
    } else {
        text.to_string()
    }
}

/// What the deploy reads from an installed extension's `package.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Its `displayName`, a `%key%` looked up in its `package.nls.json`.
    pub name: Option<String>,
    /// The ids it cannot work without, lower case: its `extensionDependencies` and its `extensionPack`.
    pub needs: Vec<String>,
}

/// The folder of one installed extension: `<id>-<version>`, with or without a platform after it (`redhat.java-1.40.0-linux-x64`).
fn extension_folder(extensions_dir: &Path, extension: &Installed) -> Option<PathBuf> {
    let mut names: Vec<String> =
        std::fs::read_dir(extensions_dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let fits = |name: &str| {
        let name = name.to_ascii_lowercase();
        match &extension.version {
            Some(version) => {
                let stem = format!("{}-{}", extension.id, version.to_ascii_lowercase());
                name == stem || name.starts_with(&format!("{stem}-"))
            }
            None => name
                .strip_prefix(&format!("{}-", extension.id))
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit())),
        }
    };
    names.iter().find(|n| fits(n)).map(|n| extensions_dir.join(n))
}

/// The name and the needs of the extension in `folder`, from its `package.json`.
pub fn read_manifest(folder: &Path) -> Option<Manifest> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("package.json")).ok()?).ok()?;
    let ids = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect())
            .unwrap_or_default()
    };
    let mut needs = ids("extensionDependencies");
    needs.extend(ids("extensionPack"));
    let name = value.get("displayName").and_then(Value::as_str).and_then(|name| match name.strip_prefix('%').and_then(|n| n.strip_suffix('%')) {
        None => Some(name.to_string()),
        Some(key) => {
            let nls: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("package.nls.json")).ok()?).ok()?;
            let entry = nls.get(key)?;
            entry.as_str().or_else(|| entry.get("message").and_then(Value::as_str)).map(str::to_string)
        }
    });
    Some(Manifest { name: name.filter(|n| !n.trim().is_empty()), needs })
}

/// The manifests of the installed extensions whose folders are found in `extensions_dir`, by id.
pub fn manifests_of(extensions_dir: &Path, installed: &[Installed]) -> std::collections::BTreeMap<String, Manifest> {
    installed
        .iter()
        .filter_map(|e| extension_folder(extensions_dir, e).and_then(|f| read_manifest(&f)).map(|m| (e.id.clone(), m)))
        .collect()
}

/// `target` and every installed extension that needs it, followed through, in the order they can be uninstalled: each before
/// anything it needs, so `target` comes last. Among those that may go next, the smaller id goes first.
///
/// DELIBERATE DUPLICATE: jawata Java (jawata-chat `packages/vscode-java/src/extension.ts`, `installedOf`) answers the same question inside the
/// editor for an install made after a deploy. The two run in different programs and repositories; both read `extensionDependencies` and
/// `extensionPack`, and each is held to "what needs it goes first" by its own tests (28h C25, architect finding 1).
pub fn removal_order(target: &str, needs: &std::collections::BTreeMap<String, Manifest>) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut set: BTreeSet<String> = BTreeSet::from([target.to_string()]);
    loop {
        let more: Vec<String> = needs
            .iter()
            .filter(|(id, m)| !set.contains(*id) && m.needs.iter().any(|n| set.contains(n)))
            .map(|(id, _)| id.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        set.extend(more);
    }
    let mut order = Vec::new();
    while !set.is_empty() {
        let needed_by_another =
            |candidate: &String| set.iter().any(|other| other != candidate && needs.get(other).is_some_and(|m| m.needs.contains(candidate)));
        let next = set.iter().find(|c| !needed_by_another(c)).or_else(|| set.iter().next()).cloned().expect("not empty");
        set.remove(&next);
        order.push(next);
    }
    order
}

/// The names a person knows these by, for when a manifest cannot be read.
fn known_name(id: &str) -> Option<&'static str> {
    match id {
        CHAT_EXTENSION_ID => Some("jawata chat"),
        JAVA_EXTENSION_ID => Some("jawata Java"),
        RIVAL_EXTENSION_ID => Some("Language Support for Java by Red Hat"),
        PROJECT_MANAGER_EXTENSION_ID => Some("Project Manager for Java"),
        _ => None,
    }
}

fn name_of(id: &str, manifests: &std::collections::BTreeMap<String, Manifest>) -> String {
    manifests
        .get(id)
        .and_then(|m| m.name.clone())
        .or_else(|| known_name(id).map(str::to_string))
        .unwrap_or_else(|| id.to_string())
}

/// "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A path under the home folder written from `~`, as a person reads it.
fn shown(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("{}/{}", "~", rest.display()),
        None => path.display().to_string(),
    }
}

/// "With `code`, into ~/.vscode/extensions", "With `codium` and `code-oss`, which both use ~/.vscode-oss/extensions", or, for a
/// build whose folder could not be read, the reason.
fn which_build(group: &[&EditorBuild], into: &str, home: Option<&Path>) -> String {
    let commands = and_list(&group.iter().map(|b| format!("`{}`", b.name)).collect::<Vec<_>>());
    match &group[0].extensions_dir {
        Ok(dir) if group.len() > 1 => format!("With {commands}, which both use {}", shown(dir, home)),
        Ok(dir) => format!("With {commands}, {into} {}", shown(dir, home)),
        Err(reason) if group.len() > 1 => format!("With {commands}, which are one program (its extensions folder is not known: {reason})"),
        Err(reason) => format!("With {commands} (its extensions folder is not known: {reason})"),
    }
}

/// Uninstalls `id` through `build`. A failure the editor reports is checked against a fresh list: an extension that went together
/// with an extension pack it belonged to is gone, which is what was wanted.
fn uninstall(build: &EditorBuild, id: &str, env: &EditorEnv) -> Result<(), String> {
    let failure = match run_code(&build.command, &["--uninstall-extension", id], env.step_timeout) {
        Ok(out) if out.ok => return Ok(()),
        Ok(out) => said(&out),
        Err(e) => e,
    };
    match listed(build, env) {
        Ok(now) if now.iter().all(|e| e.id != id) => Ok(()),
        _ => Err(failure),
    }
}

/// What one installation did, in the words of the result line.
struct GroupResult {
    done: Vec<String>,
    notes: Vec<String>,
}

fn deploy_group(group: &[&EditorBuild], files: &Vsixes, env: &EditorEnv, changed: &mut Vec<String>, errors: &mut Vec<String>) -> GroupResult {
    let build = group[0];
    let mut result = GroupResult { done: vec![], notes: vec![] };
    let before = match listed(build, env) {
        Ok(list) => list,
        Err(e) => {
            errors.push(e);
            result.done.push("nothing was installed".into());
            return result;
        }
    };
    let manifests = match &build.extensions_dir {
        Ok(dir) => manifests_of(dir, &before),
        Err(_) => Default::default(),
    };
    let mut installed = Vec::new();
    let mut java_in = false;
    for (id, file) in [(CHAT_EXTENSION_ID, &files.chat), (JAVA_EXTENSION_ID, &files.java)] {
        let Some(file) = file else { continue };
        let path = file.to_string_lossy().into_owned();
        let name = name_of(id, &Default::default());
        match run_code(&build.command, &["--install-extension", &path, "--force"], env.step_timeout) {
            Ok(out) if out.ok => {
                changed.push(format!("`{}`: installed {name}", build.name));
                installed.push(name);
                java_in |= id == JAVA_EXTENSION_ID;
            }
            Ok(out) => errors.push(format!("`{}` could not install {name} (it said: {})", build.name, said(&out))),
            Err(e) => errors.push(e),
        }
    }
    if !installed.is_empty() {
        result.done.push(format!("installed {}", and_list(&installed)));
    }
    // Only after jawata Java is in, so that a Java Projects view, a Java language server and a debugger are always there.
    if !java_in {
        return result;
    }
    let mut gone: Vec<String> = Vec::new();
    if before.iter().any(|e| e.id == PROJECT_MANAGER_EXTENSION_ID) {
        let name = name_of(PROJECT_MANAGER_EXTENSION_ID, &manifests);
        match uninstall(build, PROJECT_MANAGER_EXTENSION_ID, env) {
            Ok(()) => {
                changed.push(format!("`{}`: removed {name}", build.name));
                result
                    .done
                    .push(format!("removed {name}, because jawata Java brings its own Java Projects view and the two listed every project twice"));
                gone.push(PROJECT_MANAGER_EXTENSION_ID.to_string());
            }
            Err(e) => errors.push(format!(
                "`{}` could not remove {name} (it said: {e}); the Java Projects view shows twice until it is removed",
                build.name
            )),
        }
    }
    if before.iter().any(|e| e.id == RIVAL_EXTENSION_ID) {
        if let Err(reason) = &build.extensions_dir {
            result.notes.push(format!("which other extensions need Red Hat's could not be read, because its extensions folder is not known: {reason}"));
        }
        let rival = name_of(RIVAL_EXTENSION_ID, &manifests);
        let mut dependents = Vec::new();
        let mut rival_gone = false;
        for id in removal_order(RIVAL_EXTENSION_ID, &manifests) {
            if gone.contains(&id) {
                continue;
            }
            let name = name_of(&id, &manifests);
            match uninstall(build, &id, env) {
                Ok(()) => {
                    changed.push(format!("`{}`: removed {name}", build.name));
                    if id == RIVAL_EXTENSION_ID {
                        rival_gone = true;
                    } else {
                        dependents.push(name);
                    }
                    gone.push(id);
                }
                Err(e) => errors.push(format!("`{}` could not remove {name} (it said: {e})", build.name)),
            }
        }
        let why = "because jawata Java now brings the Java language support, the Testing view and the debugger";
        match (rival_gone, dependents.is_empty()) {
            (true, true) => result.done.push(format!("removed {rival}, {why}")),
            (true, false) => result.done.push(format!("removed {rival}, {why}, and with it {}, which need it", and_list(&dependents))),
            (false, false) => result.done.push(format!("removed {}, which need {rival}", and_list(&dependents))),
            (false, true) => {}
        }
    }
    // Uninstalling an extension pack takes the extensions it brought with it, which need not need Red Hat's (on a real VS Code on
    // 2026-11-11 Extension Pack for Java took Debugger, Maven and Gradle for Java with it). Everything that left is named, never only what
    // was asked for (28h C25).
    if !gone.is_empty() {
        if let Ok(now) = listed(build, env) {
            let mut with_a_pack: Vec<String> = before
                .iter()
                .filter(|e| !now.iter().any(|n| n.id.eq_ignore_ascii_case(&e.id)))
                .filter(|e| !gone.iter().any(|g| g.eq_ignore_ascii_case(&e.id)))
                .map(|e| name_of(&e.id, &manifests))
                .collect();
            with_a_pack.sort();
            if !with_a_pack.is_empty() {
                changed.push(format!("`{}`: removed {}", build.name, and_list(&with_a_pack)));
                result.done.push(format!("the editor removed {} with the extension pack they came in", and_list(&with_a_pack)));
            }
        }
    }
    result
}

fn result_line(heading: &str, sentences: Vec<String>, tail: Vec<String>, errors: &[String]) -> String {
    let mut message = format!("{heading} {}", sentences.join(" "));
    for sentence in tail {
        message.push(' ');
        message.push_str(&sentence);
    }
    if !errors.is_empty() {
        message.push_str(&format!(" Not done: {}.", errors.join("; ")));
    }
    message
}

fn cursor_notes(search: &EditorSearch) -> Vec<String> {
    search
        .cursors
        .iter()
        .map(|c| {
            let name = c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            format!("The `{name}` at {} is Cursor's and was left alone.", c.display())
        })
        .collect()
}

/// Install both extensions through every editor build on PATH, then remove the extensions they replace, in that order and only that
/// far: Microsoft's Project Manager for Java and Red Hat's Java extension, with every extension that needs Red Hat's, go only after
/// jawata Java is in. Two builds sharing one extensions folder are installed once.
pub fn deploy_extensions(env: &EditorEnv, cache: &Path) -> ExtensionsReport {
    let search = match locate(env) {
        Ok(search) => search,
        Err(report) => return report,
    };
    let first = Some(search.builds[0].command.clone());
    let files = obtain_vsixes(env, cache);
    let mut changed = Vec::new();
    let mut errors = files.problems.clone();
    if files.chat.is_none() && files.java.is_none() {
        return ExtensionsReport {
            outcome: Outcome::Failed,
            message: format!("Editor extensions: none installed, because {}.", errors.join("; ")),
            changed,
            errors,
            code: first,
        };
    }
    let home = env.home.as_deref();
    let mut sentences = Vec::new();
    for group in by_folder(&search.builds) {
        let result = deploy_group(&group, &files, env, &mut changed, &mut errors);
        let did = if result.done.is_empty() { "nothing changed".to_string() } else { result.done.join("; ") };
        let mut sentence = format!("{}: {did}", which_build(&group, "into", home));
        if !result.notes.is_empty() {
            sentence.push_str(&format!(" ({})", result.notes.join("; ")));
        }
        sentences.push(format!("{sentence}."));
    }
    let mut tail = vec![format!("The files came from {}.", files.source)];
    tail.extend(cursor_notes(&search));
    let outcome = if errors.is_empty() { Outcome::Success } else { Outcome::Failed };
    let message = result_line("Editor extensions, installed through each editor's own command.", sentences, tail, &errors);
    ExtensionsReport { outcome, message, changed, errors, code: first }
}

/// Take out the two extensions the deploy installed, through every editor build on PATH.
pub fn remove_extensions(env: &EditorEnv) -> ExtensionsReport {
    let search = match locate(env) {
        Ok(search) => search,
        Err(report) => return report,
    };
    let first = Some(search.builds[0].command.clone());
    let home = env.home.as_deref();
    let mut changed = Vec::new();
    let mut errors = Vec::new();
    let mut sentences = Vec::new();
    for group in by_folder(&search.builds) {
        let build = group[0];
        let present = match listed(build, env) {
            Ok(list) => list,
            Err(e) => {
                errors.push(e);
                sentences.push(format!("{}: nothing was removed.", which_build(&group, "from", home)));
                continue;
            }
        };
        let mut removed = Vec::new();
        for id in [CHAT_EXTENSION_ID, JAVA_EXTENSION_ID] {
            if !present.iter().any(|p| p.id == id) {
                continue;
            }
            let name = name_of(id, &Default::default());
            match uninstall(build, id, env) {
                Ok(()) => {
                    changed.push(format!("`{}`: removed {name}", build.name));
                    removed.push(name);
                }
                Err(e) => errors.push(format!("`{}` could not remove {name} (it said: {e})", build.name)),
            }
        }
        let did = if removed.is_empty() {
            "neither jawata extension was installed, nothing to remove".to_string()
        } else {
            format!("removed {}", and_list(&removed))
        };
        sentences.push(format!("{}: {did}.", which_build(&group, "from", home)));
    }
    let outcome = if errors.is_empty() { Outcome::Success } else { Outcome::Failed };
    let message = result_line("Editor extensions, removed through each editor's own command.", sentences, cursor_notes(&search), &errors);
    ExtensionsReport { outcome, message, changed, errors, code: first }
}

// ===== the settings =====

/// The `jawata*.studio` value: the program, `--background`, and the folder the studio leaves its answer in for the
/// editor's workspace (Stage 21 (j)). `handoff_dir` is absolute: the editor reads it from its own working folder.
fn studio_value(studio: &Path, handoff_dir: &Path) -> Value {
    json!({
        "command": studio.to_string_lossy(),
        "args": [crate::BACKGROUND_ARG],
        "handoffDir": handoff_dir.to_string_lossy(),
    })
}

/// A `jawata*.studio` entry that this module wrote: a program with no arguments whose file name says jawata, with or
/// without the `handoffDir` a later deploy adds (an entry from before that is still ours and is brought up to date).
fn is_studio_entry(entry: &Value) -> bool {
    let Some(object) = entry.as_object() else { return false };
    // `["--background"]` is what the deploy writes: the studio the extension starts shows no window.
    let no_args = object
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|a| a.is_empty() || (a.len() == 1 && a[0].as_str() == Some(crate::BACKGROUND_ARG)));
    let named = object
        .get("command")
        .and_then(Value::as_str)
        .map(|c| {
            Path::new(c)
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase().contains("jawata"))
                .unwrap_or(false)
                || c.to_ascii_lowercase().contains("jawata-studio")
        })
        .unwrap_or(false);
    let handoff_ok = object.get("handoffDir").is_none_or(Value::is_string);
    no_args && named && handoff_ok
}

fn is_java_server_entry(entry: &Value) -> bool {
    is_ours_launched_with(entry, "--lsp")
}

fn refusal(key: &str) -> String {
    format!("`{key}` is already set to something that is not jawata's own; it is left as it is")
}

/// The three editor keys put into `existing` (a VS Code `settings.json`, comments and trailing commas allowed):
/// `jawataJava.server` from the resident's launch, and both `studio` keys when the studio's path is known.
/// An entry under one of the keys that is not ours stops the merge: it is the person's own.
pub fn editor_keys_merged(existing: Option<&str>, java: &Launch, studio: Option<&Path>, handoff_dir: &Path) -> Result<String, String> {
    let mut wanted: Vec<(&str, Value, fn(&Value) -> bool)> =
        vec![(JAVA_SERVER_KEY, java.as_lsp().value(false), is_java_server_entry)];
    if let Some(studio) = studio {
        wanted.push((CHAT_STUDIO_KEY, studio_value(studio, handoff_dir), is_studio_entry));
        wanted.push((JAVA_STUDIO_KEY, studio_value(studio, handoff_dir), is_studio_entry));
    }
    // D53 (p): Copilot off. If the person had already turned it off, Remove must leave that as it was, so no mark is written.
    let already_off = existing
        .filter(|t| !t.trim().is_empty())
        .and_then(|t| read_root(t).ok())
        .and_then(|root| root.member(AI_OFF_KEY).map(|m| m.value.to_value() == json!(true)))
        .unwrap_or(false);
    let marked = existing
        .filter(|t| !t.trim().is_empty())
        .and_then(|t| read_root(t).ok())
        .is_some_and(|root| root.member(AI_OFF_BY_DEPLOY_KEY).is_some());
    wanted.push((AI_OFF_KEY, json!(true), Value::is_boolean));
    if !already_off || marked {
        wanted.push((AI_OFF_BY_DEPLOY_KEY, json!(true), Value::is_boolean));
    }
    let Some(text) = existing.filter(|t| !t.trim().is_empty()) else {
        let mut root = parse_object(existing)?;
        for (key, value, _) in wanted {
            root.insert(key.into(), value);
        }
        return Ok(render(root));
    };
    let mut text = text.to_string();
    for (key, value, ours) in wanted {
        let root = read_root(&text)?;
        if let Some(current) = root.member(key) {
            if !ours(&current.value.to_value()) {
                return Err(refusal(key));
            }
        }
        let style = jsonc_edit::style_of(&text, &root);
        text = jsonc_edit::apply(&text, jsonc_edit::set_member(&text, &root, key, &value, &style))?;
    }
    Ok(text)
}

/// The editor keys of ours taken out of `existing`; a key that is not ours stays, and so does everything else.
pub fn editor_keys_without(existing: &str) -> Result<String, String> {
    if existing.trim().is_empty() {
        return Ok(render(parse_object(Some(existing))?));
    }
    let root = read_root(existing)?;
    let mut ours: Vec<usize> = EDITOR_KEYS
        .iter()
        .filter_map(|key| {
            let index = root.member_index(key)?;
            let value = root.members[index].value.to_value();
            let is_ours = if *key == JAVA_SERVER_KEY { is_java_server_entry(&value) } else { is_studio_entry(&value) };
            is_ours.then_some(index)
        })
        .collect();
    // Copilot comes back only when it was this deploy that turned it off.
    if let Some(mark) = root.member_index(AI_OFF_BY_DEPLOY_KEY) {
        ours.push(mark);
        ours.extend(root.member_index(AI_OFF_KEY));
    }
    jsonc_edit::apply(existing, jsonc_edit::remove_members(existing, &root, &ours))
}

/// The settings file this module writes into: the chat row's own file, under the id of this row.
pub fn settings_target(home: &Path, config_dir: &Path, windows: bool) -> RowTarget {
    let chat = rows(home, config_dir, windows)
        .into_iter()
        .find(|row| row.id == CHAT_ROW)
        .expect("the chat row is one of the rows");
    RowTarget { id: SETTINGS_ROW, path: chat.path, client_folder: chat.client_folder }
}

/// Applies the editor keys to the settings file. The deploy that names VS Code names this row too, so the file
/// (and its folder) is created for a VS Code that has not been opened yet. `entries` is the same list the other
/// registration rows write; the first resident is the one the editor reaches, as for the chat.
pub fn apply_settings_row(
    target: &RowTarget,
    entries: &Result<Vec<(String, Launch)>, String>,
    studio: Option<&Path>,
    handoff_dir: &Path,
    action: RowAction,
    backup: &dyn Fn(&Path) -> Result<Option<PathBuf>, String>,
) -> RowOutcome {
    let existing = std::fs::read_to_string(&target.path).ok();
    let outcome_of = |state: RowState, message: String| RowOutcome {
        client: target.id.to_string(),
        target_path: target.path.to_string_lossy().into_owned(),
        state,
        message,
        backup_path: None,
        changed_sections: Vec::new(),
        validation_errors: Vec::new(),
        preview_content: None,
    };
    if action == RowAction::Delete {
        let Some(text) = existing else {
            return outcome_of(RowState::Skipped, "Skipped: nothing to remove, the file does not exist.".into());
        };
        let removed = editor_keys_without(&text);
        return finish(target, action, Some(text), removed, backup, "removed jawata's editor settings");
    }
    let launch = match entries {
        Ok(entries) if !entries.is_empty() => &entries[0].1,
        Ok(_) => return outcome_of(RowState::Skipped, "Skipped: no workspace has an engine to start.".into()),
        Err(reason) => return outcome_of(RowState::Skipped, format!("Skipped: no engine to start ({reason}).")),
    };
    let merged = editor_keys_merged(existing.as_deref(), launch, studio, handoff_dir);
    let mut outcome = finish(target, action, existing, merged, backup, "wrote jawata Java's server and both studio start settings");
    if outcome.state == RowState::Failed {
        let mut snippet = serde_json::Map::new();
        snippet.insert(JAVA_SERVER_KEY.into(), launch.as_lsp().value(false));
        if let Some(studio) = studio {
            snippet.insert(CHAT_STUDIO_KEY.into(), studio_value(studio, handoff_dir));
            snippet.insert(JAVA_STUDIO_KEY.into(), studio_value(studio, handoff_dir));
        }
        outcome.message = format!("{} Paste this by hand: {}", outcome.message, Value::Object(snippet));
    } else if studio.is_none() && outcome.state == RowState::Success {
        outcome.message = format!("{} (this studio's own path is not known, so the studio start settings were not written)", outcome.message);
    }
    outcome
}


#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) struct Scratch(pub(crate) PathBuf);
    impl Scratch {
        pub(crate) fn new(label: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "jawata-studio-vscode-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        pub(crate) fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A `code` with no `product.json` beside it, so the folder its extensions land in is not known.
    pub(crate) fn fake_code(dir: &Path, installed: &[&str]) -> PathBuf {
        fake_editor(dir, "code", installed, None)
    }

    /// An editor command named `name` in `<dir>/bin` that records every call to `<dir>/calls.txt` (one line per call, arguments
    /// joined by a space), answers `--list-extensions` with the lines of `<dir>/installed.txt`, takes an uninstalled extension out
    /// of that file, and exits non-zero for a second argument containing the text of `<dir>/fail.txt`. With `data_folder`, the
    /// build's own `<dir>/resources/app/product.json` names it, laid out as VS Code's is.
    pub(crate) fn fake_editor(dir: &Path, name: &str, installed: &[&str], data_folder: Option<&str>) -> PathBuf {
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(dir.join("installed.txt"), installed.join("\n")).unwrap();
        if let Some(folder) = data_folder {
            let app = dir.join("resources").join("app");
            std::fs::create_dir_all(&app).unwrap();
            std::fs::write(app.join("product.json"), json!({"nameShort": name, "dataFolderName": folder}).to_string()).unwrap();
        }
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{calls}'\nif [ \"$1\" = \"--list-extensions\" ]; then cat '{installed}'; echo; exit 0; fi\n\
             if [ -f '{fail}' ]; then pat=$(cat '{fail}'); case \"$2\" in *\"$pat\"*) echo 'boom' >&2; exit 3;; esac; fi\n\
             if [ \"$1\" = \"--uninstall-extension\" ]; then grep -v -i -e \"^$2\\$\" -e \"^$2@\" '{installed}' > '{installed}.next'; mv '{installed}.next' '{installed}'; fi\n\
             exit 0\n",
            calls = dir.join("calls.txt").display(),
            installed = dir.join("installed.txt").display(),
            fail = dir.join("fail.txt").display(),
        );
        let path = bin.join(name);
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// One installed extension's folder under `extensions`, with the manifest the deploy reads and, when given, the
    /// `package.nls.json` a `%displayName%` is looked up in.
    pub(crate) fn installed_extension(extensions: &Path, folder: &str, manifest: Value, nls: Option<Value>) {
        let dir = extensions.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), manifest.to_string()).unwrap();
        if let Some(nls) = nls {
            std::fs::write(dir.join("package.nls.json"), nls.to_string()).unwrap();
        }
    }

    pub(crate) fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.txt"))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    pub(crate) fn env_over(dir: &Path, vsix_dir: Option<PathBuf>) -> EditorEnv {
        EditorEnv {
            path_var: Some(std::env::join_paths([dir.join("bin")]).unwrap()),
            vsix_dir,
            release_api: None,
            studio: Some(PathBuf::from("/home/h/.local/bin/jawata-studio")),
            home: Some(dir.join("home")),
            step_timeout: Duration::from_secs(20),
        }
    }

    pub(crate) fn vsix_folder(dir: &Path) -> PathBuf {
        let v = dir.join("vsix");
        std::fs::create_dir_all(&v).unwrap();
        for name in ["jawata-chat-0.1.0.vsix", "jawata-chat-0.2.0.vsix", "jawata-java-0.1.0.vsix", "jawata-chat-view-9.9.9.vsix", "other-1.0.0.vsix"] {
            std::fs::write(v.join(name), b"zip").unwrap();
        }
        v
    }

}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write as _;

    /// D56 (b) against a real VS Code, because a fake `code` that always succeeds is how the old removal shipped broken. Never Cursor:
    /// `JAWATA_REAL_CODE_DIR` is a folder holding a `code` wrapper bound to an isolated profile that has Red Hat and the extensions
    /// depending on it, `JAWATA_REAL_HOME` the home whose `<dataFolderName>/extensions` that profile installs into, and
    /// `JAWATA_REAL_VSIX_DIR` the two built extensions. Run with `--ignored`.
    #[test]
    #[ignore]
    fn on_a_real_vs_code_both_extensions_arrive_and_red_hat_goes_with_everything_that_needs_it() {
        let bin = PathBuf::from(std::env::var("JAWATA_REAL_CODE_DIR").expect("JAWATA_REAL_CODE_DIR"));
        let env = EditorEnv {
            path_var: Some(std::env::join_paths([bin]).unwrap()),
            vsix_dir: Some(PathBuf::from(std::env::var("JAWATA_REAL_VSIX_DIR").expect("JAWATA_REAL_VSIX_DIR"))),
            release_api: None,
            studio: None,
            home: Some(PathBuf::from(std::env::var("JAWATA_REAL_HOME").expect("JAWATA_REAL_HOME"))),
            step_timeout: Duration::from_secs(120),
        };
        let s = Scratch::new("realcode");
        let build = &find_editors(env.path_var.as_deref(), env.home.as_deref()).builds[0];
        let before = listed(build, &env).unwrap();
        let before_names = manifests_of(build.extensions_dir.as_ref().expect("the folder is known"), &before);
        let report = deploy_extensions(&env, &s.0);
        eprintln!("DEPLOY RESULT: {}", report.message);
        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        let after = listed(build, &env).unwrap();
        // 28h C25: every extension that left is named in the result, not only those the deploy asked to remove.
        for went in before.iter().filter(|e| !after.iter().any(|a| a.id == e.id)) {
            let name = name_of(&went.id, &before_names);
            assert!(report.message.contains(&name), "{name} left without being named: {}", report.message);
        }
        for id in [CHAT_EXTENSION_ID, JAVA_EXTENSION_ID] {
            assert!(after.iter().any(|e| e.id == id), "{id} is not installed: {after:?}");
        }
        assert!(after.iter().all(|e| e.id != RIVAL_EXTENSION_ID), "Red Hat's is gone: {after:?}");
        let manifests = manifests_of(build.extensions_dir.as_ref().expect("the folder is known"), &after);
        for (id, manifest) in &manifests {
            assert!(!manifest.needs.iter().any(|n| n == RIVAL_EXTENSION_ID), "{id} still needs Red Hat's and is still installed");
        }
    }

    #[test]
    fn the_files_are_picked_by_exact_name_and_the_newest_version() {
        assert_eq!(vsix_version("jawata-chat-0.2.0.vsix", "jawata-chat"), semver::Version::parse("0.2.0").ok());
        assert_eq!(vsix_version("jawata-chat-view-9.9.9.vsix", "jawata-chat"), None, "a different package is not the chat");
        assert_eq!(vsix_version("jawata-chat-0.2.0.zip", "jawata-chat"), None);
        let names = ["jawata-chat-0.1.0.vsix", "jawata-chat-0.10.0.vsix", "jawata-chat-0.9.0.vsix"];
        assert_eq!(newest_vsix(names, "jawata-chat"), Some("jawata-chat-0.10.0.vsix"), "0.10 is newer than 0.9");
    }

    /// D56 (b): jawata Java 0.2.2 has its own Testing view and debugger, so Red Hat's Java extension goes once jawata Java is in, and a
    /// second deploy never installs it back. Here the build's folder is unknown, so the result says which part could not be read.
    #[test]
    fn a_deploy_installs_both_extensions_removes_red_hat_and_never_installs_it_back() {
        let s = Scratch::new("deploy");
        fake_code(&s.0, &["ms-python.python", "redhat.java", "REDHAT.vscode-yaml"]);
        let vsix = vsix_folder(&s.0);
        let env = env_over(&s.0, Some(vsix.clone()));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert_eq!(
            calls(&s.0),
            vec![
                "--list-extensions --show-versions".to_string(),
                format!("--install-extension {} --force", vsix.join("jawata-chat-0.2.0.vsix").display()),
                format!("--install-extension {} --force", vsix.join("jawata-java-0.1.0.vsix").display()),
                "--uninstall-extension redhat.java".to_string(),
                // 28h C25: listed again, so that what the editor removed along with an extension pack is named too.
                "--list-extensions --show-versions".to_string(),
            ]
        );
        assert!(report.message.contains("removed Language Support for Java by Red Hat"), "named, not by its id: {}", report.message);
        assert!(report.message.contains("no product.json was found next to"), "the folder that could not be read is said: {}", report.message);
        assert!(!report.message.contains("left in place") && !report.message.contains("redhat.java"), "{}", report.message);

        std::fs::write(s.path("calls.txt"), "").unwrap();
        let again = deploy_extensions(&env, &s.path("cache"));
        assert_eq!(again.outcome, Outcome::Success, "{}", again.message);
        let all = calls(&s.0);
        assert!(all.iter().all(|c| !c.contains("--uninstall-extension")), "nothing left to remove: {all:?}");
        assert!(all.iter().all(|c| !c.to_ascii_lowercase().contains("redhat")), "Red Hat's is never installed back: {all:?}");
    }

    /// D56 (b): every installed extension that needs Red Hat's, through `extensionDependencies` or `extensionPack` and followed through,
    /// goes with it, each before what it needs, each named by the name its manifest gives. The manifests are read from the folder the
    /// build's own product.json names, at the version `--show-versions` lists.
    #[test]
    fn a_deploy_removes_red_hat_and_every_extension_that_needs_it_dependents_first_each_named() {
        let s = Scratch::new("redhat");
        fake_editor(
            &s.0,
            "code",
            &[
                "redhat.java@1.40.0",
                "vscjava.vscode-java-debug@0.58.0",
                "vscjava.vscode-java-test@0.43.0",
                "vscjava.vscode-java-pack@0.29.0",
                "vscjava.vscode-gradle@3.16.0",
                "ms-python.python@2024.1.0",
            ],
            Some(".vscode"),
        );
        let ext = s.path("home/.vscode/extensions");
        installed_extension(&ext, "redhat.java-1.40.0-linux-x64", json!({"displayName": "%displayName%"}), Some(json!({"displayName": "Language Support for Java(TM) by Red Hat"})));
        installed_extension(&ext, "redhat.java-1.39.0-linux-x64", json!({"displayName": "an old copy"}), None);
        installed_extension(&ext, "vscjava.vscode-java-debug-0.58.0", json!({"displayName": "Debugger for Java", "extensionDependencies": ["redhat.java"]}), None);
        // needs Red Hat's only through the debugger: followed through
        installed_extension(&ext, "vscjava.vscode-java-test-0.43.0", json!({"displayName": "Test Runner for Java", "extensionDependencies": ["VSCJAVA.vscode-java-debug"]}), None);
        installed_extension(&ext, "vscjava.vscode-java-pack-0.29.0", json!({"displayName": "Extension Pack for Java", "extensionPack": ["redhat.java", "vscjava.vscode-java-debug"]}), None);
        installed_extension(&ext, "vscjava.vscode-gradle-3.16.0", json!({"displayName": "Gradle for Java"}), None);
        installed_extension(&ext, "ms-python.python-2024.1.0", json!({"displayName": "Python", "extensionDependencies": ["ms-python.vscode-pylance"]}), None);
        let env = env_over(&s.0, Some(vsix_folder(&s.0)));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        let all = calls(&s.0);
        let removed: Vec<&str> = all.iter().filter_map(|c| c.strip_prefix("--uninstall-extension ")).collect();
        assert_eq!(
            removed,
            ["vscjava.vscode-java-pack", "vscjava.vscode-java-test", "vscjava.vscode-java-debug", "redhat.java"],
            "each before what it needs, Red Hat's last: {all:?}"
        );
        let java = all.iter().position(|c| c.contains("jawata-java-")).unwrap();
        assert!(all.iter().skip(java).filter(|c| c.contains("--uninstall-extension")).count() == 4, "all after jawata Java is in: {all:?}");
        for name in ["Language Support for Java(TM) by Red Hat", "Debugger for Java", "Test Runner for Java", "Extension Pack for Java"] {
            assert!(report.message.contains(name), "{name} is named: {}", report.message);
        }
        assert!(report.message.contains("With `code`, into ~/.vscode/extensions:"), "{}", report.message);
        for not in ["redhat.java", "vscjava.", "Gradle", "Python", "an old copy", "left in place", "%displayName%"] {
            assert!(!report.message.contains(not), "{not:?} in: {}", report.message);
        }

        std::fs::write(s.path("calls.txt"), "").unwrap();
        deploy_extensions(&env, &s.path("cache"));
        assert!(calls(&s.0).iter().all(|c| !c.contains("--uninstall-extension")), "a second deploy finds nothing to remove");
    }

    /// D56 (c): every editor build on PATH gets the extensions through its own command, and the result names each command and the folder
    /// the extensions landed in, read from the build's own product.json. VSCodium and Code - OSS share one folder: installed once, both named.
    #[test]
    fn every_editor_build_on_path_installs_through_its_own_command_and_the_result_names_command_and_folder() {
        let s = Scratch::new("builds");
        for (name, folder) in [("code", ".vscode"), ("codium", ".vscode-oss"), ("code-oss", ".vscode-oss")] {
            fake_editor(&s.path(name), name, &["redhat.java@1.40.0"], Some(folder));
        }
        let vsix = vsix_folder(&s.0);
        let env = EditorEnv {
            path_var: Some(std::env::join_paths([s.path("code/bin"), s.path("codium/bin"), s.path("code-oss/bin")]).unwrap()),
            ..env_over(&s.0, Some(vsix.clone()))
        };

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        for name in ["code", "codium"] {
            let made = calls(&s.path(name));
            assert!(made.contains(&format!("--install-extension {} --force", vsix.join("jawata-chat-0.2.0.vsix").display())), "{name}: {made:?}");
            assert!(made.contains(&format!("--install-extension {} --force", vsix.join("jawata-java-0.1.0.vsix").display())), "{name}: {made:?}");
            assert!(made.contains(&"--uninstall-extension redhat.java".to_string()), "{name}: {made:?}");
        }
        assert!(calls(&s.path("code-oss")).is_empty(), "one folder, one installation: {:?}", calls(&s.path("code-oss")));
        assert!(report.message.contains("With `code`, into ~/.vscode/extensions:"), "{}", report.message);
        assert!(report.message.contains("With `codium` and `code-oss`, which both use ~/.vscode-oss/extensions:"), "{}", report.message);

        // Remove goes through the same commands and folders
        for name in ["code", "codium", "code-oss"] {
            std::fs::write(s.path(name).join("installed.txt"), "jawata.jawata-chat@0.2.0\njawata.jawata-java@0.1.0").unwrap();
            std::fs::write(s.path(name).join("calls.txt"), "").unwrap();
        }
        let removed = remove_extensions(&env);
        assert_eq!(removed.outcome, Outcome::Success, "{}", removed.message);
        for name in ["code", "codium"] {
            let made = calls(&s.path(name));
            assert!(made.contains(&"--uninstall-extension jawata.jawata-chat".to_string()) && made.contains(&"--uninstall-extension jawata.jawata-java".to_string()), "{name}: {made:?}");
        }
        assert!(calls(&s.path("code-oss")).is_empty());
        assert!(removed.message.contains("With `codium` and `code-oss`, which both use ~/.vscode-oss/extensions: removed jawata chat and jawata Java"), "{}", removed.message);
    }

    /// D56 (c): where the build's product.json cannot be read the result says so instead of guessing a folder, and the build is still served.
    #[test]
    fn a_build_whose_extensions_folder_cannot_be_read_says_so_and_still_gets_the_extensions() {
        let s = Scratch::new("unknownfolder");
        fake_editor(&s.path("code"), "code", &[], Some(".vscode"));
        fake_editor(&s.path("code-oss"), "code-oss", &[], None);
        let env = EditorEnv {
            path_var: Some(std::env::join_paths([s.path("code/bin"), s.path("code-oss/bin")]).unwrap()),
            ..env_over(&s.0, Some(vsix_folder(&s.0)))
        };

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert!(calls(&s.path("code-oss")).iter().any(|c| c.contains("jawata-java-")), "{:?}", calls(&s.path("code-oss")));
        assert!(report.message.contains("With `code-oss` (its extensions folder is not known: no product.json was found next to"), "{}", report.message);
        assert!(report.message.contains("With `code`, into ~/.vscode/extensions:"), "{}", report.message);

        // the macOS layout keeps product.json one folder above `bin`; a file without the name, or with a path in it, is not used
        let mac = s.path("Visual Studio Code.app/Contents/Resources/app");
        std::fs::create_dir_all(mac.join("bin")).unwrap();
        std::fs::write(mac.join("product.json"), r#"{"dataFolderName": ".vscode"}"#).unwrap();
        assert_eq!(data_folder_name(&mac.join("bin").join("code")), Ok(".vscode".to_string()));
        std::fs::write(mac.join("product.json"), r#"{"nameShort": "Code"}"#).unwrap();
        assert!(data_folder_name(&mac.join("bin").join("code")).unwrap_err().contains("dataFolderName"));
        std::fs::write(mac.join("product.json"), r#"{"dataFolderName": "../elsewhere"}"#).unwrap();
        assert!(data_folder_name(&mac.join("bin").join("code")).is_err());
    }

    /// D54 (d): with Microsoft's Project Manager for Java installed the Explorer showed Java Projects twice. The deploy removes it, after
    /// jawata Java is in, and names it in the result; with jawata Java not installed it stays, so a Java Projects view is never none.
    #[test]
    fn the_second_java_projects_view_goes_after_jawata_java_is_in_and_is_named() {
        let s = Scratch::new("projectmanager");
        fake_code(&s.0, &["redhat.java", "vscjava.vscode-java-dependency", "vscjava.vscode-maven"]);
        let vsix = vsix_folder(&s.0);
        let env = env_over(&s.0, Some(vsix.clone()));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        let all = calls(&s.0);
        let uninstall = all.iter().position(|c| c == "--uninstall-extension vscjava.vscode-java-dependency").unwrap_or_else(|| panic!("{all:?}"));
        let java = all.iter().position(|c| c.contains("jawata-java-")).unwrap();
        assert!(uninstall > java, "after jawata Java is in, never before: {all:?}");
        let removed: Vec<&str> = all.iter().filter_map(|c| c.strip_prefix("--uninstall-extension ")).collect();
        assert_eq!(removed, ["vscjava.vscode-java-dependency", "redhat.java"], "Maven for Java stays: {all:?}");
        assert!(report.message.contains("removed Project Manager for Java, because jawata Java brings its own Java Projects view"), "named: {}", report.message);
        assert!(!report.message.contains("left in place"), "{}", report.message);

        let kept = Scratch::new("projectmanagerkept");
        fake_code(&kept.0, &["vscjava.vscode-java-dependency"]);
        std::fs::write(kept.path("fail.txt"), "jawata-java").unwrap();
        let report = deploy_extensions(&env_over(&kept.0, Some(vsix_folder(&kept.0))), &kept.path("cache"));
        assert_eq!(report.outcome, Outcome::Failed);
        assert!(calls(&kept.0).iter().all(|c| !c.contains("--uninstall-extension")), "{:?}", calls(&kept.0));
    }

    #[test]
    fn without_the_rival_nothing_is_uninstalled_and_a_repeat_deploy_makes_the_same_calls() {
        let s = Scratch::new("norival");
        fake_code(&s.0, &["ms-python.python"]);
        let env = env_over(&s.0, Some(vsix_folder(&s.0)));
        deploy_extensions(&env, &s.path("cache"));
        deploy_extensions(&env, &s.path("cache"));
        let all = calls(&s.0);
        assert_eq!(all.len(), 6, "{all:?}");
        assert!(all.iter().all(|c| !c.contains("--uninstall-extension")), "{all:?}");
        assert!(!all.iter().any(|c| c.contains("install-extension redhat")), "the rival is never installed back");
    }

    #[test]
    fn red_hat_is_not_removed_while_jawata_java_could_not_be_installed_and_only_that_failure_is_reported() {
        let s = Scratch::new("rivalkept");
        fake_code(&s.0, &["redhat.java"]);
        std::fs::write(s.path("fail.txt"), "jawata-java").unwrap();
        let env = env_over(&s.0, Some(vsix_folder(&s.0)));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Failed);
        assert!(calls(&s.0).iter().all(|c| !c.contains("--uninstall-extension")), "{:?}", calls(&s.0));
        assert!(report.message.contains("boom"), "the editor's own words reach the result: {}", report.message);
        assert!(report.changed.iter().any(|c| c.contains("installed jawata chat")), "the chat still went in: {:?}", report.changed);
    }

    #[test]
    fn no_code_on_path_is_said_and_runs_nothing() {
        let s = Scratch::new("nocode");
        let mut env = env_over(&s.0, Some(vsix_folder(&s.0)));
        env.path_var = Some(std::env::join_paths([s.path("empty")]).unwrap());
        let report = deploy_extensions(&env, &s.path("cache"));
        assert_eq!(report.outcome, Outcome::Skipped);
        assert!(report.message.contains("no `code` command on PATH"), "{}", report.message);
        assert_eq!(remove_extensions(&env).outcome, Outcome::Skipped);
        env.path_var = None;
        assert_eq!(deploy_extensions(&env, &s.path("cache")).outcome, Outcome::Skipped, "no PATH at all");
    }

    #[test]
    fn a_code_that_lives_in_a_cursor_folder_is_never_run() {
        let s = Scratch::new("cursor");
        let cursor_bin = s.path("Cursor/resources/app/bin");
        std::fs::create_dir_all(&cursor_bin).unwrap();
        let fake = fake_code(&s.path("Cursor"), &[]);
        std::fs::rename(&fake, cursor_bin.join("code")).unwrap();
        let env = EditorEnv { path_var: Some(std::env::join_paths([cursor_bin.clone()]).unwrap()), ..env_over(&s.0, Some(vsix_folder(&s.0))) };

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Skipped);
        assert!(report.message.contains("Cursor's"), "{}", report.message);
        assert!(calls(&s.path("Cursor")).is_empty(), "nothing was run");

        // beside Cursor's `code`, a VSCodium on PATH is still served, and Cursor's is named as left alone
        fake_editor(&s.path("codium"), "codium", &[], Some(".vscode-oss"));
        let env = EditorEnv { path_var: Some(std::env::join_paths([cursor_bin.clone(), s.path("codium/bin")]).unwrap()), ..env };
        let report = deploy_extensions(&env, &s.path("cache"));
        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert!(calls(&s.path("codium")).iter().any(|c| c.contains("jawata-java-")), "{:?}", calls(&s.path("codium")));
        assert!(calls(&s.path("Cursor")).is_empty(), "Cursor's is still never run");
        assert!(report.message.contains("Cursor's") && report.message.contains("left alone"), "{}", report.message);

        // the control: a folder that only has Cursor in a longer name is not Cursor's
        assert!(!in_a_cursor_folder(Path::new("/home/h/CursorProjects/bin/code")));
        assert!(in_a_cursor_folder(Path::new("/home/h/.cursor/bin/code")));
    }

    #[test]
    fn a_folder_without_the_files_says_which_are_missing_and_installs_nothing() {
        let s = Scratch::new("empty-folder");
        fake_code(&s.0, &["redhat.java"]);
        let empty = s.path("vsix");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("jawata-chat-0.1.0.vsix"), b"zip").unwrap();
        let env = env_over(&s.0, Some(empty));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Failed);
        assert!(report.message.contains("holds no jawata-java-<version>.vsix"), "{}", report.message);
        let all = calls(&s.0);
        assert!(all.iter().any(|c| c.contains("jawata-chat-0.1.0.vsix")), "the file that is there is installed: {all:?}");
        assert!(all.iter().all(|c| !c.contains("--uninstall-extension")), "Red Hat's is not removed without jawata Java: {all:?}");
    }

    #[test]
    fn remove_uninstalls_the_two_extensions_that_are_there_and_never_the_rival() {
        let s = Scratch::new("remove");
        fake_code(&s.0, &["jawata.jawata-chat", "redhat.java"]);
        let env = env_over(&s.0, None);
        let report = remove_extensions(&env);
        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert_eq!(calls(&s.0), vec!["--list-extensions --show-versions".to_string(), "--uninstall-extension jawata.jawata-chat".to_string()]);
    }

    type Routes = BTreeMap<String, (String, Vec<u8>)>;

    /// A one-purpose HTTP server on localhost. The routes are built from the server's own base address, so a release
    /// document can point its assets back at the same server.
    struct Server {
        base: String,
    }
    fn serve(build: impl FnOnce(&str) -> Routes) -> Server {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = build(&base);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, ctype, body) = match routes.get(&path) {
                    Some((ctype, body)) => ("200 OK", ctype.clone(), body.clone()),
                    None => ("404 Not Found", "text/plain".to_string(), b"no".to_vec()),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        Server { base }
    }

    fn release_json(base: &str, assets: &[(&str, usize)]) -> String {
        let assets: Vec<Value> = assets
            .iter()
            .map(|(name, size)| json!({"name": name, "browser_download_url": format!("{base}/dl/{name}"), "size": size}))
            .collect();
        json!({"tag_name": "v0.1.0", "assets": assets}).to_string()
    }

    fn file(bytes: u8, len: usize) -> (String, Vec<u8>) {
        ("application/octet-stream".to_string(), vec![bytes; len])
    }

    fn release(base: &str, assets: &[(&str, usize)]) -> (String, Vec<u8>) {
        ("application/json".to_string(), release_json(base, assets).into_bytes())
    }

    #[test]
    fn the_latest_release_is_downloaded_checked_against_its_stated_size_and_installed() {
        let s = Scratch::new("net");
        fake_code(&s.0, &[]);
        let server = serve(|base| {
            let mut routes = Routes::new();
            routes.insert("/dl/jawata-chat-0.1.0.vsix".into(), file(1, 300));
            routes.insert("/dl/jawata-java-0.1.0.vsix".into(), file(2, 500));
            routes.insert("/releases/latest".into(), release(base, &[("jawata-chat-0.1.0.vsix", 300), ("jawata-java-0.1.0.vsix", 500)]));
            routes
        });
        let mut env = env_over(&s.0, None);
        env.release_api = Some(format!("{}/releases/latest", server.base));
        let cache = s.path("cache");

        let report = deploy_extensions(&env, &cache);

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert_eq!(std::fs::metadata(cache.join("jawata-chat-0.1.0.vsix")).unwrap().len(), 300);
        assert_eq!(std::fs::metadata(cache.join("jawata-java-0.1.0.vsix")).unwrap().len(), 500);
        assert!(!cache.join(".partial-jawata-chat-0.1.0.vsix").exists(), "no half file is left");
        let made = calls(&s.0);
        assert_eq!(made.len(), 3, "list, two installs, no rival: {made:?}");
        assert!(made[1].starts_with("--install-extension ") && made[1].ends_with("jawata-chat-0.1.0.vsix --force"), "{made:?}");
        assert!(report.message.contains("v0.1.0"), "the result names the release: {}", report.message);
    }

    #[test]
    fn a_release_without_the_assets_says_so_and_a_wrong_size_is_refused() {
        let s = Scratch::new("noassets");
        fake_code(&s.0, &["redhat.java"]);
        // 1. a release with no .vsix at all
        let server = serve(|base| {
            let mut routes = Routes::new();
            routes.insert("/releases/latest".into(), release(base, &[("notes.txt", 5)]));
            routes
        });
        let mut env = env_over(&s.0, None);
        env.release_api = Some(format!("{}/releases/latest", server.base));
        let report = deploy_extensions(&env, &s.path("cache"));
        assert_eq!(report.outcome, Outcome::Failed, "{}", report.message);
        assert!(report.message.contains("has no jawata-chat-<version>.vsix asset"), "{}", report.message);
        assert!(report.message.contains("has no jawata-java-<version>.vsix asset"), "{}", report.message);
        assert!(calls(&s.0).is_empty(), "nothing is run when there is nothing to install: {:?}", calls(&s.0));

        // 2. a file whose size is not the one the release states is removed, not installed
        let server = serve(|base| {
            let mut routes = Routes::new();
            routes.insert("/dl/jawata-chat-0.1.0.vsix".into(), file(1, 10));
            routes.insert("/dl/jawata-java-0.1.0.vsix".into(), file(2, 20));
            routes.insert("/releases/latest".into(), release(base, &[("jawata-chat-0.1.0.vsix", 999), ("jawata-java-0.1.0.vsix", 20)]));
            routes
        });
        env.release_api = Some(format!("{}/releases/latest", server.base));
        let cache = s.path("cache2");
        let report = deploy_extensions(&env, &cache);
        assert_eq!(report.outcome, Outcome::Failed);
        assert!(report.message.contains("came to 10 bytes, but the release says 999"), "{}", report.message);
        assert!(!cache.join("jawata-chat-0.1.0.vsix").exists() && !cache.join(".partial-jawata-chat-0.1.0.vsix").exists());
        let made = calls(&s.0);
        assert!(made.iter().all(|c| !c.contains("jawata-chat")), "the short file was never installed: {made:?}");
        assert!(made.iter().any(|c| c.contains("jawata-java-0.1.0.vsix")), "the good one was: {made:?}");
    }

    #[test]
    fn a_release_that_does_not_exist_is_said_plainly() {
        let s = Scratch::new("404");
        fake_code(&s.0, &[]);
        let server = serve(|_| Routes::new());
        let mut env = env_over(&s.0, None);
        env.release_api = Some(format!("{}/releases/latest", server.base));
        let report = deploy_extensions(&env, &s.path("cache"));
        assert_eq!(report.outcome, Outcome::Failed);
        assert!(report.message.contains("has no published release yet"), "{}", report.message);
    }

    // ----- settings -----

    fn launch() -> Launch {
        Launch::for_resident("/e/jawata.jar", "http://127.0.0.1:7001/mcp", "k-1")
    }

    fn handoff() -> &'static Path {
        Path::new("/home/h/.config/jawata-studio/editor-workspaces")
    }

    #[test]
    fn the_server_launch_is_lsp_with_the_same_address_and_key_and_the_key_out_of_the_arguments() {
        let text = editor_keys_merged(None, &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root[JAVA_SERVER_KEY]["command"], "java");
        assert_eq!(root[JAVA_SERVER_KEY]["args"], json!(["-jar", "/e/jawata.jar", "--lsp"]));
        assert_eq!(root[JAVA_SERVER_KEY]["env"]["JAWATA_URL"], "http://127.0.0.1:7001");
        assert_eq!(root[JAVA_SERVER_KEY]["env"]["JAWATA_KEY"], "k-1");
        assert!(!root[JAVA_SERVER_KEY]["args"].to_string().contains("k-1"));
        for key in [CHAT_STUDIO_KEY, JAVA_STUDIO_KEY] {
            assert_eq!(
                root[key],
                json!({"command": "/opt/jawata-studio", "args": ["--background"], "handoffDir": "/home/h/.config/jawata-studio/editor-workspaces"}),
                "{key}"
            );
        }
    }

    #[test]
    fn the_keys_go_into_a_commented_settings_file_and_everything_else_stays_byte_for_byte() {
        let existing = "{\n  // my editor\n  \"editor.fontSize\": 14, // big\n  \"jawataChat.agent\": { \"command\": \"java\", \"args\": [\"-jar\", \"/e/jawata.jar\", \"--acp\"], \"env\": { \"JAWATA_URL\": \"http://x\" } },\n  /* tail */\n  \"files.autoSave\": \"off\",\n}\n";
        let text = editor_keys_merged(Some(existing), &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).unwrap();
        for kept in ["// my editor", "\"editor.fontSize\": 14, // big", "/* tail */", "\"files.autoSave\": \"off\"", "\"jawataChat.agent\": { \"command\": \"java\", \"args\": [\"-jar\", \"/e/jawata.jar\", \"--acp\"], \"env\": { \"JAWATA_URL\": \"http://x\" } },"] {
            assert!(text.contains(kept), "lost {kept:?} in\n{text}");
        }
        let value = jsonc_edit::parse(&text).unwrap().to_value();
        assert_eq!(value[JAVA_SERVER_KEY]["args"][2], "--lsp");
        assert_eq!(value[CHAT_STUDIO_KEY]["command"], "/opt/jawata-studio");
        assert_eq!(value["editor.fontSize"], 14);
        // a second deploy of the same thing changes nothing
        let again = editor_keys_merged(Some(&text), &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).unwrap();
        assert!(jsonc_edit::same_json(&text, &again));
        // and a moved studio is synced in place, not appended
        let moved = editor_keys_merged(Some(&text), &launch(), Some(Path::new("/opt/other/jawata-studio")), handoff()).unwrap();
        assert_eq!(moved.matches(CHAT_STUDIO_KEY).count(), 1, "{moved}");
        assert_eq!(jsonc_edit::parse(&moved).unwrap().to_value()[CHAT_STUDIO_KEY]["command"], "/opt/other/jawata-studio");
    }

    #[test]
    fn both_studio_keys_carry_the_background_flag_and_the_handoff_folder_the_extension_reads() {
        let text = editor_keys_merged(None, &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        for key in [CHAT_STUDIO_KEY, JAVA_STUDIO_KEY] {
            assert_eq!(root[key]["args"], json!(["--background"]), "{key}");
            assert_eq!(root[key]["handoffDir"], "/home/h/.config/jawata-studio/editor-workspaces", "{key}: where the studio leaves its answer");
        }
        // the folder is the one the studio writes into, not a copy of its name
        assert!(handoff().ends_with(crate::editor_launch::HANDOFF_FOLDER));
        // a moved handoff folder is synced in place, not appended
        let moved = editor_keys_merged(Some(&text), &launch(), Some(Path::new("/opt/jawata-studio")), Path::new("/x/editor-workspaces")).unwrap();
        assert_eq!(moved.matches("handoffDir").count(), 2, "one per key: {moved}");
        assert_eq!(jsonc_edit::parse(&moved).unwrap().to_value()[CHAT_STUDIO_KEY]["handoffDir"], "/x/editor-workspaces");
    }

    #[test]
    fn a_studio_entry_from_before_the_handoff_folder_is_still_ours_and_is_brought_up_to_date() {
        let old = r#"{"jawataChat.studio":{"command":"/opt/jawata-studio","args":["--background"]},"jawataJava.studio":{"command":"/opt/jawata-studio","args":[]}}"#;
        let text = editor_keys_merged(Some(old), &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).unwrap();
        let value = jsonc_edit::parse(&text).unwrap().to_value();
        for key in [CHAT_STUDIO_KEY, JAVA_STUDIO_KEY] {
            assert_eq!(value[key]["handoffDir"], "/home/h/.config/jawata-studio/editor-workspaces", "{key}");
            assert_eq!(value[key]["args"], json!(["--background"]), "{key}");
        }
        // an entry whose handoffDir is not a string is not what this module wrote
        let odd = r#"{"jawataChat.studio":{"command":"/opt/jawata-studio","args":["--background"],"handoffDir":7}}"#;
        assert!(editor_keys_merged(Some(odd), &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).is_err());
        // and remove takes out both the old and the new shape
        assert_eq!(jsonc_edit::parse(&editor_keys_without(old).unwrap()).unwrap().to_value(), json!({}));
        assert_eq!(jsonc_edit::parse(&editor_keys_without(&text).unwrap()).unwrap().to_value(), json!({}));
    }

    #[test]
    fn the_settings_row_writes_the_handoff_folder_into_the_real_file_and_removal_takes_it_out_again() {
        let s = Scratch::new("row");
        let home = s.path("home");
        let config = s.path("config");
        let target = settings_target(&home, &config, false);
        let entries: Result<Vec<(String, Launch)>, String> = Ok(vec![("jawata".to_string(), launch())]);
        let no_backup = |_: &Path| -> Result<Option<PathBuf>, String> { Ok(None) };
        let dir = s.path("studio-config/editor-workspaces");

        let written = apply_settings_row(&target, &entries, Some(Path::new("/opt/jawata-studio")), &dir, RowAction::Write, &no_backup);

        assert_eq!(written.state, RowState::Success, "{}", written.message);
        let value = jsonc_edit::parse(&std::fs::read_to_string(&target.path).unwrap()).unwrap().to_value();
        assert_eq!(value[CHAT_STUDIO_KEY]["handoffDir"], dir.to_string_lossy().as_ref());
        assert_eq!(value[JAVA_STUDIO_KEY]["handoffDir"], dir.to_string_lossy().as_ref());
        let removed = apply_settings_row(&target, &entries, Some(Path::new("/opt/jawata-studio")), &dir, RowAction::Delete, &no_backup);
        assert_eq!(removed.state, RowState::Success, "{}", removed.message);
        let after = jsonc_edit::parse(&std::fs::read_to_string(&target.path).unwrap()).unwrap().to_value();
        assert!(after.get(CHAT_STUDIO_KEY).is_none() && after.get(JAVA_STUDIO_KEY).is_none() && after.get(JAVA_SERVER_KEY).is_none(), "{after}");
    }

    #[test]
    fn a_key_the_person_set_themselves_is_never_overwritten() {
        let theirs = r#"{"jawataJava.server":{"command":"mine","args":[]}}"#;
        let error = editor_keys_merged(Some(theirs), &launch(), None, handoff()).expect_err("their own server stays theirs");
        assert!(error.contains("jawataJava.server") && error.contains("not jawata's own"), "{error}");
        let theirs = r#"{"jawataChat.studio":{"command":"/bin/sh","args":["-c","x"]}}"#;
        assert!(editor_keys_merged(Some(theirs), &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).is_err());
    }

    #[test]
    fn a_settings_file_that_is_not_json_is_refused_not_rewritten() {
        let error = editor_keys_merged(Some("{ \"a\": "), &launch(), None, handoff()).expect_err("refused");
        assert!(error.contains("does not read as JSON"), "{error}");
    }

    #[test]
    fn remove_takes_out_exactly_the_keys_the_deploy_wrote_and_nothing_else() {
        let before = "{\n  // keep me\n  \"editor.fontSize\": 14,\n  \"jawataChat.agent\": {\"command\":\"mine\"}\n}\n";
        let deployed = editor_keys_merged(Some(before), &launch(), Some(Path::new("/opt/jawata-studio")), handoff()).unwrap();
        assert!(deployed.contains(JAVA_SERVER_KEY));
        let removed = editor_keys_without(&deployed).unwrap();
        assert_eq!(jsonc_edit::parse(&removed).unwrap().to_value(), jsonc_edit::parse(before).unwrap().to_value(), "{removed}");
        assert!(removed.contains("// keep me") && removed.contains("\"jawataChat.agent\": {\"command\":\"mine\"}"), "{removed}");
        // a key that is not ours is left in place
        let mixed = r#"{"jawataJava.server":{"command":"mine"},"jawataChat.studio":{"command":"/opt/jawata-studio","args":["--background"]}}"#;
        let kept = jsonc_edit::parse(&editor_keys_without(mixed).unwrap()).unwrap().to_value();
        assert_eq!(kept, json!({"jawataJava.server": {"command": "mine"}}));
    }

    #[test]
    fn the_studio_path_prefers_the_installed_file_over_a_mount_and_a_build_keeps_its_own_path() {
        let exists = |p: &Path| p.to_string_lossy().ends_with("jawata-studio") || p.to_string_lossy().ends_with(".AppImage");
        let home = Path::new("/home/h");
        // an AppImage names its own file
        assert_eq!(
            studio_command(Some("/home/h/Apps/jawata.AppImage"), Some(home), Some(Path::new("/tmp/.mount_abc/usr/bin/jawata-studio")), &exists),
            Some(PathBuf::from("/home/h/Apps/jawata.AppImage"))
        );
        // no AppImage variable: the mount path is replaced by the installer's file
        assert_eq!(
            studio_command(None, Some(home), Some(Path::new("/tmp/.mount_abc/usr/bin/jawata-studio")), &exists),
            Some(PathBuf::from("/home/h/.local/bin/jawata-studio"))
        );
        // an installed build keeps the path it runs from
        assert_eq!(
            studio_command(None, Some(home), Some(Path::new("/opt/jawata/jawata-studio")), &exists),
            Some(PathBuf::from("/opt/jawata/jawata-studio"))
        );
        // a dev build keeps its own path, whatever is installed
        assert_eq!(
            studio_command(None, Some(home), Some(Path::new("/src/target/release/jawata-studio")), &exists),
            Some(PathBuf::from("/src/target/release/jawata-studio"))
        );
        assert_eq!(studio_command(None, None, None, &exists), None);
    }
}
