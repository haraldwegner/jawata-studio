//! studio#43 — detect a shell-mediated `.java` write by its EFFECT.
//!
//! The shell tripwire in `guard.rs` reads an opaque command string and infers a
//! run-time effect. The path can arrive from a variable, a substitution, a
//! glob, a file list, or a script one level down, so three patches each closed
//! a set of spellings and each moved the hole — v3.8.1 shipped two NEW
//! regressions doing it. That tripwire is frozen at four measured passing
//! shapes; this is the other half of the C9 ruling on studio#5.
//!
//! **THIS CANNOT PREVENT, and saying so is part of it.** It runs AFTER the
//! command. A detector presented as a gate would be the same over-claim that
//! made the tripwire look patchable three times. What it does is convert a
//! silent miss into a recorded one: all four guard defects to date were found
//! by ad-hoc dogfooding and none by a standing detector, so each recorded miss
//! supplies the next spelling from ground truth instead of from imagination.
//!
//! It cannot be defeated by novel syntax, because its input is which files
//! changed rather than what the command said.
//!
//! **NO BOUND MEANS NO CLAIM.** "Changed by this command" needs a lower bound,
//! and the only honest one is a stamp the PRE hook wrote for this very command.
//! Without it the detector records nothing rather than reporting every `.java`
//! file some other process touched — an instrument that fabricates findings when
//! it cannot see is worse than one that stays quiet, because the findings are
//! the whole product here.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the PRE hook records when the current shell command started.
///
/// Beside the edit gate's own per-session state and keyed the same way, because
/// it answers about the same session and is thrown away on the same schedule.
pub fn stamp_path(home: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("jawata-studio")
        .join("editgate")
        .join(format!("{session_id}.bash-start"))
}

/// The ledger a miss lands in, beside the observer's own `outcomes.log`.
pub fn ledger_path(home: &Path) -> PathBuf {
    home.join(".claude").join("jawata-studio").join("java-write-misses.log")
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Record that a shell command is about to run. Best effort: a stamp that
/// cannot be written costs one unobserved command, and failing the user's
/// command to record it would be the wrong trade in a role that only watches.
pub fn stamp_command_start(home: &Path, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let path = stamp_path(home, session_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, now_millis().to_string());
}

/// The lower bound for "changed by this command", or `None` when there is none.
pub fn command_started_at(home: &Path, session_id: &str) -> Option<u64> {
    std::fs::read_to_string(stamp_path(home, session_id)).ok()?.trim().parse().ok()
}

/// Every `.java` file under `root` modified at or after `since` (millis).
///
/// Bounded by `MAX_DEPTH` and skipping the directories a build fills, because
/// this runs after a shell command and a walk that costs a person a noticeable
/// pause would be paid on every one of them. Measured on the largest repository
/// in this workspace — 1549 `.java` files — at about 3 ms.
pub fn java_files_changed_since(root: &Path, since: u64) -> Vec<PathBuf> {
    const MAX_DEPTH: usize = 12;
    const SKIP: &[&str] = &["target", "build", "bin", "node_modules", ".git", "out"];
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !SKIP.contains(&name.as_ref()) && !name.starts_with('.') {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("java") {
                continue;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if modified >= since {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// One recorded miss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss {
    /// The file the command changed.
    pub path: String,
    /// What ran, clipped — the ROUTE, which is the thing the next spelling of
    /// the tripwire has to recognise.
    pub route: String,
}

/// How much of the command to keep. Enough to name the route; not so much that
/// a heredoc's whole body lands in a ledger somebody reads by eye.
pub const ROUTE_CLIP: usize = 200;

/// The decision, and it is the whole content of this module.
///
/// A changed `.java` file is a MISS when the session had no authoring window
/// open — that is what "the guard should have caught this and did not" means.
/// Inside a window the write is declared and expected, so reporting it would
/// make the ledger a list of every Java edit and the real misses unfindable
/// inside it.
pub fn misses(changed: &[PathBuf], window_open: bool, command: &str) -> Vec<Miss> {
    if window_open {
        return Vec::new();
    }
    let route: String = command.chars().take(ROUTE_CLIP).collect();
    changed
        .iter()
        .map(|path| Miss { path: path.display().to_string(), route: route.clone() })
        .collect()
}

/// Append the misses to the ledger. Best effort, for the reason the stamp is.
pub fn record(home: &Path, misses: &[Miss]) {
    if misses.is_empty() {
        return;
    }
    let path = ledger_path(home);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) =
        std::fs::OpenOptions::new().create(true).append(true).open(&path)
    else {
        return;
    };
    use std::io::Write as _;
    for miss in misses {
        let line = format!(
            "{}\t{}\t{}\n",
            now_millis(),
            miss.path,
            miss.route.replace(['\n', '\t'], " ")
        );
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jawata-javawatch-{label}-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// THE MEASURE, both halves. A `.java` write by a route the text tripwire
    /// does not catch produces a recorded miss with the file and the route —
    /// and one inside a declared window produces none, so this is not merely
    /// counting every Java edit.
    #[test]
    fn a_write_outside_a_window_is_a_miss_and_one_inside_is_not() {
        let root = scratch("effect");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let java = src.join("Touched.java");

        // The bound: the command starts, THEN the file changes.
        let started = now_millis();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&java, "class Touched {}").unwrap();

        // A route the tripwire's own header names as one it does not catch:
        // the path arrives from a variable.
        let command = "F=src/Touched.java; printf 'class Touched {}' > \"$F\"";
        let changed = java_files_changed_since(&root, started);
        assert_eq!(vec![java.clone()], changed, "the EFFECT is what is read");

        let outside = misses(&changed, false, command);
        assert_eq!(1, outside.len());
        assert_eq!(java.display().to_string(), outside[0].path, "the file");
        assert!(outside[0].route.contains("printf"), "and the ROUTE: {}", outside[0].route);

        // INSIDE a declared window the same write is expected, and a ledger that
        // listed it would bury the misses among every ordinary Java edit.
        assert!(misses(&changed, true, command).is_empty(), "a declared write is not a miss");
    }

    #[test]
    fn no_bound_means_no_claim() {
        let home = scratch("no-bound");
        assert_eq!(None, command_started_at(&home, "s1"), "nothing was stamped");
        stamp_command_start(&home, "s1");
        let at = command_started_at(&home, "s1").expect("stamped now");
        assert!(at > 0);
        // An empty session id is not a session, and stamping under one would
        // hand every session the same bound.
        stamp_command_start(&home, "");
        assert_eq!(None, command_started_at(&home, ""));
    }

    #[test]
    fn a_file_that_predates_the_command_is_not_its_doing() {
        let root = scratch("predates");
        let java = root.join("Old.java");
        std::fs::write(&java, "class Old {}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let started = now_millis();

        assert!(
            java_files_changed_since(&root, started).is_empty(),
            "a command cannot be blamed for a file older than it"
        );
    }

    #[test]
    fn the_walk_skips_what_a_build_fills_and_reads_what_it_should() {
        let root = scratch("walk");
        for dir in ["src", "target", "node_modules", ".git"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("A.java"), "class A {}").unwrap();
        }
        let found = java_files_changed_since(&root, 0);
        assert_eq!(
            vec![root.join("src").join("A.java")],
            found,
            "a generated tree is not the user's source: {found:?}"
        );
    }

    #[test]
    fn the_ledger_keeps_one_line_per_miss_and_never_breaks_it() {
        let home = scratch("ledger");
        record(
            &home,
            &[Miss {
                path: "/w/A.java".into(),
                // A newline or a tab in the route would forge a second column
                // or a second row in a file read by line.
                route: "cat <<'EOF' > A.java\nclass A {}\nEOF".into(),
            }],
        );
        let body = std::fs::read_to_string(ledger_path(&home)).unwrap();
        assert_eq!(1, body.lines().count(), "one miss is one line: {body:?}");
        assert_eq!(3, body.trim_end().split('\t').count(), "three columns");
        assert!(body.contains("/w/A.java"));

        // And nothing is written when there is nothing to say.
        let quiet = scratch("ledger-quiet");
        record(&quiet, &[]);
        assert!(!ledger_path(&quiet).exists(), "an empty run leaves no file");
    }
}
