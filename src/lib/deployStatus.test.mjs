// Run with `npm test`. D56 (b): the dashboard's "Last deploy" says, as two lines, which editor extensions were installed or removed
// and which jawata servers were set up for which agents, by the names a person knows, not by ids.
import assert from "node:assert/strict";
import { test } from "node:test";
import { deployStatusLines } from "./deployStatus.ts";

const row = (client, status, message = "") => ({ client, targetPath: "", status, message, changedSections: [], validationErrors: [] });

const EDITOR =
  "Editor extensions, installed through each editor's own command. With `code`, into ~/.vscode/extensions: installed jawata chat and jawata Java.";

test("a deploy is two lines: the editor extensions first, then the jawata servers per agent, by name", () => {
  const status = deployStatusLines({
    mode: "deploy",
    ok: true,
    detail: "",
    durationMs: 1,
    clients: [
      row("cursor", "success"),
      row("claude", "success"),
      row("vscode", "success"),
      row("jawata-chat", "success"),
      row("vscode-settings", "success"),
      row("grok", "skipped"),
      row("vscode-extensions", "success", EDITOR),
    ],
  });
  assert.equal(status.failed, false);
  assert.deepEqual(status.lines, [
    `Last deploy: ${EDITOR}`,
    "jawata servers set up for agents: Cursor, Claude Code, VS Code, the jawata chat in VS Code and jawata Java in VS Code (5 of 6); skipped: Grok.",
  ]);
});

test("a failure is named by its agent's name and marks the status as failed, and an editor failure does too", () => {
  const status = deployStatusLines({
    mode: "deploy",
    ok: false,
    detail: "",
    durationMs: 1,
    clients: [row("claude_desktop", "failed"), row("copilot_cli", "success")],
  });
  assert.equal(status.failed, true);
  assert.deepEqual(status.lines, ["Last deploy: jawata servers set up for agents: Copilot CLI (1 of 2); failed: Claude Desktop."]);

  const editorFailed = deployStatusLines({
    mode: "deploy",
    ok: false,
    detail: "",
    durationMs: 1,
    clients: [row("vscode", "success"), row("vscode-extensions", "failed", "Editor extensions, installed through each editor's own command. Not done: x.")],
  });
  assert.equal(editorFailed.failed, true);
  assert.equal(editorFailed.lines.length, 2);
  assert.ok(!editorFailed.lines.join(" ").includes("vscode-extensions"), editorFailed.lines.join("\n"));
});

test("a remove says what was taken out, and none is said when nothing was", () => {
  const status = deployStatusLines({
    mode: "delete",
    ok: true,
    detail: "",
    durationMs: 1,
    clients: [row("vscode", "skipped"), row("vscode-extensions", "success", "Editor extensions, removed through each editor's own command.")],
  });
  assert.deepEqual(status.lines, [
    "Last remove: Editor extensions, removed through each editor's own command.",
    "jawata servers taken out of agents: none (0 of 1); skipped: VS Code.",
  ]);
});

test("an id the studio has no name for is shown as it is, never dropped", () => {
  const status = deployStatusLines({ mode: "deploy", ok: true, detail: "", durationMs: 1, clients: [row("some-new-agent", "success")] });
  assert.deepEqual(status.lines, ["Last deploy: jawata servers set up for agents: some-new-agent (1 of 1)."]);
});
