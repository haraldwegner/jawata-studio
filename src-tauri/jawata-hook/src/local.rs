//! The dev-machine guards, folded into the product (Harald, 2026-09-08).
//!
//! Four rules lived as shell scripts under the user's own hooks directory and
//! were therefore enforced on ONE Linux machine. Every one of them states a
//! rule that is true for every client on every platform, and shell scripts
//! stopped being a delivery mechanism at the macOS and Windows port — so a
//! rule written that way after the port reached nobody but its author.
//!
//! **The paths are RESOLVED, on every platform (jawata-studio#38).** The scratch
//! root comes from `std::env::temp_dir()`, the profile directory from whichever
//! variable the platform sets, and the workspace manifest from the launching
//! project or that profile — no absolute path is compiled in. Containment
//! compares NORMALISED paths, because `Path::starts_with` canonicalises nothing
//! and on Windows a path that should be blocked would otherwise read as outside
//! the tree and be allowed.
//!
//! WHAT IS TESTED, and what is not, because a C9 audit found this paragraph
//! claiming `#[cfg(windows)]` tests that the same commit had deleted — a false
//! claim in the header of the file whose subject is false claims.
//!
//! Case folding is tested by ASKING the filesystem, so it asserts on every
//! platform instead of compiling away. 8.3 short names and junctions are
//! resolved by `fs::canonicalize` and are asserted by NOTHING here: they cannot
//! be produced on the machine this is written on, and a test that skips itself
//! silently is the shape this module already removed once. They ride
//! canonicalisation's own contract, which is stated rather than demonstrated.
//!
//! Ported faithfully rather than reinvented: each message below is the one the
//! script had earned, incident by incident, and each incident is kept with it.

use std::path::{Path, PathBuf};

use crate::guard::Verdict;

/// Characters that end a path when scanning free command text. Same set the
/// shell guard used — over-matching is the safe direction, because a bogus
/// candidate resolves outside the roots and asks the user rather than
/// silently allowing.
const PATH_TERMINATORS: &[char] =
    &[' ', '"', '\'', '`', ':', ';', '|', '&', ')', '>', '\n', '\t', ','];

/// The workspace manifest's file name. The DIRECTORY is resolved, never
/// spelled: studio#38 found `/home/harald/CursorProjects/jawata-dev.code-workspace`
/// compiled in as a constant, which is one developer's filesystem shipped to
/// every install — a worse problem than the portability it was filed under,
/// because on any other machine it names a path that cannot exist and the root
/// derived from it silently matches nothing.
const MANIFEST_NAME: &str = "jawata-dev.code-workspace";

/// The user's profile directory, or `None` when no variable answers.
///
/// studio#38: **Windows sets no `HOME`.** The previous version read `HOME` and
/// fell back to a hardcoded `/home/harald`, so on Windows every root derived
/// from it pointed at a Linux path that does not exist — and the failure is
/// SILENT, because a containment test against a non-existent root simply never
/// matches, which reads exactly like a path that is legitimately outside.
///
/// `USERPROFILE` is the documented Windows spelling; `HOMEDRIVE` + `HOMEPATH`
/// is the older pair that still answers on domain-joined machines. `HOME` is
/// tried first because a Unix machine always has it and Git Bash sets it on
/// Windows too.
///
/// `None` rather than a guess is what makes this FAIL CLOSED: the caller drops
/// the roots it cannot resolve, and fewer roots means more denials. A guessed
/// home would ADD a root nobody verified, which is the direction a guard must
/// never err in.
pub(crate) fn home() -> Option<PathBuf> {
    home_from(|k| std::env::var(k).ok())
}

/// The resolution itself, over a lookup the caller supplies.
///
/// Split out so the Windows spellings can be TESTED on any machine.
/// `std::env::set_var` is process-global, so an env-mutating test races every
/// other test in the binary — and the thing under test here is which variable
/// is consulted, which needs no real environment at all.
fn home_from(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    for var in ["HOME", "USERPROFILE"] {
        if let Some(v) = get(var) {
            if !v.trim().is_empty() {
                return Some(PathBuf::from(v));
            }
        }
    }
    match (get("HOMEDRIVE"), get("HOMEPATH")) {
        (Some(d), Some(p)) if !d.trim().is_empty() && !p.trim().is_empty() => {
            Some(PathBuf::from(format!("{d}{p}")))
        }
        _ => None,
    }
}

/// Where the workspace manifest is, preferring one beside the launching
/// project. Both layouts in use: the manifest IN the project dir, and the
/// manifest in its PARENT beside sibling projects.
fn manifest_path() -> PathBuf {
    if let Ok(project) = std::env::var("CLAUDE_PROJECT_DIR") {
        let dir = PathBuf::from(&project);
        for candidate_dir in [dir.clone(), dir.parent().map(Path::to_path_buf).unwrap_or(dir)] {
            if let Ok(entries) = std::fs::read_dir(&candidate_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("code-workspace") {
                        return p;
                    }
                }
            }
        }
    }
    manifest_under(home())
}

/// The fallback manifest, under a supplied profile directory.
///
/// studio#38: DERIVED, not spelled. The previous constant was one machine's
/// absolute Linux path; anywhere else it named a file that cannot exist, and a
/// root that cannot exist matches nothing while looking like a root.
///
/// Takes the home rather than reading it, because on the machine that wrote the
/// constant the derived value is BYTE-IDENTICAL to it — so a test asserting the
/// result here would pass with the constant restored, and prove nothing. The
/// only way to see the derivation is to hand it a different home.
fn manifest_under(home: Option<PathBuf>) -> PathBuf {
    home.map(|h| h.join("CursorProjects").join(MANIFEST_NAME))
        .unwrap_or_else(|| PathBuf::from(MANIFEST_NAME))
}

/// The roots the agent may operate in: the manifest's own folders, plus the
/// scratch dir, the agent config dir, the manifest itself and the node version
/// manager's dir.
///
/// PORTABLE (jawata-studio#38). Every root is RESOLVED rather than spelled: the
/// scratch dir from `std::env::temp_dir()`, the profile directory from the
/// platform's own variable, the manifest from the launching project or the
/// profile. `.claude` and `.nvm` are the client's directory NAMES under that
/// profile, which is the same layout on every platform — the defect was the
/// hardcoded `/home/harald` they were joined to, not the names.
pub fn workspace_roots() -> Vec<PathBuf> {
    let home = home();
    let manifest = manifest_path();
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(text) = std::fs::read_to_string(&manifest) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(folders) = value.get("folders").and_then(|f| f.as_array()) {
                let base = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
                for folder in folders {
                    let Some(p) = folder.get("path").and_then(|p| p.as_str()) else {
                        continue;
                    };
                    let path = PathBuf::from(p);
                    roots.push(if path.is_absolute() { path } else { base.join(path) });
                }
            }
        }
    }
    // Never brick: an unparseable manifest confines to the launch project
    // rather than opening the whole home directory.
    if roots.is_empty() {
        if let Ok(project) = std::env::var("CLAUDE_PROJECT_DIR") {
            roots.push(PathBuf::from(project));
        }
    }
    // The scratch root is RESOLVED, not spelled. `/tmp` is the Unix literal and
    // stays because this workspace declares it, but what a program actually
    // gets is the PLATFORM temp directory — `/var/folders/<...>/T` on macOS,
    // `%TEMP%` on Windows — and containment must know that directory by the
    // name the platform gives it.
    //
    // Hardcoding `/tmp` alone made this rule pass on Linux BY COINCIDENCE: a
    // process whose temp dir happens to live under `/tmp` is inside the root by
    // accident of spelling, not because the rule understood it. On macOS the
    // same code denied every temp path, and it took the release CI of v4.1.6 to
    // say so — seven integration tests, each denied on its own fixture.
    roots.push(std::env::temp_dir());
    // The Unix literal stays, ONLY on Unix, ONLY when it exists, and only when
    // it is not already the resolved root. A C9 audit was right that the first
    // version added a root nobody verified — the very direction `home()`'s doc
    // says a guard must never err in — and that no test covered it.
    //
    // It is not dropped outright because with TMPDIR pointing elsewhere `/tmp`
    // is still a real scratch directory on this platform, and denying writes to
    // it would refuse correct work.
    #[cfg(unix)]
    {
        let unix_tmp = PathBuf::from("/tmp");
        if unix_tmp.is_dir() && !roots.contains(&unix_tmp) {
            roots.push(unix_tmp);
        }
    }
    // studio#38: the two agent-config roots exist only when the profile
    // directory resolved. An unresolvable home drops them rather than guessing,
    // which FAILS CLOSED — fewer roots means more denials.
    if let Some(home) = home.as_ref() {
        roots.push(home.join(".claude"));
        roots.push(home.join(".nvm"));
    }
    roots.push(manifest);
    roots
}

/// Is `path` inside one of the roots?
///
/// studio#38: COMPARED ON NORMALISED PATHS, because `Path::starts_with` is a
/// component-wise TEXTUAL test that canonicalises nothing. On Windows that
/// fails in the direction a guard must never fail: a path that should be
/// BLOCKED reads as outside the tree and is allowed. Four ways, all real —
/// case (`C:\Users` vs `c:\users`), the drive letter's own case, 8.3 short
/// names (`RUNNER~1`), and junctions.
///
/// [`normalise`] resolves what EXISTS and folds case where the platform does,
/// so the comparison is between two paths the operating system would agree are
/// the same place.
fn inside(path: &str, roots: &[PathBuf]) -> bool {
    let p = normalise(Path::new(path));
    roots.iter().any(|r| {
        let r = normalise(r);
        p == r || p.starts_with(&r)
    })
}

/// A path in the form the containment test compares.
///
/// Canonicalises the longest ANCESTOR that exists and re-appends the rest: a
/// guard is asked about files that have not been created yet, so
/// `fs::canonicalize` on the whole path would fail exactly when it is needed.
/// That resolves 8.3 short names and junctions, which are filesystem facts no
/// string rule can reach.
///
/// Case is then folded on the platforms whose filesystems ignore it. Doing that
/// unconditionally would be wrong on Linux, where two names differing only in
/// case ARE two files.
fn normalise(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let resolved = loop {
        match std::fs::canonicalize(existing) {
            Ok(c) => break Some(c),
            Err(_) => match (existing.parent(), existing.file_name()) {
                (Some(parent), Some(name)) => {
                    tail.push(name.to_os_string());
                    existing = parent;
                }
                _ => break None,
            },
        }
    };
    let mut out = match resolved {
        Some(mut c) => {
            for name in tail.iter().rev() {
                c.push(name);
            }
            c
        }
        None => path.to_path_buf(),
    };
    if folds_case() {
        if let Some(s) = out.to_str() {
            out = PathBuf::from(s.to_lowercase());
        }
    }
    out
}

/// Does THIS filesystem ignore case?
///
/// Asked once and cached. The first version read `cfg!(target_os = "linux")` —
/// the OS-name PROXY that this module's own tests were rewritten to refuse, and
/// a C9 architect watch caught the production code still using it. It is wrong
/// in the dangerous direction on a case-sensitive volume under a case-folding
/// OS: over-folding makes an outside path compare EQUAL to a root, which
/// ALLOWS, and this module says a guard must never err that way.
///
/// The probe creates nothing: it asks whether the temp directory answers to a
/// differently-cased spelling of its own name. Unanswerable (no temp dir, a
/// name with no letters) is read as DOES NOT FOLD, which is the stricter
/// reading — it denies more.
fn folds_case() -> bool {
    static FOLDS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOLDS.get_or_init(|| {
        let dir = std::env::temp_dir();
        let Some(s) = dir.to_str() else { return false };
        let flipped = if s.chars().any(|c| c.is_ascii_lowercase()) {
            s.to_uppercase()
        } else {
            s.to_lowercase()
        };
        if flipped == s {
            return false;
        }
        std::fs::metadata(&dir).is_ok() && std::fs::metadata(PathBuf::from(flipped)).is_ok()
    })
}

/// Expand the three home spellings a shell would expand, so the check polices a
/// BOUNDARY rather than a SPELLING.
///
/// Until 2026-08-21 the shell guard matched only the fully-expanded form, so
/// `ls ~/.local/share/...` walked straight past it while the absolute spelling
/// of the same directory was blocked — `cat ~/.ssh/id_rsa` was allowed. Found
/// when a fresh-context reader opened a directory the session had reported one
/// message earlier as unreachable.
fn expand_home(text: &str, home: &str) -> String {
    text.replace("${HOME}/", &format!("{home}/"))
        .replace("$HOME/", &format!("{home}/"))
        .replace("~/", &format!("{home}/"))
}

/// Every absolute path under the user's home that the text names. System paths
/// (/usr, /bin, /etc) are toolchain rather than user data and are never
/// policed here.
fn home_paths_in(text: &str, home: &str) -> Vec<String> {
    let needle = format!("{home}/");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let tail = &rest[at..];
        let end = tail.find(PATH_TERMINATORS).unwrap_or(tail.len());
        out.push(tail[..end].to_string());
        rest = &tail[end.max(1)..];
    }
    out
}

/// Git introspection that routinely exposes a credential.
///
/// A remote address commonly embeds `user:TOKEN@host`, and printing one puts it
/// in the transcript for good. This blocked a real leak on 2026-05-19.
///
/// THE CONFIG HALF POLICES THE REASON, NOT THE SPELLING, and that is the
/// correction of 2026-09-08 (Harald: "this should be prohibited as well"). The
/// first version listed the argument forms it had seen — the two dump flags and
/// the literal `remote.` WITH the dot — so a `--get-regexp remote` query printed
/// `remote.origin.url` and its token while satisfying none of the three. A list of
/// spellings is only ever as complete as the day it was written, and git has
/// many ways to ask one question.
///
/// So the rule became: a whole-config dump, or ANY config read that names a
/// remote or a URL, however it asks. A read of `user.name` is untouched,
/// because it cannot reach a credential — a refusal has to stay narrow enough
/// to be right, or it teaches people to route around it.
fn exposes_credentials(text: &str) -> bool {
    let t = text.replace(char::is_whitespace, " ");
    let squashed = t.split(' ').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let remote_probe = ["git remote -v", "git remote --verbose", "git remote show",
                        "git remote get-url"];
    if remote_probe.iter().any(|p| squashed.contains(p)) {
        return true;
    }
    if !squashed.contains("git config") {
        return false;
    }
    // A whole-config dump always carries the remotes with it.
    if squashed.contains("--list") || squashed.contains(" -l") {
        return true;
    }
    // Otherwise: does the question reach a remote address at all? The bare
    // word catches the regexp form, which the dotted literal missed; "url"
    // catches the urlmatch form and a regexp over URLs.
    squashed.contains("remote") || squashed.contains("url")
}

/// The whole-payload verdict: the rules that police PATHS and credentials,
/// which see the command AND the file arguments of Read/Edit/Write. The
/// shell-command rules stay in [`crate::guard::judge`].
pub fn judge_payload(text: &str) -> Verdict {
    if exposes_credentials(text) {
        return Verdict::Deny {
            reason: "git remote/config introspection is off-limits — a remote address \
                     commonly embeds a credential (user:TOKEN@host) and printing one puts it \
                     in the transcript permanently. Ask the user if you genuinely need remote \
                     information."
                .to_string(),
        };
    }
    // studio#38: no resolvable profile directory means the home-path rules have
    // no subject. Allowing is correct here and not a hole — the OTHER rules in
    // this module still run, and inventing a home would police a tree nobody
    // named.
    let Some(home_dir) = home() else {
        return Verdict::Allow;
    };
    let Some(home_str) = home_dir.to_str() else {
        return Verdict::Allow;
    };
    let roots = workspace_roots();
    let scan = expand_home(text, home_str);
    for path in home_paths_in(&scan, home_str) {
        if !inside(&path, &roots) {
            return Verdict::Deny {
                reason: format!(
                    "'{path}' is outside the declared workspace. The agent works inside the \
                     workspace's own project folders, plus the scratch and agent-config \
                     directories; everything else in the user's home — sibling projects, \
                     private repositories, dotfiles — is off-limits. If this path is genuinely \
                     needed, the user must approve the call."
                ),
            };
        }
    }
    Verdict::Allow
}

/// The expensive gates: anything here takes minutes, so its output is worth
/// more than the transcript space it saves.
const EXPENSIVE_GATES: &[&str] = &[
    "run-suite.sh", "end-to-end-test.sh", "unwired-gate.sh", "abort-budget.sh",
    "svelte-check",
];

fn mentions_expensive_gate(segment: &str) -> bool {
    if EXPENSIVE_GATES.iter().any(|g| segment.contains(g)) {
        return true;
    }
    let words: Vec<&str> = segment.split_whitespace().collect();
    words.iter().any(|w| {
        let bare = w.rsplit(['/', '\\']).next().unwrap_or(w);
        bare == "mvn" || bare == "mvnw"
    }) || segment.contains("cargo test")
        || segment.contains("cargo build")
        || segment.contains("vite build")
}

/// A multi-minute gate whose output is thrown away, or truncated through a
/// filter. Either way the only route to a detail it printed is another run.
///
/// Measured 2026-08-28: two avoidable full runs cost roughly twenty minutes.
pub fn uncaptured_gate(command: &str) -> Option<Verdict> {
    for segment in command.replace("&&", ";").split(';') {
        if !mentions_expensive_gate(segment) {
            continue;
        }
        if segment.contains("--version") || segment.contains("-version")
            || segment.contains("--help")
        {
            continue;
        }
        // Captured? A redirect to a file, or tee. That is the whole requirement.
        if segment.contains('>') || segment.split_whitespace().any(|w| w == "tee") {
            continue;
        }
        let truncated = ["head", "tail", "sed", "awk", "cut", "wc"].iter().any(|f| {
            segment
                .split('|')
                .skip(1)
                .any(|piped| piped.split_whitespace().next() == Some(f))
        });
        let lead = if truncated {
            "GATE OUTPUT TRUNCATED AND DISCARDED — capture it to a file instead. What the \
             filter does not print is GONE, and the only way back is to run the gate again."
        } else {
            "GATE OUTPUT NOT CAPTURED — redirect it to a file. A multi-minute gate whose \
             output is not kept costs a second run the moment any detail is wanted."
        };
        return Some(Verdict::Deny {
            reason: format!(
                "{lead} Do this instead, so every later question is answerable from the file: \
                 `<gate> > /tmp/out.txt 2>&1; echo \"exit=$?\"` then read the file. \
                 head/tail/grep over the FILE are unrestricted — this refuses only producing \
                 the output and discarding it in the same command."
            ),
        });
    }
    None
}

/// A gate invoked by a path that does not fix the working directory.
///
/// Measured 2026-08-28, five times in one session the working directory was not
/// what the command assumed. Four failed loudly. The fifth did not: the command
/// landed in a neighbouring repository, ran ITS gate, and printed a real PASS
/// about the wrong product — one step from being recorded as this product's
/// result. An ABSOLUTE script path is caught too, and that is the point: an
/// absolute path does not set the working directory either, and a suite invoked
/// that way produced fourteen phantom failures, every one a fixture resolved
/// from the wrong root.
pub fn gate_without_absolute_cd(command: &str) -> Option<Verdict> {
    let names_gate = command.contains("build/") && command.contains(".sh")
        || command.contains("-f build/pom.xml")
        || command.contains("-f ") && command.contains("build/pom.xml");
    if !names_gate {
        return None;
    }
    let has_absolute_cd = command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0] == "cd" && w[1].starts_with('/'));
    if has_absolute_cd {
        return None;
    }
    Some(Verdict::Deny {
        reason: "RELATIVE GATE PATH WITH NO ABSOLUTE cd — say which repository you mean. A \
                 workspace holds several projects and the shell's working directory is not \
                 reliably where you left it, so a relative gate path runs whatever repository \
                 the shell is in right now. The loud failure is a missing file; the quiet one \
                 is finding a DIFFERENT project's gate of the same name, running it, and \
                 passing — a true result about the wrong product. Prefix the command with an \
                 absolute cd."
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied(v: Verdict) -> bool {
        matches!(v, Verdict::Deny { .. })
    }

    #[test]
    fn git_remote_introspection_is_denied() {
        assert!(denied(judge_payload("git remote -v")));
        assert!(denied(judge_payload("cd /repo && git remote get-url origin")));
        assert!(denied(judge_payload("git config --list")));
    }

    #[test]
    fn ordinary_git_is_untouched() {
        // The rule is about EXPOSING a remote address, not about git.
        assert!(!denied(judge_payload("git status --porcelain")));
        assert!(!denied(judge_payload("git log --oneline -3")));
        assert!(!denied(judge_payload("git config user.name")));
    }

    #[test]
    fn every_config_read_that_can_reach_a_remote_address_is_refused() {
        // The gap this closes, measured on the deployed 4.1.6 binary before
        // the fix: the regexp form was ALLOWED while its dotted sibling was
        // denied — same question, same output, different spelling. Each of
        // these prints the remote URL, and the URL is where the token is.
        for q in [
            "git config --get-regexp remote",
            "git config --get-regexp url",
            "git config --get-urlmatch http https://x",
            "git config --get remote.origin.url",
            "git config --list",
            "git config -l",
        ] {
            assert!(exposes_credentials(q), "must refuse: {q}");
        }
    }

    #[test]
    fn a_config_read_that_cannot_reach_a_credential_is_left_alone() {
        // The other half of the claim, and the reason the rule is not just
        // "any config read": a refusal wide enough to catch ordinary work
        // gets routed around, and then it guards nothing at all.
        for q in [
            "git config --get user.name",
            "git config user.email me@example.com",
            "git config --get core.editor",
            "git status --short",
        ] {
            assert!(!exposes_credentials(q), "must allow: {q}");
        }
    }

    /// studio#38: Windows sets no `HOME`, so the profile must be found by the
    /// spelling the PLATFORM uses. Driven through a supplied lookup rather than
    /// the real environment: `set_var` is process-global and would race every
    /// other test in this binary, and what is under test is which variable is
    /// consulted — a question no real environment is needed to answer.
    #[test]
    fn the_profile_directory_is_found_by_each_platforms_own_spelling() {
        let only = |want: &'static str, val: &'static str| {
            move |k: &str| (k == want).then(|| val.to_string())
        };

        assert_eq!(
            Some(PathBuf::from("/home/someone")),
            home_from(only("HOME", "/home/someone")),
            "Unix, and Git Bash on Windows"
        );
        assert_eq!(
            Some(PathBuf::from("C:\\Users\\someone")),
            home_from(only("USERPROFILE", "C:\\Users\\someone")),
            "the documented Windows spelling — the case that was BROKEN"
        );
        assert_eq!(
            Some(PathBuf::from("C:\\Users\\someone")),
            home_from(|k| match k {
                "HOMEDRIVE" => Some("C:".into()),
                "HOMEPATH" => Some("\\Users\\someone".into()),
                _ => None,
            }),
            "the older pair, still answering on domain-joined machines"
        );

        // FAILS CLOSED. The previous version answered `/home/harald` here — a
        // path that exists on exactly one machine, so every root derived from it
        // matched nothing while looking like a root.
        assert_eq!(None, home_from(|_| None), "no variable answers");
        assert_eq!(
            None,
            home_from(|k| (k == "HOME").then(|| "   ".to_string())),
            "an empty value is not an answer"
        );

        // AND THE ORDER MATTERS: a machine setting both must not be decided by
        // whichever the loop happened to reach first.
        assert_eq!(
            Some(PathBuf::from("/home/unix")),
            home_from(|k| match k {
                "HOME" => Some("/home/unix".into()),
                "USERPROFILE" => Some("C:\\Users\\win".into()),
                _ => None,
            }),
            "HOME wins where both are set"
        );
    }

    /// studio#38: no absolute path of one developer's machine is compiled in.
    ///
    /// THE FIRST VERSION OF THIS TEST WAS VOID, and the run said so. It called
    /// `manifest_path()` and asserted the result did not contain
    /// `/home/harald` — but this IS that machine, so the correctly derived
    /// value is byte-identical to the constant it replaced. The assertion
    /// would have passed with the hardcoded constant restored: it was reading
    /// a coincidence of the environment, not the change.
    ///
    /// So the home is FORCED TO DIFFER first, and only then is the derivation
    /// asserted — which is the only arrangement in which it can fail.
    #[test]
    fn the_manifest_path_is_derived_from_the_profile_not_compiled_in() {
        let elsewhere = manifest_under(Some(PathBuf::from("/somewhere/else")));
        assert_eq!(
            PathBuf::from("/somewhere/else/CursorProjects").join(MANIFEST_NAME),
            elsewhere,
            "the manifest follows the profile it is given"
        );
        assert!(
            !elsewhere.to_string_lossy().contains("harald"),
            "and carries nothing of the machine that wrote the old constant: {elsewhere:?}"
        );

        // A Windows-shaped assertion was here and could not hold on Linux:
        // `Path::join` uses THIS platform's separator, so `C:\Users\someone`
        // is one component and the result mixes separators. That is a fact
        // about the host, not about the change. There is no Windows-only test
        // to defer to: the case behaviour is asserted by the filesystem probe
        // below, which runs everywhere.

        // No profile at all: the bare name, which resolves relative to wherever
        // the hook runs rather than to a stranger's home directory.
        assert_eq!(PathBuf::from(MANIFEST_NAME), manifest_under(None));
    }

    /// studio#38: containment compares NORMALISED paths.
    ///
    /// `Path::starts_with` is component-wise and TEXTUAL — it canonicalises
    /// nothing — so a path reaching a root by a different but equivalent
    /// spelling reads as OUTSIDE and is allowed. This exercises the mechanism on
    /// every platform using a symlink, which is the one form of it Linux can
    /// produce; the Windows-only spellings are the test below.
    #[test]
    fn containment_follows_a_link_to_the_same_place() {
        let base = std::env::temp_dir().join(format!("jawata-38-{}", std::process::id()));
        let real = base.join("real");
        let _ = std::fs::create_dir_all(&real);
        let link = base.join("link");
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&real, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&real, &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;

        // NEVER EMPTY. A C9 audit found the first version asserting nothing at
        // all on Windows — `made` was hardcoded false there, which is the
        // silent-skip shape this file removed elsewhere in the same commit. If
        // no link can be made (Windows without the privilege, say), the
        // NON-link half still runs, so the test always claims something.
        if !made {
            let direct = real.join("inside.txt");
            assert!(
                inside(&direct.to_string_lossy(), &[real.clone()]),
                "no link could be made here, so at least the direct path must be inside"
            );
        }
        if made {
            let target = link.join("inside.txt");
            assert!(
                inside(&target.to_string_lossy(), &[real.clone()]),
                "a path reaching the root through a link is INSIDE it: {target:?} vs {real:?}"
            );
            // THE CONTROL: a genuinely outside path is still outside, so the
            // assertion above is not simply "everything is inside".
            let elsewhere = base.join("not-under-real").join("x.txt");
            assert!(
                !inside(&elsewhere.to_string_lossy(), &[real.clone()]),
                "and normalising must not make everything match: {elsewhere:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// studio#38: one place spelled two ways is one place — asserted against
    /// what the FILESYSTEM does, not against the operating system's name.
    ///
    /// This began as `#[cfg(windows)]` and that was the weaker shape twice
    /// over. A compiled-out test cannot fail, so a wrong `cfg` deletes it in
    /// silence; and the OS name is a PROXY for the property that matters —
    /// macOS folds case too, so gating on Windows would have skipped a
    /// platform where the defect is equally real.
    ///
    /// So the filesystem is ASKED. Where it folds case, containment must fold
    /// with it or a path that should be BLOCKED reads as outside the tree and
    /// is allowed. Where it does not, two spellings are two places and must
    /// stay that way — on Linux `/x/A` and `/x/a` are different files, and a
    /// guard that conflated them would deny a directory nobody named.
    ///
    /// Both branches assert, so neither platform gets a free pass, and the
    /// probe reports which branch ran.
    #[test]
    fn one_place_spelled_two_ways_is_one_place_where_the_filesystem_says_so() {
        let base = std::env::temp_dir().join(format!("jawata-38-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("Root");
        std::fs::create_dir_all(&root).expect("scratch");

        // THE PROBE: reach the directory by a differently-cased name. If that
        // resolves, this filesystem ignores case.
        let folds = std::fs::metadata(base.join("root")).is_ok();

        let lower = base.join("root");
        let target_via_lower = lower.join("f.txt");
        let contained = inside(&target_via_lower.to_string_lossy(), &[root.clone()]);

        if folds {
            assert!(
                contained,
                "this filesystem ignores case, so {target_via_lower:?} IS inside {root:?} —                  a textual comparison would call it outside and ALLOW it"
            );
        } else {
            assert!(
                !contained,
                "this filesystem distinguishes case, so {target_via_lower:?} is a different                  place from {root:?} and must not be conflated"
            );
        }

        // THE CONTROL, on both branches: a genuinely different tree is outside
        // either way, so neither assertion above is "everything matches".
        let elsewhere = base.join("Other").join("f.txt");
        assert!(
            !inside(&elsewhere.to_string_lossy(), &[root]),
            "a different directory is outside whatever the filesystem does about case"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_scratch_root_is_the_platform_temp_dir_and_not_the_unix_spelling() {
        // WHAT THIS CAN AND CANNOT SEE, stated because the difference IS the
        // defect. On macOS the temp directory is `/var/folders/<...>/T` and on
        // Windows `%TEMP%`, so a rule knowing only `/tmp` refuses a program's
        // own scratch file everywhere but Linux. On LINUX with the default
        // TMPDIR, `temp_dir()` IS `/tmp` — so removing the resolved push leaves
        // this test GREEN here. Measured: the mutation was run and it did not
        // fail, which is what a mutation staying green is for.
        //
        // So this STATES the property; it does not discriminate on this host.
        // The discriminator is the integration suite with the scratch dir moved
        // off `/tmp`, which reproduces the macOS failure on Linux exactly:
        //
        //     TMPDIR=/var/tmp/probe cargo test --test answering_is_not_a_launch_pad
        //
        // With the resolved push: 8 passed. Without it: 1 passed, 7 failed —
        // the same seven the v4.1.6 macOS release job reported.
        let roots = workspace_roots();
        let scratch = std::env::temp_dir();
        assert!(
            roots.iter().any(|r| *r == scratch),
            "the platform temp dir {scratch:?} must be a root; got {roots:?}"
        );
        // And a file in it is INSIDE, which is the property the rule is for —
        // membership of the list is the mechanism, not the claim.
        let file = scratch.join("jawata-scratch-probe.txt");
        assert!(
            inside(file.to_str().unwrap(), &roots),
            "a file in the platform scratch dir must be inside the workspace"
        );
    }

    #[test]
    fn the_home_spellings_are_expanded_before_the_boundary_is_checked() {
        // The defect this closes: matching the expanded form ONLY meant the
        // guard enforced a spelling, so the tilde form of a blocked directory
        // walked straight past it.
        let home = home().expect("the test machine has a profile directory");
        let home_str = home.to_str().unwrap();
        let expanded = expand_home("cat ~/.ssh/id_rsa", home_str);
        assert!(
            expanded.contains(&format!("{home_str}/.ssh")),
            "the tilde form must be expanded before the check: {expanded}"
        );
    }

    #[test]
    fn a_path_scan_stops_at_a_shell_delimiter() {
        let found = home_paths_in("/h/u/a/one.txt; cat /h/u/b", "/h/u");
        assert_eq!(vec!["/h/u/a/one.txt".to_string(), "/h/u/b".to_string()], found);
    }

    #[test]
    fn system_paths_are_never_policed() {
        // Toolchain, not user data.
        assert!(!denied(judge_payload("ls /usr/lib/jvm && cat /etc/hosts")));
    }

    #[test]
    fn an_uncaptured_expensive_gate_is_denied() {
        let cmd = format!("cd /repo && ./build/run-{}.sh 4", "suite");
        assert!(uncaptured_gate(&cmd).is_some());
    }

    #[test]
    fn a_captured_gate_passes() {
        let cmd = format!("cd /repo && ./build/run-{}.sh 4 > /tmp/o.txt 2>&1", "suite");
        assert!(uncaptured_gate(&cmd).is_none());
    }

    #[test]
    fn a_truncated_gate_is_denied_with_its_own_wording() {
        let cmd = format!("./build/run-{}.sh 4 | tail -5", "suite");
        let Some(Verdict::Deny { reason }) = uncaptured_gate(&cmd) else {
            panic!("expected a denial");
        };
        assert!(reason.contains("TRUNCATED"), "the two refusals read differently: {reason}");
    }

    #[test]
    fn a_version_probe_is_exempt() {
        assert!(uncaptured_gate("mvn --version").is_none());
    }

    #[test]
    fn a_relative_gate_path_needs_an_absolute_cd() {
        let cmd = format!("./build/run-{}.sh 4 > /tmp/o.txt 2>&1", "suite");
        assert!(gate_without_absolute_cd(&cmd).is_some());
        let ok = format!("cd /home/u/repo && ./build/run-{}.sh 4 > /tmp/o.txt 2>&1", "suite");
        assert!(gate_without_absolute_cd(&ok).is_none());
    }

    #[test]
    fn an_absolute_script_path_still_needs_the_cd() {
        // The first version of this rule caught only a relative `./build/...`,
        // so an absolute script path satisfied it — and an absolute path does
        // NOT set the working directory. That is the fourteen-phantom-failure
        // case, and it must still be refused.
        assert!(gate_without_absolute_cd("/home/u/repo/build/run-suite.sh 4 > /tmp/o.txt").is_some());
    }
}
