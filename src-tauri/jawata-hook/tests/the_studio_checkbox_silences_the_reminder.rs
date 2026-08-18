//! HALF TWO OF THE FULL LOOP (Sprint 28b, Stage 5 exit clause: "checkbox stops
//! reminders (full loop with Stage 4)").
//!
//! The studio's `/report` tile writes the go-silent checkbox into
//! `<workspace>/field/state.json`; this binary reads that file on every session
//! start to decide whether a reminder is due. Neither crate may depend on the
//! other — `dependency_edges.rs` asserts the studio→hook edge is absent, and
//! the hook's dependency list forbids the reverse — so the loop is closed the
//! way the silence log's cap is: BOTH SIDES PINNED TO THE SAME LITERAL.
//!
//! The literal below is what `jawata_studio::field_view::write_state` produces
//! for a first click, asserted there by
//! `the_checkbox_writes_the_bytes_the_hook_reads_as_silence` against the
//! constant `CHECKBOX_SILENCED_STATE`. If the studio's encoding drifts — a
//! pretty printer, a renamed key, a space after a colon — that test fails on
//! its side and this one keeps proving what the hook needs on ours, so the
//! disagreement cannot pass unnoticed in either direction.

use std::path::{Path, PathBuf};

use jawata_hook::field::{reminder_due, REMINDER_INTERVAL_MILLIS};

/// Byte-for-byte what the studio checkbox writes. DO NOT hand-edit: change it
/// only together with the studio constant it mirrors.
const CHECKBOX_SILENCED_STATE: &str =
    "{\"nudges\":true,\"posted\":[],\"remindedAt\":0,\"silenced\":true,\"strikes\":0}";

/// The same shape with the checkbox CLEARED — the discriminator, so this file
/// cannot pass by asserting something that is true no matter what.
const CHECKBOX_CLEARED_STATE: &str =
    "{\"nudges\":true,\"posted\":[],\"remindedAt\":0,\"silenced\":false,\"strikes\":0}";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "jawata-checkbox-loop-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A pile with a shape well over the threshold, so a reminder is genuinely owed
/// unless something silences it.
fn seed_pile(dir: &Path) {
    let mut pile = String::from("{\"pileFormat\":1,\"contract\":1}\n");
    for _ in 0..4 {
        pile.push_str(
            "{\"t\":1,\"tool\":\"run_tests\",\"kind\":\"run\",\"ok\":false,\
             \"code\":\"RUNNER_TIMEOUT\",\"lat\":5,\"client\":\"claude_code\",\"ver\":\"3_10_0\"}\n",
        );
    }
    std::fs::write(dir.join("pile.jsonl"), pile).unwrap();
}

#[test]
fn the_studio_checkbox_stops_the_periodic_reminder() {
    let dir = scratch("silences");
    seed_pile(&dir);

    // Before the click: a reminder is owed. Without this the test below could
    // pass on a machine where nothing was ever due.
    std::fs::write(dir.join("state.json"), CHECKBOX_CLEARED_STATE).unwrap();
    assert!(
        reminder_due(&dir, 40 * REMINDER_INTERVAL_MILLIS).is_some(),
        "the fixture must actually owe a reminder, or the assertion below proves nothing"
    );

    // The click, as the studio's atomic writer leaves it on disk.
    std::fs::write(dir.join("state.json"), CHECKBOX_SILENCED_STATE).unwrap();
    assert_eq!(
        None,
        reminder_due(&dir, 40 * REMINDER_INTERVAL_MILLIS),
        "the go-silent checkbox must stop the reminder the agent speaks"
    );

    // And clearing it starts them again — silence is a choice, not a one-way door.
    std::fs::write(dir.join("state.json"), CHECKBOX_CLEARED_STATE).unwrap();
    assert!(reminder_due(&dir, 40 * REMINDER_INTERVAL_MILLIS).is_some());
}

/// The checkbox is NOT the no-nudges switch. The studio writes both keys into
/// one file, and a reader that confused them would silence the wrong half.
#[test]
fn the_checkbox_leaves_the_no_nudges_switch_alone() {
    let dir = scratch("distinct");
    seed_pile(&dir);
    std::fs::write(dir.join("state.json"), CHECKBOX_SILENCED_STATE).unwrap();
    assert!(
        jawata_hook::field::nudge_due(&dir).is_some(),
        "silencing the reminder must leave the in-session pointer switched on"
    );
}
