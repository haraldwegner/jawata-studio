//! The studio's read of the field recording (Sprint 28b, D2 + D10 + D6).
//!
//! Everything here is a FOLD over files somebody else owns — the resident's
//! `<workspace>/field/` pile and state, the hook's per-install
//! `hook_silence.log`, and the observer's `outcomes.log`. This module writes
//! exactly one file, `state.json`, and writes it the way the deploy writes
//! `hook_config.json`: temp file, then rename, preserving every key it was not
//! asked to change. Three processes touch that lane (the hook reads it, the
//! agent writes it through `FieldTool`, and studio writes it from the tile), so
//! a plain truncating write here would clobber whichever switch the other two
//! set last.
//!
//! **This module deliberately re-implements the hook's fold instead of calling
//! it.** `jawata-hook` is a sibling crate that must never link a GUI toolkit,
//! and `jawata-hook/tests/dependency_edges.rs` asserts that studio does not
//! depend on it. The shared thing is the FILE FORMAT, not a Rust symbol; the
//! copy is small and the seeded fixtures below pin the same shapes the hook's
//! own tests pin, so a format change fails on both sides rather than silently
//! on one.
//!
//! Nothing in here can interrupt the user. The view is passive: it renders what
//! the files say, the tile carries two switches, and a failing canary changes a
//! colour. That is the whole surface — see `interruption_scans` at the foot of this file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many times an unposted shape must recur before it counts toward the
/// badge. The same threshold the hook nudges on — the badge and the nudge must
/// never disagree about what is "worth reporting".
pub const NUDGE_THRESHOLD: u64 = 3;

/// Suppression tags that mean "the store answered and the channel still
/// delivered nothing" — the dead-channel numerator.
///
/// PUBLIC so the two folds can be pinned to each other: the hook's
/// `tests/the_two_folds_classify_the_same_tags.rs` asserts this list against
/// its own. The duplication is required — studio must not link the hook crate
/// — but nothing made the two lists agree until that test existed.
pub const ANSWERED_BUT_SUPPRESSED: &[&str] =
    &["cannot-inject", "contract-mismatch", "answer-unusable"];

/// Tags that mean quiet was the CORRECT outcome. A channel that only ever
/// carries these is quiet by design, never dead — the misclassification that
/// once marked every Cursor observer permanently broken.
pub const LEGITIMATELY_QUIET: &[&str] = &[
    "store-had-nothing",
    "no-cues",
    "stop-allowed",
    "role-absent-on-client",
    "recorded-not-injected",
    "nothing-to-observe",
];

/// How far back the DEAD verdict is allowed to look: seven days.
///
/// Without a window the condition has no present tense, and an append-only log
/// never forgets. This machine's `hook_silence.log` carries 175 `cannot-inject`
/// observer rows, every one written on 2026-08-09 by a stub retired nine days
/// later, and the fold called that channel dead forever — the same class of
/// built-in false alarm the C2 F2 amendment removed for by-design quiet,
/// arriving through the other door (28b closing audit, F6).
///
/// Seven days, and not arbitrarily: the verdict feeds the same surface the D9
/// reminder speaks on at most weekly, so the evidence window and the cadence
/// at which the user hears about it are one period. Shorter would call a
/// channel unknown after a weekend away; longer keeps convicting on behaviour
/// already replaced. Older rows stay in `fired`/`emitted`/`suppressed` — they
/// are history, which is what the counters are for; they just no longer
/// convict. The hook holds the same constant, for the same reason, in
/// `jawata-hook/src/field.rs`.
pub const DEAD_CHANNEL_WINDOW_MILLIS: u64 = 7 * 24 * 60 * 60 * 1000;

/// R1, shown beside the number and never only in a sprint document: the
/// denominator is hook-scoped.
pub const UTILIZATION_CAVEAT: &str = "Hook-scoped number. jawata can only see a shell \
fallback in a client where one of its hooks sits in the session — Claude Code and Cursor \
today. Work done in any other client counts jawata's own half and nothing against it, so \
the real share of shell text tools is at least this high, never lower. Sprint 28f closes \
the denominator by observing every command from the principal seat.";

// ---------------------------------------------------------------------------
// The pile
// ---------------------------------------------------------------------------

/// One recurring failure shape: `<tool>/<kind>/<code>`, with what the pile is
/// allowed to carry about it. Shapes only — the pile has no paths, no symbol
/// names and no message text to give us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorShape {
    pub shape: String,
    pub tool: String,
    pub kind: String,
    pub code: String,
    pub count: u64,
    /// Already filed — the nudge stops and the badge stops counting it.
    pub posted: bool,
    /// Which clients hit it, deduped. A shape only one client sees is a
    /// different bug from one every client sees.
    pub clients: Vec<String>,
    /// Which jawata versions hit it — a shape that stops at a version is fixed.
    pub versions: Vec<String>,
    /// Worst latency bucket seen for this shape.
    pub worst_latency_bucket: u64,
}

/// What one `pile.jsonl` says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PileFold {
    /// False when the file is absent — which is "nothing has been recorded
    /// here yet", a different statement from "zero failures".
    pub present: bool,
    /// The versioned header's `contract`, when the header is there.
    pub contract: Option<u64>,
    pub total_events: u64,
    pub failures: u64,
    pub successes: u64,
    /// Ranked by recurrence, highest first; ties break on the shape name so the
    /// view never reorders itself between two identical reads.
    pub shapes: Vec<ErrorShape>,
    /// Unposted shapes at or over [`NUDGE_THRESHOLD`] — the badge.
    pub badge: u64,
    /// Lines that did not parse. Surfaced rather than swallowed: a pile that is
    /// half garbage is a recorder bug, and a fold that hides it is how the
    /// recorder bug survives.
    pub unreadable_lines: u64,
}

/// Fold a pile's text. `posted` marks the shapes already filed.
pub fn fold_pile(text: &str, posted: &[String]) -> PileFold {
    let mut fold = PileFold {
        present: true,
        ..Default::default()
    };
    let mut by_shape: BTreeMap<String, ErrorShape> = BTreeMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(contract) = number_field(line, "\"contract\":") {
            if line.contains("\"pileFormat\":") {
                fold.contract = Some(contract);
                continue;
            }
        }
        if line.contains("\"pileFormat\":") {
            continue; // a header we could not read the contract out of
        }
        let (Some(tool), Some(kind), Some(code)) = (
            string_field(line, "\"tool\":\""),
            string_field(line, "\"kind\":\""),
            string_field(line, "\"code\":\""),
        ) else {
            fold.unreadable_lines += 1;
            continue;
        };
        fold.total_events += 1;
        if line.contains("\"ok\":true") {
            fold.successes += 1;
            continue;
        }
        fold.failures += 1;
        let shape = format!("{tool}/{kind}/{code}");
        let client = string_field(line, "\"client\":\"").unwrap_or_default();
        let version = string_field(line, "\"ver\":\"").unwrap_or_default();
        let latency = number_field(line, "\"lat\":").unwrap_or(0);
        let entry = by_shape.entry(shape.clone()).or_insert_with(|| ErrorShape {
            shape: shape.clone(),
            tool,
            kind,
            code,
            count: 0,
            posted: posted.iter().any(|p| p == &shape),
            clients: Vec::new(),
            versions: Vec::new(),
            worst_latency_bucket: 0,
        });
        entry.count += 1;
        entry.worst_latency_bucket = entry.worst_latency_bucket.max(latency);
        if !client.is_empty() && !entry.clients.contains(&client) {
            entry.clients.push(client);
        }
        if !version.is_empty() && !entry.versions.contains(&version) {
            entry.versions.push(version);
        }
    }

    let mut shapes: Vec<ErrorShape> = by_shape.into_values().collect();
    shapes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.shape.cmp(&b.shape)));
    fold.badge = shapes
        .iter()
        .filter(|s| !s.posted && s.count >= NUDGE_THRESHOLD)
        .count() as u64;
    fold.shapes = shapes;
    fold
}

/// Fold the pile at `<field_dir>/pile.jsonl`. A missing file is `present:
/// false`, never a fabricated zero.
pub fn fold_pile_file(field_dir: &Path, posted: &[String]) -> PileFold {
    match std::fs::read_to_string(field_dir.join("pile.jsonl")) {
        Ok(text) => fold_pile(&text, posted),
        Err(_) => PileFold::default(),
    }
}

// ---------------------------------------------------------------------------
// The two switches + the reminder ledger
// ---------------------------------------------------------------------------

/// The state file, as the hook reads it. Field names and the compact encoding
/// are load-bearing: the hook decides by substring (`"silenced":true`), so a
/// pretty-printed or renamed variant of this file reads as "not set".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldState {
    /// The in-session one-line pointer at `/report`. ON by default.
    #[serde(default = "switch_on")]
    pub nudges: bool,
    /// The periodic reminder the agent speaks. OFF by default (i.e. reminders
    /// on) — this is the go-silent checkbox.
    #[serde(default)]
    pub silenced: bool,
    #[serde(default)]
    pub reminded_at: u64,
    #[serde(default)]
    pub strikes: u64,
    #[serde(default)]
    pub posted: Vec<String>,
}

fn switch_on() -> bool {
    true
}

impl Default for FieldState {
    fn default() -> Self {
        // ABSENCE IS NOT A SWITCH. No state file means the user has never
        // touched either control, which is "everything on" — the hook makes the
        // same call from the other side.
        Self {
            nudges: true,
            silenced: false,
            reminded_at: 0,
            strikes: 0,
            posted: Vec::new(),
        }
    }
}

/// Read `<field_dir>/state.json`. Missing or unparseable folds to defaults.
pub fn read_state(field_dir: &Path) -> FieldState {
    std::fs::read_to_string(field_dir.join("state.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<FieldState>(&text).ok())
        .unwrap_or_default()
}

/// Set one or both switches, atomically, preserving every key we were not
/// asked to change — including keys this struct does not know about.
///
/// Temp file plus rename, mirroring `write_hook_config`: a reader during a
/// truncating write sees an empty file, and the hook reads this one on every
/// prompt. `rename` within a directory is atomic on every platform we ship, so
/// the hook sees either the whole old file or the whole new one.
pub fn write_state(
    field_dir: &Path,
    nudges: Option<bool>,
    silenced: Option<bool>,
) -> Result<FieldState, String> {
    std::fs::create_dir_all(field_dir)
        .map_err(|e| format!("cannot create {}: {e}", field_dir.display()))?;
    let target = field_dir.join("state.json");

    // Merge into the RAW document, not into FieldState: a key the resident
    // added and this build has never heard of must survive a checkbox click.
    let mut doc = std::fs::read_to_string(&target)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let defaults = FieldState::default();
    doc.entry("nudges")
        .or_insert_with(|| serde_json::Value::Bool(defaults.nudges));
    doc.entry("silenced")
        .or_insert_with(|| serde_json::Value::Bool(defaults.silenced));
    doc.entry("remindedAt").or_insert_with(|| 0.into());
    doc.entry("strikes").or_insert_with(|| 0.into());
    doc.entry("posted")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let Some(value) = nudges {
        doc.insert("nudges".into(), serde_json::Value::Bool(value));
    }
    if let Some(value) = silenced {
        doc.insert("silenced".into(), serde_json::Value::Bool(value));
    }

    let body = serde_json::Value::Object(doc.clone()).to_string();
    let tmp = field_dir.join(format!("state.json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, &body).map_err(|e| format!("failed staging {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("failed publishing {}: {e}", target.display())
    })?;
    serde_json::from_value(serde_json::Value::Object(doc))
        .map_err(|e| format!("wrote {} but could not read it back: {e}", target.display()))
}

/// EXACTLY what a first click of the go-silent checkbox leaves on disk.
///
/// This is a CONTRACT, not a formatting detail. The hook decides by substring
/// on this file and neither crate may depend on the other, so the two sides are
/// pinned to the same literal from opposite ends: this constant is asserted to
/// be what `write_state` produces (below), and
/// `jawata-hook/tests/the_studio_checkbox_silences_the_reminder.rs` asserts
/// that these exact bytes stop `field::reminder_due`. A change on either side
/// fails a test on both. Same mechanism as the silence log's cap.
/// TEST-SIDE BY CONSTRUCTION: it is an assertion fixture, not a capability —
/// production writes this shape through `write_state` and never reads a literal
/// of it. The hook's half of the pin lives in a test file for the same reason.
#[cfg(test)]
pub const CHECKBOX_SILENCED_STATE: &str =
    "{\"nudges\":true,\"posted\":[],\"remindedAt\":0,\"silenced\":true,\"strikes\":0}";

/// The reminder ledger: `<millis>\tshown|reset`, append-only, folded at read.
/// Returns (last shown, strikes since the last reset, reminders ever shown).
pub fn reminder_ledger(field_dir: &Path) -> (u64, u64, u64) {
    let Ok(content) = std::fs::read_to_string(field_dir.join("reminded.log")) else {
        return (0, 0, 0);
    };
    let (mut last_shown, mut strikes, mut shown) = (0u64, 0u64, 0u64);
    for line in content.lines() {
        let mut parts = line.splitn(2, '\t');
        let (Some(at), Some(kind)) = (parts.next(), parts.next()) else {
            continue;
        };
        match kind.trim() {
            "shown" => {
                last_shown = at.trim().parse().unwrap_or(last_shown);
                strikes += 1;
                shown += 1;
            }
            "reset" => strikes = 0,
            _ => {}
        }
    }
    (last_shown, strikes, shown)
}

/// Shapes the hook has already nudged about, from the append-only `nudged.log`.
pub fn nudged_shapes(field_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(field_dir.join("nudged.log"))
        .map(|c| {
            c.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The `/report` tile's whole state — the two switches, WITH the reason the
/// reminder is or is not speaking, and the history behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatLaneState {
    /// The seat this tile is for. 28f fills the rest of the lane.
    pub seat: String,
    /// The in-session pointer switch (D4). Distinct from `silenced`.
    pub nudges: bool,
    /// The periodic reminder's go-silent checkbox (D9).
    pub silenced: bool,
    /// Why the reminder is in the state it is in — "off by your choice" when
    /// the checkbox is ticked, "on" when it is not. Never inferred by the view.
    pub reminder_reason: String,
    pub strikes: u64,
    pub reminders_shown: u64,
    pub last_reminded_at_millis: u64,
    pub nudged_shapes: Vec<String>,
    pub posted_shapes: Vec<String>,
    /// False when no state file exists yet — both switches are then defaults,
    /// not choices, and the tile says so.
    pub state_file_present: bool,
}

/// The two reason strings. Extracted so the view cannot invent a third.
pub fn reminder_reason(silenced: bool) -> String {
    if silenced {
        "off by your choice".to_string()
    } else {
        "on".to_string()
    }
}

/// Build the `/report` tile's state from the files.
pub fn seat_lane_state(field_dir: &Path) -> SeatLaneState {
    let state = read_state(field_dir);
    let (last_shown, strikes, shown) = reminder_ledger(field_dir);
    SeatLaneState {
        seat: "/report".to_string(),
        nudges: state.nudges,
        silenced: state.silenced,
        reminder_reason: reminder_reason(state.silenced),
        // The ledger is the truth about strikes; the state file's copy is a
        // convenience the hook does not write.
        strikes,
        reminders_shown: shown,
        last_reminded_at_millis: last_shown,
        nudged_shapes: nudged_shapes(field_dir),
        posted_shapes: state.posted.clone(),
        state_file_present: field_dir.join("state.json").exists(),
    }
}

// ---------------------------------------------------------------------------
// Reach counters: the dead-channel check
// ---------------------------------------------------------------------------

/// One channel's folded counters, plus the two verdicts derived from them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelReach {
    pub role: String,
    pub fired: u64,
    pub emitted: u64,
    pub suppressed: BTreeMap<String, u64>,
    /// The store answered and nothing came out — the two-week-outage signature.
    pub dead: bool,
    /// Nothing came out and every suppression was a legitimate absence.
    pub legitimately_quiet: bool,
}

/// Fold a `hook_silence.log` (`<millis>\t<role>\t<tag>\t<detail>`) into
/// per-role counters. A half-written line loses itself, never the fold.
///
/// `now_millis` anchors [`DEAD_CHANNEL_WINDOW_MILLIS`]: a row older than the
/// window, or one whose timestamp cannot be read, counts in the history and
/// never toward the `dead` verdict.
pub fn fold_silence_log(text: &str, now_millis: u64) -> Vec<ChannelReach> {
    // The verdict's working set, kept beside the per-role history rather than
    // in it: the counters are what the tile SHOWS, and they are all-time.
    let mut by_role: BTreeMap<String, ChannelReach> = BTreeMap::new();
    let mut recent: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.splitn(4, '\t');
        let (Some(millis), Some(role), Some(tag)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if role.is_empty() || tag.is_empty() {
            continue;
        }
        let is_recent = millis
            .trim()
            .parse::<u64>()
            .map(|at| now_millis.saturating_sub(at) <= DEAD_CHANNEL_WINDOW_MILLIS)
            .unwrap_or(false);
        let entry = by_role.entry(role.to_string()).or_insert_with(|| ChannelReach {
            role: role.to_string(),
            ..Default::default()
        });
        let window = recent.entry(role.to_string()).or_insert((0, 0));
        entry.fired += 1;
        if tag == "emitted" {
            entry.emitted += 1;
            if is_recent {
                window.0 += 1;
            }
        } else {
            *entry.suppressed.entry(tag.to_string()).or_insert(0) += 1;
            if is_recent && ANSWERED_BUT_SUPPRESSED.contains(&tag) {
                window.1 += 1;
            }
        }
    }
    by_role
        .into_values()
        .map(|mut reach| {
            let answered: u64 = ANSWERED_BUT_SUPPRESSED
                .iter()
                .filter_map(|t| reach.suppressed.get(*t))
                .sum();
            let (recent_emitted, recent_answered) =
                recent.get(&reach.role).copied().unwrap_or((0, 0));
            reach.dead = recent_emitted == 0 && recent_answered > 0;
            reach.legitimately_quiet = reach.emitted == 0
                && answered == 0
                && reach
                    .suppressed
                    .keys()
                    .all(|t| LEGITIMATELY_QUIET.contains(&t.as_str()));
            reach
        })
        .collect()
}

/// Fold every install's silence log into one per-role view. Two installs
/// (Claude Code's dir and Cursor's) write separate files for the same roles;
/// the view is per machine, so they merge.
pub fn fold_silence_logs(paths: &[PathBuf], now_millis: u64) -> Vec<ChannelReach> {
    let mut merged = String::new();
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(path) {
            merged.push_str(&text);
            if !merged.ends_with('\n') {
                merged.push('\n');
            }
        }
    }
    fold_silence_log(&merged, now_millis)
}

// ---------------------------------------------------------------------------
// Utilization: jawata vs the shell, with its caveat attached
// ---------------------------------------------------------------------------

/// The observer's `outcomes.log` signals, folded. `slip` is a declared
/// `jawata-fallback:` the guard allowed; `read-ungrounded` is a `.java` read
/// with no jawata lookup behind it. Both are Java work that did not go through
/// the compiler-aware layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeSignals {
    pub present: bool,
    pub slips: u64,
    pub ungrounded_reads: u64,
    pub verifications: u64,
}

pub fn fold_outcomes(text: &str) -> OutcomeSignals {
    let mut signals = OutcomeSignals {
        present: true,
        ..Default::default()
    };
    for line in text.lines() {
        // `<iso-ts>\t<jawata-ver>\t<signal>\t<detail>`
        let signal = line.splitn(4, '\t').nth(2).unwrap_or("").trim();
        match signal {
            "slip" => signals.slips += 1,
            "read-ungrounded" => signals.ungrounded_reads += 1,
            "verify" => signals.verifications += 1,
            _ => {}
        }
    }
    signals
}

pub fn fold_outcomes_file(path: &Path) -> OutcomeSignals {
    match std::fs::read_to_string(path) {
        Ok(text) => fold_outcomes(&text),
        Err(_) => OutcomeSignals::default(),
    }
}

/// Where the agent used jawata and where it reached for the shell instead.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Utilization {
    /// Tool calls jawata recorded — its own half.
    pub jawata_calls: u64,
    /// Shell fallbacks the hook SAW: slips plus ungrounded reads.
    pub shell_fallbacks: u64,
    pub slips: u64,
    pub ungrounded_reads: u64,
    /// jawata's share of what was observed, 0–100, rounded to one decimal.
    /// `None` when nothing was observed at all — an empty denominator is not
    /// 100 %, and printing one would be the exact lie this sprint exists to end.
    pub percent: Option<f64>,
    /// R1, always carried WITH the number.
    pub caveat: String,
    /// False when the observer has never written — the number is then jawata's
    /// half only, and the view must say so rather than imply zero fallbacks.
    pub observer_present: bool,
}

pub fn utilization(jawata_calls: u64, signals: &OutcomeSignals) -> Utilization {
    let shell_fallbacks = signals.slips + signals.ungrounded_reads;
    let denominator = jawata_calls + shell_fallbacks;
    Utilization {
        jawata_calls,
        shell_fallbacks,
        slips: signals.slips,
        ungrounded_reads: signals.ungrounded_reads,
        percent: if denominator == 0 {
            None
        } else {
            Some(((jawata_calls as f64 * 1000.0) / denominator as f64).round() / 10.0)
        },
        caveat: UTILIZATION_CAVEAT.to_string(),
        observer_present: signals.present,
    }
}

// ---------------------------------------------------------------------------
// The canary
// ---------------------------------------------------------------------------

/// One resident's canary reading: a real recall and a real compiler question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryResult {
    pub workspace: String,
    pub url: String,
    pub recall_ok: bool,
    pub recall_detail: String,
    pub compiler_ok: bool,
    pub compiler_detail: String,
    pub green: bool,
    /// The resident answered the compiler question CORRECTLY, with
    /// `PROJECT_LOADING` — it is still importing. Not green, and not a failure.
    pub loading: bool,
    /// When this workspace was first seen loading in an unbroken run of loading
    /// rounds. A single round cannot know it; [`stitch_loading_runs`] carries it.
    /// `None` whenever the workspace is not loading.
    pub loading_since_millis: Option<u64>,
    pub checked_at_millis: u64,
}

/// The fixture the compiler question asks about: the one type every Java
/// workspace on earth can resolve. A canary needs a target that is present
/// without anyone setting it up, and cannot be answered from a cache the
/// compiler layer is not part of.
pub const CANARY_FIXTURE_TYPE: &str = "java.lang.String";

/// The recall cue. Deliberately a cue nothing will match: an AUTHORITATIVE
/// ABSENCE is a passing answer — what fails is a store that cannot answer at
/// all. Asking something that must match would make the canary depend on the
/// user's store contents.
pub const CANARY_RECALL_SYMPTOM: &str = "jawata studio canary probe";

/// The ONE error code that means "still importing, ask again later" rather than
/// "broken". `ToolResponse.projectLoading()` emits it. Deliberately an
/// allow-list of exactly one: `PROJECT_LOAD_FAILED` and `PROJECT_NOT_LOADED`
/// arrive in the same `Ok(value)` envelope and are real failures — treating any
/// error body as loading would paint a FAILED workspace healthy forever.
pub const LOADING_ERROR_CODE: &str = "PROJECT_LOADING";

/// How long a workspace may keep saying "loading" before the tray stops giving
/// it the benefit of the doubt. A cold start of 33 projects measured ~5 minutes;
/// three canary rounds is generous and still bounded, so a load that never
/// finishes cannot masquerade as health.
pub const LOADING_GRACE_MILLIS: u64 = 15 * 60 * 1000;

/// jawata-mcp#37's error code, as the resident spells it.
pub const KNOWLEDGE_UNAVAILABLE_CODE: &str = "KNOWLEDGE_UNAVAILABLE";

/// The message of a `KNOWLEDGE_UNAVAILABLE` body, or `None` for any other answer.
///
/// Keyed on the CODE rather than on `success:false`, because the two are not the
/// same fact: a refusal we caused (a bad parameter) says nothing about the store's
/// health, while this one says the layer could not be read.
fn unavailable_code(value: &serde_json::Value) -> Option<String> {
    let error = value.pointer("/error")?;
    if error.get("code")?.as_str()? != KNOWLEDGE_UNAVAILABLE_CODE {
        return None;
    }
    Some(
        error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or(KNOWLEDGE_UNAVAILABLE_CODE)
            .to_string(),
    )
}

/// Judge one resident from the two answers. Pure — the HTTP lives in
/// `manager_service`, so the verdict is testable with no resident, no agent
/// session and no network.
pub fn judge_canary(
    workspace: &str,
    url: &str,
    recall: Result<serde_json::Value, String>,
    compiler: Result<serde_json::Value, String>,
    now_millis: u64,
) -> CanaryResult {
    let (recall_ok, recall_detail) = match recall {
        // The store answered in its own envelope. A `success:false` body is
        // still an ANSWER — the store spoke; only silence is degradation.
        //
        // WITH ONE EXCEPTION, and it is the one this canary exists for
        // (jawata-mcp#37). `KNOWLEDGE_UNAVAILABLE` is the resident saying, in
        // its own words, that the knowledge layer could not be read: a wedged
        // connection, or a degraded in-memory fallback standing in for the real
        // corpus. Counting that as "the store spoke" would paint the tray GREEN
        // during exactly the outage the canary was built to catch — and the
        // better the engine reports its own trouble, the more thoroughly the
        // dashboard would hide it.
        Ok(value) if unavailable_code(&value).is_some() => (
            false,
            format!(
                "the store could not answer: {}",
                unavailable_code(&value).unwrap_or_default()
            ),
        ),
        Ok(value) => (
            value.is_object(),
            if value.is_object() {
                "the store answered".to_string()
            } else {
                "the store answered in a shape studio does not know".to_string()
            },
        ),
        Err(error) => (false, error),
    };
    let mut loading = false;
    let (compiler_ok, compiler_detail) = match compiler {
        Ok(value) => {
            let resolved = value
                .pointer("/data/source")
                .or_else(|| value.pointer("/data/sourceLength"))
                .or_else(|| value.pointer("/data/typeName"))
                .is_some();
            // A resident that is still IMPORTING answers this question
            // correctly — with PROJECT_LOADING — and that is not a compiler
            // failure. Exactly this code and no other.
            loading = !resolved
                && value.pointer("/error/code").and_then(|c| c.as_str()) == Some(LOADING_ERROR_CODE);
            (
                resolved,
                if resolved {
                    format!("the compiler resolved {CANARY_FIXTURE_TYPE}")
                } else if loading {
                    "the resident is still loading its projects".to_string()
                } else {
                    format!("the resident answered but could not resolve {CANARY_FIXTURE_TYPE}")
                },
            )
        }
        Err(error) => (false, error),
    };
    CanaryResult {
        workspace: workspace.to_string(),
        url: url.to_string(),
        recall_ok,
        recall_detail,
        compiler_ok,
        compiler_detail,
        green: recall_ok && compiler_ok,
        loading,
        // A single round cannot know when the run started; the board stitches it.
        loading_since_millis: None,
        checked_at_millis: now_millis,
    }
}

/// Carry a LOADING run forward across rounds, so the grace period is measured
/// from when loading STARTED and not from the latest probe. A workspace already
/// loading keeps its original stamp; one that has just started gets `now`; one
/// that stopped loading drops it. This is what bounds [`CanaryHealth::Loading`].
pub fn stitch_loading_runs(
    previous: &[CanaryResult],
    current: &mut [CanaryResult],
    now_millis: u64,
) {
    for result in current.iter_mut() {
        if !result.loading {
            result.loading_since_millis = None;
            continue;
        }
        let carried = previous
            .iter()
            .find(|p| p.workspace == result.workspace && p.loading)
            .and_then(|p| p.loading_since_millis);
        result.loading_since_millis = Some(carried.unwrap_or(now_millis));
    }
}

/// The dashboard's and the tray's shared verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CanaryHealth {
    /// Nothing has been probed yet — never rendered as green.
    Unknown,
    Green,
    /// Every not-green resident answered correctly and is still importing, and
    /// has not been doing so past [`LOADING_GRACE_MILLIS`]. A cold start of a
    /// large workspace takes minutes; that is not a fault to alarm about.
    Loading,
    Degraded,
}

/// The shared verdict. `now_millis` is needed because LOADING is only innocent
/// while it is young — past the grace period an unfinished load is a fault.
pub fn canary_health(results: &[CanaryResult], now_millis: u64) -> CanaryHealth {
    if results.is_empty() {
        return CanaryHealth::Unknown;
    }
    if results.iter().all(|r| r.green) {
        return CanaryHealth::Green;
    }
    let not_green: Vec<&CanaryResult> = results.iter().filter(|r| !r.green).collect();
    let all_loading_and_fresh = not_green.iter().all(|r| {
        // `loading` is set from the COMPILER answer alone. A resident whose STORE is dead
        // is also not green, and without this second clause its dead store would be
        // excused for the whole grace period by the fact that its compiler happened to say
        // "still importing" — silencing half of what the canary exists to watch.
        r.loading
            && r.recall_ok
            && match r.loading_since_millis {
                Some(since) => now_millis.saturating_sub(since) < LOADING_GRACE_MILLIS,
                // FAIL CLOSED. Production always stitches before judging (publish_canary),
                // so an unstitched value here means the wiring is gone — and treating it as
                // innocent would make LOADING unbounded again with every test still green.
                None => false,
            }
    });
    if all_loading_and_fresh {
        CanaryHealth::Loading
    } else {
        CanaryHealth::Degraded
    }
}

// ---------------------------------------------------------------------------
// The whole status
// ---------------------------------------------------------------------------

/// One workspace's field recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldWorkspaceStatus {
    pub workspace: String,
    pub field_dir: String,
    pub pile: PileFold,
    pub lane: SeatLaneState,
}

/// What the field view renders. Machine-level facts at the top (the reach
/// counters and the utilization number are per install, not per workspace),
/// per-workspace piles below.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldStatus {
    pub utilization: Utilization,
    pub channels: Vec<ChannelReach>,
    pub dead_channels: Vec<String>,
    pub legitimately_quiet_channels: Vec<String>,
    /// The silence logs that were actually read. Empty means no hook has ever
    /// run here — which is its own finding, not a healthy zero.
    pub silence_logs_read: Vec<String>,
    pub workspaces: Vec<FieldWorkspaceStatus>,
    /// Machine-wide badge: unposted recurring shapes across every workspace.
    pub badge: u64,
    pub canary: Vec<CanaryResult>,
    pub canary_health: CanaryHealth,
}

/// Assemble the status from explicit paths — no `dirs::home_dir()`, no globals,
/// so the whole thing is drivable from a seeded temp directory.
///
/// `now_millis` is passed in for the same reason the paths are: the
/// dead-channel verdict now depends on WHEN the rows were written, so a
/// seeded fixture must be able to name its own present. Production hands it
/// [`now_millis()`].
pub fn status_from(
    workspaces: &[(String, PathBuf)],
    silence_logs: &[PathBuf],
    outcomes_log: &Path,
    canary: Vec<CanaryResult>,
    now_millis: u64,
) -> FieldStatus {
    let folded: Vec<FieldWorkspaceStatus> = workspaces
        .iter()
        .map(|(name, field_dir)| {
            let lane = seat_lane_state(field_dir);
            let pile = fold_pile_file(field_dir, &lane.posted_shapes);
            FieldWorkspaceStatus {
                workspace: name.clone(),
                field_dir: field_dir.to_string_lossy().to_string(),
                pile,
                lane,
            }
        })
        .collect();

    let channels = fold_silence_logs(silence_logs, now_millis);
    let signals = fold_outcomes_file(outcomes_log);
    let jawata_calls = folded.iter().map(|w| w.pile.total_events).sum();

    FieldStatus {
        utilization: utilization(jawata_calls, &signals),
        dead_channels: channels
            .iter()
            .filter(|c| c.dead)
            .map(|c| c.role.clone())
            .collect(),
        legitimately_quiet_channels: channels
            .iter()
            .filter(|c| c.legitimately_quiet)
            .map(|c| c.role.clone())
            .collect(),
        silence_logs_read: silence_logs
            .iter()
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        badge: folded.iter().map(|w| w.pile.badge).sum(),
        canary_health: canary_health(&canary, now_millis),
        canary,
        channels,
        workspaces: folded,
    }
}

/// The per-install silence logs on THIS machine: one beside each client's hook
/// binaries. Absent files are kept in the list so `status_from` can report which
/// of them existed.
pub fn silence_log_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        home.join(".claude")
            .join("jawata-studio")
            .join("hook_silence.log"),
        home.join(".cursor").join("hooks").join("hook_silence.log"),
    ]
}

/// The observer's log — one per machine, written by whichever client observed.
pub fn outcomes_log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("jawata-studio")
        .join("outcomes.log")
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------

fn string_field(line: &str, key: &str) -> Option<String> {
    let from = line.find(key)? + key.len();
    let to = line[from..].find('"')? + from;
    Some(line[from..to].to_string())
}

fn number_field(line: &str, key: &str) -> Option<u64> {
    let from = line.find(key)? + key.len();
    let rest = &line[from..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jawata-field-view-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The seeded pile the whole stage is proven against: one shape over the
    /// threshold, one shape at the threshold that is already posted, one shape
    /// below it, one success, and one garbage line.
    fn seeded_pile() -> String {
        let mut pile = String::from("{\"pileFormat\":1,\"contract\":1}\n");
        for _ in 0..5 {
            pile.push_str(
                "{\"t\":1,\"tool\":\"run_tests\",\"kind\":\"run\",\"ok\":false,\
                 \"code\":\"RUNNER_TIMEOUT\",\"lat\":4,\"client\":\"claude_code\",\"ver\":\"3_10_0\"}\n",
            );
        }
        for _ in 0..3 {
            pile.push_str(
                "{\"t\":1,\"tool\":\"inspect\",\"kind\":\"source\",\"ok\":false,\
                 \"code\":\"TYPE_NOT_FOUND\",\"lat\":1,\"client\":\"cursor\",\"ver\":\"3_10_0\"}\n",
            );
        }
        pile.push_str(
            "{\"t\":1,\"tool\":\"search_symbols\",\"kind\":\"query\",\"ok\":false,\
             \"code\":\"INDEX_COLD\",\"lat\":2,\"client\":\"claude_code\",\"ver\":\"3_10_0\"}\n",
        );
        pile.push_str(
            "{\"t\":1,\"tool\":\"run_tests\",\"kind\":\"run\",\"ok\":true,\
             \"code\":\"unknown\",\"lat\":1,\"client\":\"claude_code\",\"ver\":\"3_10_0\"}\n",
        );
        pile.push_str("half a line, lost to a crash\n");
        pile
    }

    #[test]
    fn the_fold_ranks_shapes_by_recurrence_and_keeps_the_header() {
        let fold = fold_pile(&seeded_pile(), &[]);
        assert!(fold.present);
        assert_eq!(Some(1), fold.contract, "the versioned header is read");
        assert_eq!(10, fold.total_events, "9 failures + 1 success");
        assert_eq!(9, fold.failures);
        assert_eq!(1, fold.successes);
        assert_eq!(1, fold.unreadable_lines, "the torn line is COUNTED, not hidden");

        let ranked: Vec<&str> = fold.shapes.iter().map(|s| s.shape.as_str()).collect();
        assert_eq!(
            vec![
                "run_tests/run/RUNNER_TIMEOUT",
                "inspect/source/TYPE_NOT_FOUND",
                "search_symbols/query/INDEX_COLD",
            ],
            ranked
        );
        let top = &fold.shapes[0];
        assert_eq!(5, top.count);
        assert_eq!("run_tests", top.tool);
        assert_eq!(vec!["claude_code".to_string()], top.clients);
        assert_eq!(4, top.worst_latency_bucket);
        assert_eq!(vec!["cursor".to_string()], fold.shapes[1].clients);
    }

    #[test]
    fn the_badge_counts_unposted_shapes_at_or_over_the_threshold() {
        let fold = fold_pile(&seeded_pile(), &[]);
        assert_eq!(2, fold.badge, "5x and 3x qualify; the 1x shape does not");

        // Filing one takes it off the badge — the same rule that stops the nudge.
        let after = fold_pile(&seeded_pile(), &["run_tests/run/RUNNER_TIMEOUT".to_string()]);
        assert_eq!(1, after.badge);
        assert!(after.shapes[0].posted, "and it renders as posted, not hidden");
    }

    #[test]
    fn a_missing_pile_is_absent_not_a_healthy_zero() {
        let fold = fold_pile_file(&scratch("nopile"), &[]);
        assert!(!fold.present, "absence must be distinguishable from zero");
        assert_eq!(0, fold.total_events);
    }

    // ---- the two switches ----

    #[test]
    fn setting_one_switch_preserves_the_other_and_everything_else() {
        let dir = scratch("switches");
        std::fs::write(
            dir.join("state.json"),
            "{\"nudges\":false,\"silenced\":false,\"remindedAt\":77,\"strikes\":2,\
             \"posted\":[\"a/b/C\"],\"futureKeyStudioNeverHeardOf\":42}",
        )
        .unwrap();

        // Tick the go-silent checkbox. The no-nudges switch, the ledger fields,
        // the posted list and the unknown key all survive.
        let after = write_state(&dir, None, Some(true)).unwrap();
        assert!(after.silenced, "the checkbox took");
        assert!(!after.nudges, "the OTHER switch was not touched");
        assert_eq!(77, after.reminded_at);
        assert_eq!(vec!["a/b/C".to_string()], after.posted);
        let raw = std::fs::read_to_string(dir.join("state.json")).unwrap();
        assert!(raw.contains("\"futureKeyStudioNeverHeardOf\":42"), "{raw}");

        // And the reverse: flipping nudges back on leaves the checkbox ticked.
        let after = write_state(&dir, Some(true), None).unwrap();
        assert!(after.nudges);
        assert!(after.silenced, "the two switches are independent");
    }

    /// The hook decides by SUBSTRING on this file. A pretty-printed or
    /// space-separated encoding reads to it as "not set", so the encoding is a
    /// contract, not a formatting preference.
    #[test]
    fn the_state_file_stays_in_the_shape_the_hook_reads() {
        let dir = scratch("shape");
        write_state(&dir, Some(false), Some(true)).unwrap();
        let raw = std::fs::read_to_string(dir.join("state.json")).unwrap();
        assert!(raw.contains("\"nudges\":false"), "{raw}");
        assert!(raw.contains("\"silenced\":true"), "{raw}");
        assert!(!raw.contains('\n'), "one line, no pretty printing: {raw}");
        assert!(!dir.join(format!("state.json.{}.tmp", std::process::id())).exists(),
            "the staging file is renamed away, never left behind");
    }

    /// HALF ONE OF THE FULL LOOP. A first click of the go-silent checkbox, on a
    /// machine with no state file yet, leaves exactly these bytes. The other
    /// half — that these bytes actually stop the reminder — is asserted from
    /// inside the hook crate, which is the only place `reminder_due` can be
    /// called from; see `CHECKBOX_SILENCED_STATE`.
    #[test]
    fn the_checkbox_writes_the_bytes_the_hook_reads_as_silence() {
        let dir = scratch("loop");
        write_state(&dir, None, Some(true)).unwrap();
        assert_eq!(
            CHECKBOX_SILENCED_STATE,
            std::fs::read_to_string(dir.join("state.json")).unwrap(),
            "the bytes the hook's half of this loop is pinned to have moved"
        );
    }

    #[test]
    fn no_state_file_means_both_switches_are_on_by_default() {
        let lane = seat_lane_state(&scratch("defaults"));
        assert!(lane.nudges);
        assert!(!lane.silenced);
        assert_eq!("on", lane.reminder_reason);
        assert!(!lane.state_file_present, "and the tile can say it is a default");
    }

    #[test]
    fn the_lane_carries_the_reminder_reason_and_its_history() {
        let dir = scratch("lane");
        std::fs::write(dir.join("pile.jsonl"), seeded_pile()).unwrap();
        std::fs::write(
            dir.join("state.json"),
            "{\"nudges\":true,\"silenced\":true,\"remindedAt\":0,\"strikes\":0,\
             \"posted\":[\"inspect/source/TYPE_NOT_FOUND\"]}",
        )
        .unwrap();
        std::fs::write(dir.join("reminded.log"), "100\tshown\n200\tshown\n250\treset\n300\tshown\n")
            .unwrap();
        std::fs::write(dir.join("nudged.log"), "run_tests/run/RUNNER_TIMEOUT\n").unwrap();

        let lane = seat_lane_state(&dir);
        assert_eq!("off by your choice", lane.reminder_reason, "the REASON, not just the flag");
        assert!(lane.nudges, "silencing reminders does not switch off nudges");
        assert_eq!(1, lane.strikes, "the reset zeroed the run; one shown since");
        assert_eq!(3, lane.reminders_shown);
        assert_eq!(300, lane.last_reminded_at_millis);
        assert_eq!(vec!["run_tests/run/RUNNER_TIMEOUT".to_string()], lane.nudged_shapes);
        assert_eq!(vec!["inspect/source/TYPE_NOT_FOUND".to_string()], lane.posted_shapes);
    }

    // ---- reach counters ----

    /// A plausible "now" the seeded rows are written just before. The
    /// timestamps must be RELATIVE to it: the verdict has a recency window,
    /// and a fixed 2023 constant would make every assertion below pass for
    /// the wrong reason — nothing recent, therefore nothing dead.
    const NOW: u64 = 1_700_000_000_000;

    fn seeded_silence_log() -> String {
        let at = NOW - 1000;
        let mut log = String::new();
        // The two-week outage's signature: answered every time, emitted never.
        for _ in 0..3 {
            log.push_str(&format!("{at}\tuser-prompt\tcannot-inject\t\n"));
        }
        log.push_str(&format!("{at}\ttool-recall\temitted\t\n"));
        log.push_str(&format!("{at}\ttool-recall\tstore-had-nothing\t\n"));
        log.push_str(&format!("{at}\tprimer\temitted\t\n"));
        log.push_str(&format!("{at}\tguard\tstore-had-nothing\t\n"));
        log.push_str(&format!("{at}\tobserver\tnothing-to-observe\t\n"));
        log.push_str(&format!("{at}\tuser-prompt\tanswer-unusable\tShapeChanged\n"));
        log
    }

    /// 28b closing audit, F6 — a RETIRED outage is not a dead channel today.
    ///
    /// This machine's own `hook_silence.log` carries 175 `cannot-inject`
    /// observer rows, all written on 2026-08-09 by a stub retired nine days
    /// later, and the windowless fold reported that channel dead — the false
    /// alarm the C2 F2 amendment was supposed to end. The history stays
    /// visible; it just stops convicting.
    #[test]
    fn an_outage_that_is_over_does_not_read_as_dead() {
        let long_ago = NOW - DEAD_CHANNEL_WINDOW_MILLIS - 1;
        let log: String = std::iter::repeat(format!("{long_ago}\tobserver\tcannot-inject\t\n"))
            .take(175)
            .collect();
        let channels = fold_silence_log(&log, NOW);
        assert!(
            !channels.iter().any(|c| c.dead),
            "a nine-day-old outage is history, not a currently-dead channel"
        );
        let observer = channels.iter().find(|c| c.role == "observer").unwrap();
        assert_eq!(175, observer.fired, "and the history is still shown");
        assert_eq!(Some(&175), observer.suppressed.get("cannot-inject"));
    }

    /// The window does not blunt the instrument: the same rows, written now.
    #[test]
    fn the_same_outage_happening_now_does_read_as_dead() {
        let log: String = std::iter::repeat(format!("{}\tobserver\tcannot-inject\t\n", NOW - 60_000))
            .take(175)
            .collect();
        let dead: Vec<String> = fold_silence_log(&log, NOW)
            .into_iter()
            .filter(|c| c.dead)
            .map(|c| c.role)
            .collect();
        assert_eq!(vec!["observer".to_string()], dead);
    }

    /// The boundary on both sides, and a row the fold cannot date — which
    /// counts as history, because the verdict never convicts on evidence it
    /// could not place in time.
    #[test]
    fn the_window_edge_and_an_undateable_row() {
        let inside = format!("{}\tuser-prompt\tcannot-inject\t\n", NOW - DEAD_CHANNEL_WINDOW_MILLIS);
        assert!(fold_silence_log(&inside, NOW).iter().any(|c| c.dead), "inside the window");

        let outside =
            format!("{}\tuser-prompt\tcannot-inject\t\n", NOW - DEAD_CHANNEL_WINDOW_MILLIS - 1);
        assert!(!fold_silence_log(&outside, NOW).iter().any(|c| c.dead), "one ms outside");

        let undateable = "not-a-timestamp\tuser-prompt\tcannot-inject\t\n";
        let channels = fold_silence_log(undateable, NOW);
        assert!(!channels.iter().any(|c| c.dead), "an undateable row cannot convict");
        assert_eq!(1, channels[0].fired, "but it is still counted");
    }

    #[test]
    fn a_channel_that_answered_and_never_emitted_reads_as_dead() {
        let channels = fold_silence_log(&seeded_silence_log(), NOW);
        let dead: Vec<&str> = channels.iter().filter(|c| c.dead).map(|c| c.role.as_str()).collect();
        assert_eq!(vec!["user-prompt"], dead);
        let it = channels.iter().find(|c| c.role == "user-prompt").unwrap();
        assert_eq!(4, it.fired);
        assert_eq!(0, it.emitted);
        assert_eq!(Some(&3), it.suppressed.get("cannot-inject"));
    }

    #[test]
    fn quiet_by_design_is_never_dead() {
        let channels = fold_silence_log(&seeded_silence_log(), NOW);
        let quiet: Vec<&str> = channels
            .iter()
            .filter(|c| c.legitimately_quiet)
            .map(|c| c.role.as_str())
            .collect();
        assert_eq!(vec!["guard", "observer"], quiet);
        assert!(!channels.iter().find(|c| c.role == "tool-recall").unwrap().dead);
    }

    #[test]
    fn two_installs_merge_into_one_per_machine_view() {
        let dir = scratch("installs");
        let a = dir.join("hook_silence.log");
        let b = dir.join("cursor_silence.log");
        std::fs::write(&a, format!("{NOW}\tprimer\temitted\t")).unwrap(); // no trailing newline
        std::fs::write(&b, format!("{NOW}\tprimer\temitted\t\n")).unwrap();
        let channels = fold_silence_logs(&[a, b, dir.join("absent.log")], NOW);
        assert_eq!(1, channels.len());
        assert_eq!(2, channels[0].emitted, "a missing newline must not eat a record");
    }

    // ---- utilization ----

    #[test]
    fn the_utilization_number_always_travels_with_its_caveat() {
        let signals = fold_outcomes(
            "2026-08-18T10:00:00Z\t3.10.0\tslip\tBash\tgrepping\n\
             2026-08-18T10:00:01Z\t3.10.0\tslip\tEdit\treason\n\
             2026-08-18T10:00:02Z\t3.10.0\tread-ungrounded\tRead\n\
             2026-08-18T10:00:03Z\t3.10.0\tverify\tcompile_workspace\n",
        );
        assert_eq!(2, signals.slips);
        assert_eq!(1, signals.ungrounded_reads);

        let util = utilization(97, &signals);
        assert_eq!(3, util.shell_fallbacks);
        assert_eq!(Some(97.0), util.percent);
        assert!(util.caveat.contains("Claude Code and Cursor"), "R1 is SHOWN: {}", util.caveat);
        assert!(util.caveat.contains("28f"));
        assert!(util.observer_present);
    }

    /// An empty denominator is not 100 %. Reporting one would be a number
    /// invented out of no observation at all.
    #[test]
    fn nothing_observed_yields_no_percentage_rather_than_a_perfect_one() {
        let util = utilization(0, &OutcomeSignals::default());
        assert_eq!(None, util.percent);
        assert!(!util.observer_present, "and the view learns the observer never wrote");
    }

    // ---- the canary ----

    fn ok(value: serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(value)
    }

    #[test]
    fn a_resident_that_answers_both_questions_is_green() {
        let result = judge_canary(
            "ws",
            "http://127.0.0.1:1/mcp",
            ok(serde_json::json!({"success": true, "data": {"entries": []}})),
            ok(serde_json::json!({"success": true, "data": {"sourceLength": 12345}})),
            7,
        );
        assert!(result.green);
        assert!(result.recall_ok && result.compiler_ok);
        assert!(result.compiler_detail.contains(CANARY_FIXTURE_TYPE));
        assert_eq!(CanaryHealth::Green, canary_health(&[result], 7));
    }

    /// An AUTHORITATIVE ABSENCE passes: the store spoke. Only a store that
    /// cannot answer at all is degradation — otherwise the canary would depend
    /// on what the user happens to have stored.
    #[test]
    fn an_empty_recall_answer_is_still_an_answer() {
        let result = judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({"success": true, "data": {"entries": [], "absence": true}})),
            ok(serde_json::json!({"data": {"typeName": "java.lang.String"}})),
            0,
        );
        assert!(result.green, "{}", result.recall_detail);
    }

    /// THE PAIR for the test above, and the reason the canary exists
    /// (jawata-mcp#37). An empty answer and an unreadable store are different
    /// facts; the engine now says which, and the canary must not flatten them
    /// back together. Before this, `KNOWLEDGE_UNAVAILABLE` was an object, so
    /// `recall_ok` was true and the tray went GREEN during the outage the
    /// canary was built to catch.
    #[test]
    fn a_store_that_could_not_answer_is_not_a_store_that_answered() {
        let result = judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "KNOWLEDGE_UNAVAILABLE",
                    "message": "the store did not answer within 1200ms; it is unreachable or wedged"
                }
            })),
            ok(serde_json::json!({"data": {"typeName": "java.lang.String"}})),
            0,
        );
        assert!(!result.recall_ok, "an unreadable store is not a store that spoke");
        assert!(!result.green, "and the tray must not be green during the outage");
        assert!(
            result.recall_detail.contains("wedged"),
            "the reason the resident gave must survive to the dashboard: {}",
            result.recall_detail
        );
    }

    /// A refusal WE caused says nothing about the store's health, so it must
    /// not be read as an outage. Keyed on the code, not on `success:false`.
    #[test]
    fn an_ordinary_refusal_is_not_read_as_a_store_outage() {
        let result = judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({
                "success": false,
                "error": {"code": "INVALID_PARAMETER", "message": "kind is required"}
            })),
            ok(serde_json::json!({"data": {"typeName": "java.lang.String"}})),
            0,
        );
        assert!(result.recall_ok, "the store answered — it just refused our question");
    }

    #[test]
    fn a_resident_whose_compiler_layer_is_down_is_not_green() {
        let result = judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({"success": true})),
            ok(serde_json::json!({"success": false, "error": "no project loaded"})),
            0,
        );
        assert!(result.recall_ok, "the store still answers");
        assert!(!result.compiler_ok, "the compiler question did not resolve the fixture");
        assert!(!result.green);
        assert!(!result.loading, "a shapeless error is not a loading answer");
        assert_eq!(CanaryHealth::Degraded, canary_health(&[result], 0));
    }

    #[test]
    fn nothing_probed_yet_is_unknown_and_never_green() {
        assert_eq!(CanaryHealth::Unknown, canary_health(&[], 0));
    }

    /// The `#16` defect: a resident that is still IMPORTING answers the compiler
    /// question correctly — with PROJECT_LOADING — and used to be counted a
    /// compiler failure, painting the tray amber for five minutes on every
    /// healthy launch.
    #[test]
    fn a_still_loading_resident_reads_as_loading_not_degraded() {
        let result = judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({"success": true, "data": {"entries": []}})),
            ok(serde_json::json!({
                "success": false,
                "error": {"code": LOADING_ERROR_CODE, "message": "project is loading"}
            })),
            0,
        );
        assert!(result.recall_ok, "the store still answers");
        assert!(!result.green, "loading is not green — nothing was resolved");
        assert!(result.loading);
        assert!(result.compiler_detail.contains("still loading"));

        // Stitch first, exactly as `publish_canary` does. An UNSTITCHED loading result
        // deliberately does not get the grace — otherwise deleting the one wiring line
        // would make Loading unbounded with every test still green.
        let mut round = vec![result];
        stitch_loading_runs(&[], &mut round, 0);
        assert_eq!(CanaryHealth::Loading, canary_health(&round, 0));

        round[0].loading_since_millis = None;
        assert_eq!(
            CanaryHealth::Degraded,
            canary_health(&round, 0),
            "an unstitched loading result must NOT be excused — that is the missing-wiring case"
        );
    }

    /// The store half of the canary must not be excused by the compiler half. A resident
    /// whose knowledge store is dead while its compiler says "still importing" is broken,
    /// not starting up.
    #[test]
    fn a_dead_store_is_not_excused_by_a_loading_compiler() {
        let result = judge_canary(
            "ws",
            "u",
            Err("request failed: connection refused".into()),
            ok(serde_json::json!({"success": false, "error": {"code": LOADING_ERROR_CODE}})),
            0,
        );
        assert!(result.loading, "the compiler half did say it is loading");
        assert!(!result.recall_ok, "but the store did not answer at all");

        let mut round = vec![result];
        stitch_loading_runs(&[], &mut round, 0);
        assert_eq!(
            CanaryHealth::Degraded,
            canary_health(&round, 0),
            "a dead store must raise the alarm even while the compiler is still importing"
        );
    }

    /// The composition is optional at the type level: `canary_health` accepts unstitched
    /// results, and the only thing that stitches them in production is one line in
    /// `publish_canary`. Delete it and LOADING silently stops being granted (or, before the
    /// fail-closed change, silently stopped being bounded) with every unit test still green.
    /// So assert the wiring itself — the same idiom this file already uses to prove the view
    /// renders what the fold produces.
    #[test]
    fn the_board_stitches_loading_runs_before_it_judges() {
        let service = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("manager_service.rs");
        let source = std::fs::read_to_string(&service)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", service.display()));
        let publish = source
            .split_once("fn publish_canary")
            .expect("publish_canary must exist — it is the only production caller of the judge")
            .1;
        let body = &publish[..publish.len().min(1200)];
        assert!(
            body.contains("stitch_loading_runs"),
            "publish_canary judges without stitching: the loading run loses its start, so the \
             grace period is measured from the wrong instant and LOADING is no longer bounded"
        );
    }

    /// The chain is Rust variant -> serialized name -> TS union -> Svelte comparison.
    /// Asserting only the displayed word leaves the wire name free to drift, which would
    /// silently drop the dashboard back to its fallback text.
    #[test]
    fn the_loading_verdict_serializes_under_the_name_the_view_compares() {
        assert_eq!(
            serde_json::json!("loading"),
            serde_json::to_value(CanaryHealth::Loading).unwrap()
        );
    }

    /// The allow-list has exactly one member. These two arrive in the SAME
    /// `Ok(value)` envelope, and treating any error body as loading would paint
    /// a genuinely broken workspace healthy forever.
    #[test]
    fn a_failed_or_absent_load_stays_degraded_it_is_not_loading() {
        for code in ["PROJECT_LOAD_FAILED", "PROJECT_NOT_LOADED"] {
            let result = judge_canary(
                "ws",
                "u",
                ok(serde_json::json!({"success": true})),
                ok(serde_json::json!({"success": false, "error": {"code": code}})),
                0,
            );
            assert!(!result.loading, "{code} is a failure, not a loading answer");
            assert_eq!(
                CanaryHealth::Degraded,
                canary_health(&[result], 0),
                "{code} must keep its amber"
            );
        }
    }

    /// Loading is innocent while it is young. A load that never finishes must
    /// not masquerade as health forever.
    #[test]
    fn a_load_that_never_finishes_loses_the_benefit_of_the_doubt() {
        let mut result = judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({"success": true})),
            ok(serde_json::json!({"success": false, "error": {"code": LOADING_ERROR_CODE}})),
            0,
        );
        result.loading_since_millis = Some(0);

        assert_eq!(
            CanaryHealth::Loading,
            canary_health(std::slice::from_ref(&result), LOADING_GRACE_MILLIS - 1)
        );
        assert_eq!(
            CanaryHealth::Degraded,
            canary_health(std::slice::from_ref(&result), LOADING_GRACE_MILLIS),
            "past the grace period an unfinished load is a fault"
        );
    }

    /// The grace period is measured from when loading STARTED, so the run has to
    /// survive the rounds in between.
    #[test]
    fn a_loading_run_keeps_its_first_timestamp_across_rounds() {
        let loading = |workspace: &str| {
            judge_canary(
                workspace,
                "u",
                ok(serde_json::json!({"success": true})),
                ok(serde_json::json!({"success": false, "error": {"code": LOADING_ERROR_CODE}})),
                0,
            )
        };

        let mut first = vec![loading("ws")];
        stitch_loading_runs(&[], &mut first, 1_000);
        assert_eq!(Some(1_000), first[0].loading_since_millis);

        let mut second = vec![loading("ws")];
        stitch_loading_runs(&first, &mut second, 400_000);
        assert_eq!(
            Some(1_000),
            second[0].loading_since_millis,
            "the run keeps its origin, it does not restart every round"
        );

        // Once it stops loading the stamp is dropped, so a LATER run starts fresh.
        let mut recovered = vec![judge_canary(
            "ws",
            "u",
            ok(serde_json::json!({"success": true})),
            ok(serde_json::json!({"success": true, "data": {"sourceLength": 1}})),
            0,
        )];
        stitch_loading_runs(&second, &mut recovered, 500_000);
        assert_eq!(None, recovered[0].loading_since_millis);
    }

    // ---- the whole status over seeded files ----

    #[test]
    fn seeded_files_fold_into_the_status_the_view_renders() {
        let root = scratch("status");
        let ws_a = root.join("alpha").join("field");
        let ws_b = root.join("beta").join("field");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        std::fs::write(ws_a.join("pile.jsonl"), seeded_pile()).unwrap();
        std::fs::write(
            ws_a.join("state.json"),
            "{\"nudges\":true,\"silenced\":false,\"remindedAt\":0,\"strikes\":0,\"posted\":[]}",
        )
        .unwrap();
        // beta has a resident but nothing recorded yet.
        let silence = root.join("hook_silence.log");
        std::fs::write(&silence, seeded_silence_log()).unwrap();
        let outcomes = root.join("outcomes.log");
        std::fs::write(&outcomes, "t\tv\tslip\tBash\n").unwrap();

        let status = status_from(
            &[
                ("alpha".to_string(), ws_a.clone()),
                ("beta".to_string(), ws_b.clone()),
            ],
            &[silence.clone(), root.join("never-deployed.log")],
            &outcomes,
            vec![judge_canary("alpha", "u", Err("connection refused".into()), Err("connection refused".into()), 1)],
            NOW,
        );

        assert_eq!(2, status.workspaces.len());
        assert_eq!(2, status.badge, "the machine badge sums the workspaces");
        assert!(status.workspaces[0].pile.present);
        assert!(!status.workspaces[1].pile.present, "beta has recorded nothing");
        assert_eq!(vec!["user-prompt".to_string()], status.dead_channels);
        assert_eq!(
            vec!["guard".to_string(), "observer".to_string()],
            status.legitimately_quiet_channels
        );
        assert_eq!(1, status.silence_logs_read.len(), "only the log that EXISTS is claimed");
        assert_eq!(10, status.utilization.jawata_calls);
        assert_eq!(1, status.utilization.shell_fallbacks);
        assert!(status.utilization.caveat.contains("Hook-scoped"));
        assert_eq!(CanaryHealth::Degraded, status.canary_health);
        assert_eq!("/report", status.workspaces[0].lane.seat);
    }
}

/// The F2 assertions: the field view's render path can never interrupt anyone.
///
/// Stage 4 asserted the ABSENCE of a notification path in the hook crate. This
/// is that scan extended to the studio's view path, which is where the
/// temptation actually lives — a view has a window to put a box in front of.
#[cfg(test)]
mod interruption_scans {
    use std::path::{Path, PathBuf};

    fn manifest() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// The field-view/lane RENDER PATH, named file by file. A scan over a
    /// pattern can silently match nothing; these paths are asserted to exist
    /// and to be non-trivial before they are read, so this test cannot pass by
    /// looking at an empty set.
    fn render_path() -> Vec<PathBuf> {
        let root = manifest();
        let web = root.join("..").join("src").join("lib").join("components");
        vec![
            root.join("src").join("field_view.rs"),
            root.join("src").join("commands.rs"),
            // FieldSeatTile.svelte left this list when the seat lane left the
            // page (Harald's ruling, 2026-08-18): its go-silent control moved
            // into FieldView's header; the seat roster returns as its own menu
            // item in Sprint 28f and rejoins this render path then.
            web.join("FieldView.svelte"),
        ]
    }

    /// The needles are ASSEMBLED, never written whole — a scan whose own source
    /// contains its needles flags itself, and the obvious cure (skipping this
    /// file) would blind it to the module it guards most. Copied in shape from
    /// the hook's `the_user_is_never_interrupted_by_a_popping_surface`.
    fn banned_surfaces() -> Vec<String> {
        [
            ("noti", "fy_rust"),
            ("noti", "fy-rust"),
            ("notifi", "cation"),
            ("Message", "Box"),
            ("toa", "st"),
            ("dia", "log"),
            ("popu", "p"),
            ("mod", "al"),
            ("ale", "rt("),
            ("con", "firm("),
            ("win", "dow.open"),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}").to_lowercase())
        .collect()
    }

    /// Strip the comment tails so PROSE about the rule is allowed while the
    /// rule itself is enforced on code.
    fn code_of(line: &str) -> String {
        let no_slash = line.split("//").next().unwrap_or("");
        let no_html = no_slash.split("<!--").next().unwrap_or("");
        no_html.to_lowercase()
    }

    fn scan(paths: &[PathBuf], needles: &[String]) -> Vec<String> {
        let mut offenders = Vec::new();
        for path in paths {
            let Ok(body) = std::fs::read_to_string(path) else {
                continue;
            };
            for (n, line) in body.lines().enumerate() {
                let code = code_of(line);
                for needle in needles {
                    if code.contains(needle.as_str()) {
                        offenders.push(format!("{}:{} {needle}", path.display(), n + 1));
                    }
                }
            }
        }
        offenders
    }

    #[test]
    fn the_field_render_path_mounts_no_interrupting_surface() {
        let files = render_path();
        for path in &files {
            let body = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("the render path must exist: {}: {e}", path.display()));
            assert!(
                body.len() > 400,
                "{} is too small to be the real render path — a scan over an empty \
                 file passes without looking at anything",
                path.display()
            );
        }
        let offenders = scan(&files, &banned_surfaces());
        assert!(
            offenders.is_empty(),
            "the field view is PASSIVE — it renders state and changes colour; it never \
             puts a surface in front of the user, and it mounts no component whose very \
             name is one: {offenders:?}"
        );
    }

    /// The narrower rule, everywhere in the studio backend: no OS-notification
    /// API is reachable at all. Scoped to the OS needles rather than the full
    /// list on purpose — the studio legitimately opens FILE PICKERS from the
    /// Memory view and speaks the MCP protocol's `notifications/initialized`,
    /// and a scan that called those violations would be a scan nobody could
    /// keep green.
    #[test]
    fn no_studio_module_reaches_for_an_os_level_interrupt() {
        let os_only: Vec<String> = [
            ("noti", "fy_rust"),
            ("noti", "fy-rust"),
            ("tauri_plugin_noti", "fication"),
            ("Message", "BoxW"),
            ("NSUserNoti", "fication"),
            ("UNUserNoti", "ficationCenter"),
            ("Shell_Noti", "fyIcon"),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}").to_lowercase())
        .collect();

        let mut files = Vec::new();
        for entry in std::fs::read_dir(manifest().join("src")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
        assert!(files.len() > 5, "the scan must see the whole backend, saw {files:?}");
        let offenders = scan(&files, &os_only);
        assert!(
            offenders.is_empty(),
            "the reminders speak through the agent and the canary through a colour: {offenders:?}"
        );
    }

    /// The seeded-render half: every datum the fold produces for THIS page has
    /// somewhere to land. A view that renders half the fold is how a
    /// shipped-but-unwired headline happens — this sprint's own predecessor did
    /// exactly that.
    ///
    /// The seat lane left the page (Harald's ruling, 2026-08-18): the per-seat
    /// data (posted history, reminder state, nudged shapes, the no-nudges
    /// switch) is deliberately UNRENDERED until Sprint 28f's Seats page, and
    /// this test's enumeration shrank with the page rather than asserting a
    /// binding for a surface that no longer exists. What survived — the
    /// go-silent switch — is asserted below, on the view itself.
    #[test]
    fn the_view_binds_every_datum_the_fold_produces() {
        let web = manifest().join("..").join("src").join("lib").join("components");
        let view = std::fs::read_to_string(web.join("FieldView.svelte")).unwrap();

        for datum in [
            "badge",
            "shapes",
            "deadChannels",
            "legitimatelyQuietChannels",
            "utilization",
            "percent",
            "caveat",
            "shellFallbacks",
            "jawataCalls",
            "canaryHealth",
            // the go-silent control, survived from the seat lane
            "silenced",
        ] {
            assert!(
                view.contains(datum),
                "the view drops `{datum}` on the floor — the fold computes it and \
                 nothing renders it"
            );
        }

        // A datum NAME is not enough for an enum: `canaryHealth` was already in
        // the list above, so adding a variant to it in Rust leaves this test
        // green while the dashboard renders the fallback. The variant and the
        // word it displays are two separate bindings, so assert the word.
        assert!(
            view.contains("starting up"),
            "CanaryHealth::Loading reaches the view as a variant with no word — \
             the dashboard would fall through to \"not checked yet\" on every \
             healthy cold start (#16)"
        );

        // The go-silent switch has a VISIBLE control in the header, and it
        // writes through the atomic setter — the same on-disk state the hook
        // reads, so the hook-side contract test stays honest.
        assert!(
            view.matches("type=\"checkbox\"").count() >= 1,
            "the header carries the go-silent checkbox"
        );
        assert!(
            view.contains("fieldSetSilence"),
            "and the control writes through the atomic setter"
        );
        assert!(
            view.contains("/report"),
            "the header names the /report seat — the page's whole outbound path"
        );
    }
}
