//! The JAWATA client hook, as a library.
//!
//! The modules live here rather than in `main.rs` so integration tests can
//! drive them as a consumer does. That is not tidiness: the C5 audit found
//! `query::ask` — the real transport — with ZERO coverage, because a binary
//! crate's modules cannot be reached from `tests/`. The unreachable-endpoint
//! hazard was being "tested" by handing the pipeline an error value built by
//! hand, which asserts nothing about whether the transport ever produces one.
//!
//! `main.rs` stays a thin shell over [`run`].

pub mod autonomy;
pub mod config;
pub mod cue;
pub mod editgate;
pub mod emit;
pub mod field;
pub mod guard;
pub mod observer;
pub mod pipeline;
pub mod query;
pub mod recallgate;
pub mod recallledger;
pub mod roles;
pub mod safety;
pub mod silence;
pub mod stop;

use safety::{Outcome, SilenceReason};

/// One hook invocation, from `argv[0]` to an outcome. Everything decidable is
/// here; `main` only arms the watchdog and exits.
pub fn dispatch(argv0: &str) -> Outcome {
    let Some(role) = roles::role_for_binary(argv0) else {
        return Outcome::Silent(SilenceReason::UnknownRole(argv0.to_string()));
    };
    let config = match config::load() {
        Ok(c) => c,
        Err(reason) => return Outcome::Silent(reason),
    };
    let payload = match safety::read_stdin(safety::STDIN_DEADLINE) {
        Ok(p) => p,
        Err(reason) => return Outcome::Silent(reason),
    };
    let store = pipeline::LiveStore(query::Endpoint {
        url: config.url.clone(),
        token: config.token.clone(),
        timeout: config.timeout(),
    });
    pipeline::run(role, &config, &payload, &store)
}

/// The deploy's post-write self-check, mirrored from the script generation:
/// under `JAWATA_HOOK_SELFTEST=1` each role emits its contract shape through
/// the REAL render path — the same [`emit::render`] the live path uses — so
/// the deploy validates the exact bytes a real emission would produce,
/// without a store, a client, or stdin. (Sprint 21a item J, carried into the
/// binary: a self-check that takes a different route than production
/// validates a different program.)
pub fn selftest(argv0: &str) -> Outcome {
    use emit::Emission;
    use roles::{Client, Role};
    let Some(role) = roles::role_for_binary(argv0) else {
        return Outcome::Silent(SilenceReason::UnknownRole(argv0.to_string()));
    };
    let emission = match role {
        Role::Primer | Role::UserPrompt | Role::ToolRecall | Role::Observer => emit::context_for(
            role,
            Client::ClaudeCode,
            "[lesson] selftest canned line (accepted)".to_string(),
        ),
        Role::Guard => Emission::Permission { allowed: true, reason: String::new() },
        // The BLOCK shape, deliberately: it is the one with content, and the
        // deploy-side check accepts block-with-reason — so the selftest proves
        // the shape a real refusal would take.
        Role::Stop => Emission::StopDecision {
            reason: "selftest: the gate can block".to_string(),
        },
    };
    match emit::render(Client::ClaudeCode, &emission) {
        Some(rendered) => Outcome::Emitted(rendered),
        None => Outcome::Silent(SilenceReason::CannotInject),
    }
}

#[cfg(test)]
mod selftest_tests {
    use super::*;

    #[test]
    fn every_deployed_role_selftests_to_its_contract_shape() {
        // The four roles the deploy self-checks, each against the assertion
        // the deploy actually makes on its output.
        for (name, key) in [
            ("/x/jawata-hook-primer", "additionalContext"),
            ("/x/jawata-hook-recall", "additionalContext"),
            ("/x/jawata-hook-userprompt", "additionalContext"),
        ] {
            match selftest(name) {
                Outcome::Emitted(s) => {
                    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                    assert!(
                        v["hookSpecificOutput"][key].as_str().is_some_and(|c| !c.is_empty()),
                        "{name}: {s}"
                    );
                }
                other => panic!("{name} must emit its contract shape: {other:?}"),
            }
        }
        match selftest("/x/jawata-hook-stop") {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!("block", v["decision"], "{s}");
                assert!(v["reason"].as_str().is_some_and(|r| !r.is_empty()), "{s}");
            }
            other => panic!("the stop selftest must emit a Stop decision: {other:?}"),
        }
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    #[test]
    fn an_unowned_binary_name_names_itself_rather_than_guessing_a_role() {
        // A copy someone renamed, or a client entry still pointing at a name
        // we retired. Guessing a role here would run the wrong concern against
        // the wrong event.
        match dispatch("/usr/local/bin/jawata-hook-typo") {
            Outcome::Silent(SilenceReason::UnknownRole(name)) => assert!(name.contains("typo")),
            other => panic!("expected UnknownRole, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_never_panics_on_a_hostile_argv0() {
        let long = "x".repeat(4096);
        for argv0 in ["", "/", "\\", "jawata-hook-", "…", long.as_str()] {
            let out = safety::run_guarded(|| dispatch(argv0));
            assert!(matches!(out, Outcome::Silent(_)), "{argv0:?} produced {out:?}");
        }
    }
}
