//! What a hand edit of a `.java` file CHANGES, read from the text of the edit alone (Sprint 28g S0, D1).
//!
//! The blunt gate in [`crate::editgate`] asks only "is this a Java file". This module asks the
//! question the deliverable is about: does the edit perform a refactoring a jawata tool already
//! performs, and if so which tool and which call?
//!
//! # What this can and cannot see
//!
//! It reads `old_string` and `new_string` and nothing else — no AST, no other file, no earlier
//! edit. That is a hard limit and it is stated here rather than discovered in review: the shapes
//! below are the ones whose DIFF is recognisable by itself. A call site that gained an argument
//! because a signature changed elsewhere, a statement block moved between two methods across two
//! edits, a new interface assembled from methods a class already had — each is textually
//! indistinguishable from authoring, and a classifier that refused them would refuse authoring.
//! So it does not. The replay against the Sprint 8 census (`tests`, gated on
//! `JAWATA_CENSUS_EDITS`) measures how much that costs, and the number is in the plan, not here.
//!
//! # Precision over recall, on purpose
//!
//! A shape is claimed only when the whole diff is explained by it. Every rule therefore starts from
//! "the token streams, with comments removed, agree except for X" and refuses nothing it cannot
//! fully account for. A false refusal blocks real work in every session that deploys this; a miss
//! costs one hand edit that a tool would have done.

/// One recognised refactoring: what it is, which tool does it, and the call that does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub name: &'static str,
    pub tool: &'static str,
    pub call: &'static str,
}

const IMPORT: Shape = Shape {
    name: "an import added, removed or qualified by hand",
    tool: "organize_imports",
    call: "organize_imports(filePath=…)",
};
const VISIBILITY: Shape = Shape {
    name: "a visibility change",
    tool: "change_method_signature",
    call: "change_method_signature(visibility=…, symbol=pkg.Type#member)",
};
const RENAME: Shape = Shape {
    name: "a rename",
    tool: "rename_symbol",
    call: "rename_symbol(symbol=…, newName=…)",
};
const SIGNATURE: Shape = Shape {
    name: "a parameter-list change on a declaration",
    tool: "change_method_signature",
    call: "change_method_signature(kind=change_signature, symbol=pkg.Type#method, newParameters=[…])",
};
const RECORD_COMPONENT: Shape = Shape {
    name: "a record component added",
    tool: "data",
    call: "data(kind=add_record_component, symbol=pkg.Record, componentName=…, componentType=…)",
};
const DELETE_MEMBER: Shape = Shape {
    name: "a private member deleted",
    tool: "apply_cleanup",
    call: "apply_cleanup(kind=remove_dead_code, symbol=pkg.Type#member)",
};
const EXTRACT_METHOD: Shape = Shape {
    name: "a range extracted into a new method",
    tool: "extract",
    call: "extract(kind=method, filePath=…, startLine=…, endLine=…, methodName=…)",
};

/// The shape an edit performs, or `None` for everything this cannot account for — which is the
/// answer for all authoring.
///
/// `old` is empty for a `Write` (a whole file), and a whole file is never a refactoring of itself.
pub fn classify(old: &str, new: &str) -> Option<Shape> {
    if old.trim().is_empty() || old == new {
        return None;
    }
    let (ot, nt) = (tokens(old), tokens(new));
    if let Some(s) = import_shape(old, new, &ot, &nt) {
        return Some(s);
    }
    if let Some(s) = visibility_shape(&ot, &nt) {
        return Some(s);
    }
    if let Some(s) = rename_shape(&ot, &nt) {
        return Some(s);
    }
    if let Some(s) = signature_shape(&ot, &nt) {
        return Some(s);
    }
    if let Some(s) = delete_member_shape(&ot, &nt) {
        return Some(s);
    }
    extract_method_shape(old, new)
}

// ---- tokens ---------------------------------------------------------------------------------

/// Java tokens with comments and whitespace removed. A string or char literal is ONE token, so a
/// change inside a literal is a change of that token and never reads as a rename.
fn tokens(src: &str) -> Vec<String> {
    // An edit often begins in the middle of a doc comment; everything up to its closing `*/` is
    // comment, or its words would read as code and every such edit would look like a rewrite.
    let src = match (src.find("*/"), src.find("/*")) {
        (Some(close), open) if open.map_or(true, |o| close < o) => &src[close + 2..],
        _ => src,
    };
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1).copied() == Some('/')) {
                i += 1;
            }
            i += 2;
        } else if c == '"' || c == '\'' {
            let quote = c;
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            out.push(chars[start..i.min(chars.len())].iter().collect());
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else {
            out.push(c.to_string());
            i += 1;
        }
    }
    out
}

fn is_ident(t: &str) -> bool {
    t.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && !KEYWORDS.contains(&t)
}

const KEYWORDS: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const",
    "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float",
    "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native",
    "new", "package", "private", "protected", "public", "return", "short", "static", "strictfp",
    "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void",
    "volatile", "while", "true", "false", "null",
];

// ---- imports --------------------------------------------------------------------------------

fn import_shape(old: &str, new: &str, ot: &[String], nt: &[String]) -> Option<Shape> {
    // Import lines added or removed, everything else equal.
    let strip = |s: &str| -> Vec<String> {
        tokens(
            &s.lines()
                .filter(|l| !l.trim_start().starts_with("import "))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    };
    let imports = |s: &str| s.lines().filter(|l| l.trim_start().starts_with("import ")).count();
    if (imports(old) > 0 || imports(new) > 0) && strip(old) == strip(new) {
        return Some(IMPORT);
    }
    // A simple type name replaced by its fully qualified form, and nothing else changed.
    if qualification_only(ot, nt) {
        return Some(IMPORT);
    }
    None
}

/// `new` is `old` with some identifier `T` rewritten to `pkg.sub.T` (lower-case package segments).
fn qualification_only(old: &[String], new: &[String]) -> bool {
    let (mut i, mut j, mut changed) = (0, 0, false);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            i += 1;
            j += 1;
            continue;
        }
        // new[j..] must read  seg . seg . … . old[i]  with lower-case segments.
        let mut k = j;
        let mut segments = 0;
        while let (Some(seg), Some(dot)) = (new.get(k), new.get(k + 1)) {
            let lower = seg.chars().next().is_some_and(char::is_lowercase) && is_ident(seg);
            if lower && dot == "." {
                segments += 1;
                k += 2;
            } else {
                break;
            }
        }
        if segments > 0 && new.get(k) == Some(&old[i]) {
            changed = true;
            i += 1;
            j = k + 1;
        } else {
            return false;
        }
    }
    changed && i == old.len() && j == new.len()
}

// ---- visibility, rename ---------------------------------------------------------------------

fn visibility_shape(ot: &[String], nt: &[String]) -> Option<Shape> {
    const V: [&str; 3] = ["public", "protected", "private"];
    let strip = |t: &[String]| -> Vec<String> { t.iter().filter(|x| !V.contains(&x.as_str())).cloned().collect() };
    (ot != nt && strip(ot) == strip(nt)).then_some(VISIBILITY)
}

fn rename_shape(ot: &[String], nt: &[String]) -> Option<Shape> {
    if ot.len() != nt.len() {
        return None;
    }
    let mut pair: Option<(&String, &String)> = None;
    for (a, b) in ot.iter().zip(nt) {
        if a == b {
            continue;
        }
        if !is_ident(a) || !is_ident(b) {
            return None;
        }
        match pair {
            None => pair = Some((a, b)),
            Some((x, y)) if x == a && y == b => {}
            Some(_) => return None,
        }
    }
    pair.map(|_| RENAME)
}

// ---- signatures -----------------------------------------------------------------------------

/// The streams agree except INSIDE one parenthesis group that is the parameter list of a
/// declaration — `Type name ( … ) {` — or the header of a record.
fn signature_shape(ot: &[String], nt: &[String]) -> Option<Shape> {
    let mut p = 0;
    while p < ot.len() && p < nt.len() && ot[p] == nt[p] {
        p += 1;
    }
    let mut s = 0;
    while s < ot.len() - p.min(ot.len()) && s < nt.len() - p.min(nt.len()) && ot[ot.len() - 1 - s] == nt[nt.len() - 1 - s] {
        s += 1;
    }
    if p == ot.len() && p == nt.len() {
        return None;
    }
    // Unmatched '(' before the first difference, in the shared prefix.
    let mut depth = 0i32;
    let mut open = None;
    for (idx, t) in ot.iter().take(p).enumerate().rev() {
        match t.as_str() {
            ")" => depth += 1,
            "(" if depth == 0 => {
                open = Some(idx);
                break;
            }
            "(" => depth -= 1,
            _ => {}
        }
    }
    let open = open?;
    // The group must close in the shared suffix, in both streams, before anything else differs.
    let (o_end, n_end) = (ot.len() - s, nt.len() - s);
    let (o_tail, n_tail) = (&ot[o_end..], &nt[n_end..]);
    let close = group_close(o_tail)?;
    if group_close(n_tail) != Some(close) {
        return None;
    }
    // The differing region itself must stay inside the group: no unbalanced parens in it.
    let balanced = |slice: &[String]| {
        let mut d = 0i32;
        for t in slice {
            match t.as_str() {
                "(" => d += 1,
                ")" => d -= 1,
                _ => {}
            }
            if d < 0 {
                return false;
            }
        }
        d == 0
    };
    if !balanced(&ot[p..o_end]) || !balanced(&nt[p..n_end]) {
        return None;
    }
    // Before the '(' : `name`, preceded by a type-ish token — or `record Name`.
    let name = ot.get(open.checked_sub(1)?)?;
    if !is_ident(name) {
        return None;
    }
    let before = open.checked_sub(2).and_then(|i| ot.get(i));
    let record = before.is_some_and(|b| b == "record");
    let typeish = before.is_some_and(|b| {
        b == ">" || b == "]" || ["void", "int", "long", "double", "float", "boolean", "char", "byte", "short"].contains(&b.as_str())
            || (is_ident(b) && !["return", "new", "throw", "else", "case", "assert", "yield"].contains(&b.as_str()))
    });
    if !record && !typeish {
        return None;
    }
    // After the ')' : a declaration continues with `{`, `throws` or `;` (a call continues with `;`
    // too, but a call is not preceded by a type — checked above).
    let after = o_tail.get(close + 1);
    let decl_follow = after.is_some_and(|t| t == "{" || t == "throws" || t == ";");
    if !record && !decl_follow {
        return None;
    }
    // A header that gained or lost whole parameters, or retyped them — not an empty-to-empty edit.
    if ot[p..o_end] == nt[p..n_end] {
        return None;
    }
    Some(if record { RECORD_COMPONENT } else { SIGNATURE })
}

/// Index of the ')' that closes a group already opened before `tail` (depth starts at 1).
fn group_close(tail: &[String]) -> Option<usize> {
    let mut depth = 1i32;
    for (i, t) in tail.iter().enumerate() {
        match t.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// ---- deleted member -------------------------------------------------------------------------

/// Nothing was added in code; what was removed is one or more complete `private` members.
///
/// The removed block can sit anywhere, and a shared `private` keyword at its edge makes a plain
/// common-prefix/suffix cut land one token off, so every start position is tried.
fn delete_member_shape(ot: &[String], nt: &[String]) -> Option<Shape> {
    if nt.len() >= ot.len() {
        return None;
    }
    let removed_len = ot.len() - nt.len();
    for start in 0..=nt.len() {
        if ot[..start] == nt[..start] && ot[start + removed_len..] == nt[start..]
            && private_members_only(&ot[start..start + removed_len])
        {
            return Some(DELETE_MEMBER);
        }
    }
    None
}

/// The tokens are one or more COMPLETE `private` members: each starts at `private` and ends at a
/// `;` at depth zero or at the brace that closes its body.
fn private_members_only(removed: &[String]) -> bool {
    let mut i = 0;
    while i < removed.len() {
        if removed[i] != "private" {
            return false;
        }
        let (mut brace, mut paren) = (0i32, 0i32);
        let mut ended = false;
        while i < removed.len() {
            match removed[i].as_str() {
                "{" => brace += 1,
                "}" => {
                    brace -= 1;
                    if brace == 0 && paren == 0 {
                        i += 1;
                        ended = true;
                        break;
                    }
                }
                "(" => paren += 1,
                ")" => paren -= 1,
                ";" if brace == 0 && paren == 0 => {
                    i += 1;
                    ended = true;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        if !ended {
            return false;
        }
    }
    !removed.is_empty()
}

// ---- extract method -------------------------------------------------------------------------

/// The method stays where it was and calls a NEW method, and a contiguous run of at least three
/// significant lines of the old body now sits under that new declaration.
///
/// Both halves are needed. "Some lines recur" alone refused a test rewritten in the same idiom
/// (the replay found six such authoring edits), because rewriting code re-uses its lines. An
/// extraction keeps the original declaration line above the new one, and carries a CONTIGUOUS
/// range, not a scatter.
fn extract_method_shape(old: &str, new: &str) -> Option<Shape> {
    let old_lines: Vec<&str> = old.lines().map(str::trim).collect();
    let new_lines: Vec<&str> = new.lines().map(str::trim).collect();
    // The original declaration: the first line of the old text that looks like one.
    let original_decl = old_lines.iter().copied().find(|l| looks_like_method_decl(l))?;
    let original_at = new_lines.iter().position(|l| *l == original_decl)?;
    let decl_at = new_lines
        .iter()
        .enumerate()
        .position(|(i, l)| i > original_at && looks_like_method_decl(l) && !old_lines.contains(l))?;
    let below = &new_lines[decl_at + 1..];
    let significant = |l: &str| {
        l.len() >= 12 && !l.starts_with("//") && !l.starts_with('*') && !l.starts_with("/*") && l != "}"
    };
    // Longest contiguous run of old lines found, in order, under the new declaration.
    let mut best = 0;
    let mut best_run: &[&str] = &[];
    for start in 0..old_lines.len() {
        for offset in 0..below.len() {
            let mut k = 0;
            let mut sig = 0;
            while start + k < old_lines.len() && offset + k < below.len() && old_lines[start + k] == below[offset + k] {
                if significant(old_lines[start + k]) {
                    sig += 1;
                }
                k += 1;
            }
            if sig > best {
                best = sig;
                best_run = &old_lines[start..start + k];
            }
        }
    }
    // MOVED, not copied: the run must be gone from where the old method had it. A sibling that
    // repeats the original's lines leaves the original whole, and that is duplication by hand.
    let above = &new_lines[..decl_at];
    let still_above = !best_run.is_empty() && above.windows(best_run.len()).any(|w| w == best_run);
    (best >= 3 && !still_above).then_some(EXTRACT_METHOD)
}

fn looks_like_method_decl(t: &str) -> bool {
    if !t.ends_with('{') || !t.contains('(') || t.starts_with("//") || t.starts_with('*') {
        return false;
    }
    let head = t.split('(').next().unwrap_or("");
    let words: Vec<&str> = head.split_whitespace().collect();
    if words.len() < 2 {
        return false;
    }
    let first = words.first().copied().unwrap_or("");
    if ["if", "for", "while", "switch", "catch", "return", "else", "new", "try", "synchronized", "do"].contains(&first) {
        return false;
    }
    words.last().is_some_and(|w| is_ident(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(old: &str, new: &str) -> Option<&'static str> {
        classify(old, new).map(|s| s.tool)
    }

    #[test]
    fn imports_added_removed_or_qualified_by_hand_name_organize_imports() {
        assert_eq!(tool("import a.B;\nimport a.C;\n", "import a.B;\n"), Some("organize_imports"));
        assert_eq!(tool("import a.B;\n", "import a.B;\nimport a.D;\n"), Some("organize_imports"));
        assert_eq!(
            tool("void f(Contract c) {}", "void f(com.jats2.model.Contract c) {}"),
            Some("organize_imports"),
            "a simple name rewritten to its qualified form is the import the author skipped"
        );
        // The control: a qualified name that REPLACES a different name is not that.
        assert_eq!(tool("x = Contract.of(1);", "x = com.x.Other.of(1);"), None);
    }

    #[test]
    fn a_visibility_change_names_the_signature_tool_and_a_body_change_is_authoring() {
        assert_eq!(
            tool("    private long exitDecisionTs() {", "    public long exitDecisionTs() {"),
            Some("change_method_signature")
        );
        // Visibility AND a body change together is authoring.
        assert_eq!(
            tool("private int f() { return 1; }", "public int f() { return 2; }"),
            None
        );
    }

    #[test]
    fn a_pure_identifier_substitution_is_a_rename_and_two_different_ones_are_not() {
        assert_eq!(
            tool("void routeFillToAlgo_delegates() {", "void slotRouteFill_delegates() {"),
            Some("rename_symbol")
        );
        // Every occurrence renamed consistently is still one rename.
        assert_eq!(tool("int a = 1; return a;", "int count = 1; return count;"), Some("rename_symbol"));
        // Two different substitutions are not.
        assert_eq!(tool("int a = b;", "int c = d;"), None);
        // A change inside a string literal is a change of that literal, never a rename.
        assert_eq!(tool("log(\"x\");", "log(\"y\");"), None);
    }

    #[test]
    fn a_parameter_list_change_on_a_declaration_is_a_signature_change_and_on_a_call_is_not() {
        assert_eq!(
            tool("private void installCallbacks() {", "private void installCallbacks(long flattenAtEpochMs) {"),
            Some("change_method_signature")
        );
        // A call that gained an argument is indistinguishable from authoring.
        assert_eq!(tool("        installCallbacks();\n", "        installCallbacks(flattenAtEpochMs);\n"), None);
        assert_eq!(
            tool(
                "public record OrderCapture(String symbol, long qty) {}",
                "public record OrderCapture(String symbol, long qty,\n Thread placedOn) {}"
            ),
            Some("data")
        );
    }

    #[test]
    fn deleting_a_private_member_names_remove_dead_code_and_deleting_a_public_one_does_not() {
        assert_eq!(
            tool("    private long a = 1;\n    private SlotCallback callback;\n\n", "    private long a = 1;\n\n"),
            Some("apply_cleanup")
        );
        assert_eq!(
            tool("    private long a = 1;\n    public SlotCallback callback;\n\n", "    private long a = 1;\n\n"),
            None
        );
    }

    #[test]
    fn new_code_and_a_new_file_pass_untouched() {
        assert_eq!(tool("", "package x;\npublic class A {}\n"), None);
        assert_eq!(tool("    void a() {}\n", "    void a() {}\n\n    void b() { work(); }\n"), None);
    }

    /// THE REPLAY (D1's measure): the Sprint 8 census, edit by edit, through the classifier.
    ///
    /// Gated on `JAWATA_CENSUS_EDITS` (the 312 edit calls as JSON, extracted from the session
    /// transcript) and `JAWATA_CENSUS_LABELS` (the census appendix, one row per `#`). The edit text
    /// is another project's source, so it is read from a path and never committed; a run without
    /// the variables says NOT RUN rather than passing. The hard assertion is the one that protects
    /// real work: NO authoring or comment edit may be refused. The refusal count of the 47 is
    /// reported, not asserted — it is the measure, and the plan records the number.
    #[test]
    fn the_sprint_8_census_replayed() {
        let (Ok(edits), Ok(labels)) = (std::env::var("JAWATA_CENSUS_EDITS"), std::env::var("JAWATA_CENSUS_LABELS")) else {
            eprintln!("census replay: NOT RUN — JAWATA_CENSUS_EDITS / JAWATA_CENSUS_LABELS unset");
            return;
        };
        let edits: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(edits).expect("edits")).expect("json");
        let labels: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(labels).expect("labels")).expect("json");
        let (mut refused, mut passed): (Vec<(u64, String, String)>, Vec<(u64, String, String)>) = (vec![], vec![]);
        let mut by_status: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
        let mut false_refusals = vec![];
        let mut missed = vec![];
        for e in edits.as_array().expect("array") {
            let n = e["n"].as_u64().expect("n");
            let Some(l) = labels.get(n.to_string()) else { continue };
            let status = l["status"].as_str().unwrap_or("?").to_string();
            if status == "NEVER-LANDED" || status == "EXCLUDED" {
                continue;
            }
            let (old, new) = (e["old"].as_str().unwrap_or(""), e["new"].as_str().unwrap_or(""));
            let verdict = classify(old, new);
            let entry = by_status.entry(status.clone()).or_default();
            match verdict {
                Some(shape) => {
                    entry.0 += 1;
                    refused.push((n, status.clone(), shape.tool.to_string()));
                    if status == "AUTHORING" || status == "COMMENT" {
                        false_refusals.push(format!("#{n} {} -> {}", l["shape"], shape.tool));
                    }
                }
                None => {
                    entry.1 += 1;
                    passed.push((n, status.clone(), String::new()));
                    if status == "EXISTS" {
                        missed.push(format!("#{n} {} / {}", l["shape"].as_str().unwrap_or(""), l["jt"].as_str().unwrap_or("")));
                    }
                }
            }
        }
        // D1's measure, through the gate and not only the classifier: each refused tool-bearing
        // edit is refused BY NAME, and with the check switched off none is.
        let mut named = 0;
        let mut let_through_when_off = 0;
        for (n, status, tool) in &refused {
            if status != "EXISTS" {
                continue;
            }
            let e = &edits.as_array().expect("array")[(*n - 1) as usize];
            let shape = classify(e["old"].as_str().unwrap_or(""), e["new"].as_str().unwrap_or("")).expect("refused above");
            match crate::editgate::judge_shaped(&shape, "X.java", None, None, true) {
                crate::editgate::EditVerdict::Denied(r) if r.contains(tool.as_str()) => named += 1,
                other => panic!("#{n} was classified {tool} but the gate said {other:?}"),
            }
            if matches!(crate::editgate::judge_shaped(&shape, "X.java", None, None, false), crate::editgate::EditVerdict::Allowed(_)) {
                let_through_when_off += 1;
            }
        }
        let report = format!(
            "refused BY NAME among the tool-bearing edits: {named}; let through with the check off: {let_through_when_off}\ncensus replay: per status (refused, passed) = {by_status:?}\nFALSE REFUSALS (authoring/comment refused): {}\n{}\nMISSED (tool-bearing, passed): {}\n{}\n",
            false_refusals.len(),
            false_refusals.join("\n"),
            missed.len(),
            missed.join("\n"),
        );
        if let Ok(path) = std::env::var("JAWATA_CENSUS_REPORT") {
            let _ = std::fs::write(path, &report);
        }
        eprintln!("{report}");
        assert!(false_refusals.is_empty(), "authoring or comment edits were refused:\n{report}");
        // The recall floor measured on 2026-10-11: a rule that quietly stops matching lowers it.
        assert!(named >= 24, "fewer tool-bearing edits are refused by name than the 24 measured:\n{report}");
        assert_eq!(named, let_through_when_off, "the control: switched off, every one of them passes");
    }

    /// THE HELD-OUT LOOK: edits the rules were never tuned on, listed for a human to judge. There
    /// are no labels, so this asserts nothing — it is how derive-on-A/validate-on-B is done by
    /// reading. Gated on `JAWATA_HELDOUT_EDITS`; writes `JAWATA_HELDOUT_REPORT`.
    #[test]
    fn edits_outside_the_census_listed_for_review() {
        let Ok(path) = std::env::var("JAWATA_HELDOUT_EDITS") else {
            eprintln!("held-out look: NOT RUN — JAWATA_HELDOUT_EDITS unset");
            return;
        };
        let edits: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("edits")).expect("json");
        let mut out = String::new();
        let (mut total, mut refused) = (0, 0);
        let mut by_tool: std::collections::BTreeMap<&str, u32> = Default::default();
        for e in edits.as_array().expect("array") {
            total += 1;
            let (old, new) = (e["old"].as_str().unwrap_or(""), e["new"].as_str().unwrap_or(""));
            if let Some(shape) = classify(old, new) {
                refused += 1;
                *by_tool.entry(shape.tool).or_default() += 1;
                let cut = |s: &str| s.chars().take(160).collect::<String>().replace('\n', "⏎");
                out.push_str(&format!("#{} {} [{}]\n  OLD {}\n  NEW {}\n", e["n"], shape.name, e["file"].as_str().unwrap_or("").rsplit('/').next().unwrap_or(""), cut(old), cut(new)));
            }
        }
        let head = format!("held-out: {refused} of {total} refused; by tool {by_tool:?}\n");
        if let Ok(p) = std::env::var("JAWATA_HELDOUT_REPORT") {
            let _ = std::fs::write(p, format!("{head}{out}"));
        }
        eprintln!("{head}");
    }

    #[test]
    fn a_sibling_method_that_repeats_the_originals_lines_is_authoring_not_an_extraction() {
        let old = "    void a(Event e) {\n        Event ev = new Event();\n        ev.type = e.type;\n        ev.when = e.when;\n        dispatch(ev);\n    }\n";
        let new = format!("{old}\n    void b(Event e) {{\n        Event ev = new Event();\n        ev.type = e.type;\n        ev.when = e.when;\n        dispatch(ev);\n    }}\n");
        assert_eq!(None, classify(old, &new), "the original stays whole, so nothing was moved");
    }

    #[test]
    fn extracting_a_range_into_a_new_method_is_recognised_by_the_lines_that_moved() {
        let old = "    int f(List<X> xs) {\n        int total = 0;\n        for (X x : xs) {\n            total += x.weightOfTheThing();\n            total += x.otherWeightOfTheThing();\n        }\n        return total;\n    }\n";
        let new = "    int f(List<X> xs) {\n        return sum(xs);\n    }\n\n    private int sum(List<X> xs) {\n        int total = 0;\n        for (X x : xs) {\n            total += x.weightOfTheThing();\n            total += x.otherWeightOfTheThing();\n        }\n        return total;\n    }\n";
        assert_eq!(tool(old, new), Some("extract"));
        // The control: the same lines appearing in BOTH places is duplication by hand, not a move.
        assert_eq!(tool("    int f() {\n        return 1;\n    }\n", "    int f() {\n        return 1;\n    }\n    int g() {\n        return 1;\n    }\n"), None);
    }
}
