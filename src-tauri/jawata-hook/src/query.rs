//! Ask the store, and parse the answer into a declared shape.
//!
//! **This module is the outage's memorial.** The shell scripts peeled jawata's
//! MCP envelope with `sed` expressions like
//! `s/.*"data"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p`. A regex that no
//! longer matches produces an EMPTY STRING, and every caller treated empty as
//! "the store had nothing" — so when the answer's shape moved, the hook
//! reported an absence it had never actually observed, for two weeks, with
//! both products' suites green.
//!
//! So the rule here is narrow and absolute: **a shape we do not recognise is a
//! typed error, never an empty success.** [`QueryError::ShapeChanged`] exists
//! to be seen in a log, and the difference between it and
//! [`Answer::Nothing`] is the entire reason this module is not a regex.

use serde::Deserialize;
use std::time::Duration;

/// What the store said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Text to work with.
    Text(String),
    /// The store answered, and genuinely had nothing. An ABSENCE WE OBSERVED —
    /// which is a different fact from every error below.
    Nothing,
}

/// Why we have no answer. Each variant is a distinct thing to write in the
/// silence log; collapsing them is what the scripts did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    // NOTE: there is deliberately no `NotConfigured` here. Config is read
    // before the store is ever reached, and its absence is
    // SilenceReason::NotConfigured — one name for one fact. A second, unused
    // variant in this enum would be exactly the hollow shape this sprint
    // measures: a value that exists, reads as covered, and nothing constructs.
    /// The resident did not answer in time, or at all.
    Unreachable(String),
    /// HTTP answered with a non-success status.
    Status(u16),
    /// The body was not JSON at all.
    NotJson(String),
    /// **The body was JSON of a shape we do not recognise.** The variant that
    /// had to exist: this is what the regex reported as "nothing found".
    ShapeChanged(String),
    /// The tool itself refused, and said why.
    ToolRefused { code: String, message: String },
}

/// jawata's MCP envelope, declared rather than pattern-matched.
///
/// `{"result":{"content":[{"type":"text","text":"<inner json>"}]}}` — and the
/// inner text is itself a JSON document carrying `{"success":…,"data":…}`.
#[derive(Deserialize)]
struct Envelope {
    result: Option<EnvelopeResult>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct EnvelopeResult {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[derive(Deserialize)]
struct Payload {
    success: bool,
    #[serde(default)]
    data: serde_json::Value,
    #[serde(default)]
    error: Option<PayloadError>,
}

#[derive(Deserialize)]
struct PayloadError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// The store's two ways of saying "I have nothing", matched on the PREFIX the
/// tool documents. Anything else that arrives as text is an answer.
const ABSENCE_PREFIXES: &[&str] = &["No known knowledge", "No domain"];

/// Peel the envelope. Split out from the transport so the shape rules are
/// testable without a server — the property that made the regex untestable was
/// that it only ran inside a curl pipeline.
pub fn parse_answer(body: &str) -> Result<Answer, QueryError> {
    let envelope: Envelope = serde_json::from_str(body)
        .map_err(|e| if body.trim_start().starts_with('{') || body.trim_start().starts_with('[') {
            QueryError::ShapeChanged(format!("envelope did not deserialize: {e}"))
        } else {
            QueryError::NotJson(clip(body))
        })?;

    if let Some(err) = envelope.error {
        return Err(QueryError::ToolRefused {
            code: "JSONRPC".to_string(),
            message: clip(&err.to_string()),
        });
    }

    let result = envelope
        .result
        .ok_or_else(|| QueryError::ShapeChanged("no `result` in the envelope".into()))?;
    let block = result
        .content
        .first()
        .ok_or_else(|| QueryError::ShapeChanged("`result.content` was empty".into()))?;

    let payload: Payload = serde_json::from_str(&block.text)
        .map_err(|e| QueryError::ShapeChanged(format!("inner payload did not deserialize: {e}")))?;

    if !payload.success {
        let (code, message) = payload
            .error
            .map(|e| (e.code, e.message))
            .unwrap_or_else(|| ("UNKNOWN".into(), "the tool refused without saying why".into()));
        return Err(QueryError::ToolRefused { code, message });
    }

    // `data` is a string for the text-format tools this hook uses. A different
    // JSON type means the contract moved — say so rather than stringifying it
    // into something that looks like an answer.
    let text = match payload.data {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => {
            return Err(QueryError::ShapeChanged("`data` was null on a success".into()))
        }
        other => {
            return Err(QueryError::ShapeChanged(format!(
                "`data` was {}, expected a string",
                kind_of(&other)
            )))
        }
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        // Success with empty data is not an absence — the store says absence
        // in words. This is the exact case the regex manufactured.
        return Err(QueryError::ShapeChanged(
            "`data` was an empty string on a success — the store reports absence in words, \
             so this is a changed contract, not 'nothing found'"
                .into(),
        ));
    }
    if ABSENCE_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return Ok(Answer::Nothing);
    }
    Ok(Answer::Text(text))
}

fn kind_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= 200 {
        return s.to_string();
    }
    s.chars().take(200).collect::<String>() + "…"
}

/// Where to ask, and with what.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub url: String,
    pub token: String,
    pub timeout: Duration,
}

/// Call the `experience` tool and peel the answer.
pub fn ask(endpoint: &Endpoint, arguments: serde_json::Value) -> Result<Answer, QueryError> {
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "experience", "arguments": arguments }
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(endpoint.timeout)
        .build()
        .map_err(|e| QueryError::Unreachable(e.to_string()))?;
    let response = client
        .post(&endpoint.url)
        .bearer_auth(&endpoint.token)
        .json(&request)
        .send()
        .map_err(|e| QueryError::Unreachable(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(QueryError::Status(status.as_u16()));
    }
    let body = response.text().map_err(|e| QueryError::Unreachable(e.to_string()))?;
    parse_answer(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(inner: &str) -> String {
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "content": [ { "type": "text", "text": inner } ] }
        })
        .to_string()
    }

    fn success(data: serde_json::Value) -> String {
        envelope(&serde_json::json!({ "success": true, "data": data }).to_string())
    }

    #[test]
    fn a_normal_answer_comes_back_whole() {
        let body = success(serde_json::json!("[lesson] the classifier reads the model\n[lesson] two"));
        assert_eq!(
            Ok(Answer::Text("[lesson] the classifier reads the model\n[lesson] two".into())),
            parse_answer(&body)
        );
    }

    #[test]
    fn multi_answer_is_normal_not_a_reason_to_skip() {
        // Sprint 27a: distance nominates, the agent judges. The script that
        // this replaces DISCARDED every answer with more than one line, which
        // is why it injected nothing for two weeks.
        let many = (1..=11).map(|i| format!("[lesson] line {i}")).collect::<Vec<_>>().join("\n");
        match parse_answer(&success(serde_json::json!(many.clone()))) {
            Ok(Answer::Text(t)) => assert_eq!(11, t.lines().count()),
            other => panic!("multi-line answers must pass through whole: {other:?}"),
        }
    }

    #[test]
    fn an_observed_absence_is_its_own_answer() {
        assert_eq!(
            Ok(Answer::Nothing),
            parse_answer(&success(serde_json::json!("No known knowledge for that cue")))
        );
    }

    // ---- THE POINT OF THE MODULE ----------------------------------------
    //
    // Each of these produced an EMPTY STRING under the regex, and every caller
    // read empty as "the store had nothing". They must all be distinguishable
    // from Answer::Nothing.

    #[test]
    fn a_moved_envelope_is_an_error_not_an_absence() {
        let moved = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "blocks": [ { "type": "text", "text": "{}" } ] }   // content -> blocks
        })
        .to_string();
        match parse_answer(&moved) {
            // The message must NAME what moved — "it didn't work" in a log is
            // the same dead end as the empty string was.
            Err(QueryError::ShapeChanged(why)) => assert!(
                why.contains("content"),
                "the error should name the field that moved, got: {why}"
            ),
            other => panic!("a moved envelope must be a typed error, got {other:?}"),
        }
    }

    #[test]
    fn a_moved_inner_payload_is_an_error_not_an_absence() {
        let moved = envelope(r#"{"ok": true, "payload": "text"}"#);   // success/data -> ok/payload
        assert!(
            matches!(parse_answer(&moved), Err(QueryError::ShapeChanged(_))),
            "renaming success/data must surface, not read as nothing"
        );
    }

    #[test]
    fn data_that_changed_type_is_an_error_not_an_absence() {
        // The likeliest real drift: `data` becomes a structured object.
        match parse_answer(&success(serde_json::json!({ "entries": ["a"] }))) {
            Err(QueryError::ShapeChanged(why)) => assert!(why.contains("object"), "{why}"),
            other => panic!("a retyped `data` must surface, got {other:?}"),
        }
    }

    #[test]
    fn empty_data_on_a_success_is_an_error_not_an_absence() {
        // THE regex's signature output. The store reports absence in words; an
        // empty string means the contract moved.
        assert!(matches!(
            parse_answer(&success(serde_json::json!(""))),
            Err(QueryError::ShapeChanged(_))
        ));
    }

    #[test]
    fn a_refusal_carries_its_code_and_message() {
        let refused = envelope(
            &serde_json::json!({
                "success": false,
                "error": { "code": "SCAN_EXAMINED_NOTHING", "message": "listed 660, read 0" }
            })
            .to_string(),
        );
        match parse_answer(&refused) {
            Err(QueryError::ToolRefused { code, message }) => {
                assert_eq!("SCAN_EXAMINED_NOTHING", code);
                assert!(message.contains("660"));
            }
            other => panic!("a refusal must not read as an absence: {other:?}"),
        }
    }

    #[test]
    fn non_json_is_told_apart_from_a_changed_shape() {
        // An HTML error page and a moved contract are different problems.
        match parse_answer("<html>502 Bad Gateway</html>") {
            Err(QueryError::NotJson(body)) => assert!(body.contains("502")),
            other => panic!("expected NotJson, got {other:?}"),
        }
        assert!(matches!(parse_answer("{\"a\":1}"), Err(QueryError::ShapeChanged(_))));
    }

    #[test]
    fn every_failure_is_distinguishable_from_an_observed_absence() {
        // The single property the whole module exists for.
        let bodies = [
            envelope("not json at all"),
            envelope(r#"{"success": true}"#),
            success(serde_json::json!("")),
            success(serde_json::json!(42)),
            success(serde_json::json!(null)),
            "".to_string(),
            "<html>".to_string(),
        ];
        for body in bodies {
            assert_ne!(
                Ok(Answer::Nothing),
                parse_answer(&body),
                "this body read as an observed absence, which is the outage: {body}"
            );
        }
    }

    #[test]
    fn a_clipped_message_stays_bounded() {
        let long = "x".repeat(10_000);
        if let Err(QueryError::NotJson(b)) = parse_answer(&long) {
            assert!(b.chars().count() <= 201, "a log line must not carry 10k characters");
        } else {
            panic!("expected NotJson");
        }
    }
}
