//! The VS Code half of "Deploy to agents" (Sprint 28h, Stage 21 (c)).
//!
//! Deploying to VS Code does four things, and "Remove from agents" takes out exactly those:
//!
//! 1. installs two extensions with `code --install-extension <file.vsix> --force`: the chat
//!    (`jawata-chat-<v>.vsix`) and jawata Java (`jawata-java-<v>.vsix`), from the assets of the latest
//!    release of `haraldwegner/jawata-chat`, or from a local folder named by `JAWATA_STUDIO_VSIX_DIR`;
//! 2. writes the editor settings the extensions read: `jawataJava.server` (the language-server launch,
//!    `--lsp`, the resident's address and key in the environment, never in the arguments) and
//!    `jawataChat.studio` / `jawataJava.studio`, how an extension starts this studio when the engine's
//!    service does not answer: `{command: <this studio>, args: ["--background"], handoffDir: <folder>}` (no window, only the
//!    tray; the extension adds one `--editor-folder <path>` per folder of its workspace, and reads the answer, the matched
//!    jawata workspace's address and key, from `<handoffDir>/<id>.json`, see `editor_launch`). (`jawataChat.agent` is the
//!    existing chat row's, `agent_registration`.) Text edits, so comments and every other key stay;
//! 3. uninstalls `redhat.java` when it is installed, because it competes with jawata Java. Never put back,
//!    and only after jawata Java is installed, so a failed download never leaves a machine with no Java support;
//! 4. leaves Cursor alone: only a `code` found on PATH is run, and one that lives in a Cursor folder is refused.
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
/// The extension that competes with jawata Java for the same files.
pub const RIVAL_EXTENSION_ID: &str = "redhat.java";
/// The repository whose latest release carries the two `.vsix` files.
pub const EXTENSIONS_REPO: &str = "haraldwegner/jawata-chat";
/// A folder of local `.vsix` files, used instead of the network (tests, offline use).
pub const VSIX_DIR_ENV: &str = "JAWATA_STUDIO_VSIX_DIR";

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

// ===== finding `code` =====

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeCommand {
    Found(PathBuf),
    Missing,
    /// The `code` on PATH lives in a Cursor folder; Cursor is never driven.
    Cursors(PathBuf),
}

fn code_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["code.cmd", "code.exe", "code.bat"]
    } else {
        &["code"]
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

pub fn find_code(path_var: Option<&OsStr>) -> CodeCommand {
    let Some(path_var) = path_var else { return CodeCommand::Missing };
    for dir in std::env::split_paths(path_var) {
        for name in code_names() {
            let candidate = dir.join(name);
            if is_runnable(&candidate) {
                let resolved = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
                if in_a_cursor_folder(&candidate) || in_a_cursor_folder(&resolved) {
                    return CodeCommand::Cursors(candidate);
                }
                return CodeCommand::Found(candidate);
            }
        }
    }
    CodeCommand::Missing
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
                return Err(format!("`code {}` did not finish within {} s and was stopped", args.join(" "), timeout.as_secs()));
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
    let mut found = Vsixes { source: format!("{VSIX_DIR_ENV}={}", dir.display()), ..Default::default() };
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
    pub message: String,
    pub changed: Vec<String>,
    pub errors: Vec<String>,
    pub code: Option<PathBuf>,
}

impl ExtensionsReport {
    fn skipped(message: String, code: Option<PathBuf>) -> Self {
        ExtensionsReport { outcome: Outcome::Skipped, message, changed: vec![], errors: vec![], code }
    }
}

const NO_CODE: &str = "VS Code was not found: there is no `code` command on PATH, so no extension was installed or removed. \
Install the command (VS Code's Command Palette: Shell Command: Install 'code' command in PATH) and deploy again.";

fn locate(env: &EditorEnv) -> Result<PathBuf, ExtensionsReport> {
    match find_code(env.path_var.as_deref()) {
        CodeCommand::Found(path) => Ok(path),
        CodeCommand::Missing => Err(ExtensionsReport::skipped(NO_CODE.to_string(), None)),
        CodeCommand::Cursors(path) => Err(ExtensionsReport::skipped(
            format!("the `code` on PATH is {}, which is Cursor's; Cursor is never driven, so nothing was installed or removed.", path.display()),
            None,
        )),
    }
}

fn listed(code: &Path, env: &EditorEnv) -> Result<Vec<String>, String> {
    let out = run_code(code, &["--list-extensions"], env.step_timeout)?;
    if !out.ok {
        return Err(format!("`code --list-extensions` failed (exit {:?}): {}", out.exit, out.stderr.trim()));
    }
    Ok(out.stdout.lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect())
}

/// Install both extensions and remove the rival, in that order and only that far: the rival goes only after
/// jawata Java is in.
pub fn deploy_extensions(env: &EditorEnv, cache: &Path) -> ExtensionsReport {
    let code = match locate(env) {
        Ok(code) => code,
        Err(report) => return report,
    };
    let files = obtain_vsixes(env, cache);
    let mut changed = Vec::new();
    let mut errors = files.problems.clone();
    if files.chat.is_none() && files.java.is_none() {
        return ExtensionsReport {
            outcome: Outcome::Failed,
            message: format!("no extension was installed: {}", errors.join("; ")),
            changed,
            errors,
            code: Some(code),
        };
    }
    let before = match listed(&code, env) {
        Ok(list) => list,
        Err(e) => {
            errors.push(e);
            return ExtensionsReport {
                outcome: Outcome::Failed,
                message: format!("no extension was installed: {}", errors.join("; ")),
                changed,
                errors,
                code: Some(code),
            };
        }
    };
    let mut java_in = false;
    for (id, file) in [(CHAT_EXTENSION_ID, &files.chat), (JAVA_EXTENSION_ID, &files.java)] {
        let Some(file) = file else { continue };
        let path = file.to_string_lossy().into_owned();
        match run_code(&code, &["--install-extension", &path, "--force"], env.step_timeout) {
            Ok(out) if out.ok => {
                changed.push(format!("installed {id} from {}", file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                java_in |= id == JAVA_EXTENSION_ID;
            }
            Ok(out) => errors.push(format!("`code --install-extension {path} --force` failed (exit {:?}): {}", out.exit, out.stderr.trim())),
            Err(e) => errors.push(e),
        }
    }
    if before.iter().any(|id| id == RIVAL_EXTENSION_ID) {
        if java_in {
            match run_code(&code, &["--uninstall-extension", RIVAL_EXTENSION_ID], env.step_timeout) {
                Ok(out) if out.ok => changed.push(format!("uninstalled {RIVAL_EXTENSION_ID}")),
                Ok(out) => errors.push(format!("`code --uninstall-extension {RIVAL_EXTENSION_ID}` failed (exit {:?}): {}", out.exit, out.stderr.trim())),
                Err(e) => errors.push(e),
            }
        } else {
            errors.push(format!("{RIVAL_EXTENSION_ID} was left installed because jawata Java was not installed"));
        }
    }
    let outcome = if errors.is_empty() { Outcome::Success } else { Outcome::Failed };
    let mut message = format!("VS Code: {} (source: {})", if changed.is_empty() { "nothing changed".to_string() } else { changed.join(", ") }, files.source);
    if !errors.is_empty() {
        message.push_str(&format!(". Not done: {}", errors.join("; ")));
    }
    ExtensionsReport { outcome, message, changed, errors, code: Some(code) }
}

/// Take out the two extensions the deploy installed. `redhat.java` is not put back: the deploy removed it for
/// good, and the person installs it again if they want it.
pub fn remove_extensions(env: &EditorEnv) -> ExtensionsReport {
    let code = match locate(env) {
        Ok(code) => code,
        Err(report) => return report,
    };
    let present = match listed(&code, env) {
        Ok(list) => list,
        Err(e) => {
            return ExtensionsReport { outcome: Outcome::Failed, message: e.clone(), changed: vec![], errors: vec![e], code: Some(code) };
        }
    };
    let mut changed = Vec::new();
    let mut errors = Vec::new();
    for id in [CHAT_EXTENSION_ID, JAVA_EXTENSION_ID] {
        if !present.iter().any(|p| p == id) {
            continue;
        }
        match run_code(&code, &["--uninstall-extension", id], env.step_timeout) {
            Ok(out) if out.ok => changed.push(format!("uninstalled {id}")),
            Ok(out) => errors.push(format!("`code --uninstall-extension {id}` failed (exit {:?}): {}", out.exit, out.stderr.trim())),
            Err(e) => errors.push(e),
        }
    }
    let outcome = if errors.is_empty() { Outcome::Success } else { Outcome::Failed };
    let mut message = if changed.is_empty() && errors.is_empty() {
        "VS Code: neither jawata extension is installed; nothing to remove".to_string()
    } else {
        format!("VS Code: {}", changed.join(", "))
    };
    if !errors.is_empty() {
        message.push_str(&format!(". Not done: {}", errors.join("; ")));
    }
    ExtensionsReport { outcome, message, changed, errors, code: Some(code) }
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
    let ours: Vec<usize> = EDITOR_KEYS
        .iter()
        .filter_map(|key| {
            let index = root.member_index(key)?;
            let value = root.members[index].value.to_value();
            let is_ours = if *key == JAVA_SERVER_KEY { is_java_server_entry(&value) } else { is_studio_entry(&value) };
            is_ours.then_some(index)
        })
        .collect();
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

    /// A `code` that records every call to `calls.txt` (one line per call, arguments joined by a space) and answers
    /// `--list-extensions` with the lines of `installed.txt`. Exits non-zero for any argument in `fail.txt`.
    pub(crate) fn fake_code(dir: &Path, installed: &[&str]) -> PathBuf {
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(dir.join("installed.txt"), installed.join("\n")).unwrap();
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{calls}'\nif [ \"$1\" = \"--list-extensions\" ]; then cat '{installed}'; echo; exit 0; fi\n\
             if [ -f '{fail}' ]; then pat=$(cat '{fail}'); case \"$2\" in *\"$pat\"*) echo 'boom' >&2; exit 3;; esac; fi\nexit 0\n",
            calls = dir.join("calls.txt").display(),
            installed = dir.join("installed.txt").display(),
            fail = dir.join("fail.txt").display(),
        );
        let path = bin.join("code");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
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

    #[test]
    fn the_files_are_picked_by_exact_name_and_the_newest_version() {
        assert_eq!(vsix_version("jawata-chat-0.2.0.vsix", "jawata-chat"), semver::Version::parse("0.2.0").ok());
        assert_eq!(vsix_version("jawata-chat-view-9.9.9.vsix", "jawata-chat"), None, "a different package is not the chat");
        assert_eq!(vsix_version("jawata-chat-0.2.0.zip", "jawata-chat"), None);
        let names = ["jawata-chat-0.1.0.vsix", "jawata-chat-0.10.0.vsix", "jawata-chat-0.9.0.vsix"];
        assert_eq!(newest_vsix(names, "jawata-chat"), Some("jawata-chat-0.10.0.vsix"), "0.10 is newer than 0.9");
    }

    #[test]
    fn a_deploy_installs_both_extensions_removes_the_rival_and_makes_exactly_these_calls() {
        let s = Scratch::new("deploy");
        fake_code(&s.0, &["ms-python.python", "redhat.java", "REDHAT.vscode-yaml"]);
        let vsix = vsix_folder(&s.0);
        let env = env_over(&s.0, Some(vsix.clone()));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert_eq!(
            calls(&s.0),
            vec![
                "--list-extensions".to_string(),
                format!("--install-extension {} --force", vsix.join("jawata-chat-0.2.0.vsix").display()),
                format!("--install-extension {} --force", vsix.join("jawata-java-0.1.0.vsix").display()),
                "--uninstall-extension redhat.java".to_string(),
            ]
        );
        assert!(report.message.contains("uninstalled redhat.java"), "{}", report.message);
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
    fn the_rival_is_kept_when_jawata_java_could_not_be_installed() {
        let s = Scratch::new("rivalkept");
        fake_code(&s.0, &["redhat.java"]);
        std::fs::write(s.path("fail.txt"), "jawata-java").unwrap();
        let env = env_over(&s.0, Some(vsix_folder(&s.0)));

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Failed);
        assert!(calls(&s.0).iter().all(|c| !c.contains("--uninstall-extension")), "{:?}", calls(&s.0));
        assert!(report.message.contains("redhat.java was left installed"), "{}", report.message);
        assert!(report.message.contains("boom"), "the editor's own words reach the result: {}", report.message);
        assert!(report.changed.iter().any(|c| c.contains(CHAT_EXTENSION_ID)), "the chat still went in");
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
        let env = EditorEnv { path_var: Some(std::env::join_paths([cursor_bin]).unwrap()), ..env_over(&s.0, Some(vsix_folder(&s.0))) };

        let report = deploy_extensions(&env, &s.path("cache"));

        assert_eq!(report.outcome, Outcome::Skipped);
        assert!(report.message.contains("Cursor's"), "{}", report.message);
        assert!(calls(&s.path("Cursor")).is_empty(), "nothing was run");
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
        assert!(all.iter().all(|c| !c.contains("--uninstall-extension")), "the rival stays: {all:?}");
    }

    #[test]
    fn remove_uninstalls_the_two_extensions_that_are_there_and_never_the_rival() {
        let s = Scratch::new("remove");
        fake_code(&s.0, &["jawata.jawata-chat", "redhat.java"]);
        let env = env_over(&s.0, None);
        let report = remove_extensions(&env);
        assert_eq!(report.outcome, Outcome::Success, "{}", report.message);
        assert_eq!(calls(&s.0), vec!["--list-extensions".to_string(), "--uninstall-extension jawata.jawata-chat".to_string()]);
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
