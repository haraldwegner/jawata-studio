// D53 (t): the pure half of following VS Code: which named colours a dark editor sets.
export interface EditorThemeFile {
  dark: boolean;
  colors: Record<string, string>;
}

/** "#1e1e1e" or "rgb(30, 30, 30)" as "30, 30, 30"; null for anything else. */
export function channels(color: string): string | null {
  const hex = /^#([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(color.trim());
  if (hex) {
    const n = parseInt(hex[1], 16);
    return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
  }
  const rgb = /^rgba?\(\s*(\d+)[ ,]+(\d+)[ ,]+(\d+)/i.exec(color.trim());
  return rgb ? `${rgb[1]}, ${rgb[2]}, ${rgb[3]}` : null;
}

/** The named colours a dark editor sets. Each is named after the role it plays, so the mapping is one place. */
export function paletteOf(theme: EditorThemeFile | null): Record<string, string> {
  if (!theme?.dark) return {};
  const { background, panel, border, text, muted, accent } = theme.colors;
  const out: Record<string, string> = {};
  const set = (name: string, value: string | undefined) => { if (value) out[name] = value; };
  const rgb = (name: string, value: string | undefined) => { const c = value ? channels(value) : null; if (c) out[name] = c; };
  set("--bg", background);
  set("--text", text);
  set("--text-strong", text);
  set("--text-soft", text);
  set("--muted", muted);
  set("--accent", accent);
  set("--accent-soft", accent);
  set("--accent-strong", accent);
  set("--accent-deep", accent);
  rgb("--line-rgb", border || muted);
  rgb("--accent-rgb", accent);
  rgb("--accent-strong-rgb", accent);
  rgb("--accent-deep-rgb", accent);
  rgb("--panel-rgb", panel || background);
  rgb("--raised-rgb", panel || background);
  return out;
}

