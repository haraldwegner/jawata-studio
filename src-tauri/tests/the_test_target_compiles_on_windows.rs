//! studio#10 — no Unix-only API may be reached on a Windows build, TESTS
//! INCLUDED.
//!
//! The production crate has always compiled on Windows; the app ships there.
//! The TEST target did not. Sprint 28a added a "Deploy paths resolve correctly
//! on this platform" step to `release.yml` with no platform condition, which
//! made the v3.9.0 release run the first thing ever to build the studio crate's
//! lib tests on Windows. It failed to compile — two errors, before a single
//! assertion ran — and took the release with it:
//!
//! ```text
//! error[E0433]: cannot find `unix` in `os`   --> src\runner.rs:2415
//! error[E0599]: no method named `set_mode`   --> src\runner.rs:2627
//! ```
//!
//! Both are fixed. What was missing is anything that would say so again: the
//! only instrument that can see this defect is a Windows compiler, the Linux
//! suite is structurally blind to it, and the next `#[cfg(test)]` helper
//! reaching for `set_mode` would be found the same way — by a release run.
//!
//! WHY A SOURCE ASSERTION. The honest instrument is a Windows-targeted check,
//! and it does not run on this machine: `ring`'s build script needs a Windows C
//! toolchain (`lib.exe`) that a Linux host does not have, so the check dies in a
//! DEPENDENCY before it reaches a line of ours. What decides the outcome is
//! whether our own Unix-only calls sit behind `#[cfg(unix)]`, and that is a
//! property of the source — so the source is what this pins, with the reason in
//! the failure message. CI, on a real Windows runner, is what proves the whole
//! target; this is what stops the same defect arriving there.
//!
//! WHAT IT CANNOT SEE, so a green is not over-read: a Windows compile error
//! that is not one of ours — a dependency that stops supporting the target, a
//! `std` API that is Unix-only without being spelled `os::unix`. It pins the
//! defect class that actually broke a release, not "this compiles on Windows".

use std::fs;
use std::path::PathBuf;

/// Spelled so that a use of any of them fails to compile on Windows.
/// `os::unix` catches the imports, which is how every extension trait
/// (`PermissionsExt`, `OpenOptionsExt`, `CommandExt`) and `fs::symlink` arrive;
/// the two constructors catch a call that reached the trait some other way.
const UNIX_ONLY: &[&str] = &["os::unix", "set_mode(", "from_mode("];

/// A gate that makes the line below it absent from a Windows build.
/// `cfg!(unix)` is deliberately NOT one: it is a runtime boolean, the code
/// inside it is still COMPILED, and that is exactly how this defect hides.
fn opens_a_gate(line: &str) -> bool {
    let l = line.trim();
    l.starts_with("#[cfg(unix)]")
        || l.starts_with("#[cfg(not(windows))]")
        || l.starts_with("#[cfg(target_family = \"unix\")]")
        || (l.starts_with("#[cfg(any(") && l.contains("unix"))
}

/// A line between a gate and the thing it gates, which does NOT consume it.
///
/// THE FIRST VERSION OF THIS CHECK HAD NO SUCH RULE, and it reported four
/// sites that were correctly gated all along — every one of them the crate's
/// commonest idiom:
///
/// ```text
/// #[cfg(unix)]
/// #[test]
/// fn the_registry_is_owner_only() {
/// ```
///
/// The `#[test]` line was read as the item the gate attached to, so the gate
/// was spent before the function it was written for. A guard whose first run
/// is four false positives is not a strict guard; it is one nobody can keep
/// green, and the next real finding arrives in a list already known to be wrong.
fn carries_the_gate_forward(line: &str) -> bool {
    let l = line.trim();
    l.is_empty() || l.starts_with('#') || l.starts_with("//")
}

/// Every `.rs` file the `jawata-studio` test target compiles.
fn sources() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [root.join("src"), root.join("tests")] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // This file NAMES the Unix-only spellings as data — the token
            // list, and the fixtures the control drives. It is the one source
            // that can never be green under its own rule, and excluding it by
            // name rather than by a pattern keeps the exemption exactly one
            // file wide.
            let is_this_file = path.file_name().and_then(|n| n.to_str())
                == Some("the_test_target_compiles_on_windows.rs");
            if path.extension().and_then(|e| e.to_str()) == Some("rs") && !is_this_file {
                out.push(path);
            }
        }
    }
    assert!(
        out.len() > 5,
        "found {} source files — the walk is broken, and a walk that finds \
         nothing passes this test having looked at nothing",
        out.len()
    );
    out
}

/// The lines of `source` that are NOT behind a `#[cfg(unix)]`-style gate.
///
/// Brace-counted rather than "look back N lines": a gate attaches to the item
/// or block that follows it and ends with that item's closing brace, and a
/// line-window guess is wrong in both directions — it misses a gate on an
/// enclosing function and it credits an unrelated one that happened to be
/// nearby.
fn ungated_lines(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    // The brace depths at which a gate is currently open.
    let mut gates: Vec<i32> = Vec::new();
    // A gate seen but not yet attached to anything.
    let mut pending = false;

    for (index, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("");
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;

        if opens_a_gate(raw) {
            pending = true;
            depth += opens - closes;
            continue;
        }

        let gated = !gates.is_empty() || pending;
        if !gated && UNIX_ONLY.iter().any(|token| raw.contains(token)) {
            out.push((index + 1, raw.trim().to_string()));
        }

        if pending && !carries_the_gate_forward(raw) {
            if opens > closes {
                // The gate attached to a block or item: it holds until this
                // brace closes.
                gates.push(depth);
                pending = false;
            } else if !line.trim().is_empty() {
                // A one-line item (a `use`, a statement): the gate is spent.
                pending = false;
            }
        }
        depth += opens - closes;
        while gates.last().is_some_and(|open_at| depth <= *open_at) {
            gates.pop();
        }
    }
    out
}

#[test]
fn no_unix_only_call_is_reachable_from_a_windows_build() {
    let mut findings: Vec<String> = Vec::new();
    for path in sources() {
        let source = fs::read_to_string(&path).expect("read source");
        for (line, text) in ungated_lines(&source) {
            findings.push(format!(
                "{}:{line}: {text}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ));
        }
    }
    assert!(
        findings.is_empty(),
        "these reach a Unix-only API with nothing keeping them out of a Windows \
         build, so the studio crate's tests will not COMPILE there — which is \
         how v3.9.0's release run died (studio#10). Put the call behind \
         `#[cfg(unix)]`, or give it a portable form:\n  {}",
        findings.join("\n  ")
    );
}

#[test]
fn the_check_can_actually_fail() {
    // The control. Without it, a walk that found no files, a token list that
    // matched nothing, or a gate rule that swallowed everything would all read
    // as a clean codebase.
    let ungated = "fn f() {\n    use std::os::unix::fs::PermissionsExt;\n}\n";
    assert_eq!(
        vec![(2, "use std::os::unix::fs::PermissionsExt;".to_string())],
        ungated_lines(ungated),
        "an ungated call must be reported"
    );

    // A gate on the enclosing function covers everything inside it, which a
    // look-back window would get wrong the moment the body grew.
    let gated_fn = "#[cfg(unix)]\nfn f() {\n    let mut p = q();\n    p.set_mode(0o755);\n}\n";
    assert!(ungated_lines(gated_fn).is_empty(), "a gated fn is clean");

    // A gate on a bare block, which is the form most of this crate uses.
    let gated_block =
        "fn f() {\n    #[cfg(unix)]\n    {\n        use std::os::unix::fs::PermissionsExt;\n    }\n}\n";
    assert!(ungated_lines(gated_block).is_empty(), "a gated block is clean");

    // AND THE GATE MUST END. A call after the gated block closes is exposed
    // again — the failure a depth-blind scanner cannot see, because it would
    // still be counting the gate as open.
    let after =
        "fn f() {\n    #[cfg(unix)]\n    {\n        let _ = 1;\n    }\n    use std::os::unix::fs::symlink;\n}\n";
    assert_eq!(1, ungated_lines(after).len(), "the gate must close with its block");

    // THE CASE THE FIRST VERSION GOT WRONG, and the reason this control exists
    // at all: the gate sits above OTHER ATTRIBUTES, which is how every gated
    // test in this crate is written. Reading `#[test]` as the gated item spends
    // the gate one line early and reports the function below it.
    let stacked = "#[cfg(unix)]\n#[test]\nfn f() {\n    use std::os::unix::fs::symlink;\n}\n";
    assert!(
        ungated_lines(stacked).is_empty(),
        "a gate above #[test] must still cover the function"
    );

    // A doc comment between them, which is just as common.
    let documented =
        "#[cfg(unix)]\n/// why\n#[test]\nfn f() {\n    use std::os::unix::fs::symlink;\n}\n";
    assert!(ungated_lines(documented).is_empty(), "a comment does not spend the gate");

    // `cfg!(unix)` is a RUNTIME boolean; the body is still compiled, so it is
    // not a gate and must not be credited as one.
    let runtime = "fn f() {\n    if cfg!(unix) {\n        use std::os::unix::fs::symlink;\n    }\n}\n";
    assert_eq!(1, ungated_lines(runtime).len(), "cfg! is not a gate");
}
