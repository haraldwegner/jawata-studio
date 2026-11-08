// Run with `npm test`. Sprint 28h Stage 21 (j): the question the window asks for a partly matching workspace.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { describePrompt, nextPrompt } from "./editorPrompt.ts";

const prompt = (over = {}) => ({
  id: "abc",
  workspace: "shop",
  existing: ["/p/a", "/p/b"],
  missing: ["/p/d"],
  elsewhere: ["/p/c (in workspace lib)"],
  folders: ["/p"],
  ...over
});

test("the first open question is the one shown and none is shown when none is open", () => {
  assert.equal(nextPrompt([]), null);
  const first = prompt({ id: "1" });
  assert.equal(nextPrompt([first, prompt({ id: "2" })]), first);
});

test("the question names the workspace, counts what would be added and says nothing has changed", () => {
  const view = describePrompt(prompt());
  assert.match(view.question, /"shop"/);
  assert.match(view.question, /Add the 1 missing project to it/);
  assert.match(view.question, /Nothing has been changed yet/);
  assert.equal(describePrompt(prompt({ missing: ["/p/d", "/p/e"] })).question.includes("2 missing projects"), true);
});

test("the three lists are drawn with their headings and an empty list is left out", () => {
  const view = describePrompt(prompt());
  assert.deepEqual(
    view.sections.map((s) => s.heading),
    ["Already in shop", "Missing, added by Update workspace", "In another workspace, left there"]
  );
  assert.deepEqual(view.sections[1].paths, ["/p/d"]);
  const bare = describePrompt(prompt({ existing: [], elsewhere: [] }));
  assert.deepEqual(bare.sections.map((s) => s.heading), ["Missing, added by Update workspace"]);
});

test("the two buttons carry the words the owner's contract names", () => {
  const view = describePrompt(prompt());
  assert.equal(view.update, "Update workspace");
  assert.equal(view.decline, "Not now");
});

test("the dialog component asks the studio for the questions and answers through the two commands", () => {
  const source = readFileSync(new URL("./components/EditorUpdatePrompt.svelte", import.meta.url), "utf8");
  assert.match(source, /editorUpdatePrompts\(\)/);
  assert.match(source, /answerEditorUpdate\(prompt\.id, update\)/);
  assert.match(source, /on:click=\{\(\) => answer\(true\)\}[^>]*>\{view\.update\}/);
  assert.match(source, /on:click=\{\(\) => answer\(false\)\}[^>]*>\{view\.decline\}/);
  assert.match(source, /jawata:\/\/editor-prompts-changed/);
  // the listener is registered before the first read (an event between the two would otherwise be lost), and the first
  // read still happens, for questions asked before the window had loaded
  const follow = source.slice(source.indexOf("async function follow()"), source.indexOf("onMount(() => {"));
  assert.ok(follow.includes('await listen("jawata://editor-prompts-changed"'), "follow registers the listener");
  assert.ok(follow.indexOf("await listen(") < follow.indexOf("await refresh();"), "the listener comes before the first refresh");
  assert.match(source, /onMount\(\(\) => \{\s*void follow\(\);/);
});
