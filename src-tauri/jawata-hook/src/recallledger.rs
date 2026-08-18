//! Did the session DO anything with the knowledge it was given?
//!
//! The gate (`recallgate`) can only speak when a record's own anchor is the
//! symbol a call is about. The incident that motivated all of this was a
//! PACKAGE-anchored record: offered, read, reasoned past, with nothing
//! observing the miss. This ledger is the half that covers it — it does not
//! judge relevance, it judges whether the agent ever said anything at all.
//!
//! **Keyed on INJECTED, never on answered.** The store answering is not the
//! same event as the agent being told: on Cursor `Role::UserPrompt` and
//! `Role::Observer` are `can_inject: false`, so matches are recorded and never
//! reach the session. Counting those as skips would charge the agent for a
//! client's capability gap — a 100% false rate in the one number that is
//! supposed to be the honest form of the utilization metric. A skip means
//! knowledge WAS delivered and nothing came back.
//!
//! One line per event, append-only, per session. No read-modify-write: the
//! studio's own silence log was written that way once and lost records under
//! concurrent appends.
//!
//! **There is no signal TEXT here, deliberately.** An earlier draft carried a
//! sentence to show the agent at session end; the only shape `Role::Stop` can
//! inject is a BLOCK decision, which bounces the agent back into a turn it has
//! finished — over a measurement. So the skip is recorded and counted, the
//! studio renders it, and nothing interrupts. The unused sentence was deleted
//! rather than left sitting here looking like a delivered capability: the last
//! member kept in that state failed a release on the hollow-wiring gate.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// What a session did with what it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// Injections that actually reached the session.
    pub injected: usize,
    /// Dispositions the agent stated afterwards.
    pub dispositioned: usize,
}

impl Verdict {
    /// Knowledge was delivered and nothing was ever said about it.
    ///
    /// Deliberately not a ratio. One disposition in a session that received
    /// five injections is a judgement call about relevance, and this ledger
    /// does not make those — it catches the total silence, which is the shape
    /// that actually happened and the only one it can call without guessing.
    pub fn is_skip(&self) -> bool {
        self.injected > 0 && self.dispositioned == 0
    }
}

fn path(dir: &Path, session: &str) -> PathBuf {
    dir.join("recallledger").join(session)
}

fn append(dir: &Path, session: &str, line: &str) {
    if session.is_empty() {
        return; // no session, no ledger — never accuse blindly
    }
    let file = path(dir, session);
    let _ = std::fs::create_dir_all(file.parent().unwrap_or(dir));
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(&file) {
        let _ = writeln!(f, "{line}");
    }
}

/// Knowledge REACHED the session — the only event that can later be a skip.
pub fn record_injected(dir: &Path, session: &str) {
    append(dir, session, "injected");
}

/// The agent said what it did with it.
pub fn record_disposition(dir: &Path, session: &str, token: &str) {
    append(dir, session, &format!("disposed\t{}", token.replace('\n', " ")));
}

/// Read the session's ledger. An absent file is an empty verdict, not a skip —
/// a session that was never given anything cannot have ignored anything.
pub fn verdict(dir: &Path, session: &str) -> Verdict {
    let Ok(body) = std::fs::read_to_string(path(dir, session)) else {
        return Verdict { injected: 0, dispositioned: 0 };
    };
    let mut v = Verdict { injected: 0, dispositioned: 0 };
    for line in body.lines() {
        if line == "injected" {
            v.injected += 1;
        } else if line.starts_with("disposed") {
            v.dispositioned += 1;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A per-test scratch dir, the way the sibling modules make one — this
    /// crate deliberately carries no `tempfile` dependency.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir()
                .join(format!("jawata-recallledger-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Scratch(d)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(tag: &str) -> Scratch {
        Scratch::new(tag)
    }

    #[test]
    fn knowledge_taken_and_never_answered_is_a_skip() {
        let d = dir("skip");
        record_injected(d.path(), "s1");
        record_injected(d.path(), "s1");
        let v = verdict(d.path(), "s1");
        assert_eq!(2, v.injected);
        assert!(v.is_skip(), "two injections, no disposition: {v:?}");
    }

    #[test]
    fn one_word_from_the_agent_closes_it() {
        let d = dir("closed");
        record_injected(d.path(), "s1");
        record_disposition(d.path(), "s1", "recall-rejected: about the other overload");
        assert!(!verdict(d.path(), "s1").is_skip());
    }

    /// THE FALSE-ACCUSATION GUARD. A session that received nothing cannot have
    /// ignored anything — and this is the case every Cursor session is in,
    /// because that client cannot inject. Counting it as a skip would make the
    /// metric wrong for a whole client rather than for one agent.
    #[test]
    fn a_session_given_nothing_is_never_a_skip() {
        let d = dir("nothing");
        assert!(!verdict(d.path(), "never-seen").is_skip());
        record_disposition(d.path(), "s2", "recall-applied");
        assert!(!verdict(d.path(), "s2").is_skip(), "disposition without injection is not a skip");
    }

    /// A session with no id leaves no trace rather than writing to a shared
    /// bucket where two sessions' events would blend into one false verdict.
    #[test]
    fn no_session_id_writes_nothing() {
        let d = dir("nosession");
        record_injected(d.path(), "");
        assert!(!d.path().join("recallledger").exists());
    }

}
