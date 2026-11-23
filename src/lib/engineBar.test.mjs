// Run with `npm test`. The status bar names the version each running engine reports, and says so only where one differs from the installed one.
import assert from "node:assert/strict";
import { test } from "node:test";
import { engineBar } from "./engineBar.ts";

const row = (workspace, version) => ({ workspace, version, startedAt: null });

test("engines on the installed version are one line and no warning", () => {
  const bar = engineBar([row("a", "4.6.13"), row("b", "4.6.13"), row("c", "4.6.13")], "4.6.13");
  assert.equal(bar.summary, "Engines · 4.6.13 (3 running)");
  assert.deepEqual(bar.mismatches, []);
});

test("an engine that reports another version than the installed one gets a line naming it and what to do", () => {
  const bar = engineBar([row("alpha", "4.6.12"), row("beta", "4.6.13")], "4.6.13");
  assert.equal(bar.summary, "Engines · 4.6.12: alpha · 4.6.13: beta");
  assert.deepEqual(bar.mismatches, ["alpha is running 4.6.12, installed is 4.6.13 — restart it."]);
});

test("a workspace whose engine did not answer is not listed, and nothing running is an empty bar", () => {
  assert.deepEqual(engineBar([row("a", null)], "4.6.13"), { summary: "", mismatches: [] });
  assert.equal(engineBar([row("a", null), row("b", "4.6.13")], "4.6.13").summary, "Engines · 4.6.13 (1 running)");
});

test("without a known installed version no engine is called out", () => {
  assert.deepEqual(engineBar([row("a", "4.6.12")], null).mismatches, []);
});
