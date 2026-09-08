//! studio#10 — no Unix-only API may be reached on a Windows build, TESTS
//! INCLUDED.
//!
//! The production crate has always compiled on Windows; the app ships there.
//! The TEST target did not. Sprint 28a added a "Deploy paths resolve correctly
//! on this platform" step to the release workflow with no platform condition,
//! which made the v3.9.0 release run the first thing ever to build the studio
//! crate's lib tests on Windows. It failed to compile — two errors, before a
//! single assertion ran — and took the release with it:
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
//! property of the source — so the source is what this pins. CI, on a real
//! Windows runner, proves the whole target; this stops the same defect arriving
//! there.
//!
//! WHAT IT CANNOT SEE, so a green is not over-read: a Windows compile error
//! that is not one of ours — a dependency that stops supporting the target, a
//! `std` API that is Unix-only without being spelled `os::unix`. It pins the
//! defect class that broke a release, not "this compiles on Windows".
//!
//! AND IT SAYS WHEN IT CANNOT READ A FILE. Reading Rust with a brace counter
//! is reading Rust with a wrong parser, and the first version was: it counted
//! braces inside string literals, so the 22 embedded shell scripts and raw
//! fixtures in `manager_service.rs` left it 9 levels deep at end of file and
//! every one of the 20 Unix-only sites there was judged at a depth that was not
//! the real one. A drifting depth holds a gate open past its block, which is a
//! silent FALSE NEGATIVE — the guard's own defect class, in the file holding
//! two thirds of the population. So the scan skips literal and comment content
//! properly, and every file must return to depth zero: one that does not is
//! REPORTED AS UNREADABLE rather than reported as clean.

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
        // `all(unix, …)` is a gate as much as `unix` is — narrower, never
        // wider. Omitting it made a correctly gated call an over-report, which
        // is the direction that costs someone an afternoon proving the guard
        // wrong.
        || (l.starts_with("#[cfg(all(") && l.contains("unix"))
        || (l.starts_with("#[cfg(any(") && l.contains("unix"))
}

/// A line between a gate and the thing it gates, which does NOT consume it.
///
/// The first version had no such rule and reported four sites that were
/// correctly gated all along — every one the crate's commonest idiom, a gate
/// stacked above `#[test]`. A guard whose opening move is four false positives
/// is not a strict guard; it is one nobody can keep green.
fn carries_the_gate_forward(line: &str) -> bool {
    let l = line.trim();
    l.is_empty() || l.starts_with('#') || l.starts_with("//")
}

/// Each source line with its COMMENT and LITERAL content removed, so a brace
/// inside a shell script embedded as a Rust string is not counted as structure.
///
/// This is the part the first version did not have, and the measurement that
/// forced it: `manager_service.rs` embeds hook scripts and raw-string fixtures,
/// and counting their braces left the scan 9 levels deep at end of file.
///
/// Handles what this codebase actually contains: line and (nestable) block
/// comments, strings with escapes, raw strings with any number of hashes, byte
/// strings, and char literals — while NOT mistaking a lifetime (`&'a str`) for
/// one, which is the classic way a hand-rolled Rust scanner goes wrong.
fn code_only(source: &str) -> Vec<String> {
    #[derive(PartialEq, Clone, Copy)]
    enum In {
        Code,
        LineComment,
        BlockComment(usize),
        Str,
        RawStr(usize),
        Char,
    }
    let chars: Vec<char> = source.chars().collect();
    let mut state = In::Code;
    // Characters consumed by a token already recognised (an opening `//`, a
    // raw-string prefix, an escape). They are not code and are not re-read.
    //
    // A COUNTER RATHER THAN A JUMP, and that is not a style choice: the first
    // version advanced the index past them, and any such jump that stepped over
    // a newline dropped a line — so the code view and the source view drifted
    // apart and every finding after the first embedded script named the wrong
    // line. Stepping one character at a time makes a line boundary impossible
    // to miss.
    let mut skip = 0usize;
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();

    for i in 0..chars.len() {
        let c = chars[i];
        if c == '\n' {
            out.push(std::mem::take(&mut line));
            if state == In::LineComment {
                state = In::Code;
            }
            // An escape or a token never spans a line break in valid Rust, and
            // if one somehow did, carrying the skip across would swallow code.
            skip = 0;
            continue;
        }
        if skip > 0 {
            skip -= 1;
            continue;
        }
        match state {
            In::LineComment => {}
            In::BlockComment(depth) => {
                if c == '*' && chars.get(i + 1) == Some(&'/') {
                    state = if depth == 1 { In::Code } else { In::BlockComment(depth - 1) };
                    skip = 1;
                } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                    state = In::BlockComment(depth + 1);
                    skip = 1;
                }
            }
            In::Str => {
                if c == '\\' {
                    skip = 1;
                } else if c == '"' {
                    state = In::Code;
                }
            }
            In::RawStr(hashes) => {
                if c == '"' && (1..=hashes).all(|n| chars.get(i + n) == Some(&'#')) {
                    state = In::Code;
                    skip = hashes;
                }
            }
            In::Char => {
                if c == '\\' {
                    skip = 1;
                } else if c == '\'' {
                    state = In::Code;
                }
            }
            In::Code => {
                if c == '/' && chars.get(i + 1) == Some(&'/') {
                    state = In::LineComment;
                    skip = 1;
                    continue;
                }
                if c == '/' && chars.get(i + 1) == Some(&'*') {
                    state = In::BlockComment(1);
                    skip = 1;
                    continue;
                }
                if c == '"' {
                    state = In::Str;
                    continue;
                }
                // `r"…"`, `r#"…"#`, `b"…"`, `br#"…"#` — count the hashes.
                if c == 'r' || c == 'b' {
                    let mut j = i + 1;
                    if c == 'b' && chars.get(j) == Some(&'r') {
                        j += 1;
                    }
                    let first_hash = j;
                    while chars.get(j) == Some(&'#') {
                        j += 1;
                    }
                    if chars.get(j) == Some(&'"') {
                        let hashes = j - first_hash;
                        state = if hashes == 0 { In::Str } else { In::RawStr(hashes) };
                        skip = j - i;
                        continue;
                    }
                }
                if c == '\'' {
                    // A LIFETIME, not a char literal: `'a` followed by anything
                    // that is not a closing quote. Reading `&'a` as a char
                    // literal swallows the rest of the file from that point.
                    let next = chars.get(i + 1).copied().unwrap_or(' ');
                    let after = chars.get(i + 2).copied().unwrap_or(' ');
                    let is_lifetime = (next.is_alphabetic() || next == '_') && after != '\'';
                    if !is_lifetime {
                        state = In::Char;
                        continue;
                    }
                }
                line.push(c);
            }
        }
    }
    out.push(line);
    out
}

/// Every `.rs` file the two crates compile — RECURSIVELY, and both of them.
///
/// The first version walked one crate's `src` and `tests` non-recursively. The
/// sibling `jawata-hook` crate ships `jawata-hook.exe` and its CI test step runs
/// on all five targets, so it has exactly the same exposure and was outside the
/// guard entirely.
fn sources() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack = vec![root.join("src"), root.join("tests"), root.join("jawata-hook")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) != Some("target") {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    assert!(
        out.len() > 40,
        "found {} source files — the walk is broken, and a walk that finds \
         nothing passes this test having looked at nothing",
        out.len()
    );
    out
}

/// What one file's scan concluded.
#[derive(Debug, Default)]
struct Scan {
    /// Unix-only calls with no gate over them.
    ungated: Vec<(usize, String)>,
    /// The brace depth left at end of file. Anything but zero means the scan
    /// lost track, and its `ungated` list is then worth nothing.
    residual_depth: i32,
}

/// Gates are brace-counted rather than looked up in a line window: a window is
/// wrong in both directions — it misses a gate on an enclosing function and it
/// credits an unrelated one that happened to be nearby.
fn scan(source: &str) -> Scan {
    let code = code_only(source);
    let raw: Vec<&str> = source.lines().collect();
    let mut out = Scan::default();
    let mut depth: i32 = 0;
    let mut gates: Vec<i32> = Vec::new();
    let mut pending = false;

    for (index, line) in code.iter().enumerate() {
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;
        let raw_line = raw.get(index).copied().unwrap_or("");

        if opens_a_gate(raw_line) {
            pending = true;
            depth += opens - closes;
            continue;
        }

        // The TOKEN is looked for in code only, so a doc comment naming
        // `os::unix` and a test fixture holding one as a string are not
        // findings — which is also why this file needs no exemption of its own.
        let gated = !gates.is_empty() || pending;
        if !gated && UNIX_ONLY.iter().any(|token| line.contains(token)) {
            out.ungated.push((index + 1, raw_line.trim().to_string()));
        }

        if pending && !carries_the_gate_forward(raw_line) {
            if opens > closes {
                gates.push(depth);
                pending = false;
            } else if !line.trim().is_empty() {
                pending = false;
            }
        }
        depth += opens - closes;
        while gates.last().is_some_and(|open_at| depth <= *open_at) {
            gates.pop();
        }
    }
    out.residual_depth = depth;
    out
}

#[test]
fn no_unix_only_call_is_reachable_from_a_windows_build() {
    let mut findings: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for path in sources() {
        let source = fs::read_to_string(&path).expect("read source");
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let result = scan(&source);
        if result.residual_depth != 0 {
            unreadable.push(format!("{name}: ends at brace depth {}", result.residual_depth));
        }
        for (line, text) in result.ungated {
            findings.push(format!("{name}:{line}: {text}"));
        }
    }
    // THE SELF-CHECK FIRST. A scan that lost track of depth reports a clean
    // file for the same reason it reports a dirty one — by accident — so an
    // unreadable file is a failure in its own right rather than a caveat on a
    // green.
    assert!(
        unreadable.is_empty(),
        "the scan could not follow these files, so its verdict about them means \
         nothing — a depth that drifts UP holds a gate open past its block, which \
         is a silent false negative:\n  {}",
        unreadable.join("\n  ")
    );
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
        scan(ungated).ungated,
        "an ungated call must be reported"
    );

    // A gate on the enclosing function covers everything inside it, which a
    // look-back window would get wrong the moment the body grew.
    let gated_fn = "#[cfg(unix)]\nfn f() {\n    let mut p = q();\n    p.set_mode(0o755);\n}\n";
    assert!(scan(gated_fn).ungated.is_empty(), "a gated fn is clean");

    // A gate on a bare block, which is the form most of this crate uses.
    let gated_block =
        "fn f() {\n    #[cfg(unix)]\n    {\n        use std::os::unix::fs::PermissionsExt;\n    }\n}\n";
    assert!(scan(gated_block).ungated.is_empty(), "a gated block is clean");

    // AND THE GATE MUST END. A call after the gated block closes is exposed
    // again — the failure a depth-blind scanner cannot see, because it would
    // still be counting the gate as open.
    let after =
        "fn f() {\n    #[cfg(unix)]\n    {\n        let _ = 1;\n    }\n    use std::os::unix::fs::symlink;\n}\n";
    assert_eq!(1, scan(after).ungated.len(), "the gate must close with its block");

    // The gate above OTHER ATTRIBUTES, which is how every gated test in this
    // crate is written, and a doc comment between them.
    let stacked = "#[cfg(unix)]\n#[test]\nfn f() {\n    use std::os::unix::fs::symlink;\n}\n";
    assert!(scan(stacked).ungated.is_empty(), "a gate above #[test] still covers the fn");
    let documented =
        "#[cfg(unix)]\n/// why\n#[test]\nfn f() {\n    use std::os::unix::fs::symlink;\n}\n";
    assert!(scan(documented).ungated.is_empty(), "a comment does not spend the gate");

    // `all(unix, …)` narrows a gate; it does not stop being one.
    let narrowed =
        "#[cfg(all(unix, feature = \"x\"))]\nfn f() {\n    use std::os::unix::fs::symlink;\n}\n";
    assert!(scan(narrowed).ungated.is_empty(), "all(unix, …) is a gate");

    // `cfg!(unix)` is a RUNTIME boolean; the body is still compiled, so it is
    // not a gate and must not be credited as one.
    let runtime = "fn f() {\n    if cfg!(unix) {\n        use std::os::unix::fs::symlink;\n    }\n}\n";
    assert_eq!(1, scan(runtime).ungated.len(), "cfg! is not a gate");
}

#[test]
fn the_scan_does_not_count_braces_inside_literals_or_comments() {
    // THE DEFECT THE FIRST VERSION HAD, in its smallest form. This file's own
    // subject embeds shell scripts as Rust strings; counting their braces left
    // the scan nine levels deep by end of file and judged every site in it at a
    // depth that was not the real one.
    let script = "fn f() {\n    let s = \"if [ x ]; then { echo 1; } fi\";\n}\n";
    assert_eq!(0, scan(script).residual_depth, "a brace in a string is not structure");

    let raw = "fn f() {\n    let s = r#\"{{{ unbalanced \"#;\n}\n";
    assert_eq!(0, scan(raw).residual_depth, "a raw string is skipped whole");

    let commented = "fn f() {\n    // }}}\n    /* { nested /* { */ */\n}\n";
    assert_eq!(0, scan(commented).residual_depth, "comments carry no structure");

    let chars = "fn f() {\n    let c = '{';\n    let d = '\\'';\n}\n";
    assert_eq!(0, scan(chars).residual_depth, "a char literal is not structure");

    // AND A LIFETIME IS NOT A CHAR LITERAL. Reading `&'a` as one swallows the
    // rest of the file from that point, which turns every later finding into a
    // silent miss.
    let lifetime = "fn f<'a>(x: &'a str) -> &'a str {\n    x\n}\n";
    assert_eq!(0, scan(lifetime).residual_depth, "a lifetime is not a char literal");

    // The token search reads code only, so this file's own token list and
    // fixtures are not findings — which is why no file needs excluding by name.
    let quoted = "fn f() {\n    let s = \"use std::os::unix::fs::symlink;\";\n    // os::unix\n}\n";
    assert!(scan(quoted).ungated.is_empty(), "a token inside a literal is not a call");
}
