//! The suite cadence, read back.
//!
//! The cadence gate (`guard.rs`) refuses a full-suite run that does not declare a
//! checkpoint or a release. It is self-certified — nothing stops a false
//! `checkpoint:` — so its value is not the block: it is that N runs become N written
//! claims, and a stage with four declared checkpoints and one real one is visible
//! afterwards. This module is the "afterwards". Until it existed the gate wrote nothing
//! down, so what it did over a sprint could only be reconstructed from memory.
//!
//! Every full-suite attempt writes one line to the outcomes log; this reads them:
//!
//! ```text
//! jawata-hook-<role> cadence [--since YYYY-MM-DD] [--log <outcomes.log>]
//! ```
//!
//! It reports the count of attempts, how many ran and how many were refused, how many
//! declared a checkpoint or a release, and EVERY declared reason with its time, so a
//! reader can lay them beside the plan's checkpoints and releases and see which are
//! real. It does not decide that for them: whether "C4" was a real checkpoint is the
//! plan's fact, not the log's.

use std::path::PathBuf;

/// The outcomes-log signal a full-suite attempt writes.
pub const SIGNAL: &str = "suite-run";

/// The subcommand word.
pub const SUBCOMMAND: &str = "cadence";

/// The detail column for one attempt.
pub fn detail(allowed: bool, kind: &str, reason: &str, runner: &str) -> String {
    // The log is tab-separated and line-oriented; a reason carrying either would
    // shift every column after it.
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    format!(
        "verdict={}|declared={}|reason={}|runner={}",
        if allowed { "ran" } else { "refused" },
        kind,
        clean(reason),
        clean(runner)
    )
}

/// One attempt, as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub when: String,
    pub ran: bool,
    pub declared: String,
    pub reason: String,
}

/// The cadence over a window.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub attempts: Vec<Attempt>,
}

impl Report {
    pub fn ran(&self) -> usize {
        self.attempts.iter().filter(|a| a.ran).count()
    }
    pub fn refused(&self) -> usize {
        self.attempts.len() - self.ran()
    }
    pub fn declared(&self, kind: &str) -> usize {
        self.attempts.iter().filter(|a| a.declared == kind).count()
    }

    pub fn render(&self) -> String {
        let mut s = format!(
            "full-suite attempts: {}\n  ran: {}\n  refused (no declaration, or not bracketed): {}\n  declared a checkpoint: {}\n  declared a release: {}\n",
            self.attempts.len(),
            self.ran(),
            self.refused(),
            self.declared("checkpoint"),
            self.declared("release"),
        );
        s.push_str("declared reasons, in order (match each to a real checkpoint or release):\n");
        let mut any = false;
        for a in self.attempts.iter().filter(|a| a.declared != "none") {
            any = true;
            s.push_str(&format!(
                "  {}  {}  {}: {}\n",
                a.when,
                if a.ran { "ran    " } else { "refused" },
                a.declared,
                a.reason
            ));
        }
        if !any {
            s.push_str("  (none)\n");
        }
        s
    }
}

fn field<'a>(detail: &'a str, key: &str) -> &'a str {
    detail
        .split('|')
        .find_map(|kv| kv.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .unwrap_or("")
}

/// Read the cadence out of an outcomes log. `since` is an ISO date (or prefix of
/// one); lines stamped earlier are left out. Lines that are not this signal, or are
/// malformed, are skipped — the log is shared with every other signal.
pub fn read(log: &str, since: Option<&str>) -> Report {
    let mut attempts = Vec::new();
    for line in log.lines() {
        let mut cols = line.splitn(4, '\t');
        let (Some(when), Some(_version), Some(signal), Some(detail)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        if signal != SIGNAL {
            continue;
        }
        if since.is_some_and(|s| when < s) {
            continue;
        }
        attempts.push(Attempt {
            when: when.to_string(),
            ran: field(detail, "verdict") == "ran",
            declared: field(detail, "declared").to_string(),
            reason: field(detail, "reason").to_string(),
        });
    }
    Report { attempts }
}

fn default_log() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".claude").join("jawata-studio").join("outcomes.log"))
}

/// The whole subcommand. Returns the exit status.
pub fn main(args: &[String]) -> i32 {
    let mut since: Option<String> = None;
    let mut log: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--since" => since = it.next().cloned(),
            "--log" => log = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    let Some(path) = log.or_else(default_log) else {
        eprintln!("cadence: no home directory, and no --log given");
        return 2;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            // Not the same as "no runs": a missing log means the gate never wrote
            // here, and saying zero would read as a quiet sprint.
            eprintln!("cadence: cannot read {}: {e}", path.display());
            return 2;
        }
    };
    print!("{}", read(&text, since.as_deref()).render());
    0
}

/// Whether an argv is a cadence call.
pub fn is_cadence_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(SUBCOMMAND)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(when: &str, signal: &str, d: &str) -> String {
        format!("{when}\tv4.3.4\t{signal}\t{d}\n")
    }

    fn log() -> String {
        let mut s = String::new();
        s += &line("2026-10-08T09:00:00Z", SIGNAL, &detail(true, "checkpoint", "C3", "r"));
        s += &line("2026-10-08T09:30:00Z", SIGNAL, &detail(false, "none", "", "r"));
        s += &line("2026-10-09T10:00:00Z", SIGNAL, &detail(true, "checkpoint", "C4 (the five smaller defects)", "r"));
        s += &line("2026-10-10T11:00:00Z", SIGNAL, &detail(true, "release", "v4.2.0", "r"));
        s += &line("2026-10-10T11:05:00Z", "recall-skipped", "injected=2 disposed=0");
        s += "garbage without tabs\n";
        s
    }

    #[test]
    fn it_counts_runs_refusals_and_declarations() {
        let r = read(&log(), None);
        assert_eq!(4, r.attempts.len(), "other signals and garbage are not attempts");
        assert_eq!(3, r.ran());
        assert_eq!(1, r.refused());
        assert_eq!(2, r.declared("checkpoint"));
        assert_eq!(1, r.declared("release"));
    }

    #[test]
    fn it_lists_every_declared_reason_so_a_reader_can_match_them() {
        let out = read(&log(), None).render();
        assert!(out.contains("checkpoint: C3") && out.contains("release: v4.2.0"), "{out}");
        assert!(out.contains("C4 (the five smaller defects)"), "the whole reason, not a token: {out}");
        assert!(!out.contains("declared none"), "an undeclared refusal has no reason to list: {out}");
    }

    #[test]
    fn the_window_leaves_earlier_runs_out() {
        let r = read(&log(), Some("2026-10-09"));
        assert_eq!(2, r.attempts.len());
        assert_eq!(0, r.refused(), "the refusal was on the 8th");
    }

    #[test]
    fn a_reason_cannot_shift_the_columns() {
        let d = detail(true, "checkpoint", "a\tb\nc", "r");
        let l = line("2026-10-10T00:00:00Z", SIGNAL, &d);
        assert_eq!(1, l.lines().count());
        assert_eq!("a b c", read(&l, None).attempts[0].reason);
    }

    #[test]
    fn a_missing_log_is_not_a_quiet_sprint() {
        let a = vec!["--log".to_string(), "/no/such/outcomes.log".to_string()];
        assert_eq!(2, main(&a), "saying zero would read as no full runs");
    }

    #[test]
    fn it_is_selected_by_its_subcommand_word_only() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_cadence_call(&argv(&["jawata-hook-guard", "cadence"])));
        assert!(!is_cadence_call(&argv(&["jawata-hook-guard", "bracket"])));
    }
}
