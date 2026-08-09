//! The hook crate's half of the `role_generations` contract.
//!
//! Sprint 28 outcome audit, F3: `hook-events.json` claimed "both crates assert
//! against this section" while ZERO hook-crate references to it existed — the
//! declaration that exists to prevent contract drift was itself drifting from
//! what the code enforces. This file makes the claim true from this side:
//! every declared role must resolve in THIS crate's argv[0] dispatch table
//! (a retired role's stale binary must still resolve to stay diagnosable),
//! every declared generation must be a value the deploy understands, and the
//! declaration must be COMPLETE — a claude-code role with no generation row is
//! a cutover decided by file existence again, which is the exact 3.7.2 defect
//! the section was introduced to end.
//!
//! The studio's half binds the deploy's live/retired lists to the same rows.

const EVENTS: &str = include_str!("../../hook-events.json");

fn contract() -> serde_json::Value {
    serde_json::from_str(EVENTS).expect("hook-events.json must parse — it is a committed contract")
}

#[test]
fn every_declared_role_resolves_in_the_dispatch_table_with_a_known_generation() {
    let contract = contract();
    let generations = contract["role_generations"]
        .as_object()
        .expect("role_generations section exists");
    let mut declared = 0;
    for (role, row) in generations {
        if role.starts_with('_') {
            continue; // _why / _scope prose keys
        }
        declared += 1;
        let binary_name = format!("jawata-hook-{role}");
        assert!(
            jawata_hook::roles::role_for_binary(&binary_name).is_some(),
            "{role}: declared in role_generations but jawata-hook-{role} does not \
             resolve in the dispatch table — a stale binary under that name would \
             run as UnknownRole instead of its role"
        );
        let live = row["live"].as_str().unwrap_or_else(|| {
            panic!("{role}: role_generations row carries no `live` string")
        });
        assert!(
            live == "binary" || live == "script",
            "{role}: unknown generation {live:?} — the deploy understands binary|script"
        );
        if live == "script" {
            assert!(
                row["until"].as_str().is_some_and(|u| !u.is_empty()),
                "{role}: a script-generation role must declare `until` — a drop \
                 with no stated cure is a drop, not a decision"
            );
        }
    }

    // Completeness: every claude-code role has a generation row.
    let claude = contract["claude-code"].as_object().expect("claude-code section");
    for role in claude.keys() {
        assert!(
            generations.contains_key(role),
            "{role}: handled for claude-code but has NO role_generations row — \
             its cutover would again be decided by what happens to be on disk"
        );
    }
    assert_eq!(claude.len(), declared,
        "role_generations declares a role claude-code does not handle");
}
