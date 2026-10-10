//! A script says for itself that it is a long gate.
//!
//! Until this module the guard shipped a list of OUR OWN build-script names, and
//! matched it by substring over the whole command. Measured 2026-10-06: it
//! refused seven correct commands in one session, none of which ran a gate — a
//! `sed` reading the lines that hold the names, a `grep` counting them in the
//! binary, a commit message describing the rule. A list of names is also the
//! wrong owner: it makes a published binary know about one team's files, and it
//! goes stale the day a script is renamed, with nothing failing.
//!
//! The owner of "this script is long" is the script. It carries one line in its
//! own header, and the hook READS it, at the moment the command RUNS it:
//!
//! ```text
//! # jawata-gate: long     <- takes minutes; its output must be kept
//! # jawata-gate: suite    <- the whole-suite runner; also subject to the cadence rule
//! ```
//!
//! Two properties follow, and they are the point:
//!
//! * **Invoked, never mentioned.** Only the program a segment RUNS is considered
//!   (past wrappers and assignments), so a script named as an argument to `sed`,
//!   `grep` or `echo` is never read and never refused.
//! * **A script that cannot be read is not refused.** Not found, unreadable, no
//!   marker: the hook cannot tell, and "cannot tell" is not "long". The old rule
//!   answered the same case with a refusal, which is how it refused commands that
//!   ran nothing.
//!
//! What it gives up, stated: a long script that does not carry the marker is
//! unguarded until it does. The marker is one comment line.

use std::path::{Path, PathBuf};

/// The token a script's header carries: `# jawata-gate: long` or `suite`.
pub const MARKER: &str = "jawata-gate:";

/// How many header lines are read. A declaration is a header comment; reading
/// further would make a mention in the body count.
const HEADER_LINES: usize = 30;

/// A script larger than this is not a shell script anyone wrote a header on.
const MAX_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    /// Takes minutes; the output of a run must be kept.
    Long,
    /// The whole-suite runner: long, and also subject to the cadence rule.
    Suite,
}

/// Interpreters whose first non-flag argument is the script they run.
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash"];

/// Wrappers that run what follows them.
const WRAPPERS: &[&str] = &["sudo", "env", "time", "nice", "command", "nohup", "exec", "npx"];

fn basename(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// The words of a segment past leading wrappers and `NAME=value` assignments.
fn command_words(segment: &str) -> Vec<&str> {
    segment
        .split_whitespace()
        .skip_while(|w| {
            WRAPPERS.contains(&basename(w)) || (w.contains('=') && !w.starts_with('-') && !w.contains('/'))
        })
        .collect()
}

/// The script a segment RUNS, as written — `None` when it runs something that
/// is not a script path, or an inline program (`bash -c ...`).
pub fn script_run_by(segment: &str) -> Option<String> {
    let words = command_words(segment);
    let head = *words.first()?;
    if SHELLS.contains(&basename(head)) {
        let script = words.iter().skip(1).find(|w| !w.starts_with('-'))?;
        // `bash -c "..."` carries a program, not a path; the first non-flag
        // word there is that program's text.
        if words.iter().skip(1).any(|w| *w == "-c") {
            return None;
        }
        return Some(script.trim_matches(['"', '\'']).to_string());
    }
    if head.contains('/') || head.ends_with(".sh") {
        return Some(head.trim_matches(['"', '\'']).to_string());
    }
    None
}

/// What the script's own header says it is, resolved against `base`.
pub fn declared(script: &str, base: &Path) -> Option<GateKind> {
    let path = if Path::new(script).is_absolute() {
        PathBuf::from(script)
    } else {
        base.join(script)
    };
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    for line in text.lines().take(HEADER_LINES) {
        let line = line.trim();
        if !line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.split(MARKER).nth(1) {
            return match rest.split_whitespace().next() {
                Some("long") => Some(GateKind::Long),
                Some("suite") => Some(GateKind::Suite),
                _ => None,
            };
        }
    }
    None
}

/// A bracket call, seen through: the directory it runs in and the command it wraps.
///
/// The bracket must not become a way to hide a run from the rules that police WHAT is
/// run. Its head is the hook binary, so without this a whole-suite run wrapped in it
/// would pass the cadence gate unseen — the capture rule is satisfied by the bracket,
/// and the cadence rule would never learn there was a suite inside. The directory is
/// the bracket's own `--cd`, which is also where a relative script inside is read from.
pub fn bracket_inner(segment: &str) -> Option<(PathBuf, String)> {
    let words = command_words(segment);
    let head = basename(words.first()?);
    if !head.starts_with("jawata-hook") || words.get(1) != Some(&crate::bracket::SUBCOMMAND) {
        return None;
    }
    let dir = words
        .iter()
        .position(|w| *w == "--cd")
        .and_then(|i| words.get(i + 1))
        .map(|d| PathBuf::from(d.trim_matches(['"', '\''])))?;
    let split = words.iter().position(|w| *w == "--")?;
    let inner = words[split + 1..].join(" ");
    (!inner.is_empty()).then_some((dir, inner))
}

/// Whether the program a segment runs is a tool that is long by what it IS —
/// the build tools themselves, not our scripts.
fn is_long_tool(segment: &str) -> bool {
    let words = command_words(segment);
    let Some(head) = words.first().map(|w| basename(w)) else {
        return false;
    };
    let second = words.get(1).copied().unwrap_or("");
    head == "mvn"
        || head == "mvnw"
        || head == "svelte-check"
        || (head == "cargo" && (second == "test" || second == "build"))
        || (head == "vite" && second == "build")
}

/// Whether this segment RUNS a long gate: a long tool, or a script that declares itself one.
pub fn runs_long_gate(segment: &str, base: &Path) -> bool {
    if is_long_tool(segment) {
        return true;
    }
    script_run_by(segment).is_some_and(|s| declared(&s, base).is_some())
}

/// The script this segment runs when it declares itself the whole-suite runner.
pub fn suite_script_run_by(segment: &str, base: &Path) -> Option<String> {
    let script = script_run_by(segment)?;
    (declared(&script, base) == Some(GateKind::Suite)).then_some(script)
}

/// The directory a segment begins in, given the one before it: `cd X` moves it.
pub fn dir_after(segment: &str, base: &Path) -> PathBuf {
    let words = command_words(segment);
    if words.first().map(|w| basename(w)) == Some("cd") {
        if let Some(target) = words.get(1) {
            let target = target.trim_matches(['"', '\'']);
            let p = Path::new(target);
            return if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
        }
    }
    base.to_path_buf()
}

/// Where the command starts: the hook's own working directory, which a client
/// sets to the session's.
pub fn start_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
pub(crate) mod testkit {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// A scratch directory holding a script with the given header line.
    pub fn script_with(header: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "jawata-gatefile-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "gate.sh".to_string();
        std::fs::write(dir.join(&name), format!("#!/bin/bash\n{header}\necho hi\n")).unwrap();
        (dir, name)
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::script_with;
    use super::*;

    #[test]
    fn a_script_declares_itself_in_its_header() {
        let (dir, name) = script_with("# jawata-gate: long");
        assert_eq!(Some(GateKind::Long), declared(&name, &dir));
        let (dir, name) = script_with("# jawata-gate: suite");
        assert_eq!(Some(GateKind::Suite), declared(&name, &dir));
        let (dir, name) = script_with("# an ordinary header");
        assert_eq!(None, declared(&name, &dir), "no marker, no declaration");
    }

    #[test]
    fn a_marker_in_the_body_is_not_a_declaration() {
        let dir = std::env::temp_dir().join(format!("jawata-gatefile-body-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = String::from("#!/bin/bash\n");
        for _ in 0..HEADER_LINES {
            body.push_str("true\n");
        }
        body.push_str("# jawata-gate: long\n");
        std::fs::write(dir.join("late.sh"), body).unwrap();
        assert_eq!(None, declared("late.sh", &dir));
    }

    #[test]
    fn what_cannot_be_read_is_not_a_gate() {
        let dir = std::env::temp_dir();
        assert_eq!(None, declared("no-such-script-anywhere.sh", &dir));
        assert!(!runs_long_gate("./no-such-script-anywhere.sh 4", &dir));
    }

    #[test]
    fn the_script_a_segment_runs_is_found_past_wrappers_and_shells() {
        assert_eq!(Some("./a/b.sh".into()), script_run_by("./a/b.sh 4"));
        assert_eq!(Some("./a/b.sh".into()), script_run_by("FOO=1 nohup ./a/b.sh 4"));
        assert_eq!(Some("a/b.sh".into()), script_run_by("bash a/b.sh --x"));
        assert_eq!(Some("a/b.sh".into()), script_run_by("bash -x a/b.sh"));
        assert_eq!(None, script_run_by("bash -c 'echo hi'"), "an inline program is not a path");
        assert_eq!(None, script_run_by("sed -n 1,5p ./a/b.sh"), "a script named as an argument is not run");
        assert_eq!(None, script_run_by("grep -c gate ./a/b.sh"));
        assert_eq!(None, script_run_by("echo ./a/b.sh"));
    }

    #[test]
    fn a_cd_moves_the_base_the_next_script_is_read_from() {
        let (dir, name) = script_with("# jawata-gate: long");
        let elsewhere = std::env::temp_dir();
        let cd = format!("cd {}", dir.display());
        let moved = dir_after(&cd, &elsewhere);
        assert_eq!(dir, moved);
        assert!(runs_long_gate(&format!("./{name}"), &moved));
        assert!(!runs_long_gate(&format!("./{name}"), &elsewhere), "same script, other directory");
    }

    #[test]
    fn a_bracket_call_is_seen_through_to_what_it_runs() {
        let (dir, inner) = bracket_inner(
            "/h/.claude/jawata-hook-guard bracket --cd /repo --out /tmp/o.txt -- ./a.sh 4 --full",
        )
        .expect("a bracket call");
        assert_eq!(PathBuf::from("/repo"), dir);
        assert_eq!("./a.sh 4 --full", inner);
        assert!(bracket_inner("./a.sh 4 --full").is_none());
        assert!(bracket_inner("jawata-hook-guard bracket --cd /repo --out /o --").is_none(), "nothing wrapped");
        assert!(bracket_inner("echo jawata-hook-guard bracket --cd /r -- x").is_none(), "named, not run");
    }

    #[test]
    fn the_build_tools_are_long_by_what_they_are() {
        let d = std::env::temp_dir();
        for cmd in ["mvn -q install", "./mvnw verify", "cargo test --lib", "cargo build", "vite build", "svelte-check", "time mvn test"] {
            assert!(runs_long_gate(cmd, &d), "{cmd}");
        }
        for cmd in ["echo mvn", "grep mvn pom.xml", "cargo fmt", "cargo --version", "vite dev"] {
            assert!(!runs_long_gate(cmd, &d), "{cmd}");
        }
    }
}
