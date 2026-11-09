//! The workflow as the studio edits it (Sprint 28h, S18a, D51): who hires whom, what each seat is,
//! and where a task a seat could not do goes next.
//!
//! The ENGINE owns the rules. Its `SeatTree` lays three keys of the seat files, `parent:`, `role:` and
//! `escalates_to:`, over the signed tree and refuses the whole result when a rule is broken. This module
//! is the studio's half: it keeps the person's placement as a seat setting (like the brain), writes
//! the three lines into the deployed seat file (`seat_settings::seat_file_content`), and refuses a
//! placement BEFORE it is stored, with the engine's own reasons, so the person is never shown a
//! change the engine would then throw away.
//!
//! [`SIGNED`] and [`refusals`] are therefore a MIRROR of `SeatTree.standard()` and of its load-time
//! rules, held to the engine's texts. The engine checks again when it loads, and says what it
//! refused in a hire's answer (`seatTreeRefused`), so a drift between the two shows up there and
//! never as a silently different tree. One rule is only half mirrored: the engine knows which of its
//! tools change code, the studio sees the seat file's `changes_code` and the brain's own editors.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::seat_settings::{SeatSettings, Workflow};

/// The signed tree: seat, the seat that hires it, what it is, where its failed task goes.
/// Mirrors `SeatTree.standardDeclarations()` in jawata-mcp.
pub const SIGNED: &[(&str, &str, &str, Option<&str>)] = &[
    ("architect", "main", "reviewer", None),
    ("ux", "main", "reviewer", None),
    ("spec-editor", "main", "reviewer", None),
    ("spec-auditor", "main", "reviewer", None),
    ("consultant", "main", "reviewer", None),
    ("pm", "main", "lead", None),
    ("chief-engineer", "pm", "lead", None),
    ("test-manager", "pm", "lead", None),
    ("debugger", "pm", "reviewer", None),
    ("profiler", "pm", "reviewer", None),
    ("developer", "chief-engineer", "worker", Some("senior-developer")),
    ("senior-developer", "chief-engineer", "worker", None),
    ("javadoc-writer", "chief-engineer", "worker", None),
    ("cataloguer", "chief-engineer", "reviewer", None),
    ("test-writer", "test-manager", "worker", None),
];

/// The only seat nobody hires.
pub const TOP: &str = "main";

/// The tools of the brain itself that change code; only a worker may hold one.
const BRAIN_EDITORS: &[&str] = &["Edit", "Write", "MultiEdit"];

#[derive(Debug, Clone, PartialEq)]
struct Node {
    parent: String,
    role: String,
    escalates_to: Option<String>,
}

fn signed_tree() -> BTreeMap<String, Node> {
    SIGNED
        .iter()
        .map(|(seat, parent, role, escalates)| {
            (
                seat.to_string(),
                Node {
                    parent: parent.to_string(),
                    role: role.to_string(),
                    escalates_to: escalates.map(str::to_string),
                },
            )
        })
        .collect()
}

/// The tree as the engine would load it from the deployed seat files: the signed tree, then each
/// seat's placement (the person's setting, or the three keys a hand-edited file carries).
fn effective_tree(seats_dir: &Path, settings: &SeatSettings) -> (BTreeMap<String, Node>, Vec<String>) {
    let mut tree = signed_tree();
    let mut refusals = Vec::new();
    let (seats, _) = crate::seat_file::load_seat_definitions(seats_dir);
    for seat in &seats {
        let placed = settings.workflow(&seat.name).cloned().or_else(|| {
            (seat.parent.is_some() || seat.role.is_some() || seat.escalates_to.is_some()).then(|| {
                let before = tree.get(&seat.name);
                Workflow {
                    parent: seat.parent.clone().or_else(|| before.map(|n| n.parent.clone())).unwrap_or_default(),
                    role: seat.role.clone().or_else(|| before.map(|n| n.role.clone())).unwrap_or_default(),
                    escalates_to: seat
                        .escalates_to
                        .clone()
                        .map(|e| e.trim().to_string())
                        .filter(|e| !e.is_empty())
                        .or_else(|| if seat.escalates_to.is_some() { None } else { before.and_then(|n| n.escalates_to.clone()) }),
                }
            })
        });
        let Some(placed) = placed else { continue };
        if seat.name == TOP {
            refusals.push(format!(
                "{TOP}: main is the top of the tree and nobody hires it, so its file may declare no parent, role or escalates_to"
            ));
            continue;
        }
        if placed.parent.is_empty() {
            refusals.push(format!("{}: it has no parent, and main is the only seat with no parent", seat.name));
            continue;
        }
        if placed.role.is_empty() {
            refusals.push(format!(
                "{}: it declares a parent but no role (lead, worker or reviewer), and the default tree does not know it",
                seat.name
            ));
            continue;
        }
        tree.insert(
            seat.name.clone(),
            Node { parent: placed.parent, role: placed.role, escalates_to: placed.escalates_to },
        );
    }
    (tree, refusals)
}

fn reaches_top(seat: &str, tree: &BTreeMap<String, Node>) -> bool {
    let mut at = seat.to_string();
    for _ in 0..=tree.len() {
        match tree.get(&at) {
            None => return at == TOP,
            Some(node) => at = node.parent.clone(),
        }
    }
    false
}

/// Every rule the engine's `SeatTree` would break with the placements as they stand, each line
/// naming the seat first and in the engine's words. Empty when the engine would accept the tree.
pub fn refusals(seats_dir: &Path, settings: &SeatSettings) -> Vec<String> {
    let (tree, mut refusals) = effective_tree(seats_dir, settings);
    for (seat, node) in &tree {
        if !["lead", "worker", "reviewer"].contains(&node.role.as_str()) {
            refusals.push(format!("{seat}: role is '{}', which is none of lead, worker or reviewer", node.role));
        }
        if node.parent != TOP && !tree.contains_key(&node.parent) {
            refusals.push(format!("{seat}: its parent '{}' is no seat", node.parent));
        } else if !reaches_top(seat, &tree) {
            refusals.push(format!("{seat}: its chain of parent never reaches {TOP} (a cycle)"));
        }
        if let Some(target) = &node.escalates_to {
            if target == seat {
                refusals.push(format!("{seat}: escalates_to names itself"));
            } else if target != TOP && !tree.contains_key(target) {
                refusals.push(format!("{seat}: its escalates_to '{target}' is no seat"));
            }
        }
    }
    let (seats, _) = crate::seat_file::load_seat_definitions(seats_dir);
    for seat in &seats {
        let Some(node) = tree.get(&seat.name) else { continue };
        if node.role == "worker" {
            continue;
        }
        if seat.changes_code {
            refusals.push(format!(
                "{}: it is a {} and its file sets changes_code, but only a worker may change code",
                seat.name, node.role
            ));
        }
        for tool in &seat.tools {
            if BRAIN_EDITORS.contains(&tool.as_str()) {
                refusals.push(format!(
                    "{}: it is a {} and its file lists '{tool}', a tool that changes code, but only a worker may hold one",
                    seat.name, node.role
                ));
            }
        }
    }
    refusals.sort();
    refusals.dedup();
    refusals
}

/// One seat of the editor: where it sits now, and where the signed tree puts it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRow {
    pub name: String,
    /// What the person types for the seat (`/refactor` for the architect), when it has one.
    pub command: Option<String>,
    pub parent: String,
    pub role: String,
    pub escalates_to: Option<String>,
    /// The signed tree's own place for the seat; absent for a seat the signed tree does not know.
    pub signed: Option<SignedPlace>,
    /// Whether the person placed the seat, so the row can offer to take it back.
    pub placed: bool,
    /// Why the seat has no place in the tree, when it has none: the engine's signed tree does not know
    /// it and the person has not placed it. A seat shows its place or this, never two empty fields.
    pub no_place_reason: Option<String>,
    /// A seat of the default tree that has no seat file: it is shown so the tree is whole, and cannot be moved.
    pub no_file: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SignedPlace {
    pub parent: String,
    pub role: String,
    pub escalates_to: Option<String>,
}

/// What the Workflow view shows.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowView {
    /// The seats that have a seat file, which is what the person can place.
    pub rows: Vec<WorkflowRow>,
    /// Every name a seat may report to or escalate to: main and every seat of the tree.
    pub places: Vec<String>,
    /// What the engine would refuse of the tree as it stands; empty when it would accept it.
    pub refusals: Vec<String>,
    pub errors: Vec<String>,
}

/// The plain reason a seat has no place in the tree (Sprint 28h Stage 21 (h)2). The engine's
/// `SeatTree.standard()` places fifteen seats and leaves out `observer`, `report`, `review`,
/// and `rottenmeier`; the view used to show those with two empty fields, as though the
/// studio had failed to load a place. Each reason says what starts the seat instead of a hire.
pub fn no_place_reason(seat: &str, command: Option<&str>) -> String {
    match seat {
        "observer" => "Nothing hires it. The hook starts it beside the work, every few calls, and never waits for it.".to_string(),
        "report" => "Nothing hires it. You type /report yourself when you want a bug report drafted.".to_string(),
        "review" => "Nothing hires it. You type /review yourself, and it also runs on its own schedule.".to_string(),
        "rottenmeier" => "Nothing hires it. It starts when you are rude or interrupt, or when you type /rottenmeier.".to_string(),
        "btw" => "Nothing hires it. It answers the btw panel, the side discussion.".to_string(),
        _ => match command {
            Some(command) => format!(
                "The engine's tree does not know this seat, so nothing hires it. You type {command} yourself."
            ),
            None => "The engine's tree does not know this seat, so nothing hires it. Give it a parent and a role to put it in the tree.".to_string(),
        },
    }
}

pub fn workflow_view(seats_dir: &Path) -> WorkflowView {
    let mut errors = Vec::new();
    let settings = match SeatSettings::load(seats_dir) {
        Ok(settings) => settings,
        Err(error) => {
            errors.push(error);
            SeatSettings::default()
        }
    };
    let (tree, _) = effective_tree(seats_dir, &settings);
    let (seats, load_errors) = crate::seat_file::load_seat_definitions(seats_dir);
    errors.extend(load_errors.iter().map(|(path, error)| format!("{}: {error}", path.display())));
    let signed = signed_tree();
    let mut rows: Vec<WorkflowRow> = seats
        .iter()
        .filter(|seat| seat.name != TOP)
        .map(|seat| {
            let node = tree.get(&seat.name);
            let command = crate::conductor::command_for(&seat.name).map(|c| format!("/{c}"));
            WorkflowRow {
                name: seat.name.clone(),
                no_place_reason: node.is_none().then(|| no_place_reason(&seat.name, command.as_deref())),
                command,
                parent: node.map(|n| n.parent.clone()).unwrap_or_default(),
                role: node.map(|n| n.role.clone()).unwrap_or_default(),
                escalates_to: node.and_then(|n| n.escalates_to.clone()),
                signed: signed.get(&seat.name).map(|n| SignedPlace {
                    parent: n.parent.clone(),
                    role: n.role.clone(),
                    escalates_to: n.escalates_to.clone(),
                }),
                placed: settings.workflow(&seat.name).is_some(),
                no_file: false,
            }
        })
        .collect();
    for (name, node) in &signed {
        if !rows.iter().any(|row| &row.name == name) {
            rows.push(WorkflowRow {
                name: name.clone(),
                command: None,
                parent: node.parent.clone(),
                role: node.role.clone(),
                escalates_to: node.escalates_to.clone(),
                signed: Some(SignedPlace { parent: node.parent.clone(), role: node.role.clone(), escalates_to: node.escalates_to.clone() }),
                placed: false,
                no_place_reason: None,
                no_file: true,
            });
        }
    }
    rows.sort_by(|a, b| (a.parent.as_str(), a.name.as_str()).cmp(&(b.parent.as_str(), b.name.as_str())));
    let mut places: Vec<String> = std::iter::once(TOP.to_string()).chain(tree.keys().cloned()).collect();
    places.sort();
    places.dedup();
    WorkflowView { rows, places, refusals: refusals(seats_dir, &settings), errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seat_settings::{change_seat, SeatChange};
    use std::fs;
    use std::path::PathBuf;

    fn deployed(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jw-workflow-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let seats = dir.join("seats");
        crate::conductor::materialize_seats(&seats).unwrap();
        seats
    }

    fn place(seats: &Path, seat: &str, parent: &str, role: &str, escalates: Option<&str>) -> Result<(), String> {
        change_seat(
            seats,
            &[],
            seat,
            SeatChange::Workflow {
                parent: parent.into(),
                role: role.into(),
                escalates_to: escalates.map(str::to_string),
            },
        )
    }

    #[test]
    fn shipped_seats_that_the_signed_tree_knows_sit_where_the_signed_tree_puts_them() {
        let seats = deployed("signed");
        let view = workflow_view(&seats);
        assert!(view.refusals.is_empty(), "{:?}", view.refusals);
        let pm = view.rows.iter().find(|r| r.name == "pm").expect("the pm is a shipped seat");
        assert_eq!((pm.parent.as_str(), pm.role.as_str(), pm.placed), ("main", "lead", false));
        let cataloguer = view.rows.iter().find(|r| r.name == "cataloguer").unwrap();
        assert_eq!(cataloguer.parent, "chief-engineer");
        assert!(view.places.contains(&"developer".to_string()), "a seat of the signed tree without a file can still be a parent");
        assert!(view.rows.iter().all(|r| r.name != "main"), "main is the top and is not placed");
        assert!(SIGNED.iter().all(|(seat, ..)| view.rows.iter().any(|r| r.name == *seat)), "every seat of the default tree has a place on the chart");
        assert!(view.rows.iter().find(|r| r.name == "consultant").unwrap().no_file, "the consultant has no seat file and says so");
        assert!(view.rows.iter().all(|r| r.name != "worker"), "the seat named worker is gone");
    }

    #[test]
    fn a_placement_is_written_into_the_seat_file_and_taken_back_by_the_default() {
        let seats = deployed("written");
        place(&seats, "cataloguer", "pm", "reviewer", None).unwrap();

        let file = fs::read_to_string(seats.join("cataloguer.md")).unwrap();
        assert!(file.contains("\nparent: pm\nrole: reviewer\nescalates_to: \n"), "{file}");
        let row = workflow_view(&seats).rows.into_iter().find(|r| r.name == "cataloguer").unwrap();
        assert_eq!((row.parent.as_str(), row.placed), ("pm", true));
        assert_eq!(row.signed.unwrap().parent, "chief-engineer", "the row still says where the signed tree puts it");

        change_seat(&seats, &[], "cataloguer", SeatChange::WorkflowDefault).unwrap();
        let back = fs::read_to_string(seats.join("cataloguer.md")).unwrap();
        assert!(!back.contains("parent:") && !back.contains("escalates_to:"), "{back}");
        assert!(!workflow_view(&seats).rows.into_iter().find(|r| r.name == "cataloguer").unwrap().placed);
    }

    #[test]
    fn a_placement_the_engine_would_refuse_is_refused_before_it_is_stored() {
        let seats = deployed("refused");

        let unknown = place(&seats, "cataloguer", "nobody", "reviewer", None).unwrap_err();
        assert!(unknown.contains("cataloguer: its parent 'nobody' is no seat"), "{unknown}");
        let role = place(&seats, "cataloguer", "pm", "boss", None).unwrap_err();
        assert!(role.contains("'boss', which is none of lead, worker or reviewer"), "{role}");
        let itself = place(&seats, "cataloguer", "pm", "reviewer", Some("cataloguer")).unwrap_err();
        assert!(itself.contains("escalates_to names itself"), "{itself}");
        let ghost = place(&seats, "cataloguer", "pm", "reviewer", Some("ghost")).unwrap_err();
        assert!(ghost.contains("'ghost' is no seat"), "{ghost}");
        // nothing was stored by any of the refusals.
        assert!(SeatSettings::load(&seats).unwrap().workflow("cataloguer").is_none());
        // the control: a placement that breaks no rule is stored.
        place(&seats, "cataloguer", "pm", "reviewer", Some("developer")).unwrap();
        assert!(SeatSettings::load(&seats).unwrap().workflow("cataloguer").is_some());
    }

    /// The signed tree as one line per seat, in the form `SeatTreeFromSeatFilesTest` in jawata-mcp pins
    /// the engine's table to. If either side changes a row, its own test goes red and names the other.
    fn signed_as_the_engine_pins_it() -> String {
        let mut rows: Vec<String> = SIGNED
            .iter()
            .map(|(seat, parent, role, escalates)| format!("{seat}>{parent}:{role}:{}", escalates.unwrap_or("-")))
            .collect();
        rows.sort();
        rows.join(" ")
    }

    #[test]
    fn the_signed_table_is_the_one_the_engine_pins() {
        assert_eq!(
            signed_as_the_engine_pins_it(),
            "architect>main:reviewer:- cataloguer>chief-engineer:reviewer:- chief-engineer>pm:lead:- consultant>main:reviewer:- \
             debugger>pm:reviewer:- developer>chief-engineer:worker:senior-developer javadoc-writer>chief-engineer:worker:- pm>main:lead:- \
             profiler>pm:reviewer:- senior-developer>chief-engineer:worker:- spec-auditor>main:reviewer:- spec-editor>main:reviewer:- \
             test-manager>pm:lead:- test-writer>test-manager:worker:- ux>main:reviewer:-"
        );
    }

    #[test]
    fn a_seat_that_is_not_a_worker_and_changes_code_is_refused_with_the_reason() {
        let seats = deployed("code");
        // a seat file of a worker that changes code, copied from a shipped seat and renamed
        let shipped = fs::read_to_string(seats.join("cataloguer.md")).unwrap();
        let worker = shipped
            .replacen("name: cataloguer", "name: probe", 1)
            .replacen("\n---\n", "\nchanges_code: true\n---\n", 1);
        fs::write(seats.join("probe.md"), worker).unwrap();

        // as a reviewer it may not change code; the reason names the seat and the rule
        let refused = place(&seats, "probe", "pm", "reviewer", None).unwrap_err();
        assert!(refused.contains("probe: it is a reviewer and its file sets changes_code, but only a worker may change code"), "{refused}");
        // the control: as a worker it may
        place(&seats, "probe", "pm", "worker", None).unwrap();
        assert!(refusals(&seats, &SeatSettings::load(&seats).unwrap()).is_empty());
    }

    #[test]
    fn a_blank_parent_and_a_role_in_capitals_are_read_as_the_engine_reads_them() {
        let seats = deployed("blank");
        let shipped = fs::read_to_string(seats.join("cataloguer.md")).unwrap();
        // a hand-edited file: the engine refuses a blank parent, and accepts "Reviewer"
        fs::write(seats.join("cataloguer.md"), shipped.replacen("\n---\n", "\nparent: \nrole: Reviewer\n---\n", 1)).unwrap();
        let view = workflow_view(&seats);
        assert!(
            view.refusals.iter().any(|r| r.starts_with("cataloguer: it has no parent")),
            "{:?}",
            view.refusals
        );
        assert!(view.errors.is_empty(), "the seat stays in the view instead of vanishing with a load error: {:?}", view.errors);
    }

    #[test]
    fn a_role_in_capitals_is_the_role_the_engine_reads() {
        let seats = deployed("capitals");
        let shipped = fs::read_to_string(seats.join("cataloguer.md")).unwrap();
        // the engine lower-cases and strips a role, so this hand-edited file is a valid placement
        fs::write(seats.join("cataloguer.md"), shipped.replacen("\n---\n", "\nparent: pm\nrole:  Reviewer \n---\n", 1)).unwrap();
        let view = workflow_view(&seats);
        assert!(view.refusals.is_empty(), "{:?}", view.refusals);
        let row = view.rows.iter().find(|r| r.name == "cataloguer").unwrap();
        assert_eq!((row.parent.as_str(), row.role.as_str()), ("pm", "reviewer"));
        // the control: a word that is no role is refused in the engine's words, whatever its case
        fs::write(seats.join("cataloguer.md"), shipped.replacen("\n---\n", "\nparent: pm\nrole: BOSS\n---\n", 1)).unwrap();
        let refusals = workflow_view(&seats).refusals;
        assert!(refusals.iter().any(|r| r.contains("role is 'boss', which is none of lead, worker or reviewer")), "{refusals:?}");
    }

    #[test]
    fn taking_a_seat_back_removes_the_lines_from_a_file_the_person_edited() {
        let seats = deployed("edited");
        place(&seats, "cataloguer", "pm", "reviewer", None).unwrap();
        // the person edits the stance, so the deploy keeps the file as it is from now on
        let path = seats.join("cataloguer.md");
        fs::write(&path, format!("{}\nan edit of the stance\n", fs::read_to_string(&path).unwrap())).unwrap();

        change_seat(&seats, &[], "cataloguer", SeatChange::WorkflowDefault).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("an edit of the stance"), "the person's edit is kept");
        assert!(!after.contains("parent:") && !after.contains("escalates_to:"), "{after}");
    }

    #[test]
    fn a_cycle_is_refused_and_a_new_seat_needs_a_parent_and_a_role() {
        let seats = deployed("cycle");
        place(&seats, "pm", "chief-engineer", "lead", None).unwrap_err();
        let cycle = place(&seats, "pm", "chief-engineer", "lead", None).unwrap_err();
        assert!(cycle.contains("never reaches main (a cycle)"), "{cycle}");
        // a shipped seat the signed tree does not know joins it with a parent and a role.
        place(&seats, "review", "pm", "reviewer", None).unwrap();
        assert_eq!(workflow_view(&seats).rows.iter().find(|r| r.name == "review").unwrap().parent, "pm");
    }

    /// Stage 21 (h)2. The engine's `SeatTree.standard()` leaves out observer, report, review, rottenmeier and
    /// btw. Their rows showed two empty fields; now every row shows its place or says why it has none.
    #[test]
    fn every_seat_shows_its_place_or_the_reason_it_has_none() {
        let seats = deployed("noplace");
        let view = workflow_view(&seats);
        let named: Vec<&str> = view.rows.iter().map(|r| r.name.as_str()).collect();
        for left_out in ["observer", "report", "review", "rottenmeier", "btw"] {
            assert!(named.contains(&left_out), "control: {left_out} is a shipped seat with a row, got {named:?}");
            let row = view.rows.iter().find(|r| r.name == left_out).unwrap();
            assert!(row.parent.is_empty() && row.role.is_empty(), "{left_out} is outside the signed tree");
            let reason = row.no_place_reason.as_deref().unwrap_or_else(|| panic!("{left_out} has no reason"));
            assert!(reason.len() > 20 && reason.contains("hire"), "{left_out}: {reason}");
        }
        assert!(
            SIGNED.iter().all(|(seat, ..)| !["observer", "report", "review", "rottenmeier", "btw"].contains(seat)),
            "control: the signed table really leaves those five out (SeatTree.standard() does)"
        );
        // The invariant for every row of the shipped seats, so a sixth such seat cannot show two empty fields.
        for row in &view.rows {
            let has_place = !row.parent.is_empty() && !row.role.is_empty();
            assert!(
                has_place != row.no_place_reason.is_some(),
                "{}: it must show a place or a reason, exactly one of them ({:?}, {:?}, {:?})",
                row.name, row.parent, row.role, row.no_place_reason
            );
        }
        // A seat of the tree shows no reason, and a seat the person places loses it.
        assert!(view.rows.iter().find(|r| r.name == "pm").unwrap().no_place_reason.is_none());
        place(&seats, "review", "pm", "reviewer", None).unwrap();
        assert!(workflow_view(&seats).rows.iter().find(|r| r.name == "review").unwrap().no_place_reason.is_none());
    }
}
