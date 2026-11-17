//! Which brains this machine has, and a one-word test run through one, as the ENGINE answers them (28h, the architect's watch of C26).
//!
//! Harald, 2026-10-12: *"We cannot fake this. We need to know what is availabe"* · *"No fake model names!"* · *"The error should show
//! the reason and prompt the user to login through bash"*. The engine reads each answer off the program itself (`BrainRoster` in
//! jawata-mcp) and the studio asks it through its `brains` tool, so the rules about a brain (how it reports its login, which models it
//! names, what its command line takes for a model) are kept in one place. Until C26 this file asked every program itself, a second copy
//! of each rule, and a model fix had to be made twice.
//!
//! What stays here is what only the studio does: the brains a seat may be given (`HIRABLE`, `HELD_BY_AN_INSTRUCTION`,
//! pinned against the engine by a test on each side) and where on this machine a program is, for the settings a deploy writes.

use serde::{Deserialize, Serialize};

/// Whether the program says its account is logged in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Login {
    LoggedIn,
    NotLoggedIn,
    /// The program has no way to say without starting a session, or said something unreadable.
    Unknown,
}

/// One agent program, as this machine has it right now.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrainStatus {
    pub id: String,
    pub label: String,
    pub installed: bool,
    /// The first line the program printed for `--version`; the engine's answer does not carry it.
    #[serde(default)]
    pub version: Option<String>,
    pub login: Login,
    /// The program's own words when it is not logged in, or why the check could not answer.
    pub reason: Option<String>,
    /// What to type in a terminal to log in. Shown with every failed login.
    pub login_command: String,
    /// The models the program lists for the account (Grok, Cursor); `None` where it names none (Claude Code's
    /// come from its own session, through the engine's catalogue) — not the same as an empty list.
    pub models: Option<Vec<String>>,
    /// Whether a hire can start this brain at all (`HIRABLE`).
    pub hirable: bool,
    /// Whether a hire passes this brain a model; the others choose their own (the engine's answer).
    pub takes_model: bool,
    /// Whether this brain's writes are held only by an instruction to the model
    /// (`HELD_BY_AN_INSTRUCTION`), so a seat choosing it runs with that bound and no other.
    pub writes_held_by_instruction: bool,
}

struct Spec {
    id: &'static str,
    program: &'static str,
}

/// The program each brain is started as, for finding it on this machine; the engine's `BrainRoster` holds the same roster for its checks.
static SPECS: [Spec; 5] = [
    Spec { id: "claude-code", program: "claude" },
    Spec { id: "codex", program: "codex" },
    Spec { id: "cursor", program: "cursor-agent" },
    Spec { id: "copilot", program: "copilot" },
    Spec { id: "grok", program: "grok" },
];
/// The brains the engine's hire can start — jawata-mcp's `SeatHire.Brain`. Only these are offered
/// as a seat's brain: offering one the hire cannot start (Grok, today) would be a choice that
/// fails every hire.
pub const HIRABLE: [&str; 4] = ["claude-code", "codex", "cursor", "copilot"];

/// The hirable brains whose WRITES are held only by an instruction to the model, not by a
/// mechanism — jawata-mcp's `WriteContainment` answers INSTRUCTION for them (S16, the architect's
/// F1). The engine refuses a hire on one unless the seat's file says `accepts_uncontained_writes:
/// true`; choosing one in the Seats view is the user's word, so the view writes that line with the
/// brain (`seat_settings::seat_file_content`) and says so beside the choice.
pub const HELD_BY_AN_INSTRUCTION: [&str; 1] = ["cursor"];


/// What one test run through a brain showed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub brain: String,
    /// The model asked for; the studio fills it in, as the engine's answer does not echo it.
    #[serde(default)]
    pub model: Option<String>,
    pub ok: bool,
    /// What the brain answered.
    pub answer: Option<String>,
    /// The models the run itself reported using, most expensive first. Empty where the brain
    /// does not report one (Codex) — never filled in from the request.
    pub models_used: Vec<String>,
    /// The cost the brain reported, where it reports one (Claude Code only).
    pub cost_usd: Option<f64>,
    pub seconds: f64,
    /// The brain's own words when the run failed.
    pub error: Option<String>,
    pub login_command: String,
}

/// Where on this machine the program of one brain can be started from, as an absolute path — the
/// answer a setting that names a command line needs. Read from the same [`SPECS`] roster, never a second list of
/// programs; the one extra name is Cursor's short alias `agent`, the form its ACP runs were made
/// with, tried first, then the roster's own `cursor-agent`.
///
/// `Ok(None)` is "not installed" (nothing on the path answers to the name), `Err` is "could not
/// look" (the path variable is missing). A relative path entry is not searched: the answer must
/// be absolute, and a relative one would name a different program from every other working folder.
pub fn installed_program_path(id: &str, path_var: Option<&std::ffi::OsStr>) -> Result<Option<std::path::PathBuf>, String> {
    let Some(spec) = SPECS.iter().find(|s| s.id == id) else {
        return Ok(None);
    };
    let Some(path_var) = path_var else {
        return Err(format!("PATH is not set, so `{}` could not be looked for", spec.program));
    };
    let names: Vec<&str> = if spec.id == "cursor" { vec!["agent", spec.program] } else { vec![spec.program] };
    for name in names {
        for dir in std::env::split_paths(path_var).filter(|d| d.is_absolute()) {
            for file in program_file_names(name) {
                let candidate = dir.join(file);
                if is_executable_file(&candidate) {
                    return Ok(Some(candidate));
                }
            }
        }
    }
    Ok(None)
}

/// The full path of a program called `name`, when one of the folders of `path_var` has it.
pub fn program_on_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    let path_var = path_var?;
    for dir in std::env::split_paths(path_var).filter(|d| d.is_absolute()) {
        for file in program_file_names(name) {
            let candidate = dir.join(file);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// The file names a program called `name` can have in a folder on this system.
fn program_file_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.bat")]
    } else {
        vec![name.to_string()]
    }
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brains_held_only_by_an_instruction_are_the_ones_the_engine_answers_for() {
        assert_eq!(
            HELD_BY_AN_INSTRUCTION,
            ["cursor"],
            "jawata-mcp's TheBrainsHeldOnlyByAnInstructionAreTheOnesStudioListsTest pins the engine's side"
        );
        for brain in HELD_BY_AN_INSTRUCTION {
            assert!(HIRABLE.contains(&brain), "{brain} is listed as held by an instruction but cannot be hired");
        }
        // The engine's side iterates exactly these four names; a fifth hirable brain, or a changed
        // one, must reach that test's list too, or its answer is never asked for.
        assert_eq!(
            HIRABLE,
            ["claude-code", "codex", "cursor", "copilot"],
            "jawata-mcp's TheBrainsHeldOnlyByAnInstructionAreTheOnesStudioListsTest asks the engine about these four"
        );
    }
}

#[cfg(test)]
mod program_path_tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A folder the test made and removes, panic or not.
    struct Scratch(PathBuf);

    impl std::ops::Deref for Scratch {
        type Target = PathBuf;
        fn deref(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir_with(label: &str, programs: &[&str]) -> Scratch {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-studio-brainpath-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for name in programs {
            make_program(&dir, name);
        }
        Scratch(dir)
    }

    fn file_name(name: &str) -> String {
        if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
    }

    fn make_program(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(file_name(name));
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn path_of(dirs: &[&Scratch]) -> OsString {
        std::env::join_paths(dirs.iter().map(|d| &d.0)).unwrap()
    }

    #[test]
    fn the_short_cursor_name_is_found_on_the_path_as_an_absolute_path() {
        let dir = dir_with("agent", &["agent"]);
        let found = installed_program_path("cursor", Some(&path_of(&[&dir]))).unwrap();
        assert_eq!(found, Some(dir.join(file_name("agent"))));
        assert!(found.unwrap().is_absolute());
    }

    #[test]
    fn the_rosters_own_program_is_found_when_the_short_name_is_absent() {
        let dir = dir_with("cursor-agent", &["cursor-agent"]);
        let found = installed_program_path("cursor", Some(&path_of(&[&dir]))).unwrap();
        assert_eq!(found, Some(dir.join(file_name("cursor-agent"))));
    }

    #[test]
    fn the_short_name_wins_over_the_long_one_and_an_earlier_folder_over_a_later() {
        let first = dir_with("both-first", &["agent", "cursor-agent"]);
        let second = dir_with("both-second", &["agent"]);
        let found = installed_program_path("cursor", Some(&path_of(&[&first, &second]))).unwrap();
        assert_eq!(found, Some(first.join(file_name("agent"))));
    }

    #[test]
    fn a_machine_without_the_program_answers_not_installed() {
        let empty = dir_with("none", &[]);
        assert_eq!(installed_program_path("cursor", Some(&path_of(&[&empty]))), Ok(None));
        let other = dir_with("other", &["claude"]);
        assert_eq!(installed_program_path("cursor", Some(&path_of(&[&other]))), Ok(None));
    }

    #[test]
    fn a_brain_the_roster_does_not_know_names_no_program() {
        let dir = dir_with("unknown", &["agent"]);
        assert_eq!(installed_program_path("no-such-brain", Some(&path_of(&[&dir]))), Ok(None));
    }

    #[test]
    fn a_missing_path_variable_is_could_not_look_with_a_reason() {
        let reason = installed_program_path("cursor", None).expect_err("no PATH cannot be searched");
        assert!(reason.contains("PATH"), "{reason}");
    }

    #[test]
    fn a_relative_path_entry_is_not_searched() {
        // A folder that really holds an `agent`, reachable only by a path relative to the working folder.
        let name = format!("jawata-relpath-test-{}", std::process::id());
        let dir = std::env::current_dir().unwrap().join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        make_program(&dir, "agent");
        let relative = std::env::join_paths([Path::new(&name)]).unwrap();
        let found = installed_program_path("cursor", Some(&relative));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, Ok(None), "a relative folder would name a different program from every other working folder");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_executed_is_not_a_program() {
        let dir = dir_with("noexec", &[]);
        let path = dir.join("agent");
        std::fs::write(&path, "x").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(installed_program_path("cursor", Some(&path_of(&[&dir]))), Ok(None));
    }
}
