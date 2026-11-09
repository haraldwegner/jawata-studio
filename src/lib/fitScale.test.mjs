import assert from "node:assert/strict";
import { test } from "node:test";
import { fitScale } from "./fitScale.ts";

test("a chart in a bigger room grows, by the side that fills first", () => {
  assert.equal(fitScale({ width: 700, height: 300 }, { width: 1050, height: 900 }), 1.5, "width fills first");
  assert.equal(fitScale({ width: 700, height: 300 }, { width: 2100, height: 450 }), 1.5, "height fills first");
});

test("it never grows past the limit, so a wide screen does not make a poster", () => {
  assert.equal(fitScale({ width: 700, height: 300 }, { width: 5000, height: 5000 }), 1.8);
});

test("a chart wider than the room shrinks, down to the lower limit and no further", () => {
  assert.equal(fitScale({ width: 1000, height: 300 }, { width: 800, height: 900 }), 0.8);
  assert.equal(fitScale({ width: 1000, height: 300 }, { width: 200, height: 900 }), 0.6);
});

test("a chart or a room with no size is left as it is", () => {
  assert.equal(fitScale({ width: 0, height: 300 }, { width: 800, height: 900 }), 1);
  assert.equal(fitScale({ width: 700, height: 300 }, { width: 0, height: 900 }), 1);
  assert.equal(fitScale({ width: NaN, height: 300 }, { width: 800, height: 900 }), 1);
});
