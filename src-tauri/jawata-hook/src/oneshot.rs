//! A record kept for one session that is good for ONE use and for a bounded time.
//!
//! The crate had two private copies of this — the exception that lets one hand edit past the shape
//! check, and the duplicate gate's declaration — and a third was about to be written. They are one
//! mechanism: a small file beside the authoring window, `now<TAB>field<TAB>field…`, that a later
//! call consumes. A record that could be used twice, or that never expired, would be a standing
//! bypass, so both limits live HERE, once, and the callers cannot forget them.
//!
//! Best effort in both directions, and the direction is deliberate: a record that cannot be written
//! simply is not there, which makes the gate it unlocks stay shut, the safe way to fail.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn path_of(home: &Path, session_id: &str, suffix: &str) -> PathBuf {
    crate::editgate::window_path(home, &format!("{session_id}.{suffix}"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Write the record for this session under `suffix`, replacing any earlier one.
pub fn open(home: &Path, session_id: &str, suffix: &str, fields: &[&str]) {
    let path = path_of(home, session_id, suffix);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let clean: Vec<String> = fields.iter().map(|f| f.replace(['\n', '\t', '\r'], " ")).collect();
    let _ = std::fs::write(&path, format!("{}\t{}\n", now(), clean.join("\t")));
}

fn read_fresh(path: &Path) -> Option<Vec<String>> {
    let body = std::fs::read_to_string(path).ok()?;
    let mut parts = body.trim_end().split('\t');
    let opened: u64 = parts.next()?.parse().ok()?;
    if now().saturating_sub(opened) >= crate::editgate::WINDOW_TTL_SECS {
        return None;
    }
    Some(parts.map(str::to_string).collect())
}

/// The record's fields if it exists and is fresh, WITHOUT using it up.
pub fn peek(home: &Path, session_id: &str, suffix: &str) -> Option<Vec<String>> {
    read_fresh(&path_of(home, session_id, suffix))
}

/// Use the record up: returns its fields if it is fresh and `accept` agrees, and removes it.
/// A record `accept` refuses is left in place for the call it was meant for.
pub fn take(
    home: &Path,
    session_id: &str,
    suffix: &str,
    accept: impl Fn(&[String]) -> bool,
) -> Option<Vec<String>> {
    let path = path_of(home, session_id, suffix);
    let fields = read_fresh(&path)?;
    if !accept(&fields) {
        return None;
    }
    // Remove BEFORE answering: if the removal fails the record must not be usable twice.
    std::fs::remove_file(&path).ok()?;
    Some(fields)
}

/// Drop the record if there is one.
pub fn discard(home: &Path, session_id: &str, suffix: &str) {
    let _ = std::fs::remove_file(path_of(home, session_id, suffix));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jawata-oneshot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_record_is_good_for_one_use() {
        let home = scratch("once");
        open(&home, "s", "x", &["a", "b c"]);
        assert_eq!(Some(vec!["a".to_string(), "b c".to_string()]), peek(&home, "s", "x"));
        assert_eq!(Some(vec!["a".to_string(), "b c".to_string()]), take(&home, "s", "x", |_| true));
        assert_eq!(None, take(&home, "s", "x", |_| true), "and not a second");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_record_the_caller_refuses_stays_for_the_call_it_was_meant_for() {
        let home = scratch("refused");
        open(&home, "s", "x", &["rename_symbol"]);
        assert_eq!(None, take(&home, "s", "x", |f| f[0] == "move"), "the wrong tool");
        assert!(take(&home, "s", "x", |f| f[0] == "rename_symbol").is_some(), "still there for its own");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_expired_or_torn_record_is_nothing_and_a_session_sees_only_its_own() {
        let home = scratch("stale");
        let path = path_of(&home, "old", "x");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stale = now() - crate::editgate::WINDOW_TTL_SECS - 5;
        std::fs::write(&path, format!("{stale}\tf\n")).unwrap();
        assert_eq!(None, peek(&home, "old", "x"), "it expires");
        std::fs::write(path_of(&home, "torn", "x"), "not-a-timestamp\tf\n").unwrap();
        assert_eq!(None, peek(&home, "torn", "x"));
        open(&home, "mine", "x", &["f"]);
        assert_eq!(None, peek(&home, "yours", "x"));
        discard(&home, "mine", "x");
        assert_eq!(None, peek(&home, "mine", "x"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
