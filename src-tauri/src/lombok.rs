//! Sprint 15 B5c — conditional Lombok comprehension agent.
//!
//! Lombok synthesizes members (`@Data` getters/setters, `@Builder`, …) that the
//! Eclipse compiler only "sees" when its bytecode is patched by the Lombok
//! agent at JVM start. GOJA *is* JDT/ECJ, so for its analysis tools to
//! understand Lombok-using code the resident JVM must launch with
//! `-javaagent:lombok.jar`.
//!
//! That patch is expensive and only relevant to Lombok-using workspaces, so it
//! is applied **conditionally**: this module detects Lombok in the workspace's
//! project(s) and, only then, points `-javaagent` at the `lombok.jar` shipped
//! inside the GOJA product (Option B — version-locked to the product's own
//! JDT, not the analyzed project's compile version).
//!
//! Mirror of the fork's source-only `LombokDetector` (different runtime: Rust
//! here, Java there) — keep the two detection heuristics in sync.

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
    false
}

fn file_mentions_lombok(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| {
            content.contains("org.projectlombok") || content.contains("lombok:lombok")
        })
        .unwrap_or(false)
}

/// Locate the `lombok.jar` shipped inside the GOJA product, given the
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

/// The `-javaagent:<lombok.jar>` argument to prepend to the resident JVM, or
/// `None` when comprehension should not be enabled.
///
/// Enabled only when BOTH hold: at least one project in the workspace uses
/// Lombok, AND the product actually ships a `lombok.jar`. A missing jar (e.g. an
/// older runtime that predates Option B) degrades gracefully to no agent rather
/// than a launch failure.
pub fn javaagent_arg(project_roots: &[PathBuf], resolved_jar_path: &Path) -> Option<String> {
    if !project_roots.iter().any(|p| project_uses_lombok(p)) {
        return None;
    }
    let jar = bundled_lombok_jar(resolved_jar_path)?;
    Some(format!("-javaagent:{}", jar.display()))
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

    #[test]
    fn javaagent_arg_requires_both_lombok_and_jar() {
        let product = tmp();
        let jar = product.join("goja.jar");
        fs::write(&jar, "x").unwrap();

        let lombok_proj = tmp();
        fs::write(lombok_proj.join("lombok.config"), "x").unwrap();
        let plain_proj = tmp();
        fs::write(plain_proj.join("pom.xml"), "<project/>").unwrap();

        // Lombok project but no bundled jar → None.
        assert!(javaagent_arg(&[lombok_proj.clone()], &jar).is_none());

        // Add the jar → enabled, well-formed arg.
        fs::write(product.join("lombok.jar"), "x").unwrap();
        let arg = javaagent_arg(&[lombok_proj], &jar).expect("agent expected");
        assert!(arg.starts_with("-javaagent:"));
        assert!(arg.ends_with("lombok.jar"));

        // Plain project → None even with the jar present.
        assert!(javaagent_arg(&[plain_proj], &jar).is_none());
    }
}
