//! No stop rule may be lost at cutover.
//!
//! Sprint 28, C8 architect finding F3. Two stop gates exist: the deployed bash
//! script, live today, and this crate's `stop::judge`, built and inert. Their
//! rule sets diverged — the audit-fix-loop trigger and the unjudged-ask check
//! went into the bash generation, which the architecture schedules for
//! DELETION. At cutover the binary would ship without rules the script had:
//! the hook-outage shape inverted, at the one moment nobody reads the old file.
//!
//! This test does not demand parity. It demands that every rule has a DECLARED
//! status on both sides, so a rule can be ported or deliberately dropped but
//! never silently lost. The cutover checkpoint reads this list.

use serde_json::Value;

const EVENTS: &str = include_str!("../../hook-events.json");

fn rules() -> Value {
    serde_json::from_str::<Value>(EVENTS)
        .expect("hook-events.json is a committed contract")["stop_rules"]["rules"]
        .clone()
}

const ALLOWED: [&str; 4] = ["present", "absent", "todo", "present-but-inert"];

#[test]
fn every_stop_rule_declares_its_status_on_both_sides() {
    let r = rules();
    let map = r.as_object().expect("stop_rules.rules is an object");
    assert!(!map.is_empty(), "the rule list must not be empty");
    for (name, row) in map {
        for side in ["bash", "rust"] {
            let v = row[side]
                .as_str()
                .unwrap_or_else(|| panic!("rule {name:?} has no {side} status"));
            assert!(
                ALLOWED.contains(&v),
                "rule {name:?} {side} status {v:?} is not one of {ALLOWED:?}"
            );
        }
        assert!(
            row["what"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
            "rule {name:?} must say what it does, or the cutover reader cannot judge it"
        );
    }
}

/// The RUST column, asserted the way the studio-side test asserts the bash one.
///
/// The asymmetry was real: `manager_service` pinned every `bash: "present"`
/// claim to a marker in the script template, and nothing at all checked the
/// rust column — so a rule could be deleted from `judge` while the contract
/// went on declaring it present. That is the same divergence the whole section
/// exists to prevent, facing the other way.
#[test]
fn every_stop_rule_that_claims_rust_code_has_its_marker_in_judge() {
    // THE PRODUCTION HALF ONLY. The first version of this scanned the whole
    // file — tests and doc comments included — so every marker was satisfied by
    // the tests that exercise the rule, and the assertion held even with the
    // rule DELETED from `judge`. The C5 audit proved it by doing exactly that:
    // 623 characters removed from the branch, `stop_rule_parity` still 3/3.
    // Two markers ("Rule A", "Rule B") live in the module doc comment and would
    // have survived even a test-stripped scan, so the cut is at the test module
    // and the doc comment sits above it either way — which is why the markers
    // below are the RULE NAMES the reasons print, not prose from the header.
    // JUDGE'S BODY ONLY, and the markers are the rule NAMES the reasons print.
    //
    // Round 1 scanned the whole file: every marker was satisfied by the tests,
    // so deleting a rule left this green. Round 2 cut it to production code and
    // found FOUR of nine still dead — `already_bounced` was satisfied by the
    // struct field, `UNJUDGED ASK` by a doc comment, and Rules A and B by prose
    // in the module header. The five that held all had one thing in common:
    // their marker lived inside the emitted `reason` string. So the two prose
    // ones now say their own name in the text the model reads, and the scan
    // stops at the end of `judge` — a doc comment above a deleted branch can no
    // longer vouch for it.
    const WHOLE_FILE: &str = include_str!("../src/stop.rs");
    let start = WHOLE_FILE.find("pub fn judge(").expect("judge is the gate; find it");
    let end = WHOLE_FILE[start..]
        .find("\n/// Parse the last assistant turn")
        .map(|o| start + o)
        .expect("judge is followed by read_turn's doc comment; if that moved, re-cut this");
    let judge = &WHOLE_FILE[start..end];
    let marker: std::collections::HashMap<&str, &str> = [
        ("anti_loop", "already_bounced"),
        ("audit_fix_loop", "AUDIT-FIX LOOP"),
        // `unjudged_ask` is gone from this table with the rule (v4.0.0). The
        // contract row survives as "absent" carrying the reason, which is what
        // keeps a retirement distinguishable from a deletion nobody explained.
        //
        // The judge shares Rule B's marker rather than owning one, and that is
        // a fact about its SHAPE: bounce-and-verify means both of its outcomes
        // — demand the seat, and carry the seat's next action — are Rule B
        // blocks. A second marker would claim a second rule.
        // THE CONSTANT, not the string it expands to. Every branch names the
        // seat through `JUDGE_SEAT` so the deployed file, the block reason and
        // the launch detector cannot drift to three different spellings — which
        // means the literal "autocontinue" does not appear in `judge` at all,
        // and a marker of that word would fail here while the rule was present
        // and working.
        ("autocontinue_judge", "JUDGE_SEAT"),
        ("unreported_degradation", "UNREPORTED DEGRADATION"),
        
        ("nothing_armed_rule_b", "RULE B:"),
        ("seat_discipline", "SEAT DISCIPLINE"),
        ("decision_test_length", "TOO LONG"),
        ("undefined_abbreviations", "UNDEFINED"),
    ]
    .into();
    for (rule, row) in rules().as_object().unwrap() {
        // `present-but-inert` counts: the rule IS in the code and can still be
        // deleted while the contract declares it. Skipping it here would have
        // silently turned the guard off the moment a rule was honestly marked
        // inert, which is the opposite of what honesty should cost.
        if !matches!(row["rust"].as_str(), Some("present") | Some("present-but-inert")) {
            continue;
        }
        let m = marker.get(rule.as_str()).unwrap_or_else(|| {
            panic!(
                "{rule}: declared present in rust but this test knows no marker \
                 for it — add one here alongside the rule"
            )
        });
        assert!(
            judge.contains(m),
            "{rule}: declared present in the rust generation, but its marker \
             {m:?} is not in stop.rs — the contract says more than the code does"
        );
    }
}

/// THE CUTOVER GATE. A rule the bash gate has and the binary does not is an
/// open item — permitted while the script still ships, fatal once it does not.
/// Flip `SCRIPTS_RETIRED` in the same change that stops deploying them.
#[test]
fn no_rule_is_lost_when_the_scripts_retire() {
    // FLIPPED. All nine rules now exist on both sides, and the deploy points a
    // client at the binary when one is present. From here, a rule that lives
    // only in the scripts fails this test.
    const SCRIPTS_RETIRED: bool = true;
    let r = rules();
    let missing: Vec<String> = r
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, row)| row["bash"] == "present" && row["rust"] == "todo")
        .map(|(k, _)| k.clone())
        .collect();
    if SCRIPTS_RETIRED {
        assert!(
            missing.is_empty(),
            "the scripts are retired but these rules exist only in them: {missing:?}"
        );
    } else {
        // While both ship, the list is the worklist — asserted non-silent, not empty.
        assert!(
            !missing.is_empty() || r.as_object().unwrap().is_empty(),
            "if nothing is outstanding, retire the scripts and flip SCRIPTS_RETIRED"
        );
    }
}

/// The other half of the shared budget fact.
///
/// The studio's store-slow line is derived from these two numbers and asserted
/// against the contract file; this asserts the contract still describes the
/// HOOK. Without it the contract could drift from the code it claims to
/// publish, and the studio would go on deriving a threshold from a fiction —
/// the same divergence `stop_rules` exists to prevent, one file over.
#[test]
fn the_contract_publishes_the_hooks_real_budget() {
    let contract: Value = serde_json::from_str(EVENTS).expect("committed contract");
    let budget = &contract["hook_budget"];
    assert_eq!(
        jawata_hook::safety::STDIN_DEADLINE.as_millis() as u64,
        budget["stdin_deadline_millis"].as_u64().expect("stdin_deadline_millis"),
        "hook-events.json publishes a deadline the hook does not use"
    );
    assert_eq!(
        jawata_hook::pipeline::BUDGET_MARGIN_MILLIS,
        budget["budget_margin_millis"].as_u64().expect("budget_margin_millis"),
        "hook-events.json publishes a margin the hook does not use"
    );
    assert_eq!(
        jawata_hook::safety::STDIN_DEADLINE.as_millis() as u64
            - jawata_hook::pipeline::BUDGET_MARGIN_MILLIS,
        budget["store_slow_millis"].as_u64().expect("store_slow_millis"),
        "the published store-slow line is not what a recall actually gets"
    );
}
