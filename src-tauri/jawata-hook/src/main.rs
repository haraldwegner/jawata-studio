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
    // The run bracket is a LONG command by design, so it is answered before the
    // watchdog below is armed: that deadline exists for a hook event, and would
    // kill a bracketed build at its first minute.
    let all_args: Vec<String> = std::env::args().collect();
    if jawata_hook::bracket::is_bracket_call(&all_args) {
        std::process::exit(jawata_hook::bracket::main(&all_args[2..]));
    }

    // Sprint 28g S23 O2: the observer's detached judge. Started by a call and never awaited by
    // one; it has its own clock (the seat's), so it is answered before the hook watchdog is armed.
    if jawata_hook::judge::is_judge_call(&all_args) {
        let code = match jawata_hook::config::load() {
            Ok(config) => jawata_hook::judge::main(&all_args[2..], &|session, brief| {
                jawata_hook::judge::hire_as_child(&config, session, brief)
            }),
            Err(_) => 0,
        };
        std::process::exit(code);
    }

    // The cadence read is a report, not a hook event: no stdin, no store, no deadline.
    if jawata_hook::cadence::is_cadence_call(&all_args) {
        std::process::exit(jawata_hook::cadence::main(&all_args[2..]));
    }

    // REL44-1: which build this binary IS. The integration tests ask it first, because once a
    // stale sidecar copied over target/debug/jawata-hook made a whole run drive an older hook
    // without anything noticing — the one test whose rule the old binary lacked failed, and
    // every other test passed against the wrong program. No stdin, no store, no deadline.
    if all_args.get(1).map(String::as_str) == Some("--version") {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

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
