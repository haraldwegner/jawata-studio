import type { DeployToAgentsResult } from "./api/tauri";

/** The result row of the editor extensions (the studio's `vscode_deploy::EXTENSIONS_ROW`): one step of a run, not an agent he ticked. */
export const EXTENSIONS_ROW = "vscode-extensions";

/** The names a person knows the result rows by. An id without a name here is shown as it is, never dropped. */
const AGENT_NAMES: Record<string, string> = {
  cursor: "Cursor",
  claude: "Claude Code",
  claude_desktop: "Claude Desktop",
  codex: "Codex",
  copilot_cli: "Copilot CLI",
  vscode: "VS Code",
  grok: "Grok",
  antigravity: "Antigravity",
  intellij: "IntelliJ",
  zed: "Zed",
  "jawata-chat": "the jawata chat in VS Code",
  "vscode-settings": "jawata Java in VS Code",
};

export function agentName(id: string): string {
  return AGENT_NAMES[id] ?? id;
}

/** "a", "a and b", "a, b and c". */
function andList(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/**
 * The last run in plain words (D56 (b)), as two lines: the editor extensions installed or removed, the studio's own sentence, and
 * the jawata servers set up for each agent, by name. Without an editor step the run is the one line about the servers. `failed` is
 * true when any part failed.
 */
export function deployStatusLines(result: DeployToAgentsResult): { lines: string[]; failed: boolean } {
  const remove = result.mode === "delete";
  const run = remove ? "Last remove" : "Last deploy";
  const editor = result.clients.find((entry) => entry.client === EXTENSIONS_ROW);
  const agents = result.clients.filter((entry) => entry.client !== EXTENSIONS_ROW);
  const named = (status: string) => agents.filter((entry) => entry.status === status).map((entry) => agentName(entry.client));
  const done = named("success");
  const skipped = named("skipped");
  const failed = named("failed");
  let servers = `${remove ? "jawata servers taken out of agents" : "jawata servers set up for agents"}: ${done.length ? andList(done) : "none"} (${done.length} of ${agents.length})`;
  if (skipped.length) servers += `; skipped: ${andList(skipped)}`;
  if (failed.length) servers += `; failed: ${andList(failed)}`;
  servers += ".";
  const lines = editor ? [`${run}: ${editor.message}`, servers] : [`${run}: ${servers}`];
  return { lines, failed: result.clients.some((entry) => entry.status === "failed") };
}
