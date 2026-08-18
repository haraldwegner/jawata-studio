//! The two silence-log folds classify every tag the same way (Sprint 28b
//! closing audit, F5).
//!
//! `jawata_hook::field` and `jawata_studio_lib::field_view` implement the same
//! fold twice, each with its own `ANSWERED_BUT_SUPPRESSED` and
//! `LEGITIMATELY_QUIET`. The DUPLICATION IS REQUIRED: studio must never link
//! the hook crate — a process that fires on every keystroke must not carry a
//! GUI toolkit, and `dependency_edges.rs` pins the absence of that edge in the
//! resolved dependency graph. What was missing is the other half: nothing made
//! the two lists AGREE, so one side could gain a tag and the other keep
//! classifying it as an unknown suppression. The consequence is not cosmetic —
//! a tag on one list and not the other means the tile and the hook disagree
//! about whether a channel is dead, which is the exact false-alarm class the
//! C2 F2 amendment and F6 both exist to remove.
//!
//! This closes it WITHOUT a dependency edge, the way the checkbox loop is
//! closed: a pinned literal both sides are held to. The difference is that
//! this file also READS the studio's source text — a file read, not a link, in
//! the same manner as the crates' own no-pop-up scans — so a change to either
//! list fails here rather than merely failing on the side that changed.
//!
//! Adding a tag is therefore a three-line change: both constants and the
//! literal below. That is the intended cost.

use std::path::PathBuf;

/// THE CONTRACT. Both crates' `ANSWERED_BUT_SUPPRESSED`, in order.
const ANSWERED_BUT_SUPPRESSED: &[&str] =
    &["cannot-inject", "contract-mismatch", "answer-unusable"];

/// THE CONTRACT. Both crates' `LEGITIMATELY_QUIET`, in order.
const LEGITIMATELY_QUIET: &[&str] = &[
    "store-had-nothing",
    "no-cues",
    "stop-allowed",
    "role-absent-on-client",
    "recorded-not-injected",
    "nothing-to-observe",
];

/// The studio's copy of the fold. Reached as a FILE, never as a crate.
fn studio_field_view() -> String {
    let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/field_view.rs"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read the studio's fold at {} ({e}). If that file moved, this assertion is \
             NOT being made — repoint it rather than deleting it.",
            path.display()
        )
    });
    assert!(
        text.len() > 5_000,
        "the studio's fold is suspiciously small ({} bytes) — a scan that reads the wrong file \
         passes by looking at nothing",
        text.len()
    );
    text
}

/// Pull one `const NAME: &[&str] = &[ … ];` out of Rust source, in order.
///
/// Deliberately strict: an empty result is a PARSE FAILURE, not an empty list,
/// and it panics rather than quietly comparing nothing.
fn string_list_in(source: &str, name: &str) -> Vec<String> {
    let marker = format!("const {name}: &[&str] =");
    let from = source
        .find(&marker)
        .unwrap_or_else(|| panic!("no `{marker}` in the studio's fold — it was renamed or removed"));
    let rest = &source[from + marker.len()..];
    let to = rest
        .find(';')
        .unwrap_or_else(|| panic!("`{name}` has no terminating `;`"));
    let body = &rest[..to];

    let mut items = Vec::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        items.push(chars.by_ref().take_while(|c| *c != '"').collect::<String>());
    }
    assert!(
        !items.is_empty(),
        "parsed no entries out of `{name}` — the shape changed and this test would otherwise \
         compare two empty lists and pass"
    );
    items
}

#[test]
fn the_hook_matches_the_pinned_contract() {
    assert_eq!(
        ANSWERED_BUT_SUPPRESSED,
        jawata_hook::field::ANSWERED_BUT_SUPPRESSED,
        "the hook's dead-channel numerator drifted from the pinned contract"
    );
    assert_eq!(
        LEGITIMATELY_QUIET,
        jawata_hook::field::LEGITIMATELY_QUIET,
        "the hook's by-design-quiet list drifted from the pinned contract"
    );
}

#[test]
fn the_studio_matches_the_pinned_contract() {
    let source = studio_field_view();
    assert_eq!(
        ANSWERED_BUT_SUPPRESSED.to_vec(),
        string_list_in(&source, "ANSWERED_BUT_SUPPRESSED"),
        "the studio's dead-channel numerator drifted from the pinned contract"
    );
    assert_eq!(
        LEGITIMATELY_QUIET.to_vec(),
        string_list_in(&source, "LEGITIMATELY_QUIET"),
        "the studio's by-design-quiet list drifted from the pinned contract"
    );
}

/// The direct statement, independent of the literal above: whatever the two
/// crates classify, they classify identically. If someone updates both
/// constants AND the pinned literal in one sweep, the two tests above stay
/// green and so does this one — as they should. If they update either side
/// alone, this fails naming the difference.
#[test]
fn neither_fold_knows_a_tag_the_other_does_not() {
    let source = studio_field_view();
    for list in ["ANSWERED_BUT_SUPPRESSED", "LEGITIMATELY_QUIET"] {
        let studio = string_list_in(&source, list);
        let hook: Vec<String> = match list {
            "ANSWERED_BUT_SUPPRESSED" => jawata_hook::field::ANSWERED_BUT_SUPPRESSED,
            _ => jawata_hook::field::LEGITIMATELY_QUIET,
        }
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            hook, studio,
            "{list} differs between the two folds. The copy is deliberate (studio must not link \
             the hook crate); two DIFFERENT copies are not — the tile and the hook would \
             disagree about which channels are dead."
        );
    }
}
