//! Sprint 21a (item E): centralized, pruned deploy backups.
//!
//! Before this module, every managed write dropped a `<file>.bak-<epoch-ms>` sibling
//! NEXT TO the user's file — littering `~/.claude`, project dirs, and client configs
//! without any retention. Now all backups land in ONE managed area
//! (`<data_root>/backups/<path-key>/<version>`), keep the last
//! [`DEFAULT_RETENTION`] versions per file, and a one-time GC (dry-run first) sweeps
//! the historically scattered `.bak` files in.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// Keep the last N versions per original file; pruned on every write.
pub const DEFAULT_RETENTION: usize = 10;

/// Item F: retention is a Knowledge-prefs setting; follows it like the root does.
static RETENTION: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_RETENTION);

pub fn set_backup_retention(keep: usize) {
    RETENTION.store(keep.max(1), Ordering::Relaxed);
}

fn retention() -> usize {
    RETENTION.load(Ordering::Relaxed)
}

/// The configured `data_root` (the managed area lives under it). Set at ConfigStore
/// load + settings updates + deploy entry; falls back to the default data root so a
/// backup is never silently dropped before configuration is read.
static BACKUPS_ROOT: RwLock<Option<String>> = RwLock::new(None);
/// Tie-breaker so two backups in the same millisecond stay distinct + ordered.
static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn set_backups_root(data_root: &str) {
    let trimmed = data_root.trim();
    if !trimmed.is_empty() {
        *BACKUPS_ROOT.write().expect("backups root lock poisoned") = Some(trimmed.to_string());
    }
}

fn backups_dir() -> PathBuf {
    let root = BACKUPS_ROOT
        .read()
        .expect("backups root lock poisoned")
        .clone()
        .unwrap_or_else(crate::config::default_data_root);
    PathBuf::from(root).join("backups")
}

/// The managed-area key for an original path: the absolute path with separators (and
/// the Windows drive colon) as dashes — collision-free, human-readable, and the same
/// convention Claude uses for per-project dirs.
fn key_for(original: &Path) -> String {
    original
        .to_string_lossy()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect()
}

/// Copy `original` into the managed area as a new version and prune to retention.
/// `Ok(None)` when the original does not exist (nothing to back up). NEVER writes a
/// sibling next to the original.
pub fn backup_before_write(original: &Path) -> Result<Option<PathBuf>, String> {
    if !original.exists() {
        return Ok(None);
    }
    let dir = backups_dir().join(key_for(original));
    fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create backup dir {}: {error}", dir.display()))?;
    let version = format!(
        "{:013}-{:04}",
        crate::config::current_timestamp_millis(),
        COUNTER.fetch_add(1, Ordering::Relaxed) % 10_000
    );
    let target = dir.join(version);
    fs::copy(original, &target).map_err(|error| {
        format!(
            "cannot back up {} to {}: {error}",
            original.display(),
            target.display()
        )
    })?;
    prune_dir(&dir, retention());
    Ok(Some(target))
}

/// The most recent managed backup of `original`, if any.
pub fn latest_backup_path(original: &Path) -> Option<PathBuf> {
    let dir = backups_dir().join(key_for(original));
    let mut versions = list_versions(&dir);
    versions.pop()
}

/// Version files of one key dir, sorted ascending (name = zero-padded millis-counter).
fn list_versions(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    versions.sort();
    versions
}

fn prune_dir(dir: &Path, keep: usize) {
    let versions = list_versions(dir);
    if versions.len() > keep {
        for stale in &versions[..versions.len() - keep] {
            let _ = fs::remove_file(stale);
        }
    }
}

// ===== One-time GC of historically scattered backups ==================================

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcItem {
    pub file: String,
    pub original: String,
    pub action: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcReport {
    pub dry_run: bool,
    pub scanned_dirs: usize,
    pub items: Vec<GcItem>,
    pub moved: usize,
    pub unrecognized_skipped: usize,
}

/// Recognize a goja-studio-authored scattered backup file name. Returns the ORIGINAL
/// file name + the version stamp. Two shapes were ever written:
/// `<name>.bak-<epoch-ms>` (managed writes) and `<name>.json.bak.<epoch-ms>`
/// (`write_json`). Anything else is NOT ours and is never touched.
fn recognize(file_name: &str) -> Option<(String, String)> {
    if let Some((original, stamp)) = file_name.rsplit_once(".bak-") {
        if (10..=17).contains(&stamp.len()) && stamp.bytes().all(|b| b.is_ascii_digit()) {
            return Some((original.to_string(), stamp.to_string()));
        }
    }
    if let Some((prefix, stamp)) = file_name.rsplit_once(".bak.") {
        if prefix.ends_with(".json")
            && (10..=17).contains(&stamp.len())
            && stamp.bytes().all(|b| b.is_ascii_digit())
        {
            return Some((prefix.to_string(), stamp.to_string()));
        }
    }
    None
}

/// Sweep the direct children of each given dir for scattered goja-studio backups and
/// move them into the managed area (then prune to retention). `dry_run` reports the
/// exact plan without touching anything. Unrecognized files are never touched.
pub fn gc_scattered_backups(dirs: &[PathBuf], dry_run: bool) -> GcReport {
    let mut items = Vec::new();
    let mut moved = 0;
    let mut unrecognized = 0;
    let mut scanned = 0;
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        scanned += 1;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some((original_name, stamp)) = recognize(&name) else {
                if name.contains(".bak") {
                    unrecognized += 1;
                }
                continue;
            };
            let original = dir.join(&original_name);
            let action = format!("move to managed area (version {stamp})");
            if !dry_run {
                let target_dir = backups_dir().join(key_for(&original));
                let stamp13 = format!("{:0>13}", stamp);
                let ok = fs::create_dir_all(&target_dir).is_ok()
                    && fs::rename(&path, target_dir.join(&stamp13))
                        .or_else(|_| {
                            fs::copy(&path, target_dir.join(&stamp13))
                                .map(|_| ())
                                .and_then(|_| fs::remove_file(&path))
                        })
                        .is_ok();
                if ok {
                    prune_dir(&target_dir, retention());
                    moved += 1;
                } else {
                    continue;              // unreadable/unmovable → leave it, report nothing done
                }
            }
            items.push(GcItem {
                file: path.to_string_lossy().to_string(),
                original: original.to_string_lossy().to_string(),
                action,
            });
        }
    }
    GcReport {
        dry_run,
        scanned_dirs: scanned,
        items,
        moved,
        unrecognized_skipped: unrecognized,
    }
}

/// Serializes root-mutating tests (the root is process-global).
#[cfg(test)]
pub(crate) fn test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &LOCK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "goja-backups-{tag}-{}",
            crate::config::current_timestamp_millis()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recognize_only_our_shapes() {
        assert_eq!(
            recognize("CLAUDE.md.bak-1783004539489"),
            Some(("CLAUDE.md".into(), "1783004539489".into()))
        );
        assert_eq!(
            recognize("settings.json.bak.1783004539489"),
            Some(("settings.json".into(), "1783004539489".into()))
        );
        assert_eq!(recognize("notes.bak-abc"), None, "non-numeric stamp is not ours");
        assert_eq!(recognize("foo.backup"), None);
        assert_eq!(recognize("data.bak.123"), None, ".bak. only after .json");
    }

    #[test]
    fn backup_versions_prune_and_latest_no_siblings() {
        let _guard = test_lock().lock().unwrap();
        let root = tempdir("root");
        set_backups_root(root.to_string_lossy().as_ref());

        let workdir = tempdir("work");
        let original = workdir.join("settings.json");

        for i in 0..(DEFAULT_RETENTION + 3) {
            fs::write(&original, format!("v{i}")).unwrap();
            backup_before_write(&original).unwrap().expect("existing file is backed up");
        }

        // ZERO siblings beside the user's file.
        let siblings: Vec<_> = fs::read_dir(&workdir).unwrap().flatten().collect();
        assert_eq!(siblings.len(), 1, "only the original lives in the user dir");

        // Managed area holds a bounded history; latest returns the newest content.
        let key_dir = root.join("backups").join(key_for(&original));
        assert_eq!(list_versions(&key_dir).len(), DEFAULT_RETENTION, "pruned to retention");
        let latest = latest_backup_path(&original).expect("has a latest backup");
        assert_eq!(
            fs::read_to_string(latest).unwrap(),
            format!("v{}", DEFAULT_RETENTION + 2)
        );

        // A missing original is a clean no-op.
        assert!(backup_before_write(&workdir.join("nope.txt")).unwrap().is_none());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(workdir);
    }

    #[test]
    fn gc_dry_run_reports_and_touches_nothing_then_moves() {
        let _guard = test_lock().lock().unwrap();
        let root = tempdir("gc-root");
        set_backups_root(root.to_string_lossy().as_ref());

        let scattered = tempdir("gc-scattered");
        let ours = scattered.join("CLAUDE.md.bak-1783004539489");
        fs::write(&ours, "old backup").unwrap();
        let not_ours = scattered.join("keepme.bak-notes");
        fs::write(&not_ours, "user file").unwrap();

        let dry = gc_scattered_backups(&[scattered.clone()], true);
        assert!(dry.dry_run);
        assert_eq!(dry.items.len(), 1, "exactly the recognized file is planned");
        assert_eq!(dry.moved, 0);
        assert!(ours.exists(), "dry-run touches nothing");
        assert_eq!(dry.unrecognized_skipped, 1, "the user's .bak-notes is reported skipped");

        let real = gc_scattered_backups(&[scattered.clone()], false);
        assert_eq!(real.moved, 1);
        assert!(!ours.exists(), "recognized backup moved into the managed area");
        assert!(not_ours.exists(), "unrecognized file NEVER touched");
        let original = scattered.join("CLAUDE.md");
        let versions = list_versions(&root.join("backups").join(key_for(&original)));
        assert_eq!(versions.len(), 1, "landed as a managed version");

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(scattered);
    }
}
