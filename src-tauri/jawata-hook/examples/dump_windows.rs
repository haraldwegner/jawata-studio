//! Sprint 28g S21 (C21 audit fold) — what the hook would store, for a measurement.
//!
//! Reads each transcript file named on the command line through the hook's own reader
//! (`stop::read_turn_keeping`, the one parser of that file) and prints one JSON line per window: the
//! session, the key, the time, his words, the agent's words to him and one line per tool call. The
//! latency and size figures of the discussion lane are taken from these lines, so they describe the
//! windows the hook stores and not the records a script guesses are windows.
//!
//! Not shipped: an example is built only on request, and nothing in the hook calls it.

use jawata_hook::stop;

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let session = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(turn) = stop::read_turn_keeping(&text, usize::MAX) else { continue };
        for w in turn.closed.iter().chain(std::iter::once(&turn.capture)) {
            if w.is_empty() {
                continue;
            }
            let Some(key) = w.key.as_deref() else { continue };
            let line = serde_json::json!({
                "session": session, "key": key, "ts": w.opened_at, "his": w.his,
                "agent": w.agent_text(), "tools": w.tool_lines(),
            });
            println!("{line}");
        }
    }
}
