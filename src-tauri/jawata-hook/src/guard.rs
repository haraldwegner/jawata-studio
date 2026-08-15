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

/// Text tools that BECOME writers when handed an in-place flag. `sed` reads;
/// `sed -i` rewrites the file. The distinction decides which gate applies and
/// therefore which declaration lets it through — a read needs
/// `jawata-fallback:`, a write needs `jawata-author:`.
const IN_PLACE_TOOLS: &[&str] = &["sed", "awk", "gawk", "perl", "ruby"];

/// Split a command into the segments that can each hold their own command in
/// COMMAND POSITION.
///
/// `&&` and `||` belong here and their absence was a hole, not a nuance: with
/// only `| ; \n` as separators, `cd repo && python3 -c '…' Foo.java` puts `cd`
/// in command position, and BOTH gates look at `cd` and shrug. The v3.8.0
/// tripwire was therefore defeated by the most ordinary prefix an agent writes
/// — measured live on 2026-08-15, one day after it shipped, by the agent that
/// wrote it, using `cd … && sed -i` on a real source file.
fn segments(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(['|', ';', '\n', '&'])
        .filter(|s| !s.trim().is_empty())
}

/// Whether THIS segment carries an in-place flag for a tool that only writes
/// with one. Conservative on `awk`, which needs gawk's `-i inplace` extension:
/// a bare `-i` there is an include, not a rewrite.
fn has_in_place_flag(segment: &str, tool: &str) -> bool {
    let words: Vec<&str> = segment.split_whitespace().collect();
    if tool == "awk" || tool == "gawk" {
        return words.iter().any(|w| *w == "inplace" || *w == "-iinplace");
    }
    words.iter().any(|w| {
        *w == "--in-place"
            || w.starts_with("--in-place=")
            || (w.starts_with("-i") && !w.starts_with("--"))
    })
}

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
    // THE WRITE TRIPWIRE comes first: a shell-mediated WRITE to a .java file is
    // strictly worse than a text search over one, and it happened — the agent
    // developing this very gate rewrote .java call sites with a python3 heredoc
    // from Bash on 2026-08-14, and this guard watched the command go by because
    // it only knew the four search tools. Goodwill did not hold in the agent
    // that built the gate; the pattern space cannot be enumerated, but the
    // common spellings can be, and every denial teaches the tool that should
    // have been called.
    if let Some(writer) = write_route_in(command) {
        return Verdict::Deny {
            reason: format!(
                "Shell-mediated WRITE to a .java file is blocked — `{writer}` edits text, \
                 not the program: it cannot see the references, overloads and call sites the \
                 compiler sees, which is how regressions ship. Use the JAWATA refactoring \
                 tools instead — change_method_signature / rename_symbol / extract / \
                 refactoring(action=plan), all addressable by symbol name — or the Edit tool \
                 inside a declared authoring window. If this genuinely is authoring no tool \
                 can do (new file content, fixtures), re-run with `{AUTHOR_DECLARATION} \
                 <narrow reason>` in the command; the declaration is logged and audited."
            ),
        };
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

/// Write-capable programs in COMMAND POSITION: interpreters that can rewrite
/// files handed a script, stream writers, editors, and patchers. Same
/// command-position discipline as [`text_tool_in`] — `echo 'python Foo.java'`
/// must not fire.
const WRITE_TOOLS: &[&str] = &[
    "python", "python3", "python2", "perl", "ruby", "node", "php",
    "tee", "dd", "ex", "ed", "patch",
    "mv", "cp", "ln", "install", "rsync", "truncate",
];

/// Shells that become writers when handed inline code.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

/// Which write route the command takes toward a `.java` file, if any.
///
/// Three shapes, all requiring `.java` in the command text (checked by the
/// caller): a write-capable program in command position; a shell handed inline
/// code (`-c`); or output redirection whose target is a `.java` path. Like
/// [`text_tool_in`], deliberately not a shell parser — a miss on an exotic
/// spelling is the accepted cost, and the layers above (artifact watch, ledger)
/// exist because no enumeration closes this space.
fn write_route_in(command: &str) -> Option<&'static str> {
    for segment in segments(command) {
        // The .java target must be in THIS segment. Checking the whole command
        // denied `git status Foo.java && python3 -c 'print(1)'`, where nothing
        // writes Java at all — a false denial, and under Cursor's failClosed a
        // false denial blocks real work.
        //
        // EXCEPT for a heredoc: its body is part of the command that opened it,
        // and the body is where the path lives. The original live bypass was
        // exactly that shape — `python3 - <<'PY'` on one line, the .java path
        // three lines down — so scoping it to the opening line alone would have
        // re-opened the hole this guard exists to close. Its anchor test caught
        // that within a minute of the change.
        let scope = if segment.contains("<<") { command } else { segment };
        if !mentions_java_source(scope) {
            continue;
        }
        // Redirection into a .java target: `> Foo.java`, `>>Foo.java`.
        let mut after_redirect = false;
        for raw in segment.split_whitespace() {
            if after_redirect {
                if raw.trim_matches(['"', '\'']).ends_with(".java") {
                    return Some(">");
                }
                after_redirect = false;
            }
            if raw == ">" || raw == ">>" {
                after_redirect = true;
            } else if let Some(target) = raw.strip_prefix(">>").or_else(|| raw.strip_prefix('>')) {
                if target.trim_matches(['"', '\'']).ends_with(".java") {
                    return Some(">");
                }
            }
        }
        // A writer or code-carrying shell in command position.
        let mut words = segment
            .split_whitespace()
            .skip_while(|w| {
                let bare = w.rsplit(['/', '\\']).next().unwrap_or(w);
                PREFIXES.contains(&bare) || (w.contains('=') && !w.starts_with('-'))
            });
        if let Some(first) = words.next() {
            let word = first.rsplit(['/', '\\']).next().unwrap_or(first);
            if let Some(tool) = WRITE_TOOLS.iter().find(|t| **t == word) {
                return Some(tool);
            }
            // A reader that was handed an in-place flag is a writer.
            if let Some(tool) = IN_PLACE_TOOLS.iter().find(|t| **t == word) {
                if has_in_place_flag(segment, tool) {
                    return Some(tool);
                }
            }
            if SHELLS.contains(&word) && segment.contains(" -c") {
                return Some("sh -c");
            }
            // `git apply` / `git checkout --` rewrite files too.
            if word == "git" {
                if let Some(sub) = words.next() {
                    if sub == "apply" {
                        return Some("git apply");
                    }
                }
            }
        }
    }
    None
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
    for segment in segments(command) {
        if !mentions_java_source(segment) {
            continue;
        }
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

    /// v3.8.1. The v3.8.0 tripwire shipped with `&&` missing from the segment
    /// separators, so `cd repo && <writer> Foo.java` put `cd` in command
    /// position and BOTH gates shrugged. Measured live one day after release,
    /// by the agent that wrote the gate, on a real source file. These are the
    /// exact commands that got through.
    #[test]
    fn a_cd_prefix_does_not_smuggle_a_writer_past_the_tripwire() {
        for cmd in [
            // the live bypass, verbatim
            "cd /home/harald/CursorProjects/jawata-mcp && sed -i 's/a/b/' \
             org.jawata.core/src/org/jawata/core/host/HostOS.java",
            "cd repo && python3 -c \"print('rewrites Foo.java')\"",
            "cd repo && tee Foo.java",
            "cd a && cd b && perl -pi -e 's/x/y/' Foo.java",
            "cd repo || python3 write.py Foo.java",
        ] {
            assert!(denied(cmd), "a cd prefix must not smuggle a writer: {cmd}");
        }
    }

    /// `sed` reads; `sed -i` rewrites. The flag decides the GATE, and therefore
    /// which declaration lets it through: a read needs jawata-fallback, a write
    /// needs jawata-author.
    #[test]
    fn an_in_place_flag_turns_a_reader_into_a_writer() {
        match judge("sed -i 's/a/b/' Foo.java") {
            Verdict::Deny { reason } => assert!(
                reason.contains("WRITE"),
                "sed -i must be judged a WRITE, not a text search: {reason}"
            ),
            Verdict::Allow => panic!("sed -i on a .java file must be denied"),
        }
        match judge("sed -n '1,5p' Foo.java") {
            Verdict::Deny { reason } => assert!(
                reason.contains("text search"),
                "plain sed is a READ and must be judged as one: {reason}"
            ),
            Verdict::Allow => panic!("a text search over .java is still denied"),
        }
        // gawk needs the `inplace` extension; a bare -i there is an include.
        assert!(denied("awk -i inplace '{print}' Foo.java"));
    }

    /// The other half of the same root cause: the .java target must be in the
    /// segment that carries the writer. Checking the whole command denied a
    /// read of a Java file that merely shared a line with an unrelated
    /// interpreter — and under Cursor's failClosed a false denial blocks work.
    #[test]
    fn a_writer_in_another_segment_is_not_a_write_to_java() {
        for cmd in [
            "git status --porcelain Foo.java && python3 -c \"print('probe')\"",
            "cat Foo.java | wc -l && node -e \"console.log(1)\"",
        ] {
            assert!(!denied(cmd), "false positive — nothing writes Java here: {cmd}");
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

#[cfg(test)]
mod write_tripwire_tests {
    use super::*;

    fn denied(cmd: &str) -> bool {
        matches!(judge(cmd), Verdict::Deny { .. })
    }

    /// THE control: the exact shape that bypassed this guard live on
    /// 2026-08-14 — a python3 heredoc rewriting .java call sites, watched by a
    /// guard that only knew the four search tools. This command line is the
    /// regression anchor; if it ever passes again, the tripwire is gone.
    #[test]
    fn the_live_bypass_command_is_now_denied() {
        let bypass = "python3 - <<'PY'\np='org.jawata.core.tests/src/org/jawata/core/project/ProjectImporterTest.java'\ns=open(p).read()\nPY";
        match judge(bypass) {
            Verdict::Deny { reason } => {
                assert!(reason.contains("change_method_signature"),
                    "the denial must hand over the tool that should have been called: {reason}");
                assert!(reason.contains(AUTHOR_DECLARATION),
                    "the denial must name the authoring escape: {reason}");
            }
            Verdict::Allow => panic!("the live bypass shape passed the guard AGAIN"),
        }
    }

    #[test]
    fn every_write_route_over_java_is_denied() {
        for cmd in [
            "perl -i -pe 's/a/b/' src/Foo.java",
            "ruby rewrite.rb Foo.java",
            "node patch.js src/main/java/Foo.java",
            "tee src/Foo.java",
            "patch -p1 Foo.java.patch src/Foo.java",
            "mv staged.txt src/main/java/Foo.java",
            "cp fixed.txt src/Foo.java",
            "echo 'class X {}' > src/Foo.java",
            "cat fix.txt >> src/Foo.java",
            "bash -c 'printf x > Foo.java'",
            "git apply fix-Foo.java.patch",
        ] {
            assert!(denied(cmd), "write route slipped through: {cmd}");
        }
    }

    /// Reads stay reads. A guard that denies looking at a file is a guard
    /// people turn off — and under Cursor's failClosed, a false denial blocks
    /// real work.
    #[test]
    fn reading_java_is_untouched() {
        for cmd in [
            "cat src/main/java/Foo.java",
            "head -20 Foo.java",
            "wc -l src/Foo.java",
            "ls -la src/main/java/",
            "git diff -- src/Foo.java",
            "git log --oneline Foo.java",
            "java Foo.java",
            "javac src/main/java/Foo.java",
            "echo 'please fix Foo.java with python'",
            "python3 analyze_log.py build.log",
        ] {
            assert!(!denied(cmd), "a read/innocent command was denied: {cmd}");
        }
    }

    #[test]
    fn the_authoring_declaration_still_opens_the_narrow_door() {
        assert!(!denied("jawata-author: writing a cmd fixture body no tool produces; tee src/Foo.java"));
    }
}
