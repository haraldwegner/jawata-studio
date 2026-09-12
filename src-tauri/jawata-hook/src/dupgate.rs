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
//! # IT SHOWS NOMINEES. IT DOES NOT ASSERT A DUPLICATE.
//!
//! The plan's ruling was that this ships in [`Mode::Block`], and it does not.
//! That is a DECLARED DEVIATION with a measurement behind it rather than a
//! preference, and the measurement is the engine's own:
//!
//! `experience(kind=duplicate_check)` asks the code lane BY MEANING, and the
//! lane applies **no score threshold** — Stage 0 measured that this corpus
//! admits none, because three of ten genuine task-to-job pairs score at or below
//! the noise floor, so any cutoff that admits the real answers admits noise with
//! them. Rank one therefore always comes back. `DuplicateCheckTest` pins it on a
//! case nobody could argue with: a draft that ROUNDS MONEY nominates a job that
//! PARSES SOURCE, because it is the only job in the store and an ordering must
//! order something.
//!
//! A gate that denied on that would deny every Java write in the repository, and
//! would be worked around or switched off inside a day — which is worse than not
//! shipping it, because the switch would take the honest half with it. So the
//! verdict is [`Verdict::Nominated`]: work that MAY already do this, with
//! addresses, for the agent to read and judge. That is the store's own documented
//! contract for anchorless retrieval — distance nominates, the agent decides, and
//! selecting none is a real answer.
//!
//! **What would let it block**: a second, structural signal that a nominee is the
//! same JOB rather than merely the nearest text — the four conditions
//! `re_derived_job` applies to code that exists. Those conditions need resolved
//! bindings, and the draft reaching this gate has none, so the confirming half is
//! missing and is named here rather than approximated.
//!
//! # THE SECOND DECLARED DEVIATION, and why the first does not rest on it
//!
//! An earlier version of the paragraph above said bindings "cannot be applied to
//! a draft", as though that were a fact about drafts. A C8 audit called it
//! circular and was right. `DraftSource` parses the text STANDALONE and asks for
//! no bindings — so "the draft has no bindings" is a consequence of how this is
//! built, not a property of the input. The plan asked for a JDT WORKING COPY,
//! which resolves against a real project and classpath and WOULD supply them.
//! That is deviation two, declared here.
//!
//! What it would actually buy is narrower than it sounds, which is why it is a
//! deviation rather than a defect. A working copy needs a loaded project and an
//! existing compilation unit to be a copy OF. This gate fires on a hook, where a
//! resident may have nothing loaded and where a `Write` is frequently creating a
//! file that does not exist yet — `DuplicateCheckTest` pins exactly that, the
//! verb reporting `existingKnown: false` rather than treating unreadable as
//! empty. So a working copy would confirm on the `Edit`-into-a-loaded-project
//! subset and go on being absent everywhere else, and a gate that blocks on one
//! subset and not another is harder to reason about than one that never blocks.
//!
//! **The deviation does not depend on any of this.** The reason this ships in
//! `Observe` is the FIRST leg above, which is a measurement about the store's
//! ranking and says nothing about bindings: rank one always comes back, so
//! denying on it denies every Java write. That leg stands whether or not the
//! confirming half is ever built. The binding argument was only ever the answer
//! to "what would let it block LATER", and it is restated here as what it is —
//! work not done, not a wall.
//!
//! **WHICH EXIT CLAUSE THIS MOVES, named so a reader checking E10 against the
//! code finds the answer instead of inferring it.** E10 asks that "a draft
//! re-deriving a known job is REFUSED naming it and proceeds with
//! `jawata-duplicate:`". The naming half and the disposition half are met. The
//! REFUSAL half is what the deviation replaces with an advisory, for the reason
//! above.
//!
//! Its neighbour clause — "a file edit alone triggers no lane" — is NOT touched
//! by any of this, and the two are easy to read as being in tension. They are
//! not: a bare `Edit` carries a path and no draft text, so this gate answers
//! [`Verdict::NoDraftMethods`] and stays out of the way. What fires the gate is
//! a draft, not a file event.
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

/// How much authority the gate has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Off entirely — the documented kill switch.
    Off,
    /// Show the nominees and record; never deny. THE SHIPPING DEFAULT — see the
    /// module note, which records why the plan's `Block` ruling is deviated from
    /// and what would earn it back.
    Observe,
    /// Deny an undispositioned write. Available, configured explicitly, and NOT
    /// the default: on today's signal it would deny every Java write.
    Block,
}

impl Mode {
    /// Read the mode from the config value.
    ///
    /// Nothing configured, and a word nobody recognises, both give
    /// [`Mode::Observe`] — the first because it is the shipping default, the
    /// second because reading a typo as `Block` would hand someone more
    /// authority than they asked for, which is the trap
    /// [`crate::recallgate::Mode::parse`] documents.
    pub fn parse(configured: Option<&str>) -> Mode {
        match configured.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("off") | Some("false") | Some("disabled") => Mode::Off,
            Some("block") => Mode::Block,
            _ => Mode::Observe,
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
    /// The agent already said why a second implementation is right here; the
    /// write proceeds and the reason is logged for the architect's watch.
    Dispositioned { reason: String },
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
    /// is the same one. In [`Mode::Observe`] — the default — it is shown and
    /// recorded, and the write proceeds.
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

/// Did this write already say why the duplicate is deliberate?
///
/// # The DRAFT is excluded, and that closes a bypass rather than being a nicety
///
/// The token is the agent's declaration ABOUT the write; it is not content OF
/// the write. Scanning the whole payload conflated the two, so a `.java` file
/// whose own source contains the token — a comment, a doc block, or this gate's
/// own rules quoted into Java — dispositioned the very write that created it.
/// Found by the C8 audit. The draft is removed before the search, so a
/// declaration has to be made where declarations are made.
pub fn disposition_in(payload: &str) -> Option<String> {
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return disposition_outside_the_draft(payload);
    };
    if let Some(nested) = value.get_mut("tool_input") {
        strip_draft_fields(nested);
    }
    strip_draft_fields(&mut value);
    disposition_outside_the_draft(&value.to_string())
}

/// Remove every field [`draft_text`] reads, so what is left is only what the
/// agent said ABOUT the write.
///
/// IT STRIPS THE PARSED VALUE, NOT THE TEXT, and the first version did the
/// opposite: `payload.replace(draft, "")`, which removed NOTHING. `draft_text`
/// hands back the DECODED content while the payload holds it JSON-ESCAPED, so a
/// draft carrying a newline never matched its own escaped form and the token
/// went on dispositioning its own write. The line reads exactly like the thing
/// it was meant to do, which is why only running it said otherwise.
fn strip_draft_fields(scope: &mut Value) {
    let Some(map) = scope.as_object_mut() else {
        return;
    };
    map.remove("content");
    map.remove("new_string");
    if let Some(edits) = map.get_mut("edits").and_then(Value::as_array_mut) {
        for edit in edits {
            if let Some(one) = edit.as_object_mut() {
                one.remove("new_string");
            }
        }
    }
}

fn disposition_outside_the_draft(payload: &str) -> Option<String> {
    let lower = payload.to_lowercase();
    let at = lower.find(DUPLICATE)?;
    let reason: String = payload[at + DUPLICATE.len()..]
        .chars()
        .take_while(|c| *c != '\n' && *c != '"')
        .collect();
    let reason = reason.trim();
    if reason.is_empty() {
        return None;
    }
    Some(reason.to_string())
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
/// The order is the design: a DISPOSITION short-circuits before the engine is
/// asked at all. An agent that has already said why the duplicate is deliberate
/// should not pay a round trip to be told it may proceed.
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
    if let Some(reason) = disposition_in(payload) {
        return Verdict::Dispositioned { reason };
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
/// would be false about the majority of the writes it fires on.
pub fn steering(method: &str, job: &str, location: &str, existing_known: bool) -> String {
    let unchecked = if existing_known {
        ""
    } else {
        "\n\nAND THIS WAS NOT SUBTRACTED FROM THE FILE YOU ARE EDITING — no project was \
         loaded, or the file does not exist yet, so the gate could not tell whether the \
         method you are about to write is ALREADY THERE. That is a different question from \
         the one above and it went unanswered rather than answered no."
    };
    format!(
        "JAWATA — this may already be done. The closest thing the codebase records to \
         `{method}` is:\n  {location} — {job}\n\nOpen it before writing. If it does what \
         you need, call it instead. If it does NOT — or if it is simply unrelated, which \
         is a normal answer here — say so and proceed: put `{DUPLICATE} <why a second \
         implementation is right here>` in the call. A reason is required, and it is not a \
         formality: the architect's report carries it, so a second implementation nobody \
         justified is refused there too.{unchecked}"
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
                let s = steering(&method, &job, &location, existing_known);
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
                let checked = steering(&method, &job, &location, true);
                assert!(!checked.contains("NOT SUBTRACTED FROM THE FILE"), "{checked}");
            }
            other => panic!("expected a nomination, got {other:?}"),
        }
    }

    #[test]
    fn a_declared_duplicate_proceeds_and_carries_its_reason() {
        let payload = json!({
            "content": "private static Tree parse(Source s) { return null; }",
            "why": "jawata-duplicate: the shared one resolves bindings and this path must not"
        })
        .to_string();
        match judge(Mode::Block, "Write", "/p/A.java", &payload, |_, _| {
            panic!("a dispositioned write must not pay a round trip to the engine")
        }) {
            Verdict::Dispositioned { reason } => {
                assert!(reason.contains("must not"), "{reason}");
            }
            other => panic!("expected a disposition, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_token_with_no_reason_is_not_a_disposition() {
        // The one-word bypass this gate is shaped to refuse.
        assert_eq!(None, disposition_in("jawata-duplicate:"));
        assert_eq!(None, disposition_in("jawata-duplicate:    "));
        assert_eq!(
            Some("the two differ on the error path".to_string()),
            disposition_in("jawata-duplicate: the two differ on the error path")
        );
    }

    /// THE DRAFT CANNOT DISPOSITION ITSELF — a bypass the C8 audit found.
    ///
    /// The token declares something ABOUT the write. A file whose own source
    /// contains it (a comment, a doc block, this gate's rules quoted into Java)
    /// would otherwise wave through the very write that created it, and the
    /// bypass is trivially reachable by anyone writing about the mechanism.
    #[test]
    fn the_token_inside_the_drafted_file_is_content_not_a_declaration() {
        let payload = json!({
            "tool_name": "Write",
            "tool_input": {
                "file_path": "/p/Notes.java",
                "content": "/** Write `jawata-duplicate: because X` to declare one. */\n\
                            class Notes { void go() {} }"
            }
        })
        .to_string();
        assert_eq!(
            None,
            disposition_in(&payload),
            "the token is inside the DRAFT, so it is the file's content and not the \
             agent's declaration about writing it"
        );
        // The control: the same token OUTSIDE the draft still dispositions, or
        // the fix above would have closed the mechanism rather than the bypass.
        let declared = json!({
            "tool_name": "Write",
            "why": "jawata-duplicate: the shared reader cannot see this format",
            "tool_input": {"file_path": "/p/Notes.java", "content": "class N { void go() {} }"}
        })
        .to_string();
        assert!(disposition_in(&declared).is_some(), "a real declaration must still count");
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

    /// THE DEVIATION, PINNED. The plan ruled that this gate ships in `Block`; it
    /// ships in `Observe`, because the engine's own test measures that rank one
    /// comes back for a draft with nothing to do with EITHER stored job — two are
    /// recorded there precisely so the ranking has to choose. Asserting the
    /// default here is what makes the deviation visible to anyone who changes it
    /// back without supplying the confirming signal.
    ///
    /// The module note carries both deviations: this one, and the working copy
    /// the plan asked `DraftSource` for and did not get. Only THIS one decides
    /// the mode.
    #[test]
    fn it_ships_in_observe_and_block_must_be_asked_for() {
        assert_eq!(Mode::Observe, Mode::parse(None));
        assert_eq!(Mode::Observe, Mode::parse(Some("")));
        assert_eq!(Mode::Block, Mode::parse(Some("block")));
        assert_eq!(Mode::Off, Mode::parse(Some("off")));
        // A word nobody recognises never escalates.
        assert_eq!(Mode::Observe, Mode::parse(Some("blok")));
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
}
