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

mod cue;
mod roles;

fn main() {
    // Stage 5 lands cue/query/emit/roles and the fail-safe boundary behind
    // this entry point. Until then the binary exists, compiles, and does the
    // only thing a hook is unconditionally required to do.
    std::process::exit(0);
}
