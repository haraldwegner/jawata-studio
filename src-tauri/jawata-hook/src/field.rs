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
}
