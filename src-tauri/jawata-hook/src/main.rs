//! The JAWATA client hook — one binary, every event, every platform.
//!
//! Sprint 28 (D-SHIM). This replaces ten generated shell scripts. The scripts
//! were the reason the per-prompt memory hook injected nothing for two weeks
//! while both products' suites stayed green: they parsed JSON with regexes, so
//! a payload whose SHAPE moved slightly still "matched" and produced an empty
//! cue, and an empty cue is indistinguishable from "nothing to say". Nothing
//! failed. Nothing was reported. The capability was simply absent.
//!
//! Everything decidable lives in the library beside this file, so it can be
//! driven from integration tests without a client, a resident, or a process
//! exit. This file is the shell: arm the watchdog, resolve the role, exit.

use jawata_hook::safety::Outcome;

fn main() {
    let argv0 = std::env::args().next().unwrap_or_default();
    let explain = std::env::args().any(|a| a == "--explain");

    let role_name = jawata_hook::roles::role_for_binary(&argv0)
        .map(|r| r.name().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // FIRST, before anything that could block: whatever the main thread is
    // doing when the deadline passes, the process ends with status 0 — and now
    // the deadline path WRITES ITS OWN REASON before it does. A timeout used to
    // be the one silence this log could not explain, because `record` below
    // runs only after the body returns, which on a timeout it never does.
    jawata_hook::safety::arm_watchdog_recording(
        jawata_hook::safety::TOTAL_DEADLINE,
        role_name.clone(),
    );

    // The deploy's self-check: no stdin, no store — the role's contract shape
    // through the real render path (see `jawata_hook::selftest`).
    if std::env::var("JAWATA_HOOK_SELFTEST").ok().as_deref() == Some("1") {
        let argv0_owned = argv0.clone();
        let outcome =
            jawata_hook::safety::run_guarded(move || jawata_hook::selftest(&argv0_owned));
        jawata_hook::safety::exit_with(&outcome);
    }

    let argv0_owned = argv0.clone();
    let outcome =
        jawata_hook::safety::run_guarded(move || jawata_hook::dispatch(&argv0_owned));

    // Stage 8: the reason is written down rather than discarded. It was always
    // computed — every path in the pipeline produces one — and thrown away a
    // line before it could answer "why did nothing happen?".
    let logged = jawata_hook::silence::record_reporting(&role_name, &outcome);

    // `--explain` runs THE REAL PATH and reports it, rather than describing
    // what the path would do. A diagnostic that takes a different route
    // answers a question nobody asked: the two-week outage would have been
    // invisible to a self-check that did not make the same call.
    if explain {
        match &outcome {
            Outcome::Emitted(rendered) => {
                eprintln!("role {role_name}: EMITTED {} bytes", rendered.len());
            }
            Outcome::Silent(reason) => {
                eprintln!(
                    "role {role_name}: SILENT [{}] {reason:?}",
                    reason.tag()
                );
            }
        }
    }

    // F4: a DROPPED record (past the hard ceiling) and an UNWRITABLE log were
    // byte-identical from outside — and a small stale log is indistinguishable
    // from "the hook never ran", which is the two-week outage's own signature
    // one level up. --explain now says which. Off the silent fire path.
    if explain && !logged {
        eprintln!("role {role_name}: the silence log was NOT written (dropped past the ceiling, or unwritable)");
    }
    jawata_hook::safety::exit_with(&outcome);
}
