import assert from "node:assert/strict";
import { test } from "node:test";
import { adapterNotice } from "./adapterNotice.ts";

test("names the brain a person knows, gives the install command exactly as the studio carries it, and says where to look afterwards", () => {
  const notice = adapterNotice("Claude Code", { brain: "claude-code", install: "npm install -g @agentclientprotocol/claude-agent-acp" });
  assert.equal(notice.before, "Claude Code is not available for the chat. To use it, run");
  assert.equal(notice.command, "npm install -g @agentclientprotocol/claude-agent-acp");
  assert.match(notice.after, /open this tab again/);
});

test("the command is never reworded: what is shown is what is copied", () => {
  const install = "npm install -g @zed-industries/codex-acp";
  assert.equal(adapterNotice("Codex", { brain: "codex", install }).command, install);
});
