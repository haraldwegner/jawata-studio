//! studio#27 — the project-scanning commands must not run on the UI thread.
//!
//! Tauri dispatches a SYNCHRONOUS `#[tauri::command]` on the main thread. The
//! three commands below walk a filesystem: a `.code-workspace` naming ~200
//! modules takes 15-30 seconds, and while that runs on the main thread the
//! window cannot repaint or respond to input. That is precisely how it was
//! reported — "frozen for a couple of seconds (15-30)" — and no amount of
//! frontend progress reporting can fix it, because the frontend cannot paint
//! either.
//!
//! Declaring them `async` moves them onto the async runtime and frees the UI
//! thread.
//!
//! Why a source assertion rather than a behavioural one: "did this run on the
//! main thread?" is a property of Tauri's dispatcher, which a unit test has no
//! way to observe without standing up a real app and a real window. The
//! declaration is what decides the behaviour, so the declaration is what this
//! pins. It fails loudly, with the reason, if someone makes one synchronous
//! again — which is the regression worth catching, and the one a green suite
//! would otherwise miss entirely.

use std::fs;

/// Commands whose body walks a filesystem and therefore must be async.
const MUST_BE_ASYNC: &[&str] = &[
    "discover_workspace_projects",
    "scan_folder_for_projects",
    "import_workspace_projects",
];

#[test]
fn the_project_scanning_commands_are_async() {
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands.rs"))
        .expect("read commands.rs");

    for name in MUST_BE_ASYNC {
        let asynchronous = format!("pub async fn {name}(");
        let synchronous = format!("pub fn {name}(");

        assert!(
            !source.contains(&synchronous),
            "`{name}` is declared synchronous. Tauri runs a sync command on the MAIN \
             thread, so this filesystem walk would freeze the window for its whole \
             duration — 15-30s on a ~200-module workspace (studio#27). Declare it \
             `pub async fn` instead."
        );
        assert!(
            source.contains(&asynchronous),
            "`{name}` was not found as `pub async fn` in commands.rs. If it was renamed \
             or moved, update MUST_BE_ASYNC in this test — do not delete the check: it \
             is the only thing standing between a long scan and a frozen window."
        );
    }
}
