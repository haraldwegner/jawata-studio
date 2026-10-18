//! Sprint 28g S23 step O1/O3 — run Tier 1 over the frozen replay and count.
//!
//! Usage: `replay_watch <replay folder> [--list]`
//!
//! The folder is `evidence/observer-replay-2026-10-17/`. For every window in `windows.json`
//! the calls in `calls.jsonl` that fall inside it are fed to the rules in order, each judged
//! against the calls before it IN THAT WINDOW, which is what the hook holds when a window
//! begins fresh. The first finding of a class halts; later ones of the same class do not
//! (the same once-per-window rule the hook applies).
//!
//! Counts are the KPI and gate nothing: bypass windows halted, bypass windows missed, clean
//! windows halted (false refusals). The replay rows carry the argument cut to 300 characters
//! and no background flag, so a rule that needs either reads less here than in the hook; the
//! output says so rather than hiding it.

use jawata_hook::watch::{judge, CallView, Class};
use serde_json::Value;
use std::collections::HashMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(folder) = args.get(1) else {
        eprintln!("usage: replay_watch <replay folder> [--list]");
        std::process::exit(2);
    };
    let list = args.iter().any(|a| a == "--list");
    let read = |name: &str| std::fs::read_to_string(format!("{folder}/{name}")).unwrap_or_else(|e| {
        eprintln!("cannot read {name}: {e}");
        std::process::exit(2);
    });

    let mut calls: HashMap<u64, (String, String)> = HashMap::new();
    let mut at: HashMap<u64, String> = HashMap::new();
    for line in read("calls.jsonl").lines() {
        let v: Value = serde_json::from_str(line).expect("a calls.jsonl row");
        at.insert(v["i"].as_u64().expect("call number"), v["ts"].as_str().unwrap_or("").to_string());
        calls.insert(
            v["i"].as_u64().expect("call number"),
            (
                v["tool"].as_str().unwrap_or("").to_string(),
                v["arg"].as_str().unwrap_or("").to_string(),
            ),
        );
    }
    let windows: Value = serde_json::from_str(&read("windows.json")).expect("windows.json");
    let marks: Value = serde_json::from_str(&read("marks.json")).expect("marks.json");
    let events = marks["events"].as_array().expect("events");
    let weak: Vec<String> = events
        .iter()
        .filter(|e| e["weak"].as_bool() == Some(true))
        .map(|e| e["id"].as_str().unwrap_or("").to_string())
        .collect();
    // A rule is "in force" from the moment he first said it. The event each class was learned
    // from, and the time of his first correction of it.
    let source = [
        (Class::ShellOnJava, "B1"),
        (Class::ForegroundWait, "B3"),
        (Class::RogueGate, "B5"),
        (Class::ScriptHabit, "B6"),
    ];
    let in_force_from = |class: Class| -> String {
        let id = source.iter().find(|(c, _)| *c == class).map(|(_, id)| *id).unwrap_or("");
        events
            .iter()
            .find(|e| e["id"].as_str() == Some(id))
            .and_then(|e| e["his_words"][0]["ts"].as_str())
            .unwrap_or("9999")
            .to_string()
    };
    let (mut clean_halted_in_force, mut clean_halted_before) = (0, 0);

    // `--briefs <outdir> <his.jsonl>`: write, for every window, the exact text the hired observer
    // is shown (`judge::brief`, the function the hook uses) with the last three things he typed
    // BEFORE the window opened. Never one typed after: that would be the correction itself.
    let flag = |name: &str| args.iter().position(|a| a == name);
    let briefs_dir = flag("--briefs").and_then(|i| args.get(i + 1)).cloned();
    let his: Vec<(String, String)> = match flag("--briefs").and_then(|i| args.get(i + 2)) {
        Some(path) => std::fs::read_to_string(path)
            .expect("his.jsonl")
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .map(|v| (v["ts"].as_str().unwrap_or("").to_string(), v["text"].as_str().unwrap_or("").to_string()))
            .collect(),
        None => Vec::new(),
    };

    let (mut bypass_halted, mut bypass_missed, mut clean_halted, mut clean_total) = (0, 0, 0, 0);
    let mut by_class: HashMap<&'static str, u32> = HashMap::new();
    for w in windows["windows"].as_array().expect("windows") {
        let range = w["window"].as_array().expect("window range");
        let (from, to) = (range[0].as_u64().unwrap(), range[1].as_u64().unwrap());
        let is_bypass = w["class"].as_str() == Some("BYPASS");
        let id = w["id"].as_str().unwrap_or("?");

        let mut seen: Vec<CallView> = Vec::new();
        let mut halted: Vec<Class> = Vec::new();
        let window_at = at.get(&from).cloned().unwrap_or_default();
        for n in from..=to {
            let Some((tool, arg)) = calls.get(&n) else { continue };
            let call = CallView { tool: tool.clone(), text: arg.clone(), background: false };
            if let Some(f) = judge(&seen, &call) {
                if !halted.contains(&f.class) {
                    halted.push(f.class);
                    *by_class.entry(f.class.name()).or_insert(0) += 1;
                    if list || is_bypass {
                        let force = if is_bypass || window_at >= in_force_from(f.class) { "IN-FORCE" } else { "before-ruling" };
                        println!("  {id} [{force}]: call {n} {} — {}", f.class.name(), arg.chars().take(110).collect::<String>().replace('\n', " "));
                    }
                }
            }
            seen.push(call);
        }
        if let Some(out) = &briefs_dir {
            let typed: Vec<String> = his
                .iter()
                .filter(|(ts, text)| ts.as_str() < window_at.as_str() && !text.trim().is_empty())
                .map(|(_, text)| text.clone())
                .collect();
            let last_three = typed[typed.len().saturating_sub(3)..].to_vec();
            let window = jawata_hook::watch::Window { number: to, calls: seen.clone(), ..Default::default() };
            std::fs::create_dir_all(out).expect("briefs dir");
            std::fs::write(format!("{out}/{id}.txt"), jawata_hook::judge::brief(&window, &last_three)).expect("write brief");
        }
        if is_bypass {
            let tag = if weak.iter().any(|x| x == id) { " (weak)" } else { "" };
            if halted.is_empty() {
                bypass_missed += 1;
                println!("MISSED {id}{tag}");
            } else {
                bypass_halted += 1;
                println!("HALTED {id}{tag}: {}", halted.iter().map(|c| c.name()).collect::<Vec<_>>().join(", "));
            }
        } else {
            clean_total += 1;
            if !halted.is_empty() {
                clean_halted += 1;
                if halted.iter().any(|c| window_at >= in_force_from(*c)) {
                    clean_halted_in_force += 1;
                } else {
                    clean_halted_before += 1;
                }
            }
        }
    }
    println!(
        "bypass windows halted {bypass_halted}, missed {bypass_missed}; clean windows halted {clean_halted} of {clean_total} (false refusals)"
    );
    println!(
        "of those, halted by a rule already in force when the window happened: {clean_halted_in_force}; by a rule he had not yet spoken: {clean_halted_before}"
    );
    let mut classes: Vec<_> = by_class.into_iter().collect();
    classes.sort();
    println!("halts by class: {classes:?}");
}
