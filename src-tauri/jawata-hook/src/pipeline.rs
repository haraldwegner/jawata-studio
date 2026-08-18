//! The pipeline: role → cue → query → emit, driven by the table.
//!
//! One function, taking everything it needs as arguments, so the whole flow is
//! testable without a client, a resident, or a process exit. `main` is then a
//! thin shell: arm the watchdog, gather the real inputs, call this, exit.
//!
//! Every branch that ends without emitting returns a
//! [`SilenceReason`](crate::safety::SilenceReason). That is the invariant
//! Stage 8's log depends on, and the reason the return type is not an
//! `Option`.

use crate::config::HookConfig;
use crate::emit::{self, Emission};
use crate::query::{self, Answer, Endpoint, QueryError};
use crate::roles::{Availability, Client, Role};
use crate::safety::{Outcome, SilenceReason};

/// How the store is reached. Injected so the pipeline can be driven with a
/// stub — a hook whose only test needs a live JVM is a hook nobody tests.
pub trait Store {
    fn ask(&self, arguments: serde_json::Value) -> Result<Answer, QueryError>;

    /// The STRUCTURED answer, for the one caller that needs more than the
    /// rendered line: the recall gate reads `entries[].symbol` to tell a record
    /// anchored at the member from one anchored at its package, and the text
    /// line carries no anchor.
    ///
    /// A required method, not a defaulted one. A default returning "not
    /// supported" would let a Store implementation look complete while the gate
    /// silently never fires against it — the hollow-shape failure this codebase
    /// has already paid for once.
    fn ask_value(&self, arguments: serde_json::Value) -> Result<serde_json::Value, QueryError>;
}

/// The real one.
pub struct LiveStore(pub Endpoint);

impl Store for LiveStore {
    fn ask(&self, mut arguments: serde_json::Value) -> Result<Answer, QueryError> {
        with_budget(&mut arguments, self.0.timeout);
        query::ask(&self.0, arguments)
    }

    fn ask_value(&self, mut arguments: serde_json::Value) -> Result<serde_json::Value, QueryError> {
        with_budget(&mut arguments, self.0.timeout);
        query::ask_value(&self.0, arguments)
    }
}

/// Margin for the round trip itself — request, JSON, response — so the store's
/// deadline fires with time left for its answer to reach us.
const BUDGET_MARGIN_MILLIS: u64 = 300;

/// Floor, mirroring the engine's own: below this a healthy read would be cut off
/// and a working store reported as an outage.
const BUDGET_FLOOR_MILLIS: u64 = 200;

/// Tell the store OUR deadline (jawata-mcp#37).
///
/// The engine bounds a retrieval and answers `KNOWLEDGE_UNAVAILABLE` when the
/// budget blows — but its default budget is 15 seconds, and this hook gives up
/// at 1500 ms. Without this, a wedged store still produced an anonymous
/// transport timeout here and the typed answer arrived ten times too late to be
/// read: the fix existed and its consumer could never see it.
///
/// Injected HERE rather than at each call site, so a caller cannot forget it —
/// a capability that three production sites turned off by picking the shorter
/// constructor is this repository's own recorded lesson.
fn with_budget(arguments: &mut serde_json::Value, timeout: std::time::Duration) {
    let budget = (timeout.as_millis() as u64)
        .saturating_sub(BUDGET_MARGIN_MILLIS)
        .max(BUDGET_FLOOR_MILLIS);
    if let Some(map) = arguments.as_object_mut() {
        map.insert("budget_ms".into(), serde_json::json!(budget));
    }
}

/// Run one hook invocation.
pub fn run(role: Role, config: &HookConfig, payload: &str, store: &dyn Store) -> Outcome {
    let client = match config.client() {
        Ok(c) => c,
        Err(reason) => return Outcome::Silent(reason),
    };
    let Some(spec) = crate::roles::spec(role, client) else {
        return Outcome::Silent(SilenceReason::RoleAbsentOnClient);
    };
    if matches!(spec.availability, Availability::Absent { .. }) {
        return Outcome::Silent(SilenceReason::RoleAbsentOnClient);
    }

    match role {
        // The guard decides locally and never asks: it must answer while the
        // resident is down, and a guard that asked and failed open would leak
        // exactly the calls it exists to deny.
        Role::Guard => guard(client, payload),
        Role::Primer => primer(client, config, store),
        // Stage 4: the recall gate runs BEFORE the ordinary injection, and only
        // on ToolRecall — a tool call about a symbol is the one event where the
        // store's answer can be checked against the call's own subject. In
        // Observe mode (the shipping default) it records what it WOULD have
        // held and falls through; only in Block mode does it stop the call.
        Role::ToolRecall => match recall_gate(client, config, payload, store) {
            Some(outcome) => outcome,
            None => recall(role, client, payload, store),
        },
        Role::UserPrompt => recall(role, client, payload, store),
        Role::Stop => stop_gate(client, payload, crate::stop::Autonomy::Unknown),
        // Sprint 28b D8: the ported observer arm (outcome capture, slip trail,
        // edit feed) — no longer the stub that read as a dead channel.
        Role::Observer => crate::observer::observe(client, payload, Some(config)),
    }
}

/// The guard. Reads the command out of the payload and answers locally.
///
/// A payload it cannot read resolves to ALLOW, never to silence: on Cursor
/// this hook runs under `failClosed: true`, so emitting nothing is itself a
/// block on the user's command. "I could not tell" must therefore be an
/// explicit allow, which is the opposite of the default everywhere else in
/// this binary — and is why it is written down here.
fn guard(client: Client, payload: &str) -> Outcome {
    // THE EDIT HALF, FIRST. Sprint 28's binary read only a shell command and
    // never looked at which tool fired, so a front-door `Edit` of a `.java`
    // file went through unblocked — caught by the 3.7.3 dogfood, and the reason
    // this role was reverted to its script for months. A guard that enforces
    // half its contract is the failure this whole sprint exists to end.
    if let Some(tool) = tool_name_in(payload) {
        // A Bash command declaring `jawata-author:` OPENS the window rather
        // than being judged by it — authoring new code is not a refactor, and
        // the declaration is the audit trail.
        if tool.eq_ignore_ascii_case("Bash") && payload.contains(crate::guard::AUTHOR_DECLARATION) {
            if let (Some(home), Some(session)) = (home_dir(), session_id_in(payload)) {
                let reason = after_marker(payload, crate::guard::AUTHOR_DECLARATION);
                crate::editgate::open_window(&home, &session, &reason);
            }
            return emit_permission(client, true, String::new());
        }
        if let Some(path) = edit_path_in(payload) {
            let window_open = match (home_dir(), session_id_in(payload)) {
                (Some(home), Some(session)) => crate::editgate::window_is_open(&home, &session),
                _ => false,
            };
            let exists = std::path::Path::new(&path).exists();
            match crate::editgate::judge_edit(&tool, &path, payload, window_open, exists) {
                crate::editgate::EditVerdict::Denied(reason) => {
                    return emit_permission(client, false, reason)
                }
                crate::editgate::EditVerdict::Allowed(_) => {
                    return emit_permission(client, true, String::new())
                }
                crate::editgate::EditVerdict::NotApplicable => {}
            }
        }
    }
    let command = command_in(payload).unwrap_or_default();
    let emission = match crate::guard::judge(&command) {
        crate::guard::Verdict::Allow => Emission::Permission {
            allowed: true,
            reason: String::new(),
        },
        crate::guard::Verdict::Deny { reason } => Emission::Permission { allowed: false, reason },
    };
    match emit::render(client, &emission) {
        Some(rendered) => Outcome::Emitted(rendered),
        None => Outcome::Silent(SilenceReason::CannotInject),
    }
}

/// Render one permission decision. Shared by the edit half and the shell half
/// so both speak the client's dialect through exactly one code path.
fn emit_permission(client: Client, allowed: bool, reason: String) -> Outcome {
    match emit::render(client, &Emission::Permission { allowed, reason }) {
        Some(rendered) => Outcome::Emitted(rendered),
        None => Outcome::Silent(SilenceReason::CannotInject),
    }
}

/// Which tool fired, from either client's payload shape.
pub(crate) fn tool_name_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = parse_payload(payload).ok()?;
    for key in ["tool_name", "toolName", "tool"] {
        if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// The file an editing tool is about to write.
pub(crate) fn edit_path_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = parse_payload(payload).ok()?;
    for path in [
        &["tool_input", "file_path"][..], // Claude Code, Edit/Write/MultiEdit
        &["tool_input", "path"][..],
        &["file_path"][..], // Cursor
        &["path"][..],
    ] {
        let mut cursor = &value;
        let mut found = true;
        for key in path {
            match cursor.get(key) {
                Some(next) => cursor = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            if let Some(s) = cursor.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The session this call belongs to — the authoring window's scope.
pub(crate) fn session_id_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = parse_payload(payload).ok()?;
    for key in ["session_id", "sessionId", "conversation_id"] {
        if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The user's home, for the authoring window's directory.
///
/// `USERPROFILE` is checked too: on Windows `HOME` is often unset, and this
/// role now runs there natively rather than through a shell that would have
/// supplied it.
pub(crate) fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// The text following a declaration marker, trimmed and capped.
///
/// Capped because it lands in a file and in the audit trail: an unbounded
/// reason from a payload is an unbounded write.
fn after_marker(payload: &str, marker: &str) -> String {
    payload
        .find(marker)
        .map(|i| &payload[i + marker.len()..])
        .map(|rest| {
            rest.split(['"', '\\', '\n'])
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(200)
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// The shell command, from either client's payload shape.
fn command_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = parse_payload(payload).ok()?;
    for path in [
        &["tool_input", "command"][..],   // Claude Code, PreToolUse/Bash
        &["command"][..],                 // Cursor, beforeShellExecution
    ] {
        let mut cursor = &value;
        let mut found = true;
        for key in path {
            match cursor.get(key) {
                Some(next) => cursor = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            if let Some(s) = cursor.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn primer(client: Client, config: &HookConfig, store: &dyn Store) -> Outcome {
    let answer = store.ask(serde_json::json!({
        "kind": "primer", "format": "text", "limit": 12
    }));
    let heading = "JAWATA domain primer (what this codebase is about):";

    // Sprint 28b D9: the periodic failure reminder rides the primer — the
    // session's own opening, spoken by the agent. NO pop-up exists anywhere in
    // this sprint: the user's ruling was that the main agent tells him.
    let owed = config.field_dir.as_ref().and_then(|field_dir| {
        let dir = std::path::Path::new(field_dir);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        crate::field::reminder_due(dir, now)
            .map(|(line, _carries_question)| (dir.to_path_buf(), now, line))
    });

    let outcome = match (answer, owed.as_ref()) {
        // The store had nothing to prime with, and a reminder is owed. The
        // reminder is then the WHOLE injected context, not a heading for an
        // answer that does not exist: it used to be prepended to the heading,
        // which `finish` DISCARDS on `Nothing`, so the user was told nothing
        // (28b closing audit, F3). D9's point is that the user who never opens
        // studio still learns jawata is failing for him — an empty primer is
        // no reason to withhold that.
        (Ok(Answer::Nothing), Some((_, _, line))) => {
            emit_body(client, Role::Primer, line.clone())
        }
        // Anything else keeps the store's own outcome and its classification:
        // a query failure or a contract mismatch must stay THE reason the
        // primer was silent, because that is the dead-channel fold's
        // numerator. The reminder rides along when there is something to ride.
        (answer, Some((_, _, line))) => {
            // NO SESSION, DELIBERATELY (Stage 5): the primer is the always-on
            // domain layer, not a nominee raised about a question the agent
            // asked. Demanding a disposition for it would turn the skip signal
            // into a session-start tax and teach the token as a reflex — the
            // failure the gate's own narrowness exists to avoid.
            finish(client, Role::Primer, answer, &format!("{line}\n\n{heading}"), "")
        }
        (answer, None) => finish(client, Role::Primer, answer, heading, ""),
    };

    // Recorded ONLY on a real emission — the discipline `observer.rs` already
    // follows for the nudge. A reminder the client never saw is still OWED;
    // burning its weekly slot for an emission that never happened is exactly
    // how the user gets told nothing while the ledger says he was told.
    if let (Outcome::Emitted(_), Some((dir, now, _))) = (&outcome, &owed) {
        crate::field::record_reminded(dir, *now);
    }
    outcome
}

/// Stage 4: the recall gate's I/O half — the pure decision lives in
/// [`crate::recallgate`].
///
/// `Some(outcome)` means the gate has spoken for this call; `None` means it has
/// nothing to say and the ordinary recall injection should run. It answers
/// `None` far more often than not, and that is the design: it fires only when a
/// record's own anchor IS the symbol the call is about.
///
/// FAILS OPEN at every step. No member cue, kill switch off, store unreachable,
/// answer unusable — all `None`. A gate that blocked when the knowledge layer
/// was down would turn an outage into a work stoppage, which is worse than the
/// miss it prevents.
fn recall_gate(
    client: Client,
    config: &HookConfig,
    payload: &str,
    store: &dyn Store,
) -> Option<Outcome> {
    let mode = crate::recallgate::Mode::parse(config.recall_gate.as_deref());
    let verdict = crate::recallgate::judge(mode, payload, |cue| {
        store.ask_value(serde_json::json!({ "kind": "recall", "symbol": cue }))
    });

    match verdict {
        // Nothing to say — the ordinary injection runs.
        crate::recallgate::Verdict::Disabled
        | crate::recallgate::Verdict::NoMemberCue
        | crate::recallgate::Verdict::NoAnchoredRecord => None,

        // The agent already said what it did. Recorded, then out of the way.
        crate::recallgate::Verdict::Dispositioned { token } => {
            emit_gate_signal("recall-dispositioned", &token);
            note_disposition(&session_id_in(payload).unwrap_or_default(), &token);
            None
        }

        // The knowledge layer could not answer. Recorded as ITS OWN fact — this
        // is exactly the distinction jawata-mcp#37 built, and folding it into
        // "nothing to gate on" would throw it away at the one consumer that
        // asked for it.
        crate::recallgate::Verdict::Unavailable { why } => {
            emit_gate_signal("recall-gate-unavailable", &why);
            None
        }

        crate::recallgate::Verdict::Undispositioned { cue, summary } => match mode {
            // OBSERVE (the shipping default): record what would have been held
            // and let the call through. Promotion to Block is decided on this
            // number, not on intent.
            crate::recallgate::Mode::Observe => {
                emit_gate_signal("recall-would-block", &cue);
                None
            }
            crate::recallgate::Mode::Block => {
                emit_gate_signal("recall-blocked", &cue);
                Some(emit_body(
                    client,
                    Role::ToolRecall,
                    crate::recallgate::steering(&cue, &summary),
                ))
            }
            crate::recallgate::Mode::Off => None,
        },
    }
}

/// One line in the outcomes log the observer already owns, so the gate's
/// counters live where every other signal lives rather than in a second file
/// with its own idea of the format.
fn emit_gate_signal(signal: &str, detail: &str) {
    if let Some(home) = home_dir() {
        crate::observer::emit_signal(
            &home.join(".claude").join("jawata-studio"),
            signal,
            detail,
        );
    }
}

fn recall(role: Role, client: Client, payload: &str, store: &dyn Store) -> Outcome {
    // Stage 5: the ledger is per session, so a skip is a property of ONE
    // conversation rather than of the machine.
    let session = session_id_in(payload).unwrap_or_default();
    let cues = match cues_for(role, payload) {
        Ok(c) => c,
        Err(reason) => return Outcome::Silent(reason),
    };

    // Symbol cues first — they are precise, and they fire independently of the
    // two-token gate. Then symptoms. The FIRST answer wins; an absence falls
    // through to the next cue, which is why an observed absence must be
    // distinguishable from a failure here.
    let mut last_failure: Option<QueryError> = None;
    for (key, cue) in cues
        .symbols
        .iter()
        .map(|c| ("symbol", c))
        .chain(cues.symptoms.iter().map(|c| ("symptom", c)))
    {
        match store.ask(serde_json::json!({ "kind": "recall", "format": "text", key: cue })) {
            Ok(Answer::Text(text)) => {
                return finish(
                    client,
                    role,
                    Ok(Answer::Text(text)),
                    "JAWATA recalled candidate prior knowledge for this topic — these are \
                     NOMINEES, not vouched answers; judge whether each fits before relying on it:",
                    &session,
                )
            }
            Ok(Answer::Nothing) => continue,
            Err(e) => {
                // Remember it, keep trying: one unreachable attempt should not
                // discard the cues we have not tried. But do NOT let the run
                // end reporting "the store had nothing" when it never answered.
                last_failure = Some(e);
            }
        }
    }
    match last_failure {
        // Sprint 28b D7: a contract mismatch is ITS OWN reason, never folded
        // into query-failed — the fold classifies it as answered-but-suppressed
        // (the dead-channel numerator), which query-failed is not.
        Some(crate::query::QueryError::ContractMismatch { ours, theirs }) => Outcome::Silent(
            SilenceReason::ContractMismatch(format!("ours={ours} theirs={theirs}")),
        ),
        Some(e) => Outcome::Silent(unusable_or_failed(e)),
        None => Outcome::Silent(SilenceReason::StoreHadNothing),
    }
}

fn finish(
    client: Client,
    role: Role,
    answer: Result<Answer, QueryError>,
    heading: &str,
    session: &str,
) -> Outcome {
    match answer {
        Ok(Answer::Text(text)) => {
            let out = emit_body(client, role, format!("{heading}\n{text}"));
            if matches!(out, Outcome::Emitted(_)) {
                note_injection(session);
            }
            out
        }
        Ok(Answer::Nothing) => Outcome::Silent(SilenceReason::StoreHadNothing),
        Err(crate::query::QueryError::ContractMismatch { ours, theirs }) => Outcome::Silent(
            SilenceReason::ContractMismatch(format!("ours={ours} theirs={theirs}")),
        ),
        Err(e) => Outcome::Silent(unusable_or_failed(e)),
    }
}

/// Put one body into this client's context, or say why it could not go.
///
/// The one place a body becomes an emission, so "was it actually delivered?"
/// has a single answer for every caller — the question D9's ledger has to ask
/// before it burns a week's slot.
fn emit_body(client: Client, role: Role, body: String) -> Outcome {
    match emit::context_for(role, client, body) {
        Emission::Silent => Outcome::Silent(by_design_or_failed(role, client)),
        other => match emit::render(client, &other) {
            Some(rendered) => Outcome::Emitted(rendered),
            None => Outcome::Silent(by_design_or_failed(role, client)),
        },
    }
}

/// Stage 5: record that knowledge REACHED this session.
///
/// Called only on a real emission, and that is the whole point — the skip
/// ledger keys on INJECTED, never on the store having answered. On a client
/// that cannot inject, nothing is recorded and no session can later be accused
/// of ignoring what it was never given.
fn note_injection(session: &str) {
    if let Some(home) = home_dir() {
        crate::recallledger::record_injected(
            &home.join(".claude").join("jawata-studio"),
            session,
        );
    }
}

/// Stage 5: record that the agent SAID what it did with recalled knowledge.
fn note_disposition(session: &str, token: &str) {
    if let Some(home) = home_dir() {
        crate::recallledger::record_disposition(
            &home.join(".claude").join("jawata-studio"),
            session,
            token,
        );
    }
}

/// Why this role emitted nothing when the store HAD something to say.
///
/// The role table is the authority: a cell whose client cannot inject this
/// event is quiet BY DESIGN (Cursor's user-prompt and observer), and folding
/// that as `cannot-inject` marked every Cursor machine's channel permanently
/// dead — a built-in false alarm (C2 audit F2). A render that fails on a cell
/// which CAN inject is the real defect and keeps the old name.
fn by_design_or_failed(role: Role, client: Client) -> SilenceReason {
    match crate::roles::spec(role, client) {
        Some(spec) if !spec.can_inject => SilenceReason::RecordedNotInjected,
        _ => SilenceReason::CannotInject,
    }
}

/// The store ANSWERED — a 200 with a body — and the answer was unusable.
///
/// That is a different fact from never reaching the store, and it is the
/// historic two-week outage's own mechanism (a shape that drifted, read as an
/// absence). It counts toward the dead-channel condition; `query-failed` does
/// not, because nothing answered at all.
fn unusable_or_failed(e: QueryError) -> SilenceReason {
    match e {
        QueryError::ShapeChanged(_) => SilenceReason::AnswerUnusable("ShapeChanged".into()),
        QueryError::ToolRefused { code, .. } => {
            SilenceReason::AnswerUnusable(format!("ToolRefused:{code}"))
        }
        other => SilenceReason::QueryFailed(format!("{other:?}")),
    }
}

/// Pull the prompt out of the client's event payload.
///
/// Both clients put it under `prompt`; Claude's `PreToolUse` payload instead
/// carries a tool input. `serde_json`, never a regex — a payload whose shape
/// moved must be a named failure, not an empty prompt.
/// Cues for a recall, derived PER ROLE — because the two roles receive
/// different kinds of text and the difference is load-bearing.
///
/// A UserPrompt payload is TYPED text: the slash-command rule applies. A
/// ToolRecall payload is a tool target — a symbol, a file path, a command —
/// and applying the typed rules to it is the 3.7.2 dogfood bug F2: every
/// absolute path begins with `/`, so every Read/Edit recall was skipped as a
/// "slash command" and the role went silent on Linux entirely.
fn cues_for(role: Role, payload: &str) -> Result<crate::cue::Cues, SilenceReason> {
    let value = parse_payload(payload)?;
    match role {
        Role::UserPrompt => {
            let prompt = string_at(&value, &["prompt"]).ok_or_else(|| {
                SilenceReason::PayloadUnreadable(
                    "the payload carried no `prompt` — the event shape moved".into(),
                )
            })?;
            crate::cue::extract(&prompt)
                .map_err(|skip| SilenceReason::NoCues(format!("{skip:?}")))
        }
        _ => tool_cues(&value),
    }
}

/// Cues for a TOOL event, in the script generation's own priority order
/// (Sprint 21a dogfood: the subject identifiers win — a rename carrying
/// `symbol` + `newName` must query the OLD name, so `newName` is last):
/// the refactor-subject keys, then the edited file's type name
/// (`Foo.java` → `Foo`), then the raw strings through the untyped extractor.
fn tool_cues(value: &serde_json::Value) -> Result<crate::cue::Cues, SilenceReason> {
    for key in ["typeName", "symbol", "query", "newName"] {
        if let Some(sym) = string_at(value, &["tool_input", key]) {
            return Ok(crate::cue::Cues {
                symbols: vec![sym],
                symptoms: Vec::new(),
                content_tokens: 0,
            });
        }
    }
    if let Some(path) = string_at(value, &["tool_input", "file_path"]) {
        if let Some(sym) = crate::cue::symbol_from_path(&path) {
            return Ok(crate::cue::Cues {
                symbols: vec![sym],
                symptoms: Vec::new(),
                content_tokens: 0,
            });
        }
        return crate::cue::extract_tool_target(&path)
            .map_err(|skip| SilenceReason::NoCues(format!("{skip:?}")));
    }
    if let Some(cmd) = string_at(value, &["tool_input", "command"]) {
        return crate::cue::extract_tool_target(&cmd)
            .map_err(|skip| SilenceReason::NoCues(format!("{skip:?}")));
    }
    Err(SilenceReason::PayloadUnreadable(
        "the payload carried no recognised tool input — the event shape moved".into(),
    ))
}

/// A leading byte-order mark, removed.
///
/// `str::trim` does NOT remove U+FEFF — it is categorised as format, not
/// whitespace — so a BOM survives every emptiness check and then breaks
/// `serde_json` at line 1 column 1. Windows tooling emits one routinely.
fn strip_bom(payload: &str) -> &str {
    payload.strip_prefix('\u{FEFF}').unwrap_or(payload)
}

/// The first bytes, in hex, for an error that must say what it actually saw.
///
/// "payload is not JSON" names a category and nothing else: a BOM, a stray log
/// line and a truncated write are indistinguishable in it. Nine identical lines
/// in a live silence log could not be diagnosed from the log alone, which is the
/// same unfalsifiable-error shape this codebase keeps paying for.
fn payload_prefix(payload: &str) -> String {
    let hex: Vec<String> = payload.bytes().take(8).map(|b| format!("{b:02x}")).collect();
    hex.join(" ")
}

/// THE payload parser. Every site goes through it, so tolerance and diagnostics
/// are decided once rather than six times — four of the previous sites swallowed
/// the failure with `.ok()?` and reported nothing at all.
fn parse_payload(payload: &str) -> Result<serde_json::Value, SilenceReason> {
    let payload = strip_bom(payload);
    if payload.trim().is_empty() {
        return Err(SilenceReason::PayloadUnreadable("the event payload was empty".into()));
    }
    serde_json::from_str(payload).map_err(|e| {
        SilenceReason::PayloadUnreadable(format!(
            "payload is not JSON: {e} (first bytes: {})",
            payload_prefix(payload)
        ))
    })
}

/// The non-empty string at a key path, or `None` — an empty string is an
/// absence, not a cue.
fn string_at(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(key)?;
    }
    cursor
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}


/// The stop gate. Reads the transcript the HARNESS wrote — never a marker the
/// agent writes, because skipping such a write would be passing the gate.
///
/// Fails OPEN on every unreadable condition (no payload, no path, no file),
/// but RECORDS which one: the previous generation of this hook failed open
/// silently, and a silent fail-open is indistinguishable from a pass.
fn stop_gate(client: Client, payload: &str, autonomy: crate::stop::Autonomy) -> Outcome {
    use crate::stop::{self, StopFacts, StopVerdict};

    let v = match parse_payload(payload) {
        Ok(v) => v,
        Err(reason) => return Outcome::Silent(reason),
    };
    let already_bounced = v
        .get("stop_hook_active")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let Some(path) = v.get("transcript_path").and_then(|p| p.as_str()) else {
        return Outcome::Silent(SilenceReason::NoTranscript);
    };
    // BOUNDED, and from the END. Reading the whole file was measured at 4,983
    // ms of parsing on a 330 MB session transcript — past the 4,000 ms
    // watchdog, which exits the process from its own thread BEFORE the silence
    // log is written. The gate then stayed silent and recorded nothing: the
    // exact two-week-outage signature this stage exists to end, reproduced by
    // the stage's own code.
    //
    // Only the window since the last human message is needed, so the tail is
    // sufficient — and the read is now O(1) in session length rather than
    // O(history).
    let Ok(text) = read_tail(path, TRANSCRIPT_TAIL_BYTES) else {
        return Outcome::Silent(SilenceReason::NoTranscript);
    };
    let turn = match stop::read_turn(&text) {
        Ok(t) => t,
        Err(reason) => return Outcome::Silent(reason),
    };

    // RULE B's honest position: production passes `Unknown` from `run`,
    // because nothing readable here says whether the human granted autonomy.
    // It is a PARAMETER rather than a constant so the blocking paths — and the
    // anti-loop wire that guards them — are reachable from a test. Hard-coding
    // it made `stop_hook_active` hollow: seeding that read to either constant
    // left the whole suite green, so the anti-wedge valve could be deleted and
    // nothing would notice.

    match stop::judge(&StopFacts { already_bounced, turn, autonomy }) {
        StopVerdict::Block { reason } => {
            match crate::emit::render(client, &crate::emit::Emission::StopDecision { reason }) {
                Some(rendered) => Outcome::Emitted(rendered),
                None => Outcome::Silent(SilenceReason::CannotInject),
            }
        }
        // Report WHICH allow this was. Logging `autonomy-unknown` for every
        // pass was fine only while production could not observe autonomy; the
        // moment Studio supplies it, every judged autonomous stop would file a
        // false reason.
        // Stage 5: the stop gate allows the turn — but this is also the LAST
        // moment anything can observe what the session did with the knowledge
        // it was handed. A session that took recalled knowledge and never said
        // one word about it is the failure that motivated the whole gate work,
        // and it is invisible at every earlier event, because at every earlier
        // event the agent might still speak.
        //
        // It never BLOCKS on this. The stop gate blocks on its own rules; a
        // skip is reported, and reporting it after the fact is the honest
        // shape — the agent cannot be asked to judge knowledge it has already
        // finished with.
        StopVerdict::Allow => {
            let session = session_id_in(payload).unwrap_or_default();
            if let Some(home) = home_dir() {
                let dir = home.join(".claude").join("jawata-studio");
                let ledger = crate::recallledger::verdict(&dir, &session);
                if ledger.is_skip() {
                    // RECORDED, NOT BLOCKED — and the distinction is not a
                    // detail. The Stop role's only injection shape is a BLOCK
                    // decision, which bounces the agent back into the turn; a
                    // first version of this line did exactly that, which would
                    // have wedged a session over an observation. The skip is a
                    // measurement, and the number is what decides whether it
                    // ever earns an interruption.
                    crate::observer::emit_signal(
                        &dir,
                        "recall-skipped",
                        &format!("injected={} disposed=0", ledger.injected),
                    );
                }
            }
            Outcome::Silent(match autonomy {
                crate::stop::Autonomy::Unknown => SilenceReason::AutonomyUnknown,
                _ => SilenceReason::StopAllowed,
            })
        }
    }
}

/// How much of a transcript's tail the stop gate reads. Generous enough to
/// hold many turns, small enough that parsing cannot approach the watchdog.
pub const TRANSCRIPT_TAIL_BYTES: u64 = 1_048_576;

/// Read at most `max` bytes from the END of a file, starting at a line
/// boundary so the first record is never half a line.
fn read_tail(path: &str, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let truncated = len > max;
    if truncated {
        // Seek ONE BYTE EARLIER than the window so we can tell whether the
        // boundary already fell on a record edge. Dropping the first line
        // unconditionally destroyed a COMPLETE record whenever it did —
        // measured: a 100-byte file of ten records, window 50, lost the whole
        // record at the boundary and returned four lines where five were
        // readable.
        f.seek(SeekFrom::Start(len - max - 1))?;
    }
    let mut buf = Vec::with_capacity((max + 1).min(len) as usize);
    f.take(max + 1).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if !truncated {
        return Ok(text);
    }
    // The probe byte decides. If it is a newline the window already began at a
    // record edge and everything after it is whole.
    if let Some(rest) = text.strip_prefix('\n') {
        // Even here the window may hold no COMPLETE record: the probe byte was
        // the only newline. Returning it would let read_turn parse zero records
        // and report "this turn launched nothing" — the manufactured absence
        // again, on the branch the first fix did not cover.
        // `contains` subsumes `ends_with`; the latter was dead.
        return if rest.contains('\n') {
            Ok(rest.to_string())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transcript tail holds no complete record",
            ))
        };
    }
    {
        // Otherwise the first line is a fragment and must go.
        return match text.find('\n') {
            Some(i) => Ok(text[i + 1..].to_string()),
            // No newline anywhere in the window means we could not read a
            // single whole record. Returning the fragment would let the caller
            // parse zero records and report "this turn launched nothing" — a
            // manufactured absence, which is the failure this crate exists to
            // end. Say we could not look.
            None => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transcript tail holds no complete record",
            )),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(Result<Answer, QueryError>);
    impl Store for Stub {
        fn ask(&self, _: serde_json::Value) -> Result<Answer, QueryError> {
            self.0.clone()
        }
        fn ask_value(&self, _: serde_json::Value) -> Result<serde_json::Value, QueryError> {
            // These tests exercise the TEXT path. A stub that answered the
            // structured question too would let a gate test pass against a
            // fixture nobody wrote.
            Err(QueryError::ShapeChanged("this stub answers only the text path".into()))
        }
    }

    fn config(client: &str) -> HookConfig {
        HookConfig {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "t".into(),
            client: client.into(),
            timeout_ms: Some(50),
            field_dir: None,
            recall_gate: None,
        }
    }

    #[test]
    fn a_recall_with_an_answer_injects_it() {
        let store = Stub(Ok(Answer::Text("[lesson] a line".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"the importer classifier regression"}"#,
            &store,
        );
        match out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
                assert!(ctx.contains("NOMINEES"), "the label must say what these are");
                assert!(ctx.contains("[lesson] a line"));
            }
            other => panic!("expected an emission: {other:?}"),
        }
    }

    #[test]
    fn an_unreachable_store_is_never_reported_as_an_absence() {
        // THE hazard: the resident is down. This must not end the run saying
        // "the store had nothing", which is a claim about the store.
        let store = Stub(Err(QueryError::Unreachable("connection refused".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::QueryFailed(why)) => {
                assert!(why.contains("Unreachable"), "{why}")
            }
            other => panic!("an unreachable store must be QueryFailed, got {other:?}"),
        }
    }

    #[test]
    fn a_contract_mismatch_is_its_own_reason_never_query_failed() {
        // Sprint 28b D7 (C2 audit F1): the recall path — deleting the
        // ContractMismatch arm folds this into QueryFailed and turns it red.
        let store = Stub(Err(QueryError::ContractMismatch { ours: 1, theirs: "2".into() }));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::ContractMismatch(detail)) => {
                assert!(detail.contains("ours=1") && detail.contains("theirs=2"), "{detail}");
            }
            other => panic!("a mismatch must be its own typed reason, got {other:?}"),
        }
    }

    #[test]
    fn a_contract_mismatch_on_the_primer_path_is_typed_too() {
        // The finish() arm (primer goes through finish, not the cue loop).
        let store = Stub(Err(QueryError::ContractMismatch { ours: 1, theirs: "3".into() }));
        let out = run(Role::Primer, &config("claude-code"), "{}", &store);
        match out {
            Outcome::Silent(SilenceReason::ContractMismatch(detail)) => {
                assert!(detail.contains("theirs=3"), "{detail}");
            }
            other => panic!("expected the typed mismatch on the primer path, got {other:?}"),
        }
    }

    #[test]
    fn an_observed_absence_says_so_and_is_not_a_failure() {
        let store = Stub(Ok(Answer::Nothing));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        assert_eq!(Outcome::Silent(SilenceReason::StoreHadNothing), out);
    }

    // ---- D9's reminder: it is ledgered when it is SPOKEN, and only then ----

    fn field_config(client: &str, dir: &std::path::Path) -> HookConfig {
        HookConfig {
            field_dir: Some(dir.to_string_lossy().into_owned()),
            ..config(client)
        }
    }

    fn primer_scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!("jawata-primer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A field directory that genuinely OWES a reminder: unposted failures in
    /// the pile, no ledger yet, neither switch set.
    fn seed_owed_reminder(dir: &std::path::Path) {
        let mut pile = String::from("{\"pileFormat\":1,\"contract\":1}\n");
        for _ in 0..3 {
            pile.push_str(
                "{\"t\":1,\"tool\":\"run_tests\",\"kind\":\"run\",\"ok\":false,\
                 \"code\":\"RUNNER_TIMEOUT\",\"lat\":5,\"client\":\"claude_code\",\
                 \"ver\":\"3_11_0\"}\n",
            );
        }
        std::fs::write(dir.join("pile.jsonl"), pile).unwrap();
        assert!(
            crate::field::reminder_due(dir, 40 * crate::field::REMINDER_INTERVAL_MILLIS)
                .is_some(),
            "the fixture must actually owe a reminder, or the assertions below prove nothing"
        );
    }

    /// 28b closing audit, F3 — the reminder was SWALLOWED and burned its
    /// weekly slot anyway. It was prepended to the primer's HEADING, and
    /// `finish` discards the heading when the store answers `Nothing`, but
    /// `record_reminded` had already fired. Observed live:
    /// `role primer: SILENT [store-had-nothing]` while `reminded.log` gained a
    /// `shown` line — the user was told nothing, the ledger said he was, and
    /// the next reminder was blocked for seven days.
    #[test]
    fn a_reminder_owed_still_reaches_the_user_when_the_store_has_nothing() {
        let dir = primer_scratch("empty-store");
        seed_owed_reminder(&dir);
        let store = Stub(Ok(Answer::Nothing));
        let out = run(Role::Primer, &field_config("claude-code", &dir), "{}", &store);
        match &out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(s).unwrap();
                let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
                assert!(
                    ctx.contains("/report") && ctx.contains("failed tool calls"),
                    "the reminder itself is the injected context when the primer has \
                     nothing else to say: {ctx}"
                );
                assert!(
                    !ctx.contains("domain primer"),
                    "and it does not carry a heading for an answer that does not exist: {ctx}"
                );
            }
            other => panic!("a reminder that is owed must reach the user: {other:?}"),
        }
        let (last_shown, strikes) = crate::field::reminder_ledger(&dir);
        assert!(last_shown > 0, "a SPOKEN reminder is ledgered");
        assert_eq!(1, strikes, "and counts one unanswered strike");
    }

    /// The other half: a reminder the client never saw is still OWED. Anything
    /// that stops the primer emitting must leave the ledger untouched — the
    /// exact discipline `observer.rs` already follows for the nudge
    /// ("Recorded ONLY on a real emission").
    #[test]
    fn a_reminder_the_user_never_saw_is_not_ledgered() {
        let dir = primer_scratch("suppressed");
        seed_owed_reminder(&dir);
        // The store never answered. The primer stays silent with the FAILURE as
        // its reason — the dead-channel fold's numerator depends on that
        // classification, so the reminder cannot claim the slot here.
        let store = Stub(Err(QueryError::Unreachable("connection refused".into())));
        let out = run(Role::Primer, &field_config("claude-code", &dir), "{}", &store);
        assert!(
            matches!(out, Outcome::Silent(SilenceReason::QueryFailed(_))),
            "the store failure stays the reason: {out:?}"
        );
        assert_eq!(
            (0, 0),
            crate::field::reminder_ledger(&dir),
            "nothing was spoken, so nothing is recorded and the reminder is still owed"
        );
    }

    /// And the ordinary path still works: the store answered, so the reminder
    /// rides the primer's own opening and IS ledgered.
    #[test]
    fn a_reminder_rides_a_primer_that_has_something_to_say() {
        let dir = primer_scratch("with-answer");
        seed_owed_reminder(&dir);
        let store = Stub(Ok(Answer::Text("[domain] this codebase indexes ledgers".into())));
        let out = run(Role::Primer, &field_config("claude-code", &dir), "{}", &store);
        match &out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(s).unwrap();
                let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
                assert!(ctx.contains("/report"), "the reminder is there: {ctx}");
                assert!(ctx.contains("indexes ledgers"), "and so is the primer: {ctx}");
            }
            other => panic!("expected both: {other:?}"),
        }
        assert_eq!(1, crate::field::reminder_ledger(&dir).1);
    }

    /// Cursor's prompt hook cannot inject context — the role table says so.
    /// That is quiet BY DESIGN, and it must not read as a dead channel: with
    /// `cannot-inject` here, every Cursor machine's user-prompt channel folded
    /// as permanently dead — a built-in false alarm (C2 audit F2).
    #[test]
    fn cursor_queries_the_prompt_hook_but_emits_nothing() {
        let store = Stub(Ok(Answer::Text("[lesson] a line".into())));
        let out = run(
            Role::UserPrompt,
            &config("cursor"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        assert_eq!(Outcome::Silent(SilenceReason::RecordedNotInjected), out);
        assert!(
            crate::field::LEGITIMATELY_QUIET.contains(&"recorded-not-injected"),
            "and the fold must classify it as quiet, not dead"
        );
    }

    /// C2 audit F1: the store ANSWERED and the answer was unusable — the
    /// historic two-week outage's own mechanism. It is its own reason and
    /// counts toward the dead-channel condition; `query-failed` (nothing
    /// answered at all) does not.
    #[test]
    fn an_answered_but_unusable_reply_is_not_query_failed() {
        for (error, expected) in [
            (QueryError::ShapeChanged("data was null".into()), "ShapeChanged"),
            (
                QueryError::ToolRefused { code: "BAD".into(), message: "m".into() },
                "ToolRefused:BAD",
            ),
        ] {
            match run(
                Role::UserPrompt,
                &config("claude-code"),
                r#"{"prompt":"importer classifier regression"}"#,
                &Stub(Err(error)),
            ) {
                Outcome::Silent(SilenceReason::AnswerUnusable(detail)) => {
                    assert_eq!(expected, detail)
                }
                other => panic!("an answered-but-unusable reply must be typed: {other:?}"),
            }
        }
        // …while never reaching the store keeps its own name.
        match run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"importer classifier regression"}"#,
            &Stub(Err(QueryError::Unreachable("refused".into()))),
        ) {
            Outcome::Silent(SilenceReason::QueryFailed(_)) => {}
            other => panic!("an unreachable store is not an unusable answer: {other:?}"),
        }
    }

    #[test]
    fn a_role_absent_on_the_client_says_so() {
        let store = Stub(Ok(Answer::Text("x".into())));
        assert_eq!(
            Outcome::Silent(SilenceReason::RoleAbsentOnClient),
            run(Role::Stop, &config("cursor"), "{}", &store)
        );
    }

    #[test]
    fn a_moved_payload_is_named_not_treated_as_an_empty_prompt() {
        let store = Stub(Ok(Answer::Text("x".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"userMessage":"the field was renamed"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::PayloadUnreadable(why)) => {
                assert!(why.contains("shape moved"), "{why}")
            }
            other => panic!("a moved payload must name itself: {other:?}"),
        }
    }

    #[test]
    fn a_slash_command_is_skipped_with_the_cue_modules_reason() {
        let store = Stub(Ok(Answer::Text("x".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"/memorize this"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::NoCues(why)) => assert!(why.contains("SlashCommand")),
            other => panic!("expected NoCues: {other:?}"),
        }
    }

    #[test]
    fn the_whole_pipeline_survives_every_shape_without_panicking() {
        // The boundary catches panics; not needing it is better.
        let stubs = [
            Ok(Answer::Text("x".into())),
            Ok(Answer::Nothing),
            Err(QueryError::Status(503)),
            Err(QueryError::ShapeChanged("data moved".into())),
            Err(QueryError::ToolRefused { code: "X".into(), message: "y".into() }),
        ];
        let payloads = ["", "{}", "not json", r#"{"prompt":""}"#, r#"{"prompt":"a b c"}"#];
        for stub in stubs {
            for payload in payloads {
                for client in ["claude-code", "cursor", "windsurf"] {
                    for role in [Role::Primer, Role::UserPrompt, Role::Guard, Role::Stop] {
                        let out = run(role, &config(client), payload, &Stub(stub.clone()));
                        // C5 audit F7: `len() > 3` made this a panic smoke
                        // test wearing an assertion, and the Emitted arm was
                        // never checked at all. Both arms now carry a real
                        // obligation — anything we emit must be JSON the
                        // client can read, and anything silent must name a
                        // cause a log could print.
                        match out {
                            Outcome::Emitted(text) => {
                                serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(
                                    |e| panic!("emitted non-JSON for {role:?}/{client}: {e}\n{text}"),
                                );
                                assert!(!text.contains('\n'), "an emission must be one line");
                            }
                            Outcome::Silent(reason) => {
                                // C5 audit round 2, R5: "non-empty and contains
                                // an uppercase char" is true of every derived
                                // Debug on a PascalCase enum — it could not
                                // fail. The real obligation is that the reason
                                // FITS: a run that never reached the store must
                                // not claim the store had nothing, which is the
                                // specific lie this crate exists to stop.
                                if matches!(reason, SilenceReason::StoreHadNothing) {
                                    assert!(
                                        matches!(stub, Ok(Answer::Nothing)),
                                        "reported StoreHadNothing for {role:?}/{client} with \
                                         payload {payload:?} while the store answered \
                                         {stub:?} — that is a claim about the store this run \
                                         never earned"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

        use crate::stop::{Autonomy, StopFacts, StopVerdict, Turn, ToolUse};

    /// THE WIRE TEST. Seeding `Role::Stop => stop_gate(...)` back to
    /// `CannotInject` left all eleven suites green — the gate was HOLLOW, the
    /// very shape this sprint exists to catch, one hour after building the
    /// detector for it. Every assertion below goes through `run`, so the arm
    /// in the match is load-bearing.
    fn transcript(body: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("jawata-stopwire-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join(format!("t-{}.jsonl", body.len()));
        std::fs::write(&p, body).unwrap();
        p
    }


    #[test]
    fn stop_reaches_the_gate_through_run() {
        // A turn with no communicator and nothing armed. Autonomy is Unknown in
        // production today, so the honest outcome is the RECORDED reason — not
        // a block, and not the inherited CannotInject the hollow arm produced.
        let p = transcript(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n",
        );
        // serde_json, NOT format!. A Windows path is C:\Users\... and a raw
        // backslash in a JSON string is an ESCAPE — the payload parsed on Linux
        // and was invalid JSON on Windows, where these tests had never run until
        // the crate's tests became a CI gate.
        let payload = serde_json::json!({
            "transcript_path": p, "stop_hook_active": false
        }).to_string();
        let out = run(Role::Stop, &config("claude-code"), &payload, &Stub(Ok(Answer::Nothing)));
        assert_eq!(
            Outcome::Silent(SilenceReason::AutonomyUnknown),
            out,
            "Stop must reach the gate and record why it did not enforce"
        );
    }

    #[test]
    fn a_stop_payload_without_a_transcript_names_itself_through_run() {
        let out = run(Role::Stop, &config("claude-code"), "{}", &Stub(Ok(Answer::Nothing)));
        assert_eq!(Outcome::Silent(SilenceReason::NoTranscript), out);
    }

    /// The gate's blocking path renders the third dialect. Driven at the
    /// judge+emit seam because production cannot yet produce `Granted`.
    #[test]
    fn a_block_renders_claudes_stop_dialect() {
        let facts = StopFacts {
            already_bounced: false,
            turn: Turn { final_text: "summary".into(), launches: vec![], refusals_emitted: 0, asks_the_human: false, user_asked: false, degraded_consumed: 0, seats_invoked: vec![], gate_ran: true },
            autonomy: Autonomy::Granted,
        };
        let StopVerdict::Block { reason } = crate::stop::judge(&facts) else {
            panic!("must block");
        };
        let rendered = crate::emit::render(
            crate::roles::Client::ClaudeCode,
            &crate::emit::Emission::StopDecision { reason },
        )
        .expect("claude renders a stop decision");
        let v: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!("block", v["decision"], "got {rendered}");
        assert!(v["reason"].as_str().unwrap().contains("communicator"));
    }

    /// Cursor has no Stop event, so the dialect must render to nothing at all
    /// rather than to an empty object a client could read as a decision.
    #[test]
    fn cursor_renders_no_stop_decision() {
        assert_eq!(
            None,
            crate::emit::render(
                crate::roles::Client::Cursor,
                &crate::emit::Emission::StopDecision { reason: "x".into() }
            )
        );
    }

    #[test]
    fn the_communicator_never_counts_as_armed_work() {
        let c = ToolUse { name: "Agent".into(), subagent: Some("communicator".into()), backgrounded: false };
        assert!(!c.arms_work(), "else the two rules cancel each other out");
    }

    /// HOLLOW-WIRE FIX. A sweep that seeded every arm of `run` found the guard
    /// arm load-bearing for NOTHING: `guard::judge` is well unit-tested, but no
    /// test drove it through the pipeline, so the arm could be deleted and all
    /// 132 tests stayed green. Production reaches it; a regression would not
    /// have been caught.
    #[test]
    fn the_guard_reaches_its_verdict_through_run() {
        let out = run(
            Role::Guard,
            &config("claude-code"),
            r#"{"tool_input":{"command":"grep -rn 'foo' src/main/java/Thing.java"}}"#,
            &Stub(Ok(Answer::Nothing)),
        );
        match out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!("deny", v["hookSpecificOutput"]["permissionDecision"], "got {s}");
            }
            other => panic!("the guard must decide through run: {other:?}"),
        }
    }

    /// Same sweep, same hole: every existing test passed `Role::UserPrompt`,
    /// which shares an arm with `ToolRecall`, so the recall role was never
    /// itself exercised. Asserting the EVENT NAME pins the role rather than
    /// merely the shared code path.
    #[test]
    fn tool_recall_reaches_the_store_through_run() {
        let store = Stub(Ok(Answer::Text("[lesson] a line".into())));
        let out = run(
            Role::ToolRecall,
            &config("claude-code"),
            r#"{"tool_input":{"file_path":"src/main/java/com/example/Importer.java"}}"#,
            &store,
        );
        match out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!("PreToolUse", v["hookSpecificOutput"]["hookEventName"], "got {s}");
            }
            other => panic!("expected a PreToolUse recall: {other:?}"),
        }
    }

    /// 3.7.2 dogfood F2, pinned at the run() level: an ABSOLUTE path must
    /// recall, not be skipped as a slash command. The existing test above used
    /// a relative path, which is why the bug lived through it.
    #[test]
    fn tool_recall_on_an_absolute_path_queries_the_type_symbol() {
        struct SymbolAsserting;
        impl Store for SymbolAsserting {
            fn ask(&self, args: serde_json::Value) -> Result<Answer, QueryError> {
                assert_eq!("ProjectImporter", args["symbol"], "the .java stem is the cue");
                Ok(Answer::Text("[lesson] a line".into()))
            }
            fn ask_value(&self, _: serde_json::Value) -> Result<serde_json::Value, QueryError> {
                Err(QueryError::ShapeChanged("text path only".into()))
            }
        }
        let out = run(
            Role::ToolRecall,
            &config("claude-code"),
            r#"{"tool_input":{"file_path":"/home/u/org/jawata/core/ProjectImporter.java"}}"#,
            &SymbolAsserting,
        );
        assert!(matches!(out, Outcome::Emitted(_)), "an absolute path must recall: {out:?}");
    }

    /// The subject-key priority carried over from the script generation: a
    /// rename carrying `symbol` AND `newName` queries the OLD name.
    #[test]
    fn tool_recall_prefers_the_subject_key_over_new_name_and_path() {
        struct SymbolAsserting;
        impl Store for SymbolAsserting {
            fn ask(&self, args: serde_json::Value) -> Result<Answer, QueryError> {
                assert_eq!("com.example.Old#field", args["symbol"]);
                Ok(Answer::Text("[hazard] a line".into()))
            }
            fn ask_value(&self, _: serde_json::Value) -> Result<serde_json::Value, QueryError> {
                Err(QueryError::ShapeChanged("text path only".into()))
            }
        }
        let out = run(
            Role::ToolRecall,
            &config("claude-code"),
            r#"{"tool_input":{"newName":"renamed","symbol":"com.example.Old#field","file_path":"/x/Y.java"}}"#,
            &SymbolAsserting,
        );
        assert!(matches!(out, Outcome::Emitted(_)), "{out:?}");
    }

    /// And the TYPED slash-command skip still holds for the prompt role —
    /// the fix must not have widened UserPrompt.
    #[test]
    fn user_prompt_still_skips_slash_commands_after_the_path_fix() {
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"/sprint resume"}"#,
            &Stub(Ok(Answer::Text("x".into()))),
        );
        match out {
            Outcome::Silent(SilenceReason::NoCues(why)) => assert!(why.contains("SlashCommand")),
            other => panic!("expected NoCues(SlashCommand): {other:?}"),
        }
    }

    /// Sprint 28b D8: the observer arm is PORTED — an uneventful payload is
    /// quiet with its own honest reason (nothing-to-observe, legitimately
    /// quiet in the reach fold), never the stub's cannot-inject that read as
    /// a permanently dead channel (C2 audit F2).
    #[test]
    fn the_observer_stays_silent_through_run_and_says_why() {
        let payload = serde_json::json!({
            "tool_name": "search_symbols", "session_id": "s", "tool_input": {}
        })
        .to_string();
        assert_eq!(
            Outcome::Silent(SilenceReason::NothingToObserve),
            run(Role::Observer, &config("claude-code"), &payload, &Stub(Ok(Answer::Nothing)))
        );
    }

    /// N1: the window boundary landing EXACTLY on a record edge used to destroy
    /// the whole record that began there — the unconditional first-line drop.
    #[test]
    fn a_window_boundary_on_a_record_edge_keeps_every_record() {
        let d = std::env::temp_dir().join(format!("jawata-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("edge.jsonl");
        // Ten 10-byte records; a 50-byte window falls exactly on an edge.
        let body: String = (0..10).map(|i| format!("L{i:08}\n")).collect();
        std::fs::write(&p, &body).unwrap();
        let got = read_tail(p.to_str().unwrap(), 50).expect("reads");
        assert_eq!(5, got.lines().count(), "five whole records fit the window: {got:?}");
        assert!(got.starts_with("L00000005"), "the edge record must survive: {got:?}");
    }

    /// N2: a window holding no complete record used to come back as an empty
    /// turn — a MANUFACTURED absence, read downstream as "this turn launched
    /// nothing". It must say it could not look.
    #[test]
    fn a_window_with_no_complete_record_is_an_error_not_an_empty_turn() {
        let d = std::env::temp_dir().join(format!("jawata-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("no-newline.jsonl");
        std::fs::write(&p, "x".repeat(500)).unwrap();
        assert!(
            read_tail(p.to_str().unwrap(), 50).is_err(),
            "an unreadable window must not become a positive 'nothing happened'"
        );
    }

    #[test]
    fn a_file_smaller_than_the_window_is_returned_whole() {
        let d = std::env::temp_dir().join(format!("jawata-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("small.jsonl");
        std::fs::write(&p, "a\nb\nc\n").unwrap();
        assert_eq!("a\nb\nc\n", read_tail(p.to_str().unwrap(), 4096).expect("reads"));
    }

    /// F5: the anti-loop wire. `stop_hook_active` is read from the payload and
    /// feeds `judge`'s short-circuit, but seeding that read to either constant
    /// used to leave all 140 tests green — the valve that stops the gate
    /// wedging a session could be deleted unnoticed. These two drive the same
    /// transcript through `stop_gate` under Granted, differing ONLY in the JSON
    /// key, and must disagree.
    #[test]
    fn the_anti_loop_flag_is_read_from_the_payload() {
        let d = std::env::temp_dir().join(format!("jawata-antiloop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("t.jsonl");
        std::fs::write(
            &p,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n",
        )
        .unwrap();

        let first = serde_json::json!({"transcript_path": p, "stop_hook_active": false}).to_string();
        let again = serde_json::json!({"transcript_path": p, "stop_hook_active": true}).to_string();

        let blocked = stop_gate(Client::ClaudeCode, &first, crate::stop::Autonomy::Granted);
        assert!(
            matches!(blocked, Outcome::Emitted(_)),
            "first pass under autonomy must block: {blocked:?}"
        );
        let allowed = stop_gate(Client::ClaudeCode, &again, crate::stop::Autonomy::Granted);
        assert!(
            !matches!(allowed, Outcome::Emitted(_)),
            "a second pass must NOT block again — that wedges the session: {allowed:?}"
        );
    }


    /// F2: the probe-is-newline branch was correct but UNFORCED — mutating its
    /// guard to `if true` left all 141 tests green. The existing manufactured-
    /// absence test uses a window with no newline at all, so it takes the other
    /// branch.
    #[test]
    fn a_window_whose_only_newline_is_the_probe_byte_is_an_error() {
        let d = std::env::temp_dir().join(format!("jawata-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("probe.jsonl");
        // 60 bytes: a newline exactly at the probe position, then no other.
        let mut body = "A".repeat(9);
        body.push('\n');
        body.push_str(&"B".repeat(50));
        std::fs::write(&p, &body).unwrap();
        assert!(
            read_tail(p.to_str().unwrap(), 50).is_err(),
            "a window holding no COMPLETE record must not become an empty turn"
        );
    }

    #[test]
    fn a_probe_newline_followed_by_a_complete_record_is_read() {
        let d = std::env::temp_dir().join(format!("jawata-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("probe-ok.jsonl");
        let body = format!("{}\n{}\n", "A".repeat(9), "B".repeat(40));
        std::fs::write(&p, &body).unwrap();
        let got = read_tail(p.to_str().unwrap(), 50).expect("reads");
        assert!(got.starts_with('B'), "the whole record must survive: {got:?}");
    }

    /// F3: the autonomy -> reason branch was unforced — reverting it to always
    /// log `autonomy-unknown` left all 141 tests green, so the moment Studio
    /// supplies real autonomy every judged stop would file a false reason.
    #[test]
    fn a_judged_allow_is_not_logged_as_autonomy_unknown() {
        let d = std::env::temp_dir().join(format!("jawata-judged-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("t.jsonl");
        std::fs::write(
            &p,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"x\"}]}}\n",
        )
        .unwrap();
        let payload = serde_json::json!({"transcript_path": p, "stop_hook_active": false}).to_string();

        assert_eq!(
            Outcome::Silent(SilenceReason::StopAllowed),
            stop_gate(Client::ClaudeCode, &payload, crate::stop::Autonomy::NotGranted),
            "a KNOWN autonomy that allows must not claim it was unknown"
        );
        assert_eq!(
            Outcome::Silent(SilenceReason::AutonomyUnknown),
            stop_gate(Client::ClaudeCode, &payload, crate::stop::Autonomy::Unknown),
            "and unknown must still say unknown"
        );
    }

    /// F4 (re-pinned at 28b D8): the Observer table and the code must not
    /// drift apart. The rows now declare the PORTED capability — a store
    /// query (the slip bridge + edit-feed posts) on both clients, an emission
    /// only where the client can inject (Claude Code's PostToolUse; Cursor's
    /// afterMCPExecution cannot) — and the pipeline actually makes them: a
    /// slip payload emits the steering on Claude Code and is recorded-not-
    /// injected on Cursor.
    #[test]
    fn the_observer_table_matches_what_the_pipeline_does() {
        for client in [Client::ClaudeCode, Client::Cursor] {
            let spec = crate::roles::spec(Role::Observer, client).expect("observer row");
            assert!(spec.concerns.query, "the port bridges slips to the store");
        }
        assert!(crate::roles::spec(Role::Observer, Client::ClaudeCode).unwrap().can_inject);
        assert!(!crate::roles::spec(Role::Observer, Client::Cursor).unwrap().can_inject);
        // An unreadable payload is quiet with its own reason — never a crash,
        // never the dead-channel numerator.
        assert_eq!(
            Outcome::Silent(SilenceReason::PayloadUnreadable("observer payload".into())),
            run(Role::Observer, &config("claude-code"), "", &Stub(Ok(Answer::Nothing)))
        );
    }

}

#[cfg(test)]
mod payload_parsing_tests {
    use super::*;

    /// A byte-order mark must not make a payload unreadable.
    ///
    /// Nine lines of a live Windows silence log said "payload is not JSON:
    /// expected value at line 1 column 1" — every user-prompt and every stop
    /// invocation, so recall and the stop gate were dead on that platform while
    /// the install looked complete. Reproduced by prefixing a BOM: `str::trim`
    /// does not remove U+FEFF (it is format, not whitespace), so it survives the
    /// emptiness check and breaks serde at the first column.
    #[test]
    fn a_byte_order_mark_does_not_make_a_payload_unreadable() {
        let plain = r#"{"prompt":"hello"}"#;
        let with_bom = format!("\u{FEFF}{plain}");

        let a = parse_payload(plain).expect("plain payload parses");
        let b = parse_payload(&with_bom).expect("a BOM-prefixed payload must parse too");
        assert_eq!(a, b, "the BOM must not change the parsed value");
    }

    /// An empty payload keeps its OWN diagnosis, rather than being reported as
    /// malformed JSON. They have different causes and different fixes.
    #[test]
    fn an_empty_payload_is_still_reported_as_empty() {
        for empty in ["", "   ", "\u{FEFF}", "\u{FEFF}  "] {
            match parse_payload(empty) {
                Err(SilenceReason::PayloadUnreadable(m)) => {
                    assert!(m.contains("empty"), "{empty:?} should read as empty, got {m}")
                }
                other => panic!("{empty:?} must be unreadable-empty, got {other:?}"),
            }
        }
    }

    /// A genuinely malformed payload NAMES WHAT IT SAW.
    ///
    /// "payload is not JSON" alone is unfalsifiable: a BOM, a stray log line and
    /// a truncated write all produce it, and a live log full of them could not
    /// be diagnosed without reproducing the bug locally — which is exactly what
    /// this codebase keeps paying for.
    #[test]
    fn a_malformed_payload_reports_the_bytes_it_saw() {
        match parse_payload("not json at all") {
            Err(SilenceReason::PayloadUnreadable(m)) => {
                assert!(m.contains("first bytes:"), "must name the bytes: {m}");
                assert!(m.contains("6e"), "'n' of \"not\" is 0x6e: {m}");
            }
            other => panic!("expected an unreadable payload, got {other:?}"),
        }
    }

    // ---- jawata-mcp#37: our deadline must reach the store -------------------

    use std::time::Duration;

    // ---- Stage 5: the skip signal ----------------------------------------

    /// The skip is an OBSERVATION and must never bounce the agent back into a
    /// turn. The Stop role's only injection shape is a block decision, and the
    /// first version of this reached for it — which would have wedged a session
    /// over a measurement. This pins the rule the code now follows.
    #[test]
    fn a_recall_skip_is_recorded_and_never_blocks_the_stop() {
        let home = std::env::temp_dir().join(format!("jawata-skip-{}", std::process::id()));
        let dir = home.join(".claude").join("jawata-studio");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&dir).unwrap();

        // A session that took knowledge and said nothing.
        crate::recallledger::record_injected(&dir, "s-skip");
        crate::recallledger::record_injected(&dir, "s-skip");
        let v = crate::recallledger::verdict(&dir, "s-skip");
        assert!(v.is_skip(), "fixture must actually be a skip: {v:?}");

        // The signal is a LOG LINE, not a decision: emitting it must not
        // produce anything the client would read as "block".
        crate::observer::emit_signal(&dir, "recall-skipped", "injected=2 disposed=0");
        let log = std::fs::read_to_string(dir.join("outcomes.log")).unwrap();
        assert!(log.contains("recall-skipped"), "the skip must be recorded: {log}");
        assert!(
            !log.contains("\"decision\"") && !log.to_lowercase().contains("block"),
            "the skip signal must not carry a stop decision: {log}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// The false-accusation guard, at the level that matters: a session which
    /// disposed of what it was given is not a skip, so nothing is recorded.
    #[test]
    fn a_session_that_answered_leaves_no_skip() {
        let home = std::env::temp_dir().join(format!("jawata-noskip-{}", std::process::id()));
        let dir = home.join(".claude").join("jawata-studio");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&dir).unwrap();

        crate::recallledger::record_injected(&dir, "s-ok");
        crate::recallledger::record_disposition(&dir, "s-ok", "recall-applied");
        assert!(!crate::recallledger::verdict(&dir, "s-ok").is_skip());

        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- Stage 4: the recall gate, WIRED ---------------------------------

    struct GateStore {
        anchor: &'static str,
        asked_structured: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Store for GateStore {
        fn ask(&self, _: serde_json::Value) -> Result<Answer, QueryError> {
            Ok(Answer::Text("[lesson] the ordinary injection".into()))
        }
        fn ask_value(&self, args: serde_json::Value) -> Result<serde_json::Value, QueryError> {
            self.asked_structured.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!("recall", args["kind"], "the gate asks the store for a recall");
            Ok(serde_json::json!({
                "result": "match",
                "entries": [{"symbol": self.anchor, "summary": "the pool already resolves these"}]
            }))
        }
    }

    fn member_call() -> &'static str {
        r#"{"tool_input":{"symbol":"com.example.Importer#addDependencyEntries"}}"#
    }

    /// This module's own config — the sibling test module's helper is private
    /// to it, and reaching across would couple two test modules for one line.
    fn gate_config(client: &str) -> HookConfig {
        HookConfig {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "t".into(),
            client: client.into(),
            timeout_ms: Some(50),
            field_dir: None,
            recall_gate: None,   // absent = Observe, the shipping default
        }
    }

    fn gate_store(asked: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> GateStore {
        GateStore { anchor: "com.example.Importer#addDependencyEntries", asked_structured: asked }
    }

    /// THE WIRING. `recallgate::judge` is unit-tested on its own, but a pure
    /// function nothing calls is this repository's recorded failure shape — a
    /// release shipped its headline inert exactly that way. So this drives
    /// `run()` and asserts the gate was REACHED: the structured question is one
    /// only the gate asks.
    #[test]
    fn the_gate_is_on_the_tool_recall_path_not_merely_present() {
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let _ = run(Role::ToolRecall, &gate_config("claude-code"), member_call(), &gate_store(asked.clone()));
        assert_eq!(
            1,
            asked.load(std::sync::atomic::Ordering::SeqCst),
            "the gate never asked the store — it is present but not wired"
        );
    }

    /// OBSERVE IS THE SHIPPING DEFAULT, and observe does not block.
    #[test]
    fn observe_mode_records_but_lets_the_call_through() {
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        match run(Role::ToolRecall, &gate_config("claude-code"), member_call(), &gate_store(asked)) {
            Outcome::Emitted(text) => assert!(
                text.contains("ordinary injection"),
                "observe must fall through to the normal recall, got: {text}"
            ),
            other => panic!("observe must not hold the call: {other:?}"),
        }
    }

    /// A USER PROMPT IS NOT A TOOL CALL. The gate's soundness claim is that the
    /// record's anchor IS the symbol the call is about; a typed prompt has no
    /// such subject, so the gate must not run there.
    #[test]
    fn the_gate_does_not_run_on_the_user_prompt_role() {
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let _ = run(
            Role::UserPrompt,
            &gate_config("claude-code"),
            r#"{"prompt":"what about com.example.Importer and the bundle pool"}"#,
            &gate_store(asked.clone()),
        );
        assert_eq!(
            0,
            asked.load(std::sync::atomic::Ordering::SeqCst),
            "the gate ran on a typed prompt — it belongs to tool calls only"
        );
    }

    /// FAIL OPEN, against the shape jawata-mcp#37 introduced: an unreadable
    /// knowledge layer must never hold a tool call.
    #[test]
    fn an_unavailable_store_never_holds_the_call() {
        struct Outage;
        fn outage() -> QueryError {
            QueryError::ToolRefused {
                code: "KNOWLEDGE_UNAVAILABLE".into(),
                message: "the store did not answer within 1200ms".into(),
            }
        }
        impl Store for Outage {
            fn ask(&self, _: serde_json::Value) -> Result<Answer, QueryError> {
                Err(outage())
            }
            fn ask_value(&self, _: serde_json::Value) -> Result<serde_json::Value, QueryError> {
                Err(outage())
            }
        }
        match run(Role::ToolRecall, &gate_config("claude-code"), member_call(), &Outage) {
            Outcome::Silent(_) => {}
            Outcome::Emitted(text) => assert!(
                !text.contains("JAWATA GATE"),
                "an outage must never produce a gate hold: {text}"
            ),
        }
    }

    #[test]
    fn the_store_is_told_our_deadline_and_it_fires_before_ours() {
        let mut args = serde_json::json!({"kind": "recall", "symbol": "com.example.Foo#bar"});
        with_budget(&mut args, Duration::from_millis(1_500));
        let budget = args["budget_ms"].as_u64().expect("the budget must travel with the ask");

        assert!(
            budget < 1_500,
            "a store deadline at or past ours is unreachable: its typed answer would arrive \
             after this process has already given up, which is the state #37 was filed about"
        );
        assert!(budget >= BUDGET_FLOOR_MILLIS, "and not so tight it cuts off a healthy read");
    }

    #[test]
    fn an_absurdly_short_timeout_still_asks_for_a_workable_budget() {
        // saturating_sub would otherwise hand the store a 0 ms deadline, and a
        // store told to answer in no time answers UNAVAILABLE every time — a
        // manufactured outage, which is worse than the timeout it replaced.
        let mut args = serde_json::json!({"kind": "recall"});
        with_budget(&mut args, Duration::from_millis(50));
        assert_eq!(BUDGET_FLOOR_MILLIS, args["budget_ms"].as_u64().unwrap());
    }

    #[test]
    fn every_live_ask_carries_the_budget_no_call_site_can_forget_it() {
        // The injection is in `LiveStore::ask`, not at the call sites, so this
        // asserts the seam rather than one caller's discipline. The URL is
        // unroutable on purpose: we are asserting what was SENT, not an answer.
        let live = LiveStore(Endpoint {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "t".into(),
            timeout: Duration::from_millis(1_500),
        });
        // `ask` fails (nothing is listening) — the point is that it fails as an
        // Unreachable transport error, having gone through the budget seam,
        // rather than never reaching it.
        assert!(matches!(
            live.ask(serde_json::json!({"kind": "recall"})),
            Err(QueryError::Unreachable(_))
        ));
        let mut args = serde_json::json!({"kind": "recall"});
        with_budget(&mut args, live.0.timeout);
        assert!(args["budget_ms"].is_number());
    }
}
