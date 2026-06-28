use crate::config::{
    current_timestamp_string, display_path, effective_release_repo, ManagerSettings, UpdatePolicy,
};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
use tar::Archive;
use walkdir::WalkDir;
use zip::ZipArchive;

/// Compose the GitHub releases-API URL for the configured release repo.
/// Source of truth: GOJA_RELEASE_REPO env var, then settings.release_repo.
fn latest_release_url(settings: &ManagerSettings) -> String {
    format!(
        "https://api.github.com/repos/{}/releases/latest",
        effective_release_repo(settings)
    )
}

/// Represents a cached, managed GOJA runtime installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRuntimeRecord {
    pub version: String,
    pub install_dir: String,
    pub jar_path: String,
    pub asset_name: String,
    pub installed_at: String,
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
}

/// Detailed status information about the managed GOJA release.
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

/// Manages downloading, caching, and updating the GOJA runtime.
pub struct ReleaseManager {
    client: Client,
}

impl ReleaseManager {
    /// Creates a new release manager with a configured HTTP client.
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .user_agent("goja-studio/0.1.0")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("failed to create release manager HTTP client: {error}"))?;

        Ok(Self { client })
    }

    /// Checks for updates and installs the latest release if permitted by settings.
    pub fn sync_with_settings(
        &self,
        settings: &mut ManagerSettings,
    ) -> Result<(Option<ManagedRuntimeRecord>, ReleaseStatus), String> {
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
                detail: "Automatic GOJA release checks are disabled.".into(),
            };
            return Ok((installed, status));
        }

        match self.fetch_latest_release(settings) {
            Ok(release) => {
                let checked_at = current_timestamp_string();
                settings.last_release_check = Some(checked_at.clone());
                settings.last_seen_latest_version = Some(release.version.clone());

                let latest_installed = installed
                    .as_ref()
                    .map_or(false, |runtime| runtime.version == release.version);
                let should_download = installed.is_none()
                    || (settings.update_policy == UpdatePolicy::Always && !latest_installed);

                let mut detail = if installed.is_none() {
                    "No managed GOJA runtime is cached yet.".to_string()
                } else {
                    format!("Latest upstream release is {}.", release.version)
                };

                if should_download {
                    let runtime = self.install_release(&release, settings)?;
                    installed = self.get_installed_runtime(settings)?;
                    detail = format!(
                        "Downloaded GOJA {} into the managed tools cache.",
                        runtime.version
                    );
                }

                let status = self.build_release_status(
                    Some(&release),
                    installed.as_ref(),
                    settings,
                    Some(detail),
                    None,
                );

                Ok((installed, status))
            }
            Err(error) => {
                let status = self.build_release_status(
                    None,
                    installed.as_ref(),
                    settings,
                    None,
                    Some(format!(
                        "Could not check the latest GOJA release: {error}"
                    )),
                );
                Ok((installed, status))
            }
        }
    }

    /// Forces a download and installation of the latest upstream GOJA release.
    pub fn download_latest_runtime(
        &self,
        settings: &mut ManagerSettings,
    ) -> Result<ManagedRuntimeRecord, String> {
        let release = self.fetch_latest_release(settings)?;
        let runtime = self.install_release(&release, settings)?;
        settings.last_release_check = Some(current_timestamp_string());
        settings.last_seen_latest_version = Some(release.version.clone());
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

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name.ends_with(".tar.gz"))
            .or_else(|| {
                release
                    .assets
                    .iter()
                    .find(|asset| asset.name.ends_with(".zip"))
            })
            .ok_or("latest GOJA release did not include a downloadable archive")?;

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
    ) -> Result<ManagedRuntimeRecord, String> {
        let tools_dir = settings.tools_dir();
        fs::create_dir_all(&tools_dir).map_err(|error| {
            format!(
                "failed to create tools dir {}: {error}",
                tools_dir.display()
            )
        })?;
        let target_dir = tools_dir.join(format!("goja-{}", release.version));
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

        let bytes = self
            .client
            .get(&release.download_url)
            .send()
            .map_err(|error| format!("failed to download GOJA release archive: {error}"))?
            .error_for_status()
            .map_err(|error| format!("GOJA archive download failed: {error}"))?
            .bytes()
            .map_err(|error| format!("failed to read GOJA archive bytes: {error}"))?;

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

        match release.archive_kind {
            ArchiveKind::TarGz => {
                let decoder = GzDecoder::new(Cursor::new(bytes));
                let mut archive = Archive::new(decoder);
                archive.unpack(&extract_root).map_err(|error| {
                    format!("failed to unpack GOJA tar.gz archive: {error}")
                })?;
            }
            ArchiveKind::Zip => {
                let mut archive = ZipArchive::new(Cursor::new(bytes))
                    .map_err(|error| format!("failed to read GOJA zip archive: {error}"))?;
                for index in 0..archive.len() {
                    let mut file = archive.by_index(index).map_err(|error| {
                        format!("failed to inspect GOJA zip entry: {error}")
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
                                "failed to create extracted GOJA file {}: {error}",
                                output_path.display()
                            )
                        })?;
                        std::io::copy(&mut file, &mut output_file).map_err(|error| {
                            format!(
                                "failed to write extracted GOJA file {}: {error}",
                                output_path.display()
                            )
                        })?;
                    }
                }
            }
        }

        let jar_relative_path = find_relative_jar_path(&extract_root)?;

        // Sprint 15 Stage 8 (bug #8): the install sequence is now ordered so
        // that the `current` symlink swap is the atomic commit point. Any
        // reader of the stable jar path (deploy writer, resident-JVM spawner
        // in Stage 10) sees EITHER the old version OR the new one, never a
        // dangling path.
        //
        // 1. Extract to a tmp dir (above).
        // 2. Rename tmp → versioned target dir.
        // 3. Atomically swap `current` → new versioned dir.
        // 4. Clean up older `goja-*` dirs (single-cached-runtime
        //    invariant preserved from pre-bug-#8 behaviour).
        // 5. Write runtime.json with the stable `current/...` jar path.

        fs::rename(&extract_root, &target_dir).map_err(|error| {
            format!(
                "failed to finalize managed runtime dir {}: {error}",
                target_dir.display()
            )
        })?;
        let _ = fs::remove_dir_all(&tmp_dir);

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
                    "goja-studio: falling back to versioned jar path ({}). \
                     Bug #8 mitigation degraded on this platform.",
                    error
                );
                target_dir.join(&jar_relative_path)
            }
        };

        // Clean up older goja-* dirs (NOT the current one).
        if let Ok(entries) = fs::read_dir(&tools_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("goja-") && name != target_dir_name {
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
                        format!("Managed GOJA runtime {} is up to date.", release.version)
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
                detail: "Automatic GOJA release checks are disabled.".into(),
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
                    "No managed GOJA runtime is cached yet. Last known upstream release is {}.",
                    latest_version
                )
            } else if update_available {
                format!(
                    "Last known upstream release is {}. Use Refresh release info to check again.",
                    latest_version
                )
            } else {
                format!(
                    "Managed GOJA runtime {} matches the last known upstream release.",
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
/// `target_dir_name` (a sibling directory name like `goja-1.8.5`).
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
            entry.map_err(|error| format!("failed to walk extracted GOJA archive: {error}"))?;
        if entry.file_type().is_file() && entry.file_name() == "goja.jar" {
            return entry
                .path()
                .strip_prefix(root)
                .map(PathBuf::from)
                .map_err(|error| {
                    format!("failed to compute extracted GOJA jar path: {error}")
                });
        }
    }

    Err("downloaded GOJA archive did not contain goja.jar".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppPaths;
    use std::path::PathBuf;

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
            default_data_root: PathBuf::from("/tmp/cache/goja-studio"),
            log_dir: PathBuf::from("/tmp/state/logs"),
        };
        let manager = ReleaseManager::new().expect("failed to build release manager");
        let settings = ManagerSettings::default_for_paths(&paths);
        let installed = Some(ManagedRuntimeRecord {
            version: "1.1.5".into(),
            install_dir: "/tmp/cache/tools/goja/goja-1.1.5".into(),
            jar_path: "/tmp/cache/tools/goja/goja-1.1.5/goja.jar".into(),
            asset_name: "goja-v1.1.5.tar.gz".into(),
            installed_at: "123".into(),
        });
        let release = RemoteRelease {
            version: "1.2.0".into(),
            asset_name: "goja-v1.2.0.tar.gz".into(),
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

    #[cfg(unix)]
    fn unique_test_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "goja-stage8-{}-{}-{}",
            label,
            nanos,
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create test tools dir");
        dir
    }

    #[cfg(unix)]
    #[test]
    fn current_symlink_points_at_target() {
        let tools_dir = unique_test_dir("symlink-points");
        let target = "goja-1.8.0";
        let target_dir = tools_dir.join(target);
        fs::create_dir_all(target_dir.join("inner"))
            .expect("create target dir");
        fs::write(target_dir.join("inner/goja.jar"), b"fake-jar")
            .expect("write jar fixture");

        update_current_symlink(&tools_dir, target).expect("symlink");

        let current = tools_dir.join("current");
        assert!(current.exists(), "current symlink must exist");
        // Reading through the symlink should hit the jar contents.
        let bytes = fs::read(current.join("inner/goja.jar"))
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

        let v1_name = "goja-1.8.0";
        let v1_dir = tools_dir.join(v1_name);
        fs::create_dir_all(v1_dir.join("inner")).expect("create v1 dir");
        fs::write(v1_dir.join("inner/goja.jar"), b"v1-jar").expect("v1 jar");
        update_current_symlink(&tools_dir, v1_name).expect("symlink v1");

        let v2_name = "goja-1.8.5";
        let v2_dir = tools_dir.join(v2_name);
        fs::create_dir_all(v2_dir.join("inner")).expect("create v2 dir");
        fs::write(v2_dir.join("inner/goja.jar"), b"v2-jar").expect("v2 jar");
        update_current_symlink(&tools_dir, v2_name).expect("symlink v2 (re-point)");

        let bytes = fs::read(tools_dir.join("current/inner/goja.jar"))
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
        fs::create_dir_all(tools_dir.join("goja-a")).unwrap();
        fs::create_dir_all(tools_dir.join("goja-b")).unwrap();

        update_current_symlink(&tools_dir, "goja-a").expect("first symlink");
        // Re-point: the swap must succeed even though `current` already
        // exists (POSIX rename overwrites).
        update_current_symlink(&tools_dir, "goja-b").expect("re-point");

        let resolved = fs::read_link(tools_dir.join("current"))
            .expect("read symlink target");
        assert_eq!(resolved, std::path::Path::new("goja-b"));

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
            default_data_root: PathBuf::from("/tmp/cache/goja-studio"),
            log_dir: PathBuf::from("/tmp/state/logs"),
        };
        let mut settings = ManagerSettings::default_for_paths(&paths);
        // Ensure env var is not influencing this test.
        std::env::remove_var("GOJA_RELEASE_REPO");

        // Default: fork at the post-rename URL (haraldwegner).
        assert_eq!(
            latest_release_url(&settings),
            "https://api.github.com/repos/haraldwegner/goja-mcp/releases/latest"
        );

        // Custom override (e.g. legacy upstream or another fork).
        settings.release_repo = "pzalutski-pixel/goja-mcp".into();
        assert_eq!(
            latest_release_url(&settings),
            "https://api.github.com/repos/pzalutski-pixel/goja-mcp/releases/latest"
        );
    }
}
