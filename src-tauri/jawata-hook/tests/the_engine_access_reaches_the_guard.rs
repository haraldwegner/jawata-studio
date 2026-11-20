//! THE GUARD AGREES WITH THE ENGINE — through the REAL binary, under its deployed role name.
//!
//! # The failure this refuses
//!
//! Harald, 2026-11-20: *"the workspace of VSCode has to be included. Otherwise the jawata agents have no access."* The engine now covers
//! the folders of the editor window he has open, whichever started first. jawata's own guard kept a rule of its own (the workspace file
//! beside the session's folder, else one fixed file), so a brain the engine hired in a folder of that window was refused by the guard
//! what the engine allowed (external audit 2026-11-20, row 28h-C27-31). The engine now hands every brain it starts the folders it may
//! work in, in `JAWATA_ACCESS_FOLDERS`, and the guard adds them to its roots.
//!
//! # Why the REAL binary
//!
//! A unit test on the root list passes whether or not the guard ever reads it; every case below spawns the built executable.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn guard_says(payload: &str, home: &Path, access: Option<&str>) -> String {
    guard_with(payload, home, access, None)
}

fn guard_with(payload: &str, home: &Path, access: Option<&str>, lists: Option<&Path>) -> String {
    let mut env: Vec<(&str, String)> = Vec::new();
    if let Some(folders) = access {
        env.push(("JAWATA_ACCESS_FOLDERS", folders.to_string()));
    }
    if let Some(dir) = lists {
        env.push(("JAWATA_EDITOR_FOLDERS_DIR", dir.display().to_string()));
    }
    guard_env(payload, home, &env)
}

/// The guard run with exactly these of the engine's variables (and `CLAUDE_PROJECT_DIR`), every other one removed.
fn guard_env(payload: &str, home: &Path, env: &[(&str, String)]) -> String {
    let dir = home.join("bin");
    std::fs::create_dir_all(&dir).expect("scratch bin");
    let exe = dir.join(if cfg!(windows) { "jawata-hook-guard.exe" } else { "jawata-hook-guard" });
    // copied once per path: macOS checks a binary's signature per inode, and Linux refuses to run a file still open for writing
    if !exe.exists() {
        std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    }
    std::fs::write(dir.join("hook_config.json"), r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"cursor"}"#)
        .expect("write the hook config");
    let mut attempt = 0;
    let mut child = loop {
        let mut command = Command::new(&exe);
        command.env("HOME", home).env("USERPROFILE", home).env_remove("CLAUDE_PROJECT_DIR").env_remove("JAWATA_ACCESS_FOLDERS")
            .env_remove("JAWATA_ACCESS_FOLDERS_FILE").env_remove("JAWATA_EDITOR_FOLDERS_DIR").env_remove("JAWATA_ACCESS_FROM_ENGINE");
        for (key, value) in env {
            command.env(key, value);
        }
        match command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && attempt < 40 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("the guard binary must be executable: {e}"),
        }
    };
    child.stdin.take().expect("piped").write_all(payload.as_bytes()).expect("write");
    let status = child.wait().expect("the guard must terminate");
    assert_eq!(Some(0), status.code(), "a guard must never fail the client — a non-zero exit is itself a block");
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    out
}

fn scratch_home(name: &str) -> PathBuf {
    // NOT under the platform temp folder: the guard allows that as scratch, which would let every case through
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("jawata-engine-access-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Projects").join("jats2")).expect("scratch home");
    std::fs::create_dir_all(d.join("private")).expect("scratch home");
    d
}

fn read_of(file: &Path) -> String {
    serde_json::json!({
        "session_id": "engine-access",
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": { "file_path": file.to_str().unwrap() },
    })
    .to_string()
}

const REFUSED: &str = "outside the declared workspace";

#[test]
fn a_folder_the_engine_hands_its_brain_is_inside_the_guards_workspace_and_one_it_does_not_hand_is_not() {
    let home = scratch_home("hands");
    let trading = home.join("Projects");
    let separator = if cfg!(windows) { ";" } else { ":" };
    let handed = format!("{}{separator}{}", trading.display(), home.join("notes").display());

    let without = guard_says(&read_of(&trading.join("jats2").join("Order.java")), &home, None);
    let with = guard_says(&read_of(&trading.join("jats2").join("Order.java")), &home, Some(&handed));
    let private = guard_says(&read_of(&home.join("private").join("diary.txt")), &home, Some(&handed));

    let _ = std::fs::remove_dir_all(&home);
    assert!(without.contains(REFUSED), "CONTROL: with no folders from the engine the folder was not refused: {without}");
    assert!(!with.contains(REFUSED), "a folder the engine handed its brain was refused by the guard: {with}");
    assert!(private.contains(REFUSED), "a folder the engine did not hand was let through: {private}");
}

#[test]
fn a_folder_an_editor_window_opens_after_the_brain_started_is_inside_at_once() {
    // Audit 2026-11-20 (bug E): the folders a brain was started with are a picture of that moment; the windows' own lists are read on every
    // call, so a folder the window opens later is reached at once and one it closes is not.
    let home = scratch_home("lists");
    let lists = home.join("editor-folders");
    std::fs::create_dir_all(&lists).expect("lists");
    let later = home.join("Projects");
    let read = read_of(&later.join("jats2").join("Order.java"));

    let before = guard_with(&read, &home, None, Some(&lists));
    std::fs::write(lists.join("window-a.txt"), format!("{}\n", later.display())).expect("a window opens the folder");
    let opened = guard_with(&read, &home, None, Some(&lists));
    std::fs::remove_file(lists.join("window-a.txt")).expect("the window closes");
    let closed = guard_with(&read, &home, None, Some(&lists));

    let _ = std::fs::remove_dir_all(&home);
    assert!(before.contains(REFUSED), "CONTROL: a folder no window holds was not refused: {before}");
    assert!(!opened.contains(REFUSED), "a folder a window opened was refused: {opened}");
    assert!(closed.contains(REFUSED), "a folder of a closed window was still let through: {closed}");
}

#[test]
fn the_projects_folders_are_read_from_the_engines_file_on_every_call() {
    // Second audit 2026-11-20 (finding 1): the engine keeps the loaded projects' folders in a file it rewrites when they change; a picture
    // taken at the brain's start kept a closed folder allowed and refused a project loaded later.
    let home = scratch_home("file");
    let file = home.join("engine-access.txt");
    let trading = home.join("Projects");
    let read = read_of(&trading.join("jats2").join("Order.java"));
    let env = [("JAWATA_ACCESS_FOLDERS_FILE", file.display().to_string())];

    let before = guard_env(&read, &home, &env);
    std::fs::write(&file, format!("{}\n", trading.display())).expect("a project is loaded");
    let loaded = guard_env(&read, &home, &env);
    std::fs::write(&file, "").expect("the project is unloaded");
    let unloaded = guard_env(&read, &home, &env);
    let stale = [("JAWATA_ACCESS_FOLDERS_FILE", file.display().to_string()), ("JAWATA_ACCESS_FOLDERS", trading.display().to_string())];
    let file_wins = guard_env(&read, &home, &stale);

    let _ = std::fs::remove_dir_all(&home);
    assert!(before.contains(REFUSED), "CONTROL: a folder the file does not name yet was not refused: {before}");
    assert!(!loaded.contains(REFUSED), "a project the engine loaded after the brain started was refused: {loaded}");
    assert!(unloaded.contains(REFUSED), "a project the engine no longer holds was still let through: {unloaded}");
    assert!(file_wins.contains(REFUSED), "a list fixed at the brain's start was read beside the engine's file: {file_wins}");
}

#[test]
fn a_brain_the_engine_started_reads_no_workspace_file_and_a_persons_session_still_does() {
    // Second audit 2026-11-20 (finding 9): a workspace file that came with a cloned repository may list any folder; the engine never reads
    // one inside a repository, so the guard of a brain it started reads none, while a person's own session keeps its workspace file.
    let home = scratch_home("clone");
    let clone = home.join("Projects").join("cloned");
    std::fs::create_dir_all(clone.join(".git")).expect("a cloned repository");
    let private = home.join("private");
    std::fs::write(clone.join("shipped.code-workspace"), format!(r#"{{"folders":[{{"path":"{}"}}]}}"#, private.display())).expect("its file");
    let read = read_of(&private.join("diary.txt"));
    let lists = home.join("editor-folders");
    std::fs::create_dir_all(&lists).expect("lists");

    let person = guard_env(&read, &home, &[("CLAUDE_PROJECT_DIR", clone.display().to_string())]);
    let brain = guard_env(&read, &home, &[("CLAUDE_PROJECT_DIR", clone.display().to_string()), ("JAWATA_EDITOR_FOLDERS_DIR", lists.display().to_string())]);

    let _ = std::fs::remove_dir_all(&home);
    assert!(!person.contains(REFUSED), "CONTROL: a person's session no longer reads its workspace file: {person}");
    assert!(brain.contains(REFUSED), "a brain the engine started was let into a folder a cloned repository's workspace file listed: {brain}");
}

#[test]
fn a_brain_the_engine_started_with_nothing_allowed_is_not_given_its_working_folder() {
    // Third audit 2026-11-20 (finding F): with no workspace file read, a brain the engine started can have no folder at all; the old fallback
    // then allowed its working folder, which the engine does not allow. A person's own session keeps that fallback.
    let home = scratch_home("nothing");
    let project = home.join("Projects");
    let lists = home.join("editor-folders");
    std::fs::create_dir_all(&lists).expect("lists");
    let read = read_of(&project.join("notes.txt"));

    let person = guard_env(&read, &home, &[("CLAUDE_PROJECT_DIR", project.display().to_string())]);
    let brain = guard_env(&read, &home, &[("CLAUDE_PROJECT_DIR", project.display().to_string()), ("JAWATA_EDITOR_FOLDERS_DIR", lists.display().to_string())]);

    let _ = std::fs::remove_dir_all(&home);
    assert!(!person.contains(REFUSED), "CONTROL: a person's session lost the fallback to its own folder: {person}");
    assert!(brain.contains(REFUSED), "a brain the engine allows nothing was given its working folder: {brain}");
}
