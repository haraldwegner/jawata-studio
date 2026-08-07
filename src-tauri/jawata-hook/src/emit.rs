//! Encode for the client. Two dialects, and they differ in more than wording.
//!
//! Everything is built with `serde_json` rather than `format!`. The scripts
//! used `printf` with a hand-escaped format string, so a recalled line
//! containing a quote or a newline produced malformed JSON — and a client
//! reading malformed JSON from a hook either ignores it silently or, on
//! Cursor's `failClosed` guard, blocks the user's command. Neither failure
//! names itself.

use crate::roles::{Client, Role};

/// What the client should be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Emission {
    /// Put this text into the model's context.
    Context { event: &'static str, body: String },
    /// A permission decision from the guard.
    Permission { allowed: bool, reason: String },
    /// The stop gate's answer. A third dialect: Claude's Stop hook takes
    /// `{"decision":"block","reason":…}` and feeds `reason` back to the model,
    /// which is why the reason must say what to DO, not merely what is wrong.
    StopDecision { reason: String },
    /// Deliberately nothing — and the caller knows why.
    Silent,
}

/// Render an emission as the bytes this client reads on stdout.
///
/// [`Emission::Silent`] renders to `None`, never to an empty object: a client
/// handed `{}` may treat it as a decision it was not given.
pub fn render(client: Client, emission: &Emission) -> Option<String> {
    match emission {
        Emission::Silent => None,
        Emission::Context { event, body } => Some(match client {
            Client::ClaudeCode => serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": event,
                    "additionalContext": body,
                }
            })
            .to_string(),
            // Cursor's own shape: no hookSpecificOutput wrapper, no event echo.
            Client::Cursor => serde_json::json!({ "additional_context": body }).to_string(),
        }),
        Emission::StopDecision { reason } => match client {
            Client::ClaudeCode => Some(
                serde_json::json!({ "decision": "block", "reason": reason }).to_string(),
            ),
            // Cursor has no Stop event at all (roles::spec reports it Absent),
            // so there is nothing to say rather than something to say quietly.
            Client::Cursor => None,
        },
        Emission::Permission { allowed, reason } => Some(match client {
            Client::ClaudeCode => serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": if *allowed { "allow" } else { "deny" },
                    "permissionDecisionReason": reason,
                }
            })
            .to_string(),
            Client::Cursor => serde_json::json!({
                "permission": if *allowed { "allow" } else { "deny" },
                "agent_message": reason,
            })
            .to_string(),
        }),
    }
}

/// The context an injecting role produces, or [`Emission::Silent`] when the
/// role cannot inject on this client.
///
/// Cursor's `beforeSubmitPrompt` CANNOT inject — the query still runs and is
/// still recorded, but there is nothing to hand back. Expressing that here
/// rather than at each call site is why the role table carries `can_inject`.
pub fn context_for(role: Role, client: Client, body: String) -> Emission {
    let Some(spec) = crate::roles::spec(role, client) else {
        return Emission::Silent;
    };
    if !spec.can_inject || !spec.concerns.emit {
        return Emission::Silent;
    }
    let event = match spec.availability {
        crate::roles::Availability::Handled { event } => event,
        crate::roles::Availability::Absent { .. } => return Emission::Silent,
    };
    Emission::Context { event, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("every emission must be valid JSON")
    }

    #[test]
    fn claude_context_carries_the_event_name_it_was_fired_for() {
        let e = context_for(Role::UserPrompt, Client::ClaudeCode, "a line".into());
        let v = parse(&render(Client::ClaudeCode, &e).unwrap());
        assert_eq!("UserPromptSubmit", v["hookSpecificOutput"]["hookEventName"]);
        assert_eq!("a line", v["hookSpecificOutput"]["additionalContext"]);
    }

    #[test]
    fn cursor_cannot_inject_on_the_prompt_hook_so_it_emits_nothing() {
        // Not a bug and not a silent skip — a property of the client, carried
        // in the role table and honoured here.
        let e = context_for(Role::UserPrompt, Client::Cursor, "a line".into());
        assert_eq!(Emission::Silent, e);
        assert_eq!(None, render(Client::Cursor, &e));
    }

    #[test]
    fn a_role_absent_on_a_client_emits_nothing() {
        let e = context_for(Role::Stop, Client::Cursor, "x".into());
        assert_eq!(Emission::Silent, e);
    }

    #[test]
    fn silence_renders_to_nothing_at_all_not_to_an_empty_object() {
        // `{}` is a decision the client was not given.
        assert_eq!(None, render(Client::ClaudeCode, &Emission::Silent));
        assert_eq!(None, render(Client::Cursor, &Emission::Silent));
    }

    #[test]
    fn the_guard_speaks_each_clients_own_permission_dialect() {
        let deny = Emission::Permission { allowed: false, reason: "use JAWATA".into() };

        let claude = parse(&render(Client::ClaudeCode, &deny).unwrap());
        assert_eq!("deny", claude["hookSpecificOutput"]["permissionDecision"]);
        assert_eq!("use JAWATA", claude["hookSpecificOutput"]["permissionDecisionReason"]);

        let cursor = parse(&render(Client::Cursor, &deny).unwrap());
        assert_eq!("deny", cursor["permission"]);
        assert_eq!("use JAWATA", cursor["agent_message"]);
        assert!(
            cursor.get("hookSpecificOutput").is_none(),
            "Cursor's shape is flat — wrapping it is the dialect mistake"
        );
    }

    #[test]
    fn hostile_text_cannot_break_the_json() {
        // THE reason this is serde_json and not printf: the scripts
        // hand-escaped a format string, so one quote in a recalled line
        // produced malformed JSON — which a client ignores silently, or, under
        // Cursor's failClosed guard, turns into a blocked command.
        let nasty = "he said \"no\"\nline two\ttab \\ backslash \u{1f600} and }{ braces";
        for client in [Client::ClaudeCode, Client::Cursor] {
            let e = Emission::Context { event: "SessionStart", body: nasty.into() };
            let rendered = render(client, &e).unwrap();
            let v = parse(&rendered);   // panics if malformed
            let back = match client {
                Client::ClaudeCode => v["hookSpecificOutput"]["additionalContext"].as_str(),
                Client::Cursor => v["additional_context"].as_str(),
            };
            assert_eq!(Some(nasty), back, "the text must survive the round trip intact");
            assert!(!rendered.contains('\n'), "the emission must be ONE line for the client");
        }
    }

    #[test]
    fn a_permission_reason_is_escaped_too() {
        let e = Emission::Permission {
            allowed: false,
            reason: "blocked: grep \"foo\" over *.java\nuse search_symbols".into(),
        };
        for client in [Client::ClaudeCode, Client::Cursor] {
            let rendered = render(client, &e).unwrap();
            parse(&rendered);
            assert!(!rendered.contains('\n'));
        }
    }

    #[test]
    fn every_injecting_role_renders_valid_json_for_its_client() {
        for spec in crate::roles::ROLES {
            let e = context_for(spec.role, spec.client, "body".into());
            match render(spec.client, &e) {
                Some(s) => {
                    parse(&s);
                    assert!(spec.can_inject, "{:?}/{:?} emitted but cannot inject", spec.role, spec.client);
                }
                None => assert!(
                    !spec.can_inject || !spec.concerns.emit,
                    "{:?}/{:?} can inject and emits, but produced nothing",
                    spec.role,
                    spec.client
                ),
            }
        }
    }
}
