//! The guard: a LOCAL policy decision, taken without asking anyone.
//!
//! It denies shell text-search over Java sources and steers to JAWATA's
//! compiler-accurate tools. Two properties make it different from every other
//! role, and both are deliberate:
//!
//! * **It never queries the store.** It has to answer while the resident is
//!   down, and a guard that asked and failed open would leak exactly the calls
//!   it exists to deny.
//! * **On Cursor it runs under `failClosed: true`**, so a crash or a timeout
//!   is itself a block on the user's command. That is why the fail-safe
//!   boundary matters more here than anywhere else — and why the DEFAULT is
//!   allow: a guard that cannot decide must not invent a denial.
//!
//! The escape hatch is deliberate and audited: a command declaring
//! `jawata-fallback:` proceeds. Being inconvenient is the point; being
//! impossible is not.

/// The declaration that lets a justified shell fallback through. It is logged
/// by the observer, which is what makes it an audit trail rather than a
/// bypass.
pub const FALLBACK_DECLARATION: &str = "jawata-fallback:";

/// The declaration that lets a justified hand-edit of a `.java` file through.
pub const AUTHOR_DECLARATION: &str = "jawata-author:";

/// Text-search tools whose use over Java sources is what we redirect.
const TEXT_TOOLS: &[&str] = &["grep", "rg", "sed", "awk", "ack", "ag"];

/// The verdict, with the reason the model is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny { reason: String },
}

/// Decide on one shell command.
pub fn judge(command: &str) -> Verdict {
    // A declared fallback proceeds — and the declaration IS the audit trail.
    if command.contains(FALLBACK_DECLARATION) || command.contains(AUTHOR_DECLARATION) {
        return Verdict::Allow;
    }
    if !mentions_java_source(command) {
        return Verdict::Allow;
    }
    let Some(tool) = text_tool_in(command) else {
        return Verdict::Allow;
    };
    Verdict::Deny {
        reason: format!(
            "Shell text search on .java is blocked — `{tool}` cannot see symbols, so it \
             misses references and overmatches names. Call search_symbols / find_references / \
             get_call_hierarchy via JAWATA MCP instead. If JAWATA genuinely cannot answer this \
             one, re-run with `{FALLBACK_DECLARATION} <why>` in the command; the declaration is \
             logged."
        ),
    }
}

/// Whether the command names Java sources. Deliberately narrow: `.java` as a
/// path or a glob. Widening this is how a guard starts denying things nobody
/// asked it to.
fn mentions_java_source(command: &str) -> bool {
    command.contains(".java")
}

/// Wrappers that precede the real command without being it.
const PREFIXES: &[&str] = &["sudo", "env", "time", "nice", "command", "xargs", "nohup"];

/// Which text tool the command INVOKES.
///
/// Checked in COMMAND POSITION — the first word of a pipeline segment, past
/// any wrapper — not anywhere in the line. Scanning the whole line denies
/// `echo 'do not grep the Foo.java file'`, which a test caught; a guard that
/// fires on substrings inside quoted text is a guard people turn off, and
/// under Cursor's `failClosed` a false denial blocks real work.
///
/// Deliberately NOT a shell parser. The cost of the simple rule is a miss on
/// exotic shapes (a tool invoked via `$VAR`, or inside `$(…)`), and a miss is
/// the safe direction: the model is steered, not policed, and the real
/// enforcement is that JAWATA answers better.
fn text_tool_in(command: &str) -> Option<&'static str> {
    for segment in command.split(['|', ';', '\n']) {
        let mut words = segment
            .split_whitespace()
            .skip_while(|w| {
                // Wrappers, and leading VAR=value assignments.
                let bare = w.rsplit(['/', '\\']).next().unwrap_or(w);
                PREFIXES.contains(&bare) || (w.contains('=') && !w.starts_with('-'))
            });
        if let Some(first) = words.next() {
            let word = first.rsplit(['/', '\\']).next().unwrap_or(first);
            if let Some(tool) = TEXT_TOOLS.iter().find(|t| **t == word) {
                return Some(tool);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied(cmd: &str) -> bool {
        matches!(judge(cmd), Verdict::Deny { .. })
    }

    #[test]
    fn a_java_grep_is_denied_and_told_where_to_go() {
        match judge("grep -rn 'foo' src/main/java/Thing.java") {
            Verdict::Deny { reason } => {
                assert!(reason.contains("search_symbols"), "it must name the alternative");
                assert!(reason.contains(FALLBACK_DECLARATION), "and the escape hatch");
            }
            other => panic!("expected a denial: {other:?}"),
        }
    }

    #[test]
    fn every_text_tool_is_covered_over_java() {
        for tool in TEXT_TOOLS {
            assert!(denied(&format!("{tool} pattern Foo.java")), "{tool} slipped through");
            assert!(
                denied(&format!("cat x | /usr/bin/{tool} -n 'x' Foo.java")),
                "{tool} slipped through when path-qualified or piped"
            );
        }
    }

    #[test]
    fn a_declared_fallback_proceeds_because_the_declaration_is_the_audit_trail() {
        assert!(!denied("jawata-fallback: reading a fixture with no bindings; grep Foo.java"));
        assert!(!denied("jawata-author: doc-only edit; sed -i 's/a/b/' Foo.java"));
    }

    #[test]
    fn non_java_work_is_never_touched() {
        for cmd in [
            "grep -rn TODO src/main.rs",
            "rg 'fn main' Cargo.toml",
            "sed -i 's/a/b/' README.md",
            "awk '{print $1}' data.csv",
        ] {
            assert!(!denied(cmd), "the guard must not reach outside Java: {cmd}");
        }
    }

    #[test]
    fn word_boundaries_matter_or_the_guard_denies_things_nobody_asked_about() {
        // A guard that fires on substrings becomes a guard people disable.
        for cmd in [
            "pgrep -f Foo.java",                       // pgrep is not grep
            "cat regrep-notes-Foo.java.txt",           // no tool at all
            "echo 'do not grep the Foo.java file'",    // grep inside a message
            "./my-sed-tool Foo.java",                  // a filename containing sed
        ] {
            assert!(!denied(cmd), "false positive on: {cmd}");
        }
    }

    #[test]
    fn the_guard_defaults_to_allow_on_anything_it_cannot_read() {
        // Under Cursor's failClosed, a wrong denial blocks the user's work.
        // Uncertainty must resolve to allow, always.
        for cmd in ["", "   ", "\u{0}\u{1}", "…", &"x".repeat(10_000)] {
            assert_eq!(Verdict::Allow, judge(cmd), "unreadable input must not deny: {cmd:?}");
        }
    }

    #[test]
    fn judging_never_panics() {
        for cmd in ["", "|||", ";;;", "&&", "/", "\\", ".java", "grep", "grep .java"] {
            let _ = judge(cmd);
        }
    }
}
