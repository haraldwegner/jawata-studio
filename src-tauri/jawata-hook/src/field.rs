//! Reach counters + the dead-channel check (Sprint 28b, D5).
//!
//! The hook already writes one record per fire into the silence log —
//! `<millis>\t<role>\t<tag>\t<detail>` (see [`crate::silence`]). These counters
//! are a FOLD over that stream, never a second write path: the architecture
//! artifact's rule is one observation seam, and the silence log's append
//! discipline took six audit rounds to get right once.
//!
//! The deterministic dead-channel condition is the spec's own sentence: *the
//! store answered N > 0 and the channel emitted 0.* In tag terms: a role whose
//! fires include answered-but-suppressed outcomes while none emitted. Absence
//! (`store-had-nothing`, `no-cues`) is NOT dead — there was genuinely nothing
//! to say; that distinction is the whole reason a naive threshold would be
//! noise (the spec's "absence is often legitimate").

use std::collections::BTreeMap;
use std::path::Path;

/// The studio↔store contract version this binary speaks (Sprint 28b, D7 —
/// the decision in ARCHITECTURE-field-recordings-28b.md). Sent with every
/// store request; a PRESENT-but-different echo is a typed, counted refusal
/// to inject under an unverified seam. An ABSENT echo is an older store and
/// proceeds — the seam predates the contract there.
pub const HOOK_CONTRACT: u32 = 1;

/// Tags that mean "the store answered, and the channel still delivered
/// nothing" — the dead-channel numerator. Extended, not edited, when a new
/// answered-class suppression appears.
const ANSWERED_BUT_SUPPRESSED: &[&str] =
    &["cannot-inject", "contract-mismatch", "answer-unusable"];

/// Tags that mean "quiet was the correct outcome".
pub const LEGITIMATELY_QUIET: &[&str] = &[
    "store-had-nothing", "no-cues", "stop-allowed", "role-absent-on-client",
    "recorded-not-injected", "nothing-to-observe",
];

/// How far back the DEAD verdict is allowed to look: seven days.
///
/// The condition without a window has no present tense. This machine's log
/// carries 175 `cannot-inject` observer rows, every one written on 2026-08-09
/// by a stub that was retired nine days later — and the fold called that
/// channel dead, forever, because the log is append-only and the rows never
/// age out. That is the same class of built-in false alarm the C2 F2
/// amendment removed for by-design quiet, arriving through the other door.
///
/// Seven days, and not an arbitrary seven: the verdict feeds the same surface
/// D9 speaks on at most weekly ([`REMINDER_INTERVAL_MILLIS`]), so the evidence
/// window and the cadence at which the user hears about it are one period. A
/// shorter window would call a channel unknown after a weekend away; a longer
/// one keeps convicting on behaviour the user has already replaced.
///
/// Older rows are HISTORY, not evidence: they stay in `fired` / `emitted` /
/// `suppressed`, which is what the counters are for. They simply no longer
/// convict.
pub const DEAD_CHANNEL_WINDOW_MILLIS: u64 = 7 * 24 * 60 * 60 * 1000;

/// One channel's folded counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelCounters {
    pub fired: u64,
    pub emitted: u64,
    /// Suppressions by their bounded tag — the reason enum, never free text.
    pub suppressed: BTreeMap<String, u64>,
    /// The same two facts, restricted to [`DEAD_CHANNEL_WINDOW_MILLIS`]. The
    /// verdict's working set, kept separate from the history above.
    recent_emitted: u64,
    recent_answered_but_suppressed: u64,
}

impl ChannelCounters {
    fn answered_but_suppressed(&self) -> u64 {
        ANSWERED_BUT_SUPPRESSED
            .iter()
            .filter_map(|t| self.suppressed.get(*t))
            .sum()
    }

    /// The deterministic condition, on RECENT evidence only: the store
    /// answered and the channel delivered nothing, within the window.
    pub fn dead(&self) -> bool {
        self.recent_emitted == 0 && self.recent_answered_but_suppressed > 0
    }

    /// Quiet, and rightly so: nothing emitted, and every suppression was a
    /// legitimate absence (or there were none).
    pub fn legitimately_quiet(&self) -> bool {
        self.emitted == 0
            && self.answered_but_suppressed() == 0
            && self
                .suppressed
                .keys()
                .all(|t| LEGITIMATELY_QUIET.contains(&t.as_str()))
    }
}

/// Fold the silence log into per-role counters. Unparseable lines are
/// skipped — a half-written line loses itself, never the fold (the log's own
/// contract).
/// `now_millis` anchors the recency window: a row whose timestamp is missing,
/// unparseable, or older than [`DEAD_CHANNEL_WINDOW_MILLIS`] still counts in
/// the history, and never toward the verdict.
pub fn fold_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
    now_millis: u64,
) -> BTreeMap<String, ChannelCounters> {
    let mut by_role: BTreeMap<String, ChannelCounters> = BTreeMap::new();
    for line in lines {
        let mut parts = line.splitn(4, '\t');
        let (Some(millis), Some(role), Some(tag)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if role.is_empty() || tag.is_empty() {
            continue;
        }
        let recent = millis
            .trim()
            .parse::<u64>()
            .map(|at| now_millis.saturating_sub(at) <= DEAD_CHANNEL_WINDOW_MILLIS)
            .unwrap_or(false);
        let counters = by_role.entry(role.to_string()).or_default();
        counters.fired += 1;
        if tag == "emitted" {
            counters.emitted += 1;
            if recent {
                counters.recent_emitted += 1;
            }
        } else {
            *counters.suppressed.entry(tag.to_string()).or_insert(0) += 1;
            if recent && ANSWERED_BUT_SUPPRESSED.contains(&tag) {
                counters.recent_answered_but_suppressed += 1;
            }
        }
    }
    by_role
}

/// Fold a silence log file; a missing or unreadable file folds to empty —
/// callers surface THAT distinctly (an absent log is "the hook never ran",
/// which is its own finding, not a healthy zero).
pub fn fold_file(path: &Path, now_millis: u64) -> BTreeMap<String, ChannelCounters> {
    match std::fs::read_to_string(path) {
        Ok(content) => fold_lines(content.lines(), now_millis),
        Err(_) => BTreeMap::new(),
    }
}

/// The roles whose channels are dead by the deterministic condition.
pub fn dead_channels(folded: &BTreeMap<String, ChannelCounters>) -> Vec<String> {
    folded
        .iter()
        .filter(|(_, c)| c.dead())
        .map(|(role, _)| role.clone())
        .collect()
}

// ---- D4: the nudge (Sprint 28b) ----

/// How many times a shape must recur before the one-line pointer appears.
pub const NUDGE_THRESHOLD: u64 = 3;

/// The line itself. It informs — it asks nothing, it repeats for no shape, and
/// ignoring it forever is a supported answer (the spec's ladder).
pub fn nudge_line(shape: &str, count: u64) -> String {
    format!(
        "This failure shape has now happened {count} times here ({shape}). \
         Running /report drafts a bug from jawata's local recording — shapes only, \
         no paths or code — for you to review and post from your own GitHub account."
    )
}

/// Which shape (if any) deserves the nudge right now.
///
/// Four conditions, all read from files someone else owns — this function
/// writes nothing: the switch is on, the shape has recurred at least
/// [`NUDGE_THRESHOLD`] times, it has not been posted, and it has not already
/// been nudged. A missing state file means DEFAULTS (nudge on), never silence.
pub fn nudge_due(field_dir: &Path) -> Option<(String, u64)> {
    let state = std::fs::read_to_string(field_dir.join("state.json")).unwrap_or_default();
    if state.contains("\"nudges\":false") {
        return None;
    }
    let posted = quoted_items(&state, "\"posted\":[");
    let already = std::fs::read_to_string(field_dir.join("nudged.log"))
        .map(|c| c.lines().map(|l| l.trim().to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let pile = std::fs::read_to_string(field_dir.join("pile.jsonl")).ok()?;
    for line in pile.lines() {
        if !line.starts_with("{\"t\":") || line.contains("\"ok\":true") {
            continue;
        }
        let (Some(tool), Some(kind), Some(code)) = (
            between(line, "\"tool\":\"", '"'),
            between(line, "\"kind\":\"", '"'),
            between(line, "\"code\":\"", '"'),
        ) else {
            continue;
        };
        *counts.entry(format!("{tool}/{kind}/{code}")).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .filter(|(shape, count)| {
            *count >= NUDGE_THRESHOLD
                && !posted.contains(shape)
                && !already.contains(shape)
        })
        .max_by_key(|(_, count)| *count)
}

/// Records that a shape was nudged. APPEND-ONLY, its own file: the state file
/// has two other writers (the resident and studio), and a read-modify-write
/// from a hook would clobber whichever switch they set last.
pub fn record_nudged(field_dir: &Path, shape: &str) {
    use std::io::Write as _;
    let _ = std::fs::create_dir_all(field_dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(field_dir.join("nudged.log"))
    {
        let _ = f.write_all(format!("{shape}\n").as_bytes());
    }
}

// ---- D9: the periodic reminder (Sprint 28b) ----

/// At most one reminder a week, and only when there is something new to say.
pub const REMINDER_INTERVAL_MILLIS: u64 = 7 * 24 * 60 * 60 * 1000;

/// From this strike onward the reminder carries the go-silent question.
pub const STRIKES_BEFORE_ASKING: usize = 2;

/// What the agent says, and whether it carries the question.
///
/// It INFORMS: the user who never opens studio must still learn that jawata is
/// failing for him, or he decides it is not worth using while jawata knew all
/// along. It never lists the shapes — those live in the dashboard.
pub fn reminder_line(shapes: usize, failures: u64, carries_question: bool) -> String {
    let mut line = format!(
        "jawata has recorded {failures} failed tool calls here across {shapes} recurring \
         failure shapes. Running /report turns one into a bug report you review and post \
         from your own GitHub account — shapes only, no code or paths."
    );
    if carries_question {
        line.push_str(
            " You have not acted on this before; should jawata go silent about failures? \
             The checkbox on the /report tile in jawata-studio decides, or just say so here.",
        );
    }
    line
}

/// Whether a reminder is due, and whether it carries the question.
///
/// Four gates, in order: not silenced · something new since the last reminder ·
/// at least a week since the last one · something to report at all. Everything
/// is read from files the resident and studio own; the caller records the fact
/// through [`record_reminded`].
pub fn reminder_due(field_dir: &Path, now_millis: u64) -> Option<(String, bool)> {
    let state = std::fs::read_to_string(field_dir.join("state.json")).unwrap_or_default();
    if state.contains("\"silenced\":true") {
        return None;
    }
    let posted = quoted_items(&state, "\"posted\":[");

    let mut shapes: BTreeMap<String, u64> = BTreeMap::new();
    let mut failures = 0u64;
    let pile = std::fs::read_to_string(field_dir.join("pile.jsonl")).ok()?;
    for line in pile.lines() {
        if !line.starts_with("{\"t\":") || line.contains("\"ok\":true") {
            continue;
        }
        let (Some(tool), Some(kind), Some(code)) = (
            between(line, "\"tool\":\"", '"'),
            between(line, "\"kind\":\"", '"'),
            between(line, "\"code\":\"", '"'),
        ) else {
            continue;
        };
        let shape = format!("{tool}/{kind}/{code}");
        if posted.contains(&shape) {
            continue; // already reported: not news, and not a reason to nag
        }
        failures += 1;
        *shapes.entry(shape).or_insert(0) += 1;
    }
    if shapes.is_empty() {
        return None; // nothing to say: an absence is an answer
    }

    let (last_shown, strikes) = reminder_ledger(field_dir);
    if last_shown > 0 && now_millis.saturating_sub(last_shown) < REMINDER_INTERVAL_MILLIS {
        return None; // too soon — "now and then", never a nag
    }
    Some((
        reminder_line(shapes.len(), failures, strikes >= STRIKES_BEFORE_ASKING),
        strikes >= STRIKES_BEFORE_ASKING,
    ))
}

/// The append-only reminder ledger: `<millis>\tshown` from the hook,
/// `<millis>\treset` from the resident when `/report` is used. Append-only for
/// the same reason the pile is — three processes touch this lane, and a
/// read-modify-write from any of them would lose another's record.
pub fn reminder_ledger(field_dir: &Path) -> (u64, usize) {
    let Ok(content) = std::fs::read_to_string(field_dir.join("reminded.log")) else {
        return (0, 0);
    };
    let mut last_shown = 0u64;
    let mut strikes = 0usize;
    for line in content.lines() {
        let mut parts = line.splitn(2, '\t');
        let (Some(at), Some(kind)) = (parts.next(), parts.next()) else {
            continue;
        };
        match kind.trim() {
            "shown" => {
                last_shown = at.trim().parse().unwrap_or(last_shown);
                strikes += 1;
            }
            "reset" => strikes = 0,
            _ => {}
        }
    }
    (last_shown, strikes)
}

/// Records that a reminder was shown.
pub fn record_reminded(field_dir: &Path, now_millis: u64) {
    use std::io::Write as _;
    let _ = std::fs::create_dir_all(field_dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(field_dir.join("reminded.log"))
    {
        let _ = f.write_all(format!("{now_millis}\tshown\n").as_bytes());
    }
}

fn between(line: &str, key: &str, end: char) -> Option<String> {
    let from = line.find(key)? + key.len();
    let to = line[from..].find(end)? + from;
    Some(line[from..to].to_string())
}

fn quoted_items(doc: &str, key: &str) -> Vec<String> {
    let Some(from) = doc.find(key) else {
        return Vec::new();
    };
    let rest = &doc[from + key.len()..];
    let to = rest.find(']').unwrap_or(0);
    rest[..to]
        .split(',')
        .map(|s| s.replace('"', "").trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Stage-0 fixture (dead-channel.json) in tag terms: the recall
    /// channel answered every time and emitted nothing (the two-week outage's
    /// exact signature), the tool-call recall emitted, the primer emitted,
    /// and the guard was legitimately quiet.
    /// A plausible "now" for the fold, and rows written moments before it —
    /// the timestamps must be RELATIVE, because the verdict now has a
    /// recency window and a fixed 2023 constant would make every one of these
    /// tests pass for the wrong reason (nothing recent, therefore not dead).
    const NOW: u64 = 1_700_000_000_000;

    fn fixture_log() -> String {
        let at = NOW - 1000;
        let mut log = String::new();
        for _ in 0..3 {
            log.push_str(&format!("{at}\tuser-prompt\tcannot-inject\t\n"));
        }
        log.push_str(&format!("{at}\ttool-recall\temitted\t\n"));
        log.push_str(&format!("{at}\ttool-recall\tstore-had-nothing\t\n"));
        log.push_str(&format!("{at}\tprimer\temitted\t\n"));
        log.push_str(&format!("{at}\tguard\tstore-had-nothing\t\n"));
        log
    }

    #[test]
    fn the_dead_channel_condition_fires_on_answered_but_never_emitted() {
        let folded = fold_lines(fixture_log().lines(), NOW);
        assert_eq!(dead_channels(&folded), vec!["user-prompt".to_string()]);
        let dead = &folded["user-prompt"];
        assert_eq!(dead.fired, 3);
        assert_eq!(dead.emitted, 0);
        assert_eq!(dead.suppressed.get("cannot-inject"), Some(&3));
    }

    #[test]
    fn legitimate_absence_is_not_dead() {
        let folded = fold_lines(fixture_log().lines(), NOW);
        assert!(!folded["guard"].dead(), "absence is often legitimate");
        assert!(folded["guard"].legitimately_quiet());
        assert!(!folded["tool-recall"].dead(), "it emitted");
    }

    // ---- the recency window (28b closing audit, F6) ----

    /// A RETIRED outage does not read as a currently-dead channel.
    ///
    /// This machine's `hook_silence.log` carries 175 `cannot-inject` observer
    /// rows, all written on 2026-08-09 by a stub retired nine days later, and
    /// the windowless fold called that channel dead forever — the same
    /// built-in false alarm the C2 F2 amendment removed for by-design quiet.
    /// The history is still counted; it just no longer convicts.
    #[test]
    fn an_outage_that_is_over_does_not_read_as_dead() {
        let long_ago = NOW - DEAD_CHANNEL_WINDOW_MILLIS - 1;
        let log: String = std::iter::repeat(format!("{long_ago}\tobserver\tcannot-inject\t\n"))
            .take(175)
            .collect();
        let folded = fold_lines(log.lines(), NOW);
        assert!(
            dead_channels(&folded).is_empty(),
            "a nine-day-old outage is history, not a dead channel today"
        );
        assert_eq!(175, folded["observer"].fired, "and the history is still there");
        assert_eq!(Some(&175), folded["observer"].suppressed.get("cannot-inject"));
    }

    /// The window does not blunt the instrument: the SAME rows, written now,
    /// are exactly the condition the fold exists to catch.
    #[test]
    fn the_same_outage_happening_now_does_read_as_dead() {
        let log: String = std::iter::repeat(format!("{}\tobserver\tcannot-inject\t\n", NOW - 60_000))
            .take(175)
            .collect();
        let folded = fold_lines(log.lines(), NOW);
        assert_eq!(dead_channels(&folded), vec!["observer".to_string()]);
    }

    /// The boundary, both sides, and a row the fold cannot date: an
    /// unparseable timestamp counts as history — the verdict never convicts
    /// on evidence it could not place in time.
    #[test]
    fn the_window_edge_and_an_undateable_row() {
        let edge = format!("{}\tuser-prompt\tcannot-inject\t\n", NOW - DEAD_CHANNEL_WINDOW_MILLIS);
        assert!(!dead_channels(&fold_lines(edge.lines(), NOW)).is_empty(), "inside the window");

        let past = format!("{}\tuser-prompt\tcannot-inject\t\n", NOW - DEAD_CHANNEL_WINDOW_MILLIS - 1);
        assert!(dead_channels(&fold_lines(past.lines(), NOW)).is_empty(), "one millisecond outside");

        let undateable = "not-a-timestamp\tuser-prompt\tcannot-inject\t\n";
        let folded = fold_lines(undateable.lines(), NOW);
        assert!(dead_channels(&folded).is_empty(), "an undateable row cannot convict");
        assert_eq!(1, folded["user-prompt"].fired, "but it is still counted");
    }

    /// A channel that emitted recently is alive even if it was suppressed a
    /// month ago — and one that emitted only long ago is not kept alive by it.
    #[test]
    fn only_recent_emissions_answer_recent_suppressions() {
        let stale_emit = format!(
            "{}\tuser-prompt\temitted\t\n{}\tuser-prompt\tcannot-inject\t\n",
            NOW - DEAD_CHANNEL_WINDOW_MILLIS - 1,
            NOW - 1000
        );
        assert_eq!(
            vec!["user-prompt".to_string()],
            dead_channels(&fold_lines(stale_emit.lines(), NOW)),
            "an emission from before the window does not vouch for the channel today"
        );

        let fresh_emit = format!(
            "{}\tuser-prompt\temitted\t\n{}\tuser-prompt\tcannot-inject\t\n",
            NOW - 1000,
            NOW - 2000
        );
        assert!(
            dead_channels(&fold_lines(fresh_emit.lines(), NOW)).is_empty(),
            "it delivered inside the window — it is not dead"
        );
    }

    /// C2 audit, NO CONTROL #2: the by-design quiet tags must never make a
    /// channel dead — that misclassification is what marked every Cursor
    /// machine's observer and prompt channels permanently broken.
    #[test]
    fn by_design_quiet_never_reads_as_dead() {
        // Written just now, so the window cannot be what saves them: they are
        // not dead because the TAGS are by-design quiet.
        let log = format!(
            "{NOW}\tobserver\tnothing-to-observe\t\n\
             {NOW}\tobserver\trecorded-not-injected\t\n\
             {NOW}\tuser-prompt\trecorded-not-injected\t\n"
        );
        let folded = fold_lines(log.lines(), NOW);
        assert!(dead_channels(&folded).is_empty(), "quiet by design is not dead");
        assert!(folded["observer"].legitimately_quiet());
        assert!(folded["user-prompt"].legitimately_quiet());
    }

    /// C2 audit F1: an ANSWERED-but-unusable reply IS the dead-channel
    /// numerator — it is the two-week outage's own mechanism.
    #[test]
    fn an_unusable_answer_counts_toward_dead() {
        let log = format!("{NOW}\tuser-prompt\tanswer-unusable\tShapeChanged\n");
        let folded = fold_lines(log.lines(), NOW);
        assert_eq!(dead_channels(&folded), vec!["user-prompt".to_string()]);
    }

    #[test]
    fn a_contract_mismatch_counts_toward_dead() {
        let log = format!("{NOW}\tuser-prompt\tcontract-mismatch\tours=1 theirs=2\n");
        let folded = fold_lines(log.lines(), NOW);
        assert_eq!(dead_channels(&folded), vec!["user-prompt".to_string()]);
    }

    #[test]
    fn garbage_lines_lose_themselves_never_the_fold() {
        let log = format!("not a record\n{}\t\n\n", fixture_log());
        let folded = fold_lines(log.lines(), NOW);
        assert_eq!(folded["primer"].emitted, 1);
    }

    #[test]
    fn a_missing_log_folds_to_empty() {
        let folded = fold_file(Path::new("/nonexistent/hook_silence.log"), NOW);
        assert!(folded.is_empty());
    }

    // ---- D4: the nudge's four conditions ----

    fn field_scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!("jawata-nudge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A pile with `times` failures of one shape, plus one success and one
    /// other shape below the threshold.
    fn seed_pile(dir: &Path, times: usize) {
        let mut pile = String::from("{\"pileFormat\":1,\"contract\":1}\n");
        for _ in 0..times {
            pile.push_str("{\"t\":1,\"tool\":\"run_tests\",\"kind\":\"run\",\"ok\":false,\
                \"code\":\"RUNNER_TIMEOUT\",\"lat\":5,\"client\":\"claude_code\",\"ver\":\"3_11_0\"}\n");
        }
        pile.push_str("{\"t\":1,\"tool\":\"run_tests\",\"kind\":\"run\",\"ok\":true,\
            \"code\":\"unknown\",\"lat\":2,\"client\":\"claude_code\",\"ver\":\"3_11_0\"}\n");
        pile.push_str("{\"t\":1,\"tool\":\"inspect\",\"kind\":\"source\",\"ok\":false,\
            \"code\":\"TYPE_NOT_FOUND\",\"lat\":1,\"client\":\"claude_code\",\"ver\":\"3_11_0\"}\n");
        std::fs::write(dir.join("pile.jsonl"), pile).unwrap();
    }

    #[test]
    fn the_nudge_waits_for_the_threshold_then_names_the_shape() {
        let dir = field_scratch("threshold");
        seed_pile(&dir, 2);
        assert_eq!(None, nudge_due(&dir), "below the threshold, nothing is owed");
        seed_pile(&dir, 3);
        let (shape, count) = nudge_due(&dir).expect("the threshold is met");
        assert_eq!("run_tests/run/RUNNER_TIMEOUT", shape);
        assert_eq!(3, count, "successes never count toward a failure shape");
    }

    #[test]
    fn a_posted_shape_never_nudges_again() {
        let dir = field_scratch("posted");
        seed_pile(&dir, 4);
        std::fs::write(
            dir.join("state.json"),
            "{\"nudges\":true,\"silenced\":false,\"remindedAt\":0,\"strikes\":0,\
             \"posted\":[\"run_tests/run/RUNNER_TIMEOUT\"]}",
        )
        .unwrap();
        assert_eq!(None, nudge_due(&dir));
    }

    #[test]
    fn the_no_nudges_switch_suppresses_it() {
        // The plan's C1 amendment (F1): the switch is its OWN state — this
        // fixture sets `nudges`, never the reminders' `silenced` flag.
        let dir = field_scratch("switch");
        seed_pile(&dir, 5);
        std::fs::write(
            dir.join("state.json"),
            "{\"nudges\":false,\"silenced\":false,\"remindedAt\":0,\"strikes\":0,\"posted\":[]}",
        )
        .unwrap();
        assert_eq!(None, nudge_due(&dir), "the switch is off — nothing is owed");
        // And the OTHER switch does not suppress it: silencing the reminders
        // leaves the in-session line alone.
        std::fs::write(
            dir.join("state.json"),
            "{\"nudges\":true,\"silenced\":true,\"remindedAt\":0,\"strikes\":0,\"posted\":[]}",
        )
        .unwrap();
        assert!(nudge_due(&dir).is_some(), "the two switches are independent");
    }

    #[test]
    fn a_shape_nudges_once_and_only_once() {
        let dir = field_scratch("once");
        seed_pile(&dir, 9);
        let (shape, _) = nudge_due(&dir).expect("owed the first time");
        record_nudged(&dir, &shape);
        assert_eq!(None, nudge_due(&dir), "and never again for that shape");
    }

    #[test]
    fn a_missing_state_file_never_silences() {
        let dir = field_scratch("nostate");
        seed_pile(&dir, 3);
        assert!(nudge_due(&dir).is_some(), "defaults are ON — absence is not a switch");
    }

    #[test]
    fn no_pile_means_nothing_owed() {
        assert_eq!(None, nudge_due(&field_scratch("nopile")));
    }

    // ---- D9: the reminder's cadence, its question, and its two off-switches ----

    const WEEK: u64 = REMINDER_INTERVAL_MILLIS;

    #[test]
    fn the_reminder_is_due_first_time_then_waits_a_week() {
        let dir = field_scratch("cadence");
        seed_pile(&dir, 3);
        let (line, question) = reminder_due(&dir, 10 * WEEK).expect("first time is due");
        assert!(!question, "the first two are plain");
        assert!(line.contains("/report"), "{line}");
        assert!(!line.contains("RUNNER_TIMEOUT"), "the shapes stay in the dashboard: {line}");
        record_reminded(&dir, 10 * WEEK);

        assert_eq!(None, reminder_due(&dir, 10 * WEEK + WEEK / 2), "too soon");
        assert!(reminder_due(&dir, 11 * WEEK + 1).is_some(), "a week later, due again");
    }

    #[test]
    fn the_third_reminder_carries_the_question_and_keeps_carrying_it() {
        let dir = field_scratch("question");
        seed_pile(&dir, 4);
        record_reminded(&dir, WEEK);
        record_reminded(&dir, 2 * WEEK);
        let (line, question) = reminder_due(&dir, 3 * WEEK).expect("due");
        assert!(question, "the third asks");
        assert!(line.contains("go silent"), "{line}");
        // Ignoring the question changes nothing: it keeps being asked, and the
        // checkbox stays one click away.
        record_reminded(&dir, 3 * WEEK);
        let (_, still_asking) = reminder_due(&dir, 4 * WEEK).expect("due");
        assert!(still_asking, "an ignored question is not an answer either way");
    }

    #[test]
    fn using_report_resets_the_strikes_and_silences_that_shape() {
        let dir = field_scratch("reset");
        seed_pile(&dir, 4);
        record_reminded(&dir, WEEK);
        record_reminded(&dir, 2 * WEEK);
        // The resident appends this marker when /report is used.
        std::fs::write(dir.join("reminded.log"),
            format!("{}\tshown\n{}\tshown\n{}\treset\n", WEEK, 2 * WEEK, 2 * WEEK + 1)).unwrap();
        let (_, question) = reminder_due(&dir, 4 * WEEK).expect("due");
        assert!(!question, "a /report use resets the count — it is plain again");
        assert_eq!((2 * WEEK, 0), reminder_ledger(&dir));
    }

    #[test]
    fn both_silence_routes_stop_the_reminder() {
        let dir = field_scratch("silenced");
        seed_pile(&dir, 5);
        // Route 1: the checkbox / the agent writing the state.
        std::fs::write(dir.join("state.json"),
            "{\"nudges\":true,\"silenced\":true,\"remindedAt\":0,\"strikes\":0,\"posted\":[]}")
            .unwrap();
        assert_eq!(None, reminder_due(&dir, 99 * WEEK));
        // Route 2: every shape reported — nothing left to remind about.
        std::fs::write(dir.join("state.json"),
            "{\"nudges\":true,\"silenced\":false,\"remindedAt\":0,\"strikes\":0,\
             \"posted\":[\"run_tests/run/RUNNER_TIMEOUT\",\"inspect/source/TYPE_NOT_FOUND\"]}")
            .unwrap();
        assert_eq!(None, reminder_due(&dir, 99 * WEEK));
        // And the nudge switch does NOT silence reminders — they are distinct.
        std::fs::write(dir.join("state.json"),
            "{\"nudges\":false,\"silenced\":false,\"remindedAt\":0,\"strikes\":0,\"posted\":[]}")
            .unwrap();
        assert!(reminder_due(&dir, 99 * WEEK).is_some());
    }

    /// The user's ruling: "No annoying pop-ups. The main agent should tell the
    /// user!" This asserts the ABSENCE of any notification path in the crate —
    /// a rule stated only in prose is a rule the next change breaks.
    #[test]
    fn the_user_is_never_interrupted_by_a_popping_surface() {
        // The needles are ASSEMBLED, never written whole: a scan whose own
        // source contains its needles flags itself, and the obvious cure —
        // skipping this file — would blind it to the module it guards most.
        let banned: Vec<String> = [
            ("noti", "fy_rust"), ("notifi", "cation"), ("Message", "Box"),
            ("toa", "st"), ("dia", "log"), ("popu", "p"),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}").to_lowercase())
        .collect();

        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let body = std::fs::read_to_string(&path).unwrap();
            for (n, line) in body.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("").to_lowercase();
                for needle in &banned {
                    if code.contains(needle.as_str()) {
                        offenders.push(format!("{}:{} {needle}", path.display(), n + 1));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "the reminders speak through the agent: {offenders:?}");
    }
}
