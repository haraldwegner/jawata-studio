//! C5 exit clause 1, asserted mechanically.
//!
//! > `hook → tauri` and `hook → studio crate` are **compile errors**, asserted
//! > by the build, not by review; and `manager_service → jawata-hook` does not
//! > exist (a dependency-tree assertion — deploy writes files, it never runs
//! > hooks).
//!
//! "There is no tauri in Cargo.toml" is review, and review is what let ten
//! shell scripts drift. This reads the resolved dependency graph — the thing
//! that would actually be true at link time — via `cargo metadata`.
//!
//! Two independent properties, and they fail for different reasons:
//!
//! * the hook's closure must not contain the GUI toolkit or the studio's own
//!   library — a process that fires on every keystroke must not link either;
//! * the studio must not depend on the hook — deploy WRITES the binary and
//!   never runs it, so an edge in that direction would mean the hook's code is
//!   linked into the desktop app, and a change to it would rebuild and
//!   redeploy the GUI.

use std::process::Command;

fn metadata() -> serde_json::Value {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--manifest-path", manifest])
        .output()
        .expect("cargo metadata must run — without it this assertion is not made at all");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("cargo metadata emits JSON")
}

/// Every package reachable from `root` in the resolved graph.
fn closure_of(meta: &serde_json::Value, root_name: &str) -> Vec<String> {
    let nodes = meta["resolve"]["nodes"].as_array().expect("resolve.nodes");
    let id_of = |name: &str| -> Option<String> {
        meta["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .map(|p| p["id"].as_str().unwrap().to_string())
    };
    let name_of = |id: &str| -> String {
        meta["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .map(|p| p["name"].as_str().unwrap().to_string())
            .unwrap_or_else(|| id.to_string())
    };

    let root = id_of(root_name)
        .unwrap_or_else(|| panic!("{root_name} is not in the workspace — the assertion cannot be made"));
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(name_of(&id));
        if let Some(node) = nodes.iter().find(|n| n["id"] == id.as_str()) {
            for dep in node["dependencies"].as_array().into_iter().flatten() {
                stack.push(dep.as_str().unwrap().to_string());
            }
        }
    }
    out
}

#[test]
fn the_hook_links_neither_tauri_nor_the_studio_library() {
    let meta = metadata();
    let closure = closure_of(&meta, "jawata-hook");

    // tauri itself, and the whole family — a transitive edge through a plugin
    // links the toolkit just as surely as a direct one.
    let forbidden: Vec<&String> = closure
        .iter()
        .filter(|name| name.starts_with("tauri") || name.as_str() == "jawata-studio")
        .collect();

    assert!(
        forbidden.is_empty(),
        "the hook's dependency closure reaches {forbidden:?}. A hook fires on every prompt \
         and every shell command; linking a GUI toolkit into it is how a hook becomes a hang. \
         This is a dependency-graph fact, not a Cargo.toml reading — a transitive edge counts."
    );

    // And the closure must be small enough to have been read. If this number
    // grows sharply, someone added a dependency tree to a process whose whole
    // budget is milliseconds.
    // MEASURED at 155. The old bound of 200 left 45 packages of headroom in a
    // check whose comment claimed the closure was "small enough to have been
    // read" (C5 audit F7) — a ratchet with that much slack is not one. 165
    // absorbs ordinary transitive churn and still fails a new dependency tree.
    assert!(
        closure.len() <= 165,
        "the hook's closure is {} packages (was 155 when this bound was set) — a hook that \
         pulls a world starts like one. If the growth is deliberate, move the bound WITH the \
         change that justifies it.",
        closure.len()
    );
}

#[test]
fn the_studio_does_not_depend_on_the_hook() {
    // Deploy WRITES the binary; it never runs it. An edge this way would link
    // the hook's code into the desktop app and make a hook change rebuild the
    // GUI.
    let meta = metadata();
    let closure = closure_of(&meta, "jawata-studio");
    assert!(
        !closure.iter().any(|n| n == "jawata-hook"),
        "jawata-studio depends on jawata-hook. Deploy writes files; it must not link them."
    );
}

#[test]
fn both_crates_are_in_one_workspace_so_the_edges_are_checkable_at_all() {
    // If they ever split, the two assertions above would silently start
    // passing by measuring nothing — the shape of a gate that stops gating.
    let meta = metadata();
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(members.iter().any(|m| m.contains("jawata-hook")), "{members:?}");
    assert!(members.iter().any(|m| m.contains("jawata-studio")), "{members:?}");
}
