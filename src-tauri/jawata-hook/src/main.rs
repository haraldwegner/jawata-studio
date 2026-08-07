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

fn main() {
    // FIRST, before anything that could block: whatever the main thread is
    // doing when the deadline passes, the process ends with status 0.
    jawata_hook::safety::arm_watchdog(jawata_hook::safety::TOTAL_DEADLINE);

    let argv0 = std::env::args().next().unwrap_or_default();
    let outcome = jawata_hook::safety::run_guarded(move || jawata_hook::dispatch(&argv0));

    // Stage 8 writes `outcome` to the silence log here. Until then the reason
    // is computed and carried — the value already exists, which is what makes
    // that log a small addition rather than a redesign.
    jawata_hook::safety::exit_with(&outcome);
}
