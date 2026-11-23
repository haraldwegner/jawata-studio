/** What the engines that are running report about themselves, as the small bar under the window reads it (28h z57).
 *
 * His words, 2026-11-23: a workspace ran 4.6.12 while the studio showed 4.6.13 — "a single status line with the server running would be
 * greatz" — and *"This should be a small bar only."* So: one line with the version each running engine reports, and a line of its own only
 * where one of them differs from the installed version, naming the workspace and what to do. */

export interface EngineVersionRow {
  workspace: string;
  version: string | null;
  startedAt: string | null;
}

export interface EngineBar {
  /** One line: the running engines and their versions; grouped where several report the same. Empty when none is running. */
  summary: string;
  /** One line per engine that runs another version than the installed one. */
  mismatches: string[];
}

/** `installed` is the version on disk, or `null` when the studio runs a hand-picked jar (whose version it cannot know) or has none. */
export function engineBar(rows: EngineVersionRow[], installed: string | null): EngineBar {
  const running = rows.filter((r): r is EngineVersionRow & { version: string } => r.version !== null);
  if (running.length === 0) return { summary: "", mismatches: [] };
  const byVersion = new Map<string, string[]>();
  for (const row of running) byVersion.set(row.version, [...(byVersion.get(row.version) ?? []), row.workspace]);
  const parts = [...byVersion.entries()].map(([version, workspaces]) =>
    byVersion.size === 1 ? `${version} (${workspaces.length} running)` : `${version}: ${workspaces.join(", ")}`,
  );
  const mismatches = installed
    ? running
        .filter((r) => r.version !== installed)
        .map((r) => `${r.workspace} is running ${r.version}, installed is ${installed} — restart it.`)
    : [];
  return { summary: `Engines · ${parts.join(" · ")}`, mismatches };
}
