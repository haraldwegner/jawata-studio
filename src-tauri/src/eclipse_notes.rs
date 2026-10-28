//! Sprint 28k, Stage 20 (contract §14.1, the studio part) — the notes an Eclipse running the jawata plug-in
//! leaves for the machine, and the rows studio builds from them.
//!
//! An Eclipse with the plug-in writes one small JSON note per workspace into
//! `<OS cache folder>/jawata-studio/engines/` when its engine is ready, rewrites it when the project set
//! changes, and deletes it at a clean stop. Studio **reads** them. It never deletes one, never writes one,
//! and never opens the Eclipse workspace folder a note names: a note whose process is gone is shown as
//! "not running", because a delete racing a restarting Eclipse could remove the live note.
//!
//! The reader mirrors the Java one (`org.jawata.mcp.models.EngineNotes`), outcome for outcome:
//! LIVE, STALE, NEWER_MAJOR, MALFORMED. The two read the same fixture set (`test-fixtures/engine-notes`,
//! byte-identical, checked against its SHA256SUMS), so they cannot drift apart unnoticed.
//!
//! Whether a note is live is decided HERE and nowhere else; the screens and the tray read the verdict from
//! the row (`EclipseWorkspaceRow::running`), the same discipline `RuntimePhase::is_live` keeps for the
//! headless workspaces (S19-10b).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The prefix of the `schema` field: `jawata.engine-note/<major>`.
const SCHEMA_PREFIX: &str = "jawata.engine-note/";

/// The highest major of the schema this reader understands.
const SUPPORTED_MAJOR: i64 = 1;

/// The producer writes the start time to the second and the OS reports finer, so a second is the same moment.
const START_SLACK_MILLIS: i64 = 1000;

/// What a note is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteStatus {
    /// The process that wrote the note is alive and started when the note says.
    Live,
    /// The engine crashed or was killed; the note stays (only the producer deletes it).
    Stale,
    /// Written by a newer jawata: reported by name, never read further.
    NewerMajor,
    /// Unreadable, or lacking what makes it usable. Reported, and the notes beside it are still read.
    Malformed,
}

/// One project of an Eclipse workspace, as the note lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EclipseProject {
    pub name: String,
    pub path: String,
    pub open: bool,
}

/// One note, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoteReading {
    /// The note's file name, which is how a reader finds it again.
    pub file_name: String,
    pub status: NoteStatus,
    /// The workspace the note names, or the file name when the note is too damaged to say.
    pub workspace_name: String,
    /// The canonical workspace path the note carries, when it carries one.
    pub workspace_path: Option<String>,
    /// The process id the note carries, 0 when it carries none.
    pub pid: i64,
    /// The engine's port, present for a note that was read far enough to have one.
    pub port: Option<u16>,
    /// The note's project list; empty for a note of a newer major (not read further) and a malformed one.
    pub projects: Vec<EclipseProject>,
    /// One sentence saying why the note has the status it has.
    pub detail: String,
}

/// Whether the process that wrote a note is still the one running: given the pid and the start time the note
/// carries (epoch milliseconds, or None when it carries none).
pub(crate) type Liveness<'a> = &'a dyn Fn(u32, Option<i64>) -> bool;

/// Where the Eclipse engine writes its notes: studio's default data folder, independent of the `data_root`
/// setting. `dirs::cache_dir()` is the OS cache folder the Java side computes by hand (Linux
/// `$XDG_CACHE_HOME` or the `.cache` folder of the home folder, macOS `Library/Caches` of the home folder,
/// Windows `%LOCALAPPDATA%`).
pub(crate) fn default_notes_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|cache| cache.join("jawata-studio").join("engines"))
}

/// Every note in the folder, in file-name order. An absent folder holds no notes: the ordinary machine without
/// the plug-in. Only `*.json` regular files are notes; the producer's half-written temporary files are not.
pub(crate) fn read_notes(dir: Option<&Path>, alive: Liveness) -> Vec<NoteReading> {
    let Some(dir) = dir.filter(|d| d.is_dir()) else {
        return Vec::new();
    };
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) => {
            eprintln!("[jawata-studio] the engine notes folder {} could not be listed ({error}); showing none.", dir.display());
            return Vec::new();
        }
    };
    let mut files: Vec<PathBuf> = listing
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".json")))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files.iter().map(|file| read_one(file, alive)).collect()
}

fn read_one(path: &Path, alive: Liveness) -> NoteReading {
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let malformed = |name: &str, pid: i64, why: String| NoteReading {
        file_name: file_name.clone(),
        status: NoteStatus::Malformed,
        workspace_name: name.to_string(),
        workspace_path: None,
        pid,
        port: None,
        projects: Vec::new(),
        detail: why,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => return malformed(&file_name, 0, format!("the file is not readable ({error})")),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return malformed(&file_name, 0, "the file is not readable JSON".to_string()),
    };
    let Some(note) = value.as_object() else {
        return malformed(&file_name, 0, "the file holds no JSON object".to_string());
    };
    let text_of = |key: &str| note.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let named = text_of("workspaceName");
    let workspace_name = if named.trim().is_empty() { file_name.clone() } else { named.clone() };
    let pid = pid_of(note.get("pid"));

    let schema = text_of("schema");
    let major = major_of(&schema);
    if major < 1 {
        return malformed(&workspace_name, pid, format!("the schema field '{schema}' is not {SCHEMA_PREFIX}<major>"));
    }
    if major > SUPPORTED_MAJOR {
        return NoteReading {
            file_name,
            status: NoteStatus::NewerMajor,
            workspace_name,
            workspace_path: None,
            pid,
            port: None,
            projects: Vec::new(),
            detail: format!(
                "Eclipse workspace (newer jawata): the note is schema major {major} and this reader understands {SUPPORTED_MAJOR}; it is not read further"
            ),
        };
    }
    if pid <= 0 {
        return malformed(&workspace_name, pid, "the note carries no process id".to_string());
    }
    if named.trim().is_empty() {
        return malformed(&workspace_name, pid, "the note names no workspace".to_string());
    }
    let Some(port) = port_of(&text_of("endpoint")) else {
        return malformed(&workspace_name, pid, "the note carries no endpoint to ask at".to_string());
    };
    let noted_start = parse_instant(&text_of("processStartedAt"));
    let is_alive = match u32::try_from(pid) {
        Ok(pid) => alive(pid, noted_start),
        Err(_) => false, // no OS hands out a pid this large: the note is not from a process that runs here
    };
    let workspace_path = Some(text_of("workspacePath")).filter(|p| !p.trim().is_empty());
    NoteReading {
        file_name,
        status: if is_alive { NoteStatus::Live } else { NoteStatus::Stale },
        workspace_name,
        workspace_path,
        pid,
        port: Some(port),
        projects: projects_of(note.get("projects")),
        detail: if is_alive {
            format!("process {pid} is alive and started when the note says")
        } else {
            format!("process {pid} is gone, or is not the process that wrote the note (a pid reused after a crash has another start time)")
        },
    }
}

fn pid_of(value: Option<&serde_json::Value>) -> i64 {
    match value {
        Some(serde_json::Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// The major of a schema field, or 0 when the field is not `jawata.engine-note/<major>`.
fn major_of(schema: &str) -> i64 {
    schema
        .strip_prefix(SCHEMA_PREFIX)
        .and_then(|major| major.trim().parse::<i64>().ok())
        .map_or(0, |major| major.max(0))
}

/// The port of an `http://host:port/path` endpoint, when it has a usable one.
fn port_of(endpoint: &str) -> Option<u16> {
    let (_, rest) = endpoint.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let port_text = match host_port.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once(']')?.1.strip_prefix(':')?,
        None => host_port.rsplit_once(':')?.1,
    };
    port_text.parse::<u32>().ok().filter(|p| (1..=65535).contains(p)).map(|p| p as u16)
}

fn projects_of(value: Option<&serde_json::Value>) -> Vec<EclipseProject> {
    let Some(list) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let (name, path) = (text("name"), text("path"));
            if name.is_empty() && path.is_empty() {
                return None;
            }
            Some(EclipseProject { name, path, open: entry.get("open").and_then(|v| v.as_bool()).unwrap_or(false) })
        })
        .collect()
}

/// An ISO-8601 instant (`2026-10-22T09:00:00Z`, optional fraction, `Z` or `±HH:MM`) as epoch milliseconds.
fn parse_instant(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-');
    let (year, month, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    if d.next().is_some() || !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let (clock, offset_minutes) = if let Some(clock) = time.strip_suffix('Z') {
        (clock, 0)
    } else {
        let at = time.rfind(['+', '-'])?;
        let (clock, zone) = time.split_at(at);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let (oh, om) = zone[1..].split_once(':')?;
        (clock, sign * (oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?))
    };
    let (hms, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let mut t = hms.split(':');
    let (hour, minute, second): (i64, i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    if t.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let millis = if fraction.is_empty() {
        0
    } else if fraction.bytes().all(|b| b.is_ascii_digit()) {
        format!("{:0<3}", &fraction[..fraction.len().min(3)]).parse::<i64>().ok()?
    } else {
        return None;
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1000 + millis)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Two start times are the same moment when they differ by no more than a second: the note's is truncated to
/// the second and the OS reports finer.
fn same_start(noted: i64, actual: i64) -> bool {
    (noted - actual).abs() <= START_SLACK_MILLIS
}

/// The real liveness check: the pid is alive (studio's existing pid check) and, where the OS reports a start
/// time, it equals the one the note carries. Where the OS reports none (Windows), the pid alone has to do.
pub(crate) fn os_liveness(pid: u32, noted_start: Option<i64>) -> bool {
    if !crate::runtime_manager::process_alive(pid) {
        return false;
    }
    match (noted_start, process_start_millis(pid)) {
        (Some(noted), Some(actual)) => same_start(noted, actual),
        _ => true,
    }
}

/// When the process started, in epoch milliseconds, where the OS tells: Linux from `/proc` (the boot time plus
/// the process's start tick, which is how the JVM's own `ProcessHandle` reads it, so the plug-in's note and this
/// reading agree), macOS and the other Unixes from `ps -o lstart=`.
#[cfg(unix)]
pub(crate) fn process_start_millis(pid: u32) -> Option<i64> {
    if Path::new("/proc/stat").exists() {
        proc_start_millis(pid)
    } else {
        ps_start_millis(pid)
    }
}

/// Windows reports no start time through the pid check studio uses; the pid alone decides, as the contract allows.
#[cfg(windows)]
pub(crate) fn process_start_millis(_pid: u32) -> Option<i64> {
    None
}

#[cfg(unix)]
fn proc_start_millis(pid: u32) -> Option<i64> {
    let ticks = proc_stat_start_ticks(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    let boot_seconds: i64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    // SAFETY: sysconf only reads a system constant.
    let per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if per_second <= 0 {
        return None;
    }
    Some(boot_seconds * 1000 + (ticks as i64) * 1000 / per_second)
}

/// The 22nd field of a `/proc/<pid>/stat` line: the process's start, in clock ticks after boot. The command name
/// (field 2) may hold spaces and parentheses, so the fields are counted from the LAST closing parenthesis.
#[cfg(unix)]
fn proc_stat_start_ticks(line: &str) -> Option<u64> {
    let after_command = &line[line.rfind(')')? + 1..];
    after_command.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(unix)]
fn ps_start_millis(pid: u32) -> Option<i64> {
    // TZ=UTC makes ps print the start in UTC, so no time-zone table is needed to read it.
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_lstart(String::from_utf8_lossy(&output.stdout).trim())
}

/// `ps -o lstart=` prints `Thu Oct 22 09:00:00 2026`.
#[cfg(unix)]
fn parse_lstart(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let (_weekday, month, day, clock, year) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = MONTHS.iter().position(|m| *m == month)? as i64 + 1;
    let mut t = clock.split(':');
    let (hour, minute, second): (i64, i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    let seconds = days_from_civil(year.parse().ok()?, month, day.parse().ok()?) * 86_400 + hour * 3600 + minute * 60 + second;
    Some(seconds * 1000)
}

/// What studio remembers of an Eclipse workspace it has seen, in its own settings, so a closed one stays listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedEclipseWorkspace {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub projects: Vec<EclipseProject>,
}

/// One Eclipse workspace as the list and the tray show it. It carries no action of any kind: an Eclipse
/// workspace is shown, never switched (contract §14.1, revision 2a). `running` is the one verdict both screens
/// read, so neither spells out the rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EclipseWorkspaceRow {
    /// The workspace path, or `file:<note name>` for a note that carries none.
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub running: bool,
    /// A note written by a newer jawata: named, and not read further.
    pub newer_jawata: bool,
    pub status_text: String,
    pub projects: Vec<EclipseProject>,
}

/// The rows to show and the list to remember, from what the folder holds now and what was remembered.
///
/// A workspace studio has seen stays listed when its note is gone (Eclipse closed and deleted it) or stale (it
/// crashed). The malformed note has no row: it names nothing studio can show.
pub(crate) fn eclipse_rows(
    readings: &[NoteReading],
    remembered: &[RememberedEclipseWorkspace],
) -> (Vec<EclipseWorkspaceRow>, Vec<RememberedEclipseWorkspace>) {
    let mut known: BTreeMap<String, RememberedEclipseWorkspace> =
        remembered.iter().map(|r| (r.path.clone(), r.clone())).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut running: HashSet<String> = HashSet::new();
    let mut loose: Vec<EclipseWorkspaceRow> = Vec::new();

    for reading in readings {
        let live = reading.status == NoteStatus::Live;
        match (reading.status, reading.workspace_path.as_deref()) {
            (NoteStatus::Live | NoteStatus::Stale, Some(path)) => {
                // A live reading outranks a stale one for the same workspace.
                if live || !seen.contains(path) {
                    known.insert(
                        path.to_string(),
                        RememberedEclipseWorkspace {
                            path: path.to_string(),
                            name: reading.workspace_name.clone(),
                            projects: reading.projects.clone(),
                        },
                    );
                    seen.insert(path);
                }
                if live {
                    running.insert(path.to_string());
                }
            }
            (NoteStatus::Live | NoteStatus::Stale, None) => {
                loose.push(row(format!("file:{}", reading.file_name), &reading.workspace_name, None, live, false, &reading.projects))
            }
            (NoteStatus::NewerMajor, _) => {
                loose.push(row(format!("file:{}", reading.file_name), &reading.workspace_name, None, false, true, &[]))
            }
            (NoteStatus::Malformed, _) => {}
        }
    }

    let mut rows: Vec<EclipseWorkspaceRow> = known
        .values()
        .map(|w| row(w.path.clone(), &w.name, Some(w.path.clone()), running.contains(&w.path), false, &w.projects))
        .collect();
    rows.extend(loose);
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    (rows, known.into_values().collect())
}

fn row(id: String, name: &str, path: Option<String>, running: bool, newer_jawata: bool, projects: &[EclipseProject]) -> EclipseWorkspaceRow {
    let status_text = if newer_jawata {
        "Eclipse workspace (newer jawata)"
    } else if running {
        "running"
    } else {
        "not running"
    };
    EclipseWorkspaceRow {
        id,
        name: name.to_string(),
        path,
        running,
        newer_jawata,
        status_text: status_text.to_string(),
        projects: projects.to_vec(),
    }
}

impl EclipseWorkspaceRow {
    /// The tray menu's line for this row: the same row the list shows, in one line. The tray item built from it
    /// is greyed and has no handler, so there is nothing to start.
    pub fn tray_label(&self) -> String {
        let glyph = if self.running { "●" } else { "○" };
        if self.running {
            format!("  {glyph}  {} — Eclipse", self.name)
        } else if self.newer_jawata {
            format!("  {glyph}  {} — {}", self.name, self.status_text)
        } else {
            format!("  {glyph}  {} — Eclipse, {}", self.name, self.status_text)
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs;

    /// 2026-10-22T09:00:00Z, the start time the live and the newer fixtures carry.
    const NOTED_START: i64 = 1_792_659_600_000;

    /// The checked-in copy of the engine's fixtures. Tests READ it only to compare it with the engine's sums and to
    /// copy it; every test that reads notes works on a temporary copy (`fixtures()`), so a defect that deleted a
    /// note could never delete the checked-in fixture.
    fn fixture_source() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("test-fixtures/engine-notes")
    }

    /// A temporary copy of the fixtures, removed when it goes out of scope (whoever creates it removes it).
    struct FixtureCopy(PathBuf);

    impl std::ops::Deref for FixtureCopy {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for FixtureCopy {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixtures() -> FixtureCopy {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("jawata-eclipse-fixtures-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for entry in fs::read_dir(fixture_source()).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
        FixtureCopy(dir)
    }

    /// The fake OS of the fixtures: pid 4242 is not a real process, so the test says who is alive. Only the process
    /// that started at the noted time is "alive"; a pid reused after a crash has another start time.
    fn fake_os(pid: u32, started: Option<i64>) -> bool {
        pid == 4242 && started == Some(NOTED_START)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jawata-eclipse-notes-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn reading<'a>(all: &'a [NoteReading], file: &str) -> &'a NoteReading {
        all.iter().find(|r| r.file_name == file).unwrap_or_else(|| panic!("no reading for {file}"))
    }

    // ---- the fixture copy ----

    #[test]
    fn the_fixture_copy_is_byte_identical_to_the_engines() {
        let dir = fixture_source();
        let sums = fs::read_to_string(dir.join("SHA256SUMS")).expect("the SHA256SUMS file");
        let mut listed = Vec::new();
        for line in sums.lines().filter(|l| !l.trim().is_empty()) {
            let (want, name) = line.split_once("  ").expect("<sum>  <name>");
            let bytes = fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(want, got, "{name} differs from the engine's fixture");
            listed.push(name.to_string());
        }
        let mut present: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "SHA256SUMS")
            .collect();
        present.sort();
        listed.sort();
        assert_eq!(listed, present, "the copy holds exactly the files the sums list");
        assert_eq!(5, listed.len());
    }

    // ---- the four outcomes, over the engine's fixtures ----

    #[test]
    fn a_live_note_is_live_and_carries_its_projects_and_port() {
        let all = read_notes(Some(&*fixtures()), &fake_os);
        let live = reading(&all, "eclipse-1111111111111111.json");
        assert_eq!(NoteStatus::Live, live.status);
        assert_eq!("live", live.workspace_name);
        assert_eq!(Some("/home/user/eclipse-ws/live".to_string()), live.workspace_path);
        assert_eq!(4242, live.pid);
        assert_eq!(Some(43123), live.port);
        assert_eq!(
            vec![
                EclipseProject { name: "foo".into(), path: "/home/user/eclipse-ws/foo".into(), open: true },
                EclipseProject { name: "bar".into(), path: "/home/user/eclipse-ws/bar".into(), open: false },
            ],
            live.projects
        );
    }

    #[test]
    fn a_note_whose_process_is_gone_is_stale() {
        let all = read_notes(Some(&*fixtures()), &fake_os);
        let stale = reading(&all, "eclipse-2222222222222222.json");
        assert_eq!(NoteStatus::Stale, stale.status);
        assert_eq!("stale", stale.workspace_name);
        assert_eq!(1, stale.projects.len(), "a stale note still says which projects the workspace had");
    }

    #[test]
    fn a_reused_pid_is_stale() {
        // Same pid 4242 as the live note, alive on the fake OS, but the process started at another time.
        let all = read_notes(Some(&*fixtures()), &fake_os);
        assert_eq!(NoteStatus::Stale, reading(&all, "eclipse-3333333333333333.json").status);
        // The control: the same pid with the noted start time is live, so it was the start time that decided.
        assert_eq!(NoteStatus::Live, reading(&all, "eclipse-1111111111111111.json").status);
    }

    #[test]
    fn a_malformed_note_is_reported_and_the_notes_beside_it_are_still_read() {
        let all = read_notes(Some(&*fixtures()), &fake_os);
        assert_eq!(5, all.len());
        let bad = reading(&all, "eclipse-4444444444444444.json");
        assert_eq!(NoteStatus::Malformed, bad.status);
        assert_eq!("eclipse-4444444444444444.json", bad.workspace_name, "a note too damaged to name itself is named by its file");
        let names: Vec<&str> = all.iter().map(|r| r.file_name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, names, "in file-name order");
        assert_eq!(1, all.iter().filter(|r| r.status == NoteStatus::Live).count());
    }

    #[test]
    fn a_note_of_a_higher_major_is_reported_by_name_and_not_read_further() {
        let all = read_notes(Some(&*fixtures()), &fake_os);
        let newer = reading(&all, "eclipse-5555555555555555.json");
        assert_eq!(NoteStatus::NewerMajor, newer.status);
        assert_eq!("newer", newer.workspace_name);
        assert!(newer.projects.is_empty(), "a higher major is shown without its projects");
        assert_eq!(None, newer.port, "its endpoints are in a shape this reader does not know");
    }

    #[test]
    fn an_absent_folder_holds_no_notes() {
        let missing = std::env::temp_dir().join(format!("jawata-no-such-notes-{}", std::process::id()));
        assert!(read_notes(Some(&missing), &fake_os).is_empty());
        assert!(read_notes(None, &fake_os).is_empty());
    }

    // ---- what makes a note malformed, one at a time ----

    fn note_with(tag: &str, body: &str) -> NoteReading {
        let dir = temp_dir(tag);
        fs::write(dir.join("eclipse-aaaaaaaaaaaaaaaa.json"), body).unwrap();
        let all = read_notes(Some(&dir), &|_, _| true);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(1, all.len());
        all.into_iter().next().unwrap()
    }

    const GOOD: &str = r#"{"schema":"jawata.engine-note/1","workspaceName":"w","workspacePath":"/w","pid":7,"processStartedAt":"2026-10-22T09:00:00Z","projects":[],"endpoint":"http://127.0.0.1:5000/mcp"}"#;

    #[test]
    fn the_good_baseline_is_live() {
        assert_eq!(NoteStatus::Live, note_with("good", GOOD).status);
    }

    #[test]
    fn each_thing_that_makes_a_note_unusable_makes_it_malformed() {
        let cases: Vec<(&str, String)> = vec![
            ("not json", "{ this is not json".into()),
            ("not an object", "[1,2,3]".into()),
            ("bad schema", GOOD.replace("jawata.engine-note/1", "something/1")),
            ("schema without a major", GOOD.replace("jawata.engine-note/1", "jawata.engine-note/x")),
            ("major zero", GOOD.replace("jawata.engine-note/1", "jawata.engine-note/0")),
            ("pid zero", GOOD.replace("\"pid\":7", "\"pid\":0")),
            ("negative pid", GOOD.replace("\"pid\":7", "\"pid\":-3")),
            ("no pid", GOOD.replace("\"pid\":7,", "")),
            ("no workspace name", GOOD.replace("\"workspaceName\":\"w\",", "")),
            ("blank workspace name", GOOD.replace("\"workspaceName\":\"w\"", "\"workspaceName\":\"  \"")),
            ("no endpoint", GOOD.replace(",\"endpoint\":\"http://127.0.0.1:5000/mcp\"", "")),
            ("endpoint without a port", GOOD.replace("127.0.0.1:5000", "127.0.0.1")),
            ("port zero", GOOD.replace(":5000", ":0")),
            ("port out of range", GOOD.replace(":5000", ":99999")),
        ];
        for (what, body) in cases {
            let r = note_with("bad", &body);
            assert_eq!(NoteStatus::Malformed, r.status, "{what}: {}", r.detail);
        }
    }

    #[test]
    fn a_note_with_no_start_time_is_live_where_the_process_is_alive() {
        let r = note_with("nostart", &GOOD.replace(",\"processStartedAt\":\"2026-10-22T09:00:00Z\"", ""));
        assert_eq!(NoteStatus::Live, r.status);
    }

    #[test]
    fn only_json_files_are_notes() {
        let dir = temp_dir("kinds");
        fs::write(dir.join("eclipse-aaaaaaaaaaaaaaaa.json"), GOOD).unwrap();
        fs::write(dir.join("eclipse-aaaaaaaaaaaaaaaa.json.tmp"), "{ half").unwrap();
        fs::write(dir.join("README.txt"), "x").unwrap();
        fs::create_dir_all(dir.join("sub.json")).unwrap();
        let all = read_notes(Some(&dir), &|_, _| true);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(1, all.len());
    }

    // ---- the time ----

    #[test]
    fn the_noted_start_time_is_read_as_epoch_milliseconds() {
        assert_eq!(Some(NOTED_START), parse_instant("2026-10-22T09:00:00Z"));
        assert_eq!(Some(978_307_200_000), parse_instant("2001-01-01T00:00:00Z"));
        assert_eq!(Some(NOTED_START + 250), parse_instant("2026-10-22T09:00:00.25Z"));
        assert_eq!(Some(NOTED_START), parse_instant("2026-10-22T11:00:00+02:00"));
        assert_eq!(None, parse_instant("yesterday"));
        assert_eq!(None, parse_instant(""));
        assert_eq!(None, parse_instant("2026-13-22T09:00:00Z"));
    }

    #[test]
    fn two_start_times_within_a_second_are_the_same_moment() {
        assert!(same_start(NOTED_START, NOTED_START + 999));
        assert!(same_start(NOTED_START, NOTED_START - 1000));
        assert!(!same_start(NOTED_START, NOTED_START + 1001));
    }

    #[cfg(unix)]
    #[test]
    fn a_proc_stat_line_is_read_past_a_command_name_with_parentheses_and_spaces() {
        let line = "4242 (my (odd) name) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 100 18446744073709551615";
        assert_eq!(Some(987_654), proc_stat_start_ticks(line));
        assert_eq!(None, proc_stat_start_ticks("4242 no parenthesis"));
    }

    #[cfg(unix)]
    #[test]
    fn a_ps_start_line_is_read_as_utc() {
        assert_eq!(Some(NOTED_START), parse_lstart("Thu Oct 22 09:00:00 2026"));
        assert_eq!(Some(NOTED_START), parse_lstart("Thu Oct  22 09:00:00 2026"));
        assert_eq!(Some(978_307_200_000), parse_lstart("Mon Jan  1 00:00:00 2001"));
        assert_eq!(None, parse_lstart("garbage"));
    }

    // ---- the real OS check, over this very test process ----

    #[test]
    fn the_real_check_knows_this_process_and_not_a_process_that_started_at_another_time() {
        let me = std::process::id();
        assert!(os_liveness(me, None), "no noted start: the pid alone decides");
        #[cfg(unix)]
        {
            let started = process_start_millis(me).expect("this OS reports a start time");
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
            assert!(started <= now && now - started < 24 * 3600 * 1000, "a test process started within the day: {started} vs {now}");
            assert!(os_liveness(me, Some(started)), "this process, started when it started");
            assert!(os_liveness(me, Some(started + 400)), "within the second");
            assert!(!os_liveness(me, Some(started - 3_600_000)), "a reused pid: same number, another start time");
        }
        assert!(!os_liveness(2_147_483_646, None), "no such process");
    }

    #[test]
    fn a_pid_beyond_what_an_os_can_hand_out_is_stale_and_not_an_error() {
        let dir = temp_dir("bigpid");
        fs::write(dir.join("eclipse-aaaaaaaaaaaaaaaa.json"), GOOD.replace("\"pid\":7", "\"pid\":99999999999")).unwrap();
        let all = read_notes(Some(&dir), &|_, _| panic!("no process has this pid"));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(NoteStatus::Stale, all[0].status);
    }

    // ---- the rows ----

    fn rows_of(dir: &Path, remembered: &[RememberedEclipseWorkspace]) -> (Vec<EclipseWorkspaceRow>, Vec<RememberedEclipseWorkspace>) {
        eclipse_rows(&read_notes(Some(dir), &fake_os), remembered)
    }

    #[test]
    fn the_fixtures_make_one_running_one_not_running_and_one_newer_row_and_no_row_for_the_malformed_one() {
        let (rows, remembered) = rows_of(&fixtures(), &[]);
        let by_name = |n: &str| rows.iter().find(|r| r.name == n).unwrap_or_else(|| panic!("no row {n}"));
        assert_eq!(4, rows.len(), "live, stale, reused, newer; the malformed note has no row: {rows:?}");
        assert!(by_name("live").running);
        assert_eq!("running", by_name("live").status_text);
        assert!(!by_name("stale").running);
        assert_eq!("not running", by_name("stale").status_text);
        assert!(!by_name("reused").running);
        let newer = by_name("newer");
        assert!(newer.newer_jawata && !newer.running);
        assert_eq!("Eclipse workspace (newer jawata)", newer.status_text);
        assert!(newer.projects.is_empty());
        assert_eq!(2, by_name("live").projects.len());
        // Studio remembers the three it could read a path from, and not the newer one it must not guess at.
        let mut names: Vec<&str> = remembered.iter().map(|r| r.name.as_str()).collect();
        names.sort();
        assert_eq!(vec!["live", "reused", "stale"], names);
    }

    #[test]
    fn a_workspace_that_was_seen_and_whose_note_is_gone_stays_listed_as_not_running() {
        let dir = temp_dir("remember");
        fs::write(dir.join("eclipse-aaaaaaaaaaaaaaaa.json"), GOOD).unwrap();
        let (rows, remembered) = eclipse_rows(&read_notes(Some(&dir), &|_, _| true), &[]);
        assert!(rows[0].running);
        // Eclipse closes cleanly and deletes its note.
        fs::remove_file(dir.join("eclipse-aaaaaaaaaaaaaaaa.json")).unwrap();
        let (rows, again) = eclipse_rows(&read_notes(Some(&dir), &|_, _| true), &remembered);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(1, rows.len());
        assert_eq!("w", rows[0].name);
        assert!(!rows[0].running);
        assert_eq!("not running", rows[0].status_text);
        assert_eq!(remembered, again, "nothing about the closed workspace is forgotten or rewritten");
    }

    #[test]
    fn the_last_project_list_is_what_is_remembered() {
        let old = RememberedEclipseWorkspace {
            path: "/w".into(),
            name: "w".into(),
            projects: vec![EclipseProject { name: "gone".into(), path: "/gone".into(), open: true }],
        };
        let dir = temp_dir("lastlist");
        fs::write(
            dir.join("eclipse-aaaaaaaaaaaaaaaa.json"),
            GOOD.replace("\"projects\":[]", "\"projects\":[{\"name\":\"p\",\"path\":\"/p\",\"open\":false}]"),
        )
        .unwrap();
        let (rows, remembered) = eclipse_rows(&read_notes(Some(&dir), &|_, _| true), &[old]);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(1, rows.len());
        assert_eq!(vec!["p"], rows[0].projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>());
        assert_eq!(1, remembered.len());
        assert_eq!("p", remembered[0].projects[0].name);
    }

    #[test]
    fn three_open_and_one_closed_are_three_running_and_one_not_running() {
        let dir = temp_dir("three");
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            let body = GOOD.replace("\"workspaceName\":\"w\"", &format!("\"workspaceName\":\"{name}\"")).replace("\"/w\"", &format!("\"/ws/{name}\""));
            fs::write(dir.join(format!("eclipse-{i:016}.json")), body).unwrap();
        }
        let closed = RememberedEclipseWorkspace { path: "/ws/d".into(), name: "d".into(), projects: vec![] };
        let (rows, _) = eclipse_rows(&read_notes(Some(&dir), &|_, _| true), &[closed]);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(vec![("a", true), ("b", true), ("c", true), ("d", false)], rows.iter().map(|r| (r.name.as_str(), r.running)).collect::<Vec<_>>());
    }

    // ---- shown, never switched ----

    #[test]
    fn an_eclipse_row_carries_no_action() {
        let (rows, _) = rows_of(&fixtures(), &[]);
        let json = serde_json::to_value(&rows[0]).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            vec!["id", "name", "newerJawata", "path", "projects", "running", "statusText"],
            keys,
            "a row is facts about a workspace: no switch, no start, no stop, no handler"
        );
    }

    #[test]
    fn the_tray_line_of_a_row_says_eclipse_and_whether_it_runs() {
        let (rows, _) = rows_of(&fixtures(), &[]);
        let line = |n: &str| rows.iter().find(|r| r.name == n).unwrap().tray_label();
        assert_eq!("  ●  live — Eclipse", line("live"));
        assert_eq!("  ○  stale — Eclipse, not running", line("stale"));
        assert_eq!("  ○  newer — Eclipse workspace (newer jawata)", line("newer"));
    }

    // ---- never delete, never write ----

    #[test]
    fn reading_the_notes_leaves_the_folder_exactly_as_it_was() {
        let dir = temp_dir("untouched");
        for entry in fs::read_dir(fixture_source()).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
        let snapshot = |d: &Path| -> BTreeMap<String, Vec<u8>> {
            fs::read_dir(d).unwrap().map(|e| {
                let e = e.unwrap();
                (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap())
            }).collect()
        };
        let before = snapshot(&dir);
        let readings = read_notes(Some(&dir), &fake_os);
        let _ = eclipse_rows(&readings, &[]);
        let after = snapshot(&dir);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(before, after, "the stale, reused, newer and malformed notes are all still there, unchanged");
        assert_eq!(6, after.len());
    }

    #[test]
    fn this_module_has_no_call_that_deletes_or_writes_a_file() {
        let source = include_str!("eclipse_notes.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["remove_file", "remove_dir", "fs::write", "File::create", "fs::rename", "OpenOptions", "fs::copy", "set_permissions"] {
            assert!(!production.contains(forbidden), "studio must not delete or write a note or an Eclipse workspace: found {forbidden}");
        }
    }
}
