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

pub mod config;
pub mod cue;
pub mod emit;
pub mod guard;
pub mod pipeline;
pub mod query;
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
