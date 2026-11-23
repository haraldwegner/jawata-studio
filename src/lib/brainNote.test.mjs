// Run with `npm test`. The Seats page warns that a brain cannot be restricted from writing only beside a seat that must not change code,
// and offers Connect for every installed brain that is not known to be connected (his words, 2026-11-23).
import assert from "node:assert/strict";
import { test } from "node:test";
import { brainNote, canConnect } from "./seatBrainChoice.ts";

const cursor = { label: "Cursor-Grok", login: "loggedIn", writesHeldByInstruction: true };
const claude = { label: "Claude Code", login: "loggedIn", writesHeldByInstruction: false };

test("a seat that must not change code is warned about a brain that cannot be held back, and a seat that may is not", () => {
  assert.match(brainNote(cursor, false), /Cursor-Grok cannot be restricted from writing/);
  assert.equal(brainNote(cursor, true), "");
});

test("a brain whose writes a mechanism holds carries no warning on any seat", () => {
  assert.equal(brainNote(claude, false), "");
});

test("the login is named beside the brain, with the warning after it", () => {
  assert.equal(brainNote({ ...cursor, login: "notLoggedIn" }, false), " (not connected) (Cursor-Grok cannot be restricted from writing)");
});

test("Connect is offered for an installed brain that is not connected or cannot say, and for no other", () => {
  assert.equal(canConnect({ installed: true, login: "notLoggedIn", loginCommand: "agy acp --login" }), true);
  assert.equal(canConnect({ installed: true, login: "unknown", loginCommand: "copilot login" }), true, "a brain that cannot say is offered it too");
  assert.equal(canConnect({ installed: true, login: "loggedIn", loginCommand: "claude auth login" }), false);
  assert.equal(canConnect({ installed: false, login: "notLoggedIn", loginCommand: "agy acp --login" }), false);
  assert.equal(canConnect({ installed: true, login: "notLoggedIn", loginCommand: " " }), false);
});
