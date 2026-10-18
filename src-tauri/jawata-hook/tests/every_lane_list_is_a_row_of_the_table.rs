//! ARCH-C22-3 — every lane list the studio writes down is a row of the ONE table.
//!
//! `jawata_hook::lanes` is the table of which lanes each occasion asks. The studio, the shell
//! templates it deploys and the seat and skill texts it ships cannot link this crate
//! (`dependency_edges.rs` keeps the crates apart), so they spell their lane lists as literals — and
//! a literal can drift: a seat that asks `["experience", "patterns"]` is a lane choice nobody put in
//! the table, made at one site.
//!
//! The same remedy the other cross-crate contracts use: BOTH SIDES PINNED TO THE SAME LITERAL.
//! This test reads text and refuses any list that is not a row of the table.
//!
//! WHAT IT READS: `src-tauri/src/**` and this crate's `src/**` (`.rs .sh .ts .js`), and the
//! studio's `seats/**` and `skills/**` (`.md`). WHAT IT RECOGNISES: the JSON key `"lanes": [..]`
//! (also with escaped quotes inside a quoted shell string), the prose argument `lanes=[a, b]`, and
//! the single-lane prose argument `lane=a` / `lane="a"`.
//!
//! WHAT IT DOES NOT SEE, stated: a list built at run time from a variable or a constant; a list in
//! a file type or directory above (docs, the sprint documents, JSON seat configs); a lane named in
//! a sentence rather than in an argument. This crate builds its own lists only from the table's
//! constants, which is what the `"lanes": crate::lanes::argument(…)` sites are, and they are
//! skipped because no `[` follows. The engine's own lists are held by a sibling sweep in the engine
//! (EveryLaneScopeTheEngineBuildsIsDeclaredTest), and the two tables are mirrors by hand until the
//! engine owns the table (ARCH-C22-3's open note).

use std::path::{Path, PathBuf};

use jawata_hook::lanes::OCCASIONS;

/// Every lane list a text spells, in any of the forms the product uses.
fn lane_literals(text: &str) -> Vec<Vec<String>> {
    let mut found = json_lane_lists(text);
    found.extend(prose_lane_lists(text));
    found
}

/// `lanes=[a, b]` and `lane=a` as seat and skill texts write them for the agent to type.
fn prose_lane_lists(text: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices("lane") {
        let rest = &text[at + "lane".len()..];
        if let Some(rest) = rest.strip_prefix("s=[") {
            if let Some(close) = rest.find(']') {
                found.push(
                    rest[..close].split(',').map(|n| n.trim().trim_matches('"').to_string())
                        .filter(|n| !n.is_empty()).collect(),
                );
            }
        } else if let Some(rest) = rest.strip_prefix('=') {
            let rest = rest.trim_start_matches('"');
            let name: String = rest.chars().take_while(|c| c.is_ascii_lowercase()).collect();
            if !name.is_empty() {
                found.push(vec![name]);
            }
        }
    }
    found
}

/// Every `"lanes"` key followed by a bracketed literal, as the names inside the brackets.
fn json_lane_lists(text: &str) -> Vec<Vec<String>> {
    // An escaped quote inside a quoted string reads as the quote it stands for.
    let text = text.replace("\\\"", "\"");
    let text = text.as_str();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("\"lanes\"") {
        let key_end = from + at + "\"lanes\"".len();
        from = key_end;
        let rest = text[key_end..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('[') else { continue };
        let Some(close) = rest.find(']') else { continue };
        let names = rest[..close]
            .split(',')
            .map(|n| n.trim().trim_matches('"').to_string())
            .filter(|n| !n.is_empty())
            .collect();
        found.push(names);
    }
    found
}

fn on_the_table(list: &[String]) -> bool {
    OCCASIONS.iter().any(|row| row.len() == list.len() && row.iter().zip(list).all(|(a, b)| a == b))
}

fn files_under(dir: &Path, extensions: &[&str], into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, extensions, into);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.contains(&e)) {
            into.push(path);
        }
    }
}

/// The studio's source, this crate's, and the seat and skill texts — less the table itself, which
/// defines the rows.
fn swept_files() -> Vec<PathBuf> {
    let hook = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for code in ["../src", "src"] {
        files_under(&hook.join(code), &["rs", "sh", "ts", "js"], &mut files);
    }
    for text in ["../../seats", "../../skills"] {
        files_under(&hook.join(text), &["md"], &mut files);
    }
    files.retain(|f| f.file_name().and_then(|n| n.to_str()) != Some("lanes.rs"));
    files
}

#[test]
fn every_lane_list_the_studio_writes_down_is_a_row_of_the_table() {
    let mut off_table = Vec::new();
    let mut seen = 0;
    let mut files_with_lists = std::collections::BTreeSet::new();
    for file in swept_files() {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for list in lane_literals(&text) {
            seen += 1;
            files_with_lists.insert(file.file_name().unwrap().to_string_lossy().to_string());
            if !on_the_table(&list) {
                off_table.push(format!("{} asks {:?}", file.display(), list));
            }
        }
    }
    // PROOF OF LIFE. A sweep that finds nothing passes whatever the sources say, which is the
    // failure this test exists to prevent. The seat runner and the deployed hook templates write
    // seven JSON literals, and the seat and skill texts write more; every kind must be seen.
    assert!(seen >= 12, "the sweep saw only {seen} lane lists; it is not reading the sources");
    for file in ["runner.rs", "manager_service.rs", "main.md", "architect.md", "brainstorm.md"] {
        assert!(files_with_lists.contains(file), "no lane list read from {file}: {files_with_lists:?}");
    }
    assert!(
        off_table.is_empty(),
        "lane lists that are not a row of jawata_hook::lanes — add the occasion to the table, or \
         use a row that exists:\n{}",
        off_table.join("\n")
    );
}

#[test]
fn the_matcher_refuses_a_list_the_table_does_not_hold() {
    // The discriminator: without it the test above could pass for a matcher that accepts anything.
    assert!(on_the_table(&["experience".into(), "code".into()]));
    assert!(on_the_table(&["experience".into(), "rules".into()]), "a typed message's row (D45)");
    assert!(!on_the_table(&["rules".into()]), "the rules lane alone is no occasion");
    assert!(!on_the_table(&["rules".into(), "experience".into()]), "order is part of the row");
    assert!(!on_the_table(&["code".into(), "experience".into()]), "order is part of the row");
}

#[test]
fn the_reader_finds_every_form_and_skips_a_non_literal() {
    let json = r#"a: "lanes": ["experience", "domain"], b: \"lanes\":[\"transcript\"], c: "lanes": argument(x)"#;
    assert_eq!(vec![vec!["experience", "domain"], vec!["transcript"]], lane_literals(json));

    let prose = "ask `nominate, lanes=[experience, patterns], question=...` or `recall, lane=code` or `lane=\"domain\"`, never `lane=<one>`";
    assert_eq!(
        vec![vec!["experience", "patterns"], vec!["code"], vec!["domain"]],
        lane_literals(prose)
    );
}
