// Run with `npm test`. 28h z80: a new workspace with no project was held only in the page, so it was gone after a restart.
import assert from "node:assert/strict";
import { test } from "node:test";
import { loadPinned, savePinned } from "./pinnedWorkspaces.ts";

function store() {
  const values = new Map();
  return { getItem: (k) => values.get(k) ?? null, setItem: (k, v) => values.set(k, String(v)) };
}

test("the empty workspaces saved are the ones loaded again", () => {
  const kept = store();
  savePinned(kept, new Set(["dogfood", "second"]));
  assert.deepEqual([...loadPinned(kept)].sort(), ["dogfood", "second"]);
});

test("nothing saved, a broken value and a store that throws all load as no workspace", () => {
  assert.equal(loadPinned(store()).size, 0);
  const broken = store();
  broken.setItem("jawata.emptyWorkspaces", "{not json");
  assert.equal(loadPinned(broken).size, 0);
  const refusing = { getItem: () => { throw new Error("no storage"); }, setItem: () => { throw new Error("no storage"); } };
  assert.equal(loadPinned(refusing).size, 0);
  assert.doesNotThrow(() => savePinned(refusing, new Set(["x"])));
});
