//! D27 (Sprint 28h): the rollout order. A skill this studio writes names engine verbs (`hire`,
//! `plan_board(...)`); when the studio is newer than the engine that is RUNNING, a skill can name a
//! verb that engine does not offer, and the agent is told to call something that is refused. The
//! floor is held here, at the deploy: a skill naming a verb the running engine lacks is not
//! written, and the person is told which skill and which verb.
//!
//! What counts as a verb a skill names: an identifier in backticks written as a call (`name(`) or
//! on its own (`name`), that is one of the engine's tools as this studio knows them — the vendored
//! surface (`engine-surface.json`: refreshed by hand with `build/engine-surface.sh`, and CHECKED by
//! the release job against the tool list of the engine that release bundles, which fails the release
//! when the two differ — so a release cannot ship a surface other than its own engine's; the
//! `engineVersion` label inside it says when it was last refreshed, not that it is stale) or the
//! seat's own `tools:` list. Other backticked words (`kind`, `lane`, a parameter value) are not
//! verbs and are never judged: the floor refuses on a name it KNOWS is a tool, never on a guess.
//!
//! What "the running engine" offers is its own `tools/list`, asked of the managed servers that have
//! an endpoint and a token, the first three tried in order, the first that answers taken. When none
//! answers there is nothing to compare with and the deploy proceeds as it did before (a deploy with
//! no engine answering is not checked, and holds nothing back).

use std::collections::BTreeSet;

use crate::runner::SeatDefinition;

const ENGINE_SURFACE_JSON: &str = include_str!("../engine-surface.json");

/// The tools the studio knows the engine to offer: the live names of the vendored surface.
pub fn surface_verbs() -> BTreeSet<String> {
    let json: serde_json::Value = serde_json::from_str(ENGINE_SURFACE_JSON).unwrap_or_default();
    json["live"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The identifiers `text` writes in backticks as a call or alone (`name(` or `name`), of those
/// in `universe`.
pub fn verbs_named(text: &str, universe: &BTreeSet<String>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && (chars[end].is_ascii_lowercase() || chars[end].is_ascii_digit() || chars[end] == '_') {
            end += 1;
        }
        let closes = end < chars.len() && (chars[end] == '`' || chars[end] == '(');
        if end > start && chars[start].is_ascii_lowercase() && closes {
            let name: String = chars[start..end].iter().collect();
            if universe.contains(&name) {
                found.insert(name);
            }
        }
        // Resume after the opening tick only: a closing tick is the next opening one in a pair
        // of adjacent spans, and skipping it would lose the span after it.
        i = start;
    }
    found
}

/// The verbs `text` names that the running engine does not offer, in name order.
pub fn absent_verbs(text: &str, universe: &BTreeSet<String>, running: &BTreeSet<String>) -> Vec<String> {
    verbs_named(text, universe)
        .into_iter()
        .filter(|verb| !running.contains(verb))
        .collect()
}

/// A skill the floor holds back, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBack {
    /// The seat's name or the utility command's name.
    pub name: String,
    pub absent: Vec<String>,
}

/// The one sentence the deploy result and the studio log carry for a held skill.
pub fn describe(held: &[HeldBack]) -> String {
    held.iter()
        .map(|h| format!("{} (names {}, which the running engine does not offer)", h.name, h.absent.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The seats a deploy may write, and the ones the floor holds back. `running` is `None` when no
/// engine answered: nothing is held then, because nothing can be compared.
pub fn split_seats(
    seats: &[SeatDefinition],
    running: Option<&BTreeSet<String>>,
) -> (Vec<SeatDefinition>, Vec<HeldBack>) {
    let Some(running) = running else {
        return (seats.to_vec(), Vec::new());
    };
    let surface = surface_verbs();
    let mut deployable = Vec::new();
    let mut held = Vec::new();
    for seat in seats {
        let mut universe = surface.clone();
        universe.extend(seat.tools.iter().cloned());
        // The text the agent reads is the rendered skill, not the seat file: the generated
        // instrument list and gate sentences are part of it.
        let body = crate::conductor::render_claude_skill(seat).unwrap_or_default();
        let absent = absent_verbs(&body, &universe, running);
        if absent.is_empty() {
            deployable.push(seat.clone());
        } else {
            held.push(HeldBack { name: seat.name.clone(), absent });
        }
    }
    (deployable, held)
}

/// The utility commands (`/btw`, `/autocontinue`, ...) the floor holds back, by the same rule.
pub fn held_utilities(running: Option<&BTreeSet<String>>) -> Vec<HeldBack> {
    let Some(running) = running else {
        return Vec::new();
    };
    let universe = surface_verbs();
    crate::conductor::UTILITY_MAP
        .iter()
        .filter_map(|(command, description)| {
            let body = crate::conductor::render_claude_utility(command, description);
            let absent = absent_verbs(&body, &universe, running);
            (!absent.is_empty()).then(|| HeldBack { name: (*command).to_string(), absent })
        })
        .collect()
}

/// A verb that some of `seats` name and others do not: every seat names `compile_workspace` (the
/// generated gate rule), so a test that takes THAT one away holds every seat back and has no control
/// for the seats that stay.
#[cfg(test)]
pub(crate) fn a_verb_some_seats_name(seats: &[SeatDefinition]) -> String {
    let surface = surface_verbs();
    let bodies: Vec<String> = seats
        .iter()
        .map(|seat| crate::conductor::render_claude_skill(seat).unwrap_or_default())
        .collect();
    surface
        .iter()
        .find(|verb| {
            let n = bodies.iter().filter(|body| verbs_named(body, &surface).contains(*verb)).count();
            n >= 1 && n < bodies.len()
        })
        .cloned()
        .expect("some engine verb is named by some seats and not by others")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn a_backticked_call_or_name_in_the_universe_is_a_verb_and_anything_else_is_not() {
        let universe = set(&["hire", "plan_board", "compile_workspace"]);
        let text = "Call `hire(seat=...)` then read `plan_board`; run `compile_workspace` and set `kind` to `lane`. Not `hire_x` nor hire.";
        assert_eq!(
            verbs_named(text, &universe),
            set(&["hire", "plan_board", "compile_workspace"]),
            "kind and lane are parameter words, hire_x is another name, bare hire is prose"
        );
    }

    #[test]
    fn adjacent_spans_are_each_read() {
        let universe = set(&["a_tool", "b_tool"]);
        assert_eq!(verbs_named("`a_tool``b_tool`", &universe), set(&["a_tool", "b_tool"]));
    }

    #[test]
    fn the_verbs_the_running_engine_lacks_are_named_in_order() {
        let universe = set(&["hire", "plan_board", "compile_workspace"]);
        let running = set(&["compile_workspace"]);
        assert_eq!(
            absent_verbs("`plan_board` and `hire(` and `compile_workspace`", &universe, &running),
            vec!["hire".to_string(), "plan_board".to_string()]
        );
        assert!(absent_verbs("`compile_workspace`", &universe, &running).is_empty(), "control: all offered");
    }

    #[test]
    fn with_no_answer_from_an_engine_nothing_is_held() {
        let seat = crate::runner::load_seat_definitions(&seats_dir()).0;
        assert!(!seat.is_empty());
        let (deployable, held) = split_seats(&seat, None);
        assert_eq!(deployable.len(), seat.len());
        assert!(held.is_empty());
        assert!(held_utilities(None).is_empty());
    }

    #[test]
    fn an_engine_that_offers_everything_the_surface_names_holds_nothing_back() {
        let seats = crate::runner::load_seat_definitions(&seats_dir()).0;
        let running = surface_verbs();
        let (deployable, held) = split_seats(&seats, Some(&running));
        assert!(held.is_empty(), "held: {held:?}");
        assert_eq!(deployable, seats, "and the seats are handed on unchanged");
        assert!(held_utilities(Some(&running)).is_empty());
    }

    #[test]
    fn a_seat_that_names_a_verb_the_running_engine_lacks_is_held_and_the_others_are_not() {
        let seats = crate::runner::load_seat_definitions(&seats_dir()).0;
        let verb = a_verb_some_seats_name(&seats);
        let mut running = surface_verbs();
        assert!(running.remove(&verb), "control: the surface offers {verb}");
        let (deployable, held) = split_seats(&seats, Some(&running));
        assert!(!held.is_empty(), "some seat names {verb}");
        assert!(!deployable.is_empty(), "and some do not: they stay deployable");
        assert!(held.iter().all(|h| h.absent == vec![verb.clone()]), "held: {held:?}");
        assert_eq!(deployable.len() + held.len(), seats.len());
        for h in &held {
            assert!(deployable.iter().all(|s| s.name != h.name), "{} is not also deployable", h.name);
        }
        assert!(describe(&held).contains(&verb), "the sentence names the verb");
    }

    /// The seat's own `tools:` list is part of what the floor judges: a verb outside the vendored
    /// surface that the seat declares and the engine does not offer holds the seat back (and an
    /// engine that offers it does not).
    #[test]
    fn a_verb_only_the_seats_own_tools_list_names_is_judged_too() {
        let seats = crate::runner::load_seat_definitions(&seats_dir()).0;
        let mut seat = seats
            .iter()
            .find(|s| matches!(s.kind, crate::runner::SeatKind::Code))
            .cloned()
            .expect("a code seat is shipped");
        seat.tools.push("zz_new_verb".to_string());
        let surface = surface_verbs();
        assert!(!surface.contains("zz_new_verb"), "control: it is outside the vendored surface");
        let body = crate::conductor::render_claude_skill(&seat).unwrap();
        assert!(body.contains("`zz_new_verb`"), "control: the generated skill names it");

        let (deployable, held) = split_seats(&[seat.clone()], Some(&surface));
        assert!(deployable.is_empty(), "the seat is held back");
        assert_eq!(held[0].absent, vec!["zz_new_verb".to_string()]);

        let mut offering = surface.clone();
        offering.insert("zz_new_verb".to_string());
        assert!(split_seats(&[seat], Some(&offering)).1.is_empty(), "control: an engine that offers it holds nothing");
    }

    /// The utility floor decides here: the memorize utility names `experience(`, so an engine without
    /// `experience` holds exactly the utilities that name it, and an engine with everything holds none.
    #[test]
    fn a_utility_command_naming_a_verb_the_running_engine_lacks_is_held() {
        let mut running = surface_verbs();
        assert!(running.remove("experience"), "control: the surface offers it");
        let held = held_utilities(Some(&running));
        let memorize = held.iter().find(|h| h.name == "memorize").unwrap_or_else(|| panic!("memorize is held: {held:?}"));
        assert_eq!(memorize.absent, vec!["experience".to_string()]);
        assert!(
            held.len() < crate::conductor::UTILITY_MAP.len(),
            "control: a utility that names no missing verb is not held: {held:?}"
        );
        assert!(held_utilities(Some(&surface_verbs())).is_empty(), "control: an engine offering everything holds none");
    }

    fn seats_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("engine-floor-seats-{}", std::process::id())).join("seats");
        crate::conductor::materialize_seats(&dir).expect("materialize");
        dir
    }
}
