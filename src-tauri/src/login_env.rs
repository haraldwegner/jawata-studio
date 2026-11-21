//! The person's login-shell PATH, taken once at the studio's start (28h z21).
//!
//! A studio started from the desktop menu, the dock or an AppImage inherits the desktop session's environment, which never read the
//! person's shell start-up files. Observed 2026-11-21 on the running studio and its engine residents: the studio's PATH lacked
//! `/opt/apache-maven/bin` (and nvm's, cuda's and grok's folders) that the person's terminal has, and the engines, the brains the engine
//! starts, and so a hired developer's shell, inherit exactly that, so `mvn` was "not found" there. Terminal-launched editors have no such
//! problem because the terminal's shell already read the files.
//!
//! The fix is the one editors use: ask the person's own shell, once, what its PATH is, and add the folders the studio's PATH does not hold
//! after the studio's own, so the folders the studio and the AppImage put first keep winning.

use std::ffi::{OsStr, OsString};
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

/// The text that brackets the PATH in the shell's output, so what start-up files print around it (a banner, a warning) is not read as PATH.
const MARK: &str = "__JAWATA_LOGIN_PATH__";

/// How long the shell may take to start and answer. A shell whose start-up files wait for a terminal or a network would otherwise hold the
/// studio's start.
const LIMIT: Duration = Duration::from_secs(5);

/// `own` followed by the folders of `login` that `own` does not already hold. None where both are absent.
pub fn merge_paths(own: Option<&OsStr>, login: Option<&OsStr>) -> Option<OsString> {
    let mut folders: Vec<std::path::PathBuf> = own.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    for folder in login.map(|p| std::env::split_paths(p).collect::<Vec<_>>()).unwrap_or_default() {
        if !folder.as_os_str().is_empty() && !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    if folders.is_empty() {
        return own.map(|p| p.to_os_string());
    }
    std::env::join_paths(folders).ok()
}

/// The PATH the person's shell (`shell`, as `$SHELL` names it) has after reading its login and interactive start-up files, or None where the
/// shell cannot be started, does not answer within [`LIMIT`], or prints no PATH between the marks.
pub fn login_shell_path(shell: &OsStr) -> Option<OsString> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let script = format!("printf '%s%s%s' '{MARK}' \"$PATH\" '{MARK}'");
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let deadline = std::time::Instant::now() + LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => break None,
        }
    };
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let out = reader.join().ok()?;
    if !status?.success() && !out.contains(MARK) {
        return None;
    }
    let after = out.split_once(MARK)?.1;
    let path = after.split_once(MARK)?.0;
    (!path.is_empty()).then(|| OsString::from(path))
}

/// The PATH the studio's children (the engines, and through them the brains and the hires' shells) should start with: the studio's own,
/// then the person's login shell's. `shell` is `$SHELL`.
pub fn children_path(own: Option<&OsStr>, shell: Option<&OsStr>) -> Option<OsString> {
    let login = shell.and_then(login_shell_path);
    merge_paths(own, login.as_deref())
}

/// Set this process's PATH to [`children_path`], once, at the studio's start. Unix only: Windows has no login shell to ask, and a program
/// started from its menu has the person's own environment already.
#[cfg(unix)]
pub fn adopt_login_path() {
    let own = std::env::var_os("PATH");
    let shell = std::env::var_os("SHELL");
    if let Some(path) = children_path(own.as_deref(), shell.as_deref()) {
        std::env::set_var("PATH", path);
    }
}

#[cfg(not(unix))]
pub fn adopt_login_path() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(p: Option<OsString>) -> String {
        p.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The studio's own folders keep their place, and the login shell's that the studio lacks follow, none twice.
    #[test]
    fn the_studios_folders_come_first_and_the_login_shells_follow_without_repeats() {
        let merged = merge_paths(
            Some(OsStr::new("/tmp/.mount_x/usr/bin:/usr/bin:/home/h/.local/bin")),
            Some(OsStr::new("/opt/apache-maven/bin:/usr/bin:/home/h/.nvm/bin:/home/h/.local/bin")),
        );
        assert_eq!("/tmp/.mount_x/usr/bin:/usr/bin:/home/h/.local/bin:/opt/apache-maven/bin:/home/h/.nvm/bin", text(merged));
        assert_eq!("/usr/bin", text(merge_paths(Some(OsStr::new("/usr/bin")), None)), "no login PATH: the studio's own, unchanged");
        assert_eq!("/opt/x", text(merge_paths(None, Some(OsStr::new("/opt/x")))), "no own PATH: the login shell's");
    }

    #[cfg(unix)]
    fn fake_shell(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let file = dir.join(name);
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[cfg(unix)]
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jawata-login-env-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// What start-up files print around the PATH (a banner, a warning) is not read as PATH.
    #[cfg(unix)]
    #[test]
    fn the_path_is_read_between_the_marks_and_not_from_the_noise_around_it() {
        let d = scratch("noise");
        let shell = fake_shell(&d, "noisy-shell", "echo 'Welcome back'; echo \"__JAWATA_LOGIN_PATH__/opt/apache-maven/bin:/usr/bin__JAWATA_LOGIN_PATH__\"; echo 'bye'");
        assert_eq!("/opt/apache-maven/bin:/usr/bin", text(login_shell_path(shell.as_os_str())));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A shell that fails, prints no marks, or is not there gives nothing, and a shell that does not answer in time does not hold the start.
    #[cfg(unix)]
    #[test]
    fn a_shell_that_does_not_answer_gives_no_path_and_does_not_hold_the_start() {
        let d = scratch("silent");
        let none = fake_shell(&d, "no-marks", "echo hello");
        assert_eq!(None, login_shell_path(none.as_os_str()), "no marks, no PATH");
        let failing = fake_shell(&d, "failing", "exit 3");
        assert_eq!(None, login_shell_path(failing.as_os_str()));
        assert_eq!(None, login_shell_path(OsStr::new("/no/such/shell")));
        let slow = fake_shell(&d, "slow", "sleep 30");
        let started = std::time::Instant::now();
        assert_eq!(None, login_shell_path(slow.as_os_str()));
        assert!(started.elapsed() < Duration::from_secs(10), "the limit holds: {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// THE DEFECT, AS ITS OWN TEST: the studio's PATH has no Maven; the person's login shell has it; the PATH the studio hands its children
    /// resolves `mvn`. With the children started on the studio's own PATH, as before, it does not.
    #[cfg(unix)]
    #[test]
    fn a_program_only_the_login_shell_puts_on_the_path_is_found_by_the_children() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("mvn");
        let maven = d.join("apache-maven").join("bin");
        std::fs::create_dir_all(&maven).unwrap();
        let mvn = maven.join("jawata-fake-mvn");
        std::fs::write(&mvn, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&mvn, std::fs::Permissions::from_mode(0o755)).unwrap();
        let shell = fake_shell(&d, "login-shell", &format!("echo \"__JAWATA_LOGIN_PATH__{}:/usr/bin__JAWATA_LOGIN_PATH__\"", maven.display()));

        let own = OsStr::new("/usr/bin:/bin");
        assert_eq!(None, crate::brains::program_on_path("jawata-fake-mvn", Some(own)), "CONTROL: the studio's own PATH has no such program");
        let children = children_path(Some(own), Some(shell.as_os_str()));
        assert_eq!(Some(mvn), crate::brains::program_on_path("jawata-fake-mvn", children.as_deref()), "the children's PATH resolves it");
        let _ = std::fs::remove_dir_all(&d);
    }
}
