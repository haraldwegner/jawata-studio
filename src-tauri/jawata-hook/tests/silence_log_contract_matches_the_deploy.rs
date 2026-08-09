//! The silence log's numbers must match the contract BOTH crates read.
//!
//! Sprint 28, C8 architect finding F2. The hook appends to `hook_silence.log`
//! and the studio renames it away: two processes, two crates, one file. That
//! seam was declared nowhere, and it had already produced three hand-copied
//! facts with nothing asserting they agree —
//!
//!   * the cap: `silence::MAX_BYTES` here, a bare `const MAX_BYTES` in
//!     `manager_service::rotate_silence_log`;
//!   * the filename: `log_path_for()` here, a string literal there;
//!   * the rotated name: only there, the hook never names it.
//!
//! A shared Rust constant is impossible for the reason `hook-events.json`
//! already documents: neither crate may depend on the other, and both forbidden
//! edges are asserted by tests. A committed data file is the one thing both
//! sides can read without an edge — so the cap lives there now, and this test
//! is the hook's half of the assertion. The studio's half lives beside
//! `rotate_silence_log`.
//!
//! Change a number in the contract and BOTH sides fail until they agree.

use serde_json::Value;

const EVENTS: &str = include_str!("../../hook-events.json");

fn contract() -> Value {
    serde_json::from_str(EVENTS).expect("hook-events.json must parse — it is a committed contract")
}

fn silence_row() -> Value {
    contract()["seam_files"]["hook_silence.log"].clone()
}

#[test]
fn the_cap_matches_the_contract() {
    let want = silence_row()["max_bytes"]
        .as_u64()
        .expect("seam_files.hook_silence.log.max_bytes must be a number");
    assert_eq!(
        want,
        jawata_hook::silence::MAX_BYTES,
        "the hook's cap disagrees with the contract the studio rotates by"
    );
}

#[test]
fn the_hard_ceiling_matches_the_contract() {
    let row = silence_row();
    let max = row["max_bytes"].as_u64().expect("max_bytes");
    let mult = row["hard_ceiling_multiple"]
        .as_u64()
        .expect("seam_files.hook_silence.log.hard_ceiling_multiple must be a number");
    assert_eq!(
        max * mult,
        jawata_hook::silence::HARD_CEILING_BYTES,
        "the ceiling past which the hook DROPS records disagrees with the contract"
    );
}

/// The filename is the studio's only handle on this file. If the hook renames
/// it, rotation silently stops and the log grows to the ceiling — where records
/// are dropped, correctly but invisibly to anyone reading the old name.
#[test]
fn the_log_filename_matches_the_contract() {
    let path = jawata_hook::silence::log_path_for(std::path::Path::new(
        "/opt/jawata/jawata-hook-primer",
    ))
    .expect("a log path");
    let name = path.file_name().and_then(|n| n.to_str()).expect("a file name");
    let rows = contract();
    let want = rows["seam_files"]
        .as_object()
        .expect("seam_files is an object")
        .keys()
        .find(|k| k.as_str() == name)
        .map(|k| k.to_string());
    assert_eq!(
        Some(name.to_string()),
        want,
        "the hook writes {name:?}, which has no row in seam_files"
    );
}

/// The contract states who bounds the file, because getting that wrong is what
/// six audit rounds were spent on. This asserts the ROLE, not the mechanism:
/// the hook must never be the rotator.
#[test]
fn the_contract_says_the_studio_rotates_and_the_hook_only_appends() {
    let row = silence_row();
    assert_eq!(
        Some("studio"),
        row["rotator"].as_str(),
        "the hook must never rotate this file — in-path bounding destroyed records"
    );
    assert_eq!(Some("hook"), row["writer"].as_str());
    assert_eq!(
        1,
        row["retention_generations"].as_u64().expect("retention_generations"),
        "one generation is the stated trade; changing it is a decision, not a detail"
    );
}

/// Sprint 28 outcome audit, F3: the `hook_config.json` seam row was asserted
/// by NEITHER side. This is the hook's half: the filename the crate actually
/// reads is the filename the contract declares, and the row says the hook is
/// its reader. (The studio's half is its temp-file-plus-rename writer tests;
/// the `atomicity` cell names that discipline for the human reader.)
#[test]
fn the_hook_config_seam_row_names_the_file_this_crate_reads() {
    let row = contract()["seam_files"][jawata_hook::config::CONFIG_FILE].clone();
    assert!(
        row.is_object(),
        "seam_files has no row for {} — the file this crate reads at every \
         invocation is an undeclared seam",
        jawata_hook::config::CONFIG_FILE
    );
    assert_eq!("hook", row["reader"].as_str().unwrap_or(""),
        "the contract must name the hook as this file's reader");
    assert_eq!("studio", row["writer"].as_str().unwrap_or(""),
        "the contract must name the studio as this file's writer");
}
