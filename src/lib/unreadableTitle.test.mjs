// Run with `npm test`. 28h z74: the workspace's "cannot be read" badge said a project's directory was missing or unreadable whatever the
// engine's reason was; in the dogfood the directory was there and the reason was that Maven could not resolve the dependencies.
import assert from "node:assert/strict";
import { test } from "node:test";
import { unreadableTitle } from "./unreadableTitle.ts";

const warning = "NOTHING IS LOADED. Every configured project of this workspace failed to load: Maven dependency resolution failed.";

test("the badge gives the engine's own reason", () => {
  assert.equal(unreadableTitle(warning), warning);
});

test("with no reason from the engine the badge states the consequence and guesses no cause", () => {
  const title = unreadableTitle(undefined);
  assert.match(title, /Refactorings are refused/);
  assert.doesNotMatch(title, /directory|missing/);
});
