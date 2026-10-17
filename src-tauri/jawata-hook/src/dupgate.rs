//! Sprint 28f Stage 8 D3 — THE DUPLICATE GATE.
//!
//! The moment this whole sprint is aimed at. An agent is about to write a method
//! that already exists somewhere in the codebase, under a different name, doing
//! the same job. Nobody is at fault: the agent cannot know, because nothing told
//! it. This gate is the thing that tells it, at the only moment the telling is
//! free — before the method is written.
//!
//! It is [`crate::recallgate`]'s shape, deliberately, so the two read alike: a
//! [`Mode`] with a documented kill switch, a [`Verdict`] every variant of which
//! can explain itself in one log line, a disposition token that takes a REASON,
//! and fail-open on an unavailable knowledge layer.
//!
//! # IT REFUSES, AND THE AGENT JUDGES — Sprint 28g S22 (the signed D8, restored)
//!
//! The 28f spec signed this gate as a refusal: *"A match refuses the write unless the call
//! carries `jawata-duplicate: <the architect's reason>` … The cost, accepted by your ruling: a
//! false match blocks a legitimate write for one extra line."* The first build shipped it as an
//! advisory instead, on a measurement that is TRUE and does not support that: the code lane
//! applies **no score threshold** — three of ten genuine task-to-job pairs score at or below the
//! noise floor — so rank one always comes back, a draft that rounds money nominates a job that
//! parses source, and no score can tell a duplicate from a new job.
//!
//! That measurement says no SCORE can judge. It does not say nobody can: the store's own rule is
//! that distance nominates and the reading agent decides. So the gate does not wait for a bar that
//! cannot exist. A write that adds a method is **refused until the call carries one of two
//! declarations**, and either one lets it through:
//!
//! - `jawata-duplicate: <why a second implementation is right here>` — the job exists, and a second
//!   one is deliberate;
//! - `jawata-distinct: <how this method's job differs from the nominated one>` — the nominee is
//!   the nearest text and not the same job.
//!
//! Both take a REASON and both are logged for the architect's watch — the reason together with the
//! location of the job it was given about — so the agent is never stuck, the same as the
//! `jawata-author:` lattice. The declaration is made in a shell command, after the refusal, and
//! covers the next write once (see the section below on where it is made). The cost is the one he accepted: a false match costs
//! one extra line.
//!
//! The nominated job and its location are named in the refusal, so the declaration is made after
//! looking at it, not instead of looking. Selecting none is a real answer, and `jawata-distinct:`
//! is how it is said.
//!
//! Its neighbour clause — "a file edit alone triggers no lane" — is untouched: a bare `Edit`
//! carries a path and no draft text, so this gate answers [`Verdict::NoDraftMethods`] and stays out
//! of the way. What fires the gate is a draft, not a file event.
//!
//! # Fail OPEN, and say which
//!
//! When the engine cannot answer — `KNOWLEDGE_UNAVAILABLE`, a dead resident, a
//! timeout — the write PROCEEDS and the reason is recorded. A gate that blocks
//! when the knowledge layer is down converts an outage into a work stoppage,
//! which is a worse failure than the one it prevents. Crucially it is recorded
//! as "could not answer" and never as "nothing like this exists": those are
//! different facts and this sprint exists because they used to print the same.

use serde_json::Value;
use std::path::Path;

/// How much authority the gate has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Off entirely — the documented kill switch.
    Off,
    /// Show the nominees and record; never deny. Available when configured explicitly, for a
    /// project that wants to measure before it refuses.
    Observe,
    /// Deny an undispositioned write until it carries a declaration. THE SHIPPING DEFAULT, as
    /// signed.
    Block,
}

impl Mode {
    /// Read the mode from the config value.
    ///
    /// Nothing configured, and a word nobody recognises, both give
    /// [`Mode::Block`]: the first because it is the shipping default, the second because the
    /// default is the strict reading and a typo must not quietly switch a gate off.
    pub fn parse(configured: Option<&str>) -> Mode {
        match configured.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("off") | Some("false") | Some("disabled") => Mode::Off,
            Some("observe") => Mode::Observe,
            _ => Mode::Block,
        }
    }
}

/// What the gate concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not an editing tool, or not a `.java` path — not our business.
    NotAJavaEdit,
    /// The kill switch is off.
    Disabled,
    /// The draft declares no method, so there is nothing to ask about.
    NoDraftMethods,
    /// The lane was asked and nominated nothing with an address.
    NoNominee,
    /// The engine could not answer. The write proceeds and the reason is
    /// recorded — NOT as "nothing like this exists".
    Unavailable { why: String },
    /// The lane nominated work that MAY already do this job.
    ///
    /// **A nomination and not a finding.** See the module note: the ranking
    /// carries no threshold, so this names the nearest job and never asserts it
    /// is the same one. In [`Mode::Block`] — the default — the write is held until the
    /// agent declares what it judged; in [`Mode::Observe`] it is shown and recorded.
    ///
    /// `existing_known` is the ENGINE's own answer to a different question: could
    /// it subtract what the drafted file already declares. When it is false the
    /// gate cannot tell "this job exists elsewhere" from "this method is already
    /// in the very file you are editing", and the steering says so. Dropping that
    /// flag would print a confident sentence over an unchecked one, which is the
    /// distinction this whole sprint exists to keep.
    Nominated { method: String, job: String, location: String, existing_known: bool },
}

/// The declaration that says the second implementation is DELIBERATE.
///
/// It takes a reason, and a bare token is not a disposition. A one-word bypass
/// is how a gate decays into a ritual, and here the reason is load-bearing
/// beyond this gate: the architect seat's report must carry it, so an order for
/// a second implementation that nobody justified is refused downstream too.
pub const DUPLICATE: &str = "jawata-duplicate:";

/// The declaration that says the nominated job is NOT this method's job.
///
/// Same shape and same rule as [`DUPLICATE`]: it takes a reason, and a bare token is not a
/// disposition. It exists because the ranking always nominates something, so the honest answer to
/// a nominee that is merely the nearest text needs a form of its own.
pub const DISTINCT: &str = "jawata-distinct:";

/// What the agent declared about a nominated write, with the reason it gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// `jawata-duplicate:` — the job exists and a second implementation is deliberate.
    Duplicate(String),
    /// `jawata-distinct:` — the nominee is not the same job.
    Distinct(String),
}

impl Declaration {
    /// The reason the agent gave.
    pub fn reason(&self) -> &str {
        match self {
            Declaration::Duplicate(r) | Declaration::Distinct(r) => r,
        }
    }

    /// The name the gate's signal log records this declaration under.
    pub fn signal(&self) -> &'static str {
        match self {
            Declaration::Duplicate(_) => "duplicate-declared",
            Declaration::Distinct(_) => "duplicate-distinct-declared",
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Declaration::Duplicate(_) => "duplicate",
            Declaration::Distinct(_) => "distinct",
        }
    }
}

// ---- WHERE A DECLARATION IS MADE, AND WHAT IT ANSWERS ---------------------------------------
//
// A `Write` or an `Edit` carries a path and the text to write, and nothing else — there is no field
// in it for an agent to say what it judged, and a token inside the drafted text is content, not a
// declaration. So the declaration is made in a SHELL command, the way the authoring window is
// opened: the guard reads the command and records the declaration for the session, and the next
// write that would have been refused consumes it.
//
// It is BOUND TO A REFUSAL. A refusal leaves a pending record naming the nominated job; a
// declaration is accepted only while one is pending, and it carries that job's location with it.
// So a declaration made before the agent was shown anything — or a command that merely mentions the
// token — opens nothing, and the log can say which job each reason was given about. It covers ONE
// write and expires, so a genuinely new method costs exactly one line and nothing becomes a standing
// bypass. Both records live in [`crate::oneshot`], the one store of this kind.

const PENDING: &str = "dup-pending";
const SAID: &str = "dup-said";

/// A write was refused (or shown) because of this nominee: remember it, so the declaration that
/// answers it can be bound to it.
pub fn note_refusal(home: &Path, session_id: &str, method: &str, location: &str) {
    crate::oneshot::open(home, session_id, PENDING, &[method, location]);
}

/// Record a declaration for the refusal that is pending. Returns false — and records nothing — when
/// none is: the agent has not been shown a nominee, so it has nothing to answer.
pub fn open_declaration(home: &Path, session_id: &str, declaration: &Declaration) -> bool {
    let Some(pending) = crate::oneshot::peek(home, session_id, PENDING) else {
        return false;
    };
    let location = pending.get(1).map(String::as_str).unwrap_or("");
    crate::oneshot::open(home, session_id, SAID, &[declaration.kind(), declaration.reason(), location]);
    true
}

/// Consume the session's declaration, with the location of the job it was made about. TAKING it is
/// the point: the same line cannot cover a second write.
pub fn take_declaration(home: &Path, session_id: &str) -> Option<(Declaration, String)> {
    let fields = crate::oneshot::take(home, session_id, SAID, |_| true)?;
    crate::oneshot::discard(home, session_id, PENDING);
    let reason = fields.get(1)?.clone();
    let location = fields.get(2).cloned().unwrap_or_default();
    match fields.first()?.as_str() {
        "duplicate" => Some((Declaration::Duplicate(reason), location)),
        "distinct" => Some((Declaration::Distinct(reason), location)),
        _ => None,
    }
}

/// The declaration a shell command carries, if it carries one.
///
/// It reads the COMMAND, decoded — `tool_input.command`, or a top-level `command` — and nothing
/// else in the payload. The earlier version scanned the re-serialised payload, which left the
/// escaping in the text: `grep -rn "jawata-distinct:" src` ended in `\"` and the stray backslash
/// counted as a reason. A reason must now contain a letter or a digit, so a quote or a backslash
/// cannot stand in for one.
pub fn declaration_in_command(payload: &str) -> Option<Declaration> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let command = value
        .pointer("/tool_input/command")
        .or_else(|| value.get("command"))?
        .as_str()?;
    declaration_in_text(command)
}

fn declaration_in_text(text: &str) -> Option<Declaration> {
    // ASCII lowering keeps every byte offset, so the index found in the lowered copy is valid in
    // the original even when the text before the token is not ASCII.
    let lower = text.to_ascii_lowercase();
    // Either token counts, and a bare one counts as neither: the first spelling that carries a
    // reason decides, so a stray empty `jawata-duplicate:` does not hide a real `jawata-distinct:`.
    for (token, make) in [
        (DUPLICATE, Declaration::Duplicate as fn(String) -> Declaration),
        (DISTINCT, Declaration::Distinct as fn(String) -> Declaration),
    ] {
        let Some(at) = lower.find(token) else { continue };
        let rest = &text[at + token.len()..];
        // The reason follows the token after a space, as a sentence does. A token followed at
        // once by a quote or a bracket is a pattern being SEARCHED FOR (`grep "jawata-distinct:"
        // src`), and what comes after the quote is a path, not a judgement.
        if !rest.starts_with([' ', '\t']) {
            continue;
        }
        let line = rest.lines().next().unwrap_or("");
        let reason = line.trim().trim_end_matches(['\'', '"', '`']).trim();
        if reason.chars().any(char::is_alphanumeric) {
            return Some(make(reason.to_string()));
        }
    }
    None
}

/// The draft text a write would put on disk.
///
/// `Write` carries the whole file in `content`; `Edit` carries a FRAGMENT in
/// `new_string`; `MultiEdit` carries several. All three are forwarded as-is and
/// the engine decides what to do with them — it is the side that holds JDT and
/// can parse a working copy, and a gate that tried to assemble the resulting
/// file here would be a second, worse implementation of the engine's job.
/// It looks under `tool_input` AND at the top level, and the first version looked
/// only at the top level — which is where none of this arrives. A Claude Code
/// payload nests the write under `tool_input`, so the gate would have answered
/// "no draft methods" for every real write while passing every unit test, because
/// the unit tests handed it the shape it expected. [`edit_path_in`] already reads
/// both spellings for the same reason; this follows it rather than inventing a
/// third convention.
///
/// [`edit_path_in`]: crate::pipeline
pub fn draft_text(payload: &str) -> Option<String> {
    let value: Value = serde_json::from_str(payload).ok()?;
    for scope in [value.get("tool_input"), Some(&value)].into_iter().flatten() {
        if let Some(content) = scope.get("content").and_then(Value::as_str) {
            return Some(content.to_string());
        }
        if let Some(new_string) = scope.get("new_string").and_then(Value::as_str) {
            return Some(new_string.to_string());
        }
        if let Some(edits) = scope.get("edits").and_then(Value::as_array) {
            let joined: Vec<&str> = edits
                .iter()
                .filter_map(|e| e.get("new_string").and_then(Value::as_str))
                .collect();
            if !joined.is_empty() {
                return Some(joined.join("\n"));
            }
        }
    }
    None
}

/// THE PURE CORE: the first nominee the engine returned that can be OPENED.
///
/// Structured, because a rendered line carries prose and no addresses. Reading a
/// nominee off prose would be the regex mistake this crate exists to end.
pub fn first_nominee(data: &Value) -> Option<(String, String, String)> {
    let nominees = data.get("nominees")?.as_array()?;
    for n in nominees {
        let method = n.get("method").and_then(Value::as_str).unwrap_or_default();
        let job = n.get("job").and_then(Value::as_str).unwrap_or_default();
        let location = n.get("location").and_then(Value::as_str).unwrap_or_default();
        // A nominee with no LOCATION is not actionable: the whole point is to
        // send the reader at code they can open, and "something like this exists
        // somewhere" is the unhelpful half of the answer.
        if !location.is_empty() && !job.is_empty() {
            return Some((method.to_string(), job.to_string(), location.to_string()));
        }
    }
    None
}

/// Judge one write.
///
/// `ask` is a closure rather than the `Store` trait so this module stays
/// independent of the pipeline's transport, and so a test can hand it a fixture
/// without standing up an engine.
///
/// Judge one write: what would the engine nominate for it.
///
/// This decides nothing about a declaration — that is the pipeline's, because a declaration is
/// made in a shell command and bound to the refusal it answers (see [`open_declaration`]).
pub fn judge<F>(mode: Mode, tool_name: &str, path: &str, payload: &str, ask: F) -> Verdict
where
    F: Fn(&str, &str) -> Result<Value, crate::query::QueryError>,
{
    if mode == Mode::Off {
        return Verdict::Disabled;
    }
    // The editing-tool and `.java` tests belong to the edit gate and are reused
    // rather than rewritten. A second copy of "is this a Java edit" is precisely
    // the smell the sibling detector reports.
    if !crate::editgate::is_editing_tool(tool_name) || !crate::editgate::is_java_source(path) {
        return Verdict::NotAJavaEdit;
    }
    let Some(draft) = draft_text(payload) else {
        return Verdict::NoDraftMethods;
    };
    match ask(path, &draft) {
        Ok(answer) => match first_nominee(&answer) {
            Some((method, job, location)) => Verdict::Nominated {
                method,
                job,
                location,
                // Absent is treated as NOT known, deliberately: an engine that
                // stopped sending the flag would otherwise silently upgrade every
                // answer to "checked".
                existing_known: answer
                    .get("existingKnown")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            None => Verdict::NoNominee,
        },
        Err(e) => Verdict::Unavailable { why: format!("{e:?}") },
    }
}

/// The line the agent is shown when the gate has a nominee.
///
/// It says MAY, and the hedge is the honest part rather than a softening: the
/// ranking carries no threshold, so a sentence claiming this IS the same job
/// would be false about the majority of the writes it fires on. `refuses` is whether the write
/// is being held: the same facts are given either way, and only the consequence differs.
pub fn steering(
    method: &str,
    job: &str,
    location: &str,
    existing_known: bool,
    refuses: bool,
) -> String {
    let unchecked = if existing_known {
        ""
    } else {
        "\n\nAND THIS WAS NOT SUBTRACTED FROM THE FILE YOU ARE EDITING — no project was \
         loaded, or the file does not exist yet, so the gate could not tell whether the \
         method you are about to write is ALREADY THERE. That is a different question from \
         the one above and it went unanswered rather than answered no."
    };
    let consequence = if refuses {
        "This write is HELD until the call carries a declaration of what you judged."
    } else {
        "Say what you judged, in the call."
    };
    format!(
        "JAWATA — this may already be done. The closest thing the codebase records to \
         `{method}` is:\n  {location} — {job}\n\nOpen it before writing. If it does what \
         you need, call it instead. {consequence} If it does NOT, say which of the two it is \
         and proceed:\n  `{DUPLICATE} <why a second implementation is right here>` — the job \
         exists and a second one is deliberate;\n  `{DISTINCT} <how this method's job differs \
         from the one above>` — the nearest text is not the same job, which is a normal answer \
         here. Say it in a SHELL command (for example `echo '{DISTINCT} …'`) and then repeat \
         this write: the line covers that one write. A reason is required in both and both are \
         logged: the architect's report carries it, so a second implementation nobody justified \
         is refused there too.{unchecked}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine_answer(nominees: &[(&str, &str, &str)]) -> Value {
        json!({
            "nominees": nominees.iter()
                .map(|(m, j, l)| json!({"method": m, "job": j, "location": l}))
                .collect::<Vec<_>>()
        })
    }

    fn write_of(text: &str) -> String {
        json!({"content": text}).to_string()
    }

    #[test]
    fn a_draft_with_a_nominee_is_told_where_to_look() {
        let v = judge(
            Mode::Observe,
            "Write",
            "/p/src/Reader.java",
            &write_of("private static Tree parse(Source s) { return null; }"),
            |_, _| {
                Ok(engine_answer(&[(
                    "parse",
                    "Parse a compilation unit with binding resolution",
                    "org.jawata.mcp.tools.shared.SourceScan#parse",
                )]))
            },
        );
        match v {
            Verdict::Nominated { method, job, location, existing_known } => {
                assert_eq!("parse", method);
                assert!(job.contains("binding resolution"), "{job}");
                assert_eq!("org.jawata.mcp.tools.shared.SourceScan#parse", location);
                // This answer carries no `existingKnown`, and absent reads as NOT known —
                // an engine that stopped sending the flag must not silently upgrade every
                // answer to "checked".
                assert!(!existing_known, "an absent flag is not a yes");
                let s = steering(&method, &job, &location, existing_known, false);
                // The steering must NAME what to open — a gate that says "this may be a
                // duplicate" without saying of what leaves the reader where they started.
                assert!(s.contains("SourceScan#parse"), "{s}");
                // And it must say MAY. The ranking carries no threshold, so a sentence
                // asserting sameness would be false on most of the writes it fires on.
                assert!(s.contains("may already be done"), "{s}");
                // And it must say that the OTHER question went unanswered. Without this
                // the reader cannot tell "this job exists elsewhere" from "this method is
                // already in the file you are editing".
                assert!(s.contains("NOT SUBTRACTED FROM THE FILE"), "{s}");

                // THE CONTROL: when the engine DID subtract, the caveat is absent. Without
                // it the assertion above is satisfied by a gate that prints the sentence
                // unconditionally, which would be a caveat that means nothing.
                let checked = steering(&method, &job, &location, true, false);
                assert!(!checked.contains("NOT SUBTRACTED FROM THE FILE"), "{checked}");
            }
            other => panic!("expected a nomination, got {other:?}"),
        }
    }

    #[test]
    fn an_unavailable_engine_lets_the_write_through_and_says_which() {
        match judge(Mode::Block, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
            Err(crate::query::QueryError::ToolRefused {
                code: "KNOWLEDGE_UNAVAILABLE".into(),
                message: "the store is rebuilding".into(),
            })
        }) {
            Verdict::Unavailable { why } => {
                assert!(why.contains("KNOWLEDGE_UNAVAILABLE"), "{why}");
            }
            other => panic!("an outage must not become a work stoppage, got {other:?}"),
        }
    }

    #[test]
    fn silence_from_the_lane_is_not_a_nomination() {
        assert_eq!(
            Verdict::NoNominee,
            judge(Mode::Observe, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
                Ok(json!({"nominees": []}))
            })
        );
    }

    #[test]
    fn a_nominee_with_no_location_is_not_actionable() {
        // "Something like this exists somewhere" is the unhelpful half of the
        // answer, and showing it would spend the agent's time for nothing.
        assert_eq!(
            Verdict::NoNominee,
            judge(Mode::Observe, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
                Ok(engine_answer(&[("parse", "parses something", "")]))
            })
        );
    }

    #[test]
    fn only_java_edits_are_our_business() {
        let p = write_of("void x() {}");
        assert_eq!(
            Verdict::NotAJavaEdit,
            judge(Mode::Observe, "Read", "/p/A.java", &p, |_, _| Ok(json!({})))
        );
        assert_eq!(
            Verdict::NotAJavaEdit,
            judge(Mode::Observe, "Write", "/p/notes.txt", &p, |_, _| Ok(json!({})))
        );
    }

    #[test]
    fn the_kill_switch_is_off_and_nothing_is_asked() {
        assert_eq!(
            Verdict::Disabled,
            judge(Mode::Off, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
                panic!("Off must not reach the engine")
            })
        );
    }

    /// THE SIGNED DEFAULT, PINNED. The 28f spec signed this gate as a refusal and the first build
    /// shipped an advisory; asserting the default here is what makes a change back visible.
    #[test]
    fn it_ships_in_block_and_observe_must_be_asked_for() {
        assert_eq!(Mode::Block, Mode::parse(None));
        assert_eq!(Mode::Block, Mode::parse(Some("")));
        assert_eq!(Mode::Observe, Mode::parse(Some("observe")));
        assert_eq!(Mode::Off, Mode::parse(Some("off")));
        // A word nobody recognises never switches the gate off or down.
        assert_eq!(Mode::Block, Mode::parse(Some("blok")));
    }

    /// The refusal names the nominated job and its location, and says both declarations.
    #[test]
    fn the_refusal_names_the_job_the_location_and_both_declarations() {
        let held = steering("parse", "Parse a compilation unit", "a.b.SourceScan#parse", true, true);
        assert!(held.contains("HELD"), "{held}");
        assert!(held.contains("a.b.SourceScan#parse"), "{held}");
        assert!(held.contains("Parse a compilation unit"), "{held}");
        assert!(held.contains(DUPLICATE) && held.contains(DISTINCT), "{held}");
        // The control: shown without refusing, the same facts are given and the hold is not.
        let shown = steering("parse", "Parse a compilation unit", "a.b.SourceScan#parse", true, false);
        assert!(!shown.contains("HELD"), "{shown}");
    }

    #[test]
    fn an_edit_fragment_is_forwarded_as_the_draft() {
        // An Edit carries a fragment rather than a file, and it must still reach
        // the engine: adding one method inside a class body is exactly how a
        // re-derived job arrives.
        let payload = json!({"new_string": "  private Tree parse(Source s) { return null; }"})
            .to_string();
        let seen = std::cell::RefCell::new(String::new());
        let v = judge(Mode::Observe, "Edit", "/p/A.java", &payload, |_, draft| {
            *seen.borrow_mut() = draft.to_string();
            Ok(json!({"nominees": []}))
        });
        assert_eq!(Verdict::NoNominee, v);
        assert!(seen.borrow().contains("parse"), "the fragment must reach the engine");
    }

    #[test]
    fn the_mode_decides_what_is_done_with_a_verdict_not_what_the_verdict_is() {
        // The same fact either way, which is what makes an Observe count
        // comparable with a Block count if the mode is ever promoted.
        let ask = |_: &str, _: &str| Ok(engine_answer(&[("x", "does x", "com.example.X#x")]));
        let observed = judge(Mode::Observe, "Write", "/p/A.java", &write_of("void x() {}"), ask);
        let blocked = judge(Mode::Block, "Write", "/p/A.java", &write_of("void x() {}"), ask);
        assert_eq!(observed, blocked);
        assert!(matches!(observed, Verdict::Nominated { .. }), "{observed:?}");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jawata-dup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shell(command: &str) -> String {
        json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string()
    }

    /// A declaration is read from the DECODED command, and a bare token is nothing — including on
    /// the payloads that really arrive, where the escaping once turned a quote into a reason.
    #[test]
    fn a_bare_token_with_no_reason_is_not_a_declaration_on_real_payloads() {
        for command in [
            "grep -rn \"jawata-distinct:\" src",
            "echo \"jawata-distinct:\"",
            "echo 'jawata-duplicate:'",
            "echo jawata-distinct:   ",
            "git commit -m \"mention jawata-duplicate:\"",
        ] {
            assert_eq!(None, declaration_in_command(&shell(command)), "{command}");
        }
        assert_eq!(
            Some(Declaration::Distinct("this one rounds money, that one parses source".into())),
            declaration_in_command(&shell("echo 'jawata-distinct: this one rounds money, that one parses source'"))
        );
        assert_eq!(
            Some(Declaration::Duplicate("the shared runner cannot see this format".into())),
            declaration_in_command(&shell("echo jawata-duplicate: the shared runner cannot see this format"))
        );
        // A stray empty token must not hide a real one beside it.
        assert_eq!(
            Some(Declaration::Distinct("a different job".into())),
            declaration_in_command(&shell("echo jawata-duplicate:\necho jawata-distinct: a different job"))
        );
        // A top-level `command`, the other client's shape.
        assert!(declaration_in_command(&json!({"command": "echo jawata-distinct: a real reason"}).to_string()).is_some());
    }

    /// THE DRAFT AND THE FILE'S EXISTING TEXT ARE NEVER A DECLARATION: only a shell command's text
    /// is read, so a token sitting in a file (written earlier, or quoted in an `old_string`) opens
    /// nothing.
    #[test]
    fn a_token_in_a_write_or_an_edit_is_content_and_never_a_declaration() {
        for payload in [
            json!({"tool_name": "Write", "tool_input": {"file_path": "/p/N.java",
                "content": "// jawata-distinct: because x\nclass N {}"}}),
            json!({"tool_name": "Edit", "tool_input": {"file_path": "/p/N.java",
                "old_string": "// jawata-distinct: because x", "new_string": "void go() {}"}}),
            json!({"tool_name": "Edit", "tool_input": {"file_path": "/p/N.java", "why": "jawata-distinct: sneaked in"}}),
        ] {
            assert_eq!(None, declaration_in_command(&payload.to_string()), "{payload}");
        }
    }

    /// A DECLARATION ANSWERS A REFUSAL. Made before one is pending it opens nothing; made after, it
    /// carries the refused job's location, and it covers exactly one write.
    #[test]
    fn a_declaration_is_bound_to_the_refusal_it_answers_and_covers_one_write() {
        let home = scratch("bound");
        let said = Declaration::Distinct("that job reads files, this one reads a string".into());

        assert!(!open_declaration(&home, "s", &said), "nothing is pending, so there is nothing to answer");
        assert_eq!(None, take_declaration(&home, "s"), "and nothing was recorded");

        note_refusal(&home, "s", "parse", "a.b.SourceScan#parse");
        assert!(open_declaration(&home, "s", &said));
        assert_eq!(
            Some((said.clone(), "a.b.SourceScan#parse".to_string())),
            take_declaration(&home, "s"),
            "it carries the location of the job it was about"
        );
        assert_eq!(None, take_declaration(&home, "s"), "and the same line covers no second write");
        // The refusal was answered, so a later declaration has no refusal to attach to.
        assert!(!open_declaration(&home, "s", &said));
        // Another session never sees it.
        note_refusal(&home, "other", "parse", "x#y");
        assert!(open_declaration(&home, "other", &said));
        assert_eq!(None, take_declaration(&home, "s"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
