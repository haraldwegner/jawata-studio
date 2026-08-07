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

/// Strip test code by BRACE COUNTING from each `#[cfg(test)]` item, not by
/// truncating at the first occurrence.
///
/// C5 audit round 2, R3 defeated the truncating version with three shapes: a
/// `#[cfg(test)] use …` on line 2 blanked the whole file; a production `fn`
/// placed after `mod tests` was never seen; and `//` inside a string literal
/// (this crate is full of `http://` URLs) truncated the line before the code.
fn production_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut skip_depth: i32 = 0;
    let mut arming = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        if skip_depth == 0 && line.starts_with("#[cfg(test)]") {
            // The next item is test code; find its extent by braces.
            arming = true;
            continue;
        }
        if arming {
            let opens = raw.matches('{').count() as i32;
            let closes = raw.matches('}').count() as i32;
            if opens > 0 {
                arming = false;
                skip_depth = opens - closes;
                if skip_depth <= 0 {
                    skip_depth = 0;   // a one-line item
                }
                continue;
            }
            if line.ends_with(';') {
                arming = false;      // a `use` or a const: one line, skipped
                continue;
            }
            continue;                // still on the item's signature
        }
        if skip_depth > 0 {
            skip_depth += raw.matches('{').count() as i32;
            skip_depth -= raw.matches('}').count() as i32;
            continue;
        }
        out.push((n + 1, strip_comment(raw)));
    }
    out
}

/// Drop a trailing `//` comment — but only when the `//` is OUTSIDE a string
/// literal. `let u = "http://x";` must keep its code.
fn strip_comment(line: &str) -> String {
    let bytes: Vec<char> = line.chars().collect();
    let mut in_string = false;
    let mut escaped = false;
    for i in 0..bytes.len() {
        let c = bytes[i];
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '/' if !in_string && i + 1 < bytes.len() && bytes[i + 1] == '/' => {
                return bytes[..i].iter().collect();
            }
            _ => {}
        }
    }
    line.to_string()
}

/// Every `.rs` under `src`, RECURSIVELY — a submodule directory would
/// otherwise never be scanned (R3's fourth shape).
fn rust_sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_panicking_construct_is_reachable_when_the_hook_fires() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(!files.is_empty(), "the scan found no sources — it would pass forever");

    let mut offences = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("readable source");
        for (n, code) in production_lines(&text) {
            for bad in FORBIDDEN {
                if code.contains(bad) {
                    offences.push(format!(
                        "{}:{}: {bad} — {}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        n,
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
fn the_scan_survives_the_shapes_that_defeated_its_first_version() {
    // Each of these was planted by the C5 audit and found NOTHING. They are
    // kept as the control: a scan with a silent hole is the failure class this
    // stage exists to remove.
    let early_cfg_test = "use std::fmt;\n#[cfg(test)]\nuse std::fmt::Debug;\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n";
    let url_in_string = "fn f(o: Option<u8>) { let _d = \"http://127.0.0.1:8800/mcp\"; let _ = o.unwrap(); }\n";
    let after_mod_tests = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n";

    for (name, sample) in [
        ("a #[cfg(test)] use before production code", early_cfg_test),
        ("a // inside a string literal", url_in_string),
        ("production code after mod tests", after_mod_tests),
    ] {
        let found = production_lines(sample)
            .iter()
            .any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad)));
        assert!(found, "the scan is blind to: {name}");
    }

    // And test code is still exempt — otherwise every assertion trips it.
    let real_test_module = "fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { panic!(\"fine\"); }\n}\n";
    let leaked = production_lines(real_test_module)
        .iter()
        .any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad)));
    assert!(!leaked, "test code must stay exempt, or the rule becomes noise");
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
