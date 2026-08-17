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
const LEGITIMATELY_QUIET: &[&str] = &[
    "store-had-nothing", "no-cues", "stop-allowed", "role-absent-on-client",
    "recorded-not-injected", "nothing-to-observe",
];

/// One channel's folded counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelCounters {
    pub fired: u64,
    pub emitted: u64,
    /// Suppressions by their bounded tag — the reason enum, never free text.
    pub suppressed: BTreeMap<String, u64>,
}

impl ChannelCounters {
    fn answered_but_suppressed(&self) -> u64 {
        ANSWERED_BUT_SUPPRESSED
            .iter()
            .filter_map(|t| self.suppressed.get(*t))
            .sum()
    }

    /// The deterministic condition: answered > 0 while emitted == 0.
    pub fn dead(&self) -> bool {
        self.emitted == 0 && self.answered_but_suppressed() > 0
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
pub fn fold_lines<'a>(lines: impl Iterator<Item = &'a str>) -> BTreeMap<String, ChannelCounters> {
    let mut by_role: BTreeMap<String, ChannelCounters> = BTreeMap::new();
    for line in lines {
        let mut parts = line.splitn(4, '\t');
        let (Some(_millis), Some(role), Some(tag)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if role.is_empty() || tag.is_empty() {
            continue;
        }
        let counters = by_role.entry(role.to_string()).or_default();
        counters.fired += 1;
        if tag == "emitted" {
            counters.emitted += 1;
        } else {
            *counters.suppressed.entry(tag.to_string()).or_insert(0) += 1;
        }
    }
    by_role
}

/// Fold a silence log file; a missing or unreadable file folds to empty —
/// callers surface THAT distinctly (an absent log is "the hook never ran",
/// which is its own finding, not a healthy zero).
pub fn fold_file(path: &Path) -> BTreeMap<String, ChannelCounters> {
    match std::fs::read_to_string(path) {
        Ok(content) => fold_lines(content.lines()),
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
    fn fixture_log() -> String {
        let mut log = String::new();
        for _ in 0..3 {
            log.push_str("1700000000000\tuser-prompt\tcannot-inject\t\n");
        }
        log.push_str("1700000000000\ttool-recall\temitted\t\n");
        log.push_str("1700000000000\ttool-recall\tstore-had-nothing\t\n");
        log.push_str("1700000000000\tprimer\temitted\t\n");
        log.push_str("1700000000000\tguard\tstore-had-nothing\t\n");
        log
    }

    #[test]
    fn the_dead_channel_condition_fires_on_answered_but_never_emitted() {
        let folded = fold_lines(fixture_log().lines());
        assert_eq!(dead_channels(&folded), vec!["user-prompt".to_string()]);
        let dead = &folded["user-prompt"];
        assert_eq!(dead.fired, 3);
        assert_eq!(dead.emitted, 0);
        assert_eq!(dead.suppressed.get("cannot-inject"), Some(&3));
    }

    #[test]
    fn legitimate_absence_is_not_dead() {
        let folded = fold_lines(fixture_log().lines());
        assert!(!folded["guard"].dead(), "absence is often legitimate");
        assert!(folded["guard"].legitimately_quiet());
        assert!(!folded["tool-recall"].dead(), "it emitted");
    }

    #[test]
    fn a_contract_mismatch_counts_toward_dead() {
        let log = "1\tuser-prompt\tcontract-mismatch\tours=1 theirs=2\n";
        let folded = fold_lines(log.lines());
        assert_eq!(dead_channels(&folded), vec!["user-prompt".to_string()]);
    }

    #[test]
    fn garbage_lines_lose_themselves_never_the_fold() {
        let log = format!("not a record\n{}\t\n\n", fixture_log());
        let folded = fold_lines(log.lines());
        assert_eq!(folded["primer"].emitted, 1);
    }

    #[test]
    fn a_missing_log_folds_to_empty() {
        let folded = fold_file(Path::new("/nonexistent/hook_silence.log"));
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
}
