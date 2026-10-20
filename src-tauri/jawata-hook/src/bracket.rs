//! The run bracket: any long command's output survives it.
//!
//! A long run whose output is thrown away costs a second run the moment any detail
//! is wanted (measured 2026-08-28: two avoidable full runs, about twenty minutes).
//! The old guard answered with a text rule — "redirect it to a file" — that had to
//! guess, from the command's wording, whether a gate was being run and where. The
//! bracket is the other half of that rule made real: instead of asking the caller to
//! write the capture correctly, it IS the capture.
//!
//! ```text
//! jawata-hook-<role> bracket --cd <absolute dir> --out <file> -- <program> [args...]
//! ```
//!
//! It runs the program (no shell) in `--cd`, writes stdout and stderr, interleaved
//! in the order they happened, to `--out`, writes `<out>.summary` when the run ends,
//! prints one line pointing at both, and exits with the program's own status.
//!
//! **It cannot be invoked without a directory.** `--cd` is required and must be
//! absolute, which is why the guard's working-directory rule retires with it instead
//! of being policed beside it: the thing that was being asked for is now the only
//! way to run the command.
//!
//! The summary is a small `key=value` file so it can be read back with nothing but
//! `cat`, after the run, by anyone: command, directory, exit status, seconds, byte
//! and line counts, and the last lines of output — the part a reader wants first.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The word after the binary name that selects this behaviour.
pub const SUBCOMMAND: &str = "bracket";

/// How much of the end of the output the summary repeats.
const TAIL_LINES: usize = 12;

/// What the arguments asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub dir: PathBuf,
    pub out: PathBuf,
    pub argv: Vec<String>,
}

/// Why a bracket call was refused before anything ran.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    MissingDir,
    RelativeDir(String),
    NotADirectory(String),
    MissingOut,
    OutDirMissing(String),
    NoProgram,
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::MissingDir => "bracket: --cd <absolute dir> is required. The run happens \
                there, and the point of the bracket is that it cannot be asked to run \
                anywhere unsaid."
                .into(),
            Refusal::RelativeDir(d) => format!(
                "bracket: --cd must be ABSOLUTE, got '{d}'. A relative directory is resolved \
                 against wherever the shell happens to be, which is the failure this exists to end."
            ),
            Refusal::NotADirectory(d) => format!("bracket: --cd '{d}' is not a directory."),
            Refusal::MissingOut => "bracket: --out <file> is required.".into(),
            Refusal::OutDirMissing(d) => format!("bracket: the directory for --out ('{d}') does not exist."),
            Refusal::NoProgram => "bracket: nothing to run after `--`.".into(),
        }
    }
}

/// Read the arguments after the subcommand word.
pub fn parse(args: &[String]) -> Result<Request, Refusal> {
    let mut dir: Option<String> = None;
    let mut out: Option<String> = None;
    let mut argv = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cd" => dir = it.next().cloned(),
            "--out" => out = it.next().cloned(),
            "--" => {
                argv = it.by_ref().cloned().collect();
                break;
            }
            _ => {}
        }
    }
    let dir = dir.ok_or(Refusal::MissingDir)?;
    if !Path::new(&dir).is_absolute() {
        return Err(Refusal::RelativeDir(dir));
    }
    if !Path::new(&dir).is_dir() {
        return Err(Refusal::NotADirectory(dir));
    }
    let out = out.ok_or(Refusal::MissingOut)?;
    let parent = Path::new(&out).parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(p) = parent {
        if !p.is_dir() {
            return Err(Refusal::OutDirMissing(p.display().to_string()));
        }
    }
    if argv.is_empty() {
        return Err(Refusal::NoProgram);
    }
    Ok(Request { dir: PathBuf::from(dir), out: PathBuf::from(out), argv })
}

/// What a finished run looks like, as written to `<out>.summary`.
#[derive(Debug)]
pub struct Summary {
    pub command: String,
    pub dir: String,
    /// `None` when the program was killed by a signal or never started.
    pub exit: Option<i32>,
    pub started: bool,
    pub seconds: u64,
    pub bytes: u64,
    pub lines: usize,
    pub tail: Vec<String>,
}

impl Summary {
    pub fn render(&self) -> String {
        let exit = match (self.started, self.exit) {
            (false, _) => "not-started".to_string(),
            (true, Some(c)) => c.to_string(),
            (true, None) => "killed-by-signal".to_string(),
        };
        let mut s = format!(
            "command={}\ndir={}\nexit={}\nseconds={}\nbytes={}\nlines={}\n--- last {} line(s) ---\n",
            self.command,
            self.dir,
            exit,
            self.seconds,
            self.bytes,
            self.lines,
            self.tail.len()
        );
        for l in &self.tail {
            s.push_str(l);
            s.push('\n');
        }
        s
    }
}

fn summarise(req: &Request, started: bool, exit: Option<i32>, took: Duration) -> Summary {
    let text = std::fs::read(&req.out).unwrap_or_default();
    let body = String::from_utf8_lossy(&text);
    let lines: Vec<&str> = body.lines().collect();
    let tail = lines
        .iter()
        .rev()
        .take(TAIL_LINES)
        .rev()
        .map(|l| l.to_string())
        .collect();
    Summary {
        command: req.argv.join(" "),
        dir: req.dir.display().to_string(),
        exit,
        started,
        seconds: took.as_secs(),
        bytes: text.len() as u64,
        lines: lines.len(),
        tail,
    }
}

/// The path the summary is written to.
pub fn summary_path(out: &Path) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(".summary");
    PathBuf::from(s)
}

/// Run the request. Returns the summary and the status to exit with.
pub fn execute(req: &Request) -> (Summary, i32) {
    let begin = Instant::now();
    let file = match File::create(&req.out) {
        Ok(f) => f,
        Err(e) => {
            let s = Summary {
                command: req.argv.join(" "),
                dir: req.dir.display().to_string(),
                exit: None,
                started: false,
                seconds: 0,
                bytes: 0,
                lines: 0,
                tail: vec![format!("bracket: could not create {}: {e}", req.out.display())],
            };
            return (s, 126);
        }
    };
    // ONE open file description for both streams, so a line on stderr lands where
    // it happened relative to stdout instead of after it.
    let err_file = file.try_clone();
    let spawned = Command::new(&req.argv[0])
        .args(&req.argv[1..])
        .current_dir(&req.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(match err_file {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        })
        .spawn();
    let (started, exit) = match spawned {
        Ok(mut child) => match child.wait() {
            Ok(status) => (true, status.code()),
            Err(_) => (true, None),
        },
        Err(e) => {
            // Never started: say so IN THE OUTPUT FILE too, or the file a reader
            // opens would be empty and read as "ran, printed nothing".
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&req.out) {
                let _ = writeln!(f, "bracket: could not start `{}`: {e}", req.argv[0]);
            }
            (false, None)
        }
    };
    let summary = summarise(req, started, exit, begin.elapsed());
    let status = match (started, exit) {
        (true, Some(c)) => c,
        (true, None) => 128,
        (false, _) => 127,
    };
    (summary, status)
}

/// The whole subcommand: parse, run, write the summary, print the pointer line.
/// Returns the process exit status.
pub fn main(args: &[String]) -> i32 {
    let req = match parse(args) {
        Ok(r) => r,
        Err(r) => {
            eprintln!("{}", r.message());
            return 2;
        }
    };
    let (summary, status) = execute(&req);
    let path = summary_path(&req.out);
    if let Err(e) = std::fs::write(&path, summary.render()) {
        eprintln!("bracket: could not write {}: {e}", path.display());
    }
    println!(
        "bracket: exit={} seconds={} lines={} out={} summary={}",
        match (summary.started, summary.exit) {
            (false, _) => "not-started".to_string(),
            (true, Some(c)) => c.to_string(),
            (true, None) => "killed-by-signal".to_string(),
        },
        summary.seconds,
        summary.lines,
        req.out.display(),
        path.display()
    );
    status
}

/// Whether an argv is a bracket call (`<binary> bracket ...`).
pub fn is_bracket_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(SUBCOMMAND)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jawata-bracket-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_cannot_be_invoked_without_an_absolute_directory() {
        let d = scratch("dir");
        let out = d.join("o.txt").display().to_string();
        assert_eq!(Err(Refusal::MissingDir), parse(&args(&["--out", &out, "--", "true"])));
        assert_eq!(
            Err(Refusal::RelativeDir("build".into())),
            parse(&args(&["--cd", "build", "--out", &out, "--", "true"]))
        );
        assert!(matches!(
            parse(&args(&["--cd", "/no/such/dir/anywhere", "--out", &out, "--", "true"])),
            Err(Refusal::NotADirectory(_))
        ));
    }

    #[test]
    fn it_needs_an_output_file_and_a_program() {
        let d = scratch("req");
        let dir = d.display().to_string();
        assert_eq!(Err(Refusal::MissingOut), parse(&args(&["--cd", &dir, "--", "true"])));
        let out = d.join("o.txt").display().to_string();
        assert_eq!(Err(Refusal::NoProgram), parse(&args(&["--cd", &dir, "--out", &out, "--"])));
        assert!(matches!(
            parse(&args(&["--cd", &dir, "--out", "/no/such/dir/o.txt", "--", "true"])),
            Err(Refusal::OutDirMissing(_))
        ));
    }

    /// The summary of three kinds of run the stage names — a Rust test run, a front-end
    /// check and a shell script — here stood in for by programs that print what each
    /// prints and fail the way each fails. The real ones go through the same function.
    #[test]
    fn a_run_leaves_its_output_and_a_summary_after_it_ends() {
        let d = scratch("run");
        let out = d.join("run.txt");
        let req = Request {
            dir: d.clone(),
            out: out.clone(),
            // "three" is PRINTED, never written in the command, so the tail assertion below
            // cannot be satisfied by the command line the summary also repeats.
            argv: args(&["sh", "-c", "echo one; echo two 1>&2; printf 'thr%s\\n' ee; exit 3"]),
        };
        let (summary, status) = execute(&req);
        assert_eq!(3, status, "the program's own status comes through");
        assert_eq!(Some(3), summary.exit);
        let written = std::fs::read_to_string(&out).unwrap();
        assert_eq!("one\ntwo\nthree\n", written, "stdout and stderr, in the order they happened");
        let rendered = summary.render();
        assert!(rendered.contains("exit=3") && rendered.contains("lines=3"), "{rendered}");
        assert_eq!(Some(&"three".to_string()), summary.tail.last(), "the tail is the output's own last line");
        assert!(rendered.contains("\nthree\n"), "and it is in the rendered summary: {rendered}");
    }

    #[test]
    fn main_writes_the_summary_file_beside_the_output() {
        let d = scratch("main");
        let out = d.join("o.txt");
        let a = args(&["--cd", &d.display().to_string(), "--out", &out.display().to_string(), "--", "sh", "-c", "printf 'hel%s\\n' lo"]);
        assert_eq!(0, main(&a));
        let s = std::fs::read_to_string(summary_path(&out)).unwrap();
        assert!(s.contains("exit=0") && s.contains("hello"), "{s}");
    }

    #[test]
    fn it_runs_in_the_directory_it_was_given_and_nowhere_else() {
        let d = scratch("cwd");
        let out = d.join("o.txt");
        let req = Request { dir: d.clone(), out: out.clone(), argv: args(&["pwd"]) };
        let (_, status) = execute(&req);
        assert_eq!(0, status);
        let printed = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            std::fs::canonicalize(&d).unwrap(),
            std::fs::canonicalize(printed.trim()).unwrap()
        );
    }

    #[test]
    fn a_program_that_never_started_says_so_in_the_file_a_reader_opens() {
        let d = scratch("nostart");
        let out = d.join("o.txt");
        let req = Request { dir: d.clone(), out: out.clone(), argv: args(&["definitely-not-a-program-xyz"]) };
        let (summary, status) = execute(&req);
        assert_eq!(127, status);
        assert!(!summary.started);
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("could not start"), "an empty file would read as 'ran, printed nothing': {written}");
        assert!(summary.render().contains("exit=not-started"));
    }

    #[test]
    fn only_the_subcommand_word_selects_it() {
        assert!(is_bracket_call(&args(&["jawata-hook-guard", "bracket", "--cd", "/x"])));
        assert!(!is_bracket_call(&args(&["jawata-hook-guard"])));
        assert!(!is_bracket_call(&args(&["jawata-hook-guard", "--explain"])));
    }
}
