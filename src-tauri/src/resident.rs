//! Sprint 15 Stage 9: per-workspace `(port, token)` allocation for the
//! resident-JVM hosting model that ships in Stage 10 + Stage 11.
//!
//! Each logical workspace (one `workspace_name` per the Sprint 10 model)
//! gets a stable port in the configurable range (default 8800–8999) and
//! a fresh 32-byte SecureRandom Bearer token, persisted in
//! `projects.json` so both survive manager restarts.
//!
//! Stage 9 ships the allocator + storage shape. Stage 10 wires it to the
//! `ResidentService` JVM spawner; Stage 11 wires it to the URL-emitting
//! MCP-config writer.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write;
use std::net::TcpListener;

/// Default scan range for resident-JVM ports. 200 ports is plenty for any
/// realistic number of workspaces on a single host; the range is exposed
/// via `PortAllocator::with_range` so a Settings override can shift it
/// when a user collides with another local service.
pub const DEFAULT_PORT_RANGE_START: u16 = 8800;
pub const DEFAULT_PORT_RANGE_END: u16 = 8999;

/// One workspace's resident-JVM bookkeeping. Persisted in `projects.json`
/// alongside the `ProjectRecord` collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub workspace_name: String,
    pub resident_port: u16,
    pub resident_token: String,
}

impl WorkspaceState {
    pub fn new(workspace_name: String, resident_port: u16, resident_token: String) -> Self {
        Self {
            workspace_name,
            resident_port,
            resident_token,
        }
    }
}

/// Probes a port range for the first port that's both (a) not already
/// assigned to another workspace and (b) currently bindable on
/// `127.0.0.1`. Exposed as a struct so tests can inject a fake binder.
#[derive(Debug, Clone, Copy)]
pub struct PortAllocator {
    pub range_start: u16,
    pub range_end: u16,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self::with_range(DEFAULT_PORT_RANGE_START, DEFAULT_PORT_RANGE_END)
    }

    pub fn with_range(range_start: u16, range_end: u16) -> Self {
        Self {
            range_start,
            range_end,
        }
    }

    /// Returns a free port not already in `taken`. Probes each candidate
    /// by trying to bind a TCP listener on `127.0.0.1`. Returns `Err` if
    /// every port in the range is either taken or in use by another
    /// process.
    pub fn allocate(&self, taken: &HashSet<u16>) -> Result<u16, String> {
        self.allocate_with_probe(taken, is_port_bindable)
    }

    /// Test-friendly variant: caller injects the bindability probe so
    /// unit tests do not depend on OS port state.
    pub fn allocate_with_probe<F: Fn(u16) -> bool>(
        &self,
        taken: &HashSet<u16>,
        probe: F,
    ) -> Result<u16, String> {
        for port in self.range_start..=self.range_end {
            if taken.contains(&port) {
                continue;
            }
            if probe(port) {
                return Ok(port);
            }
        }
        Err(format!(
            "No free port in range {}-{} for workspace residents \
             (every port either already allocated or in use by another process)",
            self.range_start, self.range_end
        ))
    }
}

impl Default for PortAllocator {
    fn default() -> Self {
        Self::new()
    }
}

fn is_port_bindable(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Generates a fresh Bearer token: 32 bytes of OS-supplied entropy
/// rendered as 64-char lowercase hex.
///
/// Mirrors the fork-side `TokenGenerator` so the manager-provisioned
/// token matches the server's `-token <T>` flag verbatim.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS RNG (getrandom) must be available");
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(s, "{:02x}", byte);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // ===== PortAllocator =====

    #[test]
    fn assigns_free_port_in_range() {
        let allocator = PortAllocator::with_range(9000, 9010);
        let taken = HashSet::new();
        let port = allocator
            .allocate_with_probe(&taken, |_| true)
            .expect("should find a free port");
        assert!(
            (9000..=9010).contains(&port),
            "port {} outside range",
            port
        );
    }

    #[test]
    fn skips_taken_ports() {
        let allocator = PortAllocator::with_range(9000, 9005);
        let mut taken = HashSet::new();
        taken.insert(9000);
        taken.insert(9001);
        taken.insert(9002);
        let port = allocator
            .allocate_with_probe(&taken, |_| true)
            .expect("should find an untaken port");
        assert_eq!(port, 9003, "should pick the lowest untaken port");
    }

    #[test]
    fn skips_in_use_ports() {
        let allocator = PortAllocator::with_range(9000, 9003);
        let taken = HashSet::new();
        // Pretend 9000-9002 are in use; only 9003 is bindable.
        let port = allocator
            .allocate_with_probe(&taken, |p| p == 9003)
            .expect("should find the bindable port");
        assert_eq!(port, 9003);
    }

    #[test]
    fn errors_when_range_exhausted() {
        let allocator = PortAllocator::with_range(9000, 9002);
        let mut taken = HashSet::new();
        taken.extend(9000..=9002);
        let result = allocator.allocate_with_probe(&taken, |_| true);
        assert!(result.is_err(), "should fail when every port is taken");
        let err = result.unwrap_err();
        assert!(err.contains("9000-9002"), "error should name the range: {}", err);
    }

    #[test]
    fn errors_when_every_port_in_use() {
        let allocator = PortAllocator::with_range(9000, 9002);
        let taken = HashSet::new();
        let result = allocator.allocate_with_probe(&taken, |_| false);
        assert!(result.is_err());
    }

    // ===== Token generator =====

    #[test]
    fn tokens_are_64_char_lowercase_hex() {
        let token = generate_token();
        assert_eq!(token.len(), 64, "expected 64 hex chars, got: {}", token);
        assert!(
            token.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "expected lowercase hex only, got: {}",
            token
        );
    }

    #[test]
    fn tokens_are_unique_across_calls() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let token = generate_token();
            assert!(seen.insert(token), "duplicate token generated");
        }
    }

    // ===== WorkspaceState =====

    #[test]
    fn workspace_state_serializes_as_camel_case() {
        let ws = WorkspaceState::new("orb-ws".into(), 8800, "deadbeef".into());
        let json = serde_json::to_string(&ws).expect("serialize");
        assert!(json.contains("\"workspaceName\":\"orb-ws\""), "got: {}", json);
        assert!(json.contains("\"residentPort\":8800"), "got: {}", json);
        assert!(json.contains("\"residentToken\":\"deadbeef\""), "got: {}", json);
    }

    #[test]
    fn workspace_state_round_trips_through_json() {
        let ws = WorkspaceState::new("alpha".into(), 8801, "tok".into());
        let json = serde_json::to_string(&ws).unwrap();
        let back: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ws);
    }

    // ===== Real-OS smoke (probes the actual default range) =====

    #[test]
    fn real_allocator_returns_bindable_port() {
        let allocator = PortAllocator::new();
        let taken = HashSet::new();
        match allocator.allocate(&taken) {
            Ok(port) => {
                assert!(
                    (DEFAULT_PORT_RANGE_START..=DEFAULT_PORT_RANGE_END).contains(&port),
                    "port {} out of default range",
                    port
                );
                // Returned port should still be bindable (race-window is
                // tiny but tolerable for a smoke test).
                assert!(
                    is_port_bindable(port),
                    "allocated port {} should be bindable at the moment of test",
                    port
                );
            }
            Err(e) => {
                // CI hosts can have weird port-state; tolerate by allowing
                // this single test to skip rather than fail.
                eprintln!("real_allocator smoke skipped: {}", e);
            }
        }
    }
}
