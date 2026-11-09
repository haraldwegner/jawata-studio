//! The seat file: what a seat IS, read from its markdown + frontmatter.
//!
//! Sprint 28h S15 — moved out of `runner.rs` unchanged. The Rust agent runner is reference code
//! that S20 deletes after the switch; the seat reader is not: the conductor (which renders each
//! seat as a command for each client), the deploy writer, the engine floor and the seat settings
//! all read seats through it. This module is the part of the old runner those four depend on,
//! and nothing in it starts, drives or judges a run.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ceilings {
    /// Wall-clock limit for the WHOLE run (all phases together).
    pub wall_ttl_secs: u64,
    /// Maximum detect→do→verify iterations before the run is stopped.
    pub max_iterations: u32,
    /// Cost ceiling in USD, accumulated from the adapter's usage events.
    pub cost_budget_usd: f64,
    /// Maximum TOOL CALLS for the whole run, accumulated across phases.
    ///
    /// This is not the same bound as the adapter's turn limit
    /// (`--max-turns`), and that is the reason it exists: ONE model turn may
    /// carry several tool calls, so a turn bound leaves the tool-call count
    /// unbounded. A RUNAWAY bound rather than a budget — like the wall TTL
    /// it is not expected to fire on an ordinary run, and a seat that needs
    /// more says so in its own frontmatter.
    pub max_tool_steps: u32,
}

impl Default for Ceilings {
    fn default() -> Self {
        Self {
            wall_ttl_secs: 600,
            max_iterations: 3,
            cost_budget_usd: 1.0,
            max_tool_steps: 150,
        }
    }
}

/// The file a project declares itself in, at its own root.
///
/// It lives WITH THE CODE rather than in the studio's `projects.json`, and the
/// two are not alternatives: `projects.json` is this machine's bookkeeping
/// about a workspace (resident port, token, heap ceiling), written by the
/// manager and true only here. A declaration is the project's own statement
/// about itself, committed, reviewed, and true on any machine that checks the
/// repository out — including one with no studio installed at all.
pub const PROJECT_DECLARATION_FILE: &str = "jawata-project.md";

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
/// WHAT A SEAT'S INPUT IS, which decides whether reaching for a compiler-accurate
/// instrument is optional.
///
/// His ruling, which this encodes: *"the auditor and the communicator read text and use none;
/// the architect must use them; the production seats already do."*
///
/// It is REQUIRED and it is CROSS-CHECKED against `tools`, because a field nothing reads is
/// one more frontmatter line — a defect this project spent a whole checkpoint on, where every
/// gate asserted a new field was WRITTEN and none that anything READ it. Here the pairing
/// decides whether a seat loads at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeatKind {
    /// Reads an ARTIFACT — a document, a diff, a report. Its judgement is about prose, there is
    /// no compiler fact to fetch, and an empty `tools` is correct rather than lazy.
    Text,
    /// Reads the CODEBASE. A compiler-accurate answer exists for every question it asks, so an
    /// empty `tools` is a contradiction: the seat would reach for a text search, which is the
    /// one thing this product exists to replace.
    Code,
}

impl SeatKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "code" => Ok(Self::Code),
            other => Err(format!(
                "kind '{other}': a seat is 'text' (reads an artifact, no tools) or \
                 'code' (reads the codebase, must name tools)"
            )),
        }
    }
}

/// A seat's own gate for its scheduled runs, read from `schedule_gate: <tool> [<json object>]`.
///
/// THE PAIRING with the engine's `ScheduledHires.Gate`: the same value is read by the same grammar
/// and refused for the same reasons, so the studio and the engine never disagree about a seat file.
/// A key that is present and cannot be read is refused with its reason, never read as absent: that
/// would hand a seat that asked for a stronger check the weakest one in silence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleGate {
    /// A published tool name: lower case letters, digits and underscores, starting with a letter.
    pub tool: String,
    /// The arguments it is called with; empty when the seat names none.
    pub arguments: serde_json::Map<String, serde_json::Value>,
}

/// The engine's own `Character.isWhitespace`, which is not Rust's: the no-break spaces are not
/// whitespace there, the information separators are, and U+0085 is not.
fn is_java_whitespace(c: char) -> bool {
    match c {
        '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{0085}' => false,
        '\u{001C}'..='\u{001F}' => true,
        _ => c.is_whitespace(),
    }
}

/// The engine's frontmatter `unquote`: ONE matching quote pair around the whole value.
fn unquote(value: &str) -> &str {
    let b = value.as_bytes();
    if b.len() >= 2 && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\'')) {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

impl ScheduleGate {
    /// Read the text after `schedule_gate:`. The error starts with the key and says why.
    pub fn parse(raw: &str) -> Result<Self, String> {
        const KEY: &str = "schedule_gate";
        let declared = unquote(raw.trim_matches(is_java_whitespace)).trim_matches(is_java_whitespace);
        if declared.is_empty() {
            return Err(format!(
                "{KEY} is present but names no tool: write the tool name, then optionally a JSON \
                 object of its arguments, or remove the key to be gated by compile_workspace"
            ));
        }
        let (tool, rest) = match declared.find(is_java_whitespace) {
            Some(space) => (&declared[..space], declared[space..].trim_matches(is_java_whitespace)),
            None => (declared, ""),
        };
        let mut chars = tool.chars();
        let name_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !name_ok {
            return Err(format!(
                "{KEY} '{declared}': '{tool}' is not a tool name (lower case letters, digits and \
                 underscores, starting with a letter)"
            ));
        }
        if rest.is_empty() {
            return Ok(Self { tool: tool.to_string(), arguments: serde_json::Map::new() });
        }
        // serde_json refuses trailing characters, as the engine's reader refuses trailing tokens.
        match serde_json::from_str::<serde_json::Value>(rest) {
            Ok(serde_json::Value::Object(arguments)) => Ok(Self { tool: tool.to_string(), arguments }),
            Ok(_) => Err(format!(
                "{KEY} '{declared}': the arguments after '{tool}' must be a JSON object"
            )),
            Err(e) => Err(format!(
                "{KEY} '{declared}': the arguments after '{tool}' are not valid JSON: {e}"
            )),
        }
    }
}

/// (optional schedule), WITH WHAT (tools allowlist), judged HOW (gate
/// classes + ceilings), and the stance prompt (the markdown body).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatDefinition {
    pub name: String,
    /// The model to ask the brain for. EMPTY when the seat names none, which means the agent
    /// program's own default. Optional since 2026-10-12 (Harald: *"No fake model names!"*): the
    /// shipped seats named `claude-sonnet-5` and `claude-haiku-4-5`, which no program on the
    /// machine vouched for, so they name none until a name is read off the brain itself.
    pub model: String,
    /// Which agent program a HIRE of this seat starts (`claude-code`, `cursor`, `codex`,
    /// `copilot`). The engine's hire path reads the same key and defaults an absent one to
    /// Claude Code; this parser used to refuse it as an unknown key, so a seat the engine could
    /// hire was one studio could not load.
    #[serde(default)]
    pub brain: Option<String>,
    /// The slash command a person types for this seat (architect is `refactor`), written once in the
    /// seat's own file. The engine's hire path reads the same key so the Agent Control panel names the
    /// seat the way the command picker does; the studio's command table is held equal to it by a test.
    #[serde(default)]
    pub command: Option<String>,
    /// The executable a `brain: script` seat runs (S16-6) — the engine's escape hatch for a CLI
    /// with no dialect of its own. Read here under the engine's own two rules, so one file never
    /// gets two answers: a script brain must name one, and no other brain may carry one (the
    /// other brains would ignore it, and an ignored line lets its writer believe they chose
    /// what runs).
    #[serde(default)]
    pub script: Option<String>,
    /// WHAT this seat reads — see [`SeatKind`]. Required; cross-checked against `tools`.
    pub kind: SeatKind,
    pub effort: Option<String>,
    /// Cron-like five-field schedule (`m h dom mon dow`). Parsed and matched
    /// by `scheduler_tick` (library-level, test-proven); NOT yet fired by the
    /// manager — production scheduling ships with Lane B in Sprint 28.
    /// `None` = ad-hoc only.
    pub schedule: Option<String>,
    /// The gate a SCHEDULED hire of this seat is checked by (`schedule_gate: <tool> [<json object>]`).
    /// The engine's scheduler reads the same key under the same grammar and the same refusals
    /// (`ScheduledHires.Gate`); `None` keeps its default, the compile gate. Sprint 28h S12.
    #[serde(default)]
    pub schedule_gate: Option<ScheduleGate>,
    /// Where the seat sits in the workflow (28h, D51): the seat that hires it, what it is (`lead`,
    /// `worker`, `reviewer`) and where a task it could not do goes next. The engine reads the same
    /// three keys when it loads the tree of seats (`SeatTree`); a blank `escalates_to:` says it goes
    /// nowhere. `None` keeps the signed tree's own value.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub escalates_to: Option<String>,
    /// Tool allowlist injected into the seat prompt (advisory for the CLI
    /// v1 contract; hard enforcement is a later graduation).
    pub tools: Vec<String>,
    /// Gate classes beyond `Always` this seat's proposals must pass.
    pub gate_classes: Vec<GateClass>,
    /// The seat declares, in its own frontmatter, that the gate rule does not apply to it
    /// (`gate_exempt: true`). The default is false and a seat gains the exemption only by
    /// saying so in writing: the generated command otherwise tells EVERY seat to run
    /// `compile_workspace`, which contradicts a seat whose subject is not Java and whose own
    /// stance declares itself exempt (C7-2).
    #[serde(default)]
    pub gate_exempt: bool,
    /// Checks the seat declares over its OWN answer (`answer_gates:`), run by the engine's hire
    /// path as steps of the plan. Known: `upward_message` (the PM's upward gate, Sprint 28g D13).
    /// Validated here so a typo is refused at load instead of silently never running.
    #[serde(default)]
    pub answer_gates: Vec<String>,
    pub ceilings: Ceilings,
    /// The stance prompt — the markdown body below the frontmatter.
    pub stance: String,
    /// What this seat is FOR, in one sentence a person choosing between seats can read. Lives
    /// in the seat's own frontmatter so the seats view reads it rather than carrying a copy.
    /// Empty when the file declares none; a test requires every SHIPPED seat to declare one.
    #[serde(default)]
    pub purpose: String,
    /// WHEN this seat fires — the moment, not the schedule (`schedule` is the cron). Same
    /// rule as `purpose`: the seat file is the one place it is written.
    #[serde(default)]
    pub fires: String,
    /// Whether the seat is ON. Default true; only `enabled: false` in the seat FILE turns it
    /// off, and that line is written by this app when the user switches the seat off
    /// (`seat_settings::seat_file_content`). C11-6: the settings sidecar was consulted by the
    /// deploy alone, so the scheduler and the engine's hire — both of which read the seat file —
    /// went on treating an off seat as on. Carrying it in the file makes every reader see it.
    #[serde(default = "seat_enabled_by_default")]
    pub enabled: bool,
    /// The seat's own word that it may be hired on a brain whose writes are held only by an
    /// instruction to the model (S16-37). The engine's hire reads the same key and REFUSES such a
    /// brain without it; this parser used to refuse the key itself as unknown, so a seat the view
    /// had just written for Cursor was one studio could no longer load.
    #[serde(default)]
    pub accepts_uncontained_writes: bool,
    /// The seat's own word that its job is to CHANGE CODE, through the engine's tools (S10-8: the
    /// worker). The engine's hire reads the same key: absent means the hire runs read-only, so a
    /// seat that must write and does not say so has its first code-changing call refused.
    #[serde(default)]
    pub changes_code: bool,
    /// The seat's own word that it must READ the project's files (S10-12: the UX seat opens the
    /// component it is asked about). The engine's hire reads the same key and then offers the
    /// brain Read, Glob and Grep and no other built-in tool; absent means none. Reading is not the
    /// cut, writing is, so this is not a grant of writes.
    #[serde(default)]
    pub reads_files: bool,
    /// The seat's own word that its job is to read and write the knowledge store through its tool
    /// (S24: the rottenmeier seat). The engine's hire reads the same key and hires the seat outside
    /// plan mode, which refuses every `experience` call; the seat still changes no code and is
    /// offered none of the CLI's own file tools. Absent means plan mode.
    #[serde(default)]
    pub writes_store: bool,
    /// The activity labels this seat carries beyond its own name (Sprint 28g D45): a rule anchored
    /// to one of them, such as `release`, binds this seat's brief. The engine's hire reads the same
    /// key; a rule's label is matched EXACTLY, so the labels a seat asks for are written in its file
    /// and nowhere else.
    #[serde(default)]
    pub activities: Vec<String>,
}

/// A seat nobody switched off is on.
fn seat_enabled_by_default() -> bool {
    true
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
    let mut brain = None;
    let mut command = None;
    let mut script: Option<String> = None;
    let mut effort = None;
    let mut kind = None;
    let mut schedule = None;
    let mut schedule_gate: Option<ScheduleGate> = None;
    let mut parent: Option<String> = None;
    let mut role: Option<String> = None;
    let mut escalates_to: Option<String> = None;
    let mut tools: Vec<String> = Vec::new();
    let mut gate_classes: Vec<GateClass> = Vec::new();
    let mut gate_exempt = false;
    let mut enabled = true;
    let mut accepts_uncontained_writes = false;
    let mut changes_code = false;
    let mut reads_files = false;
    let mut writes_store = false;
    let mut activities: Vec<String> = Vec::new();
    let mut answer_gates: Vec<String> = Vec::new();
    let mut ceilings = Ceilings::default();
    let mut body_start = false;
    let mut stance = String::new();
    let mut purpose = String::new();
    let mut fires = String::new();
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
            "brain" => brain = Some(value.to_string()),
            "command" => command = Some(value.to_string()),
            "script" => script = Some(value.to_string()).filter(|s| !s.is_empty()),
            "effort" => effort = Some(value.to_string()),
            "kind" => kind = Some(SeatKind::parse(value)?),
            "schedule" => schedule = Some(value.trim_matches('"').to_string()),
            "schedule_gate" => schedule_gate = Some(ScheduleGate::parse(value)?),
            // The engine reads these three keys with its own rules and refuses the whole tree on a
            // broken one, naming the seat (`SeatTree`). The reader here keeps what the file says, a
            // blank parent included and a role in any case, so `workflow::refusals` can say the same
            // thing the engine will, instead of this parser dropping the seat from the view.
            "parent" => parent = Some(value.trim_matches('"').trim().to_string()),
            "role" => role = Some(value.trim_matches('"').trim().to_lowercase()),
            "escalates_to" => escalates_to = Some(value.trim_matches('"').to_string()),
            "purpose" => purpose = value.to_string(),
            "fires" => fires = value.to_string(),
            "tools" => tools = parse_inline_list(value)?,
            "gate_exempt" => {
                gate_exempt = match value {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("gate_exempt '{other}': expected true or false")),
                }
            }
            "enabled" => {
                enabled = match value {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("enabled '{other}': expected true or false")),
                }
            }
            // Read by the engine alone: `typed: false` keeps a seat off the chat's `/` list (D53 (d)).
            "typed" => {
                if value != "true" && value != "false" {
                    return Err(format!("typed '{value}': expected true or false"));
                }
            }
            "accepts_uncontained_writes" => {
                accepts_uncontained_writes = match value {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(format!(
                            "accepts_uncontained_writes '{other}': expected true or false"
                        ))
                    }
                }
            }
            "changes_code" => {
                changes_code = match value {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("changes_code '{other}': expected true or false")),
                }
            }
            "reads_files" => {
                reads_files = match value {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("reads_files '{other}': expected true or false")),
                }
            }
            "writes_store" => {
                writes_store = match value {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("writes_store '{other}': expected true or false")),
                }
            }
            "activities" => activities = parse_inline_list(value)?,
            "gates" => {
                for entry in parse_inline_list(value)? {
                    let class = GateClass::parse(&entry)?;
                    if class != GateClass::Always && !gate_classes.contains(&class) {
                        gate_classes.push(class);
                    }
                }
            }
            "answer_gates" => {
                for entry in parse_inline_list(value)? {
                    if entry != "upward_message" {
                        return Err(format!(
                            "answer_gates '{entry}': this build knows only upward_message"
                        ));
                    }
                    if !answer_gates.contains(&entry) {
                        answer_gates.push(entry);
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
            "max_tool_steps" => {
                ceilings.max_tool_steps = value
                    .parse()
                    .map_err(|e| format!("max_tool_steps '{value}': {e}"))?
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
    let model = model.unwrap_or_default();
    // REQUIRED rather than defaulted: a default would answer for the author which kind of work
    // this seat does, which is the one thing the field is for.
    let kind = kind.ok_or(
        "seat definition needs 'kind:' — 'text' (reads an artifact, no tools) or 'code' \
         (reads the codebase, must name tools)",
    )?;
    // THE PAIRING, and it is why the field is not decoration. Each refusal names BOTH halves,
    // so the author knows which one they meant to change.
    if kind == SeatKind::Code && tools.is_empty() {
        return Err(format!(
            "seat '{name}' declares 'kind: code' but names no tools — a seat that reads the \
             codebase has a compiler-accurate answer available for every question it asks, so \
             an empty 'tools' would send it to a text search instead"
        ));
    }
    if kind == SeatKind::Text && !tools.is_empty() {
        return Err(format!(
            "seat '{name}' declares 'kind: text' but names tools {tools:?} — a seat that reads \
             an artifact has no compiler fact to fetch, and an allowlist nothing needs is how \
             an allowlist stops meaning anything"
        ));
    }
    let stance_trimmed = stance.trim();
    if stance_trimmed.is_empty() {
        return Err("seat definition needs a stance body below the frontmatter".into());
    }
    // S16-6: the engine's two rules for the script brain, read the same here.
    let is_script = brain.as_deref().map(str::trim) == Some("script");
    if is_script && script.is_none() {
        return Err(format!(
            "seat '{name}' declares brain 'script' and no 'script:' line naming the executable \
             to run, so there is nothing to start"
        ));
    }
    if !is_script {
        if let Some(path) = &script {
            return Err(format!(
                "seat '{name}' declares script '{path}' on brain '{}', which would ignore it — \
                 set brain: script to run it, or remove the line",
                brain.as_deref().unwrap_or("claude-code")
            ));
        }
    }
    Ok(SeatDefinition {
        name,
        model,
        brain,
        command,
        script,
        kind,
        effort,
        schedule,
        schedule_gate,
        parent,
        role,
        escalates_to,
        tools,
        gate_classes,
        gate_exempt,
        enabled,
        accepts_uncontained_writes,
        changes_code,
        reads_files,
        writes_store,
        activities,
        answer_gates,
        ceilings,
        stance: stance_trimmed.to_string(),
        purpose,
        fires,
    })
}

pub(crate) fn parse_inline_list(value: &str) -> Result<Vec<String>, String> {
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


#[derive(Debug, Clone, PartialEq)]
enum CronField {
    Any,
    Exact(u32),
    Step(u32),
}

/// A parsed five-field cron expression supporting `*`, plain numbers and
/// `*/n` steps per field — the subset the seats need; anything else is a
/// loud parse error, never a silently-ignored schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    fields: [CronField; 5],
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

/// A complete seat file, shared by this module's tests and the runner's.
#[cfg(test)]
pub(crate) const SEAT_FIXTURE: &str = r#"---
name: echo-seat
model: claude-sonnet-5
kind: code
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("jawata-seat-file-test-{label}-{}-{nanos}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("test tempdir");
        dir
    }

    /// D12: EVERY SEAT STATES ITS TASK KIND, AND THE KIND HAS TEETH.
    ///
    /// His ruling: *"the auditor and the communicator read text and use none; the architect must
    /// use them; the production seats already do."* So the kind is not a label — it is a claim
    /// about what the seat's input IS, and the tools field has to agree with it:
    ///
    ///   `kind: text` — the seat reads an ARTIFACT (a document, a diff, a report). Its judgement
    ///   is about prose, there is no compiler fact to fetch, and an empty `tools` is correct.
    ///
    ///   `kind: code` — the seat reads the CODEBASE. A compiler-accurate answer exists for every
    ///   question it asks, so reaching for one is not optional and an empty `tools` is a
    ///   contradiction rather than a default.
    ///
    /// THE PAIRING IS WHAT MAKES THE FIELD LOAD-BEARING. A `kind` nothing cross-checks would be
    /// one more frontmatter line — this repository spent a whole checkpoint on a field whose
    /// every gate asserted it was WRITTEN and none that anything READ it. Here the parser
    /// refuses the two states that cannot be true, so the field decides whether a seat loads.
    #[test]
    fn a_code_seat_must_name_tools_and_a_text_seat_must_not() {
        let with = |kind: &str, tools: &str| {
            format!(
                "---\nname: s\nmodel: m\nkind: {kind}\ntools: {tools}\ngates: []\n\
                 ttl_secs: 60\nmax_iterations: 1\ncost_budget_usd: 0.1\n---\nbody\n"
            )
        };

        // The two legitimate shapes load.
        let text = parse_seat_definition(&with("text", "[]")).expect("a text seat with no tools");
        assert_eq!(text.kind, SeatKind::Text);
        assert!(text.tools.is_empty());
        let code = parse_seat_definition(&with("code", "[search_symbols]")).expect("a code seat");
        assert_eq!(code.kind, SeatKind::Code);

        // A code seat with no tools is the defect the pairing exists to catch: it would reach
        // for grep, which is what the whole product refuses.
        let err = parse_seat_definition(&with("code", "[]")).unwrap_err();
        assert!(
            err.contains("kind: code") && err.contains("tools"),
            "the refusal must name both halves so the author knows which to change; got: {err}"
        );

        // And a text seat holding tools is the other contradiction: it claims an instrument it
        // has no question for, which is how an allowlist stops meaning anything.
        let err = parse_seat_definition(&with("text", "[search_symbols]")).unwrap_err();
        assert!(err.contains("kind: text"), "got: {err}");
    }

    /// AND THE FIELD IS REQUIRED, because a default would answer the question for the author.
    #[test]
    fn a_seat_without_a_kind_does_not_load() {
        // NO `kind:` LINE — that is the whole fixture. A sweep that added the field to every
        // frontmatter in this file put one here too, and this test caught it, which is the
        // cheapest possible demonstration that the requirement is live.
        let no_kind = "---\nname: s\nmodel: m\ntools: []\ngates: []\n\
                       ttl_secs: 60\nmax_iterations: 1\ncost_budget_usd: 0.1\n---\nbody\n";
        let err = parse_seat_definition(no_kind).unwrap_err();
        assert!(err.contains("kind"), "got: {err}");
    }

    /// S10-8: `changes_code` is the key the engine's hire reads to decide whether a seat is hired
    /// read-only. This parser must load a seat carrying it (it refused the key as unknown, so the
    /// worker did not load at all) and read it as strictly as the engine does.
    #[test]
    fn a_seat_that_changes_code_loads_and_a_misspelled_grant_of_writes_is_refused() {
        let with = |value: &str| {
            format!("---\nname: w\nkind: code\ntools: [rename_symbol]\ngates: []\nchanges_code: {value}\n---\nbody\n")
        };
        assert!(parse_seat_definition(&with("true")).expect("loads").changes_code);
        assert!(!parse_seat_definition(&with("false")).expect("loads").changes_code);
        let absent = "---\nname: w\nkind: code\ntools: [rename_symbol]\ngates: []\n---\nbody\n";
        assert!(!parse_seat_definition(absent).expect("loads").changes_code, "absent is read-only");
        let err = parse_seat_definition(&with("yes")).unwrap_err();
        assert!(err.contains("changes_code") && err.contains("true or false"), "got: {err}");
    }

    /// S10-12: `reads_files` is the key the engine's hire reads to offer a seat's brain the
    /// read-only built-ins. Studio must load a seat carrying it and read it as strictly.
    #[test]
    fn a_seat_that_reads_files_loads_and_a_misspelled_grant_is_refused() {
        let with = |value: &str| {
            format!("---\nname: u\nkind: code\ntools: [experience]\ngates: []\nreads_files: {value}\n---\nbody\n")
        };
        assert!(parse_seat_definition(&with("true")).expect("loads").reads_files);
        assert!(!parse_seat_definition(&with("false")).expect("loads").reads_files);
        let absent = "---\nname: u\nkind: code\ntools: [experience]\ngates: []\n---\nbody\n";
        assert!(!parse_seat_definition(absent).expect("loads").reads_files, "absent offers none");
        let err = parse_seat_definition(&with("yes")).unwrap_err();
        assert!(err.contains("reads_files") && err.contains("true or false"), "got: {err}");
    }

    /// The seat file the engine hires is the one that says it reads: a seat that needs to open a
    /// component and does not say so is hired with no reader and reports a census of 0 (measured).
    #[test]
    fn the_ux_seat_file_declares_that_it_reads_files_and_no_other_shipped_seat_does_by_accident() {
        let ux = parse_seat_definition(include_str!("../../seats/ux.md")).expect("the ux seat loads");
        assert!(ux.reads_files, "ux.md must say reads_files: true");
        assert!(!ux.changes_code, "and reading is not writing: it still does not change code");
        let pm = parse_seat_definition(include_str!("../../seats/pm.md")).expect("the pm seat loads");
        assert!(!pm.reads_files, "a seat nobody ruled on is offered no built-in tool");
    }

    /// S24 R4: `writes_store` is the key the engine's hire reads to hire a seat outside plan mode,
    /// which refuses every `experience` call. Studio must load a seat carrying it, read it as
    /// strictly, and the shipped rottenmeier seat must say it — measured 2026-10-19, a hire without
    /// it could read none of his turns and wrote no rule.
    #[test]
    fn a_seat_that_writes_the_store_loads_and_the_shipped_rottenmeier_says_so() {
        let with = |value: &str| {
            format!("---\nname: g\nkind: code\ntools: [experience]\ngates: []\nwrites_store: {value}\n---\nbody\n")
        };
        assert!(parse_seat_definition(&with("true")).expect("loads").writes_store);
        assert!(!parse_seat_definition(&with("false")).expect("loads").writes_store);
        let absent = "---\nname: g\nkind: code\ntools: [experience]\ngates: []\n---\nbody\n";
        assert!(!parse_seat_definition(absent).expect("loads").writes_store, "absent is plan mode");
        let err = parse_seat_definition(&with("yes")).unwrap_err();
        assert!(err.contains("writes_store") && err.contains("true or false"), "got: {err}");

        let governess = parse_seat_definition(include_str!("../../seats/rottenmeier.md"))
            .expect("the rottenmeier seat loads");
        assert!(governess.writes_store, "rottenmeier.md must say writes_store: true");
        assert!(!governess.changes_code, "and writing the store is not changing code");
        let cataloguer = parse_seat_definition(include_str!("../../seats/cataloguer.md"))
            .expect("the cataloguer seat loads");
        assert!(
            cataloguer.writes_store,
            "the cataloguer's product is jobs written through `experience`; hired in plan mode its \
             one call was refused with 'Cannot call mcp__jawata__experience while in plan mode' \
             (measured 2026-10-19)"
        );
        assert!(!cataloguer.changes_code, "and it still changes no code");
        let pm = parse_seat_definition(include_str!("../../seats/pm.md")).expect("the pm seat loads");
        assert!(!pm.writes_store, "a seat nobody ruled on stays in plan mode");
    }

    #[test]
    fn a_seat_without_a_model_loads_and_means_the_brains_own_default() {
        let no_model = "---\nname: s\nkind: text\ntools: []\ngates: []\n---\nbody\n";
        let seat = parse_seat_definition(no_model).expect("a missing model is not an error");
        assert_eq!(seat.model, "", "empty means: ask the agent program for its own default");
    }

    /// S16-6: a seat the engine can hire through the script escape hatch is one studio loads —
    /// the boundary this parser broke once already for `brain:` itself — and the engine's two
    /// refusals hold here too, so one file never gets two answers.
    #[test]
    fn a_script_seat_loads_and_its_two_mismatches_are_refused_as_the_engine_refuses_them() {
        let head = "---\nname: s\nkind: text\ntools: []\ngates: []\n";
        let seat = parse_seat_definition(&format!("{head}brain: script\nscript: /opt/x.sh\n---\nbody\n"))
            .expect("a script seat loads");
        assert_eq!(seat.script.as_deref(), Some("/opt/x.sh"));

        let nothing_to_run = parse_seat_definition(&format!("{head}brain: script\n---\nbody\n"));
        assert!(nothing_to_run.unwrap_err().contains("script:"));

        let ignored = parse_seat_definition(&format!("{head}brain: codex\nscript: /opt/x.sh\n---\nbody\n"));
        assert!(ignored.unwrap_err().contains("codex"));
        let ignored_by_default =
            parse_seat_definition(&format!("{head}script: /opt/x.sh\n---\nbody\n"));
        assert!(ignored_by_default.unwrap_err().contains("claude-code"),
            "no brain means claude-code, which ignores a script too");
    }

    /// S10-2: a shipped seat's cost ceiling must cover what its own brief was MEASURED to cost.
    /// The test manager shipped with 1.0; a real hire of it was stopped at 2.36 before writing
    /// its report, and the same hire with room ended on its own at 3.065 with the full report
    /// (2026-10-14). A ceiling below the measured cost makes the seat unable to finish, which
    /// reads as the seat failing rather than as the number being wrong.
    #[test]
    fn the_test_managers_cost_ceiling_covers_the_run_it_was_measured_to_cost() {
        const MEASURED_FULL_RUN_USD: f64 = 3.065;
        let (_, body) = crate::conductor::EMBEDDED_SEATS
            .iter()
            .find(|(file, _)| file.ends_with("test-manager.md"))
            .expect("the test manager ships");
        let seat = parse_seat_definition(body).expect("it parses");
        assert!(
            seat.ceilings.cost_budget_usd >= MEASURED_FULL_RUN_USD,
            "the shipped ceiling {} is below the {} a full run of its brief was measured to cost",
            seat.ceilings.cost_budget_usd,
            MEASURED_FULL_RUN_USD
        );
    }

    /// Harald, 2026-10-12: *"No fake model names!"* The shipped seats named `claude-sonnet-5` and
    /// `claude-haiku-4-5`, which no program on the machine vouched for. Until a name is read off
    /// a brain itself, no shipped seat names one. The one exception is his own word of 2026-11-08 (D53 (j)), *"rottenmeier -> gets
    /// Claude sonnet as default"*, written as the alias `sonnet`, which the Claude program itself resolves.
    #[test]
    fn no_shipped_seat_names_a_model_nobody_verified() {
        for (file, body) in crate::conductor::EMBEDDED_SEATS.iter() {
            let seat = parse_seat_definition(body).unwrap_or_else(|e| panic!("{file}: {e}"));
            let expected = if *file == "rottenmeier.md" { "sonnet" } else { "" };
            assert_eq!(seat.model, expected, "{file} names model '{}'", seat.model);
        }
    }

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
        let unknown = "---\nname: x\nmodel: m\nkind: text\nttl_seconds: 5\n---\nbody\n";
        let err = parse_seat_definition(unknown).unwrap_err();
        assert!(err.contains("unknown frontmatter key 'ttl_seconds'"), "{err}");

        let missing = "---\nmodel: m\n---\nbody\n";
        let err = parse_seat_definition(missing).unwrap_err();
        assert!(err.contains("needs 'name:'"), "{err}");
    }

    #[test]
    fn answer_gates_are_read_and_an_unknown_one_is_refused_at_load() {
        // D13: a seat declares the checks over its own answer in its own frontmatter. A name the
        // engine has no check for must be refused here, or the seat is hired believing a gate runs.
        let ok = "---\nname: s\nmodel: m\nkind: text\nanswer_gates: [upward_message]\n---\nbody\n";
        assert_eq!(parse_seat_definition(ok).unwrap().answer_gates, vec!["upward_message".to_string()]);

        let none = "---\nname: s\nmodel: m\nkind: text\n---\nbody\n";
        assert!(parse_seat_definition(none).unwrap().answer_gates.is_empty());

        let typo = "---\nname: s\nmodel: m\nkind: text\nanswer_gates: [upward_mesage]\n---\nbody\n";
        let err = parse_seat_definition(typo).unwrap_err();
        assert!(err.contains("upward_mesage") && err.contains("upward_message"), "{err}");
    }

    #[test]
    fn seat_definition_defaults_ceilings() {
        let seat = parse_seat_definition("---\nname: s\nmodel: m\nkind: text\n---\nstance\n").unwrap();
        assert_eq!(seat.ceilings, Ceilings::default());
        assert!(seat.gate_classes.is_empty());
        assert!(seat.schedule.is_none());
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

    /// A seat file with one extra frontmatter line, the shape of every `schedule_gate:` test below.
    fn seat_with_gate_line(line: &str) -> String {
        format!("---\nname: g\nkind: text\n{line}\n---\nbody\n")
    }

    /// Sprint 28h S12 (the pairing): the engine reads an optional `schedule_gate: <tool> [<json object>]`
    /// and the studio must read the same key, or a seat the engine can hire is one the studio refuses
    /// to load (it refuses an unknown key). A bare tool name means empty arguments.
    #[test]
    fn a_schedule_gate_with_arguments_and_a_bare_one_both_load() {
        let with = parse_seat_definition(&seat_with_gate_line(
            r#"schedule_gate: get_diagnostics {"severity": "error", "maxResults": 5}"#,
        ))
        .expect("a gate with arguments loads");
        let gate = with.schedule_gate.expect("and is carried");
        assert_eq!(gate.tool, "get_diagnostics");
        assert_eq!(gate.arguments["severity"], "error");
        assert_eq!(gate.arguments["maxResults"], 5);

        let bare = parse_seat_definition(&seat_with_gate_line("schedule_gate: health_check")).unwrap();
        let gate = bare.schedule_gate.expect("a bare tool is a gate");
        assert_eq!(gate.tool, "health_check");
        assert!(gate.arguments.is_empty(), "a bare tool name means empty arguments");

        let empty_object = parse_seat_definition(&seat_with_gate_line("schedule_gate: health_check {}")).unwrap();
        assert!(empty_object.schedule_gate.unwrap().arguments.is_empty());
    }

    /// The engine strips the value and removes ONE matching quote pair before reading it, so a
    /// quoted value reads as the unquoted one.
    #[test]
    fn a_quoted_or_padded_schedule_gate_reads_as_the_engine_reads_it() {
        let quoted = parse_seat_definition(&seat_with_gate_line(r#"schedule_gate:   "health_check"  "#)).unwrap();
        assert_eq!(quoted.schedule_gate.unwrap().tool, "health_check");
        let single = parse_seat_definition(&seat_with_gate_line("schedule_gate: 'compile_workspace {\"summary\": true}'"))
            .unwrap();
        let gate = single.schedule_gate.unwrap();
        assert_eq!((gate.tool.as_str(), gate.arguments["summary"].as_bool()), ("compile_workspace", Some(true)));
    }

    /// Absent keeps the engine's default (the compile gate), and the studio carries no gate of its own.
    #[test]
    fn a_seat_without_a_schedule_gate_has_none() {
        assert!(parse_seat_definition(SEAT_FIXTURE).unwrap().schedule_gate.is_none());
    }

    /// A key that is present and cannot be read is REFUSED with its reason and is never read as
    /// absent: that would hand a seat that asked for a stronger check the weakest one in silence.
    #[test]
    fn a_schedule_gate_that_cannot_be_read_is_refused_with_its_reason() {
        let refused = |line: &str| parse_seat_definition(&seat_with_gate_line(line)).unwrap_err();

        let empty = refused("schedule_gate:");
        assert!(empty.starts_with("schedule_gate") && empty.contains("names no tool"), "{empty}");
        let blank_quotes = refused("schedule_gate: \"\"");
        assert!(blank_quotes.contains("names no tool"), "{blank_quotes}");

        for bad in ["Compile_Workspace", "9lives", "compile-workspace", "get.diagnostics"] {
            let err = refused(&format!("schedule_gate: {bad}"));
            assert!(
                err.starts_with("schedule_gate") && err.contains("is not a tool name") && err.contains(bad),
                "{bad}: {err}"
            );
        }

        let not_json = refused("schedule_gate: get_diagnostics {severity: error}");
        assert!(not_json.starts_with("schedule_gate") && not_json.contains("not valid JSON"), "{not_json}");

        let trailing = refused(r#"schedule_gate: get_diagnostics {"a": 1} and then some"#);
        assert!(trailing.contains("not valid JSON"), "text after the JSON is refused: {trailing}");
        let two_objects = refused(r#"schedule_gate: get_diagnostics {"a": 1} {"b": 2}"#);
        assert!(two_objects.contains("not valid JSON"), "{two_objects}");

        for not_object in ["[1, 2]", "5", "\"text\"", "null", "true"] {
            let err = refused(&format!("schedule_gate: get_diagnostics {not_object}"));
            assert!(
                err.starts_with("schedule_gate") && err.contains("must be a JSON object"),
                "{not_object}: {err}"
            );
        }
    }

    /// A seat that carries the key still passes every other check of the parser, and the file the
    /// seats view reads loads with no error and shows the seat.
    #[test]
    fn a_seat_with_a_schedule_gate_still_passes_the_other_frontmatter_checks() {
        let text = "---\nname: watcher\nkind: code\ntools: [health_check]\ngates: [docs]\n\
                    schedule: \"*/30 * * * *\"\nschedule_gate: health_check\nenabled: true\n---\nbody\n";
        let seat = parse_seat_definition(text).expect("loads");
        assert_eq!(seat.schedule.as_deref(), Some("*/30 * * * *"));
        assert_eq!(seat.gate_classes, vec![GateClass::Docs]);
        assert_eq!(seat.schedule_gate.unwrap().tool, "health_check");

        // The other checks still refuse beside it: the key does not loosen the parser.
        let still_unknown = seat_with_gate_line("schedule_gate: health_check\nttl_seconds: 5");
        assert!(parse_seat_definition(&still_unknown).unwrap_err().contains("unknown frontmatter key"));
        let dir = unique_tempdir("gate-load");
        fs::write(dir.join("w.md"), text).unwrap();
        let (seats, errors) = load_seat_definitions(&dir);
        assert!(errors.is_empty() && seats.len() == 1, "{errors:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// No shipped seat declares a gate yet (a decision for the person, recorded in the plan).
    #[test]
    fn no_shipped_seat_declares_a_schedule_gate() {
        for (file, body) in crate::conductor::EMBEDDED_SEATS.iter() {
            let seat = parse_seat_definition(body).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(seat.schedule_gate.is_none(), "{file} declares a schedule_gate");
        }
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
}
