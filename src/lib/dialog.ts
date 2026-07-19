import { writable } from "svelte/store";

/**
 * In-app themed confirmation for destructive actions.
 *
 * Finding #8 (Sprint 26 dogfood): the frontend guarded destructive actions with
 * the browser `window.confirm()`, which is a no-op in the WebKitGTK/Tauri
 * webview — it returned without showing a dialog, so a "confirmed" delete ran
 * immediately with no prompt (a real workspace was lost this way). The first
 * fix routed through `tauri-plugin-dialog`, which renders — but as an unstyled
 * OS/GTK box that does not match the app.
 *
 * This is a Svelte modal instead (app DOM, not a browser or OS API): it renders
 * reliably AND matches the app theme. `confirmDestructive()` publishes a request
 * that the single mounted `<ConfirmDialog/>` shows, and resolves to the user's
 * choice (OK = true, Cancel / Esc / backdrop = false).
 */
export type ConfirmRequest = {
  message: string;
  title: string;
  resolve: (ok: boolean) => void;
};

export const confirmRequest = writable<ConfirmRequest | null>(null);

export function confirmDestructive(
  message: string,
  title = "Please confirm",
): Promise<boolean> {
  return new Promise((resolve) => {
    confirmRequest.set({ message, title, resolve });
  });
}
