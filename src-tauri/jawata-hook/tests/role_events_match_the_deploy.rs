//! C5 exit clause 5, at the level the clause states.
//!
//! > The role table is **asserted against** the six Claude and four Cursor
//! > entries the deploy writes.
//!
//! The table previously asserted the literals `6` and `4`. That is a count,
//! not a linkage: nine of the ten event NAMES had no discriminator, so
//! renaming `afterMCPExecution` in the deploy while leaving `roles.rs` alone
//! left both crates green with the observer mapped to an event Cursor never
//! fires — the ten-scripts drift the table was built to end, reproduced inside
//! the table.
//!
//! `../hook-events.json` is the shared source. A shared Rust constant is
//! impossible here by design: the hook must not depend on the studio and the
//! studio must not depend on the hook, and both edges are asserted elsewhere.
//! A committed data file is the one thing both sides can read with no edge
//! between them.

use jawata_hook::roles::{Availability, Client, ROLES};

const EVENTS: &str = include_str!("../../hook-events.json");

fn table() -> serde_json::Value {
    serde_json::from_str(EVENTS).expect("hook-events.json must parse — it is a committed contract")
}

fn key_of(binary_name: &str) -> &str {
    binary_name.strip_prefix("jawata-hook-").unwrap_or(binary_name)
}

fn client_key(client: Client) -> &'static str {
    match client {
        Client::ClaudeCode => "claude-code",
        Client::Cursor => "cursor",
    }
}

#[test]
fn every_handled_role_uses_the_event_the_shared_contract_names() {
    let doc = table();
    let mut checked = 0;
    for spec in ROLES {
        let Availability::Handled { event } = spec.availability else {
            continue;
        };
        let expected = doc[client_key(spec.client)][key_of(spec.binary_name)]
            .as_str()
            .unwrap_or_else(|| {
                panic!(
                    "hook-events.json has no entry for {}/{} — the table handles a role the \
                     shared contract does not list",
                    client_key(spec.client),
                    key_of(spec.binary_name)
                )
            });
        assert_eq!(
            expected, event,
            "{:?}/{:?} maps to {event:?} but the shared contract says {expected:?}",
            spec.role, spec.client
        );
        checked += 1;
    }
    assert_eq!(10, checked, "six Claude + four Cursor entries must all be checked");
}

#[test]
fn the_contract_lists_nothing_the_table_does_not_handle() {
    // The other direction: an event added to the shared file and forgotten in
    // the table is the same drift, mirrored.
    let doc = table();
    for client in ["claude-code", "cursor"] {
        let entries = doc[client].as_object().expect("a client object");
        for role_key in entries.keys() {
            let found = ROLES.iter().any(|s| {
                client_key(s.client) == client
                    && key_of(s.binary_name) == role_key
                    && matches!(s.availability, Availability::Handled { .. })
            });
            assert!(
                found,
                "hook-events.json lists {client}/{role_key}, which the role table does not handle"
            );
        }
    }
}

#[test]
fn the_absences_are_declared_in_both_places_with_the_same_reason() {
    let doc = table();
    let absent = doc["_absent"].as_object().expect("_absent");
    let mut seen = 0;
    for spec in ROLES {
        if let Availability::Absent { because } = spec.availability {
            let key = format!("{}.{}", client_key(spec.client), key_of(spec.binary_name));
            let declared = absent[&key].as_str().unwrap_or_else(|| {
                panic!("{key} is absent in the table but not declared in hook-events.json")
            });
            assert_eq!(
                declared, because,
                "{key}: the two declarations of WHY it is absent have drifted"
            );
            seen += 1;
        }
    }
    assert_eq!(absent.len(), seen, "every declared absence must exist in the table");
}
