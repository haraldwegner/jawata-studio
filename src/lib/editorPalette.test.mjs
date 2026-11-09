import test from "node:test";
import assert from "node:assert/strict";
import { channels, paletteOf } from "./editorPalette.ts";

const dark = { dark: true, colors: { background: "#1e1e1e", panel: "rgb(37, 37, 38)", border: "#454545", text: "#cccccc", muted: "#9d9d9d", accent: "#007acc" } };

test("a colour is read as its three channels, and anything else is not", () => {
  assert.equal(channels("#1e1e1e"), "30, 30, 30");
  assert.equal(channels("rgba(1, 2, 3, 0.5)"), "1, 2, 3");
  assert.equal(channels("transparent"), null);
});

test("a dark VS Code sets the named colours from its own", () => {
  const palette = paletteOf(dark);
  assert.equal(palette["--bg"], "#1e1e1e");
  assert.equal(palette["--accent"], "#007acc");
  assert.equal(palette["--panel-rgb"], "37, 37, 38");
  assert.equal(palette["--line-rgb"], "69, 69, 69");
});

test("a light VS Code, or none, leaves our blue", () => {
  assert.deepEqual(paletteOf({ dark: false, colors: dark.colors }), {});
  assert.deepEqual(paletteOf(null), {});
});
