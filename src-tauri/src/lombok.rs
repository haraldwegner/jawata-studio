//! Sprint 15 B5c — conditional Lombok comprehension agent.
//!
//! Lombok synthesizes members (`@Data` getters/setters, `@Builder`, …) that the
//! Eclipse compiler only "sees" when its bytecode is patched by the Lombok
//! agent at JVM start. JAWATA *is* JDT/ECJ, so for its analysis tools to
//! understand Lombok-using code the resident JVM must launch with
//! `-javaagent:lombok.jar`.
//!
//! That patch is expensive and only relevant to Lombok-using workspaces, so it
//! is applied **conditionally**: this module detects Lombok in the workspace's
//! project(s) and, only then, points `-javaagent` at the `lombok.jar` shipped
//! inside the JAWATA product (Option B — version-locked to the product's own
//! JDT, not the analyzed project's compile version).
//!
//! Not a mirror of the engine's `LombokDetector`: that one finds Lombok use per
//! source file, by import, for the removal tools; this one decides per workspace,
//! from build files and `lombok.config`, whether the JVM needs the agent at start.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// How deep to walk a project tree looking for Lombok markers. Multi-module
/// Maven/Gradle builds declare Lombok in the parent or a module pom; a shallow
/// walk catches both without scanning the whole source tree.
const MAX_DEPTH: usize = 3;

/// True when the project tree shows any sign of Lombok use: a `lombok.config`,
/// or a build file (`pom.xml` / `build.gradle[.kts]`) that references Lombok.
///
/// Source `import lombok.*` scanning is deliberately omitted — build-file /
/// config detection is cheaper and a compiling Lombok project always has one.
pub fn project_uses_lombok(project_root: &Path) -> bool {
    for entry in WalkDir::new(project_root)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            // Skip noisy build-output dirs early.
            if entry.file_type().is_dir() {
                let name = entry.file_name().to_string_lossy();
                if matches!(name.as_ref(), "target" | "build" | "node_modules" | ".git") {
                    continue;
                }
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        match name.as_ref() {
            "lombok.config" => return true,
            "pom.xml" | "build.gradle" | "build.gradle.kts" => {
                if file_mentions_lombok(entry.path()) {
                    return true;
                }
            }
            _ => {}
        }
    }
    // And UP: a module of a multi-module build usually inherits Lombok from its parent pom, and
    // Lombok itself looks for lombok.config in every directory above a source file. Only the
    // marker files of each ancestor are read, never a tree.
    project_root
        .ancestors()
        .skip(1)
        .take(UPWARD_DEPTH)
        .any(|dir| {
            dir.join("lombok.config").is_file()
                || ["pom.xml", "build.gradle", "build.gradle.kts"]
                    .iter()
                    .any(|file| file_mentions_lombok(&dir.join(file)))
        })
}

/// How many directories above a project to look for a parent build file or a lombok.config.
const UPWARD_DEPTH: usize = 4;

fn file_mentions_lombok(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| {
            content.contains("org.projectlombok") || content.contains("lombok:lombok")
        })
        .unwrap_or(false)
}

/// Locate the `lombok.jar` shipped inside the JAWATA product, given the
/// resolved resident jar path. The product is an Equinox tree; the agent jar is
/// a product root file, so it sits at — or one/two levels above — the launcher
/// jar. Returns the first existing candidate.
pub fn bundled_lombok_jar(resolved_jar_path: &Path) -> Option<PathBuf> {
    let mut dir = resolved_jar_path.parent();
    for _ in 0..3 {
        let Some(d) = dir else { break };
        let candidate = d.join("lombok.jar");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// What to do about the Lombok agent for one resident launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentDecision {
    /// Lombok is used and the engine ships the jar: the `-javaagent:` argument to add.
    Add(String),
    /// No project of the workspace uses Lombok.
    NotUsed,
    /// Lombok is used but this engine ships no `lombok.jar`. The resident starts without the
    /// agent, so every Lombok-generated member reads as a compile error; the caller must SAY so
    /// (S19 dogfood: the 4.4.x distribution lost the jar and nothing noticed).
    NoJar,
}

/// The agent decision for a workspace whose projects are `project_roots`.
pub fn decide(project_roots: &[PathBuf], resolved_jar_path: &Path) -> AgentDecision {
    if !project_roots.iter().any(|p| project_uses_lombok(p)) {
        return AgentDecision::NotUsed;
    }
    match bundled_lombok_jar(resolved_jar_path) {
        Some(jar) => AgentDecision::Add(format!("-javaagent:{}", jar.display())),
        None => AgentDecision::NoJar,
    }
}

/// Every project root the resident will load: the `projects` of `<workspace_dir>/workspace.json`,
/// which the manager writes before every spawn. Falls back to the launch project alone when the
/// file is missing or unreadable, so the answer is never an empty list.
pub fn workspace_project_roots(workspace_dir: &Path, launch_project: &Path) -> Vec<PathBuf> {
    let listed: Vec<PathBuf> = std::fs::read_to_string(workspace_dir.join("workspace.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| {
            json["projects"].as_array().map(|rows| {
                rows.iter().filter_map(|row| row.as_str().map(PathBuf::from)).collect()
            })
        })
        .unwrap_or_default();
    if listed.is_empty() {
        vec![launch_project.to_path_buf()]
    } else {
        listed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        let base = std::env::temp_dir().join(format!("jlm-lombok-test-{}", unique()));
        fs::create_dir_all(&base).unwrap();
        base
    }

    // Deterministic unique suffix without Date/rand: a process-local counter.
    fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        std::process::id() as u64 * 1_000_000 + N.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn detects_lombok_in_pom() {
        let root = tmp();
        fs::write(
            root.join("pom.xml"),
            "<project><dependencies><dependency><groupId>org.projectlombok</groupId>\
             <artifactId>lombok</artifactId></dependency></dependencies></project>",
        )
        .unwrap();
        assert!(project_uses_lombok(&root));
    }

    #[test]
    fn detects_lombok_in_gradle() {
        let root = tmp();
        fs::write(
            root.join("build.gradle"),
            "dependencies { compileOnly 'org.projectlombok:lombok:1.18.34' }",
        )
        .unwrap();
        assert!(project_uses_lombok(&root));
    }

    #[test]
    fn detects_lombok_config() {
        let root = tmp();
        fs::write(root.join("lombok.config"), "lombok.addLombokGeneratedAnnotation = true").unwrap();
        assert!(project_uses_lombok(&root));
    }

    #[test]
    fn detects_lombok_in_submodule() {
        let root = tmp();
        let module = root.join("app");
        fs::create_dir_all(&module).unwrap();
        fs::write(root.join("pom.xml"), "<project/>").unwrap();
        fs::write(
            module.join("pom.xml"),
            "<project><dependencies><dependency><groupId>org.projectlombok</groupId></dependency></dependencies></project>",
        )
        .unwrap();
        assert!(project_uses_lombok(&root));
    }

    #[test]
    fn no_lombok_is_false() {
        let root = tmp();
        fs::write(root.join("pom.xml"), "<project><artifactId>plain</artifactId></project>").unwrap();
        fs::write(root.join("build.gradle"), "dependencies { implementation 'com.google.guava:guava:33' }").unwrap();
        assert!(!project_uses_lombok(&root));
    }

    #[test]
    fn bundled_jar_found_as_sibling_and_above() {
        let product = tmp();
        let plugins = product.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        let launcher = plugins.join("org.eclipse.equinox.launcher.jar");
        fs::write(&launcher, "x").unwrap();
        // No jar yet → None.
        assert!(bundled_lombok_jar(&launcher).is_none());
        // Ship lombok.jar at product root (two levels above the launcher).
        fs::write(product.join("lombok.jar"), "x").unwrap();
        assert_eq!(bundled_lombok_jar(&launcher), Some(product.join("lombok.jar")));
    }

    /// S19 dogfood, 2026-10-21: java-design-patterns declares Lombok in its PARENT pom and a root
    /// lombok.config; a module's own pom never names it. Walking only down from the module missed it,
    /// so the agent would not have been added even with the jar shipped.
    #[test]
    fn detects_lombok_declared_above_the_project() {
        let parent = tmp();
        fs::write(
            parent.join("pom.xml"),
            "<project><dependencies><dependency><groupId>org.projectlombok</groupId></dependency></dependencies></project>",
        )
        .unwrap();
        let module = parent.join("singleton");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("pom.xml"), "<project><parent><artifactId>java-design-patterns</artifactId></parent></project>").unwrap();
        assert!(project_uses_lombok(&module), "a parent pom that declares Lombok counts");

        let configured = tmp();
        fs::write(configured.join("lombok.config"), "lombok.log.fieldName = LOGGER").unwrap();
        let deep = configured.join("a").join("b");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("pom.xml"), "<project/>").unwrap();
        assert!(project_uses_lombok(&deep), "a lombok.config above the project counts");
    }

    /// The same dogfood: detection asked ONE project, the one the launch was made for, while the
    /// resident loads every project of the workspace. Any one of them using Lombok needs the agent.
    #[test]
    fn every_project_of_the_workspace_is_asked() {
        let plain = tmp();
        fs::write(plain.join("pom.xml"), "<project/>").unwrap();
        let lombok = tmp();
        fs::write(lombok.join("lombok.config"), "x").unwrap();
        let ws = tmp();
        fs::write(
            ws.join("workspace.json"),
            serde_json::json!({ "version": 1, "name": "w", "projects": [plain, lombok] }).to_string(),
        )
        .unwrap();
        let roots = workspace_project_roots(&ws, &plain);
        assert_eq!(roots, vec![plain.clone(), lombok.clone()]);

        // No readable workspace.json: the launch project alone, never an empty list.
        assert_eq!(workspace_project_roots(&tmp(), &plain), vec![plain.clone()]);
    }

    /// A Lombok workspace on an engine that ships no lombok.jar must be NAMED, not quietly started
    /// without the agent: every generated member then reads as a compile error and nothing says why.
    #[test]
    fn a_lombok_workspace_without_the_jar_is_named_not_silent() {
        let product = tmp();
        let jar = product.join("jawata.jar");
        fs::write(&jar, "x").unwrap();
        let lombok = tmp();
        fs::write(lombok.join("lombok.config"), "x").unwrap();
        let plain = tmp();
        fs::write(plain.join("pom.xml"), "<project/>").unwrap();

        assert_eq!(decide(&[plain.clone()], &jar), AgentDecision::NotUsed);
        assert_eq!(decide(&[lombok.clone()], &jar), AgentDecision::NoJar);
        fs::write(product.join("lombok.jar"), "x").unwrap();
        match decide(&[plain, lombok], &jar) {
            AgentDecision::Add(arg) => assert!(arg.starts_with("-javaagent:") && arg.ends_with("lombok.jar")),
            other => panic!("expected the agent: {other:?}"),
        }
    }
}
