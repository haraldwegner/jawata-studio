//! C6 exit clause 7 — the sibling channels, checked against the store's CURRENT
//! answers rather than assumed.
//!
//! > The `PreToolUse` recall and the session primer ride the store contract that
//! > killed the per-prompt hook. **State the finding; do not assume it.**
//!
//! The per-prompt hook injected nothing for two weeks because the answer's shape
//! moved under a regex. Its siblings read the same envelope. Nobody had checked
//! whether they still parse — and "they probably do" is exactly the assumption
//! that cost the two weeks.
//!
//! So the bodies in `tests/store-answers/` are REAL: captured 2026-08-07 from a
//! jawata resident built from this sprint's tree, over HTTP, by the same
//! `experience` calls the hooks make. They are committed so the check runs
//! without a live JVM, and so a future contract change shows up as a diff on a
//! file rather than as silence in the field.
//!
//! **The finding, per channel, recorded here because the plan asks for it to be
//! stated:**
//!
//! | Channel | Call | Verdict |
//! |---|---|---|
//! | session primer | `experience(kind=primer, format=text)` | HOLDS — absence parsed as an observed absence |
//! | `PreToolUse` recall, symbol cue | `experience(kind=recall, format=text, symbol=…)` | HOLDS — answer parsed, text intact |
//! | `PreToolUse` recall, symptom cue | `experience(kind=recall, format=text, symptom=…)` | HOLDS — absence parsed as an observed absence |
//!
//! Both absence phrasings the store actually emits — `"No domain knowledge
//! loaded."` and `"No known knowledge for this cue."` — are recognised as
//! ABSENCES rather than as answers. That is the distinction the two-week outage
//! turned on, and it is now pinned to the store's own words.

use jawata_hook::query::{parse_answer, Answer};

const PRIMER: &str = include_str!("store-answers/primer.json");
const RECALL_SYMBOL: &str = include_str!("store-answers/recall-symbol.json");
const RECALL_SYMPTOM: &str = include_str!("store-answers/recall-symptom.json");

#[test]
fn the_session_primer_channel_still_parses() {
    match parse_answer(PRIMER) {
        Ok(Answer::Nothing) => {}   // the captured store had no domain layer
        Ok(Answer::Text(t)) => assert!(!t.trim().is_empty(), "an empty answer is the outage"),
        Err(e) => panic!(
            "the PRIMER channel no longer parses the store's answer: {e:?}\n\
             This is the contract that killed the per-prompt hook, on its sibling."
        ),
    }
}

#[test]
fn the_pretooluse_recall_channel_still_parses_a_real_answer() {
    // The POSITIVE path, which is the one that broke — an absence-only fixture
    // would have exercised the wrong half.
    match parse_answer(RECALL_SYMBOL) {
        Ok(Answer::Text(t)) => {
            assert!(t.contains("[lesson]"), "the answer's own shape is intact: {t}");
            assert!(!t.trim().is_empty());
        }
        other => panic!(
            "the PreToolUse recall channel did not return the seeded answer: {other:?}\n\
             The capture contains one; parsing it as anything else is the defect."
        ),
    }
}

#[test]
fn a_symptom_cue_that_finds_nothing_is_an_observed_absence_not_a_failure() {
    assert_eq!(
        Ok(Answer::Nothing),
        parse_answer(RECALL_SYMPTOM),
        "the store said it had nothing; reading that as an error would make every \
         empty cue look like a broken channel"
    );
}

#[test]
fn the_stores_own_absence_phrasings_are_the_ones_we_recognise() {
    // THE distinction the outage turned on. These strings come from the
    // captures above, not from our source — if the store rewords an absence,
    // this fails and the hook stops mistaking an absence for an answer.
    for (name, body) in [("primer", PRIMER), ("recall-symptom", RECALL_SYMPTOM)] {
        assert_eq!(
            Ok(Answer::Nothing),
            parse_answer(body),
            "{name}: the store's absence wording is no longer recognised — the hook would \
             treat it as an ANSWER and inject the words 'No known knowledge' into context"
        );
    }
}

#[test]
fn the_captures_are_real_envelopes_not_hand_written_ones() {
    // A fixture someone typed proves nothing about the live contract. Each
    // capture must carry the full MCP envelope the resident emits.
    for (name, body) in [
        ("primer", PRIMER),
        ("recall-symbol", RECALL_SYMBOL),
        ("recall-symptom", RECALL_SYMPTOM),
    ] {
        let v: serde_json::Value = serde_json::from_str(body)
            .unwrap_or_else(|e| panic!("{name} is not JSON: {e}"));
        assert_eq!("2.0", v["jsonrpc"], "{name} lacks the JSON-RPC envelope");
        assert!(v["result"]["content"][0]["text"].is_string(), "{name} lacks the content block");
        // The `meta.steering` the server appends is what a naive regex used to
        // swallow; its presence is why these captures are worth having.
        let inner = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(inner.contains("\"meta\""), "{name} lacks the trailing meta the server appends");
    }
}
