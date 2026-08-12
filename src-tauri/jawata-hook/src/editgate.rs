//! The `.java` hand-edit gate — the half of the guard that lived only in bash.
//!
//! # Why this file exists
//!
//! Sprint 28 replaced ten hook scripts with one binary, and the guard was
//! reverted to its script generation almost immediately: the 3.7.3 dogfood
//! caught a front-door `Edit` of a `.java` file going through **unblocked**,
//! because the binary read a shell command out of the payload and never looked
//! at *which tool* had fired. `BINARY_RETIRED_ROLES` has held the guard back
//! ever since, which is why Cursor's four hooks are still shell scripts.
//!
//! That mattered more than it looked. On Windows a `.sh` cannot run at all:
//! the hook is launched as an interactive login shell, its first act is to read
//! its payload from standard input with `cat`, no payload is piped in, and it
//! waits forever in a visible window. Under Cursor's `failClosed: true` a hook
//! that never returns **blocks the user's command** — a guard strictly worse
//! than no guard. Observed live on 2026-08-12.
//!
//! So this module is not a refactor. It is the missing half, and it is what
//! lets the guard role go binary-live, the Cursor scripts be deleted, and
//! Windows need no shell.
//!
//! # The rules, ported from the script generation
//!
//! 1. A `.java` hand-edit through `Edit`/`Write`/`MultiEdit` is **denied** and
//!    redirected to jawata's refactoring tools.
//! 2. A declared `jawata-fallback:` proceeds — the declaration is the audit
//!    trail, not a bypass.
//! 3. **Authoring is not refactoring.** Writing new Java is not a restructuring
//!    jawata can express, and no text-level hook can tell the two apart (that
//!    judgement needs the AST). So a `Bash` command declaring `jawata-author:`
//!    opens a short, session-scoped window; `.java` edits inside it pass and are
//!    logged. The window is TTL-bounded so it cannot become a standing bypass.
//! 4. A `Write` to a path that does **not exist** passes: a brand-new file has
//!    nothing to refactor.
//!
//! Rule 4 is the one that keeps this honest. Without it the gate would block
//! the creation of every new Java file in the project, which is authoring by
//! definition.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long an authoring window stays open. Matches the script generation.
///
/// Bounded on purpose: a window that never expired would be a permanent
/// bypass one `jawata-author:` away, which is the opposite of an audit trail.
pub const WINDOW_TTL_SECS: u64 = 1800;

/// The tools that write source, and therefore pass through this gate.
const EDITING_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit"];

/// What the gate decided, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditVerdict {
    /// Not our business — not an editing tool, or not a `.java` path.
    NotApplicable,
    /// Allowed, with the reason for the audit trail.
    Allowed(&'static str),
    /// Denied, with the text the model and the human are shown.
    Denied(String),
}

/// Whether this tool writes source.
pub fn is_editing_tool(tool_name: &str) -> bool {
    EDITING_TOOLS.iter().any(|t| t.eq_ignore_ascii_case(tool_name))
}

/// Whether the path names a Java source file.
///
/// Deliberately narrow — the extension, case-insensitively. A path merely
/// *mentioning* `.java` (a log file, a directory called `java`) is not a Java
/// source file, and over-matching here blocks work the gate was never meant to
/// touch.
pub fn is_java_source(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("java"))
        .unwrap_or(false)
}

/// Where a session's authoring window lives.
///
/// Mirrors the script generation exactly (`$HOME/.claude/jawata-studio/editgate/<session>`)
/// so a window opened under either generation is honoured by the other — the
/// cutover must not silently close a window the user already declared.
pub fn window_path(home: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("jawata-studio")
        .join("editgate")
        .join(session_id)
}

/// Open an authoring window for this session.
///
/// Best effort by design: a window that cannot be recorded must not fail the
/// user's command. The cost of not writing it is one more declaration later,
/// which is an inconvenience; the cost of failing here is a blocked command.
pub fn open_window(home: &Path, session_id: &str, reason: &str) {
    let path = window_path(home, session_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(&path, format!("{now}\t{reason}\n"));
}

/// Whether a window is open AND still fresh.
///
/// A missing file, an unreadable one, or a malformed timestamp all mean NO
/// WINDOW — the gate then applies. Failing closed is right *here* and wrong in
/// the guard as a whole: this decides whether to relax a rule, so an unreadable
/// state must not relax it.
pub fn window_is_open(home: &Path, session_id: &str) -> bool {
    let path = window_path(home, session_id);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Some(stamp) = body.split('\t').next() else {
        return false;
    };
    let Ok(opened) = stamp.trim().parse::<u64>() else {
        return false;
    };
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return false,
    };
    now.saturating_sub(opened) < WINDOW_TTL_SECS
}

/// Rule on one editing-tool call.
///
/// `path_exists` is injected rather than probed here so the decision is
/// testable without touching the filesystem — the new-file allowance is the
/// branch most likely to regress silently, and a test that cannot exercise it
/// is how the previous generation shipped half a guard.
pub fn judge_edit(
    tool_name: &str,
    edit_path: &str,
    payload: &str,
    window_open: bool,
    path_exists: bool,
) -> EditVerdict {
    if !is_editing_tool(tool_name) || !is_java_source(edit_path) {
        return EditVerdict::NotApplicable;
    }
    // The declared escape, same as the shell half.
    if payload.contains(crate::guard::FALLBACK_DECLARATION) {
        return EditVerdict::Allowed("declared jawata-fallback");
    }
    // Authoring new code is not a refactor.
    if window_open {
        return EditVerdict::Allowed("inside a declared authoring window");
    }
    // A brand-new file has nothing to refactor.
    if tool_name.eq_ignore_ascii_case("Write") && !path_exists {
        return EditVerdict::Allowed("new file — nothing to refactor");
    }
    EditVerdict::Denied(format!(
        "USE A JAWATA REFACTOR TOOL — hand-editing {edit_path} (a .java file) is blocked. \
         Renaming, moving, extracting or changing a signature by hand misses references; \
         rename_symbol / move / extract / change_method_signature do not. \
         Authoring NEW code rather than restructuring? Declare a window: run a Bash command \
         containing 'jawata-author: <reason>', then edit (session-scoped, logged). \
         Or declare 'jawata-fallback: <why>' on this call; the declaration is the audit trail."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_java_hand_edit_is_denied_and_told_where_to_go() {
        match judge_edit("Edit", "/p/src/Main.java", "{}", false, true) {
            EditVerdict::Denied(reason) => {
                assert!(reason.contains("rename_symbol"), "names the tools: {reason}");
                assert!(reason.contains("jawata-author:"), "names the authoring escape");
                assert!(reason.contains("jawata-fallback:"), "names the fallback escape");
            }
            other => panic!("a .java hand-edit must be denied, got {other:?}"),
        }
    }

    #[test]
    fn every_editing_tool_is_covered() {
        for tool in ["Edit", "Write", "MultiEdit", "edit", "WRITE"] {
            // `path_exists: true` so the new-file allowance cannot mask a gap.
            assert!(
                matches!(judge_edit(tool, "A.java", "{}", false, true), EditVerdict::Denied(_)),
                "{tool} must reach the gate"
            );
        }
    }

    #[test]
    fn non_java_and_non_editing_calls_are_not_our_business() {
        assert_eq!(EditVerdict::NotApplicable, judge_edit("Edit", "notes.md", "{}", false, true));
        assert_eq!(EditVerdict::NotApplicable, judge_edit("Read", "A.java", "{}", false, true));
        // A path that merely mentions java is not a Java source file.
        assert_eq!(EditVerdict::NotApplicable, judge_edit("Edit", "/java/notes.txt", "{}", false, true));
    }

    #[test]
    fn a_brand_new_file_passes_because_there_is_nothing_to_refactor() {
        assert!(matches!(
            judge_edit("Write", "New.java", "{}", false, false),
            EditVerdict::Allowed(_)
        ));
        // ...but EDITING a file that exists is still a hand-edit.
        assert!(matches!(
            judge_edit("Edit", "New.java", "{}", false, false),
            EditVerdict::Denied(_)
        ));
    }

    #[test]
    fn a_declared_window_covers_the_edit_and_an_expired_one_does_not() {
        assert!(matches!(
            judge_edit("Edit", "A.java", "{}", true, true),
            EditVerdict::Allowed(_)
        ));
        assert!(matches!(
            judge_edit("Edit", "A.java", "{}", false, true),
            EditVerdict::Denied(_)
        ));
    }

    #[test]
    fn a_declared_fallback_proceeds() {
        assert!(matches!(
            judge_edit("Edit", "A.java", r#"{"x":"jawata-fallback: porting a fixture"}"#, false, true),
            EditVerdict::Allowed(_)
        ));
    }

    #[test]
    fn a_window_expires_and_a_missing_or_torn_one_never_opens() {
        let dir = std::env::temp_dir().join(format!("jawata-editgate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        assert!(!window_is_open(&dir, "no-such-session"), "a missing window is closed");

        open_window(&dir, "s1", "adding a new class");
        assert!(window_is_open(&dir, "s1"), "a fresh window is open");

        // Expired: stamp it well past the TTL.
        let stale = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
            - WINDOW_TTL_SECS
            - 60;
        std::fs::write(window_path(&dir, "s2"), format!("{stale}\told\n")).unwrap();
        assert!(!window_is_open(&dir, "s2"), "an expired window is closed");

        // Torn: unparseable stamp must NOT relax the gate.
        std::fs::write(window_path(&dir, "s3"), "not-a-timestamp\tx\n").unwrap();
        assert!(!window_is_open(&dir, "s3"), "a torn window is closed");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
