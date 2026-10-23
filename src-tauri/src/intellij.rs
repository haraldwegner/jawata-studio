//! IntelliJ projects as a workspace definition (Sprint 28k, contract step S4): the third kind beside a `.code-workspace` file and a
//! folder studio scans. An IntelliJ project lists its modules in `.idea/modules.xml`; each module's `.iml` says where its folder and
//! its source folders are. A module with a build file is the project its build file makes; a module without one is a project when
//! its `.iml` names a source folder that exists, and is left out when it names none (there is nothing for jawata to read).
//!
//! The files are read as text: they are small, IntelliJ writes them in one shape, and studio carries no XML parser.

use std::path::{Component, Path, PathBuf};

/// One module of an IntelliJ project, as its `.iml` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntellijModule {
    /// The module's name: its `.iml` file's name, as IntelliJ shows it.
    pub name: String,
    /// The module's folder: its content root, or the `.iml`'s folder when it names none.
    pub dir: PathBuf,
    /// The source folders the `.iml` names, test sources included.
    pub source_folders: Vec<PathBuf>,
}

/// The IntelliJ project a path names: the folder holding `.idea/modules.xml`, given either that folder or the file itself.
pub fn project_dir_of(path: &Path) -> Option<PathBuf> {
    if path.is_dir() && path.join(".idea").join("modules.xml").is_file() {
        return Some(path.to_path_buf());
    }
    let is_modules_xml = path.file_name().is_some_and(|n| n == "modules.xml")
        && path.parent().and_then(|p| p.file_name()).is_some_and(|n| n == ".idea");
    if is_modules_xml && path.is_file() {
        return path.parent().and_then(Path::parent).map(Path::to_path_buf);
    }
    None
}

/// The modules `.idea/modules.xml` lists, in its order, each read from its `.iml`. A module whose `.iml` cannot be read is an error
/// that names it, never a module quietly dropped.
pub fn read_modules(project_dir: &Path) -> Result<Vec<IntellijModule>, String> {
    let modules_xml = project_dir.join(".idea").join("modules.xml");
    let text = std::fs::read_to_string(&modules_xml)
        .map_err(|e| format!("failed to read {}: {e}", modules_xml.display()))?;
    let mut modules = Vec::new();
    for tag in tags(&text, "module") {
        let Some(filepath) = attribute(tag, "filepath") else { continue };
        let iml = normalize(&PathBuf::from(expand(&filepath, "$PROJECT_DIR$", project_dir)));
        modules.push(read_iml(&iml)?);
    }
    Ok(modules)
}

fn read_iml(iml: &Path) -> Result<IntellijModule, String> {
    let text = std::fs::read_to_string(iml).map_err(|e| format!("failed to read the module file {}: {e}", iml.display()))?;
    let module_dir = iml.parent().unwrap_or(Path::new("/"));
    let url_path = |url: String| normalize(&PathBuf::from(expand(url.trim_start_matches("file://"), "$MODULE_DIR$", module_dir)));
    let dir = tags(&text, "content").into_iter().filter_map(|t| attribute(t, "url")).map(url_path).next()
        .unwrap_or_else(|| module_dir.to_path_buf());
    let source_folders = tags(&text, "sourceFolder").into_iter().filter_map(|t| attribute(t, "url")).map(url_path).collect();
    let name = iml.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "module".into());
    Ok(IntellijModule { name, dir, source_folders })
}

/// The text of every `<name ...>` opening tag, attributes included.
fn tags<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // `<module` must not match `<modules`: the tag name ends at whitespace, `/` or `>`
        if after.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            let end = after.find('>').unwrap_or(after.len());
            found.push(&after[..end]);
        }
        rest = after;
    }
    found
}

/// The value of `name="..."` in a tag's text, with the XML entities IntelliJ writes in paths decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let mut search = tag;
    loop {
        let at = search.find(&key)?;
        let preceded_by_space = at == 0 || search[..at].ends_with(char::is_whitespace);
        let value_start = at + key.len();
        if preceded_by_space {
            let len = search[value_start..].find('"')?;
            let raw = &search[value_start..value_start + len];
            return Some(raw.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&"));
        }
        search = &search[value_start..];
    }
}

fn expand(text: &str, variable: &str, dir: &Path) -> String {
    text.replace(variable, &dir.to_string_lossy())
}

/// `a/b/../c` as `a/c`, without asking the file system: a module's folder is named the way the project names it, links kept.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => { out.pop(); }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-fixtures").join(name)
    }

    #[test]
    fn a_project_is_named_by_its_folder_or_by_its_modules_xml_and_nothing_else_is_one() {
        let dir = fixture("intellij-without-build");
        assert_eq!(project_dir_of(&dir), Some(dir.clone()));
        assert_eq!(project_dir_of(&dir.join(".idea").join("modules.xml")), Some(dir.clone()));
        assert_eq!(project_dir_of(&dir.join("core")), None);
        assert_eq!(project_dir_of(&dir.join("core").join("core.iml")), None);
    }

    #[test]
    fn the_modules_are_read_in_order_with_their_folders_and_source_folders() {
        let dir = fixture("intellij-without-build");
        let modules = read_modules(&dir).unwrap();
        assert_eq!(modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["core", "tools", "empty"]);
        assert_eq!(modules[0].dir, dir.join("core"));
        assert_eq!(modules[0].source_folders, vec![dir.join("core").join("java"), dir.join("core").join("test")]);
        // an .iml kept outside its module's folder names the folder through its content root
        assert_eq!(modules[1].dir, dir.join("tools"));
        assert_eq!(modules[1].source_folders, vec![dir.join("tools").join("src")]);
        assert!(modules[2].source_folders.is_empty());
    }

    #[test]
    fn a_module_file_that_cannot_be_read_is_an_error_that_names_it() {
        let scratch = std::env::temp_dir().join(format!("jawata-intellij-missing-{}", std::process::id()));
        std::fs::create_dir_all(scratch.join(".idea")).unwrap();
        std::fs::write(scratch.join(".idea").join("modules.xml"),
            r#"<modules><module fileurl="file://$PROJECT_DIR$/gone/gone.iml" filepath="$PROJECT_DIR$/gone/gone.iml" /></modules>"#).unwrap();
        let error = read_modules(&scratch).unwrap_err();
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(error.contains("gone.iml"), "{error}");
    }

    #[test]
    fn the_modules_tag_is_not_read_as_a_module_and_an_attribute_is_matched_whole() {
        assert_eq!(tags(r#"<modules><module filepath="a"/></modules>"#, "module").len(), 1);
        assert_eq!(attribute(r#" fileurl="x" filepath="y""#, "filepath").as_deref(), Some("y"));
        assert_eq!(attribute(r#" url="a&amp;b""#, "url").as_deref(), Some("a&b"));
    }
}
