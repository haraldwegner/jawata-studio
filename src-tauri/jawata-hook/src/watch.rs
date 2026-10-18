//! Sprint 28g S23 step O1 — the observer's first tier: mechanical rules over each call.
//!
//! His design, in his words: *"jawata injects and enforces tool usage and the observer will
//! watch if some agent still misbehaves and returns"*, and *"he can halt and bounce back but
//! not on every call. And the next call just reminds the worker."* This module is the part
//! that costs microseconds: a pure function of the call and the last nineteen calls, no model,
//! no store, no network. A judgement that needs a conversation (a direct question left
//! unanswered, work re-homed on the agent's own authority) is Tier 2's and is NOT attempted
//! here — a pattern matcher that guesses at those would refuse honest work.
//!
//! WHAT A RULE MAY DO. [`judge`] answers [`Finding`] or nothing. A finding is a CLASS plus the
//! correction, addressed to the worker (today the host agent is the only worker). The caller
//! halts on the first finding of a class in a window and lets later ones through, so a rule
//! that is wrong costs one refusal that names how to proceed, not a wall.
//!
//! THE FOUR CLASSES are the call-visible ones in the frozen replay
//! (`evidence/observer-replay-2026-10-17/marks.json`), each stated as the rule he ruled:
//! - [`Class::ForegroundWait`] — a long wait runs in the background and is monitored;
//! - [`Class::RogueGate`] — the patch-streak gate is not part of the release process;
//! - [`Class::ScriptHabit`] — a `jawata-author:`/`jawata-fallback:` declaration is an
//!   exception; a window full of them is a habit;
//! - [`Class::ShellOnJava`] — a shell search or rewrite of `.java` carries a declaration.
//!
//! NO FABRICATION. A call whose text cannot be read gets no finding: an instrument that halts
//! on what it cannot see turns every unreadable payload into a refusal.

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
    ShellOnJava,
}

impl Class {
    /// The stable name written to the signal log and the KPI.
    pub fn name(self) -> &'static str {
        match self {
            Class::ForegroundWait => "foreground-wait",
            Class::RogueGate => "rogue-gate",
            Class::ScriptHabit => "script-habit",
            Class::ShellOnJava => "shell-on-java",
        }
    }

    /// The reminder that rides on the call after a halt: the rule, once, without the story.
    pub fn reminder(self) -> &'static str {
        match self {
            Class::ForegroundWait => "Long waits run with run_in_background and are monitored; do not block the foreground.",
            Class::RogueGate => "The patch-streak gate is not part of the release process; run what the process names and nothing else.",
            Class::ScriptHabit => "Java is read and changed through the jawata tools; a declaration is an exception, and the tool that failed is what you report.",
            Class::ShellOnJava => "Java questions go to the jawata tools; a shell search over .java needs a declared reason.",
        }
    }
}

/// What the rules saw about one call. Built from a hook payload, or from a replay row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallView {
    pub tool: String,
    /// The shell command for a Bash call; the path or the one-line argument otherwise.
    pub text: String,
    /// The call said it runs in the background.
    pub background: bool,
}

impl CallView {
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
    foreground_wait(call)
        .or_else(|| rogue_gate(call))
        .or_else(|| script_habit(recent, call))
        .or_else(|| shell_on_java(call))
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

/// The longest wait this command declares, in seconds, or `None` below the line.
///
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

fn shell_on_java(call: &CallView) -> Option<Finding> {
    if !call.is_shell() || call.declares() || !touches_java_source(&call.text) {
        return None;
    }
    found(
        Class::ShellOnJava,
        "A SHELL SEARCH OR READ OF JAVA, WITH NO DECLARED REASON. Java questions go to the \
         jawata tools (search_symbols, find_references, inspect, analyze). If jawata cannot \
         answer THIS one, say why in the command: `jawata-fallback: <why>`."
            .to_string(),
    )
}

/// The shell words that search or read a file.
const FILE_READERS: [&str; 9] = ["grep", "egrep", "fgrep", "rg", "sed", "awk", "find", "head", "tail"];

/// Does some command in this line SEARCH OR READ a `.java` file as an operand?
///
/// By segment, not by substring: `org ` contains `rg `, and `mvn ... | grep -E 'ERROR.*\.java'`
/// filters build output rather than reading source. A segment counts when its command word is a
/// file reader and one of its words IS a Java path or glob: it ends in `.java` and carries no
/// regex escape.
fn touches_java_source(text: &str) -> bool {
    text.split(|c| matches!(c, ';' | '|' | '&' | '\n')).any(|segment| {
        let mut words = segment.split_whitespace();
        let Some(command) = words.find(|w| !w.contains('=') && *w != "cd" && *w != "sudo") else {
            return false;
        };
        FILE_READERS.contains(&command) && words.any(is_java_operand)
    })
}

fn is_java_operand(word: &str) -> bool {
    let word = word.trim_matches(|c| matches!(c, '"' | '\'' | ','));
    word.ends_with(".java") && !word.contains('\\') && !word.contains('$')
}

// ---- the per-session window, kept between hook processes -------------------------------

/// Where a session's window lives.
fn state_path(home: &Path, session: &str) -> PathBuf {
    home.join(".claude")
        .join("jawata-studio")
        .join("watch")
        .join(format!("{}.window", crate::pipeline::sanitize_session(session)))
}

/// What the file holds: the running call number, the last halt per class, the pending
/// reminder, and the last calls.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub number: u64,
    pub halted: Vec<(String, u64)>,
    pub reminder: Option<String>,
    pub calls: Vec<CallView>,
}

impl Window {
    /// The window as the rules read it.
    fn views(&self) -> &[CallView] {
        &self.calls
    }

    /// Was this class halted within the current window of calls?
    fn recently_halted(&self, class: Class) -> bool {
        self.halted
            .iter()
            .any(|(name, at)| name == class.name() && self.number.saturating_sub(*at) < WINDOW as u64)
    }

    fn render(&self) -> String {
        let mut out = format!("n\t{}\n", self.number);
        for (name, at) in &self.halted {
            out.push_str(&format!("h\t{name}\t{at}\n"));
        }
        if let Some(r) = &self.reminder {
            out.push_str(&format!("r\t{}\n", escape(r)));
        }
        for c in &self.calls {
            out.push_str(&format!(
                "c\t{}\t{}\t{}\n",
                escape(&c.tool),
                u8::from(c.background),
                escape(&c.text.chars().take(400).collect::<String>())
            ));
        }
        out
    }

    fn parse(text: &str) -> Window {
        let mut w = Window::default();
        for line in text.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            match parts.as_slice() {
                ["n", n] => w.number = n.parse().unwrap_or(0),
                ["h", name, at] => w.halted.push(((*name).to_string(), at.parse().unwrap_or(0))),
                ["r", r] => w.reminder = Some(unescape(r)),
                ["c", tool, bg, text] => w.calls.push(CallView {
                    tool: unescape(tool),
                    background: *bg == "1",
                    text: unescape(text),
                }),
                _ => {}
            }
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

/// Read a session's window; a missing or unreadable file is an empty window.
pub fn load(home: &Path, session: &str) -> Window {
    std::fs::read_to_string(state_path(home, session))
        .map(|t| Window::parse(&t))
        .unwrap_or_default()
}

fn save(home: &Path, session: &str, window: &Window) {
    let path = state_path(home, session);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, window.render());
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
/// no id has no window to keep, so it is judged against nothing and never halted twice.
pub fn watch(home: &Path, session: &str, call: CallView) -> Watched {
    let mut window = if session.is_empty() { Window::default() } else { load(home, session) };
    let finding = judge(window.views(), &call);
    window.number += 1;
    window.calls.push(call);
    let keep = WINDOW - 1;
    if window.calls.len() > keep {
        let drop = window.calls.len() - keep;
        window.calls.drain(..drop);
    }
    let outcome = match finding {
        Some(f) if !window.recently_halted(f.class) => {
            window.halted.retain(|(name, _)| name != f.class.name());
            window.halted.push((f.class.name().to_string(), window.number));
            window.reminder = Some(f.class.reminder().to_string());
            Watched::Halt(f)
        }
        _ => Watched::Pass,
    };
    if !session.is_empty() {
        save(home, session, &window);
    }
    outcome
}

/// Take the reminder waiting for this session, if any. It is delivered once.
pub fn take_reminder(home: &Path, session: &str) -> Option<String> {
    if session.is_empty() {
        return None;
    }
    let mut window = load(home, session);
    let reminder = window.reminder.take()?;
    save(home, session, &window);
    Some(reminder)
}

// ---- the KPI (O3): counted, gating nothing ------------------------------------------------

/// The subcommand that prints the observer's counts.
pub const KPI_SUBCOMMAND: &str = "observer-kpi";

/// What the observer has done, read from the signal log it already writes. Nothing here decides
/// anything: a halt is counted when it happens and the count is read on request. Whether a halt
/// was a catch or a false refusal is a judgement only he can make, so it is NOT in the log and is
/// not guessed here; the report says so.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Kpi {
    /// First-tier halts by rule class.
    pub first_tier: std::collections::BTreeMap<String, u32>,
    /// Hired-observer halts by shape number.
    pub hired: std::collections::BTreeMap<String, u32>,
    /// Judge runs that ended without a readable verdict.
    pub no_verdict: u32,
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
            _ => {}
        }
    }
    kpi
}

impl Kpi {
    /// The text the subcommand prints.
    pub fn render(&self) -> String {
        let list = |m: &std::collections::BTreeMap<String, u32>| {
            if m.is_empty() {
                "none".to_string()
            } else {
                m.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
            }
        };
        format!(
            "observer halts, first tier, by rule: {}\nobserver halts, hired seat, by shape: {}\n\
             hired-seat runs with no readable verdict: {}\n\
             (Whether a halt was a catch or a false refusal is his to say; the log cannot know.)\n",
            list(&self.first_tier),
            list(&self.hired),
            self.no_verdict
        )
    }
}

/// Is this process being asked for the observer's counts?
pub fn is_kpi_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(KPI_SUBCOMMAND)
}

/// The whole subcommand: `observer-kpi [--since <iso>] [--log <path>]`. Returns the exit status.
/// A log that cannot be read is an error, not a zero: a missing log means nothing wrote here.
pub fn kpi_main(args: &[String]) -> i32 {
    let (mut since, mut log) = (None::<String>, None::<PathBuf>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--since" => since = it.next().cloned(),
            "--log" => log = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    let Some(path) = log.or_else(|| {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        Some(PathBuf::from(home).join(".claude").join("jawata-studio").join("outcomes.log"))
    }) else {
        eprintln!("observer-kpi: no home directory, and no --log given");
        return 2;
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            print!("{}", read_kpi(&text, since.as_deref()).render());
            0
        }
        Err(e) => {
            eprintln!("observer-kpi: cannot read {}: {e}", path.display());
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kpi_counts_each_signal_by_its_detail_and_honours_since() {
        let log = "2026-10-18T01:00:00Z\tv\tobserver-halt\tforeground-wait\n\
                   2026-10-18T02:00:00Z\tv\tobserver-halt\tforeground-wait\n\
                   2026-10-18T02:00:01Z\tv\tobserver-halt\tshell-on-java\n\
                   2026-10-18T02:00:02Z\tv\tobserver-judged-halt\t2\n\
                   2026-10-18T02:00:03Z\tv\tobserver-judge-no-verdict\tsess\n\
                   2026-10-18T02:00:04Z\tv\tslip\tnot ours\n\
                   not a log line\n";
        let all = read_kpi(log, None);
        assert_eq!(all.first_tier.get("foreground-wait"), Some(&2));
        assert_eq!(all.first_tier.get("shell-on-java"), Some(&1));
        assert_eq!(all.hired.get("2"), Some(&1));
        assert_eq!(all.no_verdict, 1);
        let later = read_kpi(log, Some("2026-10-18T01:30:00Z"));
        assert_eq!(later.first_tier.get("foreground-wait"), Some(&1));
        assert!(all.render().contains("foreground-wait 2"));
        assert!(read_kpi("", None).render().contains("none"));
    }

    #[test]
    fn a_missing_log_is_an_error_and_not_a_zero() {
        let missing = std::env::temp_dir().join("no-such-outcomes-log-for-the-kpi");
        assert_eq!(kpi_main(&["--log".to_string(), missing.to_string_lossy().into_owned()]), 2);
    }

    fn bash(text: &str) -> CallView {
        CallView { tool: "Bash".into(), text: text.into(), background: false }
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
    fn an_undeclared_shell_search_of_java_is_a_finding_and_a_declared_one_is_not() {
        let undeclared = bash("grep -rn \"Foo\" --include=Foo.java .");
        assert_eq!(judge(&[], &undeclared).unwrap().class, Class::ShellOnJava);
        let with_reason = bash("# jawata-fallback: a comment string, not a symbol\ngrep -rn \"Foo\" --include=Foo.java .");
        assert_eq!(judge(&[], &with_reason), None);
    }

    #[test]
    fn output_filters_and_words_ending_in_org_are_not_java_reads() {
        for command in [
            "mvn -B -f build/pom.xml install -q 2>&1 | grep -E '\\.java:\\[' | head -3",
            "cd /home/x/org && git add a/B.java && git commit -q -m \"test: B\"",
            "git show HEAD:org.jawata.core/src/org/jawata/core/J.java | head -5",
        ] {
            assert_eq!(judge(&[], &bash(command)), None, "{command}");
        }
    }

    #[test]
    fn a_reader_with_a_java_file_operand_is_a_finding() {
        for command in [
            "sed -n '25,60p' org.jawata.mcp/src/Foo.java",
            "cd /x && head -25 a/b/Test.java",
            "find /x -name Foo.java",
        ] {
            assert_eq!(judge(&[], &bash(command)).map(|f| f.class), Some(Class::ShellOnJava), "{command}");
        }
    }

    /// The replay keeps arguments on one line (newlines flattened), so the marked B1 call reads
    /// as one segment there and is missed; the hook sees the command with its newlines. This is
    /// that call in the shape the hook receives.
    #[test]
    fn the_marked_shell_search_is_caught_in_the_shape_the_hook_receives() {
        let command = "cd /home/harald/CursorProjects/jawata-mcp\n\
            echo \"— where the 66 refactoring rows live:\"\n\
            grep -rln \"66\" --include=PerformedRefactoringCountTest.java . 2>/dev/null\n";
        assert_eq!(judge(&[], &bash(command)).map(|f| f.class), Some(Class::ShellOnJava));
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
    fn a_search_that_only_mentions_javascript_is_not_java() {
        assert_eq!(judge(&[], &bash("grep -rn foo src/app.javascript")), None);
        assert_eq!(judge(&[], &bash("grep -rn foo src/lib.rs")), None);
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
    fn the_window_survives_a_round_trip_through_the_file() {
        let home = home();
        watch(&home, "s1", bash("echo a\tb\nc"));
        let loaded = load(&home, "s1");
        assert_eq!(loaded.number, 1);
        assert_eq!(loaded.calls[0].text, "echo a\tb\nc");
    }

    #[test]
    fn a_session_with_no_id_is_judged_and_never_stored() {
        let home = home();
        assert!(matches!(watch(&home, "", bash("sleep 600")), Watched::Halt(_)));
        assert!(matches!(watch(&home, "", bash("sleep 600")), Watched::Halt(_)));
        assert!(!home.join(".claude").exists());
    }
}
