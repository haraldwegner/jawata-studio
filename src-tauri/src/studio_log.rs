//! Studio's own log file.
//!
//! Studio reports what it does in the background — the release check, the gateway, the
//! knowledge-store reload — with `eprintln!`. Where those lines land was decided by whoever
//! launched the process, and the desktop launcher hands studio a standard error that is
//! `/dev/null`. Measured 2026-09-13: a store reload that never ran left no trace anywhere,
//! because the line saying so went nowhere.
//!
//! So at start-up studio points its own standard error at `studio.log`, beside the
//! residents' logs. Every existing `eprintln!` and every panic message follows without being
//! touched. A studio started from a terminal keeps its terminal: a developer watching the
//! output is already reading it.

use std::fs::{self, File, OpenOptions};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// Past this size the log is moved aside once, at start-up, so it cannot grow without bound.
const ROTATE_AT_BYTES: u64 = 10 * 1024 * 1024;

/// The file studio writes its log to, inside the logs directory.
pub fn log_path(log_dir: &Path) -> PathBuf {
    log_dir.join("studio.log")
}

/// Point standard error at `studio.log` unless it is a terminal. Never fails start-up: a log
/// that cannot be opened leaves standard error where it was.
pub fn install(log_dir: &Path) {
    if std::io::stderr().is_terminal() {
        return;
    }
    if fs::create_dir_all(log_dir).is_err() {
        return;
    }
    let path = log_path(log_dir);
    rotate_if_large(&path, ROTATE_AT_BYTES);
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    if redirect_stderr(file) {
        eprintln!(
            "[jawata-studio] {} jawata-studio {} started; this file is its log",
            utc_stamp(crate::field_view::now_millis()),
            env!("CARGO_PKG_VERSION")
        );
    }
}

/// Move a log past `limit` bytes to `<name>.1`, replacing an older one. Best effort.
fn rotate_if_large(path: &Path, limit: u64) {
    let too_big = fs::metadata(path).map(|m| m.len() > limit).unwrap_or(false);
    if too_big {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
}

#[cfg(unix)]
fn redirect_stderr(file: File) -> bool {
    use std::os::fd::AsRawFd;
    // SAFETY: dup2 onto descriptor 2 with a descriptor this process owns. On success
    // descriptor 2 is an independent duplicate, so dropping `file` afterwards is fine.
    unsafe { libc::dup2(file.as_raw_fd(), 2) != -1 }
}

#[cfg(windows)]
fn redirect_stderr(file: File) -> bool {
    use std::os::windows::io::IntoRawHandle;
    let handle = file.into_raw_handle();
    // SAFETY: the handle is a valid, owned file handle, deliberately leaked so it stays
    // open for the life of the process as the standard error stream.
    unsafe {
        windows_sys::Win32::System::Console::SetStdHandle(
            windows_sys::Win32::System::Console::STD_ERROR_HANDLE,
            handle as _,
        ) != 0
    }
}

#[cfg(not(any(unix, windows)))]
fn redirect_stderr(_file: File) -> bool {
    false
}

/// `YYYY-MM-DDTHH:MM:SSZ` for milliseconds since the Unix epoch, in UTC.
pub fn utc_stamp(millis: u64) -> String {
    let secs = millis / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamp_renders_known_instants() {
        assert_eq!(utc_stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_stamp(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(utc_stamp(1_789_332_824_173), "2026-09-13T20:53:44Z");
        assert_eq!(utc_stamp(4_107_542_399_000), "2100-02-28T23:59:59Z");
    }

    #[test]
    fn a_large_log_is_moved_aside_and_a_small_one_is_left() {
        let dir = std::env::temp_dir().join(format!("studio-log-rotate-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = log_path(&dir);
        fs::write(&path, vec![b'x'; 64]).unwrap();
        rotate_if_large(&path, 100);
        assert!(path.exists(), "under the limit: left in place");
        rotate_if_large(&path, 10);
        assert!(!path.exists(), "over the limit: moved aside");
        assert!(dir.join("studio.log.1").exists(), "to studio.log.1");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The redirect itself, exercised for real. It changes the process's own standard
    /// error, so it runs in a CHILD copy of this test binary whose standard error is not a
    /// terminal — the launcher's situation — and the parent reads the file.
    #[test]
    fn a_child_whose_stderr_goes_nowhere_writes_its_lines_to_studio_log() {
        const CHILD: &str = "JAWATA_STUDIO_LOG_CHILD_DIR";
        if let Ok(dir) = std::env::var(CHILD) {
            install(Path::new(&dir));
            eprintln!("marker-from-the-child");
            return;
        }
        let dir = std::env::temp_dir().join(format!("studio-log-child-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "studio_log::tests::a_child_whose_stderr_goes_nowhere_writes_its_lines_to_studio_log",
                "--nocapture",
            ])
            .env(CHILD, &dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "the child run must pass: {status}");
        let written = fs::read_to_string(log_path(&dir)).unwrap_or_default();
        assert!(
            written.contains("marker-from-the-child"),
            "a line printed to standard error must land in studio.log, got: {written:?}"
        );
        assert!(written.contains("started; this file is its log"), "with its start line");
        let _ = fs::remove_dir_all(&dir);
    }
}
