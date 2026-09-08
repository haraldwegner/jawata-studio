//! The dev-machine guards, folded into the product (Harald, 2026-09-08).
//!
//! Four rules lived as shell scripts under the user's own hooks directory and
//! were therefore enforced on ONE Linux machine. Every one of them states a
//! rule that is true for every client on every platform, and shell scripts
//! stopped being a delivery mechanism at the macOS and Windows port — so a
//! rule written that way after the port reached nobody but its author.
//!
//! **The paths here are HARDCODED and Linux-shaped, deliberately and
//! temporarily.** Making them portable needs canonicalisation the codebase
//! already has elsewhere, and that work is filed as its own issue rather than
//! guessed at inside a patch. What ships now is the RULE reaching the product;
//! what is missing is the rule reaching the other two platforms. Stated so a
//! reader is not misled into thinking this is done.
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

/// The workspace manifest, hardcoded with the same discovery the script had.
const FALLBACK_MANIFEST: &str = "/home/harald/CursorProjects/jawata-dev.code-workspace";

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/harald"))
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
    PathBuf::from(FALLBACK_MANIFEST)
}

/// The roots the agent may operate in: the manifest's own folders, plus the
/// scratch dir, the agent config dir, the manifest itself and the node version
/// manager's dir.
///
/// NOT PORTABLE YET, and this is the line that says so: `~/.claude` and
/// `~/.nvm` are one client's Unix layout, and the portable form is the client's
/// own resolved config dir — jawata-studio#38 owns that. The SCRATCH root is no
/// longer among the gaps: see the comment on the push below.
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
    roots.push(PathBuf::from("/tmp"));
    roots.push(home.join(".claude"));
    roots.push(home.join(".nvm"));
    roots.push(manifest);
    roots
}

fn inside(path: &str, roots: &[PathBuf]) -> bool {
    let p = Path::new(path);
    roots.iter().any(|r| p == r || p.starts_with(r))
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
    let home_dir = home();
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
        let home = home();
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
