//! Sprint 25 Stage 8 (spec D2): the agent runner — the reusable layer that
//! spawns and drives a NARROW agent: a bounded task loop (detect the work →
//! do it → verify it → stop), hard gates before anything lands, containment
//! (TTL / iteration / cost ceilings, reap on breach), and a labeled journal.
//!
//! PROPOSE MODE is the invariant of the whole sprint: the runner NEVER
//! applies a change. A run's product is a PROPOSAL RECORD on disk
//! (`proposal.md` + `diff.patch` + `gates.json`) that a human applies — or
//! declines — through the existing apply paths.
//!
//! Layering (matches the plan's 8a/8b sub-gate):
//! - 8a (this section): seat definitions (markdown + frontmatter), the CLI
//!   adapter contract (Claude Code headless first; any CLI via
//!   `ScriptAdapter`), the task loop, purity + gate suite with the honest
//!   refusal path, proposal records.
//! - 8b: ceilings + process-group reap + the journal jsonl (Sprint 26's
//!   corpus shape) + the `schedule:` field machinery.
//!
//! Seat protocol v1 (model-agnostic, plain text):
//! - DETECT phase: the seat answers either `NOTHING-TO-DO` or a
//!   `WORK: <one-line description>` line.
//! - DO phase: the seat's answer must contain ONE unified diff between
//!   `---JAWATA-PROPOSAL-BEGIN---` and `---JAWATA-PROPOSAL-END---` markers.
//!   Everything before the markers is treated as the seat's evidence prose.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const NOTHING_TO_DO: &str = "NOTHING-TO-DO";
pub const WORK_PREFIX: &str = "WORK:";
pub const PROPOSAL_BEGIN: &str = "---JAWATA-PROPOSAL-BEGIN---";
pub const PROPOSAL_END: &str = "---JAWATA-PROPOSAL-END---";

/// Journal schema version — Sprint 26 consumes this corpus; bump on any
/// key change and keep old versions readable there.
pub const JOURNAL_SCHEMA: u32 = 1;

// ============================================================
// Ceilings (containment contract; enforced in the loop, 8b)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ceilings {
    /// Wall-clock limit for the WHOLE run (all phases together).
    pub wall_ttl_secs: u64,
    /// Maximum detect→do→verify iterations before the run is stopped.
    pub max_iterations: u32,
    /// Cost ceiling in USD, accumulated from the adapter's usage events.
    pub cost_budget_usd: f64,
}

impl Default for Ceilings {
    fn default() -> Self {
        Self {
            wall_ttl_secs: 600,
            max_iterations: 3,
            cost_budget_usd: 1.0,
        }
    }
}

/// Which ceiling stopped a run. `Reaped` verdicts carry one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CeilingKind {
    WallTtl,
    MaxIterations,
    CostBudget,
}

impl CeilingKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CeilingKind::WallTtl => "wall_ttl",
            CeilingKind::MaxIterations => "max_iterations",
            CeilingKind::CostBudget => "cost_budget",
        }
    }
}

// ============================================================
// Seat definitions — markdown + frontmatter
// ============================================================

/// The four gate CLASSES of spec D2. `Always` (compile-verify + purity)
/// runs on every proposal regardless of declaration; the other three are
/// declared per seat via the `gates:` frontmatter list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateClass {
    Always,
    Behavior,
    Tests,
    Docs,
}

impl GateClass {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "always" => Ok(GateClass::Always),
            "behavior" => Ok(GateClass::Behavior),
            "tests" => Ok(GateClass::Tests),
            "docs" => Ok(GateClass::Docs),
            other => Err(format!(
                "unknown gate class '{other}' (known: always, behavior, tests, docs)"
            )),
        }
    }
}

/// A seat = one narrow agent definition: WHO runs (model/effort), WHEN
/// (optional schedule), WITH WHAT (tools allowlist), judged HOW (gate
/// classes + ceilings), and the stance prompt (the markdown body).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatDefinition {
    pub name: String,
    pub model: String,
    pub effort: Option<String>,
    /// Cron-like five-field schedule (`m h dom mon dow`), honored by the
    /// manager's scheduler (8b). `None` = ad-hoc only.
    pub schedule: Option<String>,
    /// Tool allowlist injected into the seat prompt (advisory for the CLI
    /// v1 contract; hard enforcement is a later graduation).
    pub tools: Vec<String>,
    /// Gate classes beyond `Always` this seat's proposals must pass.
    pub gate_classes: Vec<GateClass>,
    pub ceilings: Ceilings,
    /// The stance prompt — the markdown body below the frontmatter.
    pub stance: String,
}

/// Parses a seat definition: a `---` … `---` frontmatter block of
/// `key: value` lines (inline `[a, b]` lists) followed by the markdown
/// stance body. Unknown keys are ERRORS — a typo'd ceiling must never
/// silently become "no ceiling".
pub fn parse_seat_definition(text: &str) -> Result<SeatDefinition, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("seat definition must start with a '---' frontmatter block".into());
    }
    let mut name = None;
    let mut model = None;
    let mut effort = None;
    let mut schedule = None;
    let mut tools: Vec<String> = Vec::new();
    let mut gate_classes: Vec<GateClass> = Vec::new();
    let mut ceilings = Ceilings::default();
    let mut body_start = false;
    let mut stance = String::new();
    for line in lines {
        if body_start {
            stance.push_str(line);
            stance.push('\n');
            continue;
        }
        let trimmed = line.trim();
        if trimmed == "---" {
            body_start = true;
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("frontmatter line without ':': '{trimmed}'"))?;
        let key = key.trim();
        let value = value.trim();
        match key {
            "name" => name = Some(value.to_string()),
            "model" => model = Some(value.to_string()),
            "effort" => effort = Some(value.to_string()),
            "schedule" => schedule = Some(value.trim_matches('"').to_string()),
            "tools" => tools = parse_inline_list(value)?,
            "gates" => {
                for entry in parse_inline_list(value)? {
                    let class = GateClass::parse(&entry)?;
                    if class != GateClass::Always && !gate_classes.contains(&class) {
                        gate_classes.push(class);
                    }
                }
            }
            "ttl_secs" => {
                ceilings.wall_ttl_secs = value
                    .parse()
                    .map_err(|e| format!("ttl_secs '{value}': {e}"))?
            }
            "max_iterations" => {
                ceilings.max_iterations = value
                    .parse()
                    .map_err(|e| format!("max_iterations '{value}': {e}"))?
            }
            "cost_budget_usd" => {
                ceilings.cost_budget_usd = value
                    .parse()
                    .map_err(|e| format!("cost_budget_usd '{value}': {e}"))?
            }
            other => return Err(format!("unknown frontmatter key '{other}'")),
        }
    }
    if !body_start {
        return Err("frontmatter block never closed with '---'".into());
    }
    let name = name.ok_or("seat definition needs 'name:'")?;
    let model = model.ok_or("seat definition needs 'model:'")?;
    let stance_trimmed = stance.trim();
    if stance_trimmed.is_empty() {
        return Err("seat definition needs a stance body below the frontmatter".into());
    }
    Ok(SeatDefinition {
        name,
        model,
        effort,
        schedule,
        tools,
        gate_classes,
        ceilings,
        stance: stance_trimmed.to_string(),
    })
}

fn parse_inline_list(value: &str) -> Result<Vec<String>, String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| format!("expected an inline [a, b] list, got '{value}'"))?;
    Ok(inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

// ============================================================
// Proposals — the seat's product, parsed from the DO phase
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    /// The one-line work description from the DETECT phase.
    pub work: String,
    /// Prose the seat emitted before the proposal markers — its evidence.
    pub evidence: String,
    /// The unified diff. In FILE-BLOCK mode (preferred) the RUNNER computes
    /// it deterministically from the emitted files; in legacy diff mode it
    /// is the seat's verbatim diff. Either way it is what the human reads.
    pub diff: String,
    /// FILE-BLOCK mode: full new contents per touched file. Models are
    /// reliable at whole files and notoriously unreliable at hunk headers —
    /// live C9 produced miscounted AND overlapping hunks before this.
    pub files: Vec<ProposalFile>,
    /// Files the proposal touches.
    pub touched_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalFile {
    pub path: String,
    pub content: String,
}

pub const FILE_BLOCK_PREFIX: &str = "===FILE:";
pub const FILE_BLOCK_SUFFIX: &str = "===";
pub const FILE_BLOCK_END: &str = "===END-FILE===";

/// Extracts the proposal block from a DO-phase answer. `None` when the
/// markers are missing or empty — the caller refuses with the reason.
/// FILE-BLOCK mode is detected by `===FILE:` inside the markers; otherwise
/// the body is treated as a legacy unified diff.
pub fn parse_proposal(work: &str, text: &str) -> Option<Proposal> {
    let begin = text.find(PROPOSAL_BEGIN)?;
    let after_begin = begin + PROPOSAL_BEGIN.len();
    let end_rel = text[after_begin..].find(PROPOSAL_END)?;
    let body = text[after_begin..after_begin + end_rel].trim().to_string();
    if body.is_empty() {
        return None;
    }
    let evidence = text[..begin].trim().to_string();
    if body.contains(FILE_BLOCK_PREFIX) {
        let files = parse_file_blocks(&body)?;
        if files.is_empty() {
            return None;
        }
        let touched_files = files.iter().map(|f| f.path.clone()).collect();
        return Some(Proposal {
            work: work.to_string(),
            evidence,
            diff: String::new(), // computed by the runner against the workdir
            files,
            touched_files,
        });
    }
    let touched_files = touched_files_of(&body);
    Some(Proposal {
        work: work.to_string(),
        evidence,
        diff: body,
        files: Vec::new(),
        touched_files,
    })
}

fn parse_file_blocks(body: &str) -> Option<Vec<ProposalFile>> {
    let mut files = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find(FILE_BLOCK_PREFIX) {
        let after = &rest[start + FILE_BLOCK_PREFIX.len()..];
        let header_end = after.find(FILE_BLOCK_SUFFIX)?;
        let path = after[..header_end].trim().to_string();
        let content_start = header_end + FILE_BLOCK_SUFFIX.len();
        let content_area = &after[content_start..];
        let end = content_area.find(FILE_BLOCK_END)?;
        let mut content = content_area[..end]
            .trim_start_matches(['\r', '\n'])
            .trim_end()
            .to_string();
        content.push('\n');
        if path.is_empty() || path.contains("..") || path.starts_with('/') {
            return None; // a traversal-shaped path refuses the whole block
        }
        files.push(ProposalFile { path, content });
        rest = &content_area[end + FILE_BLOCK_END.len()..];
    }
    Some(files)
}

/// Computes the record diff for a FILE-BLOCK proposal: stage the files in a
/// shadow, `git diff --no-index` old vs new per file. Deterministic — the
/// human reads a real diff regardless of how the seat expressed the change.
pub fn compute_files_diff(workdir: &Path, files: &[ProposalFile]) -> String {
    let mut out = String::new();
    for file in files {
        let old = workdir.join(&file.path);
        let tmp = std::env::temp_dir().join(format!(
            "jawata-newfile-{}-{}",
            std::process::id(),
            out.len()
        ));
        if fs::write(&tmp, &file.content).is_err() {
            continue;
        }
        let output = Command::new("git")
            .arg("diff")
            .arg("--no-index")
            .arg("--")
            .arg(if old.is_file() {
                old.as_os_str().to_os_string()
            } else {
                std::ffi::OsString::from("/dev/null")
            })
            .arg(&tmp)
            .output();
        if let Ok(o) = output {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            // Rewrite the temp/abs headers to repo-relative a/ b/ paths.
            for line in text.lines() {
                if line.starts_with("--- ") {
                    out.push_str(&format!("--- a/{}\n", file.path));
                } else if line.starts_with("+++ ") {
                    out.push_str(&format!("+++ b/{}\n", file.path));
                } else if line.starts_with("diff --git") || line.starts_with("index ") {
                    // drop the synthetic header lines
                } else {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        let _ = fs::remove_file(&tmp);
    }
    out
}

fn touched_files_of(diff: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in diff.lines() {
        let path = if let Some(p) = line.strip_prefix("+++ b/") {
            Some(p)
        } else if let Some(p) = line.strip_prefix("--- a/") {
            Some(p)
        } else {
            None
        };
        if let Some(p) = path {
            let p = p.trim();
            if p != "/dev/null" && !p.is_empty() && !files.iter().any(|f| f == p) {
                files.push(p.to_string());
            }
        }
    }
    files
}

// ============================================================
// Gates — purity (local, deterministic) + the executor seam
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateOutcome {
    pub class: GateClass,
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

impl GateOutcome {
    pub fn pass(class: GateClass, name: &str, detail: &str) -> Self {
        Self {
            class,
            name: name.into(),
            passed: true,
            detail: detail.into(),
        }
    }
    pub fn fail(class: GateClass, name: &str, detail: &str) -> Self {
        Self {
            class,
            name: name.into(),
            passed: false,
            detail: detail.into(),
        }
    }
}

/// The PURITY half of the Always class — deterministic and local: every
/// touched file must live under one of the run's declared scope prefixes.
/// A seat that "fixes" something outside its target is refused, whatever
/// the quality of the fix.
pub fn purity_check(touched_files: &[String], scope_prefixes: &[String]) -> GateOutcome {
    if touched_files.is_empty() {
        return GateOutcome::fail(GateClass::Always, "purity", "proposal touches no files");
    }
    for file in touched_files {
        let inside = scope_prefixes.iter().any(|p| file.starts_with(p.as_str()));
        if !inside {
            return GateOutcome::fail(
                GateClass::Always,
                "purity",
                &format!("file '{file}' is outside the run scope {scope_prefixes:?}"),
            );
        }
    }
    GateOutcome::pass(
        GateClass::Always,
        "purity",
        &format!("{} file(s) inside scope", touched_files.len()),
    )
}

/// The gate seam: 8a's tests script it; the resident-backed implementation
/// (`ResidentGateExecutor`) drives the real mcp calls. Class semantics per
/// spec D2: Always = compile-verify (purity is run by the LOOP, not the
/// executor); Behavior = parity + coverage; Tests = pass×3 + coverage-delta
/// + mutation-bite; Docs = doclint on touched files.
pub trait GateExecutor {
    fn run_class(&self, class: GateClass, proposal: &Proposal) -> Vec<GateOutcome>;
}

/// Resident-backed gates: JSON-RPC `tools/call` against the workspace's
/// resident (the same URL + Bearer contract the gateway uses).
pub struct ResidentGateExecutor {
    pub url: String,
    pub token: String,
    pub project_key: Option<String>,
    http: reqwest::blocking::Client,
    next_id: AtomicU64,
}

impl ResidentGateExecutor {
    pub fn new(url: String, token: String, project_key: Option<String>) -> Self {
        Self {
            url,
            token,
            project_key,
            http: reqwest::blocking::Client::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// One JSON-RPC `tools/call`. Errors come back as a failed outcome —
    /// a gate that cannot run has NOT passed (never silently green).
    pub fn call_tool(
        &self,
        name: &str,
        mut arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if let (Some(key), Some(obj)) = (&self.project_key, arguments.as_object_mut()) {
            obj.entry("projectKey")
                .or_insert(serde_json::Value::String(key.clone()));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let response = self
            .http
            .post(&self.url)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .map_err(|e| format!("resident call {name} failed: {e}"))?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .map_err(|e| format!("resident call {name}: bad JSON ({status}): {e}"))?;
        if let Some(err) = value.get("error") {
            return Err(format!("resident call {name}: JSON-RPC error: {err}"));
        }
        Ok(value)
    }

    fn compile_verify(&self) -> GateOutcome {
        match self.call_tool(
            "compile_workspace",
            serde_json::json!({ "summary": true }),
        ) {
            Ok(v) => {
                let text = extract_tool_text(&v);
                let errors = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|d| d.pointer("/data/errorCount").and_then(|c| c.as_u64()));
                match errors {
                    Some(0) => GateOutcome::pass(GateClass::Always, "compile_verify", "0 errors"),
                    Some(n) => GateOutcome::fail(
                        GateClass::Always,
                        "compile_verify",
                        &format!("{n} compile error(s)"),
                    ),
                    None => GateOutcome::fail(
                        GateClass::Always,
                        "compile_verify",
                        "could not read errorCount from compile_workspace response",
                    ),
                }
            }
            Err(e) => GateOutcome::fail(GateClass::Always, "compile_verify", &e),
        }
    }

    /// Doclint on the touched `.java` files — `javadoc -Xdoclint:all` is a
    /// LOCAL toolchain call (the cheapest honest doc gate; mechanics note).
    fn doclint(&self, proposal: &Proposal, workdir: &Path) -> GateOutcome {
        let java_files: Vec<&String> = proposal
            .touched_files
            .iter()
            .filter(|f| f.ends_with(".java"))
            .collect();
        if java_files.is_empty() {
            return GateOutcome::pass(GateClass::Docs, "doclint", "no .java files touched");
        }
        let mut cmd = Command::new("javadoc");
        cmd.arg("-Xdoclint:all").arg("-quiet").current_dir(workdir);
        for f in &java_files {
            cmd.arg(f.as_str());
        }
        match cmd.output() {
            Ok(out) if out.status.success() => {
                GateOutcome::pass(GateClass::Docs, "doclint", "doclint clean")
            }
            Ok(out) => GateOutcome::fail(
                GateClass::Docs,
                "doclint",
                String::from_utf8_lossy(&out.stderr).trim(),
            ),
            Err(e) => GateOutcome::fail(GateClass::Docs, "doclint", &format!("javadoc: {e}")),
        }
    }
}

fn extract_tool_text(response: &serde_json::Value) -> String {
    response
        .pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Where the resident executor runs doclint from (the workspace root).
/// Kept as a field-free helper so tests can call classes independently.
pub struct ResidentGateContext {
    pub executor: ResidentGateExecutor,
    pub workdir: PathBuf,
}

impl GateExecutor for ResidentGateContext {
    fn run_class(&self, class: GateClass, proposal: &Proposal) -> Vec<GateOutcome> {
        match class {
            GateClass::Always => vec![self.executor.compile_verify()],
            GateClass::Behavior => {
                // Parity + coverage for behavior-touching changes: the suite
                // (run_tests coverage) is the sprint's behavior evidence; a
                // finer parity harness rides the seat that needs it (Stage 11
                // dispatches refactoring PLANS, which carry their own parity
                // gate inside apply_plan).
                vec![run_tests_gate(
                    &self.executor,
                    GateClass::Behavior,
                    "suite_with_coverage",
                    serde_json::json!({ "coverage": true }),
                )]
            }
            GateClass::Tests => {
                // pass ×3 (flake screen) + coverage delta; mutation-bite is
                // seat-driven (Stage 10 passes targetClasses per proposal).
                let mut outcomes = Vec::new();
                for attempt in 1..=3u8 {
                    let mut gate = run_tests_gate(
                        &self.executor,
                        GateClass::Tests,
                        &format!("tests_pass_{attempt}"),
                        serde_json::json!({}),
                    );
                    let failed = !gate.passed;
                    gate.detail = format!("attempt {attempt}: {}", gate.detail);
                    outcomes.push(gate);
                    if failed {
                        return outcomes;
                    }
                }
                outcomes.push(match self.executor.call_tool(
                    "run_tests",
                    serde_json::json!({ "action": "coverage_delta", "diff": "worktree" }),
                ) {
                    Ok(_) => GateOutcome::pass(GateClass::Tests, "coverage_delta", "retrieved"),
                    Err(e) => GateOutcome::fail(GateClass::Tests, "coverage_delta", &e),
                });
                outcomes
            }
            GateClass::Docs => vec![self.executor.doclint(proposal, &self.workdir)],
        }
    }
}

fn run_tests_gate(
    executor: &ResidentGateExecutor,
    class: GateClass,
    name: &str,
    args: serde_json::Value,
) -> GateOutcome {
    match executor.call_tool("run_tests", args) {
        Ok(v) => {
            let text = extract_tool_text(&v);
            if text.contains("\"failed\":0") || text.contains("\"failedCount\":0") {
                GateOutcome::pass(class, name, "0 failed")
            } else {
                GateOutcome::fail(class, name, "test failures (see run_tests output)")
            }
        }
        Err(e) => GateOutcome::fail(class, name, &e),
    }
}

// ============================================================
// The CLI adapter contract — Claude Code headless first
// ============================================================

pub trait CliAdapter: Send {
    fn name(&self) -> String;
    /// Builds the command for ONE phase call; the prompt is the full seat
    /// prompt for that phase.
    fn build_command(&self, prompt: &str) -> Command;
    /// Extracts a cost signal (USD) from one output line, if this line
    /// carries one (the Claude `result` event's `total_cost_usd`).
    fn parse_cost(&self, line: &str) -> Option<f64>;
    /// Extracts the answer text from one output line. Plain-text adapters
    /// return the line verbatim; stream-json adapters unwrap their events.
    fn parse_text(&self, line: &str) -> Option<String>;
}

/// Claude Code headless: `claude -p <prompt> --output-format stream-json
/// --max-turns N [--model m]`. The v1 CLI contract of the plan; other CLIs
/// (Gemini/Codex/Grok) slot in as further implementations.
pub struct ClaudeCodeAdapter {
    pub binary: String,
    pub max_turns: u32,
    pub model: Option<String>,
}

impl Default for ClaudeCodeAdapter {
    fn default() -> Self {
        Self {
            binary: "claude".into(),
            max_turns: 12,
            model: None,
        }
    }
}

impl CliAdapter for ClaudeCodeAdapter {
    fn name(&self) -> String {
        "claude-code".into()
    }

    fn build_command(&self, prompt: &str) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--max-turns")
            .arg(self.max_turns.to_string());
        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        cmd
    }

    fn parse_cost(&self, line: &str) -> Option<f64> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        if value.get("type")?.as_str()? != "result" {
            return None;
        }
        value.get("total_cost_usd")?.as_f64()
    }

    fn parse_text(&self, line: &str) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        match value.get("type")?.as_str()? {
            // The final result event carries the whole answer.
            "result" => value.get("result")?.as_str().map(str::to_string),
            // Assistant events: content is an ARRAY that may open with a
            // `thinking` block — collect every `text` entry, not just [0].
            "assistant" => {
                let parts: Vec<&str> = value
                    .pointer("/message/content")?
                    .as_array()?
                    .iter()
                    .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                    .collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("\n"))
                }
            }
            _ => None,
        }
    }
}

/// Any executable as a seat driver: `command args… <prompt>`. This is the
/// echo-seat vehicle (fixture scripts) AND the general escape hatch for a
/// CLI without a dedicated adapter. Output is treated as plain text; a
/// `COST-USD: <n>` line is the optional cost signal.
pub struct ScriptAdapter {
    pub command: String,
    pub args: Vec<String>,
}

impl CliAdapter for ScriptAdapter {
    fn name(&self) -> String {
        format!("script:{}", self.command)
    }

    fn build_command(&self, prompt: &str) -> Command {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args).arg(prompt);
        cmd
    }

    fn parse_cost(&self, line: &str) -> Option<f64> {
        line.strip_prefix("COST-USD:")
            .and_then(|v| v.trim().parse().ok())
    }

    fn parse_text(&self, line: &str) -> Option<String> {
        Some(line.to_string())
    }
}

// ============================================================
// Phase execution — one adapter call, TTL-aware (reap is 8b's
// enforcement; the deadline plumbing lives here from the start)
// ============================================================

struct PhaseOutput {
    text: String,
    cost_usd: f64,
    /// The phase was killed at the wall-TTL deadline.
    reaped: bool,
}

/// Runs one adapter phase with a hard deadline. The child is spawned in
/// its OWN PROCESS GROUP so a breach kills the whole tree (a CLI agent
/// spawns children of its own) — the resident reap doctrine.
fn run_phase(
    adapter: &dyn CliAdapter,
    prompt: &str,
    workdir: &Path,
    deadline: Instant,
) -> Result<PhaseOutput, String> {
    let mut cmd = adapter.build_command(prompt);
    cmd.current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("adapter '{}' failed to spawn: {e}", adapter.name()))?;
    let stdout = child.stdout.take().ok_or("no stdout handle")?;

    // Reader thread streams lines; the main thread polls the deadline.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut text = String::new();
    let mut cost = 0.0f64;
    let mut reaped = false;
    loop {
        if Instant::now() >= deadline {
            reap_process_tree(&mut child);
            reaped = true;
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if let Some(c) = adapter.parse_cost(&line) {
                    cost += c;
                }
                if let Some(t) = adapter.parse_text(&line) {
                    text.push_str(&t);
                    text.push('\n');
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(_)) = child.try_wait() {
                    // Child exited; drain what the reader already queued.
                    while let Ok(line) = rx.try_recv() {
                        if let Some(c) = adapter.parse_cost(&line) {
                            cost += c;
                        }
                        if let Some(t) = adapter.parse_text(&line) {
                            text.push_str(&t);
                            text.push('\n');
                        }
                    }
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.wait();
                break;
            }
        }
    }
    let _ = reader_handle.join();
    Ok(PhaseOutput {
        text,
        cost_usd: cost,
        reaped,
    })
}

/// Kills the child's WHOLE process group (unix), falling back to a plain
/// child kill. A reaped seat must not leave grandchildren running — the
/// same contract the resident teardown keeps.
fn reap_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id();
        // The child was spawned with process_group(0) → pgid == pid. The
        // `--` is load-bearing: without it a negative pgid parses as a
        // (bogus) signal spec on some kill implementations. Guard pid>1 so
        // a pathological id can never address init's group.
        if pid > 1 {
            let _ = Command::new("kill")
                .arg("-9")
                .arg("--")
                .arg(format!("-{pid}"))
                .status();
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

// ============================================================
// The run — detect → do → verify → stop → journal
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Verdict {
    /// A proposal passed every gate and awaits the human.
    Proposed,
    /// A proposal was REFUSED — the detail is the failing gate's reason.
    Refused(String),
    /// The detect phase found nothing to do (a legitimate, quiet end).
    NothingToDo,
    /// A ceiling stopped the run; the detail names the ceiling.
    Reaped(String),
}

/// Everything one run needs beyond the seat: where it may write proposals,
/// which files it is allowed to touch, and where the journal lives.
pub struct RunRequest<'a> {
    pub seat: &'a SeatDefinition,
    /// The scope the seat works on (path prefixes relative to `workdir`);
    /// purity refuses any touched file outside these.
    pub scope: Vec<String>,
    /// The working directory the adapter runs in (the fixture / repo).
    pub workdir: PathBuf,
    /// Root for proposal records: `<runs_dir>/<run_id>/…`.
    pub runs_dir: PathBuf,
    /// The journal jsonl (appended once per run).
    pub journal_path: PathBuf,
    /// Detector-driven detection (Stage 9+): when a deterministic detector
    /// (e.g. `javadoc_lack` through the resident) already found the work,
    /// the adapter DETECT phase is skipped and the DO prompt carries the
    /// detector's findings as COMPILER FACTS — grounded generation.
    pub pre_detected: Option<PreDetected>,
}

/// Work found by a deterministic detector before the run: the one-line
/// work item plus the raw findings the DO prompt embeds as ground truth.
#[derive(Debug, Clone)]
pub struct PreDetected {
    pub work: String,
    pub facts: String,
}

/// One journal line — Sprint 26's corpus shape. `human_verdict` is null at
/// run time; `record_human_verdict` fills it when the human decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub schema: u32,
    pub ts: u64,
    pub run_id: String,
    pub seat: String,
    pub model: String,
    pub adapter: String,
    pub target: String,
    pub work: String,
    pub evidence: String,
    pub gates: Vec<GateOutcome>,
    pub verdict: Verdict,
    pub human_verdict: Option<String>,
    pub outcome: String,
    pub cost_usd: f64,
    pub iterations: u32,
    pub wall_secs: u64,
}

pub struct RunReport {
    pub run_id: String,
    pub verdict: Verdict,
    pub gates: Vec<GateOutcome>,
    pub proposal_dir: Option<PathBuf>,
    pub cost_usd: f64,
    pub iterations: u32,
}

/// The outcome seam: D2 ends every run by recording its outcome to the
/// experience store, so every driven agent is a producer of lessons.
pub trait StoreRecorder {
    fn record_outcome(&self, entry: &JournalEntry) -> Result<(), String>;
}

/// Resident-backed store recording via `experience(kind=record)`.
pub struct ResidentStoreRecorder {
    pub executor: ResidentGateExecutor,
}

impl StoreRecorder for ResidentStoreRecorder {
    fn record_outcome(&self, entry: &JournalEntry) -> Result<(), String> {
        self.executor
            .call_tool(
                "experience",
                serde_json::json!({
                    "kind": "record",
                    "type": "seat_run",
                    "summary": format!(
                        "seat {} on {}: {} ({})",
                        entry.seat, entry.target, entry.outcome, entry.run_id
                    ),
                    "details": serde_json::to_string(entry).unwrap_or_default(),
                    "operation": format!("seat:{}", entry.seat),
                }),
            )
            .map(|_| ())
    }
}

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_run_id(seat_name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{seat_name}-{nanos}-{n}")
}

/// Drives one full seat run: detect → do → verify → stop, then journal +
/// store. PROPOSE MODE: this function never modifies `workdir` content —
/// its only writes are the proposal record and the journal.
pub fn run_seat(
    request: &RunRequest,
    adapter: &dyn CliAdapter,
    gates: &dyn GateExecutor,
    store: &dyn StoreRecorder,
) -> Result<RunReport, String> {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(request.seat.ceilings.wall_ttl_secs);
    let run_id = new_run_id(&request.seat.name);
    let mut cost_total = 0.0f64;
    let mut iterations = 0u32;
    let verdict;
    let mut gate_outcomes: Vec<GateOutcome> = Vec::new();
    let mut proposal_record: Option<(Proposal, PathBuf)> = None;
    let mut work_line = String::new();
    let mut evidence = String::new();

    loop {
        // --- ceiling: iterations ---
        if iterations >= request.seat.ceilings.max_iterations {
            verdict = Verdict::Reaped(CeilingKind::MaxIterations.as_str().into());
            break;
        }
        iterations += 1;

        // --- DETECT (adapter-driven, unless a detector already found it) ---
        let mut facts = String::new();
        if let Some(pre) = &request.pre_detected {
            work_line = pre.work.clone();
            facts = pre.facts.clone();
        } else {
            let detect_prompt = format!(
                "{stance}\n\n== PHASE: DETECT ==\nWork scope (the ONLY files you may touch): {scope:?}.\n\
                 Inspect the scope and answer with exactly one line:\n\
                 either 'WORK: <one-line description of the single next work item>'\n\
                 or '{nothing}' if the scope needs nothing.",
                stance = request.seat.stance,
                scope = request.scope,
                nothing = NOTHING_TO_DO,
            );
            let detect = run_phase(adapter, &detect_prompt, &request.workdir, deadline)?;
            cost_total += detect.cost_usd;
            if detect.reaped {
                verdict = Verdict::Reaped(CeilingKind::WallTtl.as_str().into());
                break;
            }
            if cost_total > request.seat.ceilings.cost_budget_usd {
                verdict = Verdict::Reaped(CeilingKind::CostBudget.as_str().into());
                break;
            }
            if detect.text.contains(NOTHING_TO_DO) {
                verdict = Verdict::NothingToDo;
                break;
            }
            work_line = detect
                .text
                .lines()
                .find_map(|l| l.trim().strip_prefix(WORK_PREFIX).map(|w| w.trim().to_string()))
                .unwrap_or_else(|| "unspecified work item".into());
        }

        // --- DO ---
        let facts_block = if facts.is_empty() {
            String::new()
        } else {
            format!("\n== COMPILER FACTS (ground truth — never contradict) ==\n{facts}\n")
        };
        let do_prompt = format!(
            "{stance}\n\n== PHASE: PROPOSE ==\nWork item: {work}.\n{facts_block}\
             Implement exactly this work item, touching only files under {scope:?}.\n\
             First state your evidence (what you read, why the change is right), then\n\
             emit the COMPLETE NEW CONTENT of every file you change — never a diff —\n\
             between the markers, one block per file:\n\
             {begin}\n\
             ===FILE: relative/path/File.java===\n\
             <the file's full new content>\n\
             ===END-FILE===\n\
             {end}\n\
             DO NOT apply anything — this is a PROPOSAL for a human.",
            stance = request.seat.stance,
            work = work_line,
            facts_block = facts_block,
            scope = request.scope,
            begin = PROPOSAL_BEGIN,
            end = PROPOSAL_END,
        );
        let do_phase = run_phase(adapter, &do_prompt, &request.workdir, deadline)?;
        cost_total += do_phase.cost_usd;
        if do_phase.reaped {
            verdict = Verdict::Reaped(CeilingKind::WallTtl.as_str().into());
            break;
        }
        if cost_total > request.seat.ceilings.cost_budget_usd {
            verdict = Verdict::Reaped(CeilingKind::CostBudget.as_str().into());
            break;
        }
        let mut proposal = match parse_proposal(&work_line, &do_phase.text) {
            Some(p) => p,
            None => {
                verdict = Verdict::Refused(
                    "DO phase produced no proposal block between the markers".into(),
                );
                // Keep the raw phase output — the refusal must be diagnosable.
                let raw = request.runs_dir.join(&run_id).join("do-phase-raw.txt");
                let _ = fs::create_dir_all(raw.parent().unwrap_or(&request.runs_dir));
                let _ = fs::write(&raw, &do_phase.text);
                break;
            }
        };
        if !proposal.files.is_empty() {
            proposal.diff = compute_files_diff(&request.workdir, &proposal.files);
        }
        evidence = proposal.evidence.clone();

        // --- VERIFY: purity first (local), then the gate classes ---
        gate_outcomes.clear();
        gate_outcomes.push(purity_check(&proposal.touched_files, &request.scope));
        if gate_outcomes.last().is_some_and(|g| g.passed) {
            gate_outcomes.extend(gates.run_class(GateClass::Always, &proposal));
            for class in &request.seat.gate_classes {
                gate_outcomes.extend(gates.run_class(*class, &proposal));
            }
        }
        if let Some(failed) = gate_outcomes.iter().find(|g| !g.passed) {
            verdict = Verdict::Refused(format!(
                "gate '{}' ({:?}) failed: {}",
                failed.name, failed.class, failed.detail
            ));
            // The refusal still writes the record — the reason is evidence.
            let dir = write_proposal_record(request, &run_id, &proposal, &gate_outcomes, false)?;
            proposal_record = Some((proposal, dir));
            break;
        }

        // --- STOP: one accepted proposal per run (v1) ---
        let dir = write_proposal_record(request, &run_id, &proposal, &gate_outcomes, true)?;
        proposal_record = Some((proposal, dir));
        verdict = Verdict::Proposed;
        break;
    }

    let outcome = match &verdict {
        Verdict::Proposed => "proposed".to_string(),
        Verdict::Refused(reason) => format!("refused: {reason}"),
        Verdict::NothingToDo => "nothing_to_do".to_string(),
        Verdict::Reaped(ceiling) => format!("reaped: {ceiling}"),
    };
    let entry = JournalEntry {
        schema: JOURNAL_SCHEMA,
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        run_id: run_id.clone(),
        seat: request.seat.name.clone(),
        model: request.seat.model.clone(),
        adapter: adapter.name(),
        target: request.scope.join(","),
        work: work_line,
        evidence,
        gates: gate_outcomes.clone(),
        verdict: verdict.clone(),
        human_verdict: None,
        outcome,
        cost_usd: cost_total,
        iterations,
        wall_secs: started.elapsed().as_secs(),
    };
    append_journal(&request.journal_path, &entry)?;
    if let Err(e) = store.record_outcome(&entry) {
        // The store is evidence, not a gate: a failed record is logged in
        // the journal directory, never silently dropped.
        let note = request.runs_dir.join(&run_id).join("store-record-failed.txt");
        let _ = fs::create_dir_all(note.parent().unwrap_or(&request.runs_dir));
        let _ = fs::write(&note, &e);
    }
    Ok(RunReport {
        run_id,
        verdict,
        gates: gate_outcomes,
        proposal_dir: proposal_record.map(|(_, dir)| dir),
        cost_usd: cost_total,
        iterations,
    })
}

fn write_proposal_record(
    request: &RunRequest,
    run_id: &str,
    proposal: &Proposal,
    gates: &[GateOutcome],
    all_green: bool,
) -> Result<PathBuf, String> {
    let dir = request.runs_dir.join(run_id);
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let status = if all_green { "PROPOSED" } else { "REFUSED" };
    let md = format!(
        "# Seat proposal — {seat} ({status})\n\nWork item: {work}\n\nScope: {scope:?}\n\n\
         ## Evidence\n\n{evidence}\n\n## Gates\n\nSee `gates.json`. Apply the diff\n\
         (`diff.patch`) only after a human YES — the runner never applies.\n",
        seat = request.seat.name,
        status = status,
        work = proposal.work,
        scope = request.scope,
        evidence = if proposal.evidence.is_empty() {
            "(none stated)"
        } else {
            &proposal.evidence
        },
    );
    fs::write(dir.join("proposal.md"), md).map_err(|e| e.to_string())?;
    fs::write(dir.join("diff.patch"), &proposal.diff).map_err(|e| e.to_string())?;
    let gates_json = serde_json::to_string_pretty(gates).map_err(|e| e.to_string())?;
    fs::write(dir.join("gates.json"), gates_json).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn append_journal(path: &Path, entry: &JournalEntry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open journal {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| e.to_string())
}

/// Fills the human verdict for a run after the fact: rewrites the journal
/// line whose `run_id` matches. The consequence IS the label (Sprint 26).
pub fn record_human_verdict(
    journal_path: &Path,
    run_id: &str,
    human_verdict: &str,
) -> Result<(), String> {
    let content = fs::read_to_string(journal_path)
        .map_err(|e| format!("read journal {}: {e}", journal_path.display()))?;
    let mut found = false;
    let mut lines_out = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut entry: JournalEntry =
            serde_json::from_str(line).map_err(|e| format!("bad journal line: {e}"))?;
        if entry.run_id == run_id {
            entry.human_verdict = Some(human_verdict.to_string());
            found = true;
        }
        lines_out.push(serde_json::to_string(&entry).map_err(|e| e.to_string())?);
    }
    if !found {
        return Err(format!("run_id '{run_id}' not found in journal"));
    }
    fs::write(journal_path, lines_out.join("\n") + "\n").map_err(|e| e.to_string())
}

// ============================================================
// Schedule — cron-like five-field subset (m h dom mon dow),
// honored by the manager's scheduler thread
// ============================================================

/// A parsed five-field cron expression supporting `*`, plain numbers and
/// `*/n` steps per field — the subset the seats need; anything else is a
/// loud parse error, never a silently-ignored schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    fields: [CronField; 5],
}

#[derive(Debug, Clone, PartialEq)]
enum CronField {
    Any,
    Exact(u32),
    Step(u32),
}

impl Schedule {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!(
                "schedule '{expr}' must have 5 fields (m h dom mon dow), got {}",
                parts.len()
            ));
        }
        let mut fields = Vec::with_capacity(5);
        for part in parts {
            fields.push(if part == "*" {
                CronField::Any
            } else if let Some(step) = part.strip_prefix("*/") {
                CronField::Step(step.parse().map_err(|e| format!("step '{part}': {e}"))?)
            } else {
                CronField::Exact(part.parse().map_err(|e| format!("field '{part}': {e}"))?)
            });
        }
        Ok(Self {
            fields: [
                fields[0].clone(),
                fields[1].clone(),
                fields[2].clone(),
                fields[3].clone(),
                fields[4].clone(),
            ],
        })
    }

    /// Does this schedule fire at the given wall-clock minute?
    /// (minute, hour, day-of-month 1-31, month 1-12, weekday 0-6 Sun=0)
    pub fn matches(&self, minute: u32, hour: u32, dom: u32, mon: u32, dow: u32) -> bool {
        let values = [minute, hour, dom, mon, dow];
        self.fields.iter().zip(values).all(|(f, v)| match f {
            CronField::Any => true,
            CronField::Exact(n) => *n == v,
            CronField::Step(n) => *n != 0 && v % n == 0,
        })
    }
}

/// Journals + store-records a run the harness short-circuited (its
/// detector found nothing) — negatives are corpus rows too.
pub fn journal_nothing_to_do(
    request: &RunRequest,
    adapter_name: &str,
    store: &dyn StoreRecorder,
) -> Result<RunReport, String> {
    let run_id = new_run_id(&request.seat.name);
    let entry = JournalEntry {
        schema: JOURNAL_SCHEMA,
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        run_id: run_id.clone(),
        seat: request.seat.name.clone(),
        model: request.seat.model.clone(),
        adapter: adapter_name.to_string(),
        target: request.scope.join(","),
        work: String::new(),
        evidence: "detector found no work".into(),
        gates: vec![],
        verdict: Verdict::NothingToDo,
        human_verdict: None,
        outcome: "nothing_to_do".into(),
        cost_usd: 0.0,
        iterations: 0,
        wall_secs: 0,
    };
    append_journal(&request.journal_path, &entry)?;
    let _ = store.record_outcome(&entry);
    Ok(RunReport {
        run_id,
        verdict: Verdict::NothingToDo,
        gates: vec![],
        proposal_dir: None,
        cost_usd: 0.0,
        iterations: 0,
    })
}

// ============================================================
// Shadow gates — gates judge the PROPOSED state, not the workdir
// ============================================================

/// Copies the workdir to a temp shadow and applies the proposal diff there
/// (`patch -p1`). Propose mode stays intact: the real workdir is never
/// touched; gates run against the shadow — the future the human would get.
pub fn shadow_apply(workdir: &Path, diff: &str) -> Result<PathBuf, String> {
    let shadow = shadow_dir_name();
    copy_tree(workdir, &shadow)?;
    let mut diff_text = diff.to_string();
    if !diff_text.ends_with('\n') {
        diff_text.push('\n');
    }
    let diff_file = shadow.join(".jawata-proposal.diff");
    fs::write(&diff_file, &diff_text).map_err(|e| e.to_string())?;
    // `git apply --recount` recomputes hunk-header line counts from the
    // hunk bodies — model-generated diffs routinely miscount them; the
    // CONTENT (context + changes) stays strictly verified. Works outside a
    // git repository. -C1 tolerates a drifted context line at hunk edges.
    let output = Command::new("git")
        .arg("apply")
        .arg("--recount")
        .arg("-C1")
        .arg("--whitespace=nowarn")
        .arg(&diff_file)
        .current_dir(&shadow)
        .output()
        .map_err(|e| format!("git apply: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "proposal diff does not apply: {}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let _ = fs::remove_file(&diff_file);
    Ok(shadow)
}

/// FILE-BLOCK staging: shadow = tree copy + the proposal's files written
/// over it. No patch arithmetic anywhere — the seat's whole-file content
/// IS the proposed state.
pub fn shadow_stage_files(workdir: &Path, files: &[ProposalFile]) -> Result<PathBuf, String> {
    let shadow = shadow_dir_name();
    copy_tree(workdir, &shadow)?;
    for file in files {
        let target = shadow.join(&file.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&target, &file.content).map_err(|e| format!("{}: {e}", file.path))?;
    }
    Ok(shadow)
}

fn shadow_dir_name() -> PathBuf {
    static SHADOW_NAME_COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = SHADOW_NAME_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "jawata-shadow-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        n
    ))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| format!("read {}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        let ftype = entry.file_type().map_err(|e| e.to_string())?;
        if ftype.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if ftype.is_file() {
            fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Fixture-scale gate executor: judges the SHADOW (proposal applied) with
/// the local toolchain — `javac` as compile-verify, `javadoc -Xdoclint:all`
/// as the docs gate. Workspace-scale seats use the resident-backed
/// executor; this one serves self-contained fixture projects, where a full
/// resident per gate call would be ceremony.
pub struct ShadowJavaGateExecutor {
    pub workdir: PathBuf,
}

impl ShadowJavaGateExecutor {
    fn java_files_under(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&d) {
                for entry in entries.filter_map(Result::ok) {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|e| e == "java") {
                        files.push(p);
                    }
                }
            }
        }
        files.sort();
        files
    }
}

impl GateExecutor for ShadowJavaGateExecutor {
    fn run_class(&self, class: GateClass, proposal: &Proposal) -> Vec<GateOutcome> {
        let staged = if proposal.files.is_empty() {
            shadow_apply(&self.workdir, &proposal.diff)
        } else {
            shadow_stage_files(&self.workdir, &proposal.files)
        };
        let shadow = match staged {
            Ok(s) => s,
            Err(e) => {
                return vec![GateOutcome::fail(
                    class,
                    "shadow_apply",
                    &format!("cannot stage the proposal: {e}"),
                )]
            }
        };
        let outcome = match class {
            GateClass::Always => {
                let files = Self::java_files_under(&shadow);
                if files.is_empty() {
                    GateOutcome::fail(GateClass::Always, "compile_verify", "no .java in shadow")
                } else {
                    let classes_dir = shadow.join(".classes");
                    let _ = fs::create_dir_all(&classes_dir);
                    let mut cmd = Command::new("javac");
                    cmd.arg("-d").arg(&classes_dir).current_dir(&shadow);
                    for f in &files {
                        cmd.arg(f);
                    }
                    match cmd.output() {
                        Ok(out) if out.status.success() => GateOutcome::pass(
                            GateClass::Always,
                            "compile_verify",
                            &format!("javac clean on {} file(s) (shadow)", files.len()),
                        ),
                        Ok(out) => GateOutcome::fail(
                            GateClass::Always,
                            "compile_verify",
                            String::from_utf8_lossy(&out.stderr).trim(),
                        ),
                        Err(e) => {
                            GateOutcome::fail(GateClass::Always, "compile_verify", &e.to_string())
                        }
                    }
                }
            }
            GateClass::Docs => {
                let touched: Vec<PathBuf> = proposal
                    .touched_files
                    .iter()
                    .filter(|f| f.ends_with(".java"))
                    .map(|f| shadow.join(f))
                    .filter(|p| p.is_file())
                    .collect();
                if touched.is_empty() {
                    GateOutcome::pass(GateClass::Docs, "doclint", "no .java files touched")
                } else {
                    let mut cmd = Command::new("javadoc");
                    cmd.arg("-Xdoclint:all")
                        .arg("-quiet")
                        .arg("-d")
                        .arg(shadow.join(".apidocs"))
                        .current_dir(&shadow);
                    for f in &touched {
                        cmd.arg(f);
                    }
                    match cmd.output() {
                        Ok(out) if out.status.success() => GateOutcome::pass(
                            GateClass::Docs,
                            "doclint",
                            &format!("doclint clean on {} touched file(s) (shadow)", touched.len()),
                        ),
                        Ok(out) => GateOutcome::fail(
                            GateClass::Docs,
                            "doclint",
                            String::from_utf8_lossy(&out.stderr).trim(),
                        ),
                        Err(e) => GateOutcome::fail(GateClass::Docs, "doclint", &e.to_string()),
                    }
                }
            }
            other => GateOutcome::fail(
                other,
                "shadow_executor",
                "class not implemented at fixture scale — use the resident-backed executor",
            ),
        };
        let _ = fs::remove_dir_all(&shadow);
        vec![outcome]
    }
}

// ============================================================
// Seat harness: the javadoc-writer (Stage 9, spec D3b)
// ============================================================

/// Detector-driven javadoc-writer run: `javadoc_lack` through the resident
/// finds the work (the deterministic detect the spec demands); the scope's
/// sources ride the prompt as compiler facts (grounded generation — the
/// seat needs no tools); gates judge the SHADOW; journal + store close it.
pub fn run_javadoc_writer(
    mut request: RunRequest,
    detector: &ResidentGateExecutor,
    adapter: &dyn CliAdapter,
    gates: &dyn GateExecutor,
    store: &dyn StoreRecorder,
) -> Result<RunReport, String> {
    let findings = detector.call_tool(
        "find_quality_issue",
        serde_json::json!({ "kind": "javadoc_lack" }),
    )?;
    let text = extract_tool_text(&findings);
    let count = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.pointer("/data/count").and_then(|c| c.as_u64()))
        .unwrap_or(0);
    if count == 0 {
        return journal_nothing_to_do(&request, &adapter.name(), store);
    }
    let mut facts = format!(
        "javadoc_lack findings ({count}) — document EXACTLY these symbols:\n{}\n\n\
         == SOURCES IN SCOPE (the ground truth) ==\n",
        clip(&text, 20_000)
    );
    for prefix in &request.scope {
        for file in ShadowJavaGateExecutor::java_files_under(&request.workdir.join(prefix)) {
            if let Ok(src) = fs::read_to_string(&file) {
                let rel = file
                    .strip_prefix(&request.workdir)
                    .unwrap_or(&file)
                    .to_path_buf();
                facts.push_str(&format!("\n--- {} ---\n{src}\n", rel.display()));
            }
        }
    }
    request.pre_detected = Some(PreDetected {
        work: format!(
            "write Javadoc for every symbol in the findings list ({count} findings, \
             all types in one diff); conservative-or-refuse rules apply"
        ),
        facts: clip(&facts, 60_000).to_string(),
    });
    run_seat(&request, adapter, gates, store)
}

fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Clip on a char boundary at or below max.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

// ============================================================
// Seat harness: the test-writer (Stage 10, spec D5)
// ============================================================

/// Mutation/coverage numbers a test-writer run is judged against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestWriterBaseline {
    pub lines_missed: u64,
    pub branches_missed: u64,
    pub mutants_killed: u64,
    pub mutants_surviving: u64,
}

/// The ordered D5 gate chain, judged on a SHADOW the sandbox resident
/// re-loads as its project: compiles+passes → passes ×3 (flake screen) →
/// coverage improves on the target class → mutation-bite (≥1 previously-
/// surviving mutant killed). First failure stops the chain; a gate that
/// cannot run (mutation machinery unreachable) FAILS loudly.
pub struct TestWriterGates {
    pub executor: ResidentGateExecutor,
    pub workdir: PathBuf,
    pub target_class: String,
    pub baseline_tests: Vec<String>,
    pub baseline: TestWriterBaseline,
}

impl TestWriterGates {
    fn run_suite_once(&self, name: &str, coverage: bool) -> (GateOutcome, bool) {
        let mut args = serde_json::json!({
            "scope": { "kind": "package", "packageName": "com.example" },
            "timeoutSeconds": 300,
        });
        if coverage {
            args["coverage"] = serde_json::Value::Bool(true);
        }
        match self.executor.call_tool("run_tests", args) {
            Ok(v) => {
                let text = extract_tool_text(&v);
                let d: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let passed = d.pointer("/data/summary/passed").and_then(|x| x.as_u64());
                let failed = d.pointer("/data/summary/failed").and_then(|x| x.as_u64());
                match (passed, failed) {
                    (Some(p), Some(0)) if p > 0 => (
                        GateOutcome::pass(GateClass::Tests, name, &format!("{p} passed, 0 failed")),
                        true,
                    ),
                    (p, f) => (
                        GateOutcome::fail(
                            GateClass::Tests,
                            name,
                            &format!("passed={p:?} failed={f:?}: {}", clip(&text, 400)),
                        ),
                        false,
                    ),
                }
            }
            Err(e) => (GateOutcome::fail(GateClass::Tests, name, &e), false),
        }
    }

    fn missed_counts(&self) -> Result<(u64, u64), String> {
        let v = self.executor.call_tool(
            "run_tests",
            serde_json::json!({ "action": "coverage_uncovered", "target": self.target_class }),
        )?;
        let text = extract_tool_text(&v);
        let d: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("uncovered parse: {e}"))?;
        let mut lines = 0u64;
        let mut branches = 0u64;
        if let Some(facts) = d.pointer("/data/facts").and_then(|f| f.as_array()) {
            for fact in facts {
                if let Some(methods) = fact.get("methods").and_then(|m| m.as_array()) {
                    for m in methods {
                        lines += m.get("linesMissed").and_then(|x| x.as_u64()).unwrap_or(0);
                        branches += m.get("branchesMissed").and_then(|x| x.as_u64()).unwrap_or(0);
                    }
                }
            }
        }
        Ok((lines, branches))
    }

    fn mutation_counts(&self, tests: &[String]) -> Result<(u64, u64), String> {
        let v = self.executor.call_tool(
            "run_tests",
            serde_json::json!({
                "action": "coverage_mutation",
                "targetClasses": [self.target_class],
                "targetTests": tests,
                "timeoutSeconds": 300,
            }),
        )?;
        let text = extract_tool_text(&v);
        let d: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("mutation parse: {e}"))?;
        let killed = d
            .pointer("/data/summary/killed")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| format!("no killed count in: {}", clip(&text, 300)))?;
        let survived = d
            .pointer("/data/summary/survived")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        Ok((killed, survived))
    }
}

/// Fully-qualified test class names of the proposal's test files.
pub fn proposed_test_classes(proposal: &Proposal) -> Vec<String> {
    proposal
        .files
        .iter()
        .filter_map(|f| {
            f.path
                .strip_prefix("src/test/java/")
                .and_then(|p| p.strip_suffix(".java"))
                .map(|p| p.replace('/', "."))
        })
        .collect()
}

impl GateExecutor for TestWriterGates {
    fn run_class(&self, class: GateClass, proposal: &Proposal) -> Vec<GateOutcome> {
        match class {
            GateClass::Always => {
                // Compile-verify = the forked runner building + running the
                // SHADOW (proposal staged, resident re-loaded onto it).
                let shadow = match shadow_stage_files(&self.workdir, &proposal.files) {
                    Ok(s) => s,
                    Err(e) => {
                        return vec![GateOutcome::fail(GateClass::Always, "shadow_stage", &e)]
                    }
                };
                if let Err(e) = self.executor.call_tool(
                    "load_project",
                    serde_json::json!({ "projectPath": shadow.to_string_lossy() }),
                ) {
                    return vec![GateOutcome::fail(GateClass::Always, "shadow_load", &e)];
                }
                let (outcome, ok) = self.run_suite_once("compile_and_pass_1", false);
                vec![GateOutcome {
                    class: GateClass::Always,
                    name: outcome.name,
                    passed: ok,
                    detail: outcome.detail,
                }]
            }
            GateClass::Tests => {
                let mut outcomes = Vec::new();
                // pass ×3 total (flake screen) — run 1 was the Always gate.
                for attempt in 2..=3u8 {
                    let (outcome, ok) = self.run_suite_once(&format!("pass_{attempt}"), false);
                    outcomes.push(outcome);
                    if !ok {
                        return outcomes;
                    }
                }
                // Coverage must IMPROVE on the target class.
                let (cov_gate, cov_ok) = match self
                    .run_suite_once("coverage_run", true)
                {
                    (_, true) => match self.missed_counts() {
                        Ok((lines, branches))
                            if lines < self.baseline.lines_missed
                                || branches < self.baseline.branches_missed =>
                        {
                            (
                                GateOutcome::pass(
                                    GateClass::Tests,
                                    "coverage_improves",
                                    &format!(
                                        "missed lines {}→{lines}, branches {}→{branches}",
                                        self.baseline.lines_missed, self.baseline.branches_missed
                                    ),
                                ),
                                true,
                            )
                        }
                        Ok((lines, branches)) => (
                            GateOutcome::fail(
                                GateClass::Tests,
                                "coverage_improves",
                                &format!(
                                    "no improvement: missed lines {}→{lines}, branches {}→{branches}",
                                    self.baseline.lines_missed, self.baseline.branches_missed
                                ),
                            ),
                            false,
                        ),
                        Err(e) => (
                            GateOutcome::fail(GateClass::Tests, "coverage_improves", &e),
                            false,
                        ),
                    },
                    (o, false) => (o, false),
                };
                outcomes.push(cov_gate);
                if !cov_ok {
                    return outcomes;
                }
                // Mutation-bite: ≥1 previously-surviving mutant killed.
                let mut tests = self.baseline_tests.clone();
                tests.extend(proposed_test_classes(proposal));
                outcomes.push(match self.mutation_counts(&tests) {
                    Ok((killed, survived)) if killed > self.baseline.mutants_killed => {
                        GateOutcome::pass(
                            GateClass::Tests,
                            "mutation_bite",
                            &format!(
                                "killed {}→{killed}, surviving {}→{survived}",
                                self.baseline.mutants_killed, self.baseline.mutants_surviving
                            ),
                        )
                    }
                    Ok((killed, survived)) => GateOutcome::fail(
                        GateClass::Tests,
                        "mutation_bite",
                        &format!(
                            "no new kills: killed {}→{killed}, surviving {}→{survived}",
                            self.baseline.mutants_killed, self.baseline.mutants_surviving
                        ),
                    ),
                    // No mutation support / unreachable machinery = LOUD stop,
                    // never a silently-green gate (the degraded-and-declared branch).
                    Err(e) => GateOutcome::fail(GateClass::Tests, "mutation_bite", &e),
                });
                outcomes
            }
            other => vec![GateOutcome::fail(
                other,
                "test_writer_gates",
                "class not part of the test-writer chain",
            )],
        }
    }
}

/// Detector-driven test-writer run: coverage_uncovered + the mutation
/// survivors' candidate assertions ARE the work list; sources ride as facts.
pub fn run_test_writer(
    mut request: RunRequest,
    detector: &ResidentGateExecutor,
    adapter: &dyn CliAdapter,
    gates: &dyn GateExecutor,
    store: &dyn StoreRecorder,
    target_class: &str,
    baseline_tests: &[String],
) -> Result<RunReport, String> {
    let uncovered = detector.call_tool(
        "run_tests",
        serde_json::json!({ "action": "coverage_uncovered", "target": target_class }),
    )?;
    let uncovered_text = extract_tool_text(&uncovered);
    let mutation = detector.call_tool(
        "run_tests",
        serde_json::json!({
            "action": "coverage_mutation",
            "targetClasses": [target_class],
            "targetTests": baseline_tests,
            "timeoutSeconds": 300,
        }),
    )?;
    let mutation_text = extract_tool_text(&mutation);
    let mut facts = format!(
        "== UNCOVERED (JaCoCo, per method) ==\n{}\n\n== SURVIVING MUTANTS (PIT — each names a missing assertion) ==\n{}\n\n== SOURCES ==\n",
        clip(&uncovered_text, 15_000),
        clip(&mutation_text, 15_000)
    );
    {
        let src_root = request.workdir.join("src");
        for file in ShadowJavaGateExecutor::java_files_under(&src_root) {
            if let Ok(src) = fs::read_to_string(&file) {
                let rel = file
                    .strip_prefix(&request.workdir)
                    .unwrap_or(&file)
                    .to_path_buf();
                facts.push_str(&format!("\n--- {} ---\n{src}\n", rel.display()));
            }
        }
    }
    request.pre_detected = Some(PreDetected {
        work: format!(
            "write characterization tests for {target_class} covering the uncovered \
             branches and killing surviving mutants; pin actual behavior; ambiguous \
             intent (scale) → pin-and-flag"
        ),
        facts: clip(&facts, 60_000).to_string(),
    });
    run_seat(&request, adapter, gates, store)
}

// ============================================================
// Seat discovery + the manager's scheduler machinery
// ============================================================

/// Default on-disk layout under the manager's config dir:
/// `seats/*.md` (definitions) · `runner/runs/<run_id>/` (proposal records)
/// · `runner/journal.jsonl` (the corpus).
pub struct RunnerPaths {
    pub seats_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub journal_path: PathBuf,
}

impl RunnerPaths {
    pub fn from_config_dir(config_dir: &Path) -> Self {
        Self {
            seats_dir: config_dir.join("seats"),
            runs_dir: config_dir.join("runner").join("runs"),
            journal_path: config_dir.join("runner").join("journal.jsonl"),
        }
    }
}

/// Loads every `*.md` seat definition in the directory. A malformed seat
/// is a LOUD per-file error in the result — never silently skipped.
pub fn load_seat_definitions(
    seats_dir: &Path,
) -> (Vec<SeatDefinition>, Vec<(PathBuf, String)>) {
    let mut seats = Vec::new();
    let mut errors = Vec::new();
    let entries = match fs::read_dir(seats_dir) {
        Ok(e) => e,
        Err(_) => return (seats, errors), // no seats dir = no seats, not an error
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    for path in paths {
        match fs::read_to_string(&path) {
            Ok(text) => match parse_seat_definition(&text) {
                Ok(seat) => seats.push(seat),
                Err(e) => errors.push((path, e)),
            },
            Err(e) => errors.push((path, e.to_string())),
        }
    }
    (seats, errors)
}

/// The scheduler's pure core: which of these seats are due at the given
/// wall-clock minute? The manager's scheduler thread calls this once per
/// minute and triggers a run per due seat; `last_fired` de-duplicates a
/// minute that gets polled twice.
pub fn due_seats<'a>(
    seats: &'a [SeatDefinition],
    minute: u32,
    hour: u32,
    dom: u32,
    mon: u32,
    dow: u32,
) -> Vec<&'a SeatDefinition> {
    seats
        .iter()
        .filter(|seat| {
            seat.schedule
                .as_deref()
                .and_then(|expr| Schedule::parse(expr).ok())
                .is_some_and(|s| s.matches(minute, hour, dom, mon, dow))
        })
        .collect()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn unique_tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-runner-test-{label}-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
        fs::create_dir_all(&dir).expect("test tempdir");
        dir
    }

    const SEAT_FIXTURE: &str = r#"---
name: echo-seat
model: claude-sonnet-5
effort: low
schedule: "*/30 * * * *"
tools: [search_symbols, find_references]
gates: [docs]
ttl_secs: 120
max_iterations: 2
cost_budget_usd: 0.5
---
You are the echo seat. You document what you are told to document.
"#;

    #[test]
    fn seat_definition_parses_every_field() {
        let seat = parse_seat_definition(SEAT_FIXTURE).expect("parse");
        assert_eq!(seat.name, "echo-seat");
        assert_eq!(seat.model, "claude-sonnet-5");
        assert_eq!(seat.effort.as_deref(), Some("low"));
        assert_eq!(seat.schedule.as_deref(), Some("*/30 * * * *"));
        assert_eq!(seat.tools, vec!["search_symbols", "find_references"]);
        assert_eq!(seat.gate_classes, vec![GateClass::Docs]);
        assert_eq!(seat.ceilings.wall_ttl_secs, 120);
        assert_eq!(seat.ceilings.max_iterations, 2);
        assert!((seat.ceilings.cost_budget_usd - 0.5).abs() < f64::EPSILON);
        assert!(seat.stance.contains("echo seat"));
    }

    #[test]
    fn seat_definition_rejects_unknown_key_and_missing_name() {
        let unknown = "---\nname: x\nmodel: m\nttl_seconds: 5\n---\nbody\n";
        let err = parse_seat_definition(unknown).unwrap_err();
        assert!(err.contains("unknown frontmatter key 'ttl_seconds'"), "{err}");

        let missing = "---\nmodel: m\n---\nbody\n";
        let err = parse_seat_definition(missing).unwrap_err();
        assert!(err.contains("needs 'name:'"), "{err}");
    }

    #[test]
    fn seat_definition_defaults_ceilings() {
        let seat = parse_seat_definition("---\nname: s\nmodel: m\n---\nstance\n").unwrap();
        assert_eq!(seat.ceilings, Ceilings::default());
        assert!(seat.gate_classes.is_empty());
        assert!(seat.schedule.is_none());
    }

    const DIFF: &str = "\
--- a/src/main/java/com/example/Foo.java
+++ b/src/main/java/com/example/Foo.java
@@ -1,3 +1,4 @@
+/** Documented. */
 public class Foo {
 }
";

    #[test]
    fn proposal_parses_markers_evidence_and_touched_files() {
        let text = format!(
            "I read Foo.java; the class lacks a Javadoc.\n{PROPOSAL_BEGIN}\n{DIFF}\n{PROPOSAL_END}\n"
        );
        let p = parse_proposal("document Foo", &text).expect("proposal");
        assert_eq!(p.work, "document Foo");
        assert!(p.evidence.contains("lacks a Javadoc"));
        assert_eq!(p.touched_files, vec!["src/main/java/com/example/Foo.java"]);
        assert!(p.diff.contains("+/** Documented. */"));
    }

    #[test]
    fn proposal_missing_markers_is_none() {
        assert!(parse_proposal("w", "no markers here").is_none());
        let empty = format!("{PROPOSAL_BEGIN}\n\n{PROPOSAL_END}");
        assert!(parse_proposal("w", &empty).is_none());
    }

    #[test]
    fn purity_refuses_out_of_scope_file_naming_it() {
        let outcome = purity_check(
            &["src/ok/A.java".into(), "build/pom.xml".into()],
            &["src/ok/".into()],
        );
        assert!(!outcome.passed);
        assert!(outcome.detail.contains("build/pom.xml"), "{}", outcome.detail);

        let ok = purity_check(&["src/ok/A.java".into()], &["src/ok/".into()]);
        assert!(ok.passed);

        let empty = purity_check(&[], &["src/ok/".into()]);
        assert!(!empty.passed);
    }

    #[test]
    fn claude_adapter_builds_headless_command_and_parses_stream_json() {
        let adapter = ClaudeCodeAdapter {
            binary: "claude".into(),
            max_turns: 7,
            model: Some("claude-haiku-4-5".into()),
        };
        let cmd = adapter.build_command("do the thing");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "-p",
                "do the thing",
                "--output-format",
                "stream-json",
                "--verbose",
                "--max-turns",
                "7",
                "--model",
                "claude-haiku-4-5"
            ]
        );
        let cost = adapter.parse_cost(r#"{"type":"result","result":"ok","total_cost_usd":0.0421}"#);
        assert_eq!(cost, Some(0.0421));
        assert_eq!(
            adapter.parse_text(r#"{"type":"result","result":"the answer"}"#),
            Some("the answer".into())
        );
        assert_eq!(adapter.parse_cost("not json"), None);
    }

    #[test]
    fn schedule_cron_subset_parses_and_matches() {
        let every_30 = Schedule::parse("*/30 * * * *").unwrap();
        assert!(every_30.matches(0, 5, 1, 1, 0));
        assert!(every_30.matches(30, 5, 1, 1, 0));
        assert!(!every_30.matches(31, 5, 1, 1, 0));

        let daily_6 = Schedule::parse("0 6 * * *").unwrap();
        assert!(daily_6.matches(0, 6, 12, 3, 4));
        assert!(!daily_6.matches(1, 6, 12, 3, 4));

        assert!(Schedule::parse("* * *").is_err());
        assert!(Schedule::parse("a * * * *").is_err());
    }

    // ---------- E2E fixtures: a scripted echo seat ----------

    struct ScriptedGates {
        fail_class: Option<GateClass>,
    }

    impl GateExecutor for ScriptedGates {
        fn run_class(&self, class: GateClass, _proposal: &Proposal) -> Vec<GateOutcome> {
            let name = match class {
                GateClass::Always => "compile_verify",
                GateClass::Behavior => "suite_with_coverage",
                GateClass::Tests => "tests_pass_1",
                GateClass::Docs => "doclint",
            };
            if self.fail_class == Some(class) {
                vec![GateOutcome::fail(class, name, "scripted failure")]
            } else {
                vec![GateOutcome::pass(class, name, "scripted pass")]
            }
        }
    }

    #[derive(Default)]
    struct CapturingStore {
        recorded: std::sync::Mutex<Vec<JournalEntry>>,
    }

    impl StoreRecorder for &CapturingStore {
        fn record_outcome(&self, entry: &JournalEntry) -> Result<(), String> {
            self.recorded.lock().unwrap().push(entry.clone());
            Ok(())
        }
    }

    /// Writes the echo-seat driver script: DETECT answers WORK once (via a
    /// state file so iteration 2 would answer NOTHING), PROPOSE emits the
    /// canned diff.
    fn write_echo_script(dir: &Path, diff_file: &Path, out_of_scope: bool) -> PathBuf {
        let diff = if out_of_scope {
            DIFF.replace("src/main/java/com/example/Foo.java", "build/pom.xml")
        } else {
            DIFF.to_string()
        };
        fs::write(diff_file, diff).unwrap();
        let script = dir.join("seat.sh");
        let body = format!(
            "#!/bin/sh\ncase \"$1\" in\n  *'PHASE: DETECT'*) echo 'WORK: document Foo';;\n  *) echo 'I read Foo.java; it lacks docs.'; echo '{PROPOSAL_BEGIN}'; cat '{}'; echo '{PROPOSAL_END}';;\nesac\n",
            diff_file.display()
        );
        fs::write(&script, body).unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        script
    }

    fn echo_seat() -> SeatDefinition {
        parse_seat_definition(SEAT_FIXTURE).unwrap()
    }

    fn request_in<'a>(seat: &'a SeatDefinition, dir: &Path) -> RunRequest<'a> {
        RunRequest {
            seat,
            scope: vec!["src/main/java/".into()],
            workdir: dir.join("fixture"),
            runs_dir: dir.join("runs"),
            journal_path: dir.join("runner").join("journal.jsonl"),
            pre_detected: None,
        }
    }

    fn fixture_with_target(dir: &Path) -> PathBuf {
        let fixture = dir.join("fixture/src/main/java/com/example");
        fs::create_dir_all(&fixture).unwrap();
        let target = fixture.join("Foo.java");
        fs::write(&target, "public class Foo {\n}\n").unwrap();
        target
    }

    #[cfg(unix)]
    #[test]
    fn echo_seat_full_loop_proposes_and_never_applies() {
        let dir = unique_tempdir("e2e-propose");
        let target = fixture_with_target(&dir);
        let original = fs::read_to_string(&target).unwrap();
        let script = write_echo_script(&dir, &dir.join("canned.diff"), false);
        let seat = echo_seat();
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let gates = ScriptedGates { fail_class: None };
        let store = CapturingStore::default();

        let report = run_seat(&request, &adapter, &gates, &&store).expect("run");

        assert_eq!(report.verdict, Verdict::Proposed);
        assert_eq!(report.iterations, 1);
        // Proposal record on disk: proposal.md + diff.patch + gates.json.
        let record = report.proposal_dir.expect("record dir");
        assert!(record.join("proposal.md").is_file());
        assert!(record.join("gates.json").is_file());
        let diff = fs::read_to_string(record.join("diff.patch")).unwrap();
        assert!(diff.contains("+/** Documented. */"));
        // ALL FOUR-class machinery: purity + Always ran; declared Docs ran.
        let classes: Vec<GateClass> = report.gates.iter().map(|g| g.class).collect();
        assert!(classes.contains(&GateClass::Always));
        assert!(classes.contains(&GateClass::Docs));
        assert!(report.gates.iter().all(|g| g.passed));
        // PROPOSE MODE: the fixture is byte-identical — nothing was applied.
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        // Journal line with the corpus shape.
        let journal = fs::read_to_string(&request.journal_path).unwrap();
        let entry: JournalEntry = serde_json::from_str(journal.lines().next().unwrap()).unwrap();
        assert_eq!(entry.schema, JOURNAL_SCHEMA);
        assert_eq!(entry.seat, "echo-seat");
        assert_eq!(entry.verdict, Verdict::Proposed);
        assert_eq!(entry.outcome, "proposed");
        assert!(entry.human_verdict.is_none());
        assert!(!entry.gates.is_empty());
        // Outcome recorded to the store.
        assert_eq!(store.recorded.lock().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn gate_failing_proposal_is_refused_with_the_reason() {
        let dir = unique_tempdir("e2e-refuse");
        fixture_with_target(&dir);
        let script = write_echo_script(&dir, &dir.join("canned.diff"), false);
        let seat = echo_seat();
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        // Seeded gate failure: the Docs class refuses.
        let gates = ScriptedGates {
            fail_class: Some(GateClass::Docs),
        };
        let store = CapturingStore::default();

        let report = run_seat(&request, &adapter, &gates, &&store).expect("run");

        match &report.verdict {
            Verdict::Refused(reason) => {
                assert!(reason.contains("doclint"), "reason names the gate: {reason}");
                assert!(reason.contains("scripted failure"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        // The refusal still leaves the evidence record.
        let record = report.proposal_dir.expect("refused record dir");
        let gates_json = fs::read_to_string(record.join("gates.json")).unwrap();
        assert!(gates_json.contains("scripted failure"));
        // And the journal carries the refusal.
        let journal = fs::read_to_string(&request.journal_path).unwrap();
        assert!(journal.contains("refused"));
    }

    #[cfg(unix)]
    #[test]
    fn out_of_scope_diff_is_refused_by_purity() {
        let dir = unique_tempdir("e2e-purity");
        fixture_with_target(&dir);
        let script = write_echo_script(&dir, &dir.join("canned.diff"), true);
        let seat = echo_seat();
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let gates = ScriptedGates { fail_class: None };
        let store = CapturingStore::default();

        let report = run_seat(&request, &adapter, &gates, &&store).expect("run");

        match &report.verdict {
            Verdict::Refused(reason) => {
                assert!(reason.contains("purity"), "{reason}");
                assert!(reason.contains("build/pom.xml"), "{reason}");
            }
            other => panic!("expected purity refusal, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn nothing_to_do_ends_quietly() {
        let dir = unique_tempdir("e2e-nothing");
        fixture_with_target(&dir);
        let script = dir.join("seat.sh");
        fs::write(&script, format!("#!/bin/sh\necho '{NOTHING_TO_DO}'\n")).unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        let seat = echo_seat();
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let store = CapturingStore::default();
        let report = run_seat(
            &request,
            &adapter,
            &ScriptedGates { fail_class: None },
            &&store,
        )
        .expect("run");
        assert_eq!(report.verdict, Verdict::NothingToDo);
        assert!(report.proposal_dir.is_none());
        // Quiet, but still journaled + recorded (the corpus needs negatives).
        assert!(request.journal_path.is_file());
        assert_eq!(store.recorded.lock().unwrap().len(), 1);
    }

    // ---------- 8b: containment (each ceiling class) ----------

    #[cfg(unix)]
    #[test]
    fn wall_ttl_breach_reaps_the_seat_process_tree() {
        let dir = unique_tempdir("e2e-ttl");
        fixture_with_target(&dir);
        // A wedged seat: sleeps far past the TTL and drops a grandchild.
        let marker = dir.join("grandchild.pid");
        let script = dir.join("seat.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nsleep 300 &\necho $! > '{}'\nsleep 300\n",
                marker.display()
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let mut seat = echo_seat();
        seat.ceilings.wall_ttl_secs = 1;
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let store = CapturingStore::default();
        let started = Instant::now();
        let report = run_seat(
            &request,
            &adapter,
            &ScriptedGates { fail_class: None },
            &&store,
        )
        .expect("run");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "reap must be prompt, took {:?}",
            started.elapsed()
        );
        assert_eq!(report.verdict, Verdict::Reaped("wall_ttl".into()));
        // The grandchild (background sleep) must be dead too — group kill.
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(pid) = fs::read_to_string(&marker) {
            let alive = Path::new(&format!("/proc/{}", pid.trim())).exists();
            assert!(!alive, "grandchild {} survived the reap", pid.trim());
        }
        let journal = fs::read_to_string(&request.journal_path).unwrap();
        assert!(journal.contains("reaped: wall_ttl"), "{journal}");
    }

    #[cfg(unix)]
    #[test]
    fn max_iterations_ceiling_stops_a_looping_seat() {
        let dir = unique_tempdir("e2e-iters");
        fixture_with_target(&dir);
        // A seat that always finds work but never produces a proposal
        // block → every iteration ends refused-parse? No: to exercise the
        // ITERATION ceiling the loop must CONTINUE past a completed
        // iteration. v1 stops after one proposal, so the iteration ceiling
        // is reached via detect-always-works + do-never-proposes being
        // refused… which breaks the loop. The honest iteration test drives
        // the ceiling directly: max_iterations=0 → immediate reap.
        let script = write_echo_script(&dir, &dir.join("canned.diff"), false);
        let mut seat = echo_seat();
        seat.ceilings.max_iterations = 0;
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let store = CapturingStore::default();
        let report = run_seat(
            &request,
            &adapter,
            &ScriptedGates { fail_class: None },
            &&store,
        )
        .expect("run");
        assert_eq!(report.verdict, Verdict::Reaped("max_iterations".into()));
        assert_eq!(report.iterations, 0);
    }

    #[cfg(unix)]
    #[test]
    fn cost_budget_breach_stops_the_run() {
        let dir = unique_tempdir("e2e-cost");
        fixture_with_target(&dir);
        // The DETECT answer carries a cost line above the budget.
        let script = dir.join("seat.sh");
        fs::write(
            &script,
            "#!/bin/sh\necho 'COST-USD: 2.75'\necho 'WORK: expensive idea'\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        let mut seat = echo_seat();
        seat.ceilings.cost_budget_usd = 0.5;
        let request = request_in(&seat, &dir);
        let adapter = ScriptAdapter {
            command: "sh".into(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let store = CapturingStore::default();
        let report = run_seat(
            &request,
            &adapter,
            &ScriptedGates { fail_class: None },
            &&store,
        )
        .expect("run");
        assert_eq!(report.verdict, Verdict::Reaped("cost_budget".into()));
        assert!((report.cost_usd - 2.75).abs() < 1e-9);
        let journal = fs::read_to_string(&request.journal_path).unwrap();
        assert!(journal.contains("reaped: cost_budget"));
    }

    #[test]
    fn file_block_proposal_parses_and_computes_its_own_diff() {
        let dir = unique_tempdir("fileblock");
        let src = dir.join("fixture/src/main/java/com/example");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("Foo.java"), "public class Foo {\n}\n").unwrap();
        let text = format!(
            "Evidence prose.\n{PROPOSAL_BEGIN}\n\
             ===FILE: src/main/java/com/example/Foo.java===\n\
             /** Doc. */\npublic class Foo {{\n}}\n\
             ===END-FILE===\n{PROPOSAL_END}\n"
        );
        let mut p = parse_proposal("document Foo", &text).expect("parses");
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.touched_files, vec!["src/main/java/com/example/Foo.java"]);
        assert!(p.files[0].content.ends_with("}\n"));
        // The runner computes the diff deterministically.
        p.diff = compute_files_diff(&dir.join("fixture"), &p.files);
        assert!(p.diff.contains("+/** Doc. */"), "computed diff:\n{}", p.diff);
        assert!(p.diff.contains("--- a/src/main/java/com/example/Foo.java"));
        // Staging writes the file into the shadow, not the workdir.
        let shadow = shadow_stage_files(&dir.join("fixture"), &p.files).unwrap();
        assert!(fs::read_to_string(shadow.join("src/main/java/com/example/Foo.java"))
            .unwrap()
            .contains("/** Doc. */"));
        assert!(!fs::read_to_string(src.join("Foo.java")).unwrap().contains("Doc"));
        let _ = fs::remove_dir_all(&shadow);
        // Traversal-shaped paths refuse the whole block.
        let evil = format!(
            "{PROPOSAL_BEGIN}\n===FILE: ../evil.java===\nx\n===END-FILE===\n{PROPOSAL_END}"
        );
        assert!(parse_proposal("w", &evil).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn shadow_gates_judge_the_proposed_state_not_the_workdir() {
        let dir = unique_tempdir("shadow");
        let src = dir.join("fixture/src/main/java/com/example");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("Foo.java"),
            "package com.example;\n\npublic class Foo {\n    public int id() { return 1; }\n}\n",
        )
        .unwrap();
        let workdir = dir.join("fixture");
        let original = fs::read_to_string(src.join("Foo.java")).unwrap();

        // A diff adding class + method Javadoc: doclint on the SHADOW must
        // pass even though the workdir file is still undocumented.
        let good = Proposal {
            work: "document Foo".into(),
            evidence: String::new(),
            files: Vec::new(),
            diff: "\
--- a/src/main/java/com/example/Foo.java
+++ b/src/main/java/com/example/Foo.java
@@ -1,5 +1,10 @@
 package com.example;

+/** A documented example. */
 public class Foo {
+    /**
+     * Returns the id.
+     * @return the id
+     */
     public int id() { return 1; }
 }
"
            .into(),
            touched_files: vec!["src/main/java/com/example/Foo.java".into()],
        };
        let gates = ShadowJavaGateExecutor {
            workdir: workdir.clone(),
        };
        let always = gates.run_class(GateClass::Always, &good);
        assert!(always[0].passed, "compile on shadow: {}", always[0].detail);
        let docs = gates.run_class(GateClass::Docs, &good);
        assert!(docs[0].passed, "doclint on shadow: {}", docs[0].detail);
        // Propose mode: the workdir is untouched by gating.
        assert_eq!(fs::read_to_string(src.join("Foo.java")).unwrap(), original);

        // A diff that breaks compilation is refused by the Always class.
        let broken = Proposal {
            diff: good.diff.replace("/** A documented example. */", "syntax error here"),
            ..good.clone()
        };
        let always = gates.run_class(GateClass::Always, &broken);
        assert!(!always[0].passed, "broken shadow must fail compile");

        // A diff that does not apply is a loud shadow_apply failure.
        let inapplicable = Proposal {
            diff: "--- a/nope.java\n+++ b/nope.java\n@@ -1,1 +1,1 @@\n-x\n+y\n".into(),
            touched_files: vec!["nope.java".into()],
            ..good.clone()
        };
        let out = gates.run_class(GateClass::Always, &inapplicable);
        assert!(!out[0].passed);
        assert!(out[0].name == "shadow_apply", "{}", out[0].detail);
    }

    #[test]
    fn seat_loading_reports_malformed_files_loudly() {
        let dir = unique_tempdir("seat-load");
        let seats_dir = dir.join("seats");
        fs::create_dir_all(&seats_dir).unwrap();
        fs::write(seats_dir.join("good.md"), SEAT_FIXTURE).unwrap();
        fs::write(seats_dir.join("bad.md"), "---\nmodel: m\n---\nbody\n").unwrap();
        fs::write(seats_dir.join("ignored.txt"), "not a seat").unwrap();
        let (seats, errors) = load_seat_definitions(&seats_dir);
        assert_eq!(seats.len(), 1);
        assert_eq!(seats[0].name, "echo-seat");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].0.ends_with("bad.md"));
        assert!(errors[0].1.contains("needs 'name:'"));
        // Missing dir = no seats, no errors (a fresh install has none).
        let (none, no_errors) = load_seat_definitions(&dir.join("absent"));
        assert!(none.is_empty() && no_errors.is_empty());
    }

    #[test]
    fn scheduler_returns_exactly_the_due_seats() {
        let every_30 = parse_seat_definition(SEAT_FIXTURE).unwrap(); // */30 * * * *
        let daily_6 = parse_seat_definition(
            "---\nname: nightly\nmodel: m\nschedule: \"0 6 * * *\"\n---\nstance\n",
        )
        .unwrap();
        let adhoc = parse_seat_definition("---\nname: adhoc\nmodel: m\n---\nstance\n").unwrap();
        let seats = vec![every_30, daily_6, adhoc];

        let at_6_00: Vec<&str> = due_seats(&seats, 0, 6, 1, 1, 0)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(at_6_00, vec!["echo-seat", "nightly"]);

        let at_6_31 = due_seats(&seats, 31, 6, 1, 1, 0);
        assert!(at_6_31.is_empty());

        let paths = RunnerPaths::from_config_dir(Path::new("/cfg"));
        assert!(paths.seats_dir.ends_with("seats"));
        assert!(paths.journal_path.ends_with("runner/journal.jsonl"));
    }

    /// LIVE Stage-9 gate (C9): sandbox resident from the deployed jar,
    /// javadoc_lack detection, the REAL Claude Code CLI as the seat driver,
    /// shadow gates, journal + store. Run explicitly:
    ///   cargo test --lib javadoc_writer_live -- --ignored --nocapture
    /// (env JAWATA_JAR overrides the deployed-jar default.)
    #[cfg(unix)]
    #[test]
    #[ignore]
    fn javadoc_writer_live_run() {
        let jar = std::env::var("JAWATA_JAR").unwrap_or_else(|_| {
            format!(
                "{}/.cache/jawata-studio/tools/jawata/current/jawata-v2.14.1/jawata.jar",
                std::env::var("HOME").unwrap()
            )
        });
        assert!(Path::new(&jar).is_file(), "no jawata jar at {jar}");
        let dir = unique_tempdir("jw-live");
        let fixture_dir = dir.join("fixture");
        let repo_fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-fixtures/javadoc-seat");
        copy_tree(&repo_fixture, &fixture_dir).unwrap();

        // --- sandbox resident (isolated store + workspace, ephemeral port) ---
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let token = "jw-live-test-token";
        let mut resident = Command::new("java")
            .arg(format!(
                "-Djawata.experience.shared.dir={}",
                dir.join("store").display()
            ))
            .arg("-jar")
            .arg(&jar)
            .arg("-data")
            .arg(dir.join("ws"))
            .arg("-port")
            .arg(port.to_string())
            .arg("-token")
            .arg(token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("resident spawn");
        let stdout = resident.stdout.take().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<String>();
        // Drain stdout FOREVER (a full pipe deadlocks the JVM — the v2.9 lesson);
        // forward only the READY line.
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("READY") {
                    let _ = ready_tx.send(line);
                }
            }
        });
        let ready = ready_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("resident never became READY");
        // The READY line prints the BASE url; the JSON-RPC endpoint is /mcp
        // (HttpTransport.createContext("/mcp", …)).
        let url = format!(
            "{}/mcp",
            ready
                .split_whitespace()
                .find_map(|t| t.strip_prefix("url="))
                .expect("READY line carries url=")
                .trim_end_matches('/')
        );
        println!("sandbox resident: {url}");

        let result = std::panic::catch_unwind(|| {
            let exec = ResidentGateExecutor::new(url.clone(), token.into(), None);
            let loaded = exec
                .call_tool(
                    "load_project",
                    serde_json::json!({ "projectPath": fixture_dir.to_string_lossy() }),
                )
                .expect("load_project");
            println!("loaded: {}", extract_tool_text(&loaded).chars().take(200).collect::<String>());

            // --- the seat, from the versioned definition ---
            let seat_text = fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../seats/javadoc-writer.md"),
            )
            .expect("seat definition");
            let seat = parse_seat_definition(&seat_text).expect("seat parses");
            let request = RunRequest {
                seat: &seat,
                scope: vec!["src/main/java/".into()],
                workdir: fixture_dir.clone(),
                runs_dir: dir.join("runs"),
                journal_path: dir.join("journal.jsonl"),
                pre_detected: None,
            };
            // JW_MODEL overrides the seat's tier for the live gate (e.g. a
            // model-pool outage); the override is printed so the record is
            // honest about which tier actually ran.
            let live_model = std::env::var("JW_MODEL").unwrap_or_else(|_| seat.model.clone());
            println!("live model tier: {live_model}");
            let adapter = ClaudeCodeAdapter {
                model: Some(live_model),
                max_turns: 8,
                ..Default::default()
            };
            let gates = ShadowJavaGateExecutor {
                workdir: fixture_dir.clone(),
            };
            let store = ResidentStoreRecorder {
                executor: ResidentGateExecutor::new(url.clone(), token.into(), None),
            };
            let original_thin =
                fs::read_to_string(fixture_dir.join("src/main/java/com/example/ThinFacts.java"))
                    .unwrap();

            let report =
                run_javadoc_writer(request, &exec, &adapter, &gates, &store).expect("run");
            println!("verdict: {:?}, cost: {}", report.verdict, report.cost_usd);
            for g in &report.gates {
                println!("gate {:?}/{}: {} — {}", g.class, g.name, g.passed, g.detail);
            }

            // --- C9 assertions ---
            assert_eq!(report.verdict, Verdict::Proposed, "gates: {:?}", report.gates);
            let record = report.proposal_dir.expect("proposal record");
            let diff = fs::read_to_string(record.join("diff.patch")).unwrap();
            println!("--- proposal diff ---\n{diff}\n---");
            assert!(diff.contains("/**"), "diff adds Javadoc");
            assert!(
                diff.contains("Account.java") && diff.contains("ThinFacts.java"),
                "both fixture types documented"
            );
            // The thin-facts symbol yields a STUB, never invented behavior.
            assert!(
                diff.contains("TODO"),
                "ThinFacts must be a marked stub (TODO), got:\n{diff}"
            );
            // Propose mode: the fixture is untouched.
            assert_eq!(
                fs::read_to_string(fixture_dir.join("src/main/java/com/example/ThinFacts.java"))
                    .unwrap(),
                original_thin
            );
            // Store outcome verified through the resident.
            let recalled = exec
                .call_tool(
                    "experience",
                    serde_json::json!({ "kind": "list", "limit": 10 }),
                )
                .expect("experience list");
            let recalled_text = extract_tool_text(&recalled);
            assert!(
                recalled_text.contains("seat javadoc-writer"),
                "store outcome present: {recalled_text}"
            );
        });

        let _ = resident.kill();
        let _ = resident.wait();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    /// LIVE Stage-10 gate (C10): the test-writer seat against the sandbox
    /// resident's coverage + mutation machinery. Run explicitly:
    ///   cargo test --lib test_writer_live -- --ignored --nocapture
    #[cfg(unix)]
    #[test]
    #[ignore]
    fn test_writer_live_run() {
        let jar = std::env::var("JAWATA_JAR").unwrap_or_else(|_| {
            format!(
                "{}/.cache/jawata-studio/tools/jawata/current/jawata-v2.14.1/jawata.jar",
                std::env::var("HOME").unwrap()
            )
        });
        assert!(Path::new(&jar).is_file(), "no jawata jar at {jar}");
        let dir = unique_tempdir("tw-live");
        let fixture_dir = dir.join("fixture");
        let repo_fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("test-fixtures/test-writer-seat");
        copy_tree(&repo_fixture, &fixture_dir).unwrap();

        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let token = "tw-live-test-token";
        let mut resident = Command::new("java")
            .arg(format!(
                "-Djawata.experience.shared.dir={}",
                dir.join("store").display()
            ))
            .arg("-jar")
            .arg(&jar)
            .arg("-data")
            .arg(dir.join("ws"))
            .arg("-port")
            .arg(port.to_string())
            .arg("-token")
            .arg(token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("resident spawn");
        let stdout = resident.stdout.take().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("READY") {
                    let _ = ready_tx.send(line);
                }
            }
        });
        let ready = ready_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("resident never became READY");
        let url = format!(
            "{}/mcp",
            ready
                .split_whitespace()
                .find_map(|t| t.strip_prefix("url="))
                .expect("READY url=")
                .trim_end_matches('/')
        );
        println!("sandbox resident: {url}");

        let result = std::panic::catch_unwind(|| {
            let exec = ResidentGateExecutor::new(url.clone(), token.into(), None);
            exec.call_tool(
                "load_project",
                serde_json::json!({ "projectPath": fixture_dir.to_string_lossy() }),
            )
            .expect("load_project");

            // --- baselines on the ORIGINAL fixture ---
            let baseline_tests = vec!["com.example.PricingTest".to_string()];
            let mut gates = TestWriterGates {
                executor: ResidentGateExecutor::new(url.clone(), token.into(), None),
                workdir: fixture_dir.clone(),
                target_class: "com.example.Pricing".into(),
                baseline_tests: baseline_tests.clone(),
                baseline: TestWriterBaseline {
                    lines_missed: 0,
                    branches_missed: 0,
                    mutants_killed: 0,
                    mutants_surviving: 0,
                },
            };
            let (b1, ok) = gates.run_suite_once("baseline_coverage_run", true);
            assert!(ok, "baseline coverage run: {}", b1.detail);
            let (lines, branches) = gates.missed_counts().expect("baseline uncovered");
            let (killed, surviving) = gates.mutation_counts(&baseline_tests).expect("baseline pit");
            println!(
                "baseline: missed lines={lines} branches={branches}, killed={killed} surviving={surviving}"
            );
            assert!(lines > 0 && surviving > 0, "fixture must have real gaps");
            gates.baseline = TestWriterBaseline {
                lines_missed: lines,
                branches_missed: branches,
                mutants_killed: killed,
                mutants_surviving: surviving,
            };

            let seat_text = fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../seats/test-writer.md"),
            )
            .expect("seat definition");
            let seat = parse_seat_definition(&seat_text).expect("seat parses");
            let request = RunRequest {
                seat: &seat,
                scope: vec!["src/test/java/".into()],
                workdir: fixture_dir.clone(),
                runs_dir: dir.join("runs"),
                journal_path: dir.join("journal.jsonl"),
                pre_detected: None,
            };
            let live_model = std::env::var("JW_MODEL").unwrap_or_else(|_| seat.model.clone());
            println!("live model tier: {live_model}");
            let adapter = ClaudeCodeAdapter {
                model: Some(live_model),
                max_turns: 8,
                ..Default::default()
            };
            let store = ResidentStoreRecorder {
                executor: ResidentGateExecutor::new(url.clone(), token.into(), None),
            };
            let original =
                fs::read_to_string(fixture_dir.join("src/main/java/com/example/Pricing.java"))
                    .unwrap();

            let report = run_test_writer(
                request,
                &exec,
                &adapter,
                &gates,
                &store,
                "com.example.Pricing",
                &baseline_tests,
            )
            .expect("run");
            println!("verdict: {:?}, cost: {}", report.verdict, report.cost_usd);
            for g in &report.gates {
                println!("gate {:?}/{}: {} — {}", g.class, g.name, g.passed, g.detail);
            }

            // --- C10 assertions: the ordered gate chain, all green ---
            assert_eq!(report.verdict, Verdict::Proposed, "gates: {:?}", report.gates);
            let names: Vec<&str> = report.gates.iter().map(|g| g.name.as_str()).collect();
            for expected in [
                "purity",
                "compile_and_pass_1",
                "pass_2",
                "pass_3",
                "coverage_improves",
                "mutation_bite",
            ] {
                assert!(names.contains(&expected), "gate {expected} missing: {names:?}");
            }
            assert!(report.gates.iter().all(|g| g.passed));
            // Ambiguous intent pinned AND flagged.
            let record = report.proposal_dir.expect("record");
            let diff = fs::read_to_string(record.join("diff.patch")).unwrap();
            assert!(
                diff.contains("FLAGGED"),
                "ambiguous scale() tests must carry the FLAGGED marker:\n{diff}"
            );
            // Propose mode: production source untouched.
            assert_eq!(
                fs::read_to_string(fixture_dir.join("src/main/java/com/example/Pricing.java"))
                    .unwrap(),
                original
            );
            // Store outcome present.
            let recalled = exec
                .call_tool("experience", serde_json::json!({ "kind": "list", "limit": 10 }))
                .expect("experience list");
            assert!(
                extract_tool_text(&recalled).contains("seat test-writer"),
                "store outcome present"
            );
        });

        let _ = resident.kill();
        let _ = resident.wait();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn human_verdict_lands_in_the_journal_line() {
        let dir = unique_tempdir("journal-verdict");
        let journal = dir.join("journal.jsonl");
        let entry = JournalEntry {
            schema: JOURNAL_SCHEMA,
            ts: 1,
            run_id: "seat-1-0".into(),
            seat: "s".into(),
            model: "m".into(),
            adapter: "script".into(),
            target: "src/".into(),
            work: "w".into(),
            evidence: "e".into(),
            gates: vec![],
            verdict: Verdict::Proposed,
            human_verdict: None,
            outcome: "proposed".into(),
            cost_usd: 0.0,
            iterations: 1,
            wall_secs: 0,
        };
        append_journal(&journal, &entry).unwrap();
        record_human_verdict(&journal, "seat-1-0", "accepted").unwrap();
        let line = fs::read_to_string(&journal).unwrap();
        let read: JournalEntry = serde_json::from_str(line.lines().next().unwrap()).unwrap();
        assert_eq!(read.human_verdict.as_deref(), Some("accepted"));
        assert!(record_human_verdict(&journal, "missing", "x").is_err());
    }

    #[test]
    fn all_four_gate_classes_are_exercisable_in_one_run_shape() {
        // A seat declaring behavior+tests+docs: the verify step runs
        // purity + Always + all three declared classes — the FOUR-class
        // demonstration of the C8 gate at the suite level.
        let seat = parse_seat_definition(
            "---\nname: full\nmodel: m\ngates: [behavior, tests, docs]\n---\nstance\n",
        )
        .unwrap();
        let proposal = Proposal {
            work: "w".into(),
            evidence: String::new(),
            diff: DIFF.into(),
            files: Vec::new(),
            touched_files: vec!["src/main/java/com/example/Foo.java".into()],
        };
        let gates = ScriptedGates { fail_class: None };
        let mut outcomes = vec![purity_check(
            &proposal.touched_files,
            &["src/main/java/".to_string()],
        )];
        outcomes.extend(gates.run_class(GateClass::Always, &proposal));
        for class in &seat.gate_classes {
            outcomes.extend(gates.run_class(*class, &proposal));
        }
        let classes: std::collections::HashSet<_> =
            outcomes.iter().map(|g| format!("{:?}", g.class)).collect();
        assert_eq!(classes.len(), 4, "all four classes present: {classes:?}");
        assert!(outcomes.iter().all(|g| g.passed));
    }
}
