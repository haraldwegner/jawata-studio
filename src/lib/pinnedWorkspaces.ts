/**
 * The workspaces that have no project yet. A workspace with a project is kept by the engine's project list; one without lived only in the
 * page and was lost at restart (28h z80), so its name is kept in the window's own storage.
 */
type Kept = Pick<Storage, "getItem" | "setItem">;

const KEY = "jawata.emptyWorkspaces";

export function loadPinned(kept: Kept): Set<string> {
  try {
    const names = JSON.parse(kept.getItem(KEY) ?? "[]");
    return new Set(Array.isArray(names) ? names.filter((name): name is string => typeof name === "string" && name.length > 0) : []);
  } catch {
    return new Set();
  }
}

export function savePinned(kept: Kept, names: Set<string>): void {
  try {
    kept.setItem(KEY, JSON.stringify([...names]));
  } catch {
    // a window that may not store keeps the names for this run only
  }
}
