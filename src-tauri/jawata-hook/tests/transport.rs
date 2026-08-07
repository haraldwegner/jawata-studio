//! The real transport, against a real socket.
//!
//! C5 audit finding F4: `query::ask` had ZERO coverage. The unreachable-endpoint
//! hazard was tested by handing the pipeline a `QueryError::Unreachable` value
//! constructed by hand, which asserts what the pipeline does with an error and
//! nothing about whether the transport ever produces one.
//!
//! What was uncovered: the reqwest-error→`Unreachable` mapping, the
//! non-success→`Status` mapping, the bearer header, and the JSON-RPC request
//! body. A misspelled `"name": "experience"` or a `is_client_error()` where
//! `is_success()` belongs would leave every test green while every real recall
//! returned nothing — and the silence log would say `StoreHadNothing`, a claim
//! about the store the hook never actually made.

use jawata_hook::query::{ask, Answer, Endpoint, QueryError};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

/// Serve exactly one request with the given raw HTTP response, and hand back
/// what the client sent so the request itself can be asserted.
fn serve_once(response: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}/mcp"), rx)
}

fn endpoint(url: String) -> Endpoint {
    Endpoint { url, token: "test-token".into(), timeout: Duration::from_millis(1500) }
}

fn http(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
}

#[test]
fn a_refused_connection_becomes_unreachable_not_an_absence() {
    // Port 1 on loopback: nothing listens, and the connection is refused
    // immediately rather than hanging.
    let result = ask(&endpoint("http://127.0.0.1:1/mcp".into()), serde_json::json!({"kind":"primer"}));
    match result {
        Err(QueryError::Unreachable(_)) => {}
        other => panic!("a refused connection must map to Unreachable, got {other:?}"),
    }
}

#[test]
fn a_non_success_status_is_reported_as_that_status() {
    let (url, _rx) = serve_once("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
    match ask(&endpoint(url), serde_json::json!({"kind":"primer"})) {
        Err(QueryError::Status(503)) => {}
        other => panic!("a 503 must surface as its status, got {other:?}"),
    }
}

#[test]
fn a_real_answer_travels_the_whole_path() {
    // End to end over a socket: request built, header set, response read,
    // envelope peeled.
    let inner = serde_json::json!({ "success": true, "data": "[lesson] a real line" }).to_string();
    let envelope =
        serde_json::json!({ "result": { "content": [ { "type": "text", "text": inner } ] } })
            .to_string();
    let (url, rx) = serve_once(Box::leak(http(&envelope).into_boxed_str()));

    let answer = ask(&endpoint(url), serde_json::json!({ "kind": "primer", "format": "text" }));
    assert_eq!(Ok(Answer::Text("[lesson] a real line".into())), answer);

    // And the REQUEST is asserted, because a misspelled tool name or a missing
    // header fails silently in exactly the way this crate exists to prevent.
    let request = rx.recv_timeout(Duration::from_secs(2)).expect("the server saw a request");
    assert!(request.contains("POST /mcp"), "wrong path/method:\n{request}");
    assert!(
        request.contains("authorization: Bearer test-token")
            || request.contains("Authorization: Bearer test-token"),
        "the bearer token must be sent:\n{request}"
    );
    assert!(request.contains(r#""method":"tools/call""#), "wrong JSON-RPC method:\n{request}");
    assert!(request.contains(r#""name":"experience""#), "wrong tool name:\n{request}");
}

#[test]
fn a_server_that_answers_with_garbage_is_not_an_absence() {
    let (url, _rx) = serve_once(Box::leak(http("<html>hello</html>").into_boxed_str()));
    match ask(&endpoint(url), serde_json::json!({"kind":"primer"})) {
        Err(QueryError::NotJson(_)) => {}
        other => panic!("garbage must not read as an absence, got {other:?}"),
    }
}
