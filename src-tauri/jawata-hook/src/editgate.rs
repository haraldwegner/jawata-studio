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

// ---- D1 / D2: the shape of the edit, and the exception that covers one ------------------------

/// The tools whose refusal can be answered by a declared failure.
///
/// A declaration reads `jawata-author: <tool>: <what it did>`. The colon is the signal: without it
/// a reason such as "extract the parser into a new class" is authoring, and a reason that merely
/// STARTS with a tool's name must not be mistaken for a defect report.
pub const DECLARABLE_TOOLS: &[&str] = &[
    "organize_imports",
    "change_method_signature",
    "rename_symbol",
    "data",
    "apply_cleanup",
    "extract",
    "move",
    "inline",
    "hierarchy",
    "refactor_to_pattern",
    "generate",
];

/// What a `jawata-author:` reason says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// Authoring: opens the window, covers no tool-shaped edit.
    Authoring,
    /// Names a tool and a failure: covers exactly ONE tool-shaped edit of that tool.
    Exception { tool: &'static str, failure: String },
    /// Names a tool and no failure. Refused — there is nothing for the record to say.
    Incomplete { tool: &'static str },
}

/// Words a failure must have before it counts as named. A tool name and "broken" is not a report.
pub const MIN_FAILURE_WORDS: usize = 3;

pub fn parse_declaration(reason: &str) -> Declaration {
    let reason = reason.trim();
    let Some((head, tail)) = reason.split_once(':') else {
        return Declaration::Authoring;
    };
    let head = head.trim().to_ascii_lowercase();
    let Some(tool) = DECLARABLE_TOOLS
        .iter()
        .copied()
        .find(|t| head == *t || head.strip_prefix(*t).is_some_and(|rest| rest.starts_with('_') || rest.starts_with('.')))
    else {
        return Declaration::Authoring;
    };
    let failure = tail.trim();
    if failure.split_whitespace().count() >= MIN_FAILURE_WORDS {
        Declaration::Exception { tool, failure: failure.to_string() }
    } else {
        Declaration::Incomplete { tool }
    }
}

fn exception_path(home: &Path, session_id: &str) -> PathBuf {
    window_path(home, &format!("{session_id}.once"))
}

/// Record a one-use exception. Best effort, like the window: an exception that cannot be recorded
/// simply does not open, which is the safe direction.
pub fn open_exception(home: &Path, session_id: &str, tool: &str, failure: &str) {
    let path = exception_path(home, session_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::write(&path, format!("{now}\t{tool}\t{failure}\n"));
}

/// Consume the exception if it is fresh and names this tool. TAKING it is the point: the same
/// declaration cannot cover a second edit.
pub fn take_exception(home: &Path, session_id: &str, tool: &str) -> Option<String> {
    let path = exception_path(home, session_id);
    let body = std::fs::read_to_string(&path).ok()?;
    let mut fields = body.trim_end().splitn(3, '\t');
    let opened: u64 = fields.next()?.parse().ok()?;
    let declared = fields.next()?;
    let failure = fields.next()?.to_string();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(opened) >= WINDOW_TTL_SECS || declared != tool {
        return None;
    }
    // Remove BEFORE answering: if the removal fails the exception must not be usable twice.
    std::fs::remove_file(&path).ok()?;
    Some(failure)
}

/// The text of the refusal: it names the tool and the call, and says how to be covered once.
pub fn shaped_refusal(shape: &crate::editshape::Shape, edit_path: &str) -> String {
    format!(
        "USE {tool} — this edit of {edit_path} is {what}, and {tool} does it: {call}. \
         The window you opened covers authoring; it does not cover an edit a tool performs. \
         If {tool} FAILS on this case, name the failure once: run a Bash command containing \
         'jawata-author: {tool}: <what it did, at least {MIN_FAILURE_WORDS} words>', then make THIS edit — \
         the declaration covers that one edit and no other.",
        tool = shape.tool,
        what = shape.name,
        call = shape.call,
    )
}

/// Judge an edit whose shape a tool performs. `enabled` is the TEST-ONLY toggle for D1's control
/// (the check turned off lets everything through); the shipped switch is S2b's.
pub fn judge_shaped(
    shape: &crate::editshape::Shape,
    edit_path: &str,
    home: Option<&Path>,
    session_id: Option<&str>,
    enabled: bool,
) -> EditVerdict {
    if !enabled {
        return EditVerdict::Allowed("shape check switched off");
    }
    if let (Some(home), Some(session)) = (home, session_id) {
        if take_exception(home, session, shape.tool).is_some() {
            return EditVerdict::Allowed("a declared tool failure, used once");
        }
    }
    EditVerdict::Denied(shaped_refusal(shape, edit_path))
}

/// Debug builds only: the shipped binary has no off switch from the environment.
pub fn shape_check_enabled() -> bool {
    !(cfg!(debug_assertions) && std::env::var("JAWATA_TEST_EDITSHAPE_OFF").is_ok())
}

/// Every (old, new) text pair the edit call carries: `Edit` has one, `MultiEdit` several, and a
/// `Write` has none — a whole file is never a refactoring of itself.
pub fn edit_pairs(payload: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Vec::new();
    };
    let input = value.get("tool_input").unwrap_or(&value);
    let pair = |v: &serde_json::Value| {
        let old = v.get("old_string").and_then(serde_json::Value::as_str)?;
        let new = v.get("new_string").and_then(serde_json::Value::as_str)?;
        Some((old.to_string(), new.to_string()))
    };
    if let Some(edits) = input.get("edits").and_then(serde_json::Value::as_array) {
        return edits.iter().filter_map(pair).collect();
    }
    pair(input).into_iter().collect()
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

    fn visibility_shape() -> crate::editshape::Shape {
        crate::editshape::classify("private int f() {", "public int f() {").expect("a visibility change")
    }

    #[test]
    fn a_declaration_must_carry_the_colon_and_the_failure_to_be_an_exception() {
        assert_eq!(Declaration::Authoring, parse_declaration("adding a new listener class"));
        // Starts with a tool's name but is prose: the colon is the signal.
        assert_eq!(Declaration::Authoring, parse_declaration("extract the parser into a new class"));
        assert_eq!(Declaration::Authoring, parse_declaration("unknown_tool: some words about it"));
        assert!(matches!(
            parse_declaration("extract_method: INTERNAL_ERROR source not specified on one file"),
            Declaration::Exception { tool: "extract", .. }
        ));
        assert_eq!(
            Declaration::Incomplete { tool: "rename_symbol" },
            parse_declaration("rename_symbol: broken"),
            "two words of failure is not a report"
        );
    }

    #[test]
    fn one_exception_covers_one_edit_of_its_own_tool_and_no_other() {
        let dir = std::env::temp_dir().join(format!("jawata-exception-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shape = visibility_shape();

        // No exception: refused, by name.
        match judge_shaped(&shape, "A.java", Some(&dir), Some("s"), true) {
            EditVerdict::Denied(r) => assert!(r.contains(shape.tool) && r.contains(shape.call), "{r}"),
            other => panic!("{other:?}"),
        }
        // An exception for a DIFFERENT tool does not cover it, and is not spent by trying.
        open_exception(&dir, "s", "rename_symbol", "refused to rename across two bundles");
        assert!(matches!(judge_shaped(&shape, "A.java", Some(&dir), Some("s"), true), EditVerdict::Denied(_)));
        assert!(take_exception(&dir, "s", "rename_symbol").is_some(), "still there for its own tool");
        // The right tool: covered once, then refused.
        open_exception(&dir, "s", shape.tool, "the visibility change was refused with no reason");
        assert!(matches!(judge_shaped(&shape, "A.java", Some(&dir), Some("s"), true), EditVerdict::Allowed(_)));
        assert!(matches!(judge_shaped(&shape, "A.java", Some(&dir), Some("s"), true), EditVerdict::Denied(_)));
        // Another session never inherits it.
        open_exception(&dir, "s", shape.tool, "the visibility change was refused with no reason");
        assert!(matches!(judge_shaped(&shape, "A.java", Some(&dir), Some("other"), true), EditVerdict::Denied(_)));
        // The control: the check switched off lets the same edit through.
        assert!(matches!(judge_shaped(&shape, "A.java", Some(&dir), Some("other"), false), EditVerdict::Allowed(_)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_edit_call_carries_its_pairs_in_every_shape_and_a_write_carries_none() {
        let edit = r#"{"tool_input":{"file_path":"A.java","old_string":"a","new_string":"b"}}"#;
        assert_eq!(vec![("a".to_string(), "b".to_string())], edit_pairs(edit));
        let multi = r#"{"tool_input":{"edits":[{"old_string":"a","new_string":"b"},{"old_string":"c","new_string":"d"}]}}"#;
        assert_eq!(2, edit_pairs(multi).len());
        assert!(edit_pairs(r#"{"tool_input":{"file_path":"A.java","content":"x"}}"#).is_empty());
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
