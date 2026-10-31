use crate::config::{
    current_timestamp_string, display_path, effective_release_repo, ManagerSettings, UpdatePolicy,
};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tar::Archive;
use walkdir::WalkDir;
use zip::ZipArchive;

use crate::engine_activity;

/// Where an archive that is being downloaded waits (28h S2a, D35). The name is the same for every attempt at the same
/// release, which is what lets the next attempt find what the last one left.
fn partial_archive_path(tools_dir: &Path, release: &RemoteRelease) -> PathBuf {
    tools_dir.join(format!(".partial-{}-{}", release.version, release.asset_name))
}

/// Removes every archive left waiting, once an engine is installed: they are of no use to a newer release.
fn remove_partial_archives(tools_dir: &Path) {
    if let Ok(entries) = fs::read_dir(tools_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(".partial-") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Brings `url` down into `path`, continuing from the bytes already there. Returns the size of the file when it is complete.
///
/// A file of N bytes is continued with `Range: bytes=N-`. A server that answers 206 is appended to; one that answers
/// 200 has ignored the range and is sending everything, so the file is begun again; one that answers 416 says the
/// range does not fit what it has (the file is as long as the resource, or from another version), so the file is
/// removed and fetched whole. A body that breaks off leaves the file as far as it got.
fn fetch_resumable(client: &Client, url: &str, path: &Path) -> Result<u64, String> {
    let mut have = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    for _ in 0..2 {
        let mut request = client.get(url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = request.send().map_err(|error| {
            format!("failed to download JAWATA release archive from {url}: {}", describe_error_chain(&error))
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
            let _ = fs::remove_file(path);
            have = 0;
            continue;
        }
        let mut response = response
            .error_for_status()
            .map_err(|error| format!("JAWATA archive download failed: {error}"))?;
        let resuming = status == reqwest::StatusCode::PARTIAL_CONTENT && have > 0;
        let mut file = if resuming {
            fs::OpenOptions::new().append(true).open(path)
        } else {
            fs::File::create(path)
        }
        .map_err(|error| format!("failed to open archive staging file {}: {error}", path.display()))?;
        let copied = response.copy_to(&mut file).map_err(|error| {
            format!("failed to download JAWATA archive body from {url}: {}", describe_error_chain(&error))
        })?;
        return Ok(if resuming { have + copied } else { copied });
    }
    Err(format!("the server refused to continue or restart the download of {url}"))
}

/// Render an error together with its FULL source chain.
///
/// Sprint 28 (v3.6.0): reqwest's `Display` prints only the outermost kind, so
/// a response body aborted by a timeout reads as the bare, undiagnosable
/// "error decoding response body" while the cause that actually names the
/// problem sits one level down. The macOS dogfood lost a day to that message.
fn describe_error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(inner) = source {
        parts.push(inner.to_string());
        source = inner.source();
    }
    parts.join(": ")
}

/// Compose the GitHub releases-API URL for the configured release repo.
/// Source of truth: JAWATA_RELEASE_REPO env var, then settings.release_repo.
fn latest_release_url(settings: &ManagerSettings) -> String {
    format!(
        "https://api.github.com/repos/{}/releases/latest",
        effective_release_repo(settings)
    )
}

/// Represents a cached, managed JAWATA runtime installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRuntimeRecord {
    pub version: String,
    pub install_dir: String,
    pub jar_path: String,
    pub asset_name: String,
    pub installed_at: String,
}

/// The trial a new engine must pass before it replaces a working one: given the unpacked engine's jar, start it,
/// ask what it offers and compare. `Err` carries the reason it is not switched to, in words for Settings.
pub type EngineTrial<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// Why an install did not produce an active engine. Held is not a failure of the download: the engine arrived and
/// was tried, and it is not the one this studio can work with.
#[derive(Debug)]
pub enum InstallError {
    Failed(String),
    Held(String),
}

impl From<String> for InstallError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for InstallError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_string())
    }
}

/// An engine release that was downloaded, tried and not switched to. Remembered, so that the next start or
/// reload does not download 112 MB again to learn the same thing: it applies to that engine version and that
/// studio version, and a newer engine or a newer studio gets a fresh trial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldUpdate {
    pub version: String,
    pub studio_version: String,
    pub reason: String,
}

impl HeldUpdate {
    pub fn applies_to(&self, latest: &str, studio_version: &str) -> bool {
        self.version == latest && self.studio_version == studio_version
    }
}

fn held_path(tools_dir: &Path) -> PathBuf {
    tools_dir.join("held-update.json")
}

fn read_held(tools_dir: &Path) -> Option<HeldUpdate> {
    serde_json::from_str(&fs::read_to_string(held_path(tools_dir)).ok()?).ok()
}

fn write_held(tools_dir: &Path, held: &HeldUpdate) {
    if let Ok(text) = serde_json::to_string_pretty(held) {
        let _ = fs::write(held_path(tools_dir), format!("{text}\n"));
    }
}

fn clear_held(tools_dir: &Path) {
    let _ = fs::remove_file(held_path(tools_dir));
}

fn studio_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Represents the current state of the managed runtime release.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseStatusKind {
    Ready,
    Missing,
    UpdateAvailable,
    CheckFailed,
    CheckingDisabled,
    /// A newer engine was downloaded and tried, and is not the one this studio can work with; the old one stays.
    UpdateHeld,
}

/// Detailed status information about the managed JAWATA release.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseStatus {
    pub kind: ReleaseStatusKind,
    pub latest_version: Option<String>,
    pub default_version: Option<String>,
    pub checked_at: Option<String>,
    pub update_available: bool,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Sprint 21a dogfood fix: the platform token in jawata release asset names
/// (`jawata-v2.1.0-linux-x64.tar.gz` → `linux-x64`).
fn platform_asset_token() -> String {
    let os = if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    format!("{os}-{arch}")
}

/// Pick the release archive for THIS platform. The old logic took the FIRST `.tar.gz` in
/// GitHub's (alphabetical) asset list — which is `linux-arm64`, so an x86_64 machine
/// installed the arm64 build and the resident died on the wrong SWT (live dogfood find,
/// v2.1.0 install). Platform+arch match first; the un-suffixed legacy fallback stays for
/// old releases with a single archive.
fn select_release_asset<'a>(assets: &'a [GitHubAsset], token: &str) -> Option<&'a GitHubAsset> {
    let is_archive =
        |asset: &GitHubAsset| asset.name.ends_with(".tar.gz") || asset.name.ends_with(".zip");
    assets
        .iter()
        .find(|asset| asset.name.contains(token) && is_archive(asset))
        .or_else(|| assets.iter().find(|asset| asset.name.ends_with(".tar.gz")))
        .or_else(|| assets.iter().find(|asset| asset.name.ends_with(".zip")))
}

#[derive(Debug, Clone)]
struct RemoteRelease {
    version: String,
    asset_name: String,
    download_url: String,
    published_at: Option<String>,
    archive_kind: ArchiveKind,
}

#[derive(Debug, Clone, Copy)]
enum ArchiveKind {
    TarGz,
    Zip,
}

/// Manages downloading, caching, and updating the JAWATA runtime.
pub struct ReleaseManager {
    /// Short-budget client for the small JSON metadata calls (releases API).
    client: Client,
    /// Long-budget client for the release ARCHIVE. Kept separate on purpose —
    /// see `new()`.
    download_client: Client,
}

/// D4: what one version check DECIDED, and why.
///
/// The requirement is that *every* version check says what it decided —
/// installed, already current, policy said no, download failed — with the
/// reason. Before this the check composed a sentence beside the branch it took,
/// so the two could disagree and the commonest outcome said the least: a
/// runtime that was NOT updated logged *"Latest upstream release is X"*, which
/// is true whether the policy declined it, it was already current, or nothing
/// had been asked at all.
///
/// So the outcome becomes a VALUE, and both the log line and the user-visible
/// detail are rendered from it. A branch that forgets to say what it did is not
/// expressible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDecision {
    /// Checking is switched off in settings; nothing was asked of the network.
    CheckingDisabled,
    /// The check itself failed, so there is no verdict about the runtime —
    /// which is NOT the same as "no update", and is why it is its own variant.
    CheckFailed { error: String },
    /// Nothing was cached yet. A first runtime is fetched whatever the policy
    /// says: `Ask` is a policy about UPDATING, and there is nothing to update.
    InstalledFirstRuntime { version: String },
    /// A newer release existed and the policy allowed taking it.
    Installed { version: String },
    /// The newest release is the one already installed.
    AlreadyCurrent { version: String },
    /// A newer release exists and the policy is `Ask`, so it waits for a click.
    PolicyDeclined { installed: String, latest: String },
    /// The policy allowed it and the download did not finish.
    DownloadFailed { version: String, error: String },
    /// The engine was downloaded, started once and compared with what this studio expects, and it differs: the
    /// installed engine stays active and `reason` says how the new one differs.
    Held { version: String, reason: String },
    /// The same engine was already tried against this studio version; nothing was downloaded this time.
    HeldAgain { version: String, reason: String },
}

impl UpdateDecision {
    /// The one line a check leaves behind. Written as what it DID and why,
    /// because a log that says only what it saw is what this replaced.
    pub fn log_line(&self) -> String {
        match self {
            Self::CheckingDisabled => {
                "no check — automatic release checks are switched off in settings".into()
            }
            Self::CheckFailed { error } => {
                format!("no verdict — the release check itself failed: {error}")
            }
            Self::InstalledFirstRuntime { version } => {
                format!("installed {version} — nothing was cached yet")
            }
            Self::Installed { version } => {
                format!("installed {version} — it was newer and the update policy allows it")
            }
            Self::AlreadyCurrent { version } => {
                format!("nothing to do — {version} is installed and is the newest release")
            }
            Self::PolicyDeclined { installed, latest } => format!(
                "declined {latest} — the update policy is `ask`, so {installed} stays until \
                 someone presses the button"
            ),
            Self::DownloadFailed { version, error } => {
                format!("could not install {version} — the download failed: {error}")
            }
            Self::Held { version, reason } => format!(
                "kept the installed engine — {version} was downloaded and started once, and it differs from \
                 what this studio expects ({reason}); update the studio first"
            ),
            Self::HeldAgain { version, reason } => format!(
                "kept the installed engine — {version} was already tried against this studio and differs from \
                 what it expects ({reason}); nothing was downloaded"
            ),
        }
    }

    /// Whether this decision means a release was fetched.
    /// Whether a newer engine exists that this studio refused to switch to.
    pub fn is_held(&self) -> bool {
        matches!(self, Self::Held { .. } | Self::HeldAgain { .. })
    }

    pub fn installed_something(&self) -> bool {
        matches!(self, Self::Installed { .. } | Self::InstalledFirstRuntime { .. })
    }
}

/// D4: what a check should DO, from the facts alone.
///
/// Pure, and separate from the check, because this IS the requirement: the
/// download, the settings write and the status rendering are what carry the
/// answer.
///
/// Two decisions are DELIBERATELY not here, and both for the same reason: they
/// are not decidable from the facts this takes. `CheckingDisabled` is settled
/// before the network is touched at all, and `CheckFailed` carries an error
/// only the caller holds. A first version took `auto_check_for_updates` and
/// answered `CheckingDisabled` from it — reachable from no caller, because the
/// check returns before it, so the arm was exercised only by its own unit test.
/// That is "a condition that cannot change the outcome", which is the shape
/// this checkpoint kept finding elsewhere.
pub fn decide_update(
    policy: UpdatePolicy,
    installed: Option<&str>,
    latest: &str,
) -> UpdateDecision {
    let Some(installed) = installed else {
        return UpdateDecision::InstalledFirstRuntime { version: latest.to_string() };
    };
    if installed == latest {
        return UpdateDecision::AlreadyCurrent { version: installed.to_string() };
    }
    match policy {
        UpdatePolicy::Always => UpdateDecision::Installed { version: latest.to_string() },
        UpdatePolicy::Ask => UpdateDecision::PolicyDeclined {
            installed: installed.to_string(),
            latest: latest.to_string(),
        },
    }
}

impl ReleaseManager {
    /// Creates a new release manager with its two HTTP clients.
    ///
    /// Sprint 28 (v3.6.0), macOS dogfood 2026-07-26: the managed-runtime
    /// update was UNINSTALLABLE on every platform. Both the metadata call and
    /// the archive download shared one client carrying
    /// `.timeout(10s)` — and reqwest's `timeout()` is the TOTAL request
    /// budget, running from connect until the response body has finished. The
    /// release archives are ~107 MB, so finishing inside 10 s demanded a
    /// sustained ~10.7 MB/s end-to-end; anything slower had its body stream
    /// aborted mid-download, surfacing as the bare, undiagnosable
    /// "error decoding response body".
    ///
    /// The metadata budget is correct and stays. The download gets its own
    /// client whose budget is sized to the ARCHIVE rather than to a JSON call:
    /// 30 minutes, i.e. a floor of roughly 60 KB/s for a 107 MB asset. It is
    /// deliberately still bounded — an unbounded download would turn a dead
    /// connection into a hang instead of an error — but nothing that is
    /// genuinely making progress can now be cut off mid-body.
    /// (reqwest's blocking builder has no `read_timeout` at 0.12.24, so a
    /// generous total budget is the honest instrument available.)
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .user_agent("jawata-studio/0.1.0")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("failed to create release manager HTTP client: {error}"))?;

        let download_client = Client::builder()
            .user_agent("jawata-studio/0.1.0")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(1800))
            .build()
            .map_err(|error| {
                format!("failed to create release manager download HTTP client: {error}")
            })?;

        Ok(Self {
            client,
            download_client,
        })
    }

    /// Checks for updates and installs the latest release if permitted by settings.
    pub fn sync_with_settings(
        &self,
        settings: &mut ManagerSettings,
        trial: EngineTrial,
    ) -> Result<(Option<ManagedRuntimeRecord>, ReleaseStatus, UpdateDecision), String> {
        let mut installed = self.get_installed_runtime(settings)?;

        if !settings.auto_check_for_updates {
            let status = ReleaseStatus {
                kind: if installed.is_none() {
                    ReleaseStatusKind::Missing
                } else {
                    ReleaseStatusKind::CheckingDisabled
                },
                latest_version: settings.last_seen_latest_version.clone(),
                default_version: installed.as_ref().map(|r| r.version.clone()),
                checked_at: settings.last_release_check.clone(),
                update_available: false,
                detail: UpdateDecision::CheckingDisabled.log_line(),
            };
            return Ok((installed, status, UpdateDecision::CheckingDisabled));
        }

        match self.fetch_latest_release(settings) {
            Ok(release) => {
                let checked_at = current_timestamp_string();
                settings.last_release_check = Some(checked_at.clone());
                settings.last_seen_latest_version = Some(release.version.clone());

                // D4: decide once, from the facts, and let everything else
                // render that decision — the log line, the user-visible detail
                // and whether anything is downloaded at all.
                let mut decision = decide_update(
                    settings.update_policy,
                    installed.as_ref().map(|runtime| runtime.version.as_str()),
                    &release.version,
                );

                let tools_dir = settings.tools_dir();
                // 4.4.14: an engine already tried against this studio and refused is not downloaded again.
                if matches!(decision, UpdateDecision::Installed { .. }) {
                    if let Some(held) = read_held(&tools_dir) {
                        if held.applies_to(&release.version, studio_version()) {
                            decision = UpdateDecision::HeldAgain {
                                version: release.version.clone(),
                                reason: held.reason,
                            };
                        }
                    }
                }

                if decision.installed_something() {
                    // A FAILED download is its own decision rather than an
                    // error thrown past the caller: before this it aborted the
                    // whole sync with `?`, so the one outcome most worth
                    // recording was the one that left no record.
                    //
                    // A REPLACING engine is tried before it is switched to; the first engine has nothing to
                    // be kept in its place, so it is installed without a trial.
                    let replacing = installed.is_some();
                    match self.install_release(&release, settings, replacing.then_some(trial)) {
                        Ok(_) => {
                            clear_held(&tools_dir);
                            installed = self.get_installed_runtime(settings)?
                        }
                        Err(InstallError::Failed(error)) => {
                            decision = UpdateDecision::DownloadFailed {
                                version: release.version.clone(),
                                error,
                            }
                        }
                        Err(InstallError::Held(reason)) => {
                            write_held(
                                &tools_dir,
                                &HeldUpdate {
                                    version: release.version.clone(),
                                    studio_version: studio_version().to_string(),
                                    reason: reason.clone(),
                                },
                            );
                            decision = UpdateDecision::Held { version: release.version.clone(), reason };
                        }
                    }
                }

                let mut status = self.build_release_status(
                    Some(&release),
                    installed.as_ref(),
                    settings,
                    Some(decision.log_line()),
                    None,
                );
                if decision.is_held() {
                    status.kind = ReleaseStatusKind::UpdateHeld;
                    status.update_available = false;
                }

                Ok((installed, status, decision))
            }
            Err(error) => {
                // The UI's sentence IS the decision's, on this branch too. It
                // was composed separately here — so a failed check logged "no
                // verdict — the release check itself failed" while the window
                // read "Could not check the latest JAWATA release", two
                // independently written sentences about one outcome. That is
                // exactly the drift this type was introduced to make
                // inexpressible, surviving on the branch it matters most on.
                let decision = UpdateDecision::CheckFailed { error };
                let status = self.build_release_status(
                    None,
                    installed.as_ref(),
                    settings,
                    None,
                    Some(decision.log_line()),
                );
                Ok((installed, status, decision))
            }
        }
    }

    /// Forces a download and installation of the latest upstream JAWATA release.
    pub fn download_latest_runtime(
        &self,
        settings: &mut ManagerSettings,
        trial: EngineTrial,
    ) -> Result<ManagedRuntimeRecord, String> {
        let release = self.fetch_latest_release(settings)?;
        let tools_dir = settings.tools_dir();
        let replacing = self.get_installed_runtime(settings)?.is_some();
        if replacing {
            if let Some(held) = read_held(&tools_dir) {
                if held.applies_to(&release.version, studio_version()) {
                    return Err(format!("Not switched to {}: {}", release.version, held.reason));
                }
            }
        }
        let runtime = match self.install_release(&release, settings, replacing.then_some(trial)) {
            Ok(runtime) => {
                clear_held(&tools_dir);
                runtime
            }
            Err(InstallError::Failed(error)) => return Err(error),
            Err(InstallError::Held(reason)) => {
                write_held(
                    &tools_dir,
                    &HeldUpdate {
                        version: release.version.clone(),
                        studio_version: studio_version().to_string(),
                        reason: reason.clone(),
                    },
                );
                return Err(format!("Not switched to {}: {reason}", release.version));
            }
        };
        settings.last_release_check = Some(current_timestamp_string());
        settings.last_seen_latest_version = Some(release.version.clone());
        // D4 says EVERY version check names its decision, and this reaches the
        // network and installs, so it is one — it simply reaches it because a
        // person pressed the button rather than because a policy allowed it.
        // Leaving it silent would have made the log's coverage depend on how
        // the download was started.
        crate::studio_log::say(&format!(
            "release check: {}",
            UpdateDecision::Installed { version: runtime.version.clone() }.log_line()
        ));
        Ok(runtime)
    }

    /// Computes the release status based on locally cached metadata without making network requests.
    pub fn status_from_cached_settings(
        &self,
        settings: &ManagerSettings,
    ) -> Result<(Option<ManagedRuntimeRecord>, ReleaseStatus), String> {
        let installed = self.get_installed_runtime(settings)?;
        let status = self.build_cached_release_status(installed.as_ref(), settings);
        Ok((installed, status))
    }

    /// Retrieves the currently installed managed runtime record, if any exists.
    pub fn get_installed_runtime(
        &self,
        settings: &ManagerSettings,
    ) -> Result<Option<ManagedRuntimeRecord>, String> {
        let tools_dir = settings.tools_dir();
        fs::create_dir_all(&tools_dir).map_err(|error| {
            format!(
                "failed to create tools dir {}: {error}",
                tools_dir.display()
            )
        })?;

        let mut runtimes = Vec::new();
        for entry in fs::read_dir(&tools_dir)
            .map_err(|error| format!("failed to read tools dir {}: {error}", tools_dir.display()))?
        {
            let entry = entry
                .map_err(|error| format!("failed to inspect managed runtime entry: {error}"))?;
            let manifest_path = entry.path().join("runtime.json");
            if manifest_path.exists() {
                let contents = fs::read_to_string(&manifest_path).map_err(|error| {
                    format!(
                        "failed to read managed runtime manifest {}: {error}",
                        manifest_path.display()
                    )
                })?;
                let runtime =
                    serde_json::from_str::<ManagedRuntimeRecord>(&contents).map_err(|error| {
                        format!(
                            "failed to parse managed runtime manifest {}: {error}",
                            manifest_path.display()
                        )
                    })?;
                runtimes.push(runtime);
            }
        }

        runtimes.sort_by(compare_runtime_versions_desc);
        Ok(runtimes.into_iter().next())
    }

    fn fetch_latest_release(&self, settings: &ManagerSettings) -> Result<RemoteRelease, String> {
        let url = latest_release_url(settings);
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|error| format!("failed to reach GitHub releases API: {error}"))?
            .error_for_status()
            .map_err(|error| format!("GitHub releases API returned an error: {error}"))?;

        let release = response
            .json::<GitHubRelease>()
            .map_err(|error| format!("failed to parse GitHub release payload: {error}"))?;
        let version = normalize_version(&release.tag_name);

        let asset = select_release_asset(&release.assets, &platform_asset_token())
            .ok_or("latest JAWATA release did not include a downloadable archive")?;

        let archive_kind = if asset.name.ends_with(".tar.gz") {
            ArchiveKind::TarGz
        } else {
            ArchiveKind::Zip
        };

        Ok(RemoteRelease {
            version,
            asset_name: asset.name.clone(),
            download_url: asset.browser_download_url.clone(),
            published_at: release.published_at,
            archive_kind,
        })
    }

    fn install_release(
        &self,
        release: &RemoteRelease,
        settings: &ManagerSettings,
        trial: Option<EngineTrial>,
    ) -> Result<ManagedRuntimeRecord, InstallError> {
        let tools_dir = settings.tools_dir();
        fs::create_dir_all(&tools_dir).map_err(|error| {
            format!(
                "failed to create tools dir {}: {error}",
                tools_dir.display()
            )
        })?;
        let target_dir = tools_dir.join(format!("jawata-{}", release.version));
        let manifest_path = target_dir.join("runtime.json");

        if manifest_path.exists() {
            let contents = fs::read_to_string(&manifest_path).map_err(|error| {
                format!(
                    "failed to read cached managed runtime manifest {}: {error}",
                    manifest_path.display()
                )
            })?;
            let runtime =
                serde_json::from_str::<ManagedRuntimeRecord>(&contents).map_err(|error| {
                    format!(
                        "failed to parse cached managed runtime manifest {}: {error}",
                        manifest_path.display()
                    )
                })?;
            return Ok(runtime);
        }

        let tmp_dir = tools_dir.join(format!(
            ".tmp-{}-{}",
            release.version,
            current_timestamp_string()
        ));
        let extract_root = tmp_dir.join("contents");
        fs::create_dir_all(&extract_root).map_err(|error| {
            format!(
                "failed to create temporary extraction dir {}: {error}",
                extract_root.display()
            )
        })?;

        // Sprint 28 (v3.6.0): stream the ~107 MB archive STRAIGHT TO DISK on
        // the long-budget client. The old path bought the whole archive into
        // RAM with `.bytes()` and then decoded a second copy out of a
        // `Cursor`, on a client whose 10 s total timeout the download could
        // never meet. Errors report their full source chain, because reqwest's
        // own Display for a timed-out body read is the useless
        // "error decoding response body" with the cause hidden underneath.
        //
        // 28h S2a (D35): the staging file has a name that stays the same from one attempt to the next, so a download
        // that was interrupted is continued where it stopped instead of begun again.
        let archive_path = partial_archive_path(&tools_dir, release);
        engine_activity::downloading(&release.version);
        let copied = match fetch_resumable(&self.download_client, &release.download_url, &archive_path) {
            Ok(total) => total,
            Err(error) => {
                engine_activity::download_ended();
                return Err(error.into());
            }
        };
        if copied == 0 {
            engine_activity::download_ended();
            return Err(format!(
                "JAWATA archive download returned an empty body for {}",
                release.asset_name
            )
            .into());
        }

        if let Err(error) = unpack_archive(&release.archive_kind, &archive_path, &extract_root) {
            // A staged file that will not unpack is no use to a resume: the next attempt starts from nothing.
            let _ = fs::remove_file(&archive_path);
            engine_activity::download_ended();
            return Err(error.into());
        }

        match activate_extracted(&tools_dir, &tmp_dir, &extract_root, release, trial) {
            Ok(runtime) => {
                remove_partial_archives(&tools_dir);
                engine_activity::installed(&release.version);
                Ok(runtime)
            }
            Err(error) => {
                engine_activity::download_ended();
                Err(error)
            }
        }
    }

    fn build_release_status(
        &self,
        release: Option<&RemoteRelease>,
        installed: Option<&ManagedRuntimeRecord>,
        settings: &ManagerSettings,
        detail_override: Option<String>,
        error_detail: Option<String>,
    ) -> ReleaseStatus {
        if let Some(error_detail) = error_detail {
            return ReleaseStatus {
                kind: ReleaseStatusKind::CheckFailed,
                latest_version: settings.last_seen_latest_version.clone(),
                default_version: installed.map(|r| r.version.clone()),
                checked_at: settings.last_release_check.clone(),
                update_available: false,
                detail: error_detail,
            };
        }

        if let Some(release) = release {
            let latest_installed =
                installed.map_or(false, |runtime| runtime.version == release.version);
            let update_available = !latest_installed;
            let kind = if installed.is_none() {
                ReleaseStatusKind::Missing
            } else if update_available {
                ReleaseStatusKind::UpdateAvailable
            } else {
                ReleaseStatusKind::Ready
            };

            return ReleaseStatus {
                kind,
                latest_version: Some(release.version.clone()),
                default_version: installed.map(|r| r.version.clone()),
                checked_at: settings.last_release_check.clone(),
                update_available,
                detail: detail_override.unwrap_or_else(|| {
                    if update_available {
                        format!(
                            "Latest upstream release is {}. Download it to keep the managed runtime current.",
                            release.version
                        )
                    } else {
                        format!("Managed JAWATA runtime {} is up to date.", release.version)
                    }
                }),
            };
        }

        ReleaseStatus {
            kind: if installed.is_none() {
                ReleaseStatusKind::Missing
            } else {
                ReleaseStatusKind::CheckingDisabled
            },
            latest_version: settings.last_seen_latest_version.clone(),
            default_version: installed.map(|r| r.version.clone()),
            checked_at: settings.last_release_check.clone(),
            update_available: false,
            detail: detail_override
                .unwrap_or_else(|| "No release information is available yet.".into()),
        }
    }

    fn build_cached_release_status(
        &self,
        installed: Option<&ManagedRuntimeRecord>,
        settings: &ManagerSettings,
    ) -> ReleaseStatus {
        if !settings.auto_check_for_updates {
            return ReleaseStatus {
                kind: if installed.is_none() {
                    ReleaseStatusKind::Missing
                } else {
                    ReleaseStatusKind::CheckingDisabled
                },
                latest_version: settings.last_seen_latest_version.clone(),
                default_version: installed.map(|r| r.version.clone()),
                checked_at: settings.last_release_check.clone(),
                update_available: false,
                detail: UpdateDecision::CheckingDisabled.log_line(),
            };
        }

        if let Some(latest_version) = settings.last_seen_latest_version.clone() {
            let update_available =
                installed.map_or(true, |runtime| runtime.version != latest_version);
            let kind = if installed.is_none() {
                ReleaseStatusKind::Missing
            } else if update_available {
                ReleaseStatusKind::UpdateAvailable
            } else {
                ReleaseStatusKind::Ready
            };

            let detail = if installed.is_none() {
                format!(
                    "No managed JAWATA runtime is cached yet. Last known upstream release is {}.",
                    latest_version
                )
            } else if update_available {
                format!(
                    "Last known upstream release is {}. Use Refresh release info to check again.",
                    latest_version
                )
            } else {
                format!(
                    "Managed JAWATA runtime {} matches the last known upstream release.",
                    latest_version
                )
            };

            return ReleaseStatus {
                kind,
                latest_version: Some(latest_version),
                default_version: installed.map(|r| r.version.clone()),
                checked_at: settings.last_release_check.clone(),
                update_available,
                detail,
            };
        }

        ReleaseStatus {
            kind: if installed.is_none() {
                ReleaseStatusKind::Missing
            } else {
                ReleaseStatusKind::Ready
            },
            latest_version: None,
            default_version: installed.map(|r| r.version.clone()),
            checked_at: settings.last_release_check.clone(),
            update_available: false,
            detail: "No cached release information is available yet. Use Refresh release info to check upstream.".into(),
        }
    }
}

/// Unpacks the staged archive into `extract_root`.
fn unpack_archive(kind: &ArchiveKind, archive_path: &Path, extract_root: &Path) -> Result<(), String> {
    let open_archive = || {
        fs::File::open(&archive_path).map_err(|error| {
            format!(
                "failed to reopen staged archive {}: {error}",
                archive_path.display()
            )
        })
    };

    match kind {
        ArchiveKind::TarGz => {
            let decoder = GzDecoder::new(open_archive()?);
            let mut archive = Archive::new(decoder);
            archive.unpack(&extract_root).map_err(|error| {
                format!("failed to unpack JAWATA tar.gz archive: {error}")
            })?;
        }
        ArchiveKind::Zip => {
            let mut archive = ZipArchive::new(open_archive()?)
                .map_err(|error| format!("failed to read JAWATA zip archive: {error}"))?;
            for index in 0..archive.len() {
                let mut file = archive.by_index(index).map_err(|error| {
                    format!("failed to inspect JAWATA zip entry: {error}")
                })?;
                let enclosed = file
                    .enclosed_name()
                    .ok_or("zip archive contained an invalid entry path")?;
                let output_path = extract_root.join(enclosed);

                if file.is_dir() {
                    fs::create_dir_all(&output_path).map_err(|error| {
                        format!(
                            "failed to create zip output dir {}: {error}",
                            output_path.display()
                        )
                    })?;
                } else {
                    if let Some(parent) = output_path.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            format!(
                                "failed to create zip parent dir {}: {error}",
                                parent.display()
                            )
                        })?;
                    }
                    let mut output_file = fs::File::create(&output_path).map_err(|error| {
                        format!(
                            "failed to create extracted JAWATA file {}: {error}",
                            output_path.display()
                        )
                    })?;
                    std::io::copy(&mut file, &mut output_file).map_err(|error| {
                        format!(
                            "failed to write extracted JAWATA file {}: {error}",
                            output_path.display()
                        )
                    })?;
                }
            }
        }
    }
    Ok(())
}

/// Everything from "the archive is unpacked" to "the engine is the active one".
///
/// 4.4.14, his word 2026-10-27 ("I want guard 2"): when a working engine is being REPLACED, the unpacked engine is
/// started once and compared with the tool surface this studio expects, and it only becomes the active one if that
/// passes. The trial runs on the unpacked files in the temporary folder, before the rename and the `current`
/// symlink swap, so a refused engine is never reachable by a deploy, a start or a restart: the swap below stays the
/// single commit point.
fn activate_extracted(
    tools_dir: &Path,
    tmp_dir: &Path,
    extract_root: &Path,
    release: &RemoteRelease,
    trial: Option<EngineTrial>,
) -> Result<ManagedRuntimeRecord, InstallError> {
    let target_dir = tools_dir.join(format!("jawata-{}", release.version));
    let manifest_path = target_dir.join("runtime.json");
    let jar_relative_path = find_relative_jar_path(extract_root)?;
    if let Some(trial) = trial {
        if let Err(reason) = trial(&extract_root.join(&jar_relative_path)) {
            let _ = fs::remove_dir_all(tmp_dir);
            return Err(InstallError::Held(reason));
        }
    }

        // Sprint 15 Stage 8 (bug #8): the install sequence is now ordered so
        // that the `current` symlink swap is the atomic commit point. Any
        // reader of the stable jar path (deploy writer, resident-JVM spawner
        // in Stage 10) sees EITHER the old version OR the new one, never a
        // dangling path.
        //
        // 1. Extract to a tmp dir (above).
        // 2. Rename tmp → versioned target dir.
        // 3. Atomically swap `current` → new versioned dir.
        // 4. Clean up older `jawata-*` dirs (single-cached-runtime
        //    invariant preserved from pre-bug-#8 behaviour).
        // 5. Write runtime.json with the stable `current/...` jar path.

        fs::rename(extract_root, &target_dir).map_err(|error| {
            format!(
                "failed to finalize managed runtime dir {}: {error}",
                target_dir.display()
            )
        })?;
        let _ = fs::remove_dir_all(tmp_dir);

        let target_dir_name = target_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("invalid target directory name")?
            .to_string();

        // Atomic commit point — on POSIX this is a rename-into-place of a
        // tmp symlink. On unsupported platforms the install still succeeds
        // but the recorded jar_path falls back to the versioned form (so
        // the install is operational; only the bug-#8 mitigation degrades).
        let stable_jar_path = match update_current_symlink(&tools_dir, &target_dir_name) {
            Ok(()) => tools_dir.join("current").join(&jar_relative_path),
            Err(error) => {
                eprintln!(
                    "jawata-studio: falling back to versioned jar path ({}). \
                     Bug #8 mitigation degraded on this platform.",
                    error
                );
                target_dir.join(&jar_relative_path)
            }
        };

        // Clean up older jawata-* dirs (NOT the current one).
        if let Ok(entries) = fs::read_dir(&tools_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("jawata-") && name != target_dir_name {
                            let _ = fs::remove_dir_all(&path);
                        }
                    }
                }
            }
        }

        let runtime = ManagedRuntimeRecord {
            version: release.version.clone(),
            install_dir: display_path(&target_dir),
            jar_path: display_path(&stable_jar_path),
            asset_name: release.asset_name.clone(),
            installed_at: release
                .published_at
                .clone()
                .unwrap_or_else(current_timestamp_string),
        };

        let manifest = serde_json::to_string_pretty(&runtime)
            .map_err(|error| format!("failed to serialize managed runtime manifest: {error}"))?;
        fs::write(&manifest_path, format!("{manifest}\n")).map_err(|error| {
            format!(
                "failed to write managed runtime manifest {}: {error}",
                manifest_path.display()
            )
        })?;

        Ok(runtime)
}

fn normalize_version(tag: &str) -> String {
    tag.trim_start_matches('v').to_string()
}

fn compare_runtime_versions_desc(
    left: &ManagedRuntimeRecord,
    right: &ManagedRuntimeRecord,
) -> std::cmp::Ordering {
    compare_version_strings(&right.version, &left.version)
}

/// Compares two version strings, preferring semantic version parsing if possible.
pub fn compare_version_strings(left: &str, right: &str) -> std::cmp::Ordering {
    match (Version::parse(left), Version::parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

/// Atomically updates the `current` symlink in `tools_dir` to point at
/// `target_dir_name` (a sibling directory name like `jawata-1.8.5`).
///
/// On POSIX: creates a relative symlink at `tools_dir/.current.tmp-<ts>`
/// then `rename(2)`s it over the existing `tools_dir/current` — atomic
/// per POSIX semantics, so any concurrent reader sees either the old
/// target or the new one, never a half-state.
///
/// On non-POSIX (Windows): returns Err. The caller (`install_release`)
/// catches this and falls back to recording the versioned jar path in
/// `runtime.json`, so the install still succeeds but bug #8's
/// auto-download-doesn't-break-deployed-configs mitigation degrades. Full
/// Windows support lands with the Sprint 16 Windows installer track.
fn update_current_symlink(tools_dir: &Path, target_dir_name: &str) -> Result<(), String> {
    let current = tools_dir.join("current");
    let tmp = tools_dir.join(format!(".current.tmp-{}", current_timestamp_string()));

    // Defensive: a previous interrupted run could have left a tmp file.
    let _ = fs::remove_file(&tmp);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // Relative target so the symlink keeps resolving if the parent dir
        // ever moves (e.g. user changes data_root and copies the tree).
        symlink(target_dir_name, &tmp).map_err(|error| {
            format!(
                "failed to create current symlink at {}: {error}",
                tmp.display()
            )
        })?;
        fs::rename(&tmp, &current).map_err(|error| {
            let _ = fs::remove_file(&tmp);
            format!(
                "failed to swap current symlink at {}: {error}",
                current.display()
            )
        })?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = target_dir_name;
        let _ = current;
        Err(format!(
            "current symlink not yet supported on this platform \
             (Sprint 16 Windows installer follow-up)"
        ))
    }
}

fn find_relative_jar_path(root: &Path) -> Result<PathBuf, String> {
    for entry in WalkDir::new(root) {
        let entry =
            entry.map_err(|error| format!("failed to walk extracted JAWATA archive: {error}"))?;
        if entry.file_type().is_file() && entry.file_name() == "jawata.jar" {
            return entry
                .path()
                .strip_prefix(root)
                .map(PathBuf::from)
                .map_err(|error| {
                    format!("failed to compute extracted JAWATA jar path: {error}")
                });
        }
    }

    Err("downloaded JAWATA archive did not contain jawata.jar".into())
}

#[cfg(test)]
mod tests {

    // ---- D4: every version check says what it DECIDED, and why ----

    /// The four outcomes the requirement names, plus the two the old code could
    /// not tell apart. Before this the check composed a sentence beside the
    /// branch it took, and the commonest outcome said the least: a runtime that
    /// was NOT updated logged "Latest upstream release is X", which is true
    /// whether the policy declined it, it was already current, or nothing had
    /// been asked at all.
    #[test]
    fn a_check_names_its_outcome_and_the_reason() {
        use crate::config::UpdatePolicy;

        // ALREADY CURRENT and POLICY DECLINED are DIFFERENT, and telling them
        // apart is the whole point: one is nothing to do, the other is an
        // update sitting there waiting for a click.
        let current = decide_update(UpdatePolicy::Always, Some("4.1.3"), "4.1.3");
        assert_eq!(UpdateDecision::AlreadyCurrent { version: "4.1.3".into() }, current);
        assert!(current.log_line().contains("nothing to do"));
        assert!(!current.installed_something());

        let declined = decide_update(UpdatePolicy::Ask, Some("4.1.3"), "4.2.0");
        assert_eq!(
            UpdateDecision::PolicyDeclined {
                installed: "4.1.3".into(),
                latest: "4.2.0".into()
            },
            declined
        );
        assert!(declined.log_line().contains("declined 4.2.0"), "{}", declined.log_line());
        assert!(declined.log_line().contains("ask"), "the REASON, not just the fact");
        assert!(!declined.installed_something());

        // The same facts under the policy this sprint migrates everyone to.
        let installed = decide_update(UpdatePolicy::Always, Some("4.1.3"), "4.2.0");
        assert_eq!(UpdateDecision::Installed { version: "4.2.0".into() }, installed);
        assert!(installed.installed_something());

        // A FIRST runtime is fetched whatever the policy says: `ask` is a
        // policy about UPDATING, and there is nothing to update.
        let first = decide_update(UpdatePolicy::Ask, None, "4.2.0");
        assert_eq!(UpdateDecision::InstalledFirstRuntime { version: "4.2.0".into() }, first);
        assert!(first.installed_something());

        // Checking switched off is NOT decided here — the check returns before
        // the network is touched — so it is asserted as the sentence it leaves
        // rather than as an arm of a function that could never reach it.
        assert!(UpdateDecision::CheckingDisabled.log_line().contains("switched off"));

        // The two failures a decision must not be silent about. A check that
        // FAILED is not "no update" — it is no verdict at all.
        let failed = UpdateDecision::CheckFailed { error: "timed out".into() };
        assert!(failed.log_line().contains("no verdict"), "{}", failed.log_line());
        assert!(failed.log_line().contains("timed out"), "the reason travels with it");
        let broken = UpdateDecision::DownloadFailed {
            version: "4.2.0".into(),
            error: "connection reset".into(),
        };
        assert!(broken.log_line().contains("could not install 4.2.0"));
        assert!(broken.log_line().contains("connection reset"));
        assert!(!broken.installed_something(), "a failed download installed nothing");
    }
    use super::*;
    use crate::config::AppPaths;
    use std::path::PathBuf;

    fn gh_asset(name: &str) -> GitHubAsset {
        GitHubAsset {
            name: name.into(),
            browser_download_url: format!("https://example.test/{name}"),
        }
    }

    #[test]
    fn select_release_asset_prefers_the_platform_archive() {
        // Sprint 21a dogfood find: the first-.tar.gz pick installed linux-arm64 on an
        // x86_64 machine (GitHub lists assets alphabetically) → wrong SWT, resident dead.
        let assets = vec![
            gh_asset("jawata-v2.1.0-linux-arm64.tar.gz"),
            gh_asset("jawata-v2.1.0-linux-x64.tar.gz"),
            gh_asset("jawata-v2.1.0-macos-arm64.zip"),
            gh_asset("jawata-v2.1.0-macos-x64.zip"),
            gh_asset("jawata-v2.1.0-win32-x64.zip"),
            gh_asset("checksums.txt"),
        ];
        for (token, expected) in [
            ("linux-x64", "jawata-v2.1.0-linux-x64.tar.gz"),
            ("linux-arm64", "jawata-v2.1.0-linux-arm64.tar.gz"),
            ("macos-arm64", "jawata-v2.1.0-macos-arm64.zip"),
            ("macos-x64", "jawata-v2.1.0-macos-x64.zip"),
            ("win32-x64", "jawata-v2.1.0-win32-x64.zip"),
        ] {
            assert_eq!(
                select_release_asset(&assets, token).unwrap().name,
                expected,
                "platform token {token}"
            );
        }
        // Legacy single-archive releases (no platform suffix) keep working.
        let legacy = vec![gh_asset("jawata-v1.7.0.tar.gz")];
        assert_eq!(
            select_release_asset(&legacy, "linux-x64").unwrap().name,
            "jawata-v1.7.0.tar.gz"
        );
        // The token this build computes matches the release naming scheme.
        let token = platform_asset_token();
        assert!(
            ["linux-x64", "linux-arm64", "macos-x64", "macos-arm64", "win32-x64", "win32-arm64"]
                .contains(&token.as_str()),
            "unexpected token {token}"
        );
    }

    #[test]
    fn compare_version_strings_prefers_newer_semver_tags() {
        assert!(compare_version_strings("1.2.0", "1.1.5").is_gt());
        assert!(compare_version_strings("1.1.5", "1.2.0").is_lt());
        assert!(compare_version_strings("1.2.0", "1.2.0").is_eq());
    }

    #[test]
    fn release_status_marks_update_when_latest_not_installed() {
        let paths = AppPaths {
            config_dir: PathBuf::from("/tmp/config"),
            state_dir: PathBuf::from("/tmp/state"),
            cache_dir: PathBuf::from("/tmp/cache"),
            projects_file: PathBuf::from("/tmp/config/projects.json"),
            settings_file: PathBuf::from("/tmp/config/settings.json"),
            runtime_state_file: PathBuf::from("/tmp/state/runtime-state.json"),
            default_data_root: PathBuf::from("/tmp/cache/jawata-studio"),
            log_dir: PathBuf::from("/tmp/state/logs"),
        };
        let manager = ReleaseManager::new().expect("failed to build release manager");
        let settings = ManagerSettings::default_for_paths(&paths);
        let installed = Some(ManagedRuntimeRecord {
            version: "1.1.5".into(),
            install_dir: "/tmp/cache/tools/jawata/jawata-1.1.5".into(),
            jar_path: "/tmp/cache/tools/jawata/jawata-1.1.5/jawata.jar".into(),
            asset_name: "jawata-v1.1.5.tar.gz".into(),
            installed_at: "123".into(),
        });
        let release = RemoteRelease {
            version: "1.2.0".into(),
            asset_name: "jawata-v1.2.0.tar.gz".into(),
            download_url: "https://example.com".into(),
            published_at: Some("124".into()),
            archive_kind: ArchiveKind::TarGz,
        };

        let status =
            manager.build_release_status(Some(&release), installed.as_ref(), &settings, None, None);
        assert!(matches!(status.kind, ReleaseStatusKind::UpdateAvailable));
        assert!(status.update_available);
        assert_eq!(status.latest_version.as_deref(), Some("1.2.0"));
    }

    // ===== Sprint 15 Stage 8 (bug #8) =====

    fn unique_test_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-stage8-{}-{}-{}",
            label,
            nanos,
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create test tools dir");
        dir
    }

    // ---- 4.4.14: a replacing engine is tried before it is switched to ----

    fn fake_release(version: &str) -> RemoteRelease {
        RemoteRelease {
            version: version.to_string(),
            asset_name: "jawata.tar.gz".into(),
            download_url: "http://127.0.0.1:1/jawata.tar.gz".into(),
            published_at: None,
            archive_kind: ArchiveKind::TarGz,
        }
    }

    /// What `install_release` has on disk when the archive is unpacked and nothing is active yet.
    fn unpacked_engine(tools_dir: &Path, version: &str) -> (PathBuf, PathBuf) {
        let tmp = tools_dir.join(format!(".tmp-{version}-x"));
        let extract = tmp.join("contents");
        fs::create_dir_all(extract.join("inner")).expect("create the unpacked folder");
        fs::write(extract.join("inner/jawata.jar"), b"new-engine").expect("write the unpacked jar");
        (tmp, extract)
    }

    #[test]
    fn an_engine_that_fails_its_trial_is_never_reachable_and_the_old_one_stays() {
        let tools = unique_test_dir("trial-refused");
        fs::create_dir_all(tools.join("jawata-1.0.0/inner")).unwrap();
        fs::write(tools.join("jawata-1.0.0/inner/jawata.jar"), b"old-engine").unwrap();
        fs::write(tools.join("jawata-1.0.0/runtime.json"), "{}").unwrap();
        #[cfg(unix)]
        update_current_symlink(&tools, "jawata-1.0.0").expect("old engine is current");
        let (tmp, extract) = unpacked_engine(&tools, "9.9.9");
        let tried = std::cell::RefCell::new(None::<PathBuf>);
        let trial = |jar: &Path| -> Result<(), String> {
            *tried.borrow_mut() = Some(jar.to_path_buf());
            Err("the engine no longer offers: extract".into())
        };

        let outcome = activate_extracted(&tools, &tmp, &extract, &fake_release("9.9.9"), Some(&trial));

        match outcome {
            Err(InstallError::Held(reason)) => assert_eq!("the engine no longer offers: extract", reason),
            other => panic!("expected the engine to be held, got {other:?}"),
        }
        let jar = tried.borrow().clone().expect("the trial was run on the unpacked jar");
        assert!(jar.starts_with(&extract), "tried in the temporary folder, not an installed one: {jar:?}");
        assert!(!tools.join("jawata-9.9.9").exists(), "the refused engine has no versioned folder");
        assert!(!tmp.exists(), "and its temporary folder is removed");
        #[cfg(unix)]
        assert_eq!(
            PathBuf::from("jawata-1.0.0"),
            fs::read_link(tools.join("current")).expect("current is still a link"),
            "current still points at the old engine"
        );
        assert_eq!(
            "jawata-1.0.0",
            get_installed_version_dir(&tools),
            "the installed engine is still the old one"
        );
        fs::remove_dir_all(&tools).ok();
    }

    /// The version folder the discovery would pick, by the same `runtime.json` rule `get_installed_runtime` uses.
    fn get_installed_version_dir(tools_dir: &Path) -> String {
        let mut found: Vec<String> = fs::read_dir(tools_dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().join("runtime.json").exists())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        found.sort();
        found.pop().unwrap_or_default()
    }

    #[test]
    fn an_engine_that_passes_its_trial_becomes_the_active_one() {
        let tools = unique_test_dir("trial-passed");
        let (tmp, extract) = unpacked_engine(&tools, "9.9.9");
        let trial = |jar: &Path| -> Result<(), String> {
            assert_eq!("jawata.jar", jar.file_name().unwrap().to_string_lossy(), "{jar:?}");
            Ok(())
        };

        let runtime = activate_extracted(&tools, &tmp, &extract, &fake_release("9.9.9"), Some(&trial))
            .expect("a passing trial activates the engine");

        assert_eq!("9.9.9", runtime.version);
        assert!(tools.join("jawata-9.9.9/runtime.json").exists());
        assert!(tools.join("jawata-9.9.9/inner/jawata.jar").exists());
        fs::remove_dir_all(&tools).ok();
    }

    #[test]
    fn the_first_engine_is_installed_without_a_trial_because_nothing_is_there_to_keep() {
        let tools = unique_test_dir("trial-first");
        let (tmp, extract) = unpacked_engine(&tools, "9.9.9");

        let runtime = activate_extracted(&tools, &tmp, &extract, &fake_release("9.9.9"), None)
            .expect("no trial, no refusal");

        assert_eq!("9.9.9", runtime.version);
        fs::remove_dir_all(&tools).ok();
    }

    #[test]
    fn a_held_engine_applies_to_that_engine_version_and_that_studio_version_only() {
        let held = HeldUpdate { version: "4.5.0".into(), studio_version: "4.4.14".into(), reason: "x".into() };
        assert!(held.applies_to("4.5.0", "4.4.14"));
        assert!(!held.applies_to("4.5.1", "4.4.14"), "a newer engine gets its own trial");
        assert!(!held.applies_to("4.5.0", "4.4.15"), "a newer studio gets a new trial of the same engine");
    }

    #[test]
    fn what_was_held_survives_a_restart_and_a_good_install_clears_it() {
        let tools = unique_test_dir("trial-held-file");
        assert_eq!(None, read_held(&tools));
        let held = HeldUpdate { version: "4.5.0".into(), studio_version: "4.4.14".into(), reason: "new: wipe".into() };

        write_held(&tools, &held);
        assert_eq!(Some(held), read_held(&tools));

        clear_held(&tools);
        assert_eq!(None, read_held(&tools));
        fs::remove_dir_all(&tools).ok();
    }

    #[test]
    fn a_held_decision_says_the_old_engine_stays_and_why_and_is_not_an_install() {
        let held = UpdateDecision::Held { version: "4.5.0".into(), reason: "new: wipe".into() };
        let again = UpdateDecision::HeldAgain { version: "4.5.0".into(), reason: "new: wipe".into() };
        for decision in [&held, &again] {
            let line = decision.log_line();
            assert!(line.contains("kept the installed engine") && line.contains("4.5.0") && line.contains("new: wipe"), "{line}");
            assert!(decision.is_held());
            assert!(!decision.installed_something());
        }
        assert!(held.log_line().contains("downloaded"), "the first time it was downloaded");
        assert!(again.log_line().contains("nothing was downloaded"), "the second time it was not");
    }

    #[cfg(unix)]
    #[test]
    fn current_symlink_points_at_target() {
        let tools_dir = unique_test_dir("symlink-points");
        let target = "jawata-1.8.0";
        let target_dir = tools_dir.join(target);
        fs::create_dir_all(target_dir.join("inner"))
            .expect("create target dir");
        fs::write(target_dir.join("inner/jawata.jar"), b"fake-jar")
            .expect("write jar fixture");

        update_current_symlink(&tools_dir, target).expect("symlink");

        let current = tools_dir.join("current");
        assert!(current.exists(), "current symlink must exist");
        // Reading through the symlink should hit the jar contents.
        let bytes = fs::read(current.join("inner/jawata.jar"))
            .expect("read jar through current symlink");
        assert_eq!(&bytes[..], b"fake-jar");

        fs::remove_dir_all(&tools_dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn current_symlink_points_at_latest_extracted() {
        // Two-version sequence: install v1, then v2 — `current` must end
        // up resolving to v2. This mirrors the real upgrade flow.
        let tools_dir = unique_test_dir("symlink-latest");

        let v1_name = "jawata-1.8.0";
        let v1_dir = tools_dir.join(v1_name);
        fs::create_dir_all(v1_dir.join("inner")).expect("create v1 dir");
        fs::write(v1_dir.join("inner/jawata.jar"), b"v1-jar").expect("v1 jar");
        update_current_symlink(&tools_dir, v1_name).expect("symlink v1");

        let v2_name = "jawata-1.8.5";
        let v2_dir = tools_dir.join(v2_name);
        fs::create_dir_all(v2_dir.join("inner")).expect("create v2 dir");
        fs::write(v2_dir.join("inner/jawata.jar"), b"v2-jar").expect("v2 jar");
        update_current_symlink(&tools_dir, v2_name).expect("symlink v2 (re-point)");

        let bytes = fs::read(tools_dir.join("current/inner/jawata.jar"))
            .expect("read jar through current after re-point");
        assert_eq!(&bytes[..], b"v2-jar",
            "current must resolve to v2 jar after re-point");
        // v1 dir still present at this layer; real install_release would
        // delete it after the symlink swap.
        assert!(v1_dir.exists(),
            "stage helper does not delete old versioned dirs — install_release does");

        fs::remove_dir_all(&tools_dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn current_symlink_overwrites_existing_symlink_atomically() {
        let tools_dir = unique_test_dir("symlink-swap");
        fs::create_dir_all(tools_dir.join("jawata-a")).unwrap();
        fs::create_dir_all(tools_dir.join("jawata-b")).unwrap();

        update_current_symlink(&tools_dir, "jawata-a").expect("first symlink");
        // Re-point: the swap must succeed even though `current` already
        // exists (POSIX rename overwrites).
        update_current_symlink(&tools_dir, "jawata-b").expect("re-point");

        let resolved = fs::read_link(tools_dir.join("current"))
            .expect("read symlink target");
        assert_eq!(resolved, std::path::Path::new("jawata-b"));

        // No leftover .current.tmp-* litter.
        let leftovers: Vec<_> = fs::read_dir(&tools_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".current.tmp-")
            })
            .collect();
        assert!(leftovers.is_empty(),
            "no tmp-symlink leftovers after successful swap");

        fs::remove_dir_all(&tools_dir).ok();
    }

    #[test]
    fn latest_release_url_uses_setting_when_env_unset() {
        let paths = AppPaths {
            config_dir: PathBuf::from("/tmp/config"),
            state_dir: PathBuf::from("/tmp/state"),
            cache_dir: PathBuf::from("/tmp/cache"),
            projects_file: PathBuf::from("/tmp/config/projects.json"),
            settings_file: PathBuf::from("/tmp/config/settings.json"),
            runtime_state_file: PathBuf::from("/tmp/state/runtime-state.json"),
            default_data_root: PathBuf::from("/tmp/cache/jawata-studio"),
            log_dir: PathBuf::from("/tmp/state/logs"),
        };
        let mut settings = ManagerSettings::default_for_paths(&paths);
        // Ensure env var is not influencing this test.
        std::env::remove_var("JAWATA_RELEASE_REPO");

        // Default: fork at the post-rename URL (haraldwegner).
        assert_eq!(
            latest_release_url(&settings),
            "https://api.github.com/repos/haraldwegner/jawata-mcp/releases/latest"
        );

        // Custom override (e.g. legacy upstream or another fork).
        settings.release_repo = "example-org/custom-mcp".into();
        assert_eq!(
            latest_release_url(&settings),
            "https://api.github.com/repos/example-org/custom-mcp/releases/latest"
        );
    }

    // ---- 28h S2a (D35): an interrupted download resumes -------------------------------------------------------------

    /// A one-purpose HTTP server on localhost: it serves `body`, notes the `Range` header of every request, and can be told
    /// to break the first response off after `cut_first_at` bytes (a connection that drops mid-body) or to ignore ranges.
    struct ArchiveServer {
        url: String,
        ranges: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
        sent: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ArchiveServer {
        fn start(body: Vec<u8>, honour_range: bool, cut_first_at: Option<usize>, requests: usize) -> Self {
            use std::io::{Read, Write};
            use std::sync::atomic::Ordering;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let url = format!("http://{}/jawata.zip", listener.local_addr().expect("addr"));
            let ranges = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (seen, counted) = (ranges.clone(), sent.clone());
            let thread = std::thread::spawn(move || {
                for served in 0..requests {
                    let Ok((mut stream, _)) = listener.accept() else { return };
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).unwrap_or(0) == 0 {
                            break;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    let range = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(|r| r.trim().to_string()));
                    seen.lock().unwrap().push(range.clone());
                    let from = range.as_ref().and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
                    let (status, payload, extra) = match from {
                        Some(n) if honour_range && n >= body.len() => ("416 Range Not Satisfiable", Vec::new(), String::new()),
                        Some(n) if honour_range => (
                            "206 Partial Content",
                            body[n..].to_vec(),
                            format!("Content-Range: bytes {n}-{}/{}\r\n", body.len() - 1, body.len()),
                        ),
                        _ => ("200 OK", body.clone(), String::new()),
                    };
                    let header = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", payload.len());
                    let _ = stream.write_all(header.as_bytes());
                    let written = if served == 0 { cut_first_at.map_or(payload.len(), |c| c.min(payload.len())) } else { payload.len() };
                    let _ = stream.write_all(&payload[..written]);
                    counted.fetch_add(written, Ordering::SeqCst);
                    let _ = stream.flush();
                }
            });
            Self { url, ranges, sent, thread: Some(thread) }
        }

        fn finish(mut self) -> (Vec<Option<String>>, usize) {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            (self.ranges.lock().unwrap().clone(), self.sent.load(std::sync::atomic::Ordering::SeqCst))
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("studio-resume-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive_bytes() -> Vec<u8> {
        (0..4000u32).map(|i| (i % 251) as u8).collect()
    }

    fn client() -> Client {
        Client::builder().timeout(Duration::from_secs(10)).build().unwrap()
    }

    #[test]
    fn a_download_that_broke_off_is_continued_from_where_it_stopped_not_begun_again() {
        let body = archive_bytes();
        let server = ArchiveServer::start(body.clone(), true, Some(1500), 2);
        let dir = scratch("resume");
        let path = dir.join(".partial-4.4.20-jawata.zip");

        let first = fetch_resumable(&client(), &server.url, &path);
        assert!(first.is_err(), "the first attempt was cut off and must say so: {first:?}");
        let kept = fs::metadata(&path).expect("what arrived is kept").len();
        assert!(kept > 0 && kept < body.len() as u64, "the broken download left {kept} bytes");

        let total = fetch_resumable(&client(), &server.url, &path).expect("the second attempt finishes");
        let (ranges, sent) = server.finish();

        assert_eq!(total, body.len() as u64);
        assert_eq!(fs::read(&path).unwrap(), body, "the continued file is not the archive");
        assert_eq!(ranges, vec![None, Some(format!("{kept}-"))], "the second request did not ask for the rest");
        assert_eq!(sent, 1500 + (body.len() - kept as usize), "bytes already on disk were sent again");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_server_that_ignores_the_range_restarts_the_file_and_does_not_append_a_second_copy() {
        let body = archive_bytes();
        let server = ArchiveServer::start(body.clone(), false, None, 1);
        let dir = scratch("ignores");
        let path = dir.join(".partial-4.4.20-jawata.zip");
        fs::write(&path, &body[..700]).unwrap();

        let total = fetch_resumable(&client(), &server.url, &path).expect("the whole archive arrives");
        let (ranges, _) = server.finish();

        assert_eq!(ranges, vec![Some("700-".to_string())], "CONTROL: a range was asked for");
        assert_eq!(total, body.len() as u64);
        assert_eq!(fs::read(&path).unwrap(), body, "a 200 answer was appended to the old bytes");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_range_the_server_cannot_satisfy_starts_the_download_over() {
        let body = archive_bytes();
        let server = ArchiveServer::start(body.clone(), true, None, 2);
        let dir = scratch("unsatisfiable");
        let path = dir.join(".partial-4.4.20-jawata.zip");
        fs::write(&path, vec![7u8; body.len() + 10]).unwrap();

        let total = fetch_resumable(&client(), &server.url, &path).expect("the archive arrives whole");
        let (ranges, _) = server.finish();

        assert_eq!(ranges.len(), 2, "one request for the range, one for the whole file");
        assert_eq!(ranges[1], None);
        assert_eq!(total, body.len() as u64);
        assert_eq!(fs::read(&path).unwrap(), body);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_first_download_asks_for_no_range() {
        let body = archive_bytes();
        let server = ArchiveServer::start(body.clone(), true, None, 1);
        let dir = scratch("fresh");
        let path = dir.join(".partial-4.4.20-jawata.zip");

        fetch_resumable(&client(), &server.url, &path).expect("downloads");
        let (ranges, _) = server.finish();

        assert_eq!(ranges, vec![None]);
        assert_eq!(fs::read(&path).unwrap(), body);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_waiting_archive_has_one_name_per_release_and_installing_clears_it() {
        let release = RemoteRelease {
            version: "4.4.20".into(),
            asset_name: "jawata.zip".into(),
            download_url: "https://example.test/jawata.zip".into(),
            published_at: None,
            archive_kind: ArchiveKind::Zip,
        };
        let dir = scratch("names");
        assert_eq!(partial_archive_path(&dir, &release), partial_archive_path(&dir, &release), "the name changes between attempts");
        fs::write(partial_archive_path(&dir, &release), b"x").unwrap();
        fs::write(dir.join(".partial-4.4.19-jawata.zip"), b"x").unwrap();
        fs::write(dir.join("jawata-4.4.19"), b"keep").unwrap();

        remove_partial_archives(&dir);

        assert!(!partial_archive_path(&dir, &release).exists());
        assert!(!dir.join(".partial-4.4.19-jawata.zip").exists());
        assert!(dir.join("jawata-4.4.19").exists(), "CONTROL: something that is not a waiting archive was removed");
        let _ = fs::remove_dir_all(&dir);
    }
}
