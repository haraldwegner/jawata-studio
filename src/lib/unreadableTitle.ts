/**
 * The mouseover of a workspace's "cannot be read" badge: the engine's own warning, which names the cause and the remedy; without one, the
 * consequence only, never a guessed cause (28h z74).
 */
export function unreadableTitle(warning: string | null | undefined): string {
  return warning || "This workspace cannot be read. Refactorings are refused while this is true.";
}
