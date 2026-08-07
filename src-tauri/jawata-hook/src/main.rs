//! The JAWATA client hook — one binary, every event, every platform.
//!
//! Sprint 28 (D-SHIM). This replaces ten generated shell scripts. The scripts
//! were the reason the per-prompt memory hook injected nothing for two weeks
//! while both products' suites stayed green: they parsed JSON with regexes, so
//! a payload whose SHAPE moved slightly still "matched" and produced an empty
//! cue, and an empty cue is indistinguishable from "nothing to say". Nothing
//! failed. Nothing was reported. The capability was simply absent.
//!
//! Two properties follow from that, and they shape every module here:
//!
//! * **A moved payload must surface as a typed error, never as silence.**
//!   Parsing is `serde_json` into declared shapes; a shape that no longer fits
//!   is a value this program can see and record, not a regex that quietly
//!   matches nothing.
//! * **Silence must be explained.** Every path that ends without emitting
//!   records WHY (Stage 8). "The hook ran and said nothing" is a fact about
//!   the hook, and it belongs in a log the user can read.
//!
//! And one absolute: **this process never blocks the editor.** Whatever
//! happens — a panic in a role, an unreachable resident, an absent config, a
//! stdin that never closes — the exit status is 0 and stdout carries nothing
//! the client will choke on. The fail-safe boundary (Stage 5) is the single
//! place that guarantees it.

mod config;
mod cue;
mod emit;
mod guard;
mod pipeline;
mod query;
mod roles;
mod safety;

use safety::{Outcome, SilenceReason};

/// `main` deliberately holds no logic. Everything decidable lives in
/// [`pipeline::run`], which takes its inputs as arguments and can be driven
/// without a client, a resident, or a process exit — because a hook whose only
/// test needs a live editor is a hook nobody tests.
fn main() {
    // FIRST, before anything that could block: whatever the main thread is
    // doing when the deadline passes, the process ends with status 0.
    safety::arm_watchdog(safety::TOTAL_DEADLINE);

    let argv0 = std::env::args().next().unwrap_or_default();
    let outcome = safety::run_guarded(move || dispatch(&argv0));

    // Stage 8 writes `outcome` to the silence log here. Until then the reason
    // is computed and carried — the value already exists, which is what makes
    // that log a small addition rather than a redesign.
    safety::exit_with(&outcome);
}

fn dispatch(argv0: &str) -> Outcome {
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
mod tests {
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
