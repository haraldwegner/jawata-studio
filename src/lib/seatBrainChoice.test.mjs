// Run with `npm test`. Which listed model a seat's saved model stands for, so the Brain list picks it instead of adding a line that says the
// choice is not available (his words, 2026-11-17: it "shouldn't show not available on this machine, but pick a model").
import assert from "node:assert/strict";
import test from "node:test";
import { listedModel, selectedValue } from "./seatBrainChoice.ts";

const CLAUDE = ["Opus 5.5", "Fable 5.2", "Sonnet 5.5", "Haiku 5.5", "Fable 5.1", "Haiku 4.5", "Sonnet 5", "Opus 5", "Sonnet 4.6"];

test("a saved name the brain lists stands for itself, whatever its letter case", () => {
  assert.equal(listedModel(CLAUDE, "Sonnet 5.5"), "Sonnet 5.5");
  assert.equal(listedModel(CLAUDE, "  sonnet 5 "), "Sonnet 5");
});

test("the short alias a brain's own command line takes stands for the first model of that family in the list", () => {
  assert.equal(listedModel(CLAUDE, "sonnet"), "Sonnet 5.5", "the list is the brain's, newest first: the first Sonnet is the one to pick");
  assert.equal(listedModel(CLAUDE, "haiku"), "Haiku 5.5");
  assert.equal(listedModel(CLAUDE, "opus"), "Opus 5.5");
});

test("a name nothing in the list stands for, and an empty list, match nothing", () => {
  assert.equal(listedModel(CLAUDE, "claude-sonnet-5"), null);
  assert.equal(listedModel(CLAUDE, "gpt-9"), null);
  assert.equal(listedModel([], "sonnet"), null);
  assert.equal(listedModel(undefined, "sonnet"), null);
  assert.equal(listedModel(CLAUDE, null), null);
  assert.equal(listedModel(CLAUDE, ""), null);
});

test("the alias is a whole word: 'son' does not pick Sonnet", () => {
  assert.equal(listedModel(CLAUDE, "son"), null);
});

test("the select holds the listed model for a saved alias, and the brain's own default when nothing in the list matches", () => {
  assert.equal(selectedValue("claude-code", "sonnet", CLAUDE), "claude-code|Sonnet 5.5");
  assert.equal(selectedValue("claude-code", "Sonnet 5.5", CLAUDE), "claude-code|Sonnet 5.5");
  assert.equal(selectedValue("claude-code", "claude-sonnet-5", CLAUDE), "claude-code|");
  assert.equal(selectedValue("claude-code", null, CLAUDE), "claude-code|");
  assert.equal(selectedValue("cursor", "grok-4.5", undefined), "cursor|grok-4.5", "a brain whose list is not known yet keeps the saved value, as it is");
});
