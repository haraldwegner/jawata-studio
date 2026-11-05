//! Comment-preserving edits of a JSON-with-comments settings file (Sprint 28h, S15, item j).
//!
//! The Zed and the VS Code settings files may hold `//` and `/* */` comments and trailing commas.
//! A rewrite through a JSON value tree would drop them, so the studio edits the TEXT instead: it
//! reads the file into a tree that remembers where every value, key and comma sits, and returns a
//! list of edits that touch only the studio's own entry. Every byte outside those edits (comments,
//! order, whitespace, the other entries) stays as it was.
//!
//! No JSONC crate is used: the grammar needed is small (strings, comments, objects, arrays, numbers
//! and the three words), and the reader below knows that a `//` inside a string is not a comment and
//! that a brace inside a comment is not structure. A file that does not read is refused with the
//! line and column, never guessed at.

use serde::Serialize;
use serde_json::{Map, Value};

const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub key: String,
    pub key_span: Span,
    pub value: Node,
    /// Where the comma that follows this member sits, when there is one (the last member may carry one too).
    pub comma: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ObjectNode {
    pub open: usize,
    pub close: usize,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Object(ObjectNode),
    Array(Vec<Node>, Span),
    Scalar(Value, Span),
}

impl Node {
    pub fn span(&self) -> Span {
        match self {
            Node::Object(object) => Span { start: object.open, end: object.close + 1 },
            Node::Array(_, span) | Node::Scalar(_, span) => *span,
        }
    }

    /// The value this text means, with every comment and trailing comma ignored.
    pub fn to_value(&self) -> Value {
        match self {
            Node::Object(object) => {
                let mut map = Map::new();
                for member in &object.members {
                    map.insert(member.key.clone(), member.value.to_value());
                }
                Value::Object(map)
            }
            Node::Array(items, _) => Value::Array(items.iter().map(Node::to_value).collect()),
            Node::Scalar(value, _) => value.clone(),
        }
    }
}

impl ObjectNode {
    /// The member under `key`; with a repeated key the last one, as a JSON reader takes it.
    pub fn member(&self, key: &str) -> Option<&Member> {
        self.member_index(key).map(|index| &self.members[index])
    }

    pub fn member_index(&self, key: &str) -> Option<usize> {
        self.members.iter().rposition(|member| member.key == key)
    }
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn bytes(&self) -> &'a [u8] {
        self.text.as_bytes()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes().get(self.pos).copied()
    }

    fn fail<T>(&self, at: usize, what: &str) -> Result<T, String> {
        let (line, column) = line_column(self.text, at);
        Err(format!("{what} at line {line} column {column}"))
    }

    /// Whitespace and comments: neither is structure.
    fn skip_trivia(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'/') => match self.bytes().get(self.pos + 1) {
                    Some(b'/') => {
                        while let Some(byte) = self.peek() {
                            if byte == b'\n' {
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                    Some(b'*') => {
                        let start = self.pos;
                        match self.text[self.pos + 2..].find("*/") {
                            Some(offset) => self.pos += 2 + offset + 2,
                            None => return self.fail(start, "a comment is never closed"),
                        }
                    }
                    _ => return self.fail(self.pos, "unexpected `/`"),
                },
                _ => return Ok(()),
            }
        }
    }

    fn parse_string(&mut self) -> Result<(String, Span), String> {
        let start = self.pos;
        let bytes = self.bytes();
        let mut index = start + 1;
        loop {
            match bytes.get(index) {
                None => return self.fail(start, "a string is never closed"),
                Some(b'\\') => index += 2,
                Some(b'"') => break,
                Some(byte) if *byte < 0x20 => {
                    return self.fail(index, "a string holds a raw control character")
                }
                Some(_) => index += 1,
            }
        }
        let end = index + 1;
        match serde_json::from_str::<String>(&self.text[start..end]) {
            Ok(value) => {
                self.pos = end;
                Ok((value, Span { start, end }))
            }
            Err(_) => self.fail(start, "a string holds an escape JSON does not have"),
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<Node, String> {
        if depth > MAX_DEPTH {
            return self.fail(self.pos, "values are nested too deeply");
        }
        match self.peek() {
            None => self.fail(self.pos, "a value is missing"),
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => {
                let (value, span) = self.parse_string()?;
                Ok(Node::Scalar(Value::String(value), span))
            }
            Some(byte) if byte == b'-' || byte.is_ascii_alphanumeric() => {
                let start = self.pos;
                while matches!(self.peek(), Some(b) if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'+' | b'.')) {
                    self.pos += 1;
                }
                let word = &self.text[start..self.pos];
                match serde_json::from_str::<Value>(word) {
                    Ok(value @ (Value::Null | Value::Bool(_) | Value::Number(_))) => {
                        Ok(Node::Scalar(value, Span { start, end: self.pos }))
                    }
                    _ => self.fail(start, &format!("`{word}` is not a JSON value")),
                }
            }
            Some(_) => {
                let found = self.text[self.pos..].chars().next().unwrap_or('?');
                self.fail(self.pos, &format!("unexpected `{found}`"))
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Node, String> {
        let open = self.pos;
        self.pos += 1;
        let mut members = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                Some(b'}') => {
                    let close = self.pos;
                    self.pos += 1;
                    return Ok(Node::Object(ObjectNode { open, close, members }));
                }
                Some(b'"') => {}
                None => return self.fail(open, "an object is never closed"),
                Some(_) => return self.fail(self.pos, "expected a string key or `}`"),
            }
            let (key, key_span) = self.parse_string()?;
            self.skip_trivia()?;
            if self.peek() != Some(b':') {
                return self.fail(self.pos, "expected `:` after a key");
            }
            self.pos += 1;
            self.skip_trivia()?;
            let value = self.parse_value(depth + 1)?;
            self.skip_trivia()?;
            let comma = match self.peek() {
                Some(b',') => {
                    let at = self.pos;
                    self.pos += 1;
                    Some(at)
                }
                Some(b'}') => None,
                None => return self.fail(open, "an object is never closed"),
                Some(_) => return self.fail(self.pos, "expected `,` or `}`"),
            };
            members.push(Member { key, key_span, value, comma });
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Node, String> {
        let open = self.pos;
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                Some(b']') => {
                    let end = self.pos + 1;
                    self.pos = end;
                    return Ok(Node::Array(items, Span { start: open, end }));
                }
                None => return self.fail(open, "an array is never closed"),
                Some(_) => {}
            }
            items.push(self.parse_value(depth + 1)?);
            self.skip_trivia()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {}
                None => return self.fail(open, "an array is never closed"),
                Some(_) => return self.fail(self.pos, "expected `,` or `]`"),
            }
        }
    }
}

fn line_column(text: &str, at: usize) -> (usize, usize) {
    let before = text.get(..at).unwrap_or(text);
    let line = 1 + before.matches('\n').count();
    let column = 1 + before.rsplit('\n').next().map_or(0, |last| last.chars().count());
    (line, column)
}

/// Reads a file of JSON with comments and trailing commas into a tree that remembers where
/// everything sits. The error names what was found and where.
pub fn parse(text: &str) -> Result<Node, String> {
    let mut parser = Parser { text, pos: if text.starts_with('\u{feff}') { 3 } else { 0 } };
    parser.skip_trivia()?;
    let node = parser.parse_value(0)?;
    parser.skip_trivia()?;
    if parser.pos < text.len() {
        return parser.fail(parser.pos, "unexpected content after the value");
    }
    Ok(node)
}

/// The top-level object of a settings file.
pub fn parse_root_object(text: &str) -> Result<ObjectNode, String> {
    match parse(text)? {
        Node::Object(object) => Ok(object),
        _ => Err("the file is not a JSON object".into()),
    }
}

/// True when both texts read as JSON and mean the same value, whatever their comments or layout.
pub fn same_json(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Ok(a), Ok(b)) => a.to_value() == b.to_value(),
        _ => false,
    }
}

/// One change to the text: the bytes `start..end` become `text` (an insertion has `start == end`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Edit {
    fn insert(at: usize, text: String) -> Edit {
        Edit { start: at, end: at, text }
    }

    fn replace(span: Span, text: String) -> Edit {
        Edit { start: span.start, end: span.end, text }
    }

    fn delete(start: usize, end: usize) -> Edit {
        Edit { start, end, text: String::new() }
    }
}

/// Applies the edits to the text. Edits that overlap are refused rather than merged.
pub fn apply(text: &str, mut edits: Vec<Edit>) -> Result<String, String> {
    edits.sort_by_key(|edit| (edit.start, edit.end));
    let mut out = String::with_capacity(text.len() + 256);
    let mut cursor = 0;
    for edit in &edits {
        if edit.start < cursor || edit.end < edit.start || edit.end > text.len() {
            return Err("two edits of the file overlap; nothing is written".into());
        }
        out.push_str(&text[cursor..edit.start]);
        out.push_str(&edit.text);
        cursor = edit.end;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

/// How the file is laid out, so an inserted entry looks like its neighbours.
pub struct Style {
    unit: String,
    newline: &'static str,
}

pub fn style_of(text: &str, root: &ObjectNode) -> Style {
    let unit = root
        .members
        .iter()
        .find_map(|member| {
            let line_begin = only_whitespace_before(text, member.key_span.start)?;
            let indent = &text[line_begin..member.key_span.start];
            (!indent.is_empty()).then(|| indent.to_string())
        })
        .unwrap_or_else(|| "  ".to_string());
    Style { unit, newline: if text.contains("\r\n") { "\r\n" } else { "\n" } }
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |index| index + 1)
}

/// The start of the line when only blanks stand between it and `pos`.
fn only_whitespace_before(text: &str, pos: usize) -> Option<usize> {
    let start = line_start(text, pos);
    text[start..pos].bytes().all(|b| b == b' ' || b == b'\t').then_some(start)
}

pub fn line_indent(text: &str, pos: usize) -> &str {
    let rest = &text[line_start(text, pos)..];
    let blanks = rest.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
    &rest[..blanks]
}

fn skip_blanks(text: &str, from: usize) -> usize {
    text.len() - text[from..].trim_start_matches([' ', '\t']).len()
}

/// The end of a comment that sits on the same line after `from`, or `from` when there is none.
fn after_trailing_comment(text: &str, from: usize) -> usize {
    let mut good = from;
    let mut pos = from;
    loop {
        pos = skip_blanks(text, pos);
        let rest = &text[pos..];
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(text.len(), |index| pos + index);
            return if end > pos && text.as_bytes()[end - 1] == b'\r' { end - 1 } else { end };
        }
        if rest.starts_with("/*") {
            match rest[2..].find("*/") {
                Some(offset) if !rest[..2 + offset].contains('\n') => {
                    pos += 2 + offset + 2;
                    good = pos;
                    continue;
                }
                _ => return good,
            }
        }
        return good;
    }
}

fn render(value: &Value, style: &Style, indent: &str) -> String {
    let mut buffer = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(style.unit.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    let text = match value.serialize(&mut serializer) {
        Ok(()) => String::from_utf8(buffer).unwrap_or_else(|_| value.to_string()),
        Err(_) => value.to_string(),
    };
    text.replace('\n', &format!("{}{}", style.newline, indent))
}

fn key_json(key: &str) -> String {
    serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""))
}

/// Puts `entries` into `object` after its last member, in the layout of the members already there.
pub fn insert_members(text: &str, object: &ObjectNode, entries: &[(&str, Value)], style: &Style) -> Vec<Edit> {
    if entries.is_empty() {
        return Vec::new();
    }
    let blank_inside = text[object.open + 1..object.close].trim().is_empty();
    let inline = !object.members.is_empty() && !text[object.open..=object.close].contains('\n');
    if inline {
        let block = entries
            .iter()
            .map(|(key, value)| format!("{}: {}", key_json(key), value))
            .collect::<Vec<_>>()
            .join(", ");
        return match object.members.last() {
            None => unreachable!("inline is only chosen for an object that has members"),
            Some(last) => match last.comma {
                Some(comma) => vec![Edit::insert(comma + 1, format!(" {block},"))],
                None => vec![Edit::insert(last.value.span().end, format!(", {block}"))],
            },
        };
    }
    let base = line_indent(text, object.open);
    let member_indent = object
        .members
        .first()
        .and_then(|first| only_whitespace_before(text, first.key_span.start).map(|start| &text[start..first.key_span.start]))
        .map_or_else(|| format!("{base}{}", style.unit), str::to_string);
    let separator = format!(",{}{}", style.newline, member_indent);
    let block = format!(
        "{}{}{}",
        style.newline,
        member_indent,
        entries
            .iter()
            .map(|(key, value)| format!("{}: {}", key_json(key), render(value, style, &member_indent)))
            .collect::<Vec<_>>()
            .join(&separator)
    );
    match object.members.last() {
        None if blank_inside => vec![Edit {
            start: object.open + 1,
            end: object.close,
            text: format!("{block}{}{base}", style.newline),
        }],
        None => vec![Edit::insert(object.open + 1, block)],
        Some(last) => match last.comma {
            Some(comma) => vec![Edit::insert(after_trailing_comment(text, comma + 1), format!("{block},"))],
            None => {
                let value_end = last.value.span().end;
                vec![
                    Edit::insert(value_end, ",".to_string()),
                    Edit::insert(after_trailing_comment(text, value_end), block),
                ]
            }
        },
    }
}

/// Takes the members at `indices` out of `object`, each with its own comma and, when it stood on
/// its own line, that line. A comment next to a member stays where it is.
pub fn remove_members(text: &str, object: &ObjectNode, indices: &[usize]) -> Vec<Edit> {
    let mut edits = Vec::new();
    for &index in indices {
        let member = &object.members[index];
        let mut start = member.key_span.start;
        let mut end = member.comma.map_or(member.value.span().end, |comma| comma + 1);
        match only_whitespace_before(text, start) {
            Some(line_begin) => {
                start = line_begin;
                let after = skip_blanks(text, end);
                if text[after..].starts_with("\r\n") {
                    end = after + 2;
                } else if text[after..].starts_with('\n') {
                    end = after + 1;
                }
            }
            None => {
                if member.comma.is_some() {
                    end = skip_blanks(text, end);
                }
            }
        }
        edits.push(Edit::delete(start, end));
    }
    // A last member has no comma of its own, so the comma of the member before it goes with it.
    if let Some(last) = object.members.len().checked_sub(1) {
        if indices.contains(&last) && object.members[last].comma.is_none() {
            if let Some(survivor) = (0..last).rev().find(|index| !indices.contains(index)) {
                if let Some(comma) = object.members[survivor].comma {
                    edits.push(Edit::delete(comma, comma + 1));
                }
            }
        }
    }
    edits
}

/// A member of another name takes this member's place: its key and its whole value are written anew.
pub fn recycle_member(text: &str, member: &Member, key: &str, value: &Value, style: &Style) -> Vec<Edit> {
    vec![
        Edit::replace(member.key_span, key_json(key)),
        Edit::replace(member.value.span(), render(value, style, line_indent(text, member.key_span.start))),
    ]
}

/// The edits that make the text at `node` mean `desired`, changing only what differs: a comment
/// between the members of an entry, or beside a value that does not change, stays. `indent` is the
/// indentation of the line the value's key stands on.
pub fn sync_value(text: &str, node: &Node, desired: &Value, style: &Style, indent: &str) -> Vec<Edit> {
    match (node, desired) {
        (Node::Object(object), Value::Object(want)) => {
            let missing: Vec<(&str, Value)> = want
                .iter()
                .filter(|(key, _)| object.member(key).is_none())
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect();
            let extra: Vec<usize> = (0..object.members.len())
                .filter(|&index| {
                    let key = &object.members[index].key;
                    !(want.contains_key(key) && object.member_index(key) == Some(index))
                })
                .collect();
            if !missing.is_empty() && !extra.is_empty() {
                return vec![Edit::replace(node.span(), render(desired, style, indent))];
            }
            let mut edits = Vec::new();
            for (key, value) in want {
                if let Some(member) = object.member(key) {
                    let child_indent = line_indent(text, member.key_span.start);
                    edits.extend(sync_value(text, &member.value, value, style, child_indent));
                }
            }
            edits.extend(insert_members(text, object, &missing, style));
            edits.extend(remove_members(text, object, &extra));
            edits
        }
        (Node::Array(items, _), Value::Array(want)) if items.len() == want.len() => items
            .iter()
            .zip(want)
            .flat_map(|(item, value)| sync_value(text, item, value, style, line_indent(text, item.span().start)))
            .collect(),
        (Node::Scalar(have, span), want) if !want.is_object() && !want.is_array() => {
            if have == want {
                Vec::new()
            } else {
                vec![Edit::replace(*span, want.to_string())]
            }
        }
        _ => {
            if node.to_value() == *desired {
                Vec::new()
            } else {
                vec![Edit::replace(node.span(), render(desired, style, indent))]
            }
        }
    }
}

/// The edits that make `object` hold `key` = `desired`: the entry is synced when it is there and
/// inserted when it is not.
pub fn set_member(text: &str, object: &ObjectNode, key: &str, desired: &Value, style: &Style) -> Vec<Edit> {
    match object.member(key) {
        Some(member) => {
            sync_value(text, &member.value, desired, style, line_indent(text, member.key_span.start))
        }
        None => insert_members(text, object, &[(key, desired.clone())], style),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn comments_and_trailing_commas_read_as_the_json_they_mean() {
        let text = "\u{feff}// head\n{\n  \"a\": [1, 2, /* c */ 3,], // t\n  \"b\": { \"c\": null, },\n}\n// tail";
        assert_eq!(parse(text).unwrap().to_value(), json!({"a": [1, 2, 3], "b": {"c": null}}));
    }

    #[test]
    fn the_spans_point_at_the_keys_values_and_commas() {
        let text = "{ \"a\": \"x//y\", \"b\": 12 }";
        let root = parse_root_object(text).unwrap();
        let a = root.member("a").unwrap();
        assert_eq!(&text[a.key_span.start..a.key_span.end], "\"a\"");
        assert_eq!(&text[a.value.span().start..a.value.span().end], "\"x//y\"");
        assert_eq!(a.comma.map(|c| &text[c..c + 1]), Some(","));
        let b = root.member("b").unwrap();
        assert_eq!(&text[b.value.span().start..b.value.span().end], "12");
        assert_eq!(b.comma, None);
    }

    #[test]
    fn what_is_not_json_is_named_with_its_place() {
        for (text, needle) in [
            ("{\n  \"a\": tru }", "line 2 column 8"),
            ("{ \"a\": 01x }", "not a JSON value"),
            ("{ \"a\": \"\\q\" }", "escape"),
            ("{ \"a\": \"x\ny\" }", "raw control"),
            ("", "a value is missing"),
            ("// only a comment", "a value is missing"),
        ] {
            let error = parse(text).expect_err(text);
            assert!(error.contains(needle), "{text:?}: {error}");
        }
        assert!(parse(&format!("{}1{}", "[".repeat(100), "]".repeat(100))).is_err(), "nesting is bounded");
    }

    #[test]
    fn edits_that_overlap_are_refused() {
        let overlapping = vec![Edit::delete(0, 3), Edit::delete(2, 4)];
        assert!(apply("abcdef", overlapping).is_err());
        assert_eq!(apply("abcdef", vec![Edit::insert(1, "X".into()), Edit::delete(1, 2)]).unwrap(), "aXcdef");
    }
}
