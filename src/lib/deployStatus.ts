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
  "intellij-acp": "IntelliJ's AI chat",
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

/** What the editor extensions step did, in the run's own direction: installed, removed, skipped or failed. */
function editorOutcome(status: string, remove: boolean): string {
  if (status === "success") return remove ? "removed" : "installed";
  if (status === "skipped") return "were skipped";
  return "failed";
}

/**
 * The last run in one sentence for the dashboard (his word, 2026-11-16: "Who is going to read this?"): for how many agents jawata is set
 * up, whether the editor extensions went in, and by name only what was skipped or failed. What each step did in full is `lines`, the
 * details under it.
 */
function summaryOf(result: DeployToAgentsResult): string {
  const remove = result.mode === "delete";
  const editor = result.clients.find((entry) => entry.client === EXTENSIONS_ROW);
  const agents = result.clients.filter((entry) => entry.client !== EXTENSIONS_ROW);
  const named = (status: string) => agents.filter((entry) => entry.status === status).map((entry) => agentName(entry.client));
  const done = agents.filter((entry) => entry.status === "success").length;
  let sentence = `${remove ? "Last remove" : "Last deploy"}: jawata is ${remove ? "taken out of" : "set up for"} ${done} of ${agents.length} agents`;
  if (editor) {
    const outcome = editorOutcome(editor.status, remove);
    sentence += `, and the editor extensions ${outcome.startsWith("were") || outcome === "failed" ? outcome : "are " + outcome}`;
  }
  sentence += ".";
  const failed = named("failed");
  const skipped = named("skipped");
  if (failed.length) sentence += ` Failed: ${andList(failed)}.`;
  if (skipped.length) sentence += ` Skipped: ${andList(skipped)}.`;
  return sentence;
}

/**
 * The last run in plain words: `summary` is the one sentence the dashboard shows; `lines` are its details (D56 (b)), the editor
 * extensions installed or removed, the studio's own sentence, and the jawata servers set up for each agent, by name. Without an editor
 * step the details are the one line about the servers. `failed` is true when any part failed.
 */
export function deployStatusLines(result: DeployToAgentsResult): { summary: string; lines: string[]; failed: boolean } {
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
  return { summary: summaryOf(result), lines, failed: result.clients.some((entry) => entry.status === "failed") };
}
