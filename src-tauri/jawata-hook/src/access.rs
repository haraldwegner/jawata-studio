//! The access policy, in the hook (Sprint 28h, S13, D20, ACL1 to ACL7 and ACL9).
//!
//! **This is the deliberate duplicate** the design names (`ARCHITECTURE-28h.md`, D20): the hook runs once per tool call and starting a JVM
//! per call is not affordable, so it carries a parser of the SAME policy file the engine reads. Both run one shared case file
//! (`tests/fixtures/access-cases.json`, a copy of the engine's `access/cases.json`, checked byte for byte against it when the engine's
//! repository sits beside this one), so the two cannot drift. When the hook retires (D7) the engine's parser is the only one.
//!
//! What a policy is, and what a decision depends on, is written once, in the engine's `AccessPolicy`: one file the person owns with a list of
//! allowed places and a list of denied ones; the deny list wins inside an allowed place; a place on neither list is refused; nothing is read
//! from a name; a file that cannot be used refuses every place and says which way it was unusable. This module is that, in Rust.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The answer for one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub reason: String,
}

impl Decision {
    fn allow() -> Self {
        Self { allowed: true, reason: String::new() }
    }
    fn deny(reason: String) -> Self {
        Self { allowed: false, reason }
    }
}

/// A policy read from its file. An unusable file is a policy that refuses everything and carries the reason.
#[derive(Debug, Clone)]
pub struct AccessPolicy {
    allow: Vec<PathBuf>,
    deny: Vec<PathBuf>,
    unusable: Option<String>,
}

impl AccessPolicy {
    /// A policy that refuses every place and says `reason` when it does.
    pub fn unusable(reason: String) -> Self {
        Self { allow: vec![], deny: vec![], unusable: Some(reason) }
    }

    /// Reads the file. Never fails: whatever is wrong with it comes back as a policy that refuses everything.
    pub fn read(file: &Path, variables: &HashMap<String, Vec<PathBuf>>) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Self::unusable(format!("there is no access policy at {}, so no place may be named", file.display()))
            }
            Err(e) => {
                return Self::unusable(format!(
                    "the access policy at {} could not be read ({e}), so no place may be named",
                    file.display()
                ))
            }
        };
        let root: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                return Self::unusable(format!(
                    "the access policy at {} is not valid JSON ({e}), so no place may be named",
                    file.display()
                ))
            }
        };
        if !root.is_object() {
            return Self::unusable(format!("the access policy at {} is not a JSON object, so no place may be named", file.display()));
        }
        let mut allow = Vec::new();
        let mut deny = Vec::new();
        for list in ["allow", "deny"] {
            let Some(entries) = root.get(list).and_then(Value::as_array) else {
                return Self::unusable(format!(
                    "the access policy at {} has no \"{list}\" list, so no place may be named",
                    file.display()
                ));
            };
            for entry in entries {
                let Some(text) = entry.as_str() else {
                    return Self::unusable(format!(
                        "the access policy at {} has an entry in \"{list}\" that is not text, so no place may be named",
                        file.display()
                    ));
                };
                let into = if list == "allow" { &mut allow } else { &mut deny };
                if let Some(problem) = expand(text, variables, into) {
                    return Self::unusable(format!(
                        "the access policy at {}: entry \"{text}\" in \"{list}\" {problem}, so no place may be named",
                        file.display()
                    ));
                }
            }
        }
        Self { allow, deny, unusable: None }
    }

    /// Whether the policy could be read.
    pub fn usable(&self) -> bool {
        self.unusable.is_none()
    }

    /// Whether `place` may be named.
    pub fn check(&self, place: &Path) -> Decision {
        if let Some(reason) = &self.unusable {
            return Decision::deny(reason.clone());
        }
        let at = crate::local::normalise(place);
        for denied in &self.deny {
            if at.starts_with(denied) {
                return Decision::deny(format!("'{}' is on the access policy's deny list ({})", place.display(), denied.display()));
            }
        }
        for allowed in &self.allow {
            if at.starts_with(allowed) {
                return Decision::allow();
            }
        }
        Decision::deny(format!("'{}' is outside every place the access policy allows", place.display()))
    }
}

/// Adds the places `entry` stands for to `into`; returns what is wrong with the entry, or none.
fn expand(entry: &str, variables: &HashMap<String, Vec<PathBuf>>, into: &mut Vec<PathBuf>) -> Option<String> {
    let mut name: Option<String> = None;
    let mut rest = entry;
    while let Some(at) = rest.find("${") {
        let after = &rest[at + 2..];
        let Some(end) = after.find('}') else { break };
        let found = &after[..end];
        let valid = !found.is_empty()
            && found.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && found.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if valid {
            if !variables.contains_key(found) {
                return Some(format!("uses ${{{found}}}, which is not defined"));
            }
            if name.as_deref().is_some_and(|n| n != found) {
                return Some("uses more than one variable".to_string());
            }
            name = Some(found.to_string());
        }
        rest = &after[end + 1..];
    }
    match name {
        None => add_if_absolute(entry, into),
        Some(name) => {
            for value in &variables[&name] {
                let replaced = entry.replace(&format!("${{{name}}}"), &value.to_string_lossy());
                if let Some(problem) = add_if_absolute(&replaced, into) {
                    return Some(problem);
                }
            }
            None
        }
    }
}

fn add_if_absolute(entry: &str, into: &mut Vec<PathBuf>) -> Option<String> {
    let path = PathBuf::from(entry);
    if !path.is_absolute() {
        return Some("is not an absolute path".to_string());
    }
    into.push(crate::local::normalise(&path));
    None
}

const PATH_KEYS: &[&str] = &[
    "file_path", "filePath", "path", "notebook_path", "projectPath", "cwd", "workingDirectory", "directory", "dir", "file", "root",
];
const PATH_LIST_KEYS: &[&str] = &["paths", "files"];

/// The places one tool call names (ACL7): the path-valued arguments, and the whole-word paths of a shell `command`. Prose names nothing.
/// Returns the places and the relative paths no base could place.
pub fn places_named(arguments: &Value, bases: &[PathBuf], home: Option<&Path>) -> (Vec<PathBuf>, Vec<String>) {
    let mut places = Vec::new();
    let mut unresolved = Vec::new();
    let Some(object) = arguments.as_object() else { return (places, unresolved) };
    for (key, value) in object {
        if PATH_KEYS.contains(&key.as_str()) {
            if let Some(text) = value.as_str() {
                place(text, bases, home, &mut places, &mut unresolved);
            }
        } else if PATH_LIST_KEYS.contains(&key.as_str()) {
            for item in value.as_array().into_iter().flatten() {
                if let Some(text) = item.as_str() {
                    place(text, bases, home, &mut places, &mut unresolved);
                }
            }
        } else if key == "command" {
            if let Some(text) = value.as_str() {
                for word in shell_words(text) {
                    if let Some(candidate) = path_inside(&word) {
                        place(&candidate, bases, home, &mut places, &mut unresolved);
                    }
                }
            }
        }
    }
    (places, unresolved)
}

fn path_inside(word: &str) -> Option<String> {
    let mut candidate = word;
    if candidate.starts_with('-') {
        let equals = candidate.find('=')?;
        candidate = &candidate[equals + 1..];
    }
    if candidate.is_empty() || candidate.chars().any(char::is_whitespace) {
        return None;
    }
    let chars: Vec<char> = candidate.chars().collect();
    let windows = chars.len() > 2 && chars[0].is_alphabetic() && chars[1] == ':' && (chars[2] == '\\' || chars[2] == '/');
    let absolute = candidate.starts_with('/')
        || candidate == "~"
        || candidate.starts_with("~/")
        || candidate == "$HOME"
        || candidate.starts_with("$HOME/")
        || candidate == "${HOME}"
        || candidate.starts_with("${HOME}/");
    (absolute || windows).then(|| candidate.to_string())
}

fn place(text: &str, bases: &[PathBuf], home: Option<&Path>, places: &mut Vec<PathBuf>, unresolved: &mut Vec<String>) {
    let mut expanded = text.to_string();
    if let Some(home) = home {
        let h = home.to_string_lossy();
        if expanded == "~" || expanded == "$HOME" || expanded == "${HOME}" {
            expanded = h.to_string();
        } else {
            expanded = expanded.replace("${HOME}/", &format!("{h}/")).replace("$HOME/", &format!("{h}/"));
            if let Some(tail) = expanded.strip_prefix("~/") {
                expanded = format!("{h}/{tail}");
            }
        }
    }
    let path = PathBuf::from(&expanded);
    if path.is_absolute() {
        places.push(path);
    } else if bases.is_empty() {
        unresolved.push(text.to_string());
    } else {
        for base in bases {
            places.push(base.join(&path));
        }
    }
}

fn is_operator(c: char) -> bool {
    ";|&<>()".contains(c)
}

/// A shell command read as words, the way a shell reads them: quotes group, a backslash escapes, whitespace and `; | & < > ( )` end a word,
/// and the body of a here-document is skipped. Substitutions are not expanded.
pub fn shell_words(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let n = chars.len();
    let mut words = Vec::new();
    let mut heredocs: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\'' {
            let end = chars[i + 1..].iter().position(|&x| x == '\'').map(|p| p + i + 1);
            let stop = end.unwrap_or(n);
            word.extend(&chars[i + 1..stop]);
            in_word = true;
            i = stop + 1;
        } else if c == '"' {
            i += 1;
            while i < n && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                }
                word.push(chars[i]);
                i += 1;
            }
            in_word = true;
            i += 1;
        } else if c == '\\' && i + 1 < n {
            word.push(chars[i + 1]);
            in_word = true;
            i += 2;
        } else if c == '<' && i + 1 < n && chars[i + 1] == '<' && !(i + 2 < n && chars[i + 2] == '<') {
            if in_word {
                words.push(std::mem::take(&mut word));
                in_word = false;
            }
            let mut at = i + 2;
            if at < n && chars[at] == '-' {
                at += 1;
            }
            while at < n && chars[at] == ' ' {
                at += 1;
            }
            let mut delimiter = String::new();
            let quote = if at < n && (chars[at] == '\'' || chars[at] == '"') {
                at += 1;
                Some(chars[at - 1])
            } else {
                None
            };
            while at < n && quote.map_or(!(chars[at].is_whitespace() || is_operator(chars[at])), |q| chars[at] != q) {
                delimiter.push(chars[at]);
                at += 1;
            }
            if quote.is_some() && at < n {
                at += 1;
            }
            if !delimiter.is_empty() {
                heredocs.push(delimiter);
            }
            i = at;
        } else if c == '\n' && !heredocs.is_empty() {
            if in_word {
                words.push(std::mem::take(&mut word));
                in_word = false;
            }
            i = skip_bodies(&chars, i + 1, &heredocs);
            heredocs.clear();
        } else if c.is_whitespace() || is_operator(c) {
            if in_word {
                words.push(std::mem::take(&mut word));
                in_word = false;
            }
            i += 1;
        } else {
            word.push(c);
            in_word = true;
            i += 1;
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

fn skip_bodies(chars: &[char], from: usize, delimiters: &[String]) -> usize {
    let mut at = from;
    for delimiter in delimiters {
        while at < chars.len() {
            let end = chars[at..].iter().position(|&c| c == '\n').map(|p| p + at);
            let line: String = chars[at..end.unwrap_or(chars.len())].iter().collect();
            at = end.map_or(chars.len(), |e| e + 1);
            if line.trim() == delimiter {
                break;
            }
        }
    }
    at
}

/// The sentence to refuse the call with, or none when every place it names is allowed or it names none.
pub fn refuse(
    policy_file: &Path,
    workspace: &[PathBuf],
    home: Option<&Path>,
    temp: &Path,
    arguments: &Value,
) -> Option<String> {
    let (places, unresolved) = places_named(arguments, workspace, home);
    if places.is_empty() && unresolved.is_empty() {
        return None;
    }
    let mut variables: HashMap<String, Vec<PathBuf>> = HashMap::new();
    variables.insert("home".into(), home.map(|h| vec![h.to_path_buf()]).unwrap_or_default());
    variables.insert("temp".into(), vec![temp.to_path_buf()]);
    variables.insert("workspace".into(), workspace.to_vec());
    let policy = AccessPolicy::read(policy_file, &variables);
    let refusal = |reason: String| {
        format!(
            "Access policy: {reason}. The agent may name only the places the policy file allows ({}); if this one is genuinely needed, \
             the person changes that file or approves the call.",
            policy_file.display()
        )
    };
    if let Some(first) = unresolved.first() {
        return Some(refusal(format!("'{first}' is a relative path and the session has no workspace to say where it is")));
    }
    for place in &places {
        let decision = policy.check(place);
        if !decision.allowed {
            return Some(refusal(decision.reason));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../tests/fixtures/access-cases.json");

    struct Machine {
        root: PathBuf,
        home: PathBuf,
        workspace: PathBuf,
        sibling: PathBuf,
        temp: PathBuf,
        policy: PathBuf,
    }

    impl Machine {
        fn lay_out(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("jawata-access-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let root = {
                std::fs::create_dir_all(&root).unwrap();
                std::fs::canonicalize(&root).unwrap()
            };
            let home = root.join("home");
            let workspace = home.join("CursorProjects").join("projA");
            let sibling = home.join("CursorProjects").join("projB");
            let temp = root.join("scratch");
            for dir in [&workspace, &sibling, &temp, &home.join(".claude").join("projects")] {
                std::fs::create_dir_all(dir).unwrap();
            }
            let cases: Value = serde_json::from_str(CASES).unwrap();
            let policy = root.join("access-policy.json");
            std::fs::write(&policy, cases["policy"].to_string()).unwrap();
            Self { root, home, workspace, sibling, temp, policy }
        }

        /// The case text is JSON, so each folder goes in as a JSON string body: on Windows the backslashes of a path are doubled, or the
        /// text is not JSON any more (the first 4.4.20 release run failed on exactly this: "invalid escape").
        fn bound(&self, text: &str) -> String {
            text.replace("${home}", &json_body(&self.home))
                .replace("${workspace}", &json_body(&self.workspace))
                .replace("${sibling}", &json_body(&self.sibling))
                .replace("${temp}", &json_body(&self.temp))
        }

        fn refuse(&self, call: &Value) -> Option<String> {
            refuse(&self.policy, std::slice::from_ref(&self.workspace), Some(&self.home), &self.temp, call)
        }
    }

    impl Drop for Machine {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// A path as the body of a JSON string: backslashes doubled, which only a Windows path has.
    fn json_body(path: &std::path::Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\")
    }

    #[test]
    fn a_windows_path_put_into_case_text_is_still_json_and_reads_back_unchanged() {
        let path = std::path::Path::new("C:\\Users\\runneradmin\\AppData\\Local\\Temp\\cases");
        let text = format!("{{\"path\":\"{}\"}}", json_body(path));
        let parsed: Value = serde_json::from_str(&text).expect("the substituted text is not JSON");
        assert_eq!(parsed["path"], "C:\\Users\\runneradmin\\AppData\\Local\\Temp\\cases");
    }

    #[test]
    fn every_case_of_the_shared_file_is_decided_as_it_says_by_the_hooks_parser() {
        let machine = Machine::lay_out("cases");
        let cases: Value = serde_json::from_str(CASES).unwrap();
        let mut wrong = Vec::new();
        let (mut allowed, mut refused) = (0, 0);
        for case in cases["cases"].as_array().unwrap() {
            let call: Value = serde_json::from_str(&machine.bound(&case["call"].to_string())).unwrap();
            let refusal = machine.refuse(&call);
            let id = format!("{} ({})", case["id"].as_str().unwrap(), case["acl"].as_str().unwrap());
            if case["expect"] == "allow" {
                allowed += 1;
                if let Some(r) = refusal {
                    wrong.push(format!("{id} was refused: {r}"));
                }
            } else {
                refused += 1;
                match refusal {
                    None => wrong.push(format!("{id} was allowed: {}", case["why"].as_str().unwrap())),
                    Some(r) if !r.starts_with("Access policy: ") => wrong.push(format!("{id} was refused in other words: {r}")),
                    Some(_) => {}
                }
            }
        }
        assert!(wrong.is_empty(), "cases decided otherwise than the file says: {wrong:#?}");
        assert!(allowed >= 5 && refused >= 10, "the case file is too small to bind a parser: {allowed} allowed, {refused} refused");
    }

    #[test]
    fn the_hooks_copy_of_the_case_file_is_the_engines_when_the_engines_repository_is_beside_this_one() {
        let engines = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../jawata-mcp/org.jawata.mcp.tests/test-resources/access/cases.json");
        match std::fs::read_to_string(&engines) {
            Ok(theirs) => assert_eq!(theirs, CASES, "the copy of the shared case file has drifted from the engine's"),
            Err(_) => eprintln!("the engine's repository is not beside this one; the copy was not compared"),
        }
    }

    #[test]
    fn a_policy_that_cannot_be_used_refuses_a_place_and_says_which_way() {
        let machine = Machine::lay_out("unusable");
        let call = serde_json::json!({"file_path": machine.workspace.join("A.java").to_string_lossy()});
        let vars: HashMap<String, Vec<PathBuf>> = HashMap::from([("workspace".to_string(), vec![machine.workspace.clone()])]);

        std::fs::write(&machine.policy, "{\"allow\": [").unwrap();
        assert!(machine.refuse(&call).unwrap().contains("not valid JSON"));
        std::fs::write(&machine.policy, "{\"allow\":[\"${workspace}\"]}").unwrap();
        assert!(machine.refuse(&call).unwrap().contains("no \"deny\" list"));
        std::fs::write(&machine.policy, "{\"allow\":[\"${nobody}/x\"],\"deny\":[]}").unwrap();
        assert!(machine.refuse(&call).unwrap().contains("${nobody}, which is not defined"));
        std::fs::write(&machine.policy, "{\"allow\":[\"src\"],\"deny\":[]}").unwrap();
        assert!(machine.refuse(&call).unwrap().contains("is not an absolute path"));
        std::fs::remove_file(&machine.policy).unwrap();
        assert!(machine.refuse(&call).unwrap().contains("there is no access policy at"));
        std::fs::write(&machine.policy, "{\"allow\":[\"${workspace}\"],\"deny\":[]}").unwrap();
        assert_eq!(machine.refuse(&call), None, "CONTROL: a good policy refused its own place");
        assert!(AccessPolicy::read(&machine.policy, &vars).usable());
    }

    #[test]
    fn a_here_document_body_and_a_quoted_sentence_name_no_place() {
        assert_eq!(shell_words("cat <<EOF\n/etc/passwd\nEOF"), vec!["cat"]);
        assert_eq!(shell_words("git commit -m \"see /etc/passwd now\""), vec!["git", "commit", "-m", "see /etc/passwd now"]);
        assert_eq!(shell_words("echo hi > /tmp/x"), vec!["echo", "hi", "/tmp/x"]);
    }
}
