//! Which brains this machine actually has — asked of each agent program itself, every time.
//!
//! Harald, 2026-10-12: *"We cannot fake this. We need to know what is availabe"* · *"No fake model
//! names!"* · *"The error should show the reason and prompt the user to login through bash"*.
//!
//! So nothing here is a list somebody typed. For each agent program the answer is read off the
//! program: is it installed (it answers `--version`), is it logged in (its own status command),
//! and which models does the account have (only where the program can list them — Grok can,
//! Claude Code, Codex and Copilot cannot, Cursor can but refused while measured). A program that
//! cannot answer a question is reported as unable to, never given a guessed answer.
//!
//! Every check is free: no session is started and nothing is billed. A brain that is not logged
//! in is reported with the program's own words and the exact command to run in a terminal —
//! jawata never handles a credential, and a headless run cannot show a login dialog.
//!
//! `jawata-duplicate:` the seat runner has its own process loop with a timeout (`runner.rs`), but
//! it streams a seat's events and enforces its ceilings; a one-line status probe needs neither,
//! and reaching into it would couple the seats view to the runner the 2026-09-13 ruling holds.

use serde::Serialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Whether the program says its account is logged in.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Login {
    LoggedIn,
    NotLoggedIn,
    /// The program has no way to say without starting a session, or said something unreadable.
    Unknown,
}

/// One agent program, as this machine has it right now.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrainStatus {
    pub id: String,
    pub label: String,
    pub installed: bool,
    /// The first line the program printed for `--version`.
    pub version: Option<String>,
    pub login: Login,
    /// The program's own words when it is not logged in, or why the check could not answer.
    pub reason: Option<String>,
    /// What to type in a terminal to log in. Shown with every failed login.
    pub login_command: String,
    /// The models the program names for itself — listed for the account (Grok, Cursor) or given in
    /// its own help (Claude Code). `None` where it names none — not the same as an empty list.
    pub models: Option<Vec<String>>,
    /// Whether a hire can start this brain at all (`HIRABLE`).
    pub hirable: bool,
    /// Whether a hire passes this brain a model (`TAKES_MODEL`); the others choose their own.
    pub takes_model: bool,
    /// Whether this brain's writes are held only by an instruction to the model
    /// (`HELD_BY_AN_INSTRUCTION`), so a seat choosing it runs with that bound and no other.
    pub writes_held_by_instruction: bool,
}

struct Spec {
    id: &'static str,
    label: &'static str,
    program: &'static str,
    login_command: &'static str,
}

/// The agent programs jawata knows how to ask. Their names, flags and login commands were read
/// off each program's own `--help` on 2026-10-12.
static SPECS: [Spec; 5] = [
    Spec { id: "claude-code", label: "Claude Code", program: "claude", login_command: "claude auth login" },
    Spec { id: "codex", label: "Codex", program: "codex", login_command: "codex login" },
    Spec { id: "cursor", label: "Cursor", program: "cursor-agent", login_command: "cursor-agent login" },
    Spec { id: "copilot", label: "Copilot", program: "copilot", login_command: "copilot login" },
    Spec { id: "grok", label: "Grok", program: "grok", login_command: "grok login" },
];

const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

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

/// The hirable brains a seat's model reaches: the engine passes `--model` to these two only
/// (Copilot routes its own model, Codex's is set in its own config).
pub const TAKES_MODEL: [&str; 2] = ["claude-code", "cursor"];

/// Runs one short command. `None` means the program could not be started at all — not installed.
fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    run_in(program, args, None, CHECK_TIMEOUT)
}

fn run_in(program: &str, args: &[&str], dir: Option<&std::path::Path>, timeout: Duration) -> Option<(bool, String)> {
    let mut command = Command::new(program);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let mut text = out_reader.join().unwrap_or_default();
    text.push_str(&err_reader.join().unwrap_or_default());
    Some((success, text))
}

fn first_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// `claude auth status` prints JSON carrying `"loggedIn": true|false`.
fn parse_claude_status(text: &str) -> (Login, Option<String>) {
    match serde_json::from_str::<serde_json::Value>(text).ok().and_then(|v| v["loggedIn"].as_bool()) {
        Some(true) => (Login::LoggedIn, None),
        Some(false) => (Login::NotLoggedIn, Some("Claude Code says it is not logged in.".into())),
        None => (Login::Unknown, first_line(text)),
    }
}

/// `codex login status` prints `Logged in using ChatGPT` (or another method) when it is.
fn parse_codex_status(text: &str) -> (Login, Option<String>) {
    if text.lines().any(|l| l.trim_start().starts_with("Logged in")) {
        (Login::LoggedIn, None)
    } else {
        (Login::NotLoggedIn, first_line(text))
    }
}

/// `cursor-agent models` lists the account's models, or refuses with
/// `Error: Authentication required. Run 'agent login', …` — measured: `status` claimed a login in
/// the same minute, so the model list is the check that reflects what a run would meet.
fn parse_cursor_models(success: bool, text: &str) -> (Login, Option<String>) {
    if success && !text.contains("Authentication required") {
        (Login::LoggedIn, None)
    } else {
        (Login::NotLoggedIn, first_line(text))
    }
}

/// `grok models` prints `You are logged in with …`, then one line per model, the default marked
/// `* … (default)` and the rest `- …`.
fn parse_grok_models(text: &str) -> (Login, Option<String>, Option<Vec<String>>) {
    if !text.contains("You are logged in") {
        return (Login::NotLoggedIn, first_line(text), None);
    }
    let models = text
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("* ").or_else(|| l.strip_prefix("- ")))
        .map(|m| m.trim_end_matches("(default)").trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    (Login::LoggedIn, None, Some(models))
}

/// The model names Claude Code's own `--help` gives for `--model` — the quoted names in that
/// option's description (measured: `'fable'`, `'opus'`, `'sonnet'`, `'claude-fable-5'`). Claude Code
/// has no command that lists the account's models, so these are the only names it vouches for;
/// a test run then shows which model actually answers.
fn parse_claude_model_names(help: &str) -> Vec<String> {
    let mut block = String::new();
    let mut inside = false;
    for line in help.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--model ") {
            inside = true;
        } else if inside && trimmed.starts_with('-') {
            break;
        }
        if inside {
            block.push_str(trimmed);
            block.push(' ');
        }
    }
    // A name is a quoted single word. Walking quote to quote rather than splitting on quotes
    // matters: the description says "a model's full name", and that apostrophe would otherwise
    // pair with the next quote and swallow the name after it.
    let chars: Vec<char> = block.chars().collect();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' {
            if let Some(len) = chars[i + 1..].iter().position(|c| *c == '\'') {
                let name: String = chars[i + 1..i + 1 + len].iter().collect();
                let is_word = !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]".contains(c));
                if is_word {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                    i += len + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    names
}

fn check(spec: &Spec) -> BrainStatus {
    let mut status = BrainStatus {
        id: spec.id.into(),
        label: spec.label.into(),
        installed: false,
        version: None,
        login: Login::Unknown,
        reason: None,
        login_command: spec.login_command.into(),
        models: None,
        hirable: HIRABLE.contains(&spec.id),
        takes_model: TAKES_MODEL.contains(&spec.id),
        writes_held_by_instruction: HELD_BY_AN_INSTRUCTION.contains(&spec.id),
    };
    let Some((_, version)) = run(spec.program, &["--version"]) else {
        status.reason = Some(format!("`{}` is not installed on this machine.", spec.program));
        return status;
    };
    status.installed = true;
    status.version = first_line(&version);
    let (login, reason, models) = match spec.id {
        "claude-code" => match run(spec.program, &["auth", "status"]) {
            Some((_, out)) => {
                let (l, r) = parse_claude_status(&out);
                let names = run(spec.program, &["--help"])
                    .map(|(_, help)| parse_claude_model_names(&help))
                    .filter(|n| !n.is_empty());
                (l, r, names)
            }
            None => (Login::Unknown, None, None),
        },
        "codex" => match run(spec.program, &["login", "status"]) {
            Some((_, out)) => {
                let (l, r) = parse_codex_status(&out);
                (l, r, None)
            }
            None => (Login::Unknown, None, None),
        },
        "cursor" => match run(spec.program, &["models"]) {
            Some((ok, out)) => {
                let (l, r) = parse_cursor_models(ok, &out);
                (l, r, None)
            }
            None => (Login::Unknown, None, None),
        },
        "grok" => match run(spec.program, &["models"]) {
            Some((_, out)) => parse_grok_models(&out),
            None => (Login::Unknown, None, None),
        },
        _ => (
            Login::Unknown,
            Some(format!("{} has no command that reports its login without starting a session.", spec.label)),
            None,
        ),
    };
    status.login = login;
    status.reason = reason;
    status.models = models;
    status
}

/// What one test run through a brain showed.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub brain: String,
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

const PROBE_PROMPT: &str = "Reply with the single word OK.";

/// Claude Code's JSON result: `result`, `is_error`, `subtype`, `total_cost_usd`, and `modelUsage`
/// keyed by every model the run touched.
fn parse_claude_probe(text: &str) -> (bool, Option<String>, Vec<String>, Option<f64>, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return (false, None, Vec::new(), None, first_line(text));
    };
    let answer = v["result"].as_str().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string);
    let is_error = v["is_error"].as_bool().unwrap_or(true);
    let mut models: Vec<(String, f64)> = v["modelUsage"]
        .as_object()
        .map(|m| m.iter().map(|(k, u)| (k.clone(), u["costUSD"].as_f64().unwrap_or(0.0))).collect())
        .unwrap_or_default();
    models.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let ok = !is_error && answer.is_some();
    let error = (!ok).then(|| {
        v["result"].as_str().filter(|r| !r.is_empty()).map(str::to_string)
            .or_else(|| v["subtype"].as_str().map(str::to_string))
            .unwrap_or_else(|| "Claude Code returned no answer".into())
    });
    (ok, answer, models.into_iter().map(|(m, _)| m).collect(), v["total_cost_usd"].as_f64(), error)
}

/// Codex's `--json` lines: the answer is an `item.completed` of type `agent_message`; a failure
/// arrives as an `error` or `turn.failed` event. Codex reports no model name.
fn parse_codex_probe(text: &str) -> (bool, Option<String>, Option<String>) {
    let mut answer = None;
    let mut error = None;
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
        match e["type"].as_str() {
            Some("item.completed") if e["item"]["type"] == "agent_message" => {
                answer = e["item"]["text"].as_str().map(|t| t.trim().to_string());
            }
            Some("error") | Some("turn.failed") => {
                error = e["message"].as_str().or_else(|| e["error"]["message"].as_str()).map(str::to_string);
            }
            _ => {}
        }
    }
    let ok = answer.is_some() && error.is_none();
    let error = (!ok).then(|| error.or_else(|| first_line(text)).unwrap_or_else(|| "Codex returned no answer".into()));
    (ok, answer, error)
}

/// Copilot's `--output-format json` lines: the answer and the model that gave it arrive together
/// on `assistant.message`.
fn parse_copilot_probe(text: &str) -> (bool, Option<String>, Vec<String>, Option<String>) {
    let mut answer = None;
    let mut model = None;
    let mut error = None;
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
        let kind = e["type"].as_str().unwrap_or("");
        if kind == "assistant.message" {
            answer = e["data"]["content"].as_str().map(|t| t.trim().to_string());
            model = e["data"]["model"].as_str().map(str::to_string);
        } else if kind.contains("error") {
            error = e["data"]["message"].as_str().or_else(|| e["data"]["error"].as_str()).map(str::to_string);
        }
    }
    let ok = answer.as_deref().is_some_and(|a| !a.is_empty()) && error.is_none();
    let error = (!ok).then(|| error.or_else(|| first_line(text)).unwrap_or_else(|| "Copilot returned no answer".into()));
    (ok, answer, model.into_iter().collect(), error)
}

/// Cursor's `--output-format json` result, or its refusal in plain words.
fn parse_cursor_probe(success: bool, text: &str) -> (bool, Option<String>, Option<String>) {
    if !success || text.contains("Authentication required") {
        return (false, None, first_line(text));
    }
    let answer = serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .and_then(|v| v["result"].as_str().map(|r| r.trim().to_string()))
        .or_else(|| first_line(text));
    (answer.is_some(), answer, None)
}

/// One test run: the brain answers one word, in an empty folder, as cheaply as each program
/// allows. Measured on 2026-10-12: Claude Code with every tool and setting stripped $0.007 in
/// 2.7 s (the same run WITHOUT `--tools ""` was $0.24, its tool definitions alone); Codex 4.9 s;
/// Copilot 9.2 s. Cursor refused with its login error. A run's cost lands on the account the
/// program is logged into.
pub fn probe(brain: &str, model: Option<&str>) -> Result<ProbeResult, String> {
    let spec = SPECS.iter().find(|s| s.id == brain).ok_or_else(|| format!("no brain named '{brain}'"))?;
    if !HIRABLE.contains(&brain) {
        return Err(format!("'{brain}' is not a brain a hire can start"));
    }
    let model = model.filter(|m| !m.trim().is_empty());
    if model.is_some() && !TAKES_MODEL.contains(&brain) {
        return Err(format!("{} chooses its own model; a hire cannot pass one to it", spec.label));
    }
    let dir = std::env::temp_dir().join(format!("jawata-brain-probe-{}-{}", brain, std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut args: Vec<String> = match brain {
        "claude-code" => vec![
            "-p", PROBE_PROMPT, "--output-format", "json", "--max-turns", "1",
            "--no-session-persistence", "--strict-mcp-config", "--setting-sources", "project",
            "--system-prompt", PROBE_PROMPT, "--tools", "",
        ].into_iter().map(String::from).collect(),
        "codex" => vec!["exec", "--json", "--sandbox", "read-only", "--skip-git-repo-check", PROBE_PROMPT]
            .into_iter().map(String::from).collect(),
        "cursor" => vec!["--print", "--output-format", "json", "--trust", "--mode", "plan"]
            .into_iter().map(String::from).collect(),
        _ => vec!["-p", PROBE_PROMPT, "--output-format", "json", "--disable-builtin-mcps"]
            .into_iter().map(String::from).collect(),
    };
    if let Some(m) = model {
        args.push("--model".into());
        args.push(m.to_string());
    }
    if brain == "cursor" {
        args.push(PROBE_PROMPT.into());
    }
    let started = Instant::now();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = run_in(spec.program, &argv, Some(&dir), PROBE_TIMEOUT);
    let seconds = started.elapsed().as_secs_f64();
    let _ = std::fs::remove_dir_all(&dir);
    let mut result = ProbeResult {
        brain: brain.into(),
        model: model.map(str::to_string),
        ok: false,
        answer: None,
        models_used: Vec::new(),
        cost_usd: None,
        seconds,
        error: None,
        login_command: spec.login_command.into(),
    };
    let Some((success, text)) = outcome else {
        result.error = Some(format!("`{}` is not installed on this machine.", spec.program));
        return Ok(result);
    };
    match brain {
        "claude-code" => {
            let (ok, answer, models, cost, error) = parse_claude_probe(&text);
            (result.ok, result.answer, result.models_used, result.cost_usd, result.error) = (ok, answer, models, cost, error);
        }
        "codex" => {
            let (ok, answer, error) = parse_codex_probe(&text);
            (result.ok, result.answer, result.error) = (ok, answer, error);
        }
        "cursor" => {
            let (ok, answer, error) = parse_cursor_probe(success, &text);
            (result.ok, result.answer, result.error) = (ok, answer, error);
        }
        _ => {
            let (ok, answer, models, error) = parse_copilot_probe(&text);
            (result.ok, result.answer, result.models_used, result.error) = (ok, answer, models, error);
        }
    }
    Ok(result)
}

/// The named brains, checked in parallel — every known one for an empty list, so a caller never
/// keeps its own copy of the roster. An unknown id is skipped: it names no program to ask.
pub fn brains_status(ids: &[String]) -> Vec<BrainStatus> {
    let handles: Vec<_> = SPECS
        .iter()
        .filter(|s| ids.is_empty() || ids.iter().any(|id| id == s.id))
        .map(|s| std::thread::spawn(move || check(s)))
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture below is output measured on 2026-10-12 on the developer machine.

    #[test]
    fn claude_reports_its_login_as_json() {
        let measured = r#"{
  "loggedIn": true,
  "authMethod": "claude.ai",
  "apiProvider": "firstParty"
}"#;
        assert_eq!(parse_claude_status(measured), (Login::LoggedIn, None));
        let (login, reason) = parse_claude_status(r#"{"loggedIn": false}"#);
        assert_eq!(login, Login::NotLoggedIn);
        assert!(reason.is_some(), "a failed login always carries a reason to show");
        assert_eq!(parse_claude_status("garbage").0, Login::Unknown, "unreadable is not 'logged in'");
    }

    #[test]
    fn codex_says_logged_in_in_words() {
        assert_eq!(parse_codex_status("Logged in using ChatGPT\n"), (Login::LoggedIn, None));
        let (login, reason) = parse_codex_status("Not logged in\n");
        assert_eq!(login, Login::NotLoggedIn);
        assert_eq!(reason.as_deref(), Some("Not logged in"));
    }

    /// S16-39. This list is a COPY of what jawata-mcp's dialects answer through
    /// `BrainDialect#writeContainment()`, and nothing derives one from the other across the two
    /// repositories, so both sides pin the same fact. The engine's side is
    /// `TheBrainsHeldOnlyByAnInstructionAreTheOnesStudioListsTest`; if its answer changes on
    /// purpose, this value, the Seats view's label and the opt-in written into the seat file change
    /// with it.
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
    }

    #[test]
    fn cursors_refusal_is_a_failed_login_with_its_own_words() {
        let measured = "Error: Authentication required. Run 'agent login', pass --api-key/--auth-token, \
                        or set CURSOR_API_KEY/CURSOR_AUTH_TOKEN.\n";
        let (login, reason) = parse_cursor_models(false, measured);
        assert_eq!(login, Login::NotLoggedIn);
        assert!(reason.unwrap().contains("Authentication required"), "the program's words are the reason");
        // Even a zero exit is not a login when the program says authentication is required.
        assert_eq!(parse_cursor_models(true, measured).0, Login::NotLoggedIn);
    }

    #[test]
    fn grok_lists_the_accounts_models_and_marks_none_as_invented() {
        let measured = "You are logged in with grok.com.\n\nDefault model: grok-4.7\n\nAvailable models:\n  \
                        * grok-4.7 (default)\n  - grok-4.7-build-fast\n  - grok-4.6\n  - grok-4.5\n";
        let (login, reason, models) = parse_grok_models(measured);
        assert_eq!((login, reason), (Login::LoggedIn, None));
        assert_eq!(
            models.unwrap(),
            vec!["grok-4.7", "grok-4.7-build-fast", "grok-4.6", "grok-4.5"],
            "exactly the models the program listed, in its order"
        );
        let (login, _, models) = parse_grok_models("Please sign in.\n");
        assert_eq!(login, Login::NotLoggedIn);
        assert_eq!(models, None, "no list is invented for an account that is not logged in");
    }

    #[test]
    fn a_program_that_is_not_installed_says_so_and_still_names_how_to_log_in() {
        let spec = Spec { id: "codex", label: "Codex", program: "jawata-no-such-program", login_command: "codex login" };
        let status = check(&spec);
        assert!(!status.installed);
        assert_eq!(status.login, Login::Unknown);
        assert!(status.reason.unwrap().contains("not installed"));
    }

    #[test]
    fn claudes_own_help_names_the_models_it_takes() {
        let measured = r#"  --model <model>                       Model for the current session. Provide
                                        an alias for the latest model (e.g.
                                        'fable', 'opus', or 'sonnet') or a
                                        model's full name (e.g.
                                        'claude-fable-5').
  -n, --name <name>                     Set a display name for this session
"#;
        assert_eq!(parse_claude_model_names(measured), vec!["fable", "opus", "sonnet", "claude-fable-5"]);
        assert!(parse_claude_model_names("no such option here").is_empty());
    }

    #[test]
    fn a_claude_test_run_reports_the_model_that_answered_and_its_cost() {
        let measured = r#"{"type":"result","subtype":"success","is_error":false,"result":"OK","total_cost_usd":0.007134,
            "modelUsage":{"claude-haiku-4-5-20251001":{"costUSD":0.000944},"claude-opus-5[1m]":{"costUSD":0.00619}}}"#;
        let (ok, answer, models, cost, error) = parse_claude_probe(measured);
        assert!(ok && error.is_none());
        assert_eq!(answer.as_deref(), Some("OK"));
        assert_eq!(models, vec!["claude-opus-5[1m]", "claude-haiku-4-5-20251001"], "most expensive first");
        assert_eq!(cost, Some(0.007134));
        // Measured too: the same run without `--tools ""` went over its cap and answered nothing.
        let over = r#"{"type":"result","subtype":"error_max_budget_usd","is_error":true,"result":null,"total_cost_usd":0.24}"#;
        let (ok, _, _, _, error) = parse_claude_probe(over);
        assert!(!ok);
        assert_eq!(error.as_deref(), Some("error_max_budget_usd"));
    }

    #[test]
    fn a_codex_test_run_answers_without_naming_its_model() {
        let measured = "{\"type\":\"thread.started\"}\n{\"type\":\"turn.started\"}\n\
            {\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"OK\"}}\n\
            {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":13212,\"output_tokens\":5}}\n";
        assert_eq!(parse_codex_probe(measured), (true, Some("OK".into()), None));
        let (ok, _, error) = parse_codex_probe("{\"type\":\"error\",\"message\":\"Not logged in\"}\n");
        assert!(!ok);
        assert_eq!(error.as_deref(), Some("Not logged in"));
    }

    #[test]
    fn a_copilot_test_run_names_the_model_it_chose() {
        let measured = "{\"type\":\"user.message\",\"data\":{\"content\":\"Reply with the single word OK.\"}}\n\
            {\"type\":\"assistant.message\",\"data\":{\"model\":\"gpt-6-luna\",\"content\":\"OK\"}}\n\
            {\"type\":\"result\",\"data\":{}}\n";
        assert_eq!(parse_copilot_probe(measured), (true, Some("OK".into()), vec!["gpt-6-luna".to_string()], None));
    }

    #[test]
    fn a_cursor_test_run_that_needs_a_login_fails_with_cursors_words() {
        let measured = "Error: Authentication required. Run 'agent login', pass --api-key/--auth-token, \
                        or set CURSOR_API_KEY/CURSOR_AUTH_TOKEN.\n";
        let (ok, answer, error) = parse_cursor_probe(false, measured);
        assert!(!ok && answer.is_none());
        assert!(error.unwrap().contains("Authentication required"));
    }

    #[test]
    fn a_test_run_refuses_a_brain_the_hire_cannot_start_and_a_model_the_brain_cannot_take() {
        assert!(probe("grok", None).is_err(), "Grok is installed here, but no hire can start it");
        assert!(probe("codex", Some("gpt-5")).is_err(), "the hire passes Codex no model");
    }

    /// One REAL test run on this machine, through the same code the view calls, for the brain and
    /// model in `JW_PROBE_BRAIN` / `JW_PROBE_MODEL`. Spends a little of that brain's account:
    /// `JW_PROBE_BRAIN=claude-code cargo test --lib brains::tests::probe_from_env -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_from_env() {
        let brain = std::env::var("JW_PROBE_BRAIN").expect("set JW_PROBE_BRAIN");
        let model = std::env::var("JW_PROBE_MODEL").ok().filter(|m| !m.is_empty());
        let result = probe(&brain, model.as_deref()).unwrap();
        println!("PROBE-JSON {}", serde_json::to_string(&result).unwrap());
    }

    /// Prints what THIS machine has, as the view would receive it. Run on demand:
    /// `cargo test --lib brains::tests::print_this_machine -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_this_machine() {
        let ids: Vec<String> = SPECS.iter().map(|s| s.id.to_string()).collect();
        println!("BRAINS-JSON {}", serde_json::to_string(&brains_status(&ids)).unwrap());
    }
}
