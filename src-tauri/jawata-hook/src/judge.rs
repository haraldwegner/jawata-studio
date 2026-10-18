//! Sprint 28g S23 step O2 — the observer's second tier: a hired seat that judges a window.
//!
//! His design, in his words: *"The observer must be fast but not superfast in the pipeline. So
//! he can halt and bounce back but not on every call."* The first tier ([`crate::watch`]) holds
//! every case a pattern can decide. What a pattern cannot decide — a question left waiting, work
//! moved on the agent's own authority — is a judgement over a short window, and a judgement is a
//! hired seat (`seats/observer.md`, the seat file being the hire).
//!
//! **NEVER AWAITED BY A CALL.** A hook process lives for milliseconds and a seat takes seconds.
//! So the ordinary call does the cheap half only: every [`EVERY`] calls it writes the window as a
//! brief, leaves a marker, and starts a DETACHED child of this same binary (`observer-judge`)
//! that runs the hire and writes the verdict. The call returns at once whether or not the child
//! starts. The NEXT call reads the verdict file: a `HALT` refuses that one call with the
//! correction, once; a `CLEAR` leaves nothing.
//!
//! **WHEN IT CANNOT RUN IT SAYS NOTHING.** No seats directory in the config, no seat file, a
//! resident that does not answer, an answer that is neither line of the contract: no verdict is
//! written and the first tier stands alone. An observer that halts on what it could not read
//! turns every outage into a refusal. The failure is recorded as a signal, not hidden.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::watch::Window;

/// A window is handed to the seat every this many calls.
pub const EVERY: u64 = 8;

/// A judge started this long ago and not finished is presumed gone; another may start.
pub const IN_FLIGHT_SECS: u64 = 150;

/// A verdict older than this is stale: the window it judged is no longer the recent past.
pub const VERDICT_TTL_SECS: u64 = 300;

/// The subcommand the detached child is started with.
pub const SUBCOMMAND: &str = "observer-judge";

/// How many of his last messages ride in the brief, and how much of each.
const HIS_MESSAGES: usize = 3;
const HIS_CHARS: usize = 600;

/// What the seat answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Clear,
    Halt { shape: u8, correction: String },
}

/// Read the seat's answer. The contract is one line, `VERDICT: CLEAR` or
/// `VERDICT: HALT <1-5>: <correction>`. Anything else is `None`: a verdict that cannot be read
/// is not a verdict, and is not guessed at.
pub fn parse_verdict(answer: &str) -> Option<Verdict> {
    let line = answer.lines().map(str::trim).rev().find(|l| l.starts_with("VERDICT:"))?;
    let rest = line.trim_start_matches("VERDICT:").trim();
    if rest.eq_ignore_ascii_case("CLEAR") {
        return Some(Verdict::Clear);
    }
    let rest = rest.strip_prefix("HALT")?.trim();
    let (shape, correction) = rest.split_once(':')?;
    let shape: u8 = shape.trim().parse().ok().filter(|n| (1..=5).contains(n))?;
    let correction = correction.trim();
    if correction.is_empty() {
        return None;
    }
    Some(Verdict::Halt { shape, correction: correction.to_string() })
}

fn dir(home: &Path) -> PathBuf {
    home.join(".claude").join("jawata-studio").join("watch")
}

fn file(home: &Path, session: &str, suffix: &str) -> PathBuf {
    dir(home).join(format!("{}.{suffix}", crate::pipeline::sanitize_session(session)))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The brief the seat is shown: the window's calls numbered as the hook numbered them, and what he
/// last typed. Plain text, because the seat has no tools and reads nothing outside it.
pub fn brief(window: &Window, his: &[String]) -> String {
    let mut out = String::from("THE WINDOW. Calls, oldest first, one line each:\n");
    let first = window.number.saturating_sub(window.calls.len() as u64) + 1;
    for (i, call) in window.calls.iter().enumerate() {
        let text: String = call.text.chars().take(240).collect::<String>().replace('\n', " ⏎ ");
        out.push_str(&format!("#{} {} {}\n", first + i as u64, call.tool, text));
    }
    out.push_str("\nWHAT HE LAST TYPED, oldest first (empty when the transcript could not be read):\n");
    if his.is_empty() {
        out.push_str("(nothing available)\n");
    }
    for message in his {
        out.push_str(&format!("> {}\n", message.chars().take(HIS_CHARS).collect::<String>().replace('\n', " ")));
    }
    out.push_str("\nAnswer with exactly one line: VERDICT: CLEAR, or VERDICT: HALT <1-5>: <correction>.\n");
    out
}

/// Is a judge due for this session now? Every [`EVERY`]th call, and none already running.
pub fn due(home: &Path, session: &str, number: u64) -> bool {
    if session.is_empty() || number == 0 || number % EVERY != 0 {
        return false;
    }
    match std::fs::read_to_string(file(home, session, "judging")).ok().and_then(|t| t.trim().parse::<u64>().ok()) {
        Some(started) => now().saturating_sub(started) >= IN_FLIGHT_SECS,
        None => true,
    }
}

/// Write the brief, mark the judge in flight, and start it detached. The call does not wait.
///
/// Returns whether a child was started; every failure is silent here because the call that made
/// it must not pay for it. `spawn` starts the child for a session: in production
/// [`spawn_judge`], in a test a recorder, so that no test starts a process.
pub fn start(home: &Path, session: &str, brief_text: &str, spawn: &dyn Fn(&str) -> bool) -> bool {
    let _ = std::fs::create_dir_all(dir(home));
    if std::fs::write(file(home, session, "brief"), brief_text).is_err() {
        return false;
    }
    let _ = std::fs::write(file(home, session, "judging"), now().to_string());
    if spawn(session) {
        return true;
    }
    // A child that did not start must not hold the marker for a window it will never judge.
    let _ = std::fs::remove_file(file(home, session, "judging"));
    false
}

/// Start this binary again as the detached judge for `session`.
pub fn spawn_judge(session: &str) -> bool {
    match std::env::current_exe() {
        Ok(exe) => spawn_detached(&exe, session),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn spawn_detached(exe: &Path, session: &str) -> bool {
    use std::process::{Command, Stdio};
    Command::new(exe)
        .arg(SUBCOMMAND)
        .arg(session)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(windows)]
fn spawn_detached(exe: &Path, session: &str) -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: the child must outlive the hook that made it.
    Command::new(exe)
        .arg(SUBCOMMAND)
        .arg(session)
        .creation_flags(0x0000_0008 | 0x0000_0200)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Record the verdict for the next call to find. A `Clear` removes any older halt.
pub fn write_verdict(home: &Path, session: &str, verdict: &Verdict) {
    let path = file(home, session, "verdict");
    match verdict {
        Verdict::Clear => {
            let _ = std::fs::remove_file(path);
        }
        Verdict::Halt { shape, correction } => {
            let _ = std::fs::create_dir_all(dir(home));
            let _ = std::fs::write(path, format!("{}\t{shape}\t{}\n", now(), correction.replace(['\n', '\t'], " ")));
        }
    }
}

/// Take the halt waiting for this session, if there is a fresh one. It is delivered once.
pub fn take_halt(home: &Path, session: &str) -> Option<(u8, String)> {
    if session.is_empty() {
        return None;
    }
    let path = file(home, session, "verdict");
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let mut parts = text.trim_end().splitn(3, '\t');
    let written: u64 = parts.next()?.parse().ok()?;
    if now().saturating_sub(written) >= VERDICT_TTL_SECS {
        return None;
    }
    let shape: u8 = parts.next()?.parse().ok()?;
    Some((shape, parts.next()?.to_string()))
}

/// Hire the observer seat through the resident and return the brain's answer, or `None` when
/// anything stands in the way: no seats directory, no seat file, the resident not answering, a
/// refused hire. The seat file is the one the Seats view edits, so its brain and its off switch
/// are honoured by the hire itself.
pub fn hire_through(config: &crate::config::HookConfig, home: &Path, session: &str, brief: &str) -> Option<String> {
    let seat_file = Path::new(config.seats_dir.as_deref()?).join("observer.md");
    if !seat_file.exists() {
        return None;
    }
    let cwd = dir(home).join("judge-cwd");
    std::fs::create_dir_all(&cwd).ok()?;
    let endpoint = crate::query::Endpoint {
        url: config.url.clone(),
        token: config.token.clone(),
        timeout: std::time::Duration::from_secs(IN_FLIGHT_SECS - 10),
    };
    let data = crate::query::call_tool(
        &endpoint,
        "hire",
        serde_json::json!({
            "hirer": "observer-hook",
            "seat": "observer",
            "seatFile": seat_file,
            "brief": brief,
            "workingDirectory": cwd,
            // The hire needs one gate to be a hire; this one reads nothing and judges nothing.
            "steps": [{ "tool": "health_check", "arguments": {} }],
        }),
    )
    .ok()?;
    let _ = session;
    data.get("answer").and_then(|a| a.as_str()).map(str::to_string)
}

/// [`hire_through`] from the detached child, which finds the home directory the way the hook does.
pub fn hire_as_child(config: &crate::config::HookConfig, session: &str, brief: &str) -> Option<String> {
    hire_through(config, &crate::pipeline::home_dir()?, session, brief)
}

/// Is this process being started as the detached judge? (`jawata-hook observer-judge <session>`)
pub fn is_judge_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(SUBCOMMAND) && args.len() >= 3
}

/// The child's work: read the brief, hire the observer seat through the resident, parse the
/// answer, write the verdict, clear the in-flight marker. Returns the process exit code; the
/// parent never sees it.
pub fn main(args: &[String], hire: &dyn Fn(&str, &str) -> Option<String>) -> i32 {
    let Some(home) = crate::pipeline::home_dir() else { return 0 };
    run(&args[0], &home, hire)
}

/// [`main`] against a given home directory, so a test never touches the real one.
pub fn run(session: &str, home: &Path, hire: &dyn Fn(&str, &str) -> Option<String>) -> i32 {
    let brief_text = std::fs::read_to_string(file(home, session, "brief")).unwrap_or_default();
    let verdict = if brief_text.is_empty() {
        None
    } else {
        hire(session, &brief_text).and_then(|answer| parse_verdict(&answer))
    };
    match verdict {
        Some(v) => write_verdict(home, session, &v),
        None => {
            if let Some(studio) = home.join(".claude").join("jawata-studio").to_str() {
                crate::observer::emit_signal(Path::new(studio), "observer-judge-no-verdict", session);
            }
        }
    }
    let _ = std::fs::remove_file(file(home, session, "judging"));
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::CallView;

    fn temp_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "judge-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_two_lines_of_the_contract_parse() {
        assert_eq!(parse_verdict("VERDICT: CLEAR"), Some(Verdict::Clear));
        assert_eq!(
            parse_verdict("some thinking\nVERDICT: HALT 1: You left his question open at #17."),
            Some(Verdict::Halt { shape: 1, correction: "You left his question open at #17.".into() })
        );
    }

    #[test]
    fn anything_else_is_not_a_verdict() {
        for answer in [
            "",
            "I think it is fine",
            "VERDICT: HALT",
            "VERDICT: HALT 9: out of range",
            "VERDICT: HALT 2:",
            "VERDICT: MAYBE",
            "HALT 1: no prefix",
        ] {
            assert_eq!(parse_verdict(answer), None, "{answer:?}");
        }
    }

    #[test]
    fn a_judge_is_due_every_eighth_call_and_not_while_one_is_running() {
        let home = temp_home();
        assert!(!due(&home, "s", 7));
        assert!(due(&home, "s", 8));
        assert!(due(&home, "s", 16));
        assert!(!due(&home, "", 8), "no session, no window to judge");
        assert!(!due(&home, "s", 0));
        std::fs::create_dir_all(dir(&home)).unwrap();
        std::fs::write(file(&home, "s", "judging"), now().to_string()).unwrap();
        assert!(!due(&home, "s", 8), "one judge at a time");
        std::fs::write(file(&home, "s", "judging"), (now() - IN_FLIGHT_SECS - 1).to_string()).unwrap();
        assert!(due(&home, "s", 8), "a judge presumed gone does not block the next");
    }

    #[test]
    fn a_halt_is_taken_once_and_a_clear_removes_it() {
        let home = temp_home();
        write_verdict(&home, "s", &Verdict::Halt { shape: 3, correction: "stop\tit".into() });
        assert_eq!(take_halt(&home, "s"), Some((3, "stop it".to_string())));
        assert_eq!(take_halt(&home, "s"), None, "delivered once");
        write_verdict(&home, "s", &Verdict::Halt { shape: 2, correction: "x".into() });
        write_verdict(&home, "s", &Verdict::Clear);
        assert_eq!(take_halt(&home, "s"), None);
    }

    #[test]
    fn a_stale_halt_is_dropped() {
        let home = temp_home();
        std::fs::create_dir_all(dir(&home)).unwrap();
        std::fs::write(
            file(&home, "s", "verdict"),
            format!("{}\t1\told news\n", now() - VERDICT_TTL_SECS - 1),
        )
        .unwrap();
        assert_eq!(take_halt(&home, "s"), None);
    }

    #[test]
    fn the_brief_numbers_the_calls_as_the_hook_counted_them() {
        let window = Window {
            number: 30,
            calls: vec![
                CallView { tool: "Bash".into(), text: "ls\npwd".into(), background: false },
                CallView { tool: "Edit".into(), text: "/x/y.rs".into(), background: false },
            ],
            ..Window::default()
        };
        let text = brief(&window, &["is this the right plan?".to_string()]);
        assert!(text.contains("#29 Bash ls ⏎ pwd"), "{text}");
        assert!(text.contains("#30 Edit /x/y.rs"), "{text}");
        assert!(text.contains("> is this the right plan?"), "{text}");
        assert!(brief(&window, &[]).contains("(nothing available)"));
    }

    #[test]
    fn the_child_writes_a_halt_a_clear_or_nothing_and_always_clears_the_marker() {
        let home = temp_home();
        let session = "child-session";
        std::fs::create_dir_all(dir(&home)).unwrap();
        let prepare = || {
            std::fs::write(file(&home, session, "brief"), "THE WINDOW").unwrap();
            std::fs::write(file(&home, session, "judging"), now().to_string()).unwrap();
        };

        prepare();
        run(session, &home, &|_, _| Some("VERDICT: HALT 4: you went around the refusal at #3".into()));
        assert_eq!(take_halt(&home, session).map(|(s, _)| s), Some(4));
        assert!(!file(&home, session, "judging").exists());

        prepare();
        run(session, &home, &|_, _| None);
        assert_eq!(take_halt(&home, session), None, "an unreadable run writes no verdict");
        assert!(!file(&home, session, "judging").exists(), "and still releases the marker");
    }
}
