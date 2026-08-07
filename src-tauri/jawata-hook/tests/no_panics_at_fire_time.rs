//! The boundary clause that had no control.
//!
//! > **no `unwrap`/`expect` on any path reachable at fire time**
//!
//! The C5 audit enumerated every construct in non-test code and found the
//! clause HELD — zero `unwrap`, zero `expect`, zero `panic!` — and then noted
//! that nothing keeps it true: no lint, no test. Adding `.unwrap()` tomorrow
//! goes green.
//!
//! `catch_unwind` would catch such a panic, so this is not about correctness —
//! it is about the budget. A panic unwinds, runs the hook's silent path, and
//! costs the user latency on every prompt; and under `panic = "abort"` it
//! would not be caught at all. Cheaper to not panic.
//!
//! A source scan, not clippy, because clippy is not in the loop that runs on
//! every change — and a control nobody runs is the thing this sprint measures.

use std::path::Path;

/// Constructs that abort or unwind. `unwrap_or`, `unwrap_or_default` and
/// `unwrap_or_else` are TOTAL and deliberately absent from this list.
const FORBIDDEN: &[&str] = &[
    ".unwrap()",
    ".expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
    ".unwrap_err(",
];

#[test]
fn no_panicking_construct_is_reachable_when_the_hook_fires() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offences = Vec::new();

    for entry in std::fs::read_dir(&src).expect("the crate has a src directory") {
        let path = entry.expect("readable entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable source");
        // Everything from the first `#[cfg(test)]` onward is test code, which
        // may panic freely — that is what an assertion is.
        let production = match text.find("#[cfg(test)]") {
            Some(i) => &text[..i],
            None => &text[..],
        };
        for (n, line) in production.lines().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            for bad in FORBIDDEN {
                if code.contains(bad) {
                    offences.push(format!(
                        "{}:{}: {bad} — {}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        n + 1,
                        code.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offences.is_empty(),
        "panicking constructs on a fire-time path:\n  {}\n\nA hook fires on every prompt and \
         every shell command. catch_unwind would catch these, but unwinding still costs the \
         user latency on every keystroke — and under panic = \"abort\" it would not be caught \
         at all. Use a total form (unwrap_or, unwrap_or_default, unwrap_or_else, match, ?) or \
         return a SilenceReason.",
        offences.join("\n  ")
    );
}

#[test]
fn the_scan_can_actually_see_a_violation() {
    // A source scan that matched nothing would pass forever. Prove the matcher
    // works on text shaped like the code it guards.
    let sample = "fn f() { let x: Option<u8> = None; let _ = x.unwrap(); }";
    assert!(
        FORBIDDEN.iter().any(|bad| sample.contains(bad)),
        "the matcher cannot see an unwrap, so the test above proves nothing"
    );
    // And that it does NOT fire on the total forms the code legitimately uses.
    let total = "let name = argv0.rsplit('/').next().unwrap_or(argv0);";
    assert!(
        !FORBIDDEN.iter().any(|bad| total.contains(bad)),
        "unwrap_or is total and must not be flagged, or the rule becomes noise"
    );
}
