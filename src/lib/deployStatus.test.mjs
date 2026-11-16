// Run with `npm test`. The dashboard's "Last deploy" is one short sentence: whether it worked, for how many agents, and by name what failed
// or was skipped (his word, 2026-11-16, on the two long lines of D56 (b): "Who is going to read this?"). The two lines of D56 (b), which
// editor extensions were installed or removed and which agents got jawata's servers, by the names a person knows, are the details under it.
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

const LONG_EDITOR =
  "Editor extensions, installed through each editor's own command. With `code`: installed jawata chat and jawata Java; removed Language Support for Java(TM) by Red Hat, because jawata Java now brings the Java language support, the Testing view and the debugger, and with it Extension Pack for Java and Test Runner for Java, which need it.";

test("the sentence counts the agents only, and says what happened in VS Code, the host, apart from them", () => {
  const status = deployStatusLines({
    mode: "deploy",
    ok: true,
    detail: "",
    durationMs: 1,
    clients: [
      row("cursor", "success"), row("claude", "success"), row("grok", "skipped"),
      row("vscode", "success"), row("jawata-chat", "success"), row("vscode-settings", "success"),
      row("zed", "skipped"), row("intellij-acp", "skipped"), row("vscode-extensions", "success", LONG_EDITOR),
    ],
  });
  assert.equal(status.summary, "Last deploy: jawata is set up for 2 of 3 agents. Skipped: Grok. VS Code: jawata chat and jawata Java installed.");
  assert.ok(!status.summary.includes("Red Hat"), "what was installed and removed is a detail, not the sentence");
  assert.equal(status.lines[0], `Last deploy: ${LONG_EDITOR}`, "the details keep the studio's own sentence in full");
});

test("a failure is named: an agent by its name, anything of VS Code as VS Code", () => {
  const agentFailed = deployStatusLines({ mode: "deploy", ok: false, detail: "", durationMs: 1, clients: [row("claude_desktop", "failed"), row("copilot_cli", "success")] });
  assert.equal(agentFailed.summary, "Last deploy: jawata is set up for 1 of 2 agents. Failed: Claude Desktop.");

  const editorFailed = deployStatusLines({ mode: "deploy", ok: false, detail: "", durationMs: 1, clients: [row("cursor", "success"), row("vscode", "success"), row("vscode-extensions", "failed", "Not done: x.")] });
  assert.equal(editorFailed.summary, "Last deploy: jawata is set up for 1 of 1 agents. VS Code: a step failed, see Details.");

  const editorSkipped = deployStatusLines({ mode: "deploy", ok: true, detail: "", durationMs: 1, clients: [row("cursor", "success"), row("vscode-extensions", "skipped", "No editor.")] });
  assert.equal(editorSkipped.summary, "Last deploy: jawata is set up for 1 of 1 agents. VS Code: the extensions were skipped.");
});

test("the sentence of a remove says what was taken out", () => {
  const status = deployStatusLines({
    mode: "delete",
    ok: true,
    detail: "",
    durationMs: 1,
    clients: [row("cursor", "success"), row("codex", "skipped"), row("vscode", "success"), row("vscode-extensions", "success", "Editor extensions, removed through each editor's own command.")],
  });
  assert.equal(status.summary, "Last remove: jawata is taken out of 1 of 2 agents. Skipped: Codex. VS Code: jawata chat and jawata Java removed.");
});

test("an id the studio has no name for is shown as it is, never dropped", () => {
  const status = deployStatusLines({ mode: "deploy", ok: true, detail: "", durationMs: 1, clients: [row("some-new-agent", "success")] });
  assert.deepEqual(status.lines, ["Last deploy: jawata servers set up for agents: some-new-agent (1 of 1)."]);
});
