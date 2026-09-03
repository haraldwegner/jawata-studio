//! THE WORD REACHES THE RULE, THROUGH THE REAL BINARY.
//!
//! # What was broken
//!
//! `stop::judge` has had Rule B since Sprint 26 — *"do not stop when autonomy
//! is granted and nothing is armed"* — and production supplied
//! `Autonomy::Unknown` at the single call site in `pipeline.rs`. The rule is
//! gated on `Granted`, so the comparison was false on every stop the product
//! has ever made. `stop.rs` records the consequence in its own comment: the
//! rule *"never fired in 267 recorded stops, and the cause was detection"*.
//!
//! Twenty-odd unit tests construct `Autonomy::Granted` and all of them pass.
//! That is exactly why nothing ever went red: the rule, its bound and its
//! message were all covered, and the one thing missing was an input. The
//! signal — Harald typing `autocontinue` — arrives at `Role::UserPrompt`, which
//! parses the prompt and knows the session. Both ends existed and were never
//! joined.
//!
//! # Why this test spawns the binary
//!
//! Because a unit test cannot see the join. `autonomy::note_prompt` writes
//! through `home_dir()`, the stop role reads through `home_dir()`, and a test
//! of either half passes while the wire between them does not exist. That is
//! the shape this project has shipped repeatedly — a decision function green
//! while the pipeline never calls it. So: two invocations of the real
//! executable, in sequence, sharing one scratch `HOME`.

use std::process::{Command, Stdio};
use std::io::Write;

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn scratch_home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jawata-autocontinue-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    // The hook refuses to act unconfigured — `--explain` reports
    // `SILENT [not-configured]`, which on stdout is indistinguishable from a
    // gate that looked and decided to stay quiet. So the scratch HOME gets the
    // same config file the deploy writes. The URL points nowhere on purpose:
    // nothing in this test needs the store, and a gate that cannot reach it
    // must still rule on autonomy.
    let studio = d.join(".claude").join("jawata-studio");
    std::fs::create_dir_all(&studio).unwrap();
    // BESIDE THE EXECUTABLE — `config_path_for` is `exe.parent().join(...)`,
    // not a HOME lookup. The state the gate writes still goes under HOME.
    std::fs::write(
        d.join("hook_config.json"),
        serde_json::json!({
            "client": "claude-code",
            "field_dir": studio.join("field").to_string_lossy(),
            "token": "test-token",
            "url": "http://127.0.0.1:9/mcp",
        })
        .to_string(),
    )
    .unwrap();
    d
}

/// A transcript whose last assistant turn is PROSE with no tool calls — the
/// exact shape of "answered his question and stopped", which is the behaviour
/// Rule B exists to push past.
fn transcript(dir: &std::path::Path, user: &str, assistant: &str) -> std::path::PathBuf {
    let p = dir.join("transcript.jsonl");
    let body = format!(
        "{}\n{}\n",
        serde_json::json!({"type":"user","message":{"role":"user","content":user}}),
        serde_json::json!({"type":"assistant","message":{"role":"assistant",
            "content":[{"type":"text","text":assistant}]}})
    );
    std::fs::write(&p, body).unwrap();
    p
}

/// The binary picks its role from **argv[0]**, not from an argument — the deploy
/// writes one link per role (`jawata-hook-stop`, `jawata-hook-userprompt`). A
/// test that passes the role as an arg reaches no role at all and gets empty
/// output, which reads exactly like a gate that decided to stay silent.
fn linked(home: &std::path::Path, role: &str) -> std::path::PathBuf {
    let link = home.join(format!("jawata-hook-{role}"));
    if !link.exists() {
        // A COPY, not a symlink. The role comes from argv[0], but the CONFIG is
        // looked up beside `current_exe()` — and on Linux that resolves
        // /proc/self/exe through the symlink to the real target/debug binary,
        // so a linked role would read the build directory's config (there is
        // none) and go silent as `not-configured`. On stdout that is
        // indistinguishable from a gate that looked and chose to say nothing,
        // which is how this test spent three rounds failing for the wrong
        // reason.
        std::fs::copy(HOOK, &link).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&link, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    link
}

fn run(role: &str, home: &std::path::Path, payload: &str) -> String {
    let exe = linked(home, role);
    let child = Command::new(&exe)
        .env("HOME", home)
        .env("JAWATA_HOOK_CLIENT", "claude-code")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    // ETXTBSY: Linux refuses to exec a file that was just written while any
    // descriptor on it may still be open. Copy-then-exec hits it under parallel
    // test threads, and it is a property of the platform rather than of this
    // gate — so it is retried rather than "fixed", and named so a future reader
    // does not go looking for a bug in the hook.
    let mut child = child;
    for _ in 0..20 {
        match child {
            Err(ref e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
                child = Command::new(&exe)
                    .env("HOME", home)
                    .env("JAWATA_HOOK_CLIENT", "claude-code")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
            }
            _ => break,
        }
    }
    let mut child = child.expect("the hook binary runs");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stop_payload(session: &str, transcript: &std::path::Path) -> String {
    serde_json::json!({
        "session_id": session,
        "transcript_path": transcript.to_string_lossy(),
        "stop_hook_active": false,
    })
    .to_string()
}

fn prompt_payload(session: &str, prompt: &str) -> String {
    serde_json::json!({"session_id": session, "prompt": prompt}).to_string()
}

/// THE test. Without the word, a finished prose turn is allowed to end — which
/// is correct, and is also what happened on every single stop for three weeks.
/// With the word, the same turn is refused.
///
/// The pair is asserted together because either half alone passes for the wrong
/// reason: a gate that blocks everything satisfies the second, and today's
/// gate — which cannot see autonomy at all — satisfies the first.
#[test]
fn his_word_turns_rule_b_on_and_its_absence_leaves_it_off() {
    let home = scratch_home("pair");
    // "carry on", not "Status?" — the original fixture opened the window with
    // a question, which the 2026-08-27 ruling (studio#33) exempts from the
    // push: a question-window's answer is never bounced into new work. This
    // test's subject is the autonomy WIRE, so its window must be a working
    // word, or it would be testing the exemption instead.
    let t = transcript(
        &home,
        "carry on",
        "Suite green, 2057 passing. That's the checkpoint done.",
    );

    let without = run("stop", &home, &stop_payload("sess-a", &t));
    assert!(
        !without.contains("RULE B"),
        "with no grant the turn must be free to end; got: {without}"
    );

    run("userprompt", &home, &prompt_payload("sess-a", "work the plan and autocontinue"));

    let with = run("stop", &home, &stop_payload("sess-a", &t));
    assert!(
        with.contains("RULE B"),
        "his word did not reach the rule — this is the wire that has never \
         existed, and the whole point of the test. got: {with}"
    );
    assert!(
        with.contains("\"block\""),
        "Rule B must BLOCK, not merely mention itself: {with}"
    );
}

/// EVERY VERDICT LEAVES A RECORD — and it took an outside auditor to say so.
///
/// This gate decides at every turn boundary and, until 2026-08-30, wrote nothing
/// about that decision: not the rule, not the autonomy state, not the counter.
/// Six failures in this one mechanism were therefore each reconstructed from the
/// session transcript by hand. The 22:54 sleep needed a fresh auditor and a
/// byte-offset dig to reach a single `if`, and the counter's value at that instant
/// could only be DEDUCED from source, because nothing had recorded it.
///
/// The audit's verdict on adding this was "non-optional: without it a sixth fix
/// cannot be verified any better than the last five".
///
/// Driven through the REAL BINARY on purpose. A unit test would assert that
/// `emit_signal` was called; this asserts that a line arrives in the file a human
/// reads afterwards — which is the whole claim, and the half this project has
/// shipped broken before.
#[test]
fn every_stop_verdict_is_recorded() {
    let home = scratch_home("verdict");
    let log = home.join(".claude").join("jawata-studio").join("outcomes.log");

    // A stop with NO grant: Rule B cannot fire, so this is the ALLOW path — the
    // one that previously left no trace at all, and the one a silent gate and a
    // working gate are indistinguishable on.
    let t = transcript(&home, "have a look", "Here is what I found.");
    run("stop", &home, &stop_payload("sess-v", &t));

    let after_allow = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        after_allow.contains("stop-verdict"),
        "an ALLOW must be recorded too — a gate that logs only its blocks cannot \
         answer 'why did it not fire', which is every question asked this week: {after_allow}"
    );
    assert!(
        after_allow.contains("autonomy=") && after_allow.contains("empty="),
        "the record carries the FACTS the rule decided on, not just its name — \
         the counter is exactly what had to be deduced from source last time: {after_allow}"
    );

    // And a BLOCK names the rule it came from.
    run("userprompt", &home, &prompt_payload("sess-v", "work the plan and autocontinue"));
    let idle = transcript(&home, "carry on", "Next I will look at the extractor.");
    run("stop", &home, &stop_payload("sess-v", &idle));

    let after_block = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        after_block.contains("block RULE B"),
        "a push must name its rule in the record, or the log cannot tell which of \
         them fired: {after_block}"
    );
}

/// studio#33 STILL HOLDS, AND NOW FOR A REASON THE GATE CAN OBSERVE.
///
/// The promise is unchanged and is his, from 2026-08-27: *"there is a loop that
/// bounces you back to work all the time — even if I just ask a question"*. A
/// turn answering him must not be pushed into new work.
///
/// WHAT CHANGED IS WHY. The old mechanism read his text at the END of the turn
/// and stood Rule B down if it looked like a question. That correlate failed in
/// both directions within three days: it matched DISCUSS inside "We had a
/// discussion" and slept a night, and it went stale for the whole window, so a
/// question at 22:43 silenced the push at 22:54 when he had long gone.
///
/// Now his message ends the grant when it ARRIVES, and Rule B simply finds no
/// grant. Nothing reads what he wrote. The test therefore no longer asserts
/// "the grant survives his question" — it asserts the opposite, because that
/// was the defect: a grant that survived his presence had to be suppressed by
/// guesswork later.
#[test]
fn his_question_ends_the_grant_so_its_answer_is_never_pushed() {
    let home = scratch_home("survives");
    run("userprompt", &home, &prompt_payload("sess-b", "autocontinue"));

    // His question, through the channel it really arrives on.
    run("userprompt", &home, &prompt_payload("sess-b", "why did you stop"));

    let answering = transcript(&home, "why did you stop", "Because I finished that piece.");
    let out = run("stop", &home, &stop_payload("sess-b", &answering));
    assert!(
        !out.contains("RULE B"),
        "answering him must not be pushed into new work — and the reason must \
         be that the grant is gone, not that his words were parsed: {out}"
    );

    // AND IT STAYS OFF until he says the word. "carry on" is him at the
    // keyboard, so it is his arrival, not a resumption — under the old rule
    // this line resumed the loop, which is how a conversation kept being
    // treated as an absence.
    let working = transcript(&home, "carry on", "Stage done.");
    run("userprompt", &home, &prompt_payload("sess-b", "carry on"));
    let still_off = run("stop", &home, &stop_payload("sess-b", &working));
    assert!(
        !still_off.contains("RULE B"),
        "only his word re-arms it; a work-order typed while he sits there is \
         still him sitting there: {still_off}"
    );

    // His word, and the loop is back — the control that proves the two
    // assertions above are the grant being OFF rather than the push being
    // broken outright.
    run("userprompt", &home, &prompt_payload("sess-b", "carry on and autocontinue"));
    let resumed = run("stop", &home, &stop_payload("sess-b", &working));
    assert!(
        resumed.contains("RULE B"),
        "his word must re-arm the push, or this fix has disabled the feature \
         instead of scoping it: {resumed}"
    );
}

/// The wedge, and the shape of the bound. Two consecutive turns that produce
/// NOTHING is a stuck session, and the gate lets go. A ceiling on BLOCKS would
/// have released a working session just as fast.
#[test]
fn two_empty_turns_release_the_session_rather_than_wedging_it() {
    let home = scratch_home("wedge");
    let t = transcript(&home, "carry on", "I have nothing further here.");
    run("userprompt", &home, &prompt_payload("sess-c", "autocontinue"));

    let first = run("stop", &home, &stop_payload("sess-c", &t));
    assert!(first.contains("RULE B"), "first push: {first}");
    let second = run("stop", &home, &stop_payload("sess-c", &t));
    assert!(second.contains("RULE B"), "second push: {second}");
    let third = run("stop", &home, &stop_payload("sess-c", &t));
    assert!(
        !third.contains("RULE B"),
        "a session that produced nothing twice must be RELEASED, not held — an \
         unbounded hold is the Cursor incident, counter at 11 and still \
         climbing. got: {third}"
    );
}

/// HIS ANSWER IS NEEDED — the grant ends by itself, with nothing typed.
///
/// A typed revoke is off exactly when he is least able to throw it: while he is
/// asleep, which is the entire scenario this exists for. So the grant ends on
/// the same fact that makes the agent unable to proceed.
#[test]
fn a_message_that_needs_his_answer_reaches_the_judge_and_the_verdict_ends_it() {
    let home = scratch_home("needs-him");
    let working = transcript(&home, "carry on", "Suite green, moving on.");
    run("userprompt", &home, &prompt_payload("sess-e", "autocontinue"));
    assert!(
        run("stop", &home, &stop_payload("sess-e", &working)).contains("RULE B"),
        "precondition: the grant is live"
    );

    // The agent now asks him something only he can settle. v4.0.0: SAYING SO
    // IS NOT ENOUGH ANY MORE. Until today the ask stood the push down by
    // itself; two stops on 2026-09-03 were exactly this shape, correctly
    // worded, and neither carried a decision — a release the plan schedules
    // five stages later, and a refused review the agent had already fixed.
    let asking = transcript(
        &home,
        "carry on",
        "Both are ready. Do you want v3.13.0 released tonight, or held for M5?",
    );
    let at_the_ask = run("stop", &home, &stop_payload("sess-e", &asking));
    assert!(
        at_the_ask.contains("RULE B") && at_the_ask.contains("autocontinue"),
        "an ask must now reach the judge rather than end the turn on its own wording: \
         {at_the_ask}"
    );

    // ...and the judge's RESERVED verdict is what ends it. THROUGH THE REAL
    // BINARY, which is this file's whole reason to exist: the verdict is read
    // out of the harness's own tool-result record, so the join between the
    // subagent's answer and the rule is exercised rather than assumed.
    // A DIFFERENT FILE. `transcript()` always writes `transcript.jsonl`, so
    // building this one there would overwrite the working fixture the last
    // assertion re-reads — and that assertion would then be judging this
    // transcript while claiming to judge that one.
    let judged = home.join("judged.jsonl");
    std::fs::write(
        &judged,
        format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::json!({"type":"user","message":{"role":"user","content":"carry on"}}),
            serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[
                {"type":"tool_use","name":"Agent","input":{"subagent_type":"autocontinue"}}]}}),
            serde_json::json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"t1",
                 "content":"The plan reserves the release.\nVERDICT: RESERVED"}]}}),
            serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[
                {"type":"text","text":"Do you want v3.13.0 released tonight, or held?"}]}}),
        ),
    )
    .unwrap();
    let settled = run("stop", &home, &stop_payload("sess-e", &judged));
    assert!(
        !settled.contains("RULE B"),
        "a RESERVED verdict must end the turn: {settled}"
    );

    // THE GRANT SURVIVED THE ASK, and that is already proven above rather than
    // here: the ask's own stop came back with RULE B, which only fires under a
    // live grant. A further stop cannot add to it — by this point the
    // empty-turn ceiling has been spent by the two blocks, so the gate
    // correctly lets go, and asserting a block there would be asserting that
    // the wedge valve does not work.
}

/// HIS ESC STOPS THE WORK. Always, and over the top of any grant.
///
/// The grant covers his ABSENCE. An interrupt is the loudest possible evidence
/// that he is present — a gate that answered it by refusing the stop would be
/// arguing with the one control he has that is not a sentence, while he sits
/// there pressing the key.
#[test]
fn his_interrupt_beats_the_grant() {
    let home = scratch_home("esc");
    run("userprompt", &home, &prompt_payload("sess-f", "autocontinue"));
    let ordinary = transcript(&home, "carry on", "Stage done.");
    assert!(
        run("stop", &home, &stop_payload("sess-f", &ordinary)).contains("RULE B"),
        "precondition: the grant is live and pushing"
    );

    let stopped = transcript(
        &home,
        "[Request interrupted by user]",
        "Stage done.",
    );
    let out = run("stop", &home, &stop_payload("sess-f", &stopped));
    assert!(
        !out.contains("RULE B"),
        "his Esc was overridden by the grant — this is the one thing the gate \
         must never do: {out}"
    );

    let mid_tool = transcript(
        &home,
        "[Request interrupted by user for tool use]",
        "Running the suite.",
    );
    let out2 = run("stop", &home, &stop_payload("sess-f", &mid_tool));
    assert!(
        !out2.contains("RULE B"),
        "stopping a tool mid-flight is the same key and the same answer: {out2}"
    );
}

/// HIS ESC ENDS THE GRANT, NOT JUST THE TURN (studio#33, 2026-08-27).
///
/// Before this, an interrupt released only the interrupted turn: the grant
/// survived, the very next stop pushed again, and he had to interrupt the same
/// session over and over — measured live, in his words "you started to work
/// afterwards". The Esc is the loudest proof of presence there is; a present
/// human re-grants with one word when he wants the loop back.
#[test]
fn his_esc_ends_the_grant_not_just_the_turn() {
    let home = scratch_home("esc-ends-grant");
    run("userprompt", &home, &prompt_payload("sess-g", "autocontinue"));
    let ordinary = transcript(&home, "carry on", "Stage done.");
    assert!(
        run("stop", &home, &stop_payload("sess-g", &ordinary)).contains("RULE B"),
        "precondition: the grant is live and pushing"
    );

    let stopped = transcript(&home, "[Request interrupted by user]", "Stage done.");
    assert!(
        !run("stop", &home, &stop_payload("sess-g", &stopped)).contains("RULE B"),
        "the interrupted turn itself must be released"
    );

    let after = run("stop", &home, &stop_payload("sess-g", &ordinary));
    assert!(
        !after.contains("RULE B"),
        "the grant survived his Esc and pushed the next turn — the exact loop \
         he had to keep interrupting: {after}"
    );
}
