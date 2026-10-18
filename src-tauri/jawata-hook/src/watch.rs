//! Sprint 28g S23 step O1 — the observer's first tier: mechanical rules over each call.
//!
//! His design, in his words: *"jawata injects and enforces tool usage and the observer will
//! watch if some agent still misbehaves and returns"*, and *"he can halt and bounce back but
//! not on every call. And the next call just reminds the worker."* This module is the part
//! that costs milliseconds: a pure function of the call and the last nineteen calls, no model,
//! no store, no network. A judgement that needs a conversation (a direct question left
//! unanswered, work re-homed on the agent's own authority) is Tier 2's and is NOT attempted
//! here — a pattern matcher that guesses at those would refuse honest work.
//!
//! WHAT A RULE MAY DO. [`judge`] answers [`Finding`] or nothing. A finding is a CLASS plus the
//! correction, addressed to the worker (today the host agent is the only worker). The caller
//! halts on the first finding of a class in a window and lets later ones through, so a rule
//! that is wrong costs one refusal that names how to proceed, not a wall.
//!
//! THE THREE CLASSES are the call-visible ones in the frozen replay
//! (`evidence/observer-replay-2026-10-17/marks.json`) that the guard does not already enforce:
//! - [`Class::ForegroundWait`] — a long wait runs in the background and is monitored;
//! - [`Class::RogueGate`] — the patch-streak gate is not part of the release process;
//! - [`Class::ScriptHabit`] — a `jawata-author:`/`jawata-fallback:` declaration is an
//!   exception; a window full of them is a habit.
//!
//! THERE IS NO "SHELL SEARCH OF JAVA" CLASS. The first build had one and the architect's watch
//! found it was a second copy of the guard's own denial (`guard::judge`), running in front of
//! it: everything it flagged the guard refuses anyway. The observer is a layer BEHIND the
//! enforcement, so it does not repeat it. The replay's marked shell search (B1) is the guard's
//! catch, and the report says so.
//!
//! THE WINDOW IS A LEDGER, NOT A FILE REWRITTEN IN PLACE. Hooks of sibling tool calls overlap,
//! so a load–modify–write file loses updates and can be read torn. Each call appends its own
//! lines (one `write` in append mode, which does not interleave at this size), and the window
//! is a fold of the tail on read. The outcome of a call (refused or let through) is a later
//! line, because the guard decides after the observer has recorded the call.
//!
//! NO FABRICATION. A call whose text cannot be read gets no finding: an instrument that halts
//! on what it cannot see turns every unreadable payload into a refusal.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::guard::AUTHOR_DECLARATION;

/// How many calls make one window: the call being judged and the nineteen before it.
pub const WINDOW: usize = 20;

/// Declarations in the window, besides the current call, that make a further one a habit.
pub const HABIT_THRESHOLD: usize = 4;

/// The marker a read-only declaration carries. The write form is [`AUTHOR_DECLARATION`].
const FALLBACK_DECLARATION: &str = "jawata-fallback:";

/// A wait this long (seconds) is not a foreground command.
const LONG_WAIT_SECONDS: u64 = 120;

/// The script he ruled out of the release process.
const STREAK_GATE: &str = "patch-streak-gate";

/// How much of a ledger's end is read to fold a window. Twenty calls of 400 characters fit many
/// times over.
const TAIL_BYTES: u64 = 64 * 1024;

/// A ledger longer than this is cut to its tail the next time a call is recorded.
const ROTATE_BYTES: u64 = 1024 * 1024;

/// The suffix of the one-use record that carries the reminder to the next call.
const REMINDER_SUFFIX: &str = "observer-reminder";

/// Is Tier 1 on? It always is outside a debug build; a test can switch it off to exercise a
/// guard rule that would otherwise be halted first by a command the test merely mentions.
pub fn enabled() -> bool {
    !(cfg!(debug_assertions) && std::env::var("JAWATA_TEST_WATCH_OFF").is_ok())
}

/// The kind of misbehaviour a rule recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    ForegroundWait,
    RogueGate,
    ScriptHabit,
}

impl Class {
    /// The stable name written to the signal log and the KPI.
    pub fn name(self) -> &'static str {
        match self {
            Class::ForegroundWait => "foreground-wait",
            Class::RogueGate => "rogue-gate",
            Class::ScriptHabit => "script-habit",
        }
    }

    /// The reminder that rides on the call after a halt: the rule, once, without the story.
    pub fn reminder(self) -> &'static str {
        match self {
            Class::ForegroundWait => "Long waits run with run_in_background and are monitored; do not block the foreground.",
            Class::RogueGate => "The patch-streak gate is not part of the release process; run what the process names and nothing else.",
            Class::ScriptHabit => "Java is read and changed through the jawata tools; a declaration is an exception, and the tool that failed is what you report.",
        }
    }
}

/// What the rules saw about one call. Built from a hook payload, or from a replay row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallView {
    /// The call's number in its session, as the ledger counted it (0 when not yet counted).
    pub n: u64,
    pub tool: String,
    /// The shell command for a Bash call; the path or the one-line argument otherwise.
    pub text: String,
    /// The call said it runs in the background.
    pub background: bool,
    /// The guard refused this call (known only for calls whose outcome is already recorded).
    pub refused: bool,
}

impl CallView {
    /// A call as the rules see it before it has a number or an outcome.
    pub fn new(tool: &str, text: &str, background: bool) -> CallView {
        CallView { n: 0, tool: tool.to_string(), text: text.to_string(), background, refused: false }
    }

    /// Does this call carry a `jawata-author:` or `jawata-fallback:` declaration?
    pub fn declares(&self) -> bool {
        self.text.contains(AUTHOR_DECLARATION) || self.text.contains(FALLBACK_DECLARATION)
    }

    fn is_shell(&self) -> bool {
        self.tool.eq_ignore_ascii_case("Bash")
    }
}

/// One rule's answer: the class and the correction addressed to the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub class: Class,
    pub correction: String,
}

/// Judge one call against the last calls. Pure: the same inputs give the same answer, and
/// nothing is read, written or asked. `window` is the calls BEFORE this one, oldest first;
/// only the last `WINDOW - 1` are looked at.
pub fn judge(window: &[CallView], call: &CallView) -> Option<Finding> {
    if call.text.trim().is_empty() {
        return None;
    }
    let recent = &window[window.len().saturating_sub(WINDOW - 1)..];
    foreground_wait(call).or_else(|| rogue_gate(call)).or_else(|| script_habit(recent, call))
}

fn found(class: Class, correction: String) -> Option<Finding> {
    Some(Finding { class, correction })
}

fn foreground_wait(call: &CallView) -> Option<Finding> {
    if !call.is_shell() || call.background || call.text.trim_end().ends_with('&') {
        return None;
    }
    let seconds = longest_wait_seconds(&call.text)?;
    found(
        Class::ForegroundWait,
        format!(
            "A WAIT OF {seconds}s OR MORE IS RUNNING IN THE FOREGROUND. His rule: \"Don't do in \
             foreground!\" — a CI watch or a gate runs with run_in_background and you are told \
             when it ends. Start it in the background, then carry on with work that does not \
             need its result, and read it when it reports back."
        ),
    )
}

/// A wait on something OUTSIDE the machine's own build: `gh run watch` (as long as the run
/// takes, so counted as an hour), `sleep N`, and a polling `until`/`while` loop with a sleep in
/// it. A bounded `timeout N mvn ...` is a build with a ceiling, not a wait, and the bracket rule
/// already polices how those run. Words are whole tokens, so `sleeps` and
/// `-Dsurefire.timeout=900` are not the command.
fn longest_wait_seconds(command: &str) -> Option<u64> {
    let mut longest = if command.contains("gh run watch") { 3600 } else { 0 };
    let tokens: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
        .filter(|t| !t.is_empty())
        .collect();
    for pair in tokens.windows(2) {
        if pair[0] == "sleep" {
            if let Ok(n) = pair[1].parse::<u64>() {
                longest = longest.max(n);
            }
        }
    }
    if tokens.iter().any(|t| matches!(*t, "until" | "while")) && tokens.contains(&"sleep") {
        longest = longest.max(LONG_WAIT_SECONDS);
    }
    (longest >= LONG_WAIT_SECONDS).then_some(longest)
}

fn rogue_gate(call: &CallView) -> Option<Finding> {
    if !call.text.contains(STREAK_GATE) {
        return None;
    }
    found(
        Class::RogueGate,
        "THE PATCH-STREAK GATE IS NOT IN THE RELEASE PROCESS. He ruled it out for ordinary \
         releases and the process he gave names no such step; running it on your own is the \
         shape he called going rogue. Run the steps the release process names. If you believe a \
         gate is missing, say so to him as a decision — do not run it."
            .to_string(),
    )
}

fn script_habit(recent: &[CallView], call: &CallView) -> Option<Finding> {
    if !call.declares() {
        return None;
    }
    let before = recent.iter().filter(|c| c.declares()).count();
    if before < HABIT_THRESHOLD {
        return None;
    }
    found(
        Class::ScriptHabit,
        format!(
            "{} DECLARED EXCEPTIONS IN THE LAST {WINDOW} CALLS, AND THIS IS ANOTHER. A \
             declaration is how you say a jawata tool could not do this one edit — not a way \
             to keep editing Java by script. His words: \"Then use jawata!\" Use the tool \
             that does it (extract, move, rename, change_method_signature). Where it truly \
             failed, report what it did once and ask.",
            before + 1
        ),
    )
}

// ---- the per-session ledger, kept between hook processes ----------------------------------

/// Where a session's ledger lives.
fn ledger_path(home: &Path, session: &str) -> PathBuf {
    home.join(".claude")
        .join("jawata-studio")
        .join("watch")
        .join(format!("{}.ledger", crate::pipeline::sanitize_session(session)))
}

/// What a ledger folds to: the running call number, the last halt per class, the call number the
/// judge was last handed a window at, and the last calls with their outcomes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub number: u64,
    pub halted: Vec<(String, u64)>,
    pub judged_at: u64,
    pub calls: Vec<CallView>,
}

impl Window {
    /// Was this class halted within the current window of calls?
    fn recently_halted(&self, class: Class) -> bool {
        self.halted
            .iter()
            .any(|(name, at)| name == class.name() && self.number.saturating_sub(*at) < WINDOW as u64)
    }

    /// Fold ledger text. Lines that do not parse are skipped: a torn tail is not a reason to lose
    /// the rest. Keeps the last `WINDOW - 1` calls.
    pub fn fold(text: &str) -> Window {
        let mut w = Window::default();
        for line in text.lines() {
            let parts: Vec<&str> = line.splitn(5, '\t').collect();
            match parts.as_slice() {
                ["c", n, tool, bg, body] => {
                    let n: u64 = n.parse().unwrap_or(0);
                    w.number = w.number.max(n);
                    w.calls.push(CallView {
                        n,
                        tool: unescape(tool),
                        background: *bg == "1",
                        text: unescape(body),
                        refused: false,
                    });
                }
                ["o", n, verdict] | ["o", n, verdict, ..] => {
                    let n: u64 = n.parse().unwrap_or(0);
                    if *verdict == "deny" {
                        if let Some(c) = w.calls.iter_mut().rev().find(|c| c.n == n) {
                            c.refused = true;
                        }
                    }
                }
                ["h", n, class] => {
                    let n: u64 = n.parse().unwrap_or(0);
                    w.number = w.number.max(n);
                    w.halted.retain(|(name, _)| name != class);
                    w.halted.push(((*class).to_string(), n));
                }
                ["j", n] => w.judged_at = w.judged_at.max(n.parse().unwrap_or(0)),
                _ => {}
            }
        }
        let keep = WINDOW - 1;
        if w.calls.len() > keep {
            let drop = w.calls.len() - keep;
            w.calls.drain(..drop);
        }
        w
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Read a session's window; a missing or unreadable ledger is an empty window.
pub fn load(home: &Path, session: &str) -> Window {
    if session.is_empty() {
        return Window::default();
    }
    match crate::pipeline::read_tail(&ledger_path(home, session).to_string_lossy(), TAIL_BYTES) {
        Ok(text) => {
            // A tail that starts mid-line has a partial first line; it does not parse and is skipped.
            Window::fold(&text)
        }
        Err(_) => Window::default(),
    }
}

/// Append lines to a session's ledger in one write, so lines from parallel hooks do not interleave.
fn append(home: &Path, session: &str, lines: &str) {
    if session.is_empty() {
        return;
    }
    let path = ledger_path(home, session);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false) {
        rotate(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(lines.as_bytes());
    }
}

/// Cut a long ledger to its tail through a temporary file and a rename, so a reader never sees
/// a truncated file. A call that appends between the read and the rename is lost; that costs one
/// line of a window that is being cut anyway.
fn rotate(path: &Path) {
    let Ok(text) = crate::pipeline::read_tail(&path.to_string_lossy(), TAIL_BYTES) else { return };
    let kept = match text.find('\n') {
        Some(at) => &text[at + 1..],
        None => "",
    };
    let tmp = path.with_extension(format!("ledger.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, kept).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The call being watched, so the guard's decision can be recorded against it afterwards.
#[derive(Debug)]
pub struct Ticket {
    home: PathBuf,
    session: String,
    pub number: u64,
}

/// The result of watching one call.
#[derive(Debug, PartialEq, Eq)]
pub enum Watched {
    /// Nothing to say.
    Pass,
    /// The first finding of this class in this window: refuse the call with the correction.
    Halt(Finding),
}

/// Watch one call: judge it, record it, and decide whether it halts.
///
/// A finding halts only the FIRST time its class appears in a window; the same class again
/// within the window passes, and the reminder rides on the next call instead. A session with
/// no id has no ledger to keep: it is judged against nothing and never halted twice.
pub fn begin(home: &Path, session: &str, mut call: CallView) -> (Ticket, Watched) {
    let window = load(home, session);
    let finding = judge(&window.calls, &call);
    let number = window.number + 1;
    call.n = number;
    let mut lines = format!(
        "c\t{number}\t{}\t{}\t{}\n",
        escape(&call.tool),
        u8::from(call.background),
        escape(&call.text.chars().take(400).collect::<String>())
    );
    let watched = match finding {
        Some(f) if !window.recently_halted(f.class) => {
            lines.push_str(&format!("h\t{number}\t{}\n", f.class.name()));
            set_reminder(home, session, f.class.reminder());
            Watched::Halt(f)
        }
        _ => Watched::Pass,
    };
    append(home, session, &lines);
    (Ticket { home: home.to_path_buf(), session: session.to_string(), number }, watched)
}

/// Record whether the guard refused the call. Called after the guard has decided.
pub fn end(ticket: &Ticket, refused: bool) {
    append(&ticket.home, &ticket.session, &format!("o\t{}\t{}\n", ticket.number, if refused { "deny" } else { "allow" }));
}

/// Record that the judge was handed the window as it stood at call `number`.
pub fn mark_judged(home: &Path, session: &str, number: u64) {
    append(home, session, &format!("j\t{number}\n"));
}

/// Leave a reminder for the next call to carry. One use, and it expires with the one-use records.
pub fn set_reminder(home: &Path, session: &str, text: &str) {
    if !session.is_empty() {
        crate::oneshot::open(home, session, REMINDER_SUFFIX, &[text]);
    }
}

/// Take the reminder waiting for this session, if any. It is delivered once.
pub fn take_reminder(home: &Path, session: &str) -> Option<String> {
    if session.is_empty() {
        return None;
    }
    crate::oneshot::take(home, session, REMINDER_SUFFIX, |_| true)?.into_iter().next()
}

// ---- the KPI (O3): counted, gating nothing ------------------------------------------------

/// The subcommand that prints the observer's counts.
pub const KPI_SUBCOMMAND: &str = "observer-kpi";

/// The subcommand that labels the most recent halt a catch or a false refusal.
pub const LABEL_SUBCOMMAND: &str = "observer-label";

/// What the observer has done, read from the signal log it already writes. Nothing here decides
/// anything. Whether a halt was a catch or a false refusal is a judgement only he can make, so a
/// halt is labelled by him (`observer-label catch|false`) and the count shows how many halts
/// carry a label and how many do not.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Kpi {
    /// First-tier halts by rule class.
    pub first_tier: std::collections::BTreeMap<String, u32>,
    /// Hired-observer halts by shape number.
    pub hired: std::collections::BTreeMap<String, u32>,
    /// Judge runs that ended without a readable verdict.
    pub no_verdict: u32,
    /// Halts labelled a catch.
    pub catches: u32,
    /// Halts labelled a false refusal.
    pub false_refusals: u32,
}

/// Count the observer's signals in a log of `time<TAB>version<TAB>signal<TAB>detail` lines,
/// optionally only those at or after `since` (an ISO time prefix compared as text).
pub fn read_kpi(log: &str, since: Option<&str>) -> Kpi {
    let mut kpi = Kpi::default();
    for line in log.lines() {
        let mut parts = line.splitn(4, '\t');
        let (Some(when), Some(_version), Some(signal)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if since.is_some_and(|s| when < s) {
            continue;
        }
        let detail = parts.next().unwrap_or("").trim().to_string();
        match signal {
            "observer-halt" => *kpi.first_tier.entry(detail).or_insert(0) += 1,
            "observer-judged-halt" => *kpi.hired.entry(detail).or_insert(0) += 1,
            "observer-judge-no-verdict" => kpi.no_verdict += 1,
            "observer-label" if detail == "catch" => kpi.catches += 1,
            "observer-label" if detail == "false" => kpi.false_refusals += 1,
            _ => {}
        }
    }
    kpi
}

impl Kpi {
    fn halts(&self) -> u32 {
        self.first_tier.values().sum::<u32>() + self.hired.values().sum::<u32>()
    }

    /// The text the subcommand prints.
    pub fn render(&self) -> String {
        let list = |m: &std::collections::BTreeMap<String, u32>| {
            if m.is_empty() {
                "none".to_string()
            } else {
                m.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
            }
        };
        let labelled = self.catches + self.false_refusals;
        format!(
            "observer halts, first tier, by rule: {}\nobserver halts, hired seat, by shape: {}\n\
             hired-seat runs with no readable verdict: {}\n\
             halts labelled by him: {} catches, {} false refusals; {} not yet labelled\n",
            list(&self.first_tier),
            list(&self.hired),
            self.no_verdict,
            self.catches,
            self.false_refusals,
            self.halts().saturating_sub(labelled)
        )
    }
}

/// Is this process being asked for the observer's counts?
pub fn is_kpi_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(KPI_SUBCOMMAND)
}

/// `observer-kpi [--since <iso>] [--log <path>]`. Returns the exit status. A log that cannot be
/// read is an error, not a zero: a missing log means nothing wrote here.
pub fn kpi_main(args: &[String]) -> i32 {
    crate::cadence::report_main("observer-kpi", args, &|text, since| read_kpi(text, since).render())
}

/// Is this process being asked to label a halt?
pub fn is_label_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(LABEL_SUBCOMMAND)
}

/// `observer-label catch|false` writes one `observer-label` signal. Anything else is refused.
pub fn label_main(args: &[String]) -> i32 {
    let Some(label) = args.first().map(String::as_str).filter(|l| matches!(*l, "catch" | "false")) else {
        eprintln!("observer-label: say `catch` or `false`");
        return 2;
    };
    let Some(home) = crate::pipeline::home_dir() else {
        eprintln!("observer-label: no home directory");
        return 2;
    };
    label_in(&home.join(".claude").join("jawata-studio"), label);
    0
}

/// Write one `observer-label` signal into a studio directory. Anything but `catch` or `false` is
/// refused and writes nothing.
pub fn label_in(studio: &Path, label: &str) -> bool {
    if !matches!(label, "catch" | "false") {
        return false;
    }
    crate::observer::emit_signal(studio, "observer-label", label);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(text: &str) -> CallView {
        CallView::new("Bash", text, false)
    }

    fn declared() -> CallView {
        bash("python3 -I - <<'EOF'\n# jawata-author: editing a file\nEOF")
    }

    fn home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "watch-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn watch(home: &Path, session: &str, call: CallView) -> Watched {
        begin(home, session, call).1
    }

    #[test]
    fn the_kpi_counts_each_signal_by_its_detail_and_honours_since() {
        let log = "2026-10-18T01:00:00Z\tv\tobserver-halt\tforeground-wait\n\
                   2026-10-18T02:00:00Z\tv\tobserver-halt\tforeground-wait\n\
                   2026-10-18T02:00:01Z\tv\tobserver-halt\trogue-gate\n\
                   2026-10-18T02:00:02Z\tv\tobserver-judged-halt\t2\n\
                   2026-10-18T02:00:03Z\tv\tobserver-judge-no-verdict\tsess\n\
                   2026-10-18T02:00:04Z\tv\tobserver-label\tcatch\n\
                   2026-10-18T02:00:05Z\tv\tobserver-label\tfalse\n\
                   2026-10-18T02:00:06Z\tv\tslip\tnot ours\n\
                   not a log line\n";
        let all = read_kpi(log, None);
        assert_eq!(all.first_tier.get("foreground-wait"), Some(&2));
        assert_eq!(all.first_tier.get("rogue-gate"), Some(&1));
        assert_eq!(all.hired.get("2"), Some(&1));
        assert_eq!((all.no_verdict, all.catches, all.false_refusals), (1, 1, 1));
        let later = read_kpi(log, Some("2026-10-18T01:30:00Z"));
        assert_eq!(later.first_tier.get("foreground-wait"), Some(&1));
        let text = all.render();
        assert!(text.contains("foreground-wait 2"), "{text}");
        assert!(text.contains("1 catches, 1 false refusals; 2 not yet labelled"), "four halts, two labels: {text}");
        assert!(read_kpi("", None).render().contains("none"));
    }

    #[test]
    fn a_missing_log_is_an_error_and_not_a_zero() {
        let missing = std::env::temp_dir().join("no-such-outcomes-log-for-the-kpi");
        assert_eq!(kpi_main(&["--log".to_string(), missing.to_string_lossy().into_owned()]), 2);
    }

    #[test]
    fn a_label_must_be_catch_or_false() {
        assert_eq!(label_main(&["maybe".to_string()]), 2);
        assert_eq!(label_main(&[]), 2);
    }

    #[test]
    fn a_label_is_written_and_the_count_reads_it_back() {
        let dir = home();
        assert!(label_in(&dir, "catch"));
        assert!(label_in(&dir, "false"));
        assert!(label_in(&dir, "false"));
        assert!(!label_in(&dir, "maybe"), "anything else writes nothing");
        let kpi = read_kpi(&std::fs::read_to_string(dir.join("outcomes.log")).unwrap(), None);
        assert_eq!((kpi.catches, kpi.false_refusals), (1, 2));
    }

    #[test]
    fn a_long_foreground_wait_is_a_finding() {
        let f = judge(&[], &bash("timeout 1500 gh run watch 123 --exit-status > /tmp/x 2>&1")).unwrap();
        assert_eq!(f.class, Class::ForegroundWait);
        assert!(f.correction.contains("run_in_background"));
    }

    #[test]
    fn the_same_wait_in_the_background_is_not() {
        let mut call = bash("gh run watch 123 --exit-status");
        call.background = true;
        assert_eq!(judge(&[], &call), None);
        assert_eq!(judge(&[], &bash("gh run watch 123 &")), None);
    }

    #[test]
    fn a_short_sleep_and_a_word_containing_timeout_are_not_waits() {
        assert_eq!(judge(&[], &bash("sleep 5; echo ok")), None);
        assert_eq!(judge(&[], &bash("echo timeouts 900")), None);
        assert_eq!(judge(&[], &bash("mvn -q test -Dsurefire.timeout=900")), None);
    }

    #[test]
    fn a_long_sleep_is_a_wait() {
        assert_eq!(judge(&[], &bash("sleep 600")).unwrap().class, Class::ForegroundWait);
    }

    #[test]
    fn a_bounded_build_with_a_timeout_is_not_a_wait_but_a_polling_loop_is() {
        assert_eq!(judge(&[], &bash("timeout 900 mvn -q -o install -DskipTests 2>&1 | tail -3")), None);
        assert_eq!(
            judge(&[], &bash("until gh run view 1 | grep -q completed; do sleep 20; done")).map(|f| f.class),
            Some(Class::ForegroundWait)
        );
    }

    #[test]
    fn running_the_streak_gate_is_a_finding_in_any_spelling() {
        for command in [
            "build/patch-streak-gate.sh v4.4.6 --explain",
            "sed -i 's|patch-streak-gate.sh$|x|' streak.sh",
        ] {
            assert_eq!(judge(&[], &bash(command)).unwrap().class, Class::RogueGate, "{command}");
        }
    }

    #[test]
    fn a_fifth_declaration_in_a_window_is_a_habit_and_a_fourth_is_not() {
        let four: Vec<CallView> = (0..4).map(|_| declared()).collect();
        assert_eq!(judge(&four[..3], &declared()), None, "the fourth declaration is still an exception");
        assert_eq!(judge(&four, &declared()).unwrap().class, Class::ScriptHabit);
    }

    #[test]
    fn declarations_older_than_the_window_do_not_count() {
        let mut calls: Vec<CallView> = (0..4).map(|_| declared()).collect();
        calls.extend((0..WINDOW).map(|_| bash("ls")));
        assert_eq!(judge(&calls, &declared()), None);
    }

    #[test]
    fn a_shell_search_of_java_is_the_guards_to_refuse_and_not_this_tiers() {
        assert_eq!(judge(&[], &bash("grep -rn \"Foo\" --include=Foo.java .")), None);
    }

    #[test]
    fn an_empty_call_is_judged_by_nothing() {
        assert_eq!(judge(&vec![declared(); 6], &bash("   ")), None);
    }

    #[test]
    fn the_first_finding_of_a_class_halts_and_the_second_in_the_window_passes() {
        let home = home();
        let first = watch(&home, "s1", bash("timeout 900 gh run watch 1"));
        assert!(matches!(first, Watched::Halt(ref f) if f.class == Class::ForegroundWait));
        assert_eq!(watch(&home, "s1", bash("timeout 900 gh run watch 2")), Watched::Pass);
    }

    #[test]
    fn the_class_halts_again_once_a_window_has_passed() {
        let home = home();
        assert!(matches!(watch(&home, "s1", bash("sleep 600")), Watched::Halt(_)));
        for _ in 0..WINDOW {
            watch(&home, "s1", bash("ls"));
        }
        assert!(matches!(watch(&home, "s1", bash("sleep 600")), Watched::Halt(_)));
    }

    #[test]
    fn a_halt_leaves_one_reminder_that_is_taken_once() {
        let home = home();
        watch(&home, "s1", bash("sleep 600"));
        let reminder = take_reminder(&home, "s1").expect("a halt leaves a reminder");
        assert!(reminder.contains("run_in_background"));
        assert_eq!(take_reminder(&home, "s1"), None);
    }

    #[test]
    fn sessions_do_not_share_a_window() {
        let home = home();
        for _ in 0..HABIT_THRESHOLD {
            watch(&home, "a", declared());
        }
        assert_eq!(watch(&home, "b", declared()), Watched::Pass);
        assert!(matches!(watch(&home, "a", declared()), Watched::Halt(ref f) if f.class == Class::ScriptHabit));
    }

    #[test]
    fn the_ledger_folds_back_to_the_window_with_numbers_outcomes_and_judge_marks() {
        let home = home();
        let (first, _) = begin(&home, "s1", bash("echo a\tb\nc"));
        end(&first, false);
        let (second, _) = begin(&home, "s1", CallView::new("Edit", "/x/y.rs", false));
        end(&second, true);
        mark_judged(&home, "s1", 2);
        let w = load(&home, "s1");
        assert_eq!((w.number, w.judged_at), (2, 2));
        assert_eq!(w.calls[0].text, "echo a\tb\nc");
        assert_eq!((w.calls[0].n, w.calls[0].refused), (1, false));
        assert_eq!((w.calls[1].n, w.calls[1].tool.as_str(), w.calls[1].refused), (2, "Edit", true));
    }

    #[test]
    fn a_torn_line_and_a_cut_head_cost_one_line_and_not_the_window() {
        let text = "ar line cut in half\nc\t1\tBash\t0\tls\nc\t2\tBash\t0\n\nc\t3\tBash\t0\tpwd\no\t3\tdeny\n";
        let w = Window::fold(text);
        assert_eq!(w.number, 3);
        assert_eq!(w.calls.iter().map(|c| c.n).collect::<Vec<_>>(), vec![1, 3]);
        assert!(w.calls[1].refused);
    }

    #[test]
    fn parallel_hooks_lose_no_line() {
        let home = home();
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let home = home.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        begin(&home, "p", bash(&format!("echo {t}-{i}")));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = std::fs::read_to_string(ledger_path(&home, "p")).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("c\t")).count(), 200, "every call kept its line");
        assert!(text.lines().all(|l| l.split('\t').count() >= 3), "no line was torn");
    }

    #[test]
    fn a_long_ledger_is_cut_to_its_tail_and_still_folds() {
        let home = home();
        let path = ledger_path(&home, "r");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let filler = format!("c\t1\tBash\t0\t{}\n", "x".repeat(300));
        std::fs::write(&path, filler.repeat((ROTATE_BYTES as usize / filler.len()) + 10)).unwrap();
        let before = std::fs::metadata(&path).unwrap().len();
        begin(&home, "r", bash("echo after"));
        let after = std::fs::metadata(&path).unwrap().len();
        assert!(after < before, "{after} < {before}");
        assert!(load(&home, "r").calls.last().unwrap().text == "echo after");
    }

    #[test]
    fn a_session_with_no_id_is_judged_and_never_stored() {
        let home = home();
        assert!(matches!(watch(&home, "", bash("sleep 600")), Watched::Halt(_)));
        assert!(matches!(watch(&home, "", bash("sleep 600")), Watched::Halt(_)));
        assert!(!home.join(".claude").exists());
    }
}
