//! Filling a resident's knowledge store from its story folder — whenever an engine comes up.
//!
//! The store is rebuilt by `experience(kind=load, path=<substrate root>)`. Until 2026-09-13 only
//! a Deploy asked for that, so an engine update — which restarts every resident on a new loader —
//! left the store exactly as the old loader wrote it. Measured that day: 55 heading rows the
//! 4.3.1 loader no longer produces were still there after the restart onto 4.3.1. Now every
//! resident that comes up asks, and a Deploy still asks.
//!
//! Two defects of the old call are closed here, both measured rather than inferred:
//!
//! - It never found the story folder. It parsed the raw HTTP body, in which the tool's answer is
//!   a string inside the MCP envelope, so the substrate root was never there to read and every
//!   seed was skipped as "no file substrate". This module asks through the envelope peel the
//!   canary already uses.
//! - It gave up after ten seconds, while a load that re-reads a folder after a loader change took
//!   minutes. This one waits on its own thread, so its log line says what the load did.
//!
//! One store is filled once per engine build. Residents share a store, and three residents coming
//! up on one engine would otherwise load the same folder three times and each start its own
//! background embedding pass over the same rows. A Deploy is an explicit request and always loads.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::{json, Value};

const STATS_TIMEOUT_SECS: u64 = 30;
/// Long on purpose: an engine older than 28f's background vectoriser holds the call until every
/// row it loaded is embedded. The thread is studio's own, so waiting costs nobody a frozen screen.
const LOAD_TIMEOUT_SECS: u64 = 900;

/// A resident the seeder can ask.
#[derive(Debug, Clone)]
pub struct SeedTarget {
    pub url: String,
    pub token: String,
    /// Which engine build it runs — a new build can mean a new loader, so it earns a reload.
    pub engine: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A resident started, or was adopted after a studio restart.
    EngineReady,
    /// The user deployed. Always loads.
    Deploy,
}

impl Trigger {
    fn label(self) -> &'static str {
        match self {
            Trigger::EngineReady => "engine came up",
            Trigger::Deploy => "deploy",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SeedOutcome {
    Loaded {
        store: String,
        root: String,
        loaded: Option<u64>,
        unembedded: Option<i64>,
    },
    AlreadySeeded { store: String },
    InFlight { store: String },
    NoSubstrate,
    Failed(String),
}

impl SeedOutcome {
    pub fn describe(&self) -> String {
        match self {
            SeedOutcome::Loaded { store, root, loaded, unembedded } => format!(
                "reloaded {root} into {store}: {} file(s) written, {} row(s) still waiting for \
                 a meaning vector",
                loaded.map_or_else(|| "?".to_string(), |n| n.to_string()),
                unembedded.map_or_else(|| "?".to_string(), |n| n.to_string()),
            ),
            SeedOutcome::AlreadySeeded { store } => {
                format!("skipped: {store} was already reloaded on this engine build")
            }
            SeedOutcome::InFlight { store } => {
                format!("skipped: a reload of {store} is already running")
            }
            SeedOutcome::NoSubstrate => "skipped: the store reports no story folder, and a load \
                 without one would crawl the legacy corpus (studio#34)"
                .to_string(),
            SeedOutcome::Failed(why) => format!("FAILED: {why}"),
        }
    }
}

/// How the seeder reaches a resident; a trait so the decisions can be tested without one.
pub trait Resident {
    fn experience(&self, url: &str, token: &str, args: Value, timeout_secs: u64)
        -> Result<Value, String>;
}

struct Http;

impl Resident for Http {
    fn experience(&self, url: &str, token: &str, args: Value, timeout_secs: u64)
        -> Result<Value, String> {
        crate::manager_service::call_experience(url, token, args, timeout_secs)
    }
}

/// What this studio has already reloaded, and what it is reloading now.
#[derive(Default)]
pub struct SeedLedger {
    seeded: Mutex<HashSet<(String, String)>>,
    in_flight: Mutex<HashSet<String>>,
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ledger() -> &'static SeedLedger {
    static LEDGER: OnceLock<SeedLedger> = OnceLock::new();
    LEDGER.get_or_init(SeedLedger::default)
}

/// The story folder `experience(kind=stats)` reports, or `None` — and no root means no seed.
pub fn substrate_root_in(value: &Value) -> Option<String> {
    let root = value
        .pointer("/data/substrate/root")
        .or_else(|| value.pointer("/substrate/root"))?
        .as_str()?
        .trim();
    (!root.is_empty()).then(|| root.to_string())
}

fn store_file_in(value: &Value) -> Option<String> {
    let file = value.pointer("/data/store/file")?.as_str()?.trim();
    (!file.is_empty()).then(|| file.to_string())
}

/// Reload one resident's store from its story folder, deciding first whether it needs it.
pub fn seed(
    target: &SeedTarget,
    trigger: Trigger,
    ledger: &SeedLedger,
    resident: &dyn Resident,
) -> SeedOutcome {
    let stats = match resident.experience(
        &target.url,
        &target.token,
        json!({"kind": "stats"}),
        STATS_TIMEOUT_SECS,
    ) {
        Ok(stats) => stats,
        Err(error) => return SeedOutcome::Failed(format!("could not read the store: {error}")),
    };
    let Some(root) = substrate_root_in(&stats) else {
        return SeedOutcome::NoSubstrate;
    };
    let store = store_file_in(&stats).unwrap_or_else(|| target.url.clone());
    let key = (store.clone(), target.engine.clone());
    if trigger == Trigger::EngineReady && locked(&ledger.seeded).contains(&key) {
        return SeedOutcome::AlreadySeeded { store };
    }
    if !locked(&ledger.in_flight).insert(store.clone()) {
        return SeedOutcome::InFlight { store };
    }
    let result = resident.experience(
        &target.url,
        &target.token,
        json!({"kind": "load", "path": root, "recursive": true}),
        LOAD_TIMEOUT_SECS,
    );
    locked(&ledger.in_flight).remove(&store);
    match result {
        Err(error) => SeedOutcome::Failed(format!("the load of {root} did not answer: {error}")),
        Ok(answer) if answer.get("success").and_then(Value::as_bool) == Some(false) => {
            SeedOutcome::Failed(format!(
                "the load of {root} was refused: {}",
                answer
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("no reason given")
            ))
        }
        Ok(answer) => {
            locked(&ledger.seeded).insert(key);
            SeedOutcome::Loaded {
                store,
                root,
                loaded: answer.pointer("/data/loaded").and_then(Value::as_u64),
                unembedded: answer.pointer("/data/unembedded").and_then(Value::as_i64),
            }
        }
    }
}

/// Reload on studio's own thread and write one line to studio's log saying what happened.
pub fn seed_in_background(target: SeedTarget, trigger: Trigger) {
    let spawned = std::thread::Builder::new()
        .name("store-seed".into())
        .spawn(move || {
            let outcome = seed(&target, trigger, ledger(), &Http);
            eprintln!(
                "[jawata-studio] {} store reload ({}) {}: {}",
                crate::studio_log::utc_stamp(crate::field_view::now_millis()),
                trigger.label(),
                target.url,
                outcome.describe()
            );
        });
    if let Err(error) = spawned {
        eprintln!("[jawata-studio] store reload could not start a thread: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const ROOT: &str = "/stories";
    const STORE: &str = "/data/experience.mv.db";

    fn stats(root: Option<&str>) -> Value {
        match root {
            Some(root) => json!({"success": true, "data": {
                "substrate": {"root": root}, "store": {"file": STORE}}}),
            None => json!({"success": true, "data": {"total": 0}}),
        }
    }

    /// Answers stats with a fixed body and every load with `load`, recording each call.
    struct Fake {
        stats: Value,
        load: Result<Value, String>,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(stats: Value, load: Result<Value, String>) -> Self {
            Self { stats, load, calls: RefCell::new(Vec::new()) }
        }
        fn loads(&self) -> usize {
            self.calls.borrow().iter().filter(|c| *c == "load").count()
        }
    }

    impl Resident for Fake {
        fn experience(&self, _url: &str, _token: &str, args: Value, _timeout: u64)
            -> Result<Value, String> {
            let kind = args["kind"].as_str().unwrap_or_default().to_string();
            if kind == "load" {
                assert_eq!(args["path"], ROOT, "the load must NAME the story folder");
            }
            self.calls.borrow_mut().push(kind.clone());
            if kind == "stats" { Ok(self.stats.clone()) } else { self.load.clone() }
        }
    }

    fn target(engine: &str) -> SeedTarget {
        SeedTarget { url: "http://127.0.0.1:1/mcp".into(), token: "t".into(), engine: engine.into() }
    }

    fn loaded_ok() -> Result<Value, String> {
        Ok(json!({"success": true, "data": {"loaded": 135, "unembedded": 55}}))
    }

    #[test]
    fn a_resident_that_comes_up_reloads_its_store_from_the_story_folder() {
        let fake = Fake::new(stats(Some(ROOT)), loaded_ok());
        let outcome = seed(&target("4.3.1"), Trigger::EngineReady, &SeedLedger::default(), &fake);
        assert_eq!(
            outcome,
            SeedOutcome::Loaded {
                store: STORE.into(),
                root: ROOT.into(),
                loaded: Some(135),
                unembedded: Some(55)
            }
        );
        assert_eq!(fake.loads(), 1);
    }

    #[test]
    fn no_story_folder_means_no_load() {
        let fake = Fake::new(stats(None), loaded_ok());
        let outcome = seed(&target("4.3.1"), Trigger::EngineReady, &SeedLedger::default(), &fake);
        assert_eq!(outcome, SeedOutcome::NoSubstrate);
        assert_eq!(fake.loads(), 0, "a pathless load is the studio#34 defect itself");
    }

    #[test]
    fn a_second_resident_on_the_same_store_and_engine_does_not_reload_it_again() {
        let ledger = SeedLedger::default();
        let fake = Fake::new(stats(Some(ROOT)), loaded_ok());
        seed(&target("4.3.1"), Trigger::EngineReady, &ledger, &fake);
        let second = seed(&target("4.3.1"), Trigger::EngineReady, &ledger, &fake);
        assert_eq!(second, SeedOutcome::AlreadySeeded { store: STORE.into() });
        assert_eq!(fake.loads(), 1, "one store, one engine build, one reload");
    }

    #[test]
    fn a_new_engine_build_and_a_deploy_each_reload_anyway() {
        let ledger = SeedLedger::default();
        let fake = Fake::new(stats(Some(ROOT)), loaded_ok());
        seed(&target("4.3.1"), Trigger::EngineReady, &ledger, &fake);
        seed(&target("4.3.2"), Trigger::EngineReady, &ledger, &fake);
        assert_eq!(fake.loads(), 2, "a new build can mean a new loader, so it reloads");
        seed(&target("4.3.2"), Trigger::Deploy, &ledger, &fake);
        assert_eq!(fake.loads(), 3, "a deploy is an explicit request and always loads");
    }

    #[test]
    fn a_failed_load_is_not_remembered_so_the_next_start_tries_again() {
        let ledger = SeedLedger::default();
        let failing = Fake::new(stats(Some(ROOT)), Err("timed out".into()));
        let outcome = seed(&target("4.3.1"), Trigger::EngineReady, &ledger, &failing);
        assert!(matches!(outcome, SeedOutcome::Failed(_)), "{outcome:?}");
        let refused = Fake::new(
            stats(Some(ROOT)),
            Ok(json!({"success": false, "error": {"message": "path does not exist"}})),
        );
        let outcome = seed(&target("4.3.1"), Trigger::EngineReady, &ledger, &refused);
        assert!(
            matches!(&outcome, SeedOutcome::Failed(why) if why.contains("path does not exist")),
            "a refusal must be reported as one, with its reason: {outcome:?}"
        );
        let working = Fake::new(stats(Some(ROOT)), loaded_ok());
        seed(&target("4.3.1"), Trigger::EngineReady, &ledger, &working);
        assert_eq!(working.loads(), 1, "neither failure may count as a reload");
    }

    /// studio#34: the seed must NAME the substrate, and must not fall back to a pathless load —
    /// that call crawls the engine's legacy default roots and is the defect itself.
    #[test]
    fn the_substrate_root_is_read_or_the_seed_is_skipped() {
        let real = json!({
            "success": true,
            "data": {
                "total": 384,
                "substrate": {
                    "root": "/home/harald/CursorProjects/jawata-enterprise/docs/knowledge/stories",
                    "derivedFrom": "190 entries carrying a memory: source path"
                }
            }
        });
        assert_eq!(
            Some("/home/harald/CursorProjects/jawata-enterprise/docs/knowledge/stories".to_string()),
            substrate_root_in(&real)
        );
        for none in [
            json!({"success": true, "data": {"total": 0}}),
            json!({"success": true, "data": {"substrate": {}}}),
            json!({"success": true, "data": {"substrate": {"root": null}}}),
            json!({"success": true, "data": {"substrate": {"root": "   "}}}),
            json!({"error": "resident is booting"}),
        ] {
            assert_eq!(None, substrate_root_in(&none), "no root means no seed: {none}");
        }
    }

    /// THE DEFECT THAT KEPT EVERY SEED FROM RUNNING. A resident answers inside the MCP envelope,
    /// with the tool's response as a STRING in `result.content[0].text`. The old seed parsed the
    /// raw body and looked for the root at its top, where it never is, so it skipped every time
    /// as "no file substrate". Driven here against a real HTTP listener speaking that envelope.
    #[test]
    fn the_real_http_path_reads_the_root_through_the_envelope_and_loads() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut kinds = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let tool = if request.contains(r#""kind":"load""#) {
                    json!({"success": true, "data": {"loaded": 3, "unembedded": 3}})
                } else {
                    stats(Some(ROOT))
                };
                kinds.push(if request.contains(r#""kind":"load""#) { "load" } else { "stats" });
                let body = json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [
                    {"type": "text", "text": tool.to_string()}]}})
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            kinds
        });
        let target = SeedTarget { url, token: "t".into(), engine: "4.3.2".into() };
        let outcome = seed(&target, Trigger::EngineReady, &SeedLedger::default(), &Http);
        assert_eq!(
            outcome,
            SeedOutcome::Loaded {
                store: STORE.into(),
                root: ROOT.into(),
                loaded: Some(3),
                unembedded: Some(3)
            }
        );
        assert_eq!(server.join().unwrap(), vec!["stats", "load"]);
    }
}
