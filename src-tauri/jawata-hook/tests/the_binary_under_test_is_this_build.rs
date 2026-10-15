//! REL44-1 — the binary every integration test drives is the one THIS crate built.
//!
//! The incident: the first workspace test run after a change to `tauri.conf.json` re-ran the
//! studio build script, which copied the Tauri sidecar — a hook built on 7 August — over
//! `target/debug/jawata-hook` after this crate had linked it. Cargo saw a fresh output and did not
//! relink, so every test in the run drove an older hook. Only the one test whose rule the old
//! binary lacked failed; every other test passed against the wrong program, which is what made it
//! hard to see.
//!
//! The local gate now builds the sidecar from this tree first, so the copy is this build. This
//! test is what notices if anything ever puts a different binary there again: it asks the binary
//! which version it is and compares it with the version this crate was compiled as.

use std::process::Command;

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

#[test]
fn the_hook_binary_these_tests_drive_reports_this_crates_version() {
    let out = Command::new(HOOK)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("could not start the hook binary at {HOOK}: {e}"));
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        out.status.success() && said == env!("CARGO_PKG_VERSION"),
        "the binary at {HOOK} is not this build: it answered {said:?} (exit {:?}) where this crate \
         is {} — a stale copy is standing where the freshly built hook should be, and every other \
         integration test in this run would be testing it",
        out.status.code(),
        env!("CARGO_PKG_VERSION")
    );
}
