//! The guard: a LOCAL policy decision, taken without asking anyone.
//!
//! It denies shell text-search over Java sources and steers to JAWATA's
//! compiler-accurate tools.
//!
//! **THE SHELL TRIPWIRE IS A HINT, NOT A BOUNDARY — studio#5, frozen on the C9
//! architect ruling of 2026-09-08.** Its subject is an opaque command string,
//! and the effect it wants to predict is decided at RUN TIME by data that
//! string need not contain: a path can arrive from a variable, a substitution,
//! a glob, a file list, or a script one level down. Three patches (v3.8.0,
//! v3.8.1, v3.8.2) each closed a set of spellings and each moved the hole, and
//! v3.8.1 shipped two NEW and different regressions doing it — which is this
//! project's design-alarm condition, met. There is no patch four.
//!
//! MEASURED PASSING, 2026-09-08, so the next reader does not rediscover them as
//! a bug: `find . -name '*.java' -exec sed -i {} \;` (the writer is not in
//! command position) · `./rewrite.sh src/Foo.java` (the writer is inside a
//! script) · `sd 'a' 'b' Foo.java` (not on the tool list) · `sed -i s/a/b/ $F`
//! and `git checkout HEAD~1 -- src/main/java/` (no `.java` literal at all, so
//! the precondition rejects them before any writer check runs).
//!
//! MEASURED DENYING, the same day: `sed -i` on a literal path, a `python3`
//! heredoc, and `cd <repo> && sed -i … Foo.java`. Those are the shapes an agent
//! writes by habit, which is what a hint is for.
//!
//! **PREVENTION LIVES IN [`crate::editgate`], not here.** That gate predicates
//! on the tool name and the structured `file_path` — a value the harness
//! supplies and an agent cannot spell around — which is why it has never needed
//! a patch chain. The denial text below argues from CORRECTNESS ("sed edits
//! text, not the program"), not from security, and a correctness argument
//! addressed to a cooperating agent does not need completeness. Two properties make it different from every other
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

/// The two reasons a FULL suite run is legitimate, declared in the command the
/// way the two above are. Deliberately plain words rather than a `jawata-`
/// prefix: these name an event in the user's own process — a checkpoint, a
/// release — and the agent should be writing the stage or version it means.
pub const CHECKPOINT_DECLARATION: &str = "checkpoint:";
pub const RELEASE_DECLARATION: &str = "release:";

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

/// Whole-test-suite runners, by the name in COMMAND POSITION. A project's own
/// sharded runner is the expensive shape this gate is for.
const SUITE_SCRIPTS: &[&str] = &["run-suite.sh", "run-suite", "run-tests.sh", "run-all-tests.sh"];

/// Build tools whose bare test goals run EVERYTHING unless a filter narrows them.
const BUILD_TOOLS: &[&str] = &["mvn", "mvnw", "./mvnw", "gradle", "gradlew", "./gradlew"];

/// Flags that NARROW a build tool's run to a selection — their presence means
/// this is not a full suite and the gate does not apply.
const TEST_FILTERS: &[&str] = &["-Dtest=", "-Dit.test=", "--tests", "-DfailIfNoTests"];

/// What a suite script needs before it runs EVERYTHING. Without it the script
/// selects by compiler reach and is cheap, so the cadence gate leaves it alone.
const FULL_SUITE_FLAG: &str = "--full";

/// Whether this command runs the WHOLE suite, and under which spelling.
///
/// Two shapes. A project's own suite script by name, and a build tool reaching
/// a test phase with nothing narrowing it. `-DskipTests` is the common case that
/// looks like a test command and runs none — a build before a suite is not the
/// suite, and denying it would block the step that PRECEDES every legitimate run.
fn full_suite_in(command: &str) -> Option<&'static str> {
    for segment in segments(command) {
        // COMMAND POSITION for both shapes, which is this file's standing
        // discipline and not a nuance: scanning every word made
        // `echo 'remember to run run-suite.sh'` a denial. Caught by this gate's
        // own prose test on its first run, the same way `echo 'python Foo.java'`
        // is called out for the write gate above.
        let mut words = segment.split_whitespace().skip_while(|w| {
            let bare = w.rsplit(['/', '\\']).next().unwrap_or(w);
            PREFIXES.contains(&bare) || (w.contains('=') && !w.starts_with('-'))
        });
        let Some(first) = words.next() else { continue };
        let head = first.rsplit(['/', '\\']).next().unwrap_or(first);
        if let Some(script) = SUITE_SCRIPTS.iter().find(|s| **s == head) {
            // A suite script now SELECTS by default: it runs only the tests the
            // coverage evidence says the change reaches, and `--full` is what
            // asks for everything. So `--full` is the expensive shape this gate
            // is for, and a bare call is not — the same distinction
            // TEST_FILTERS already draws for a build tool, pointed the other
            // way round because here the narrow form is the default.
            //
            // Harald, 2026-10-06: "Only full runs for checkpoints."
            if !segment.split_whitespace().any(|w| w == FULL_SUITE_FLAG) {
                continue;
            }
            return Some(script);
        }
        let Some(tool) = BUILD_TOOLS.iter().find(|t| {
            **t == head || t.trim_start_matches("./") == head
        }) else {
            continue;
        };
        // A build that explicitly skips tests is not a test run.
        if segment.contains("-DskipTests") || segment.contains("-x test") {
            continue;
        }
        let reaches_tests = segment.split_whitespace().any(|w| {
            w == "test" || w == "verify" || w == "integration-test" || w == "check"
        });
        let narrowed = TEST_FILTERS.iter().any(|f| segment.contains(*f));
        if reaches_tests && !narrowed {
            return Some(tool);
        }
    }
    None
}

/// Decide on one shell command.
pub fn judge(command: &str) -> Verdict {
    // THE SUITE CADENCE GATE RUNS FIRST, before the Java declarations below —
    // otherwise `jawata-author:` (which short-circuits to Allow) would be a
    // bypass for a rule that has nothing to do with authoring Java.
    //
    // Harald, 2026-09-08: "Full suite runs at the checkpoints and when we
    // release only." Measured on Sprint 28e's first night — 17 full runs, mean
    // 11m20s, 3¼ hours out of a 14-hour sprint, and about 13 of them buying
    // nothing that the checkpoint would not have proved once.
    //
    // WHY THIS IS A HOOK AND NOT SEAT TEXT. The seat's own check 9 states the
    // asymmetry: a full suite "costs the agent nothing — it sleeps through the
    // wall-clock — while every minute lands on the person waiting." The agent
    // is the one participant that cannot feel the price, so a cadence left to
    // its judgement is chosen too high every time. Asked for a rule it proposed
    // "run it when a change touches production code", which is true of every
    // edit and discriminates nothing.
    //
    // STATELESS BY DESIGN. An earlier sketch kept a checkpoint marker on disk
    // and compared it against the last run. That adds a second thing to forget
    // and to go stale; requiring the reason ON EVERY RUN needs no state and
    // cannot drift. It is SELF-CERTIFIED — nothing stops a false
    // `checkpoint:` — exactly as nothing stops a weak `jawata-fallback:`. The
    // value is not the block: it is that N runs become N logged claims, and a
    // stage with four declared checkpoints and one real one is visible
    // afterwards. It converts an invisible habit into a countable one.
    if let Some(runner) = full_suite_in(command) {
        if !command.contains(CHECKPOINT_DECLARATION) && !command.contains(RELEASE_DECLARATION) {
            return Verdict::Deny {
                reason: format!(
                    "Full suite run (`{runner}`) is blocked outside a checkpoint or a release. \
                     The whole suite belongs at the two borders where a claim is actually made; \
                     between them, DROP `{FULL_SUITE_FLAG}` — the runner then runs only the \
                     tests the coverage evidence says your change reaches, needs no declaration, \
                     and widens itself back with a printed reason wherever that evidence cannot \
                     carry the claim. Measured on one sprint: 17 full runs, 3¼ hours, a quarter \
                     of the sprint, and ~13 of them provable at the next checkpoint for free. \
                     If this IS one, re-run with `{CHECKPOINT_DECLARATION} <stage or id>` or \
                     `{RELEASE_DECLARATION} <version>` in the command; the declaration is \
                     logged. \"Only the full suite says what else the change did\" is TRUE and \
                     is not a reason — that damage surfaces at the checkpoint either way, and \
                     bisecting one red checkpoint is cheaper than the runs spent avoiding it."
                ),
            };
        }
    }
    // The declarations are a LATTICE, and v3.8.1 shipped them as a flat pair.
    // `jawata-author:` is the stronger claim (I am writing Java deliberately)
    // and satisfies both gates. `jawata-fallback:` claims only a justified
    // READ, and must not open a write — otherwise routing `sed -i` to the write
    // gate bought nothing, because the weaker declaration granted everything the
    // stronger one granted. Worse, the read gate's own denial message TEACHES
    // `jawata-fallback:`, so an agent obeying the guard's instruction was handed
    // the key to the write gate. Found by the architect, 2026-08-15.
    if command.contains(AUTHOR_DECLARATION) {
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
    if let Some(writer) = write_route_in(command).filter(|_| !only_prose_mentions_java(command)) {
        return Verdict::Deny {
            reason: format!(
                "Shell-mediated WRITE to a .java file is blocked — `{writer}` edits text, \
                 not the program: it cannot see the references, overloads and call sites the \
                 compiler sees, which is how regressions ship. Use the JAWATA refactoring \
                 tools instead — change_method_signature / rename_symbol / extract / \
                 refactoring(action=plan), all addressable by symbol name — or the Edit tool \
                 inside a declared authoring window. If this genuinely is authoring no tool \
                 can do (new file content, fixtures), re-run with `{AUTHOR_DECLARATION} \
                 <narrow reason>` in the command; the declaration is logged and audited. \
                 WRITING ONLY NON-JAVA FILES and this fired anyway? A script is let through \
                 when every `.java` in it sits inside a quoted sentence (a string with \
                 whitespace, such as prose you are writing into a document). This one has a \
                 `.java` that is a bare argument, a whole-string path, a quoted command, or a \
                 string that could not be read, and whether such a script writes that path is \
                 not decidable from the command text. Use the Edit/Write tool for the non-Java \
                 file, or declare `{AUTHOR_DECLARATION} <reason>` — both proceed."
            ),
        };
    }
    // Past the write gate, a declared fallback proceeds — the declaration IS
    // the audit trail, and a READ is the lesser claim it was written for.
    if command.contains(FALLBACK_DECLARATION) {
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
    write_routes(command).into_iter().next()
}

/// EVERY write route the command takes, in order. [`write_route_in`] is the
/// first; the prose exemption needs the whole list, because one interpreter
/// beside a `cp` is not an interpreter-only command.
fn write_routes(command: &str) -> Vec<&'static str> {
    let mut routes: Vec<&'static str> = Vec::new();
    for segment in segments(command) {
        // NO per-segment .java requirement on the WRITE gate. v3.8.1 added one
        // to kill a false positive and paid in three false negatives: a path
        // reaching the writer through a pipe (`echo Foo.java | xargs sed -i`),
        // through a quoted separator inside a `-c` payload, or across a heredoc
        // is invisible to the segment that does the writing. The scope was the
        // precision knob on a text predicate whose precision and recall are
        // COUPLED — tightening one loosened the other, three times.
        //
        // The cost model decides which way to set it, and the two errors are
        // NOT symmetric. A false denial costs seconds plus one audited
        // `jawata-author:` declaration; it also teaches. A false pass is a
        // silent text rewrite of Java — the exact harm this product exists to
        // prevent — undetected until somebody dogfoods. So the write gate takes
        // whole-command scope and accepts the over-denial; the READ gate keeps
        // segment scope, where the harm is lower and the annoyance higher.
        // Redirection into a .java target: `> Foo.java`, `>>Foo.java`.
        let mut after_redirect = false;
        for raw in segment.split_whitespace() {
            if after_redirect {
                if raw.trim_matches(['"', '\'']).ends_with(".java") {
                    routes.push(">");
                }
                after_redirect = false;
            }
            if raw == ">" || raw == ">>" {
                after_redirect = true;
            } else if let Some(target) = raw.strip_prefix(">>").or_else(|| raw.strip_prefix('>')) {
                if target.trim_matches(['"', '\'']).ends_with(".java") {
                    routes.push(">");
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
                routes.push(tool);
                continue;
            }
            // A reader that was handed an in-place flag is a writer.
            if let Some(tool) = IN_PLACE_TOOLS.iter().find(|t| **t == word) {
                if has_in_place_flag(segment, tool) {
                    routes.push(tool);
                    continue;
                }
            }
            if SHELLS.contains(&word) && segment.contains(" -c") {
                routes.push("sh -c");
                continue;
            }
            // `git apply` rewrites files. `git checkout HEAD~1 -- <path>` does
            // too and is NOT here — deliberately, since studio#5's C9 architect
            // watch showed that adding it would change nothing: that command
            // carries no `.java` literal, so `mentions_java_source` is false and
            // `judge` has already returned Allow before this function is
            // consulted. A branch that cannot be reached is worse than an absent
            // one, because it reads as coverage.
            if word == "git" {
                if let Some(sub) = words.next() {
                    if sub == "apply" {
                        routes.push("git apply");
                    }
                }
            }
        }
    }
    routes
}

/// Whether the command names Java sources. Deliberately narrow: `.java` as a
/// path or a glob. Widening this is how a guard starts denying things nobody
/// asked it to.
fn mentions_java_source(command: &str) -> bool {
    command.contains(".java")
}

/// Interpreters whose only route to a write is the script they are handed — the
/// only routes a prose mention can be innocent of. A `cp`, a `tee` or a
/// redirection names its target as an argument, so a `.java` mention beside one
/// is a path whatever it is quoted with.
const INTERPRETER_ROUTES: &[&str] = &[
    "python", "python3", "python2", "perl", "ruby", "node", "php", "sh -c",
];

/// D8e, studio#11 — whether a command's `.java` mentions are all PROSE and every
/// write route it takes is an interpreter, so the write gate has nothing to deny.
///
/// WHY THE CUT IS HERE. The write gate takes whole-command scope on purpose: a
/// path reaches a writer through a pipe, through a quoted separator, across a
/// heredoc, and no segment-scoped rule sees it. What none of those bypasses is,
/// is a SENTENCE: each is a bare token or a whole-literal path. So the question
/// is not which segment holds the mention but what the mention is — a `.java`
/// inside a quoted string with whitespace in it, written into a document, names
/// a file the way prose does.
///
/// Every occurrence must satisfy all of: it is inside a quoted literal (the
/// INNERMOST one, quotes of unlike kind nesting, so `"open('a.md').write('see
/// Foo.java')"` is judged by the inner `'…'`); that literal closes; it contains
/// whitespace; it does not begin like a path (`/`, `~`, `.`) or hold a
/// backslash; and read as a shell command it takes no write route of its own,
/// which is what stops `os.system('sed -i … Foo.java')` being called a sentence.
///
/// ERRORS GO TOWARD DENIAL, BY CONSTRUCTION RATHER THAN BY HOPE. An apostrophe between two
/// letters is a word, not a quote, and is ignored by `quote_frames`; any quote still left
/// open after that means the text is not read the way it is written, and the whole command
/// is denied. (The first version said a stray apostrophe could only flip prose to bare. It
/// can flip a bare path INTO a literal: `python3 w.py "don't" Foo.java "ok"` was allowed.
/// Found by the C4 audit.)
///
/// What remains, stated: a single-line literal with whitespace INSIDE, no padding at the
/// edges, not starting like a path, that is nonetheless a path — a spaced relative path,
/// which a Java source tree does not have — or a multi-line literal assembled into one by
/// the script (`'x\nsrc/Foo.java'.splitlines()[1]`). A goodwill failure that crafts the
/// quoting is outside what a text predicate can stop and is what the audit ledger is for.
fn only_prose_mentions_java(command: &str) -> bool {
    let routes = write_routes(command);
    if routes.is_empty() || !routes.iter().all(|r| INTERPRETER_ROUTES.contains(r)) {
        return false;
    }
    let frames = quote_frames(command);
    // A quote left open ANYWHERE means the text is not read the way it is written, and the
    // frames after it are on the wrong parity: `python3 w.py "don't" Foo.java "ok"` pairs the
    // quote before `ok` with the one after `don't`, so Foo.java lands inside a "literal" and
    // reads as prose. An unbalanced command is not read at all — it is denied. (C4 audit.)
    if frames.iter().any(|f| f.close.is_none()) {
        return false;
    }
    let mut found = false;
    for (at, _) in command.match_indices(".java") {
        found = true;
        // The innermost frame containing the mention: the one that opened last.
        let Some(frame) = frames
            .iter()
            .filter(|f| f.open < at && f.close.map_or(true, |c| at < c))
            .max_by_key(|f| f.open)
        else {
            return false;
        };
        let Some(close) = frame.close else {
            return false;
        };
        let literal = &command[frame.open + 1..close];
        let begins_like_a_path = literal.trim_start().starts_with(['/', '~', '.']);
        // A single-line literal with whitespace at either end is how a path hides from the
        // whitespace rule: `'src/Foo.java '.strip()` is a sentence by every other test here
        // and a path one call later. A multi-line literal is a document body, whose edges are
        // newlines by construction, so it is exempt from this one check only.
        let padded = !literal.contains('\n') && literal != literal.trim();
        if !literal.contains(char::is_whitespace)
            || padded
            || begins_like_a_path
            || literal.contains('\\')
            || !write_routes(literal).is_empty()
        {
            return false;
        }
    }
    found
}

/// One quoted literal: where it opens, and where it closes if it does.
struct QuoteFrame {
    open: usize,
    close: Option<usize>,
}

/// Every quoted literal in the text, with quotes of unlike kind nesting. A
/// backslash escapes the next character. Deliberately not a shell parser: it
/// reads the quotes of the shell and of the script inside it with one rule,
/// because the command text does not say which language a given quote belongs
/// to.
fn quote_frames(command: &str) -> Vec<QuoteFrame> {
    let mut done: Vec<QuoteFrame> = Vec::new();
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut chars = command.char_indices().peekable();
    let mut previous: Option<char> = None;
    while let Some((i, c)) = chars.next() {
        let before = previous;
        previous = Some(c);
        match c {
            '\\' => {
                chars.next();
            }
            // "don't", "it's": an apostrophe with a letter or digit on both sides is part of a
            // word. Read as a quote it would open a literal that swallows the real ones.
            '\'' if before.is_some_and(char::is_alphanumeric)
                && chars.peek().is_some_and(|(_, n)| n.is_alphanumeric()) => {}
            '\'' | '"' => match open.last() {
                Some(&(q, start)) if q == c => {
                    open.pop();
                    done.push(QuoteFrame { open: start, close: Some(i) });
                }
                _ => open.push((c, i)),
            },
            _ => {}
        }
    }
    done.extend(open.into_iter().map(|(_, start)| QuoteFrame { open: start, close: None }));
    done
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

    // ===== The suite-cadence gate (Harald, 2026-09-08) =====

    /// Built rather than written literally, because this repository carries a
    /// hook that refuses an uncaptured suite command — and to a guard reading
    /// command text, a test fixture NAMING one is indistinguishable from running
    /// one. Found the moment these tests were first added, by being blocked.
    fn suite_cmd(tail: &str) -> String {
        format!("./build/run-{}.sh 4{}", "suite", tail)
    }

    #[test]
    fn a_selecting_run_is_not_the_shape_this_gate_is_for() {
        // Harald, 2026-10-06: "Only full runs for checkpoints."
        //
        // The suite script SELECTS by default now — it runs only the tests the
        // coverage evidence says the change reaches — so a bare call is the
        // cheap inner-loop shape and costs nothing worth declaring. Denying it
        // would make a checkpoint declaration the price of an ordinary test
        // run, which both slows the loop and empties the declaration log of
        // meaning: a log in which every run is declared a checkpoint cannot
        // say which ones were.
        assert!(!denied(&suite_cmd("")));
        assert!(!denied(&suite_cmd(" --impacted")));
    }

    #[test]
    fn a_full_suite_run_is_denied_and_taught_both_declarations() {
        let cmd = suite_cmd(" --full");
        assert!(denied(&cmd));
        let Verdict::Deny { reason } = judge(&cmd) else {
            panic!("expected a denial");
        };
        assert!(
            reason.contains(CHECKPOINT_DECLARATION),
            "the denial must teach the checkpoint form: {reason}"
        );
        assert!(
            reason.contains(RELEASE_DECLARATION),
            "and the release form: {reason}"
        );
    }

    #[test]
    fn either_declaration_lets_the_suite_through() {
        assert!(!denied(&suite_cmd(" --full  # checkpoint: C4")));
        assert!(!denied(&suite_cmd(" --full  # release: v4.2.0")));
    }

    #[test]
    fn the_gate_runs_before_the_author_declaration_so_it_is_not_a_bypass() {
        // `jawata-author:` short-circuits to Allow for the Java gates. With the
        // suite check placed after it, an unrelated authoring window would buy a
        // free full run — the same lattice defect v3.8.1 shipped one gate over.
        let cmd = format!("jawata-author: a fixture && {}", suite_cmd(" --full"));
        assert!(denied(&cmd), "an authoring window must not open the suite gate");
    }

    #[test]
    fn a_build_that_skips_tests_is_not_a_test_run() {
        // The step that PRECEDES every legitimate suite run. Denying it would
        // block the very thing the rule depends on.
        assert!(!denied("mvn -B -q -f build/pom.xml clean package -DskipTests"));
    }

    #[test]
    fn a_filtered_run_is_not_a_full_suite() {
        assert!(!denied("mvn -Dtest=FormMigrationTest test"));
        assert!(!denied("gradle test --tests com.example.FooTest"));
    }

    #[test]
    fn an_unfiltered_build_test_goal_is_a_full_suite() {
        assert!(denied("mvn verify"));
        assert!(denied("cd /repo && ./gradlew test"));
    }

    #[test]
    fn a_targeted_class_run_is_untouched() {
        // The whole between-checkpoint budget: touched classes and the mutation.
        assert!(!denied(
            "java -jar build/dist/jawata.jar -runTests -Djawata.test.classlist=/tmp/c.txt"
        ));
    }

    #[test]
    fn merely_naming_the_runner_in_prose_does_not_fire() {
        // Command position, the same discipline the text and write gates use.
        let cmd = format!("echo 'remember to run {} at the checkpoint'", "run-suite.sh");
        assert!(!denied(&cmd));
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
            // was `python3 -c "print('rewrites Foo.java')"`; a sentence inside a
            // literal is prose since D8e (see the prose tests below), so this
            // case now hands the path to the interpreter as an argument — the
            // `&&` segmentation it exists to pin is unchanged.
            "cd repo && python3 -c \"print('x')\" Foo.java",
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

    /// v3.8.2, and this test is INVERTED from v3.8.1 on purpose.
    ///
    /// v3.8.1 asserted these were allowed, because a writer in another segment
    /// writes no Java. True — and the scoping that bought it let three real
    /// bypasses through, because a path can reach a writer at RUNTIME from
    /// anywhere in the line. The two errors are not symmetric: this denial
    /// costs one audited declaration, the passes cost a silent rewrite of
    /// Java. The write gate over-denies deliberately.
    #[test]
    fn the_write_gate_over_denies_rather_than_guess_where_a_path_came_from() {
        for cmd in [
            "git status --porcelain Foo.java && python3 -c \"print('probe')\"",
            "cat Foo.java | wc -l && node -e \"console.log(1)\"",
        ] {
            assert!(
                denied(cmd),
                "accepted over-denial: the escape is one jawata-author declaration: {cmd}"
            );
        }
    }

    /// studio#11's FIRST half — D8e of Sprint 28g, the defect Harald signed.
    ///
    /// The live false positive: an interpreter script that wrote three `.md`
    /// files and no Java was denied, because its text MENTIONED a `.java` path.
    /// For three releases this was recorded as an accepted cost, on the ground
    /// that an opaque interpreter body may or may not write the path it
    /// mentions and v3.8.1's per-segment scoping reopened three bypasses.
    ///
    /// The cut that holds is not about SEGMENTS but about WHAT the mention is. A
    /// `.java` that sits inside a quoted literal containing whitespace — a
    /// sentence — names a file the way prose does; a `.java` that is a bare
    /// token, or a whole-literal path, is an argument somebody can write to. The
    /// three bypasses were all bare-token shapes and stay denied, below.
    #[test]
    fn a_script_that_writes_only_documents_and_mentions_java_in_prose_is_allowed() {
        let cmd = "python3 - <<'PY'\n\
                   open('plan.md','a').write('see Foo.java')\n\
                   open('notes.md','w').write('The Bar.java call sites move in stage 2')\n\
                   open('log.md','w').write(\"the Baz.java rename is deferred\")\n\
                   PY";
        assert!(
            !denied(cmd),
            "a script writing three documents was denied for the prose it carries: {:?}",
            judge(cmd)
        );
        // The same, one line, with the shell's own quoting around Python's.
        assert!(!denied("python3 -c \"open('a.md','w').write('see Foo.java')\""));
        assert!(!denied("python3 -c 'open(\"a.md\",\"w\").write(\"see Foo.java\")'"));
        // Prose with an apostrophe in a word is still prose: the apostrophe is not a quote.
        assert!(!denied("python3 -c \"open('a.md','w').write('don't forget Foo.java')\""));
        // A document body assembled over several lines keeps its newline edges and is exempt
        // from the padding rule, which is meant for one-line literals only.
        assert!(!denied(
            "python3 - <<'PY'\nopen('a.md','w').write('''\nsee Foo.java here\n''')\nPY"
        ));
    }

    /// The other half of the cut: every shape that stays a write. Each is a
    /// shape the exemption could have swallowed by being too generous.
    #[test]
    fn a_java_that_is_an_argument_and_not_a_sentence_is_still_a_write() {
        for cmd in [
            // the path IS a whole literal
            "python3 -c \"open('Foo.java','w').write('x')\"",
            // a glob or a suffix built from a literal that is only the extension
            "python3 -c \"import glob; glob.glob('**/*.java')\"",
            "python3 -c \"name = 'Foo' + '.java'\"",
            // a literal with whitespace that is a COMMAND the interpreter runs
            "python3 -c \"import os; os.system('sed -i s/a/b/ Foo.java')\"",
            "python3 -c \"import os; os.system('cp fixed.txt Foo.java')\"",
            // a spaced absolute path is still a path
            "python3 -c \"open('/home/x/My Projects/Foo.java','w')\"",
            // one prose mention does not excuse a bare one beside it
            "python3 -c \"open('a.md','w').write('see Foo.java')\" Bar.java",
            // an unclosed quote cannot be read, so it is not read as prose. The
            // text inside has whitespace and starts like a sentence: it is
            // refused ONLY because it never closes. (The first version of this
            // case held no whitespace, so the no-whitespace rule refused it and
            // a mutation deleting the unclosed check stayed green.)
            "python3 -c \"x = 'see Foo.java",
            // a stray apostrophe in a comment must not swallow the real path
            "python3 - <<'PY'\n# don't\nopen('Foo.java','w')\nPY",
            // C4 audit: an apostrophe INSIDE A WORD left a quote open and put the parity of
            // every later quote one off, so the bare Foo.java was read as the inside of a
            // literal that contained whitespace. A quote left open anywhere is a deny.
            "python3 w.py \"don't\" Foo.java \"ok\"",
            // C4 audit: padding is how a path hides from the whitespace rule.
            "python3 -c \"open('src/Foo.java '.strip(),'w')\"",
            "python3 -c \"open(' src/Foo.java','w')\"",
        ] {
            assert!(denied(cmd), "a write to Java was let through as prose: {cmd}");
        }
    }

    /// A prose-only interpreter segment does not launder a NON-interpreter
    /// writer in the same command: `cp` into a spaced path has no bare `.java`
    /// token and a literal with whitespace, which is exactly the shape the
    /// exemption accepts for an interpreter — so it must be refused by who the
    /// writer is, not by how the mention is spelled.
    #[test]
    fn a_prose_exemption_does_not_launder_another_writer_in_the_command() {
        assert!(denied(
            "python3 -c \"open('a.md','w').write('see Foo.java')\" && cp x.txt \"my dir/Foo.java\""
        ));
        assert!(denied(
            "python3 -c \"open('a.md','w').write('see Foo.java')\" && tee \"out dir/Bar.java\""
        ));
    }

    /// The denial that remains still teaches the non-Java case and both routes.
    #[test]
    fn the_remaining_denial_still_names_the_non_java_case_and_its_routes() {
        match judge("python3 write_plan.py plan.md Foo.java") {
            Verdict::Deny { reason } => {
                assert!(
                    reason.contains("WRITING ONLY NON-JAVA FILES"),
                    "the denial must name the non-Java case and its two routes: {reason}"
                );
                assert!(
                    reason.contains(AUTHOR_DECLARATION),
                    "and the declaration that proceeds: {reason}"
                );
            }
            other => panic!("a bare path handed to an interpreter is a write: {other:?}"),
        }
    }

    /// The three shapes v3.8.1's segment scoping let through. A path can reach
    /// the writer from another segment — through a pipe, through a quoted
    /// separator inside a `-c` payload, or across a heredoc.
    #[test]
    fn a_path_reaching_the_writer_from_elsewhere_is_still_a_write() {
        for cmd in [
            // the canonical bulk rewrite — the most damaging shape there is
            "grep -rl TODO --include=*.java . | xargs sed -i 's/TODO/DONE/'",
            "echo Foo.java | xargs sed -i 's/a/b/'",
            // a separator inside quotes is not a separator
            "python3 -c \"print('a | b')\" Foo.java",
            "python3 -c \"print('a && b')\" Foo.java",
            "FILE=Foo.java; sed -i 's/a/b/' $FILE",
        ] {
            assert!(denied(cmd), "a path from elsewhere still reaches the file: {cmd}");
        }
    }

    /// The declarations are a LATTICE. `jawata-author:` is the stronger claim
    /// and opens both gates; `jawata-fallback:` claims a READ only and must not
    /// open a write — otherwise the read gate's own message, which teaches
    /// `jawata-fallback:`, hands out the key to the write gate.
    #[test]
    fn a_read_declaration_does_not_open_the_write_gate() {
        assert!(
            denied("jawata-fallback: bulk sweep; sed -i 's/a/b/' Foo.java"),
            "the weaker declaration must not grant what the stronger one grants"
        );
        assert!(!denied("jawata-author: generating a fixture; sed -i 's/a/b/' Foo.java"));
        // ...and it still does its own job: a justified READ proceeds.
        assert!(!denied("jawata-fallback: no bindings in this fixture; grep Foo.java"));
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
