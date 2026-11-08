// Run with `npm test` (node's own runner; the repo has no frontend test framework, so the busy-flag rule
// lives in the dependency free busy.ts and is held here). Sprint 28h Stage 21 (h)3.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { BUDGET_MS, BudgetSpentError, runBusy, withBudget } from "./busy.ts";

/** A clock the test drives by hand: nothing waits in real time. */
function fakeClock() {
  const timers = new Map();
  let next = 1;
  return {
    setTimeout: (fn, ms) => {
      const id = next++;
      timers.set(id, { fn, ms });
      return id;
    },
    clearTimeout: (id) => timers.delete(id),
    pending: () => timers.size,
    fireAll: () => {
      for (const [id, t] of [...timers]) {
        timers.delete(id);
        t.fn();
      }
    }
  };
}

/** The flag the store keeps, with the same three moves the store makes. */
function flagged(clock, call, budgetMs = 1000) {
  const state = { busy: false, error: undefined, value: undefined };
  const done = runBusy({
    what: "The action",
    budgetMs,
    begin: () => (state.busy = true),
    call,
    succeed: (v) => (state.value = v),
    fail: (e) => (state.error = e),
    release: () => (state.busy = false),
    clock
  });
  return { state, done };
}

test("an action that answers releases the flag and keeps its value", async () => {
  const clock = fakeClock();
  const { state, done } = flagged(clock, async () => 42);
  assert.equal(state.busy, true, "control: the flag is set while the action runs");
  await done;
  assert.equal(state.busy, false);
  assert.equal(state.value, 42);
  assert.equal(clock.pending(), 0, "the budget timer is cleared on an answer");
});

test("an action that fails releases the flag and reports the error", async () => {
  const clock = fakeClock();
  const { state, done } = flagged(clock, async () => {
    throw new Error("boom");
  });
  await done;
  assert.equal(state.busy, false);
  assert.equal(state.error.message, "boom");
  assert.equal(clock.pending(), 0);
});

test("an action that never answers releases the flag when its budget is spent", async () => {
  const clock = fakeClock();
  // The shape of the installed 4.4.17 defect: an invoke that neither resolves nor rejects.
  const { state, done } = flagged(clock, () => new Promise(() => {}));
  assert.equal(state.busy, true, "control: still waiting");
  clock.fireAll();
  await done;
  assert.equal(state.busy, false, "the buttons come back");
  assert.ok(state.error instanceof BudgetSpentError);
  assert.match(state.error.message, /The action did not answer within 1 s/);
});

test("an answer after the budget is spent does not set anything again", async () => {
  const clock = fakeClock();
  let late;
  const { state, done } = flagged(clock, () => new Promise((resolve) => (late = resolve)));
  clock.fireAll();
  await done;
  late("too late");
  await Promise.resolve();
  assert.equal(state.busy, false);
  assert.equal(state.value, undefined, "the late value is dropped, not applied");
});

test("a failure in the success step still releases the flag", async () => {
  const clock = fakeClock();
  const state = { busy: false, error: undefined };
  await runBusy({
    what: "The action",
    budgetMs: 1000,
    begin: () => (state.busy = true),
    call: async () => 1,
    succeed: () => {
      throw new Error("sync failed");
    },
    fail: (e) => (state.error = e),
    release: () => (state.busy = false),
    clock
  });
  assert.equal(state.busy, false);
  assert.equal(state.error.message, "sync failed");
});

test("withBudget passes the real timer path too", async () => {
  await assert.rejects(withBudget(new Promise(() => {}), 20, "A real wait"), BudgetSpentError);
  assert.equal(await withBudget(Promise.resolve("ok"), 20, "A real wait"), "ok");
});

test("the budgets are ordered and finite", () => {
  assert.ok(BUDGET_MS.quick < BUDGET_MS.deploy && BUDGET_MS.deploy <= BUDGET_MS.fleet);
  for (const ms of Object.values(BUDGET_MS)) assert.ok(Number.isFinite(ms) && ms > 0);
});

// The store itself: a flag set anywhere else would be set with no release behind it.
test("the store sets a busy flag only through runBusy", () => {
  const source = readFileSync(new URL("./stores/app.ts", import.meta.url), "utf8");
  const count = (needle) => source.split(needle).length - 1;
  // busyAction and updateManagerSettings are the two places `isBusy: true` is written; each is inside begin().
  assert.equal(count("isBusy: true"), 2, "isBusy is set true in busyAction and updateManagerSettings only");
  assert.equal(count("deployBusy: true"), 1, "deployBusy is set true in deployAction only");
  assert.equal(count("serviceProbeBusy: true"), 1, "serviceProbeBusy is set true in probeServices only");
  assert.equal(count("runBusy"), 5, "import + busyAction + updateManagerSettings + probeServices + deployAction");
});
