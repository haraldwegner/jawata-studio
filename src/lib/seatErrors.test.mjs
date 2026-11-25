// Run with `npm test`. 28h z75: with no engine running the Seats page said the same sentence twice in red, once from the page's own list
// and once from the brain check.
import assert from "node:assert/strict";
import { test } from "node:test";
import { brainCheckError } from "./seatErrors.ts";

const noEngine = "no jawata engine is running to ask; start the jawata studio's engine, then open this page again";

test("a brain check that fails with what the page already says adds no second line", () => {
  assert.equal(brainCheckError(noEngine, [noEngine]), "");
});

test("a brain check that fails for another reason says so, named as the brain check", () => {
  assert.equal(brainCheckError("the check timed out", [noEngine]), "brain check: the check timed out");
  assert.equal(brainCheckError(noEngine, []), `brain check: ${noEngine}`);
});
