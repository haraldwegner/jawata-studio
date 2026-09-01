//! A blocking sleep in a SYNCHRONOUS Tauri command freezes the window.
//!
//! Tauri runs a synchronous command on the main thread — the thread that
//! repaints. `commands.rs` has said so since studio#27, which made three
//! commands `async` after a workspace walk froze the window for 15-30 s and was
//! reported as exactly that: "frozen for a couple of seconds (15-30)".
//!
//! v3.18.0 walked into it again anyway. A stagger was added to
//! `start_all_runtimes` so three residents stop starting in the same second, and
//! it was implemented as `std::thread::sleep` between spawns. The reasoning was
//! entirely about CPU contention and never asked which thread the sleep was on.
//! Three workspaces, two gaps, ten seconds each: twenty seconds of "jawata does
//! not respond" on every fleet start — the CPU spike traded for a longer freeze,
//! which is the opposite of the point, since what the stagger protects is the
//! machine feeling usable.
//!
//! The rule was written down two lines above the function that broke it. That is
//! the lesson: a comment is a rule for whoever reads it, and I did not read it
//! while editing the function beneath it. This test asks the same question
//! mechanically, of every command, on every run.
//!
//! AND THE FIRST VERSION OF THIS TEST WOULD NOT HAVE CAUGHT IT. It matched
//! `sleep` textually inside a command body — but the sleep lives one call away,
//! in `manager_service::start_all_runtimes`, and the command merely delegates.
//! Reverting the fix proved it: the named test below went red, the general one
//! stayed green. A check that cannot see the defect it was written for is the
//! same failure as the gates this project has spent a week repairing.
//!
//! So the general check follows the codebase's OWN marker instead of guessing at
//! bodies. `manager_service.rs` documents a blocking method with the phrase
//! "must never be called from the main thread" — `sync_releases_now` has carried
//! it since v3.6.2 for a 112 MB download. Any method bearing it is blocking by
//! its author's own statement, and no synchronous command may call one.
//!
//! WHAT IT STILL CANNOT SEE, so a green is not over-read: a blocking method
//! nobody marked. The marker is a human act, and this test enforces it rather
//! than deriving it. That is a real limit — but it is the limit of a convention
//! two authors have now independently reached for, not of a guess.

use std::fs;
use std::path::Path;

/// Split `commands.rs` into (signature, body) pairs, one per `#[tauri::command]`.
fn commands(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].trim_start().starts_with("#[tauri::command") {
            // the signature is the next line beginning with pub fn / pub async fn
            let mut j = i + 1;
            while j < bytes.len() && !bytes[j].trim_start().starts_with("pub ") {
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            let signature = bytes[j].trim().to_string();
            // body runs until the next #[tauri::command] or end of file — coarse,
            // and deliberately so: a false positive here is a test to read, while
            // a false negative is a frozen window.
            let mut k = j + 1;
            let mut body = String::new();
            while k < bytes.len() && !bytes[k].trim_start().starts_with("#[tauri::command") {
                body.push_str(bytes[k]);
                body.push('\n');
                k += 1;
            }
            out.push((signature, body));
            i = k;
        } else {
            i += 1;
        }
    }
    out
}

fn commands_source() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands.rs");
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

#[test]
fn no_synchronous_command_sleeps_on_the_repaint_thread() {
    let src = commands_source();
    let all = commands(&src);
    assert!(
        all.len() > 20,
        "the parser found only {} commands — it has stopped seeing the file, \
         and a check that sees nothing passes everything",
        all.len()
    );

    let mut offenders = Vec::new();
    for (signature, body) in &all {
        let is_async = signature.starts_with("pub async fn");
        if is_async {
            continue;
        }
        if body.contains("thread::sleep") || body.contains("sleep(") {
            offenders.push(signature.clone());
        }
    }

    assert!(
        offenders.is_empty(),
        "these SYNCHRONOUS Tauri commands block the repaint thread with a sleep, \
         which is how the window stops responding — make them `pub async fn` so \
         they run on the async runtime instead:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_staggered_start_is_async_because_it_sleeps() {
    // The specific regression, pinned by name. The general test above would also
    // catch it, but only while the sleep is spelled `sleep`; this one fails if
    // the command ever goes back to synchronous, whatever the body does.
    let src = commands_source();
    let (signature, _) = commands(&src)
        .into_iter()
        .find(|(s, _)| s.contains("start_all_runtimes"))
        .expect("start_all_runtimes must exist as a Tauri command");

    assert!(
        signature.starts_with("pub async fn"),
        "start_all_runtimes spaces resident spawns apart by sleeping between them. \
         On a synchronous command that sleep runs on the thread that repaints the \
         window — v3.18.0 shipped exactly that and made every fleet start a \
         twenty-second freeze. It must stay `pub async fn`. Found: {signature}"
    );
}

/// Methods `manager_service.rs` declares blocking, by their own doc comment.
fn methods_marked_blocking() -> Vec<String> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/manager_service.rs");
    let src = fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read: {e}"));
    let lines: Vec<&str> = src.lines().collect();
    let mut marked = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("must never be called from the main thread") {
            continue;
        }
        // the next `pub fn` below the marker is the method it describes
        for probe in lines.iter().skip(i) {
            let t = probe.trim_start();
            if let Some(rest) = t.strip_prefix("pub fn ") {
                if let Some(name) = rest.split('(').next() {
                    marked.push(name.to_string());
                }
                break;
            }
        }
    }
    marked
}


/// Does this body call `method` on the CALLING thread — i.e. not inside a
/// spawned closure?
///
/// The first version of this check asked only whether the body mentioned the
/// call, and it immediately flagged `update_settings`, which is CORRECT code:
/// it calls `sync_releases_now()` inside `std::thread::spawn`, precisely because
/// its author knew Save must not block on a 112 MB download. A check that
/// condemns the code written to avoid the defect is worse than no check — it
/// gets switched off, and takes the true positives with it.
///
/// So the body is walked with brace depth. A `spawn(` opens a region that
/// belongs to another thread, and a call inside it is not on this one.
fn calls_outside_a_spawn(body: &str, method: &str) -> bool {
    let needle = format!(".{method}(");
    let mut depth: i32 = 0;
    let mut spawn_depth: Option<i32> = None;
    for line in body.lines() {
        if spawn_depth.is_none() && (line.contains("thread::spawn") || line.contains("spawn(")) {
            spawn_depth = Some(depth);
        }
        if spawn_depth.is_none() && line.contains(&needle) {
            return true;
        }
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if let Some(d) = spawn_depth {
            if depth <= d {
                spawn_depth = None;
            }
        }
    }
    false
}

#[test]
fn no_synchronous_command_calls_a_method_declared_blocking() {
    let marked = methods_marked_blocking();
    assert!(
        !marked.is_empty(),
        "no method carries the blocking marker — either the convention was \
         renamed or this test has gone blind, and a blind check passes everything"
    );

    let src = commands_source();
    let mut offenders = Vec::new();
    for (signature, body) in commands(&src) {
        if signature.starts_with("pub async fn") {
            continue;
        }
        for m in &marked {
            if calls_outside_a_spawn(&body, m) {
                offenders.push(format!("{signature}  calls  {m}()"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these SYNCHRONOUS Tauri commands call a method its own author marked \
         'must never be called from the main thread'. A sync command IS the main \
         thread, so the window stops repainting for as long as the call takes. \
         Make the command `pub async fn`:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_control_the_general_test_needs() {
    // A test that has only ever seen a clean file is indistinguishable from one
    // that cannot see anything. Feed the parser the shape it must reject.
    let fixture = r#"
#[tauri::command]
pub fn innocent(state: State<'_, AppState>) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub fn guilty(state: State<'_, AppState>) -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_secs(10));
    Ok(())
}

#[tauri::command]
pub async fn forgiven(state: State<'_, AppState>) -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_secs(10));
    Ok(())
}
"#;
    let found = commands(fixture);
    assert_eq!(found.len(), 3, "the parser must see all three commands");

    let flagged: Vec<&String> = found
        .iter()
        .filter(|(s, b)| !s.starts_with("pub async fn") && b.contains("thread::sleep"))
        .map(|(s, _)| s)
        .collect();

    assert_eq!(
        flagged.len(),
        1,
        "exactly the synchronous sleeper must be flagged — not the innocent one, \
         and not the async one that sleeps legitimately. Flagged: {flagged:?}"
    );
    assert!(flagged[0].contains("guilty"));
}

#[test]
fn the_spawn_control_the_blocking_check_needs() {
    // Both directions, or the check is only ever seen agreeing with itself.
    let direct = r#"
    let x = state.manager_service.sync_releases_now();
    Ok(x)
"#;
    assert!(
        calls_outside_a_spawn(direct, "sync_releases_now"),
        "a call made straight from the command body must be flagged"
    );

    let spawned = r#"
    if release_repo_changed {
        std::thread::spawn(move || {
            match state.manager_service.sync_releases_now() {
                Ok(true) => {}
                _ => {}
            }
        });
    }
    Ok(dashboard)
"#;
    assert!(
        !calls_outside_a_spawn(spawned, "sync_releases_now"),
        "a call inside a spawned closure runs on another thread and must NOT be \
         flagged — this is update_settings, which is correct code"
    );
}
