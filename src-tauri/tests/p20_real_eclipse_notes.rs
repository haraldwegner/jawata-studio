//! Sprint 28k, Stage 20, E2E P20 — studio's COMPILED code reads the notes REAL Eclipse instances wrote.
//!
//! Ignored by default: it needs Eclipse instances (with the plug-in) running, which only the end-to-end script
//! arranges. Run it by hand, once per moment to check:
//!
//! ```text
//! cd src-tauri && JAWATA_P20_NOTES=<dir> JAWATA_P20_SETTINGS=<dir> JAWATA_P20_EXPECT=<running>,<notrunning> \
//!     cargo test --test p20_real_eclipse_notes -- --ignored --nocapture
//! ```
//!
//! * `JAWATA_P20_NOTES`    the folder the Eclipse instances write their notes into.
//! * `JAWATA_P20_SETTINGS` a folder for studio's own settings store. Use the SAME folder for the invocations of
//!                         one run: the first remembers the workspaces it sees, the second (after one Eclipse
//!                         closed and deleted its note) must still list that workspace, as not running.
//! * `JAWATA_P20_EXPECT`   `<rows running>,<rows not running>`, e.g. `2,1`.
//!
//! A note carries the real pid and process start time of its Eclipse, so the real OS liveness check is used. The
//! rows come from the production code (`EclipseProbe`): a real dashboard build and the tray's own menu function.

use jawata_studio_lib::eclipse_probe::EclipseProbe;
use std::{collections::BTreeMap, fs, path::Path};

fn required(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!(
            "{name} is not set. This is an end-to-end test run by hand: \
             JAWATA_P20_NOTES=<notes dir> JAWATA_P20_SETTINGS=<settings dir> JAWATA_P20_EXPECT=<running>,<notrunning> \
             cargo test --test p20_real_eclipse_notes -- --ignored --nocapture"
        ),
    }
}

/// Every file of the notes folder with its bytes: what must be exactly there afterwards.
fn listing(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap_or_default()))
            .collect(),
        Err(_) => BTreeMap::new(),
    }
}

fn expectation() -> (usize, usize) {
    let text = required("JAWATA_P20_EXPECT");
    let parsed = text.split_once(',').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
    parsed.unwrap_or_else(|| panic!("JAWATA_P20_EXPECT must be <running>,<notrunning> (two numbers), not {text:?}"))
}

#[test]
#[ignore = "end-to-end: needs real Eclipse instances; see the module comment for the command"]
fn studios_compiled_code_reads_the_notes_real_eclipse_instances_wrote() {
    let notes = required("JAWATA_P20_NOTES");
    let settings = required("JAWATA_P20_SETTINGS");
    let (want_running, want_not_running) = expectation();
    let (notes, settings) = (Path::new(&notes), Path::new(&settings));

    let before = listing(notes);
    println!("P20: {} note file(s) in {}", before.len(), notes.display());

    let probe = EclipseProbe::open(settings, notes).expect("studio's settings store opens over JAWATA_P20_SETTINGS");
    let rows = probe.dashboard_rows().expect("a dashboard build");
    let tray = probe.tray_lines();
    let remembered = probe.remembered();

    // (3) one line per row, for the e2e log.
    for row in &rows {
        println!(
            "P20 row: {} | {} | {} project(s) | {}",
            row.name,
            if row.running { "running" } else { "not running" },
            row.projects.len(),
            row.path.as_deref().unwrap_or("(no path)")
        );
    }
    println!("P20: studio remembers {} Eclipse workspace(s)", remembered.len());

    // (2) the expected counts.
    let running = rows.iter().filter(|r| r.running).count();
    let not_running = rows.len() - running;
    assert_eq!(
        (want_running, want_not_running),
        (running, not_running),
        "rows running / not running, expected from JAWATA_P20_EXPECT, found in {}: {rows:?}",
        notes.display()
    );

    // No row has a start action or a switch: a row is exactly these facts and nothing that acts.
    for row in &rows {
        let json = serde_json::to_value(row).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(vec!["id", "name", "newerJawata", "path", "projects", "running", "statusText"], keys, "row {}", row.name);
    }

    // The tray, built by its own function from the same rows, equals the list: same rows, same order, greyed.
    assert_eq!(rows.len(), tray.len(), "one tray line per row");
    for (row, line) in rows.iter().zip(&tray) {
        assert_eq!(row.tray_label(), line.label, "tray line of {}", row.name);
        assert_eq!(format!("tray_eclipse:{}", row.id), line.id);
        assert!(!line.enabled, "the tray line of {} must be greyed: there is nothing to start", row.name);
    }

    // Studio remembers every workspace it has seen: each row with a path is in its own settings.
    for row in rows.iter().filter(|r| r.path.is_some()) {
        assert!(
            remembered.iter().any(|m| Some(&m.path) == row.path.as_ref()),
            "studio's settings must remember {} ({:?})",
            row.name,
            row.path
        );
    }

    // Studio deletes nothing and writes nothing into the notes folder.
    assert_eq!(before, listing(notes), "every note that was in the folder before is still there, byte for byte");
    println!("P20: ok — {running} running, {not_running} not running; the tray equals the list; no note touched");
}
