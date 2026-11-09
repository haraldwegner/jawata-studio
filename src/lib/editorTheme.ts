// D53 (t): in dark mode the studio takes VS Code's colours; a light VS Code, or none, leaves our blue.
import { editorTheme } from "./api/tauri";
import { paletteOf, type EditorThemeFile } from "./editorPalette";

const POLL_MS = 900;

/** Follows the editor from now on; the returned function stops it. */
export function followEditorTheme(root: HTMLElement = document.documentElement): () => void {
  let applied: string[] = [];
  let last = "";
  const tick = async () => {
    let theme: EditorThemeFile | null = null;
    try { theme = await editorTheme(); } catch { theme = null; }
    const palette = paletteOf(theme);
    const key = JSON.stringify(palette);
    if (key === last) return;
    last = key;
    applied.forEach((name) => root.style.removeProperty(name));
    applied = Object.keys(palette);
    applied.forEach((name) => root.style.setProperty(name, palette[name]));
  };
  void tick();
  const timer = setInterval(() => void tick(), POLL_MS);
  return () => clearInterval(timer);
}
