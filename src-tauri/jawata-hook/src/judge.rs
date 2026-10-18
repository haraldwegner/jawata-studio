//! Sprint 28g S23 step O2 — the observer's second tier: a hired seat that judges a window.
//!
//! His design, in his words: *"The observer must be fast but not superfast in the pipeline. So
//! he can halt and bounce back but not on every call."* The first tier ([`crate::watch`]) holds
//! every case a pattern can decide. What a pattern cannot decide — a question left waiting, work
//! moved on the agent's own authority — is a judgement over a short window, and a judgement is a
//! hired seat (`seats/observer.md`, the seat file being the hire).
//!
//! **NEVER AWAITED BY A CALL.** A hook process lives for milliseconds and a seat takes seconds.
//! So the ordinary call does the cheap half only: when a judge is due it writes the window as a
//! brief, leaves a marker, and starts a DETACHED child of this same binary (`observer-judge`)
//! that runs the hire and writes the verdict. The call returns at once whether or not the child
//! starts. The NEXT call reads the verdict: a `HALT` refuses that one call with the correction,
//! once; a `CLEAR` leaves nothing.
//!
//! **WHEN IT CANNOT RUN IT SAYS NOTHING.** No seats directory in the config, no seat file, a seat
//! switched off, a resident that does not answer, an answer that is neither line of the contract:
//! no verdict is written and the first tier stands alone. An observer that halts on what it could
//! not read turns every outage into a refusal. A failed run is recorded as a signal, not hidden.
//!
//! The marker and the verdict are the one-use records the crate already has ([`crate::oneshot`]),
//! given the lifetime each means; the brief is a plain file with one writer.

use std::path::{Path, PathBuf};

use crate::watch::{CallView, Window};

/// A window is handed to the seat once this many calls have passed since the last time.
pub const EVERY: u64 = 8;

/// A judge started this long ago and not finished is presumed gone; another may start.
pub const IN_FLIGHT_SECS: u64 = 150;

/// A verdict older than this is stale: the window it judged is no longer the recent past.
pub const VERDICT_TTL_SECS: u64 = 300;

/// The subcommand the detached child is started with.
pub const SUBCOMMAND: &str = "observer-judge";

/// How many of his last messages ride in the brief, and how much of each.
pub const HIS_MESSAGES: usize = 3;
const HIS_CHARS: usize = 600;

const JUDGING: &str = "observer-judging";
const VERDICT: &str = "observer-verdict";

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

/// The rule behind each shape, one line, for the reminder the call after a halt carries.
pub fn shape_reminder(shape: u8) -> &'static str {
    match shape {
        1 => "A direct question of his is answered before work goes on.",
        2 => "Findings are his to home: do not move, defer or re-home one yourself; ask him.",
        3 => "Run the steps the process names, in its order; do not add one, skip one or run ahead of its word.",
        4 => "A refusal is answered, not routed around: take up what it said before trying again.",
        _ => "A declared exception covers one case; the tool that failed is what you report.",
    }
}

fn dir(home: &Path) -> PathBuf {
    home.join(".claude").join("jawata-studio").join("watch")
}

fn brief_path(home: &Path, session: &str) -> PathBuf {
    dir(home).join(format!("{}.brief", crate::pipeline::sanitize_session(session)))
}

/// Words that, followed by `=` or `:`, mean the next word is a secret.
const SECRET_NAMES: [&str; 7] = ["token", "password", "passwd", "secret", "apikey", "api_key", "authorization"];

/// Prefixes of well-known credential formats.
const SECRET_PREFIXES: [&str; 8] = ["ghp_", "gho_", "ghs_", "github_pat_", "sk-", "xoxb-", "xoxp-", "AKIA"];

/// Shortest run of letters and digits that is taken for a credential on its own.
const SECRET_RUN: usize = 32;

/// Replace what looks like a credential with `[redacted]` before text leaves the machine for a hired
/// brain. The window carries shell commands, and a command can carry a token. This is the
/// STRICTER default: it only ever removes, so it needs no one's word, and what it cannot recognise
/// it cannot remove — it is a net with known holes (a secret in a form none of these patterns
/// names), not a promise. It acts on whole words made of letters, digits, `_`, `+`, `=` and `-`
/// (a path or a URL breaks into short pieces and is left alone).
pub fn redact(text: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '=' | '-');
    let mut out = String::with_capacity(text.len());
    let mut redact_next = false;
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
        if end == 0 {
            let Some(c) = rest.chars().next() else { break };
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let word = &rest[..end];
        let bare = word.trim_end_matches('=');
        let lower = bare.to_ascii_lowercase();
        let looks_secret = redact_next
            || SECRET_PREFIXES.iter().any(|p| word.starts_with(p) && word.len() >= 12)
            || (word.len() >= SECRET_RUN
                && word.chars().any(|c| c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_alphabetic()));
        // `NAME=value` in one word: the name says the value is a secret.
        let inline = word.find('=').filter(|&at| {
            at + 1 < word.len() && SECRET_NAMES.iter().any(|n| word[..at].to_ascii_lowercase().ends_with(n))
        });
        if let (Some(at), false) = (inline, looks_secret) {
            out.push_str(&word[..=at]);
            out.push_str("[redacted]");
            redact_next = false;
        } else if looks_secret {
            out.push_str("[redacted]");
            redact_next = false;
        } else {
            out.push_str(word);
            let after = &rest[end..];
            let separator = after.starts_with(|c: char| matches!(c, '=' | ':')) || word.ends_with('=');
            redact_next = lower == "bearer"
                || (SECRET_NAMES.iter().any(|n| lower.ends_with(n)) && separator);
        }
        rest = &rest[end..];
        // A separator and the quote or space after it keep the secret that follows pending.
        if redact_next {
            let skip = rest.find(|c: char| !matches!(c, '=' | ':' | ' ' | '"' | '\'')).unwrap_or(rest.len());
            out.push_str(&rest[..skip]);
            rest = &rest[skip..];
        }
    }
    out
}

/// One call as the brief shows it: its number, the tool, whether it was refused, what it did.
fn brief_line(c: &CallView) -> String {
    let text: String = redact(&c.text).chars().take(240).collect::<String>().replace('\n', " ⏎ ");
    let refused = if c.refused { " [REFUSED]" } else { "" };
    format!("#{} {}{} {}\n", c.n, c.tool, refused, text)
}

/// The brief the seat is shown: the window's calls numbered as the hook numbered them, those the
/// guard refused marked as refused, and what he last typed. Plain text, because the seat has no
/// tools and reads nothing outside it.
pub fn brief(window: &Window, his: &[String]) -> String {
    let mut out = String::from("THE WINDOW. Calls, oldest first, one line each:\n");
    for call in &window.calls {
        out.push_str(&brief_line(call));
    }
    out.push_str("\nWHAT HE LAST TYPED, oldest first (empty when the transcript could not be read):\n");
    if his.is_empty() {
        out.push_str("(nothing available)\n");
    }
    for message in his {
        out.push_str(&format!("> {}\n", redact(message).chars().take(HIS_CHARS).collect::<String>().replace('\n', " ")));
    }
    out.push_str("\nAnswer with exactly one line: VERDICT: CLEAR, or VERDICT: HALT <1-5>: <correction>.\n");
    out
}

/// Is a judge due for this session now? Once [`EVERY`] calls have passed since the window was last
/// handed over (so a call that was refused or delivered a verdict does not skip the turn), and none
/// is already running.
pub fn due(home: &Path, session: &str, window: &Window) -> bool {
    if session.is_empty() || window.number == 0 {
        return false;
    }
    if window.number.saturating_sub(window.judged_at) < EVERY {
        return false;
    }
    crate::oneshot::peek_within(home, session, JUDGING, IN_FLIGHT_SECS).is_none()
}

/// Write the brief, mark the judge in flight, note the hand-over, and start it detached. The call
/// does not wait.
///
/// Returns whether a child was started; every failure is silent here because the call that made
/// it must not pay for it. `spawn` starts the child for a session: in production
/// [`spawn_judge`], in a test a recorder, so that no test starts a process.
pub fn start(home: &Path, session: &str, number: u64, brief_text: &str, spawn: &dyn Fn(&str) -> bool) -> bool {
    let _ = std::fs::create_dir_all(dir(home));
    if std::fs::write(brief_path(home, session), brief_text).is_err() {
        return false;
    }
    crate::oneshot::open(home, session, JUDGING, &["running"]);
    crate::watch::mark_judged(home, session, number);
    if spawn(session) {
        return true;
    }
    // A child that did not start must not hold the marker for a window it will never judge.
    crate::oneshot::discard(home, session, JUDGING);
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
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    // Its own process group, so a client that kills the hook's group on a timeout does not take the
    // judge with it.
    Command::new(exe)
        .arg(SUBCOMMAND)
        .arg(session)
        .process_group(0)
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
    match verdict {
        Verdict::Clear => crate::oneshot::discard(home, session, VERDICT),
        Verdict::Halt { shape, correction } => {
            crate::oneshot::open(home, session, VERDICT, &[&shape.to_string(), correction]);
        }
    }
}

/// Take the halt waiting for this session, if there is a fresh one. It is delivered once.
pub fn take_halt(home: &Path, session: &str) -> Option<(u8, String)> {
    if session.is_empty() {
        return None;
    }
    let mut fields = crate::oneshot::take_within(home, session, VERDICT, VERDICT_TTL_SECS, |_| true)?.into_iter();
    let shape: u8 = fields.next()?.parse().ok()?;
    Some((shape, fields.next()?))
}

/// Is the seat file switched off in the Seats view? The view writes `enabled: false` into the
/// seat's own frontmatter (between the first two `---` lines), which is what the hire reads; the
/// same words in the body are prose and do not count.
pub fn seat_is_off(seat_file_text: &str) -> bool {
    let mut lines = seat_file_text.lines();
    if lines.next() != Some("---") {
        return false;
    }
    lines.take_while(|l| *l != "---").any(|l| l.trim() == "enabled: false")
}

/// Hire the observer seat through the resident and return the brain's answer, or `None` when
/// anything stands in the way: no seats directory, no seat file, the resident not answering, a
/// refused hire. The seat file is the one the Seats view edits, so its brain and its off switch
/// are honoured by the hire itself.
pub fn hire_through(config: &crate::config::HookConfig, home: &Path, brief: &str) -> Option<String> {
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
    data.get("answer").and_then(|a| a.as_str()).map(str::to_string)
}

/// [`hire_through`] from the detached child, which finds the home directory the way the hook does.
pub fn hire_as_child(config: &crate::config::HookConfig, brief: &str) -> Option<String> {
    hire_through(config, &crate::pipeline::home_dir()?, brief)
}

/// Is this process being started as the detached judge? (`jawata-hook observer-judge <session>`)
pub fn is_judge_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(SUBCOMMAND) && args.len() >= 3
}

/// The child's work: read the brief, hire the observer seat through the resident, parse the
/// answer, write the verdict, clear the in-flight marker. Returns the process exit code; the
/// parent never sees it.
pub fn main(args: &[String], hire: &dyn Fn(&str) -> Option<String>) -> i32 {
    let Some(home) = crate::pipeline::home_dir() else { return 0 };
    run(&args[0], &home, hire)
}

/// [`main`] against a given home directory, so a test never touches the real one.
pub fn run(session: &str, home: &Path, hire: &dyn Fn(&str) -> Option<String>) -> i32 {
    let brief_text = std::fs::read_to_string(brief_path(home, session)).unwrap_or_default();
    let verdict = if brief_text.is_empty() { None } else { hire(&brief_text).and_then(|answer| parse_verdict(&answer)) };
    match verdict {
        Some(v) => write_verdict(home, session, &v),
        None => crate::observer::emit_signal(&home.join(".claude").join("jawata-studio"), "observer-judge-no-verdict", session),
    }
    crate::oneshot::discard(home, session, JUDGING);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "judge-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn window_at(number: u64, judged_at: u64) -> Window {
        Window { number, judged_at, ..Window::default() }
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
    fn a_judge_is_due_once_eight_calls_have_passed_since_the_last_and_not_while_one_is_running() {
        let home = temp_home();
        assert!(!due(&home, "s", &window_at(7, 0)));
        assert!(due(&home, "s", &window_at(8, 0)));
        assert!(due(&home, "s", &window_at(9, 0)), "a call that skipped the turn does not push the judge to 16");
        assert!(!due(&home, "s", &window_at(16, 10)), "six since the last hand-over");
        assert!(due(&home, "s", &window_at(18, 10)));
        assert!(!due(&home, "", &window_at(8, 0)), "no session, no window to judge");
        assert!(!due(&home, "s", &window_at(0, 0)));
        crate::oneshot::open(&home, "s", JUDGING, &["running"]);
        assert!(!due(&home, "s", &window_at(8, 0)), "one judge at a time");
        crate::oneshot::discard(&home, "s", JUDGING);
        assert!(due(&home, "s", &window_at(8, 0)));
    }

    #[test]
    fn handing_a_window_over_writes_the_brief_marks_the_judge_and_notes_the_call_number() {
        let home = temp_home();
        let started = std::cell::Cell::new(0);
        assert!(start(&home, "s", 8, "THE WINDOW", &|_| {
            started.set(started.get() + 1);
            true
        }));
        assert_eq!(started.get(), 1);
        assert_eq!(std::fs::read_to_string(brief_path(&home, "s")).unwrap(), "THE WINDOW");
        assert!(crate::oneshot::peek_within(&home, "s", JUDGING, IN_FLIGHT_SECS).is_some());
        assert_eq!(crate::watch::load(&home, "s").judged_at, 8);
        // A child that does not start releases the marker.
        crate::oneshot::discard(&home, "s", JUDGING);
        assert!(!start(&home, "s", 16, "AGAIN", &|_| false));
        assert!(crate::oneshot::peek_within(&home, "s", JUDGING, IN_FLIGHT_SECS).is_none());
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
        let path = crate::editgate::window_path(&home, &format!("s.{VERDICT}"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - VERDICT_TTL_SECS - 1;
        std::fs::write(&path, format!("{old}\t1\told news\n")).unwrap();
        assert_eq!(take_halt(&home, "s"), None);
    }

    #[test]
    fn credentials_are_removed_from_what_leaves_for_the_hired_brain() {
        let ghp = format!("ghp_{}", "a1B2c3D4".repeat(5));
        let long = "9f8e7d6c5b4a39281706f5e4d3c2b1a0ffeeddcc";
        for (text, gone) in [
            (format!("curl -H \"Authorization: Bearer {long}\" https://x/y"), long.to_string()),
            (format!("git push https://{ghp}@github.com/o/r"), ghp.clone()),
            ("export API_TOKEN=abc123def456 && run".to_string(), "abc123def456".to_string()),
            ("mysql --password=hunter2hunter2 -u root".to_string(), "hunter2hunter2".to_string()),
            ("curl --header 'api_key: shortbutsecret' x".to_string(), "shortbutsecret".to_string()),
            (format!("echo {long}"), long.to_string()),
        ] {
            let red = redact(&text);
            assert!(!red.contains(&gone), "{text} -> {red}");
            assert!(red.contains("[redacted]"), "{red}");
        }
    }

    #[test]
    fn ordinary_commands_paths_and_words_are_left_alone() {
        for text in [
            "cd /home/harald/CursorProjects/jawata-mcp/org.jawata.mcp.tests/src/org/jawata/mcp/execution && ls -la",
            "gh run watch 38381348957 --exit-status",
            "git commit -q -m \"the token is read from the config\"",
            "sed -n '1,60p' src-tauri/jawata-hook/src/judge.rs",
        ] {
            assert_eq!(redact(text), text);
        }
    }

    #[test]
    fn the_brief_never_carries_a_credential_from_a_call_or_a_message() {
        let secret = "ghp_".to_string() + &"x9Y8".repeat(6);
        let window = Window {
            number: 1,
            calls: vec![CallView { n: 1, tool: "Bash".into(), text: format!("git push https://{secret}@host"), ..CallView::default() }],
            ..Window::default()
        };
        let text = brief(&window, &[format!("use token={secret} for it")]);
        assert!(!text.contains(&secret), "{text}");
    }

    #[test]
    fn every_shape_has_its_own_reminder() {
        let all: std::collections::BTreeSet<&str> = (1..=5).map(shape_reminder).collect();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn the_brief_numbers_the_calls_as_the_hook_counted_them_and_marks_the_refused() {
        let window = Window {
            number: 30,
            calls: vec![
                CallView { n: 29, tool: "Bash".into(), text: "ls\npwd".into(), background: false, refused: false },
                CallView { n: 30, tool: "Edit".into(), text: "/x/y.rs".into(), background: false, refused: true },
            ],
            ..Window::default()
        };
        let text = brief(&window, &["is this the right plan?".to_string()]);
        assert!(text.contains("#29 Bash ls ⏎ pwd"), "{text}");
        assert!(text.contains("#30 Edit [REFUSED] /x/y.rs"), "{text}");
        assert!(text.contains("> is this the right plan?"), "{text}");
        assert!(brief(&window, &[]).contains("(nothing available)"));
    }

    #[test]
    fn a_seat_switched_off_in_the_seats_view_is_seen_as_off() {
        assert!(seat_is_off("---\nname: observer\nenabled: false\n---\nbody"));
        assert!(!seat_is_off("---\nname: observer\n---\nbody"));
        assert!(!seat_is_off("---\nname: observer\n---\nthe body says\nenabled: false\nin prose"));
        assert!(!seat_is_off("no frontmatter\nenabled: false"));
    }

    #[test]
    fn the_child_writes_a_halt_a_clear_or_nothing_and_always_clears_the_marker() {
        let home = temp_home();
        let session = "child-session";
        std::fs::create_dir_all(dir(&home)).unwrap();
        let prepare = || {
            std::fs::write(brief_path(&home, session), "THE WINDOW").unwrap();
            crate::oneshot::open(&home, session, JUDGING, &["running"]);
        };

        prepare();
        run(session, &home, &|_| Some("VERDICT: HALT 4: you went around the refusal at #3".into()));
        assert_eq!(take_halt(&home, session).map(|(s, _)| s), Some(4));
        assert!(crate::oneshot::peek_within(&home, session, JUDGING, IN_FLIGHT_SECS).is_none());

        prepare();
        run(session, &home, &|_| None);
        assert_eq!(take_halt(&home, session), None, "an unreadable run writes no verdict");
        assert!(crate::oneshot::peek_within(&home, session, JUDGING, IN_FLIGHT_SECS).is_none(), "and still releases the marker");
        let log = std::fs::read_to_string(home.join(".claude").join("jawata-studio").join("outcomes.log")).unwrap();
        assert!(log.contains("observer-judge-no-verdict"), "{log}");
    }
}
