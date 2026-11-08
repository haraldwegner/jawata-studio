// Run with `npm test`. Sprint 28h Stage 21 (h)1: the Memory view must not say two things that contradict.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { memoryNotice, NO_WORKSPACE_SUMMARY, STARTING_HINT } from "./memoryNotice.ts";

test("with no workspace the view says so once and does not claim a workspace is starting", () => {
  const notice = memoryNotice([], true, "Reading the store…");
  assert.equal(notice.summary, NO_WORKSPACE_SUMMARY);
  assert.equal(notice.hint, null, "nothing is starting: nothing exists");
  assert.doesNotMatch(notice.summary, /not reachable/, "the old line also blamed services that do not exist");
});

test("before the first answer nothing is claimed about workspaces", () => {
  const notice = memoryNotice([], false, "Reading the store…");
  assert.equal(notice.summary, "Reading the store…");
  assert.equal(notice.hint, null);
});

test("a workspace whose service does not answer yet is the one case that says starting", () => {
  const notice = memoryNotice([{ reachable: true }, { reachable: false }], true, "Reading the store…");
  assert.equal(notice.summary, null, "the rows are shown");
  assert.equal(notice.hint, STARTING_HINT);
});

test("all workspaces reachable says nothing to wait for", () => {
  assert.deepEqual(memoryNotice([{ reachable: true }], true, "x"), { summary: null, hint: null });
});

test("the view reads both texts from memoryNotice and prints neither literally", () => {
  const source = readFileSync(new URL("./components/MemoryView.svelte", import.meta.url), "utf8");
  assert.ok(source.includes("memoryNotice("), "the view asks memoryNotice");
  assert.ok(!source.includes("workspace starting"), "no second copy of the hint in the view");
  assert.ok(!source.includes("No workspaces, or their"), "the contradicting line is gone from the view");
});
