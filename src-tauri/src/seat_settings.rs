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
//! # Three settings, not four, and why
//!
//! C11 asks for on/off, clients, brain and reset-to-shipped. The BRAIN was built, reviewed, and
//! removed in the same checkpoint: a seat's `model` has no production consumer in this app
//! (the hosted-runner half that reads it is documented in `lib.rs` as having no production
//! caller, the deployed commands deliberately render no `model:`, and the hire path in
//! jawata-mcp reads the seat FILE, never this sidecar). A control whose edit is only echoed
//! back in the same field is the defect the UX seat's census exists to refuse. It returns when
//! a hire reads a resolved seat; until then it is declared unmet, not shipped inert.

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
}

impl SeatOverride {
    /// Whether this override says anything at all. An override that says nothing is removed
    /// rather than stored, so the file never accumulates empty entries that read as settings.
    fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.clients.is_none()
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

    /// Set setting 1.
    pub fn set_enabled(&mut self, seat_name: &str, enabled: bool) {
        self.entry(seat_name).enabled = Some(enabled);
    }

    /// Set setting 2. An empty list is kept as an empty list — "no clients" is a real answer
    /// and must not collapse into "all clients", which is what `None` means.
    pub fn set_clients(&mut self, seat_name: &str, clients: Vec<String>) {
        self.entry(seat_name).clients = Some(clients);
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
    // A reset is the remedy for a bad customisation, and a corrupt settings file is one. Failing
    // on it would refuse the very fix the view offers for the condition it reports. Keep the
    // unreadable content beside the file, then start from nothing.
    let mut settings = match SeatSettings::load(seats_dir) {
        Ok(settings) => settings,
        Err(_) => {
            let corrupt = seats_dir.join(SETTINGS_FILE);
            fs::copy(&corrupt, seats_dir.join(format!("{SETTINGS_FILE}.pre-reset")))
                .map_err(|e| format!("cannot keep the unreadable {}: {e}", corrupt.display()))?;
            SeatSettings::default()
        }
    };
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
    pub purpose: String,
    pub fires: String,
    /// The cron, when the seat is scheduled. Shown beside `fires`, never instead of it.
    pub schedule: Option<String>,
    pub tools: Vec<String>,
    /// Gate classes beyond the always-on one. Empty is a real answer, not a missing one.
    pub gates: Vec<String>,
    /// The brain AS RESOLVED — the override already applied.
    pub model: String,
    pub enabled: bool,
    /// Whether this seat has a command a deploy writes. A driven seat has none, so turning it
    /// off would remove nothing; the view offers the toggle only where it does something.
    pub deployable: bool,
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

/// The shipped seat of this name, parsed from the content embedded in this build.
fn shipped_seat(name: &str) -> Option<SeatDefinition> {
    let file = format!("{name}.md");
    let (_, body) = crate::conductor::EMBEDDED_SEATS.iter().find(|(f, _)| *f == file)?;
    crate::runner::parse_seat_definition(body).ok()
}

fn or_shipped(value: &str, name: &str, field: fn(&SeatDefinition) -> &String) -> String {
    if !value.trim().is_empty() {
        return value.to_string();
    }
    shipped_seat(name).map(|s| field(&s).clone()).unwrap_or_default()
}

/// Whether the seat file on disk differs from the content shipped in this build.
fn stance_edited(seats_dir: &Path, name: &str) -> bool {
    let file = format!("{name}.md");
    let Some((_, shipped)) = crate::conductor::EMBEDDED_SEATS.iter().find(|(f, _)| *f == file) else {
        return false;
    };
    fs::read_to_string(seats_dir.join(&file)).map(|text| text != *shipped).unwrap_or(false)
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
    // Read for the per-seat flags. An unreadable file is NAMED in the view's errors and the rows
    // show no overrides; treating it as "no settings" silently would show a seat the user turned
    // off as on.
    let settings = match SeatSettings::load(seats_dir) {
        Ok(settings) => settings,
        Err(error) => {
            errors.push(error);
            SeatSettings::default()
        }
    };
    let rows = seats
        .iter()
        .map(|seat| SeatRow {
            name: seat.name.clone(),
            // A user-edited seat keeps its old file and may carry neither key. Fall back to the
            // SHIPPED seat's value rather than render a blank row.
            purpose: or_shipped(&seat.purpose, &seat.name, |s| &s.purpose),
            fires: or_shipped(&seat.fires, &seat.name, |s| &s.fires),
            schedule: seat.schedule.clone(),
            tools: seat.tools.clone(),
            gates: seat.gate_classes.iter().map(|g| gate_label(g).to_string()).collect(),
            model: seat.model.clone(),
            enabled: settings.is_enabled(&seat.name),
            deployable: crate::conductor::command_for(&seat.name).is_some(),
            clients: settings.clients(&seat.name).map(<[String]>::to_vec),
            // An override OR a hand-edited stance: Reset exists for both, and a row that only
            // counted overrides left the edited stance with no way to reach it.
            customised: settings.is_customised(&seat.name) || stance_edited(seats_dir, &seat.name),
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
    fn the_settings_survive_a_redeploy() {
        // C11's clause, and the thing the whole design is for. A redeploy is
        // `materialize_seats`, which rewrites the seat FILES; the settings must be untouched
        // by it because it never writes their file.
        let dir = tempdir("survive");
        let seats = dir.join("seats");
        crate::conductor::materialize_seats(&seats).unwrap();

        let mut settings = SeatSettings::load(&seats).unwrap();
        settings.set_enabled("profiler", false);
        settings.set_clients("architect", vec!["claude".to_string()]);
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
        // setting 4 is the inverse and is tested below; what survives here is that a reset
        // has something to clear.
        assert!(after.is_customised("profiler"));
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
        settings.set_enabled("profiler", false);
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
        assert!(!architect.purpose.is_empty() && !architect.fires.is_empty());
        assert!(architect.schedule.is_some(), "the architect is scheduled daily");
        assert!(architect.enabled && architect.clients.is_none() && !architect.customised);

        let auditor = view.seats.iter().find(|s| s.name == "spec-auditor").unwrap();
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

        let changed = seats_view(&dir, both_clients())
            .seats.into_iter().find(|s| s.name == "debugger").unwrap();
        assert!(!changed.enabled);
        assert_eq!(changed.clients, Some(vec!["cursor".to_string()]));
        assert!(changed.customised);

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
    fn an_unknown_seat_or_client_is_refused_and_writes_nothing() {
        let dir = seeded("view-refuse");
        let settings_file = dir.join(SETTINGS_FILE);
        assert!(!settings_file.exists(), "control: nothing written yet");

        let ghost = change_seat(&dir, &valid(), "nonesuch", SeatChange::Enabled { value: false });
        assert!(ghost.unwrap_err().contains("no seat named"));
        let bad_client = change_seat(&dir, &valid(), "debugger", SeatChange::Clients { value: vec!["emacs".into()] });
        assert!(bad_client.unwrap_err().contains("emacs"));

        assert!(!settings_file.exists(), "two refusals, and neither left a trace");
    }

    #[test]
    fn a_corrupt_settings_file_is_named_in_the_view() {
        let dir = seeded("view-corrupt");
        fs::write(dir.join(SETTINGS_FILE), "{ not json").unwrap();
        let view = seats_view(&dir, both_clients());
        assert!(view.errors.iter().any(|e| e.contains(SETTINGS_FILE)), "{:?}", view.errors);
        assert_eq!(view.seats.len(), crate::conductor::EMBEDDED_SEATS.len(), "the seats still list");
    }

    /// The seats view is a Rust model read by TypeScript, and no test takes that path: the
    /// suite calls `change_seat` with enum values and never through JSON. A renamed tag or a
    /// snake_case key would pass every other test and fail only in a running window. This pins
    /// the exact payloads `tauri.ts` sends and the exact keys `SeatsView.svelte` reads.
    #[test]
    fn the_wire_shape_is_what_the_front_end_sends_and_reads() {
        let sent = |json: &str| serde_json::from_str::<SeatChange>(json).unwrap();
        assert_eq!(sent(r#"{"setting":"enabled","value":false}"#), SeatChange::Enabled { value: false });
        assert_eq!(
            sent(r#"{"setting":"clients","value":["claude"]}"#),
            SeatChange::Clients { value: vec!["claude".into()] }
        );
        assert_eq!(sent(r#"{"setting":"reset"}"#), SeatChange::Reset);

        let dir = seeded("view-wire");
        let value = serde_json::to_value(seats_view(&dir, both_clients())).unwrap();
        assert!(value["errors"].is_array() && value["clients"][0]["id"].is_string());
        assert!(value["clients"][0]["label"].is_string());
        let row = &value["seats"][0];
        for key in [
            "name", "purpose", "fires", "schedule", "tools", "gates", "model", "enabled",
            "clients", "customised", "deployable",
        ] {
            assert!(row.get(key).is_some(), "the view reads `{key}` and the row does not carry it");
        }
    }

    #[test]
    fn a_stance_the_user_edited_counts_as_customised_so_reset_is_reachable_for_it() {
        // The Reset button is shown only for a customised row. Customisation used to mean
        // "has a settings override", which left the case reset was DESIGNED for - a hand-edited
        // stance - with no way to reach it.
        let dir = seeded("view-edited-stance");
        let file = dir.join("debugger.md");
        let shipped = fs::read_to_string(&file).unwrap();
        let untouched = seats_view(&dir, both_clients());
        assert!(
            !untouched.seats.iter().find(|s| s.name == "debugger").unwrap().customised,
            "control: a seat nobody touched is not customised"
        );

        fs::write(&file, format!("{shipped}\nMY OWN EDIT\n")).unwrap();
        let edited = seats_view(&dir, both_clients());
        assert!(edited.seats.iter().find(|s| s.name == "debugger").unwrap().customised);
    }

    #[test]
    fn reset_recovers_from_a_corrupt_settings_file_instead_of_failing_on_it() {
        // A view that reports a corrupt settings file must also be able to FIX it. Reset used to
        // load the file first and fail on it, so the one remedy the view offers was refused by
        // exactly the condition it was shown for.
        let dir = seeded("view-reset-corrupt");
        fs::write(dir.join(SETTINGS_FILE), "{ not json").unwrap();

        change_seat(&dir, &valid(), "debugger", SeatChange::Reset).unwrap();

        assert!(SeatSettings::load(&dir).is_ok(), "the settings file is readable again");
        assert!(
            fs::read_to_string(dir.join(format!("{SETTINGS_FILE}.pre-reset"))).unwrap().contains("not json"),
            "and the corrupt content was kept, not destroyed"
        );
    }

    #[test]
    fn a_seat_file_without_the_new_keys_still_shows_what_the_seat_is_for() {
        // A user-edited (shadowed) seat keeps its old file and loads with empty purpose/fires.
        // The row falls back to the SHIPPED seat's values rather than rendering blank.
        let dir = seeded("view-fallback");
        let file = dir.join("debugger.md");
        let stripped: String = fs::read_to_string(&file)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("purpose:") && !l.starts_with("fires:"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file, stripped + "\n").unwrap();

        let row = seats_view(&dir, both_clients())
            .seats.into_iter().find(|s| s.name == "debugger").unwrap();
        assert!(!row.purpose.is_empty() && !row.fires.is_empty(), "{row:?}");
    }

    #[test]
    fn only_a_seat_with_a_command_can_be_turned_off() {
        // A driven seat writes no command file, so "off" would remove nothing. The row says so
        // and the view offers the toggle only where it does something.
        let dir = seeded("view-deployable");
        let view = seats_view(&dir, both_clients());
        assert!(view.seats.iter().find(|s| s.name == "architect").unwrap().deployable);
        assert!(!view.seats.iter().find(|s| s.name == "spec-auditor").unwrap().deployable);
    }
}
