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
pub(crate) fn normalise(path: &Path) -> PathBuf {
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

// The expensive gates are no longer a list in this file. A script says for itself
// that it is one (see `gatefile`), and the hook reads that where the command
// RUNS the script — which is why a command that merely names one is never
// refused any more. The build tools that are long by what they are (maven,
// cargo, vite, svelte-check) live there too, matched as the program, not as text.

/// A multi-minute gate run outside the bracket.
///
/// Measured 2026-08-28: two avoidable full runs cost roughly twenty minutes, because
/// a gate's output was thrown away or truncated through a filter, and the only route
/// to a detail it printed was another run. The first answer was a text rule asking
/// for a redirect; the second, a text rule asking for an absolute `cd`. Both asked the
/// caller to WRITE the capture correctly. The bracket (`bracket.rs`) IS the capture:
/// it cannot be invoked without an absolute directory, it keeps the output, and it
/// leaves a summary. So a long run that is not bracketed is refused — a redirect
/// alone no longer satisfies this, because it leaves the directory unsaid, which is
/// the second failure and the quieter one.
pub fn unbracketed_gate(command: &str) -> Option<Verdict> {
    // THE DECLARED WAY PAST, ported from `guard-gate-output.sh` (studio#41).
    //
    // This rule matches command TEXT, so it cannot tell a gate being INVOKED
    // from one mentioned as DATA — a script that tests this hook, a heredoc
    // documenting the right form, a commit message describing the rule itself.
    // All of them trip it. A guard with no declared way past becomes one people
    // route around by mangling the command until it stops matching, which is
    // worse than no guard at all: the mangling is invisible and the intent is
    // lost, while a declaration is logged and reviewable.
    //
    // Measured 2026-09-09, porting this: the shell version refused four correct
    // commands in one session — and the fourth was the commit that DOCUMENTED
    // the first, because documenting a rule requires writing the literal it
    // matches.
    if command.contains("gate-output-ok:") {
        return None;
    }
    for seg in crate::gatefile::segments(command, &crate::gatefile::start_dir()) {
        // A bracket call is the answer, not the offence: what it wraps is kept by it.
        if seg.bracketed {
            continue;
        }
        if !crate::gatefile::runs_long_gate(&seg.text, &seg.dir) {
            continue;
        }
        // A probe, not a run. EXACT words: the first version tested a substring, so
        // `-Dapi-version=2` or `--helpful` walked a real build past the rule.
        if crate::gatefile::shell_words(&seg.text)
            .iter()
            .any(|w| matches!(w.as_str(), "--version" | "-version" | "-V" | "--help" | "-h"))
        {
            continue;
        }
        let how = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<this hook binary>".to_string());
        return Some(Verdict::Deny {
            reason: format!(
                "LONG RUN OUTSIDE THE BRACKET. A multi-minute gate whose output is not kept \
                 costs a second run the moment any detail is wanted, and a run that does not \
                 say WHERE it runs can pass about the wrong repository. Run it through the \
                 bracket, which needs an absolute directory and keeps the output:\n\n    \
                 {how} {sub} --cd <absolute dir> --out /tmp/<name>.txt -- <program> [args...]\n\n\
                 It prints one line with the exit status, and writes the output to the file and \
                 a summary to <name>.txt.summary (exit status, seconds, line count, last lines). \
                 Read either as much as you like. A command that only MENTIONS a gate is not \
                 refused; if this is one that does run it and must stay as written, add \
                 `gate-output-ok: <why>` — the declaration is logged.",
                sub = crate::bracket::SUBCOMMAND,
            ),
        });
    }
    None
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

    /// A gate that declares itself in its own header, and the directory it sits in.
    fn a_declared_gate() -> (String, String) {
        let (dir, name) = crate::gatefile::testkit::script_with("# jawata-gate: long");
        (dir.display().to_string(), name)
    }

    #[test]
    fn an_uncaptured_expensive_gate_is_denied() {
        let (dir, name) = a_declared_gate();
        let cmd = format!("cd {dir} && ./{name} 4");
        assert!(unbracketed_gate(&cmd).is_some());
    }

    #[test]
    fn a_declared_exception_passes_an_unbracketed_gate() {
        // studio#41: the shell guard's escape hatch, and the reason it exists —
        // a text rule cannot tell an invocation from a mention.
        let (dir, name) = a_declared_gate();
        let cmd = format!("cd {dir} && ./{name} 4  # gate-output-ok: documenting the rule");
        assert!(unbracketed_gate(&cmd).is_none(), "a declared exception is honoured");
        // The control: the SAME command without the declaration is refused, so
        // the test cannot pass because the rule stopped firing altogether.
        assert!(
            unbracketed_gate(&format!("cd {dir} && ./{name} 4")).is_some(),
            "without the declaration the gate still refuses"
        );
    }

    #[test]
    fn a_redirect_alone_no_longer_satisfies_the_rule() {
        // The redirect keeps the output and leaves the DIRECTORY unsaid, which is the
        // second failure and the quieter one (a true PASS about the wrong product).
        // The bracket needs both, so a redirected gate is still refused and the
        // refusal teaches the bracket.
        let (dir, name) = a_declared_gate();
        let cmd = format!("cd {dir} && ./{name} 4 > /tmp/o.txt 2>&1");
        let Some(Verdict::Deny { reason }) = unbracketed_gate(&cmd) else {
            panic!("a redirected but unbracketed gate must still be refused");
        };
        assert!(reason.contains("bracket") && reason.contains("--cd"), "{reason}");
    }

    #[test]
    fn a_truncated_gate_is_refused_too() {
        let (dir, name) = a_declared_gate();
        assert!(unbracketed_gate(&format!("cd {dir} && ./{name} 4 | tail -5")).is_some());
    }

    #[test]
    fn a_bracketed_run_passes_whatever_it_runs() {
        // The bracket's head is the hook binary, so the gate it wraps is not judged
        // as a bare invocation. The control: the same gate bare is refused.
        let (dir, name) = a_declared_gate();
        let bracketed = format!(
            "/home/u/.claude/jawata-studio/jawata-hook-guard bracket --cd {dir} --out /tmp/o.txt -- ./{name} 4"
        );
        assert!(unbracketed_gate(&bracketed).is_none());
        assert!(unbracketed_gate(&format!("cd {dir} && ./{name} 4")).is_some());
        for tool in ["mvn -q install", "cargo test --lib", "vite build", "svelte-check"] {
            let cmd = format!("/h/jawata-hook-guard bracket --cd /repo --out /tmp/o.txt -- {tool}");
            assert!(unbracketed_gate(&cmd).is_none(), "{cmd}");
            assert!(unbracketed_gate(tool).is_some(), "the bare form is refused: {tool}");
        }
    }

    /// The audit's probes, as the shapes they were: every one reached the shell
    /// unjudged because the capture rule cut the command line differently from the other
    /// rules. One splitter now; each of these is a long run and each is refused.
    #[test]
    fn a_long_run_is_judged_however_the_command_line_is_cut() {
        let (dir, name) = a_declared_gate();
        let abs = format!("{dir}/{name}");
        for cmd in [
            format!("echo hi\n{abs} 4"),
            format!("false || {abs} 4"),
            format!("true & {abs} 4"),
            format!("cd {dir}\n./{name} 4"),
            format!("bash -c \"{abs} 4\""),
            format!("eval {abs} 4"),
            format!("timeout 600 {abs} 4"),
            format!("(cd {dir} && ./{name} 4)"),
            format!("x=$({abs} 4)"),
            "echo hi\nmvn -q install".to_string(),
            "cargo +nightly test".to_string(),
            "timeout 600 mvn test".to_string(),
            "(mvn test)".to_string(),
            "false || cargo test".to_string(),
            "npm run check".to_string(),
            "xargs mvn test".to_string(),
        ] {
            assert!(unbracketed_gate(&cmd).is_some(), "a long run reached the shell unjudged: {cmd:?}");
        }
    }

    /// The other half, and the reason the splitter is quote-aware: a separator inside a
    /// quoted message, or text in a heredoc body, is DATA. The old splitters handed the
    /// rule a "command" that was prose, which is the false refusal R6a set out to end.
    #[test]
    fn a_separator_inside_quotes_or_a_heredoc_is_not_a_second_command() {
        let (dir, name) = a_declared_gate();
        let abs = format!("{dir}/{name}");
        for cmd in [
            format!("git commit -m \"the guard refused it; then {abs} 4 ran\""),
            format!("echo 'a && {abs} 4'"),
            format!("cat <<'EOF'\n{abs} 4\nmvn test\nEOF"),
            format!("cat <<EOF > /tmp/doc.md\nrun {abs} 4 then mvn test\nEOF"),
            format!("echo hi # {abs} 4 and mvn test"),
        ] {
            assert!(unbracketed_gate(&cmd).is_none(), "prose was judged as a command: {cmd:?}");
        }
        // The control: the same words as REAL commands are refused.
        assert!(unbracketed_gate(&format!("echo a && {abs} 4")).is_some());
    }

    /// The exemption is for a probe, not for any command that mentions one. The audit
    /// walked a real build past the rule with `-Dapi-version=2`.
    #[test]
    fn only_a_whole_word_probe_is_exempt() {
        let (dir, name) = a_declared_gate();
        for cmd in [
            "mvn -q install -Dapi-version=2".to_string(),
            "cargo test --features api-version".to_string(),
            format!("{dir}/{name} 4 --helpful"),
        ] {
            assert!(unbracketed_gate(&cmd).is_some(), "a substring walked a build past the rule: {cmd:?}");
        }
        for cmd in ["mvn --version", "mvn -V", "cargo test --help", "mvn -h"] {
            assert!(unbracketed_gate(cmd).is_none(), "a real probe was refused: {cmd:?}");
        }
    }

    #[test]
    fn the_refusal_names_the_exact_binary_to_run() {
        let Some(Verdict::Deny { reason }) = unbracketed_gate("mvn -q install") else {
            panic!("expected a denial");
        };
        let exe = std::env::current_exe().unwrap().display().to_string();
        assert!(reason.contains(&exe), "it must print a command that runs: {reason}");
        assert!(reason.contains("gate-output-ok:"), "and the declared way past: {reason}");
    }

    #[test]
    fn a_version_probe_is_exempt() {
        assert!(unbracketed_gate("mvn --version").is_none());
    }

    /// The seven correct commands the old name list refused in one session, as the
    /// SHAPES they were: the gate's name appearing as DATA in a command that runs
    /// nothing long. The names themselves were never recorded anywhere a test can
    /// open, so these are the shapes — reading the script, counting it in a binary,
    /// documenting the rule, naming it in a message, a path in an `ls`.
    #[test]
    fn a_gate_named_as_data_is_not_a_gate_being_run() {
        let (dir, name) = a_declared_gate();
        let script = format!("{dir}/{name}");
        for cmd in [
            format!("sed -n 1,40p {script}"),
            format!("grep -c gate {script}"),
            format!("grep -rn {name} {dir}"),
            format!("cat {script}"),
            format!("ls -la {script}"),
            format!("echo 'remember to run {name} at the checkpoint'"),
            format!("git commit -m 'the guard refused {name} while documenting it'"),
            format!("head -3 {script}"),
            format!("wc -l {script}"),
        ] {
            assert!(
                unbracketed_gate(&cmd).is_none(),
                "a command that runs no gate was refused: {cmd}"
            );
        }
        // The control: RUNNING that same script is refused, so every pass above is
        // the rule reading position and not the rule being off.
        assert!(unbracketed_gate(&format!("{script} 4")).is_some());
    }

    /// The measure the list deletion exists for: nothing in what ships names one
    /// of our build scripts. A test cannot open the binary, so it opens the SOURCE
    /// the binary is built from — every non-test line of every module — and
    /// refuses the names. Comments are not compiled in, so they are not scanned.
    #[test]
    fn the_shipped_source_names_none_of_our_build_scripts() {
        // Built from parts, so this file does not contain what it forbids.
        let names: Vec<String> = ["run-", "end-to-end-", "unwired-", "abort-"]
            .iter()
            .zip(["suite.sh", "test.sh", "gate.sh", "budget.sh"])
            .map(|(a, b)| format!("{a}{b}"))
            .collect();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut hits = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            // A Windows checkout holds CRLF, and the cut below searches for "\n": without this the
            // marker is never found and the whole file, tests included, is scanned as shipped.
            let text = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
            // Everything before the TESTS MODULE. The first version cut at the first
            // `#[cfg(test)]`, which sits on a mid-file item in safety.rs and left three
            // hundred lines unscanned; it also dropped everything after a `//` on a line,
            // which hides a name written after a URL in a string. Comments are scanned too:
            // a name in a comment does not compile in, but "none of our script names appear
            // outside the tests" is the claim, and it should be true as stated.
            let cut = text.find("#[cfg(test)]\nmod tests").unwrap_or(text.len());
            let shipped = &text[..cut];
            for (n, line) in shipped.lines().enumerate() {
                for name in &names {
                    if line.contains(name.as_str()) {
                        hits.push(format!("{}:{}: {name}", path.display(), n + 1));
                    }
                }
            }
        }
        assert!(hits.is_empty(), "a build-script name ships in the binary: {hits:?}");
    }
}
