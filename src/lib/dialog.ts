import { confirm } from "@tauri-apps/plugin-dialog";

/**
 * A destructive-action confirmation that ACTUALLY renders in the Tauri webview.
 *
 * Finding #8 (Sprint 26 dogfood): the frontend guarded destructive actions with
 * the browser `window.confirm()`, which is a no-op in the WebKitGTK/Tauri
 * webview — it returns without showing a dialog, so a "confirmed" delete ran
 * immediately with no prompt (a real workspace was lost this way). The
 * tauri-plugin-dialog `confirm` renders a native modal and resolves to the
 * user's choice (OK = true, Cancel = false). Every destructive control uses
 * THIS, never `window.confirm`.
 */
export function confirmDestructive(
  message: string,
  title = "jawata-studio",
): Promise<boolean> {
  return confirm(message, { title, kind: "warning" });
}
