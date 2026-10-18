//! ARCH-C22-3 — every lane list written anywhere in the product is a row of the ONE table.
//!
//! `jawata_hook::lanes` is the table of which lanes each occasion asks. The studio and the shell
//! templates it deploys cannot link this crate (`dependency_edges.rs` keeps the crates apart), so
//! they spell their lane lists as literals — and a literal can drift: a seat's recall that asks
//! `["experience", "patterns"]` is a lane choice nobody put in the table, made at one site.
//!
//! The same remedy the other cross-crate contracts use: BOTH SIDES PINNED TO THE SAME LITERAL.
//! This test reads the source text of the studio and of this crate, finds every `"lanes": [...]`
//! literal (also the escaped form inside a quoted shell string), and refuses any list that is not
//! a row of the table.
//!
//! WHAT IT DOES NOT COVER, stated: a list built at run time from a variable is not a literal and
//! is not seen here. This crate builds its own only from the table's constants, which is what the
//! `"lanes": crate::lanes::argument(…)` sites are, and they are skipped because no `[` follows.

use std::path::{Path, PathBuf};

use jawata_hook::lanes::OCCASIONS;

/// Every `"lanes"` key followed by a bracketed literal, as the names inside the brackets.
fn lane_literals(text: &str) -> Vec<Vec<String>> {
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
            .map(|n| n.trim().trim_matches('"').trim_matches('\\').trim_matches('"').to_string())
            .filter(|n| !n.is_empty())
            .collect();
        found.push(names);
    }
    found
}

fn on_the_table(list: &[String]) -> bool {
    OCCASIONS.iter().any(|row| row.len() == list.len() && row.iter().zip(list).all(|(a, b)| a == b))
}

fn sources_under(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources_under(&path, into);
        } else if matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "sh" | "ts" | "js")) {
            into.push(path);
        }
    }
}

/// The studio's source and this crate's, less the table itself (which defines the rows).
fn swept_files() -> Vec<PathBuf> {
    let hook = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources_under(&hook.join("../src"), &mut files);
    sources_under(&hook.join("src"), &mut files);
    files.retain(|f| f.file_name().and_then(|n| n.to_str()) != Some("lanes.rs"));
    files
}

#[test]
fn every_lane_list_written_in_the_product_is_a_row_of_the_table() {
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
    // failure this test exists to prevent. The studio writes seven literals today, in the seat
    // runner and in the deployed hook templates; both files must be seen.
    assert!(seen >= 7, "the sweep saw only {seen} lane lists; it is not reading the sources");
    for file in ["runner.rs", "manager_service.rs"] {
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
    assert!(!on_the_table(&["experience".into(), "patterns".into()]));
    assert!(!on_the_table(&["code".into(), "experience".into()]), "order is part of the row");
}

#[test]
fn the_reader_finds_a_plain_and_an_escaped_literal_and_skips_a_non_literal() {
    let text = r#"a: "lanes": ["experience", "domain"], b: \"lanes\":[\"transcript\"], c: "lanes": argument(x)"#;
    let got = lane_literals(text);
    assert_eq!(vec![vec!["experience", "domain"], vec!["transcript"]], got);
}
