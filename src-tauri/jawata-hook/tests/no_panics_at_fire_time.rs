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

/// One line, lexed: comments removed, string and char literal CONTENTS blanked,
/// and the brace delta counted over code only.
///
/// C5 audit round 3 defeated the previous ad-hoc counting four ways, and one of
/// them was live in the tree: `config.rs:171` writes the literal `"{not json"`,
/// whose `{` was counted as a brace, leaving that file's scan permanently one
/// level deep. Nothing was lost only because `mod tests` happened to be its last
/// item. The others: a `'"'` char literal desynchronised the string tracking so
/// a following `http://` was read as a comment; a `{` inside a comment in a
/// skipped region inflated the depth; and an inline `#[cfg(test)] fn f() {}`
/// swallowed a production line.
///
/// Blanking literal contents also removes a false-positive class the old
/// version had: a string that happens to contain `.unwrap()` is not code.
fn lex(line: &str, block_depth: &mut i32) -> (String, i32) {
    let chars: Vec<char> = line.chars().collect();
    let mut code = String::with_capacity(chars.len());
    let mut delta = 0i32;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if *block_depth > 0 {
            // NESTING counted, not a bool (round 4, H2). Rust allows
            // /* a /* b */ c */; clearing on the FIRST */ let a pair of
            // comments whose tails carried { and } skip production between
            // them and still return to depth 0, which the balance check cannot
            // see.
            if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                *block_depth += 1;
                code.push(' ');
                code.push(' ');
                i += 2;
                continue;
            }
            if c == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                *block_depth -= 1;
                code.push(' ');
                code.push(' ');
                i += 2;
                continue;
            }
            code.push(' ');
            i += 1;
            continue;
        }
        // Comments.
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            break;                       // rest of the line is a comment
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            *block_depth += 1;
            code.push(' ');
            code.push(' ');
            i += 2;
            continue;
        }
        // Raw string: r"…" or r#"…"# with any number of hashes.
        if c == 'r' && i + 1 < chars.len() && (chars[i + 1] == '"' || chars[i + 1] == '#') {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < chars.len() && chars[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < chars.len() && chars[j] == '"' {
                code.push(' ');
                j += 1;
                // Scan to the closing quote followed by `hashes` hashes.
                while j < chars.len() {
                    if chars[j] == '"' {
                        let closes = (1..=hashes).all(|k| chars.get(j + k) == Some(&'#'));
                        if closes {
                            j += hashes + 1;
                            break;
                        }
                    }
                    j += 1;
                }
                for _ in i..j.min(chars.len()) {
                    code.push(' ');
                }
                i = j;
                continue;
            }
        }
        // Ordinary string literal.
        if c == '"' {
            code.push(' ');
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    code.push(' ');
                    code.push(' ');
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    code.push(' ');
                    i += 1;
                    break;
                }
                code.push(' ');
                i += 1;
            }
            continue;
        }
        // Char literal — including '"' and '{', which are exactly what
        // desynchronised the old tracker. A lifetime ('a) has no closing quote,
        // so only treat it as a literal when one follows within three chars.
        if c == '\'' {
            let closes_at = (1..=4).find(|k| {
                chars.get(i + k) == Some(&'\'') && !(chars.get(i + k - 1) == Some(&'\\') && *k == 1)
            });
            if let Some(k) = closes_at {
                for _ in 0..=k {
                    code.push(' ');
                }
                i += k + 1;
                continue;
            }
        }
        if c == '{' {
            delta += 1;
        }
        if c == '}' {
            delta -= 1;
        }
        code.push(c);
        i += 1;
    }
    (code, delta)
}

/// True for `#[cfg(test)]` and `#[cfg(all(test, …))]`, and deliberately FALSE
/// for `#[cfg(not(test))]`.
///
/// C5 audit round 4, H1 — the hole that mattered. The check was
/// `contains("test")`, which `#[cfg(not(test))]` also satisfies, so the item was
/// skipped. But `not(test)` marks code that exists ONLY in non-test builds —
/// exactly and exclusively the fire-time path. The control was exempting the one
/// attribute whose entire meaning is "this is what ships", silently: the item
/// skipped cleanly and the balance self-check passed.
///
/// Quoted features are NOT a problem, which was the looseness I expected and
/// guessed wrong about: `lex` blanks string contents before this sees the line,
/// so `#[cfg(feature = "testing")]` has already lost the word. Only an unquoted
/// `test` token survives, which is why stripping `not( … )` groups is the whole
/// fix.
fn is_test_attribute(code: &str) -> bool {
    let t = code.trim_start();
    if !t.starts_with("#[cfg(") {
        return false;
    }
    without_not_groups(t).contains("test")
}

/// Remove every `not( … )` group, balanced, so a `test` token inside one does
/// not count.
fn without_not_groups(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&['n', 'o', 't', '(']) {
            let mut depth = 0;
            let mut j = i + 3;
            while j < chars.len() {
                if chars[j] == '(' {
                    depth += 1;
                } else if chars[j] == ')' {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                j += 1;
            }
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The production lines, plus the end state so the caller can prove the skipper
/// stayed balanced. An unbalanced file means the scan silently stopped looking.
fn production_lines(text: &str) -> (Vec<(usize, String)>, i32, bool) {
    let mut out = Vec::new();
    let mut skip_depth: i32 = 0;
    let mut arming = false;
    let mut block_depth = 0i32;

    for (n, raw) in text.lines().enumerate() {
        let (code, delta) = lex(raw, &mut block_depth);

        // A brace ON THIS LINE distinguishes "the item is still to come" from
        // "the item opened and closed here" — `#[cfg(test)] fn helper() {}` has
        // a delta of ZERO and is complete, which armed the skipper onto the
        // NEXT line and swallowed a production line (round 3, S2).
        let opened_here = code.contains('{');

        if skip_depth == 0 && !arming && is_test_attribute(&code) {
            // The attribute may carry its item on the SAME line.
            if delta > 0 {
                skip_depth = delta;
            } else if !opened_here {
                arming = true;
            }
            continue;
        }
        if arming {
            if delta > 0 {
                arming = false;
                skip_depth = delta;
                continue;
            }
            if opened_here {
                arming = false;      // opened and closed on this line
                continue;
            }
            if code.trim_end().ends_with(';') {
                arming = false;          // a `use` or a const: one line, skipped
            }
            continue;
        }
        if skip_depth > 0 {
            skip_depth += delta;
            if skip_depth < 0 {
                skip_depth = 0;
            }
            continue;
        }
        out.push((n + 1, code));
    }
    (out, skip_depth, arming)
}

/// Every `.rs` under `src`, RECURSIVELY — a submodule directory would
/// otherwise never be scanned.
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
        let (lines, depth, arming) = production_lines(&text);
        // THE SELF-CHECK the audit asked for. A file that ends mid-skip means
        // the scanner lost its place and quietly stopped looking at everything
        // after it — a silent hole in a control, which is the failure class
        // this stage exists to remove. It failed on config.rs before the lexer
        // landed, which is exactly why it is here.
        assert!(
            depth == 0 && !arming,
            "{}: the scan ended mid-skip (depth {depth}, arming {arming}) — every production \
             line after that point went unexamined, silently",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        for (n, code) in lines {
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
        let (lines, _, _) = production_lines(sample);
        let found = lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad)));
        assert!(found, "the scan is blind to: {name}");
    }

    // ---- round 3's four shapes, one of which was LIVE in config.rs ----
    let brace_in_string = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { let _ = \"{not json\"; }\n}\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n";
    let inline_cfg_item = "#[cfg(test)] fn helper() {}\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n";
    let char_literal_quote = "fn f(o: Option<u8>) { let _q = '\"'; let _u = \"http://x\"; let _ = o.unwrap(); }\n";
    let brace_in_comment = "#[cfg(test)]\nmod tests {\n    // a stray { in a comment\n}\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n";
    let cfg_all_test = "#[cfg(all(test, unix))]\nmod tests {\n    #[test]\n    fn t() { panic!(\"fine\"); }\n}\nfn g() {}\n";

    for (name, sample) in [
        ("an unbalanced { inside a test string (was LIVE at config.rs:171)", brace_in_string),
        ("an inline #[cfg(test)] item", inline_cfg_item),
        ("a '\"' char literal ahead of a URL", char_literal_quote),
        ("a { inside a comment in a skipped region", brace_in_comment),
    ] {
        let (lines, depth, arming) = production_lines(sample);
        assert!(depth == 0 && !arming, "the scan ended mid-skip on: {name}");
        let found = lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad)));
        assert!(found, "the scan is blind to: {name}");
    }

    // #[cfg(all(test, …))] used to be scanned AS PRODUCTION — the opposite
    // error, a false failure on a legitimate test module.
    let (lines, _, _) = production_lines(cfg_all_test);
    assert!(
        !lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad))),
        "#[cfg(all(test, …))] must be recognised as test code, not reported as production"
    );

    // ---- round 4's two holes ----
    // H1: not(test) marks code that exists ONLY in non-test builds — exactly
    // the fire-time path. Exempting it inverted the control's purpose, and it
    // was SILENT: the item skipped cleanly and the balance check passed.
    for (name, sample) in [
        ("#[cfg(not(test))] on a function",
         "#[cfg(not(test))]\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n"),
        ("#[cfg(all(not(test), unix))]",
         "#[cfg(all(not(test), unix))]\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n"),
        ("#[cfg(any(unix, not(test)))]",
         "#[cfg(any(unix, not(test)))]\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n"),
        ("#[cfg(not(test))] over a whole module",
         "#[cfg(not(test))]\nmod prod {\n    pub fn f(o: Option<u8>) { let _ = o.unwrap(); }\n}\n"),
    ] {
        let (lines, depth, arming) = production_lines(sample);
        assert!(depth == 0 && !arming, "ended mid-skip on: {name}");
        assert!(
            lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad))),
            "the scan exempts {name} — but not(test) IS the shipping path"
        );
    }

    // H2: compensating NESTED block comments. Rust allows /* a /* b */ c */;
    // clearing on the first */ let one comment's tail carry a { and a later
    // one's carry a }, skipping the production between them and returning to
    // depth 0 where the balance check cannot see it.
    let nested = "/* x /* y */ { */\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n/* p /* q */ } */\n";
    let (lines, depth, arming) = production_lines(nested);
    assert!(depth == 0 && !arming, "ended mid-skip on compensating nested comments");
    assert!(
        lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad))),
        "compensating nested block comments hid production code"
    );

    // Quoted feature names are NOT the looseness — lex blanks string contents
    // before the attribute check sees the line, so the word is already gone.
    for sample in [
        "#[cfg(feature = \"testing\")]\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n",
        "#[cfg(feature = \"test-utils\")]\nfn f(o: Option<u8>) { let _ = o.unwrap(); }\n",
    ] {
        let (lines, _, _) = production_lines(sample);
        assert!(
            lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad))),
            "a quoted feature name must not read as a test attribute"
        );
    }

    // A string that merely CONTAINS a forbidden token is not code.
    let (lines, _, _) = production_lines("fn f() { let _ = \"call .unwrap() here\"; }\n");
    assert!(
        !lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad))),
        "a forbidden token inside a string literal is not a panicking construct"
    );

    // And test code is still exempt — otherwise every assertion trips it.
    let real_test_module = "fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { panic!(\"fine\"); }\n}\n";
    let (lines, _, _) = production_lines(real_test_module);
    let leaked = lines.iter().any(|(_, code)| FORBIDDEN.iter().any(|bad| code.contains(bad)));
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
