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

/// THE CUTOVER GATE. A rule the bash gate has and the binary does not is an
/// open item — permitted while the script still ships, fatal once it does not.
/// Flip `SCRIPTS_RETIRED` in the same change that stops deploying them.
#[test]
fn no_rule_is_lost_when_the_scripts_retire() {
    const SCRIPTS_RETIRED: bool = false;
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
