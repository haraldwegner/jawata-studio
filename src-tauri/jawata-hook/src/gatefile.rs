//! A script says for itself that it is a long gate.
//!
//! Until this module the guard shipped a list of OUR OWN build-script names, and
//! matched it by substring over the whole command. Measured 2026-10-06: it
//! refused seven correct commands in one session, none of which ran a gate — a
//! `sed` reading the lines that hold the names, a `grep` counting them in the
//! binary, a commit message describing the rule. A list of names is also the
//! wrong owner: it makes a published binary know about one team's files, and it
//! goes stale the day a script is renamed, with nothing failing.
//!
//! The owner of "this script is long" is the script. It carries one line in its
//! own header, and the hook READS it, at the moment the command RUNS it:
//!
//! ```text
//! # jawata-gate: long     <- takes minutes; its output must be kept
//! # jawata-gate: suite    <- the whole-suite runner; also subject to the cadence rule
//! ```
//!
//! Two properties follow, and they are the point:
//!
//! * **Invoked, never mentioned.** Only the program a segment RUNS is considered
//!   (past wrappers and assignments), so a script named as an argument to `sed`,
//!   `grep` or `echo` is never read and never refused.
//! * **A script that cannot be read is not refused.** Not found, unreadable, no
//!   marker: the hook cannot tell, and "cannot tell" is not "long". The old rule
//!   answered the same case with a refusal, which is how it refused commands that
//!   ran nothing.
//!
//! What it gives up, stated: a long script that does not carry the marker is
//! unguarded until it does. The marker is one comment line.

use std::path::{Path, PathBuf};

/// The token a script's header carries: `# jawata-gate: long` or `suite`.
pub const MARKER: &str = "jawata-gate:";

/// How many header lines are read. A declaration is a header comment; reading
/// further would make a mention in the body count.
const HEADER_LINES: usize = 30;

/// A script larger than this is not a shell script anyone wrote a header on.
const MAX_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    /// Takes minutes; the output of a run must be kept.
    Long,
    /// The whole-suite runner: long, and also subject to the cadence rule.
    Suite,
}

/// Interpreters whose first non-flag argument is the script they run.
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash"];

/// Wrappers that run what follows them — ONE table, shared by every rule that asks
/// "what does this segment run". There used to be two (this one and `guard::PREFIXES`)
/// and they disagreed on `xargs`, `exec` and `npx`, so two rules could read one command
/// differently.
const WRAPPERS: &[&str] =
    &["sudo", "env", "time", "nice", "command", "nohup", "exec", "npx", "xargs", "timeout", "eval"];

fn basename(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// Whether a backslash before `next` ESCAPES it — that is, whether the shell would read the
/// pair as the one character. Outside quotes that is whitespace and the characters bash gives a
/// meaning to; inside double quotes bash honours only `$`, backtick, `"`, `\` and a newline.
///
/// REL44-2: a backslash before ANY other character is kept as a backslash. The old rule
/// escaped everything, which is bash's reading and not PowerShell's or cmd's, so on Windows a
/// path such as `C:\Users\x` became `C:Usersx` and every rule reading a path from a command
/// read the wrong one. The commands this hook judges come from bash (Claude Code, also on
/// Windows through Git Bash) and from PowerShell (Cursor and Codex on Windows), and the hook is
/// not told which; this rule reads both the same for everything a person writes on purpose.
/// It differs from bash only on an escape that changes nothing — bash reads `\U` as `U`, which
/// nobody writes — and that is exactly the place a Windows path puts its separators.
fn backslash_escapes(next: char, in_double_quotes: bool) -> bool {
    if in_double_quotes {
        return matches!(next, '$' | '`' | '"' | '\\' | '\n');
    }
    next.is_whitespace()
        || matches!(
            next,
            '\\' | '\'' | '"' | '$' | '`' | '|' | '&' | ';' | '<' | '>' | '(' | ')' | '*' | '?'
                | '[' | ']' | '#' | '~' | '!' | '{' | '}'
        )
}

/// Words of a command, quote-aware: whitespace separates outside quotes, the quotes
/// themselves are removed, and a backslash outside single quotes escapes the next character
/// when that character needs escaping (`backslash_escapes`); before any other character it is
/// an ordinary character, so a Windows path keeps its separators.
pub fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let (mut single, mut double) = (false, false);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if !single => {
                match chars.peek() {
                    Some(&n) if backslash_escapes(n, double) => {
                        cur.push(n);
                        chars.next();
                    }
                    _ => cur.push('\\'),
                }
                has = true;
            }
            '\'' if !double => {
                single = !single;
                has = true;
            }
            '"' if !single => {
                double = !double;
                has = true;
            }
            c if c.is_whitespace() && !single && !double => {
                if has {
                    words.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        words.push(cur);
    }
    words
}

/// A segment split into the `NAME=value` assignments that precede its command and the
/// words of the command itself, past leading wrappers and their arguments. ONE reading of
/// "what comes before the command", asked by `command_words` and `command_assignments`.
fn split_command(segment: &str) -> (Vec<(String, String)>, Vec<String>) {
    let all = shell_words(segment);
    let mut assignments = Vec::new();
    let mut i = 0;
    while i < all.len() {
        let w = &all[i];
        let bare = basename(w);
        if WRAPPERS.contains(&bare) {
            i += 1;
            // `timeout 600 cmd`: the duration is the wrapper's, not the command's.
            if bare == "timeout" {
                while i < all.len()
                    && (all[i].starts_with('-') || all[i].chars().next().is_some_and(|c| c.is_ascii_digit()))
                {
                    i += 1;
                }
            }
            continue;
        }
        // An assignment is decided by its NAME, not by the whole word: `FOO=/tmp/x cmd` is an
        // assignment whose value is a path, and the old whole-word test read it as the command.
        if let Some((name, value)) = w.split_once('=') {
            if !name.is_empty() && !name.starts_with('-') && !name.contains('/') {
                assignments.push((name.to_string(), value.to_string()));
                i += 1;
                continue;
            }
        }
        break;
    }
    (assignments, all[i..].to_vec())
}

/// The words of a segment past leading wrappers, their arguments and `NAME=value`
/// assignments.
pub fn command_words(segment: &str) -> Vec<String> {
    split_command(segment).1
}

/// The `NAME=value` assignments in front of a segment's command: the ones
/// `command_words` skips, read by the same code so the two cannot disagree about where
/// the command starts.
pub fn command_assignments(segment: &str) -> Vec<(String, String)> {
    split_command(segment).0
}

/// The script a segment RUNS, as written — `None` when it runs something that
/// is not a script path, or an inline program (`bash -c ...`).
pub fn script_run_by(segment: &str) -> Option<String> {
    let words = command_words(segment);
    let head = words.first()?;
    if SHELLS.contains(&basename(head)) {
        if words.iter().skip(1).any(|w| w == "-c") {
            return None;
        }
        // `bash -n script` PARSES the script and runs nothing (POSIX noexec, as is `-o noexec`),
        // so it is a syntax check of a gate, not a run of one; refusing it as an uncaptured long
        // run made the parse check of an edited gate script impossible without a declaration.
        // Only the shell's OWN flags count — those before the script; `bash run.sh -n 4` passes
        // `-n` to the script, which still runs.
        let mut rest = words.iter().skip(1).peekable();
        while let Some(flag) = rest.next_if(|w| w.starts_with('-')) {
            if flag.starts_with("--") {
                continue;
            }
            let letters = &flag[1..];
            if letters.contains('n') {
                return None;
            }
            // `-o NAME` (also clustered, `-euo pipefail`): the next word is the option's name,
            // not the script — so it is consumed here, and `noexec` among them is a parse only.
            if letters.ends_with('o') && rest.next().is_some_and(|name| name == "noexec") {
                return None;
            }
        }
        return rest.next().cloned();
    }
    if head.contains('/') || head.ends_with(".sh") {
        return Some(head.clone());
    }
    None
}

/// What the script's own header says it is, resolved against `base`.
pub fn declared(script: &str, base: &Path) -> Option<GateKind> {
    let path = if Path::new(script).is_absolute() {
        PathBuf::from(script)
    } else {
        base.join(script)
    };
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    for line in text.lines().take(HEADER_LINES) {
        let line = line.trim();
        if !line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.split(MARKER).nth(1) {
            return match rest.split_whitespace().next() {
                Some("long") => Some(GateKind::Long),
                Some("suite") => Some(GateKind::Suite),
                _ => None,
            };
        }
    }
    None
}

/// A bracket call, seen through: the directory it runs in and the command it wraps.
///
/// The bracket must not become a way to hide a run from the rules that police WHAT is
/// run. Its head is the hook binary, so without this a whole-suite run wrapped in it
/// would pass the cadence gate unseen. The directory is the bracket's own `--cd`, which
/// is also where a relative script inside is read from.
pub fn bracket_inner(segment: &str) -> Option<(PathBuf, String)> {
    let words = command_words(segment);
    let head = basename(words.first()?);
    if !head.starts_with("jawata-hook")
        || words.get(1).map(String::as_str) != Some(crate::bracket::SUBCOMMAND)
    {
        return None;
    }
    let dir = words
        .iter()
        .position(|w| w == "--cd")
        .and_then(|i| words.get(i + 1))
        .map(PathBuf::from)?;
    let split = words.iter().position(|w| w == "--")?;
    let inner = words[split + 1..].join(" ");
    (!inner.is_empty()).then_some((dir, inner))
}

/// Whether the program a segment runs is a tool that is long by what it IS —
/// the build tools themselves, not our scripts.
fn is_long_tool(segment: &str) -> bool {
    let words = command_words(segment);
    let Some(head) = words.first().map(|w| basename(w)) else {
        return false;
    };
    // `cargo +nightly test`: a toolchain selector is not the subcommand.
    let rest: Vec<&str> =
        words.iter().skip(1).map(String::as_str).filter(|w| !w.starts_with('+')).collect();
    let second = rest.first().copied().unwrap_or("");
    let third = rest.get(1).copied().unwrap_or("");
    head == "mvn"
        || head == "mvnw"
        || head == "svelte-check"
        || (head == "cargo" && (second == "test" || second == "build"))
        || (head == "vite" && second == "build")
        || (head == "npm"
            && (second == "test" || (second == "run" && matches!(third, "check" | "build" | "test"))))
}

/// Whether this segment RUNS a long gate: a long tool, or a script that declares itself one.
pub fn runs_long_gate(segment: &str, base: &Path) -> bool {
    if is_long_tool(segment) {
        return true;
    }
    script_run_by(segment).is_some_and(|s| declared(&s, base).is_some())
}

/// The script this segment runs when it declares itself the whole-suite runner.
pub fn suite_script_run_by(segment: &str, base: &Path) -> Option<String> {
    let script = script_run_by(segment)?;
    (declared(&script, base) == Some(GateKind::Suite)).then_some(script)
}

/// The directory a segment begins in, given the one before it: `cd X` moves it.
pub fn dir_after(segment: &str, base: &Path) -> PathBuf {
    let words = command_words(segment);
    if words.first().map(|w| basename(w)) == Some("cd") {
        if let Some(target) = words.get(1) {
            let p = Path::new(target.as_str());
            return if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
        }
    }
    base.to_path_buf()
}

/// Where the command starts: the hook's own working directory, which a client
/// sets to the session's.
pub fn start_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// One command of a command line, with what a rule needs to judge it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// What this segment runs. For a bracket call, the command it WRAPS.
    pub text: String,
    /// Where it runs: the cumulative effect of every earlier `cd`, or a bracket's `--cd`.
    pub dir: PathBuf,
    /// Whether it is the inside of a bracket call.
    pub bracketed: bool,
}

/// Split a command line into the commands it runs — ONE splitter for every rule.
///
/// Three rules used to split the same command three ways (`&&`/`;` in the capture rule,
/// `| ; \n &` in the guard, whitespace in the head test), so one command could be judged
/// differently by two rules: a newline between `cd` and a gate hid the gate from the
/// capture rule, and a separator inside a quoted commit message handed a rule a
/// "command" that was prose. This is quote-aware (a separator inside quotes is text),
/// drops comments, skips heredoc bodies (they are data), unwraps `bash -c "..."` and
/// `eval "..."` into the commands they carry, sees through a bracket call, and tracks
/// `cd`.
pub fn segments(command: &str, start: &Path) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut base = start.to_path_buf();
    collect(command, &mut base, &mut out, 0);
    out
}

fn collect(command: &str, base: &mut PathBuf, out: &mut Vec<Segment>, depth: usize) {
    for piece in split_commands(command) {
        let here = base.clone();
        *base = dir_after(&piece, base);
        if let Some((dir, inner)) = bracket_inner(&piece) {
            out.push(Segment { text: inner, dir, bracketed: true });
            continue;
        }
        // `bash -c "a; b"` / `eval "a; b"`: the quoted string is itself a command line.
        if depth < 4 {
            let words = shell_words(&piece);
            let carried = match words.first().map(|w| basename(w)) {
                Some(h) if SHELLS.contains(&h) => words
                    .iter()
                    .position(|w| w == "-c")
                    .and_then(|i| words.get(i + 1))
                    .cloned(),
                Some("eval") if words.len() > 1 => Some(words[1..].join(" ")),
                _ => None,
            };
            if let Some(inner) = carried {
                let mut inner_base = here.clone();
                collect(&inner, &mut inner_base, out, depth + 1);
                continue;
            }
        }
        out.push(Segment { text: piece, dir: here, bracketed: false });
    }
}

/// The commands of a line, split at separators OUTSIDE quotes. Comments and heredoc
/// bodies are not commands and are dropped.
fn split_commands(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut pieces = Vec::new();
    let mut cur = String::new();
    let (mut single, mut double) = (false, false);
    let mut heredoc: Option<String> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev = if i == 0 { ' ' } else { chars[i - 1] };
        if !single && !double {
            // A comment runs to the end of the line.
            if c == '#' && (prev.is_whitespace() || cur.trim().is_empty()) {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            // `<<WORD` / `<<'WORD'` / `<<-WORD`: remember the terminator; the body is
            // skipped at the end of this line.
            if c == '<' && chars.get(i + 1) == Some(&'<') && chars.get(i + 2) != Some(&'<') {
                let mut j = i + 2;
                if chars.get(j) == Some(&'-') {
                    j += 1;
                }
                while chars.get(j).is_some_and(|c| *c == ' ') {
                    j += 1;
                }
                let mut word = String::new();
                while let Some(&w) = chars.get(j) {
                    if w.is_whitespace() || ";&|()".contains(w) {
                        break;
                    }
                    if w != '\'' && w != '"' {
                        word.push(w);
                    }
                    j += 1;
                }
                if !word.is_empty() {
                    heredoc = Some(word);
                }
                cur.push_str(&chars[i..j].iter().collect::<String>());
                i = j;
                continue;
            }
            if c == '\n' {
                if let Some(delim) = heredoc.take() {
                    pieces.push(std::mem::take(&mut cur));
                    i += 1;
                    while i < chars.len() {
                        let mut j = i;
                        while j < chars.len() && chars[j] != '\n' {
                            j += 1;
                        }
                        let line: String = chars[i..j].iter().collect();
                        i = j + 1;
                        if line.trim() == delim {
                            break;
                        }
                    }
                    continue;
                }
            }
            if matches!(c, '\n' | ';' | '&' | '|' | '(' | ')' | '`') {
                pieces.push(std::mem::take(&mut cur));
                i += 1;
                continue;
            }
        }
        match c {
            '\\' if !single => {
                cur.push(c);
                if let Some(n) = chars.get(i + 1) {
                    cur.push(*n);
                    i += 1;
                }
            }
            '\'' if !double => {
                single = !single;
                cur.push(c);
            }
            '"' if !single => {
                double = !double;
                cur.push(c);
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    pieces.push(cur);
    pieces.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

#[cfg(test)]
pub(crate) mod testkit {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// A scratch directory holding a script with the given header line.
    pub fn script_with(header: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "jawata-gatefile-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "gate.sh".to_string();
        std::fs::write(dir.join(&name), format!("#!/bin/bash\n{header}\necho hi\n")).unwrap();
        (dir, name)
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::script_with;
    use super::*;

    #[test]
    fn an_assignment_whose_value_is_a_path_is_still_an_assignment() {
        // The whole-word test used to read `OPTS=-javaagent:/t/a.jar java -jar x` as a command
        // called `OPTS=-javaagent:/t/a.jar`, so every rule asking "what does this run" was told
        // the wrong thing whenever a value held a slash.
        assert_eq!(command_words("OPTS=/t/a.jar java -jar x"), ["java", "-jar", "x"]);
        assert_eq!(
            command_assignments("A=1 OPTS=/t/a.jar java -jar x"),
            [("A".to_string(), "1".to_string()), ("OPTS".to_string(), "/t/a.jar".to_string())]
        );
        // A path that merely contains an equals sign is a command, not an assignment.
        assert_eq!(command_words("./dir=odd/run.sh go"), ["./dir=odd/run.sh", "go"]);
        assert!(command_assignments("./dir=odd/run.sh go").is_empty());
        // The two readers cannot disagree about where the command starts.
        assert_eq!(command_assignments("timeout 5 X=1 sleep 1").len(), 1);
        assert_eq!(command_words("timeout 5 X=1 sleep 1"), ["sleep", "1"]);
    }

    #[test]
    fn a_script_declares_itself_in_its_header() {
        let (dir, name) = script_with("# jawata-gate: long");
        assert_eq!(Some(GateKind::Long), declared(&name, &dir));
        let (dir, name) = script_with("# jawata-gate: suite");
        assert_eq!(Some(GateKind::Suite), declared(&name, &dir));
        let (dir, name) = script_with("# an ordinary header");
        assert_eq!(None, declared(&name, &dir), "no marker, no declaration");
    }

    #[test]
    fn a_marker_in_the_body_is_not_a_declaration() {
        let dir = std::env::temp_dir().join(format!("jawata-gatefile-body-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = String::from("#!/bin/bash\n");
        for _ in 0..HEADER_LINES {
            body.push_str("true\n");
        }
        body.push_str("# jawata-gate: long\n");
        std::fs::write(dir.join("late.sh"), body).unwrap();
        assert_eq!(None, declared("late.sh", &dir));
    }

    #[test]
    fn what_cannot_be_read_is_not_a_gate() {
        let dir = std::env::temp_dir();
        assert_eq!(None, declared("no-such-script-anywhere.sh", &dir));
        assert!(!runs_long_gate("./no-such-script-anywhere.sh 4", &dir));
    }

    #[test]
    fn the_script_a_segment_runs_is_found_past_wrappers_and_shells() {
        assert_eq!(Some("./a/b.sh".into()), script_run_by("./a/b.sh 4"));
        assert_eq!(Some("./a/b.sh".into()), script_run_by("FOO=1 nohup ./a/b.sh 4"));
        assert_eq!(Some("a/b.sh".into()), script_run_by("bash a/b.sh --x"));
        assert_eq!(Some("a/b.sh".into()), script_run_by("bash -x a/b.sh"));
        assert_eq!(None, script_run_by("bash -c 'echo hi'"), "an inline program is not a path");
        assert_eq!(None, script_run_by("sed -n 1,5p ./a/b.sh"), "a script named as an argument is not run");
        assert_eq!(None, script_run_by("grep -c gate ./a/b.sh"));
        assert_eq!(None, script_run_by("echo ./a/b.sh"));
    }

    /// `bash -n` parses a script and runs nothing, so a syntax check of an edited gate is not a
    /// run of it — the hook refused exactly that on 2026-10-15 as an uncaptured long run.
    #[test]
    fn a_shell_told_not_to_execute_runs_no_script() {
        assert_eq!(None, script_run_by("bash -n ./a/b.sh"));
        assert_eq!(None, script_run_by("bash -xn ./a/b.sh"), "noexec inside a cluster");
        assert_eq!(None, script_run_by("bash -o noexec ./a/b.sh"));
        // The flag belongs to the SHELL only before the script; after it, it is the script's own.
        assert_eq!(Some("./a/b.sh".into()), script_run_by("bash ./a/b.sh -n 4"));
        // An option's name is not the script it runs.
        assert_eq!(Some("./a/b.sh".into()), script_run_by("bash -o pipefail ./a/b.sh"));
        assert_eq!(Some("./a/b.sh".into()), script_run_by("bash -euo pipefail ./a/b.sh"));
        assert_eq!(Some("./a/b.sh".into()), script_run_by("bash --norc ./a/b.sh"));
    }

    #[test]
    #[cfg_attr(windows, ignore = "unproven on Windows: since REL44-2 the word splitter keeps a Windows path as written (gatefile::tests::a_windows_path_keeps_its_separators, run on every platform), but these tests run end to end and have not run on a Windows machine since that fix; un-ignore them at the next Windows run")]
    fn a_cd_moves_the_base_the_next_script_is_read_from() {
        let (dir, name) = script_with("# jawata-gate: long");
        let elsewhere = std::env::temp_dir();
        let cd = format!("cd {}", dir.display());
        let moved = dir_after(&cd, &elsewhere);
        assert_eq!(dir, moved);
        assert!(runs_long_gate(&format!("./{name}"), &moved));
        assert!(!runs_long_gate(&format!("./{name}"), &elsewhere), "same script, other directory");
    }

    #[test]
    fn a_bracket_call_is_seen_through_to_what_it_runs() {
        let (dir, inner) = bracket_inner(
            "/h/.claude/jawata-hook-guard bracket --cd /repo --out /tmp/o.txt -- ./a.sh 4 --full",
        )
        .expect("a bracket call");
        assert_eq!(PathBuf::from("/repo"), dir);
        assert_eq!("./a.sh 4 --full", inner);
        assert!(bracket_inner("./a.sh 4 --full").is_none());
        assert!(bracket_inner("jawata-hook-guard bracket --cd /repo --out /o --").is_none(), "nothing wrapped");
        assert!(bracket_inner("echo jawata-hook-guard bracket --cd /r -- x").is_none(), "named, not run");
    }

    fn texts(cmd: &str) -> Vec<String> {
        segments(cmd, Path::new("/w")).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn one_splitter_cuts_on_every_separator_outside_quotes() {
        assert_eq!(vec!["a", "b", "c", "d", "e", "f"], texts("a && b || c ; d\ne & f"));
        assert_eq!(vec!["a", "b"], texts("a | b"));
        // `$(` leaves a bare `$` on the piece before it; the command inside is its own segment.
        assert_eq!(vec!["a $", "b", "c"], texts("a $(b) && (c)"));
    }

    #[test]
    fn a_separator_inside_quotes_is_text() {
        assert_eq!(vec!["git commit -m \"a; b && c\""], texts("git commit -m \"a; b && c\""));
        assert_eq!(vec!["echo 'x | y'"], texts("echo 'x | y'"));
    }

    #[test]
    fn comments_and_heredoc_bodies_are_not_commands() {
        assert_eq!(vec!["echo hi"], texts("echo hi # && mvn test"));
        assert_eq!(vec!["cat <<'EOF'", "echo after"], texts("cat <<'EOF'\nmvn test\n./x.sh\nEOF\necho after"));
        assert_eq!(vec!["echo a#b"], texts("echo a#b"), "a hash inside a word is not a comment");
    }

    #[test]
    fn a_shell_c_string_and_eval_are_commands_in_their_own_right() {
        assert_eq!(vec!["mvn test", "echo done"], texts("bash -c \"mvn test; echo done\""));
        assert_eq!(vec!["mvn test"], texts("eval mvn test"));
        assert_eq!(vec!["mvn test"], texts("eval \"mvn test\""));
    }

    #[test]
    fn a_cd_on_an_earlier_line_moves_every_later_segment() {
        let segs = segments("cd /a/b\ncd c\n./x.sh", Path::new("/w"));
        assert_eq!(PathBuf::from("/a/b/c"), segs.last().unwrap().dir);
    }

    #[test]
    fn a_bracket_call_becomes_its_wrapped_command_run_where_the_bracket_says() {
        let segs = segments(
            "/h/jawata-hook-guard bracket --cd /repo --out /tmp/o -- ./a.sh 4 --full",
            Path::new("/w"),
        );
        assert_eq!(1, segs.len());
        assert!(segs[0].bracketed);
        assert_eq!("./a.sh 4 --full", segs[0].text);
        assert_eq!(PathBuf::from("/repo"), segs[0].dir);
    }

    #[test]
    fn words_are_read_quote_aware() {
        assert_eq!(vec!["a b", "c"], shell_words("'a b' c"));
        assert_eq!(vec!["a b", "c"], shell_words("\"a b\" c"));
        assert_eq!(vec!["a b"], shell_words("a\\ b"));
    }

    /// REL44-2: a Windows path in a command keeps its separators, quoted or not — before, every
    /// backslash was an escape and `C:\Users\x` was read as `C:Usersx`.
    #[test]
    fn a_windows_path_keeps_its_separators() {
        assert_eq!(
            vec!["cd", r"C:\Users\x\proj", "&&", "bash", r"build\run.sh"],
            shell_words(r"cd C:\Users\x\proj && bash build\run.sh")
        );
        assert_eq!(
            vec!["bash", r"C:\Program Files\x\run.sh"],
            shell_words(r#"bash "C:\Program Files\x\run.sh""#)
        );
    }

    /// The escapes a bash user writes on purpose still escape, so nothing a Linux or macOS
    /// command meant changes with REL44-2's rule.
    #[test]
    fn the_escapes_bash_users_write_still_escape() {
        assert_eq!(vec!["a b"], shell_words(r"a\ b"));
        assert_eq!(vec![r#""q""#], shell_words(r#"\"q\""#));
        assert_eq!(vec![r"\"], shell_words(r"\\"));
        assert_eq!(vec![r#"a"b"#], shell_words(r#""a\"b""#));
        assert_eq!(vec!["$HOME"], shell_words(r#""\$HOME""#));
        assert_eq!(vec!["a;b"], shell_words(r"a\;b"));
        assert_eq!(vec![r"\n"], shell_words(r"'\n'"), "a single-quoted backslash is literal");
    }

    #[test]
    fn the_build_tools_are_long_by_what_they_are() {
        let d = std::env::temp_dir();
        for cmd in ["mvn -q install", "./mvnw verify", "cargo test --lib", "cargo build", "vite build", "svelte-check", "time mvn test"] {
            assert!(runs_long_gate(cmd, &d), "{cmd}");
        }
        for cmd in ["echo mvn", "grep mvn pom.xml", "cargo fmt", "cargo --version", "vite dev"] {
            assert!(!runs_long_gate(cmd, &d), "{cmd}");
        }
    }
}
