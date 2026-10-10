//! The four per-seat settings — C11's clause, *"four settings changeable per seat (on/off,
//! clients, brain, reset-to-shipped), each surviving a redeploy"*.
//!
//! # Why they do not live in the seat file, which is the whole design
//!
//! A seat's own `seats/<name>.md` is BOTH the thing this app overwrites when it ships new
//! content AND the thing [`crate::conductor::materialize_seats`] hashes to decide whether the
//! user has edited it. Write a setting into its frontmatter and the hash changes, the seat is
//! reported `shadowed`, and the user's seat stops receiving stance updates FOREVER — silently,
//! because a shadowed seat looks exactly like a deliberately customised one.
//!
//! So the settings live beside the seats in a dot-named file this app's materialization pass
//! never writes. Surviving a redeploy is then a PROPERTY OF WHERE THEY LIVE rather than a rule
//! somebody has to remember, which is the difference between a design and a convention.
//!
//! # Two rules taken from the experience store before this was written, not after
//!
//! **OVERRIDES ONLY — never a copy of what the seat declares.** Every field here is an
//! `Option` and an absent one means *ask the seat file*. The store's lesson: *"every list
//! outside the implementation is a copy, and a copy of a changing surface is wrong from the
//! first unmirrored change — with no moment at which it announces it"* (a product's README
//! said 85 tools while it published 40, for three sprints). A settings file that recorded a
//! seat's model even when the user had not changed it would be exactly that copy: ship a new
//! default and the sidecar would quietly keep the old one.
//!
//! **ONE RESOLUTION POINT, AND EVERY READER GOES THROUGH IT.** The store's other lesson: a fix
//! that changed the size an order was placed at left two later paths still sending the old
//! total, so the placement fix alone did not hold. A seat's model is read at several sites in
//! `runner.rs`; [`SeatSettings::apply`] is the one place an override is merged, and it is
//! applied at LOAD so no reader downstream can see an unresolved seat.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::runner::SeatDefinition;

/// Beside the seats, dot-named, never rendered — and, critically, NEVER written by
/// `materialize_seats`. That omission is what makes every setting survive a redeploy.
pub const SETTINGS_FILE: &str = ".settings.json";

/// What the user changed about ONE seat. Every field optional: absent means "the seat file's
/// own value", so nothing here duplicates a shipped default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeatOverride {
    /// Setting 1 — on/off. `None` and `Some(true)` both mean deployed; only `Some(false)`
    /// turns a seat off, so a settings file that merely mentions a seat does not change it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Setting 2 — which client dialects this seat is written for. `None` means every dialect
    /// this build knows, which is the shipped behaviour; an EMPTY list means none, and is
    /// distinct from `None` on purpose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clients: Option<Vec<String>>,
    /// Setting 3 — the brain. Merged into the seat's `model` by [`SeatSettings::apply`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl SeatOverride {
    /// Whether this override says anything at all. An override that says nothing is removed
    /// rather than stored, so the file never accumulates empty entries that read as settings.
    fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.clients.is_none() && self.model.is_none()
    }
}

/// Every seat's overrides, keyed by seat name. `BTreeMap` so the file is stable on disk and a
/// diff of it is readable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeatSettings {
    #[serde(default)]
    seats: BTreeMap<String, SeatOverride>,
}

impl SeatSettings {
    /// Read the settings beside `seats_dir`. An ABSENT file is the default state and answers
    /// empty settings; an UNREADABLE or corrupt one answers an error rather than empty,
    /// because silently treating a corrupt settings file as "no settings" would re-enable a
    /// seat the user turned off.
    pub fn load(seats_dir: &Path) -> Result<Self, String> {
        let path = seats_dir.join(SETTINGS_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if body.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&body)
            .map_err(|e| format!("{} is not readable settings: {e}", path.display()))
    }

    /// Write the settings beside `seats_dir`, pretty-printed because a human edits this file
    /// too. Entries that say nothing are dropped on the way out.
    pub fn save(&self, seats_dir: &Path) -> Result<(), String> {
        let mut trimmed = self.clone();
        trimmed.seats.retain(|_, o| !o.is_empty());
        fs::create_dir_all(seats_dir)
            .map_err(|e| format!("cannot create {}: {e}", seats_dir.display()))?;
        let path = seats_dir.join(SETTINGS_FILE);
        let body = serde_json::to_string_pretty(&trimmed)
            .map_err(|e| format!("cannot serialize settings: {e}"))?;
        fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Setting 1, read. Default TRUE: a seat nobody has an opinion about deploys.
    pub fn is_enabled(&self, seat_name: &str) -> bool {
        self.seats
            .get(seat_name)
            .and_then(|o| o.enabled)
            .unwrap_or(true)
    }

    /// Whether setting 3 is in force for this seat — the view marks the brain as the user's.
    pub fn has_model_override(&self, seat_name: &str) -> bool {
        self.seats.get(seat_name).is_some_and(|o| o.model.is_some())
    }

    /// Setting 2, read. `None` means "every dialect this build knows" — the caller keeps its
    /// own list rather than this module holding a copy of it, which is the no-copy rule.
    pub fn clients(&self, seat_name: &str) -> Option<&[String]> {
        self.seats
            .get(seat_name)
            .and_then(|o| o.clients.as_deref())
    }

    /// Settings 1 and 2 as ONE question — does this seat's command get written for this
    /// client. The deploy asks this and nothing else, so it never combines `is_enabled` and
    /// `clients` itself: two readers combining two settings is how an off seat ends up
    /// deployed to the one client whose list was consulted first.
    ///
    /// Off beats the client list. A seat that is off deploys nowhere, however its list reads.
    pub fn deploys_to(&self, seat_name: &str, client: &str) -> bool {
        if !self.is_enabled(seat_name) {
            return false;
        }
        match self.clients(seat_name) {
            None => true,
            Some(list) => list.iter().any(|c| c == client),
        }
    }

    /// THE ONE RESOLUTION POINT. Merges setting 3 into the seat's definition, in place, so a
    /// reader downstream cannot obtain an unresolved seat. Returns whether anything changed,
    /// which the deploy report uses to say a seat's brain is the user's rather than ours.
    pub fn apply(&self, seat: &mut SeatDefinition) -> bool {
        match self.seats.get(&seat.name).and_then(|o| o.model.as_ref()) {
            Some(model) if *model != seat.model => {
                seat.model = model.clone();
                true
            }
            _ => false,
        }
    }

    /// Set setting 1.
    pub fn set_enabled(&mut self, seat_name: &str, enabled: bool) {
        self.entry(seat_name).enabled = Some(enabled);
    }

    /// Set setting 2. An empty list is kept as an empty list — "no clients" is a real answer
    /// and must not collapse into "all clients", which is what `None` means.
    pub fn set_clients(&mut self, seat_name: &str, clients: Vec<String>) {
        self.entry(seat_name).clients = Some(clients);
    }

    /// Set setting 3.
    pub fn set_model(&mut self, seat_name: &str, model: &str) {
        self.entry(seat_name).model = Some(model.to_string());
    }

    /// SETTING 4 — reset to shipped, the settings half. Drops every override for this seat, so
    /// the seat file's own values govern again.
    ///
    /// It is deliberately NOT the whole of setting 4: a seat whose STANCE the user edited is
    /// `shadowed` and resetting that means letting the materialization pass refresh the file,
    /// which is [`reset_to_shipped`]'s job. Splitting them keeps this type ignorant of the
    /// filesystem layout it does not own.
    pub fn clear(&mut self, seat_name: &str) -> bool {
        self.seats.remove(seat_name).is_some()
    }

    /// Whether this seat has any override at all — what a view needs to mark a row as
    /// "customised" without reading the values.
    pub fn is_customised(&self, seat_name: &str) -> bool {
        self.seats.get(seat_name).is_some_and(|o| !o.is_empty())
    }

    fn entry(&mut self, seat_name: &str) -> &mut SeatOverride {
        self.seats.entry(seat_name.to_string()).or_default()
    }
}

/// SETTING 4, in full — discard the user's customisation of one seat and take this build's.
///
/// Two halves, because the user can have customised two different things: the OVERRIDES here,
/// and the seat file's own stance if they edited it. Dropping the overrides is this function's
/// first act; the second is removing the seat file so the next
/// [`crate::conductor::materialize_seats`] re-seeds it from the embedded content.
///
/// The seat file is BACKED UP before removal, not deleted. `materialize_seats` already takes
/// that care for a copy it cannot prove unedited, and a reset is the one case where we KNOW it
/// was edited — so destroying it without a copy would be the one path that loses work on
/// purpose.
pub fn reset_to_shipped(seats_dir: &Path, seat_name: &str) -> Result<(), String> {
    let mut settings = SeatSettings::load(seats_dir)?;
    settings.clear(seat_name);
    settings.save(seats_dir)?;

    let seat_file = seats_dir.join(format!("{seat_name}.md"));
    if seat_file.exists() {
        let backup = seats_dir.join(format!("{seat_name}.md.pre-reset"));
        fs::copy(&seat_file, &backup)
            .map_err(|e| format!("cannot back up {} before reset: {e}", seat_file.display()))?;
        fs::remove_file(&seat_file)
            .map_err(|e| format!("cannot remove {} for reset: {e}", seat_file.display()))?;
    }
    Ok(())
}


// ===========================================================================
// The seats view's model, and the one door the view changes settings through.
// ===========================================================================

/// One client a seat can be written for: its id (what the settings file stores) and the label a
/// person reads.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClientChoice {
    pub id: String,
    pub label: String,
}

/// One row of the seats view. Everything here is READ — from the seat's own file, from
/// `COMMAND_MAP`, from the settings sidecar — and nothing is carried as a copy, so a seat
/// whose model, tools or purpose change makes the row change with it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeatRow {
    pub name: String,
    /// The slash command a person types. `None` for a driven seat, which has none.
    pub command: Option<String>,
    pub purpose: String,
    pub fires: String,
    /// The cron, when the seat is scheduled. Shown beside `fires`, never instead of it.
    pub schedule: Option<String>,
    pub tools: Vec<String>,
    /// Gate classes beyond the always-on one. Empty is a real answer, not a missing one.
    pub gates: Vec<String>,
    /// The brain AS RESOLVED — the override already applied.
    pub model: String,
    pub model_overridden: bool,
    pub enabled: bool,
    /// `None` = every client this build writes seat commands for.
    pub clients: Option<Vec<String>>,
    pub customised: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SeatsView {
    pub seats: Vec<SeatRow>,
    /// The clients a seat can be written for — the choices the clients setting offers.
    pub clients: Vec<ClientChoice>,
    /// Malformed seat files and an unreadable settings file, named. Never swallowed.
    pub errors: Vec<String>,
}

fn gate_label(class: &crate::runner::GateClass) -> &'static str {
    use crate::runner::GateClass;
    match class {
        GateClass::Always => "always",
        GateClass::Behavior => "behavior",
        GateClass::Tests => "tests",
        GateClass::Docs => "docs",
    }
}

/// Build the view from the seats directory. `clients` is passed in because which clients get
/// seat commands is the deploy's knowledge, not this module's — holding a list here would be
/// the copy this file's header rules out.
pub fn seats_view(seats_dir: &Path, clients: Vec<ClientChoice>) -> SeatsView {
    let (seats, load_errors) = crate::runner::load_seat_definitions(seats_dir);
    let mut errors: Vec<String> = load_errors
        .iter()
        .map(|(path, error)| format!("{}: {error}", path.display()))
        .collect();
    // Read for the per-seat flags. An unreadable file was already reported by the loader above,
    // so it is not reported twice here; the rows simply show no overrides.
    let settings = SeatSettings::load(seats_dir).unwrap_or_default();
    let rows = seats
        .iter()
        .map(|seat| SeatRow {
            name: seat.name.clone(),
            command: crate::conductor::command_for(&seat.name).map(|(cmd, _)| cmd.to_string()),
            purpose: seat.purpose.clone(),
            fires: seat.fires.clone(),
            schedule: seat.schedule.clone(),
            tools: seat.tools.clone(),
            gates: seat.gate_classes.iter().map(|g| gate_label(g).to_string()).collect(),
            model: seat.model.clone(),
            model_overridden: settings.has_model_override(&seat.name),
            enabled: settings.is_enabled(&seat.name),
            clients: settings.clients(&seat.name).map(<[String]>::to_vec),
            customised: settings.is_customised(&seat.name),
        })
        .collect();
    errors.sort();
    SeatsView { seats: rows, clients, errors }
}

/// What the view asks for. One variant per setting, so a request cannot name two at once.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "setting", rename_all = "camelCase")]
pub enum SeatChange {
    Enabled { value: bool },
    Clients { value: Vec<String> },
    Model { value: String },
    /// Setting 4 — drop the overrides and take this build's seat.
    Reset,
}

/// The one door settings change through. Validates before it writes: a seat name that matches
/// no seat file is REFUSED rather than stored, because a typo would otherwise create an entry
/// that governs nothing and reads as a setting.
pub fn change_seat(
    seats_dir: &Path,
    valid_clients: &[String],
    seat_name: &str,
    change: SeatChange,
) -> Result<(), String> {
    let (seats, _) = crate::runner::load_seat_definitions(seats_dir);
    if !seats.iter().any(|s| s.name == seat_name) {
        return Err(format!("no seat named '{seat_name}'"));
    }
    match change {
        SeatChange::Reset => {
            reset_to_shipped(seats_dir, seat_name)?;
            // `reset_to_shipped` removes the seat file so the materialization pass can re-seed
            // it. Run that pass now: otherwise the seat is absent from the view, and from the
            // next deploy's input, until something else happens to run it.
            crate::conductor::materialize_seats(seats_dir).map(|_| ())
        }
        SeatChange::Enabled { value } => {
            let mut settings = SeatSettings::load(seats_dir)?;
            settings.set_enabled(seat_name, value);
            settings.save(seats_dir)
        }
        SeatChange::Clients { value } => {
            if let Some(unknown) = value.iter().find(|c| !valid_clients.contains(c)) {
                return Err(format!(
                    "'{unknown}' is not a client seat commands are written for (known: {})",
                    valid_clients.join(", ")
                ));
            }
            let mut settings = SeatSettings::load(seats_dir)?;
            settings.set_clients(seat_name, value);
            settings.save(seats_dir)
        }
        SeatChange::Model { value } => {
            let value = value.trim();
            if value.is_empty() {
                return Err("a brain cannot be empty — use reset to return to the shipped one".into());
            }
            let mut settings = SeatSettings::load(seats_dir)?;
            settings.set_model(seat_name, value);
            settings.save(seats_dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jw-seat-settings-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn a_seat(name: &str, model: &str) -> SeatDefinition {
        let body = format!(
            "---\nname: {name}\nmodel: {model}\nkind: text\ntools: []\ngates: []\n\
             ttl_secs: 60\nmax_iterations: 1\ncost_budget_usd: 1.0\nmax_tool_steps: 10\n---\n\
             A stance.\n"
        );
        crate::runner::parse_seat_definition(&body).unwrap()
    }

    #[test]
    fn an_absent_file_is_the_default_and_not_an_error() {
        let dir = tempdir("absent");
        let settings = SeatSettings::load(&dir).unwrap();
        assert_eq!(settings, SeatSettings::default());
        // and the defaults are the SHIPPED behaviour, not "off"
        assert!(settings.is_enabled("architect"));
        assert_eq!(settings.clients("architect"), None);
        assert!(!settings.is_customised("architect"));
    }

    #[test]
    fn a_corrupt_file_is_an_error_rather_than_empty_settings() {
        // Treating it as empty would silently RE-ENABLE a seat the user turned off, which is
        // the failure this branch exists to prevent.
        let dir = tempdir("corrupt");
        fs::write(dir.join(SETTINGS_FILE), "{ not json").unwrap();
        let err = SeatSettings::load(&dir).unwrap_err();
        assert!(err.contains("not readable settings"), "{err}");
    }

    #[test]
    fn all_four_settings_survive_a_redeploy() {
        // C11's clause, and the thing the whole design is for. A redeploy is
        // `materialize_seats`, which rewrites the seat FILES; the settings must be untouched
        // by it because it never writes their file.
        let dir = tempdir("survive");
        let seats = dir.join("seats");
        crate::conductor::materialize_seats(&seats).unwrap();

        let mut settings = SeatSettings::load(&seats).unwrap();
        settings.set_enabled("profiler", false);
        settings.set_clients("architect", vec!["claude".to_string()]);
        settings.set_model("cataloguer", "claude-haiku-4-5");
        settings.save(&seats).unwrap();

        // THE REDEPLOY.
        crate::conductor::materialize_seats(&seats).unwrap();

        let after = SeatSettings::load(&seats).unwrap();
        assert!(!after.is_enabled("profiler"), "setting 1 must survive");
        assert_eq!(
            after.clients("architect"),
            Some(["claude".to_string()].as_slice()),
            "setting 2 must survive"
        );
        let mut seat = a_seat("cataloguer", "claude-sonnet-5");
        assert!(after.apply(&mut seat), "setting 3 must survive and apply");
        assert_eq!(seat.model, "claude-haiku-4-5");
        // setting 4 is the inverse and is tested below; what survives here is that a reset
        // has something to clear.
        assert!(after.is_customised("cataloguer"));
    }

    #[test]
    fn an_override_is_only_an_override() {
        // The no-copy rule: a seat nobody set is untouched, and `apply` reports false rather
        // than rewriting the model with an identical value.
        let settings = SeatSettings::default();
        let mut seat = a_seat("architect", "claude-sonnet-5");
        assert!(!settings.apply(&mut seat));
        assert_eq!(seat.model, "claude-sonnet-5");

        let mut same = SeatSettings::default();
        same.set_model("architect", "claude-sonnet-5");
        let mut seat2 = a_seat("architect", "claude-sonnet-5");
        assert!(
            !same.apply(&mut seat2),
            "an override equal to the shipped value changed nothing, and must say so"
        );
    }

    #[test]
    fn no_clients_and_every_client_are_different_answers() {
        let mut settings = SeatSettings::default();
        settings.set_clients("report", vec![]);
        assert_eq!(settings.clients("report"), Some([].as_slice()));
        assert_ne!(settings.clients("report"), None);
    }

    #[test]
    fn a_settings_file_that_merely_mentions_a_seat_changes_nothing() {
        let dir = tempdir("mention");
        fs::write(
            dir.join(SETTINGS_FILE),
            r#"{"seats":{"architect":{}}}"#,
        )
        .unwrap();
        let settings = SeatSettings::load(&dir).unwrap();
        assert!(settings.is_enabled("architect"));
        assert!(!settings.is_customised("architect"));
    }

    #[test]
    fn an_empty_override_is_not_written_back() {
        let dir = tempdir("empty");
        let mut settings = SeatSettings::default();
        settings.set_enabled("architect", false);
        settings.clear("architect");
        settings.save(&dir).unwrap();
        let body = fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap();
        assert!(!body.contains("architect"), "{body}");
    }

    #[test]
    fn reset_drops_the_overrides_and_backs_the_stance_up_before_removing_it() {
        let dir = tempdir("reset");
        let seats = dir.join("seats");
        crate::conductor::materialize_seats(&seats).unwrap();

        let mut settings = SeatSettings::load(&seats).unwrap();
        settings.set_model("profiler", "claude-sonnet-5");
        settings.save(&seats).unwrap();
        let edited = seats.join("profiler.md");
        fs::write(&edited, "---\nname: profiler\n---\nmy own words\n").unwrap();

        reset_to_shipped(&seats, "profiler").unwrap();

        assert!(
            !SeatSettings::load(&seats).unwrap().is_customised("profiler"),
            "the overrides are gone"
        );
        assert!(!edited.exists(), "the edited stance is removed so a redeploy re-seeds it");
        assert_eq!(
            fs::read_to_string(seats.join("profiler.md.pre-reset")).unwrap(),
            "---\nname: profiler\n---\nmy own words\n",
            "and it is BACKED UP first — a reset is the one path that knowingly discards work"
        );

        // The redeploy that completes the reset.
        crate::conductor::materialize_seats(&seats).unwrap();
        assert!(edited.exists(), "the shipped stance is back");
        assert!(fs::read_to_string(&edited).unwrap().contains("profiler"));
    }

    #[test]
    fn reset_is_safe_on_a_seat_that_was_never_customised() {
        let dir = tempdir("reset-clean");
        let seats = dir.join("seats");
        crate::conductor::materialize_seats(&seats).unwrap();
        reset_to_shipped(&seats, "report").unwrap();
        crate::conductor::materialize_seats(&seats).unwrap();
        assert!(seats.join("report.md").exists());
    }

    #[test]
    fn deploys_to_answers_settings_one_and_two_together() {
        let mut settings = SeatSettings::default();
        // Control: a seat nobody has an opinion about goes to every client.
        assert!(settings.deploys_to("architect", "claude"));
        assert!(settings.deploys_to("architect", "cursor"));

        // A list narrows it; the client not on the list is excluded, the one on it is not.
        settings.set_clients("architect", vec!["cursor".into()]);
        assert!(settings.deploys_to("architect", "cursor"));
        assert!(!settings.deploys_to("architect", "claude"));

        // An EMPTY list is "no clients", distinct from no list at all.
        settings.set_clients("architect", vec![]);
        assert!(!settings.deploys_to("architect", "cursor"));

        // Off beats a list that would admit the client.
        settings.set_clients("architect", vec!["claude".into()]);
        settings.set_enabled("architect", false);
        assert!(!settings.deploys_to("architect", "claude"));

        // And one seat's settings never reach another's.
        assert!(settings.deploys_to("debugger", "claude"));
    }

    fn seeded(tag: &str) -> PathBuf {
        let dir = tempdir(tag).join("seats");
        crate::conductor::materialize_seats(&dir).unwrap();
        dir
    }

    fn both_clients() -> Vec<ClientChoice> {
        vec![
            ClientChoice { id: "claude".into(), label: "Claude Code".into() },
            ClientChoice { id: "cursor".into(), label: "Cursor".into() },
        ]
    }

    fn valid() -> Vec<String> {
        vec!["claude".into(), "cursor".into()]
    }

    #[test]
    fn the_view_has_one_row_per_shipped_seat_read_from_the_files() {
        let dir = seeded("view-rows");
        let view = seats_view(&dir, both_clients());
        assert!(view.errors.is_empty(), "{:?}", view.errors);
        assert_eq!(view.seats.len(), crate::conductor::EMBEDDED_SEATS.len());

        let architect = view.seats.iter().find(|s| s.name == "architect").unwrap();
        assert_eq!(architect.command.as_deref(), Some("refactor"));
        assert!(!architect.purpose.is_empty() && !architect.fires.is_empty());
        assert!(architect.schedule.is_some(), "the architect is scheduled daily");
        assert!(architect.enabled && architect.clients.is_none() && !architect.customised);

        // A driven seat has no command, and says so rather than showing a blank command.
        let auditor = view.seats.iter().find(|s| s.name == "spec-auditor").unwrap();
        assert_eq!(auditor.command, None);
        // Gates: a seat that declares none shows none (a real answer), one that does shows it.
        assert!(auditor.gates.is_empty());
        let docs = view.seats.iter().find(|s| s.name == "javadoc-writer").unwrap();
        assert!(docs.gates.contains(&"docs".to_string()), "{:?}", docs.gates);
    }

    #[test]
    fn a_change_shows_in_the_view_and_a_reset_takes_it_back() {
        let dir = seeded("view-change");
        let shipped = seats_view(&dir, both_clients())
            .seats.into_iter().find(|s| s.name == "debugger").unwrap();

        change_seat(&dir, &valid(), "debugger", SeatChange::Enabled { value: false }).unwrap();
        change_seat(&dir, &valid(), "debugger", SeatChange::Clients { value: vec!["cursor".into()] }).unwrap();
        change_seat(&dir, &valid(), "debugger", SeatChange::Model { value: "a-brain".into() }).unwrap();

        let changed = seats_view(&dir, both_clients())
            .seats.into_iter().find(|s| s.name == "debugger").unwrap();
        assert!(!changed.enabled);
        assert_eq!(changed.clients, Some(vec!["cursor".to_string()]));
        assert_eq!(changed.model, "a-brain");
        assert!(changed.model_overridden && changed.customised);

        // Reset: back to the shipped row, the seat is still THERE (the file was re-seeded),
        // and no other seat was touched.
        change_seat(&dir, &valid(), "debugger", SeatChange::Reset).unwrap();
        let reset = seats_view(&dir, both_clients());
        let after = reset.seats.iter().find(|s| s.name == "debugger").expect("seat survives a reset");
        assert_eq!(*after, shipped);
        assert_eq!(reset.seats.len(), crate::conductor::EMBEDDED_SEATS.len());
    }

    #[test]
    fn a_reset_restores_a_stance_the_user_edited() {
        let dir = seeded("view-reset-stance");
        let file = dir.join("debugger.md");
        let shipped = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("{shipped}\nMY OWN EDIT\n")).unwrap();

        change_seat(&dir, &valid(), "debugger", SeatChange::Reset).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), shipped, "the shipped stance is back");
        assert!(
            fs::read_to_string(dir.join("debugger.md.pre-reset")).unwrap().contains("MY OWN EDIT"),
            "and the edit was backed up rather than destroyed"
        );
    }

    #[test]
    fn an_unknown_seat_client_or_empty_brain_is_refused_and_writes_nothing() {
        let dir = seeded("view-refuse");
        let settings_file = dir.join(SETTINGS_FILE);
        assert!(!settings_file.exists(), "control: nothing written yet");

        let ghost = change_seat(&dir, &valid(), "nonesuch", SeatChange::Enabled { value: false });
        assert!(ghost.unwrap_err().contains("no seat named"));
        let bad_client = change_seat(&dir, &valid(), "debugger", SeatChange::Clients { value: vec!["emacs".into()] });
        assert!(bad_client.unwrap_err().contains("emacs"));
        let blank = change_seat(&dir, &valid(), "debugger", SeatChange::Model { value: "  ".into() });
        assert!(blank.is_err());

        assert!(!settings_file.exists(), "three refusals, and not one of them left a trace");
    }

    #[test]
    fn a_corrupt_settings_file_is_named_in_the_view() {
        let dir = seeded("view-corrupt");
        fs::write(dir.join(SETTINGS_FILE), "{ not json").unwrap();
        let view = seats_view(&dir, both_clients());
        assert!(view.errors.iter().any(|e| e.contains(SETTINGS_FILE)), "{:?}", view.errors);
        assert_eq!(view.seats.len(), crate::conductor::EMBEDDED_SEATS.len(), "the seats still list");
    }
}
