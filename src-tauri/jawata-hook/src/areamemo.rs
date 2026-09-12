//! Sprint 28f Stage 8 D2 — WHICH PACKAGES THIS SESSION HAS ALREADY BEEN TOLD ABOUT.
//!
//! Opening a file in an unfamiliar part of the codebase is worth an orientation; opening
//! the eleventh file in the same package is not. This records what a session has been
//! shown so the area is injected ONCE per package, and it lives beside the editgate's
//! window directory for the same reason that one does: a session's state belongs to the
//! session, on disk, where a hook process that lives for one call can find it.
//!
//! **Per SESSION, not per machine.** A new conversation has seen nothing, and a memo that
//! outlived the session would silently make the feature fire once ever and then never
//! again — the worst version of it, because nothing would say so.
//!
//! **Best effort, and failing OPEN.** An unwritable memo means the area is shown again,
//! which costs a repeated paragraph. An unreadable one that suppressed the injection would
//! cost the orientation itself, with nothing to say why. So every error here reads as "not
//! shown yet": the cost of being wrong lands on repetition rather than on silence.

use std::path::{Path, PathBuf};

/// Where a session's shown-areas live: `$HOME/.claude/jawata-studio/areamemo/<session>`.
///
/// The same shape as the editgate's window path, deliberately — one place under
/// `jawata-studio` holds per-session hook state, and a reader who has found one has found
/// the other.
pub fn memo_dir(home: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("jawata-studio")
        .join("areamemo")
        .join(session_id)
}

/// A package name as a single safe file name.
///
/// A package is dots and identifiers, so nothing in it can escape a directory — but this
/// is a name arriving from a response, and a path assembled from one is where a traversal
/// would live. Every character outside the alphabet a package can use becomes `_`, which
/// makes the mapping total rather than trusting the shape.
fn file_name_for(package: &str) -> String {
    package
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' })
        .collect()
}

/// Has this session already been shown this package?
///
/// A read error is NO — see the module note on failing open.
pub fn already_shown(home: &Path, session_id: &str, package: &str) -> bool {
    memo_dir(home, session_id)
        .join(file_name_for(package))
        .try_exists()
        .unwrap_or(false)
}

/// Record that it has been. Best effort; a failure means it is shown again.
pub fn mark_shown(home: &Path, session_id: &str, package: &str) {
    let dir = memo_dir(home, session_id);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(file_name_for(package)), "1");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("jawata-areamemo-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_package_is_unshown_until_it_is_marked() {
        let home = temp();
        assert!(
            !already_shown(&home, "s1", "com.example"),
            "a fresh session has been shown nothing — without this the mark below proves \
             nothing, because a function answering true always would satisfy it"
        );
        mark_shown(&home, "s1", "com.example");
        assert!(already_shown(&home, "s1", "com.example"), "and now it has");
    }

    #[test]
    fn the_memo_is_per_session_and_per_package() {
        let home = temp();
        mark_shown(&home, "s1", "com.example");

        assert!(
            !already_shown(&home, "s2", "com.example"),
            "a NEW conversation has seen nothing; a memo that outlived the session would \
             make this fire once ever and then never again, with nothing saying so"
        );
        assert!(
            !already_shown(&home, "s1", "com.other"),
            "and a different package is a different orientation"
        );
    }

    #[test]
    fn a_package_name_cannot_walk_out_of_the_memo_directory() {
        let home = temp();
        // Not a package a compiler would produce — but this name arrives in a RESPONSE,
        // and a path assembled from one is exactly where a traversal would live.
        mark_shown(&home, "s1", "../../escaped");

        assert!(
            !home.join("escaped").try_exists().unwrap_or(false),
            "nothing was written outside the memo directory"
        );
        assert!(
            already_shown(&home, "s1", "../../escaped"),
            "and the mapping is total, so the memo still works for it"
        );
    }

    #[test]
    fn an_unwritable_home_reads_as_not_shown_rather_than_shown() {
        // Failing OPEN: the cost of being wrong is a repeated paragraph, never a lost
        // orientation that nothing explains.
        let missing = std::env::temp_dir().join("jawata-areamemo-does-not-exist-at-all");
        let _ = std::fs::remove_dir_all(&missing);

        assert!(
            !already_shown(&missing, "s1", "com.example"),
            "an unreadable memo must not suppress the injection"
        );
    }
}
