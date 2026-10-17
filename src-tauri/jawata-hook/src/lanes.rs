//! Which lanes of the store each hook occasion asks — ONE table.
//!
//! Sprint 28g S22. The lane is chosen BEFORE the search, and the engine refuses a call that names
//! none, so every request this crate makes carries a lane list from here. The occasion decides the
//! lanes; nothing guesses a lane from a message's wording, and nothing here asks the code lane on a
//! typed message.
//!
//! | Occasion | Lanes | Why |
//! |---|---|---|
//! | a typed message | `experience` | what was learned; small enough to ask every time |
//! | a tool call naming a `Type#member` | `experience`, `code` | the lessons about it, and what it is FOR |
//! | the recall gate | `experience`, `domain` | records anchored at exactly that member; a job row there is a description and would gate every call |
//!
//! The code lane answers *"do we already have something that does this?"* and is asked by job. It is
//! never used to read code — the compiler tools do that — so a typed message does not ask it.
//! Opening a package and the duplicate check ask it inside the engine, through their own verbs.

use serde_json::{json, Value};

/// A message the user typed: the lessons, and nothing else.
pub const TYPED_MESSAGE: &[&str] = &["experience"];

/// A tool call that names a member: the lessons about it and what it is for.
pub const TOOL_CALL: &[&str] = &["experience", "code"];

/// The recall gate: records anchored at exactly the member, lessons and contracts.
pub const RECALL_GATE: &[&str] = &["experience", "domain"];

/// The `lanes` argument of a request, as the engine reads it.
pub fn argument(lanes: &[&str]) -> Value {
    json!(lanes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (i) A typed message asks no code lane — not on its own and not beside another.
    #[test]
    fn a_typed_message_never_asks_the_code_lane() {
        assert!(!TYPED_MESSAGE.contains(&"code"));
        assert_eq!(vec!["experience"], TYPED_MESSAGE);
    }

    #[test]
    fn every_occasion_names_at_least_one_lane_the_engine_knows() {
        let known = ["experience", "patterns", "domain", "code", "rules"];
        for occasion in [TYPED_MESSAGE, TOOL_CALL, RECALL_GATE] {
            assert!(!occasion.is_empty(), "an empty list is a call the engine refuses");
            for lane in occasion {
                assert!(known.contains(lane), "{lane} is not a lane the engine has");
            }
        }
    }

    #[test]
    fn the_argument_is_a_json_array_of_names() {
        assert_eq!(json!(["experience", "code"]), argument(TOOL_CALL));
    }
}
