//! Sprint 16b/B — the single-service GOJA gateway.
//!
//! The client sees ONE MCP service (`goja`) at a stable port + Bearer token.
//! The gateway is a thin blocking HTTP multiplexer: it terminates the client's
//! MCP/HTTP connection and forwards each JSON-RPC call to the right per-workspace
//! resident (which stays an isolated JVM with its own heap). It holds no JDT
//! state and never merges residents — it only routes.
//!
//! Routing (see [`RoutingTable::resolve`]):
//! - `initialize` / `tools/list` / notifications → any ready resident (the tool
//!   set is identical across residents).
//! - `tools/call` → the resident whose workspace owns the call's `filePath`
//!   (longest project-path prefix), else its `projectKey` (project dir name),
//!   else the first resident. `projectKey` then scopes within that resident as
//!   it already does today.
//!
//! Live end-to-end verification (client → gateway → ≥2 residents) requires the
//! running app and is performed on real hardware; the routing core here is
//! unit-tested.

use serde_json::{json, Value};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};

/// One routable resident: a workspace's JVM endpoint + the projects it owns.
#[derive(Debug, Clone)]
pub struct GatewayRoute {
    pub workspace_name: String,
    /// Full resident MCP URL, e.g. `http://127.0.0.1:8800/mcp`.
    pub url: String,
    /// The resident's Bearer token.
    pub token: String,
    /// Absolute project roots in this workspace (for file-path routing).
    pub project_paths: Vec<String>,
}

/// The set of residents the gateway can route to. Swapped atomically on deploy.
#[derive(Debug, Clone, Default)]
pub struct RoutingTable {
    pub routes: Vec<GatewayRoute>,
}

impl RoutingTable {
    pub fn new(routes: Vec<GatewayRoute>) -> Self {
        Self { routes }
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Choose the resident a JSON-RPC message should be forwarded to.
    /// `None` only when there are no residents at all.
    pub fn resolve(&self, method: &str, params: Option<&Value>) -> Option<&GatewayRoute> {
        if self.routes.is_empty() {
            return None;
        }
        // Workspace-agnostic methods go to any ready resident.
        if method != "tools/call" {
            return self.routes.first();
        }
        if let Some(args) = params.and_then(|p| p.get("arguments")) {
            if let Some(file_path) = args.get("filePath").and_then(Value::as_str) {
                if let Some(route) = self.by_file_path(file_path) {
                    return Some(route);
                }
            }
            if let Some(project_key) = args.get("projectKey").and_then(Value::as_str) {
                if let Some(route) = self.by_project_key(project_key) {
                    return Some(route);
                }
            }
        }
        // Workspace-scoped call with no locator → fall back to the first resident
        // (correct when there is a single workspace; best-effort otherwise).
        self.routes.first()
    }

    /// Longest project-path prefix wins (handles nested project roots).
    fn by_file_path(&self, file_path: &str) -> Option<&GatewayRoute> {
        self.routes
            .iter()
            .filter_map(|route| {
                route
                    .project_paths
                    .iter()
                    .filter(|p| !p.is_empty() && file_path.starts_with(p.as_str()))
                    .map(|p| p.len())
                    .max()
                    .map(|len| (len, route))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, route)| route)
    }

    /// Match the projectKey against the last segment of a project path
    /// (`ProjectKeys.derive` uses the sanitized last segment).
    fn by_project_key(&self, project_key: &str) -> Option<&GatewayRoute> {
        self.routes
            .iter()
            .find(|route| route.project_paths.iter().any(|p| last_segment(p) == project_key))
    }
}

fn last_segment(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// A running gateway server. Dropping it does not stop the thread (the server
/// lives for the app's lifetime); kept to expose the bound port.
pub struct GatewayHandle {
    pub port: u16,
    _join: JoinHandle<()>,
}

/// Bind the gateway on `127.0.0.1:<port>` and serve until process exit.
/// `table` is shared and may be updated (write-lock) as workspaces change.
pub fn spawn(
    port: u16,
    gateway_token: String,
    table: Arc<RwLock<RoutingTable>>,
) -> Result<GatewayHandle, String> {
    let server = tiny_http::Server::http(("127.0.0.1", port))
        .map_err(|e| format!("gateway: cannot bind 127.0.0.1:{port}: {e}"))?;
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .build()
            .map_err(|e| format!("gateway: cannot build http client: {e}"))?,
    );

    let join = thread::spawn(move || {
        for request in server.incoming_requests() {
            let token = gateway_token.clone();
            let table = Arc::clone(&table);
            let client = Arc::clone(&client);
            // One thread per request so a long-lived SSE channel does not
            // block POST /mcp request/response traffic.
            thread::spawn(move || handle(request, &token, &table, &client));
        }
    });

    Ok(GatewayHandle { port, _join: join })
}

fn handle(
    mut request: tiny_http::Request,
    gateway_token: &str,
    table: &RwLock<RoutingTable>,
    client: &reqwest::blocking::Client,
) {
    if !authorized(&request, gateway_token) {
        respond(request, 401, "");
        return;
    }

    let url_path = request.url().to_string();
    let is_post = *request.method() == tiny_http::Method::Post;
    let is_get = *request.method() == tiny_http::Method::Get;

    // SSE: proxy GET /mcp/events to the default resident (streaming passthrough).
    if is_get && url_path.starts_with("/mcp/events") {
        proxy_sse(request, table, client);
        return;
    }
    if !is_post || !url_path.starts_with("/mcp") {
        respond(request, 405, "");
        return;
    }

    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        respond(request, 400, "");
        return;
    }
    let parsed: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(_) => {
            respond(request, 400, "");
            return;
        }
    };

    let rpc_method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
    let params = parsed.get("params");
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);

    let target = {
        let guard = table.read().expect("routing table lock poisoned");
        guard.resolve(rpc_method, params).cloned()
    };
    let Some(target) = target else {
        let err = json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32001, "message": "No GOJA workspace resident is running"}
        });
        respond_json(request, 200, &err.to_string());
        return;
    };

    match client
        .post(&target.url)
        .header("Authorization", format!("Bearer {}", target.token))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let text = resp.text().unwrap_or_default();
            respond_json(request, status, &text);
        }
        Err(e) => {
            let err = json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32002, "message": format!("gateway upstream error: {e}")}
            });
            respond_json(request, 502, &err.to_string());
        }
    }
}

fn proxy_sse(
    request: tiny_http::Request,
    table: &RwLock<RoutingTable>,
    client: &reqwest::blocking::Client,
) {
    let target = { table.read().expect("routing table lock poisoned").routes.first().cloned() };
    let Some(target) = target else {
        respond(request, 503, "");
        return;
    };
    let events_url = format!("{}/events", target.url); // <base>/mcp -> <base>/mcp/events
    match client
        .get(&events_url)
        .header("Authorization", format!("Bearer {}", target.token))
        .send()
    {
        Ok(resp) => {
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..])
                    .expect("static header");
            // Unknown length → tiny_http uses chunked transfer for the stream.
            let response = tiny_http::Response::new(
                tiny_http::StatusCode(200),
                vec![header],
                resp,
                None,
                None,
            );
            let _ = request.respond(response);
        }
        Err(_) => respond(request, 502, ""),
    }
}

fn authorized(request: &tiny_http::Request, gateway_token: &str) -> bool {
    let expected = format!("Bearer {gateway_token}");
    request.headers().iter().any(|h| {
        h.field.as_str().as_str().eq_ignore_ascii_case("authorization") && h.value.as_str() == expected
    })
}

fn respond(request: tiny_http::Request, status: u16, body: &str) {
    let _ = request.respond(tiny_http::Response::from_string(body).with_status_code(status));
}

fn respond_json(request: tiny_http::Request, status: u16, body: &str) {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header");
    let response = tiny_http::Response::from_string(body)
        .with_status_code(status)
        .with_header(header);
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn route(ws: &str, port: u16, paths: &[&str]) -> GatewayRoute {
        GatewayRoute {
            workspace_name: ws.into(),
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: format!("tok-{ws}"),
            project_paths: paths.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn table() -> RoutingTable {
        RoutingTable::new(vec![
            route("alpha", 8800, &["/home/u/alpha"]),
            route("beta", 8801, &["/home/u/beta", "/home/u/beta-extra"]),
        ])
    }

    #[test]
    fn empty_table_resolves_to_none() {
        let t = RoutingTable::default();
        assert!(t.resolve("tools/list", None).is_none());
        assert!(t.resolve("tools/call", None).is_none());
    }

    #[test]
    fn non_call_methods_go_to_first_resident() {
        let t = table();
        assert_eq!(t.resolve("initialize", None).unwrap().workspace_name, "alpha");
        assert_eq!(t.resolve("tools/list", None).unwrap().workspace_name, "alpha");
        assert_eq!(t.resolve("ping", None).unwrap().workspace_name, "alpha");
    }

    #[test]
    fn tools_call_routes_by_file_path() {
        let t = table();
        let params = json!({"name": "get_at_position", "arguments": {"filePath": "/home/u/beta/src/A.java"}});
        assert_eq!(t.resolve("tools/call", Some(&params)).unwrap().workspace_name, "beta");

        let params2 = json!({"arguments": {"filePath": "/home/u/alpha/X.java"}});
        assert_eq!(t.resolve("tools/call", Some(&params2)).unwrap().workspace_name, "alpha");
    }

    #[test]
    fn tools_call_longest_prefix_wins() {
        let t = RoutingTable::new(vec![
            route("root", 8800, &["/home/u"]),
            route("nested", 8801, &["/home/u/nested"]),
        ]);
        let params = json!({"arguments": {"filePath": "/home/u/nested/deep/B.java"}});
        assert_eq!(t.resolve("tools/call", Some(&params)).unwrap().workspace_name, "nested");
    }

    #[test]
    fn tools_call_routes_by_project_key_when_no_path() {
        let t = table();
        let params = json!({"arguments": {"projectKey": "beta"}});
        assert_eq!(t.resolve("tools/call", Some(&params)).unwrap().workspace_name, "beta");
    }

    #[test]
    fn tools_call_without_locator_falls_back_to_first() {
        let t = table();
        let params = json!({"arguments": {}});
        assert_eq!(t.resolve("tools/call", Some(&params)).unwrap().workspace_name, "alpha");
        assert_eq!(t.resolve("tools/call", None).unwrap().workspace_name, "alpha");
    }

    #[test]
    fn unknown_file_path_falls_back_to_first() {
        let t = table();
        let params = json!({"arguments": {"filePath": "/somewhere/else/C.java"}});
        assert_eq!(t.resolve("tools/call", Some(&params)).unwrap().workspace_name, "alpha");
    }
}
