// Which listed model a seat's saved model stands for. A seat stores what the brain's own command line takes, often a short alias ("sonnet"),
// while the Brain list shows the names the brain's session offers ("Sonnet 5.5"). The list must select the one the alias stands for and not
// add a line that says the choice is not available (his words, 2026-11-17). Dependency free, so `npm test` holds it.

/** The model of `models` that `saved` names: the same name in any letter case, or, for a short alias, the first model of that family. */
export function listedModel(models: string[] | undefined, saved: string | null | undefined): string | null {
  const wanted = (saved ?? "").trim().toLowerCase();
  if (!wanted || !models || models.length === 0) return null;
  const same = models.find((m) => m.toLowerCase() === wanted);
  if (same) return same;
  // the list is the brain's own, newest first: the first model of the family is the one the alias stands for
  return models.find((m) => m.toLowerCase().startsWith(`${wanted} `)) ?? null;
}

/**
 * The value the Brain select holds for a seat: the listed model its saved model stands for, or the brain's own default when the brain's list
 * is known and holds nothing the saved model stands for. A brain whose list is not known yet keeps the saved value as it is.
 */
export function selectedValue(brain: string, saved: string | null | undefined, models: string[] | undefined): string {
  if (!models || models.length === 0) return `${brain}|${saved ?? ""}`;
  return `${brain}|${listedModel(models, saved) ?? ""}`;
}

/** What the Seats page says beside a brain about its login. */
export function loginNote(login: "loggedIn" | "notLoggedIn" | "unknown"): string {
  if (login === "notLoggedIn") return " (not connected)";
  if (login === "unknown") return " (connection not checkable)";
  return "";
}

/**
 * The note beside a brain in a seat's list: its login, and, only on a seat that is not allowed to change code, that the brain cannot be kept
 * from writing (his words, 2026-11-23: warn for the seats not allowed to edit; "you should not prevent in the engine"). `changesCode` is
 * the seat's own flag; a conversation seat that must not edit (main) is warned like a hire seat.
 */
export function brainNote(
  brain: { label: string; login: "loggedIn" | "notLoggedIn" | "unknown"; writesHeldByInstruction: boolean },
  seatChangesCode: boolean,
): string {
  const restriction = brain.writesHeldByInstruction && !seatChangesCode ? ` (${brain.label} cannot be restricted from writing)` : "";
  return loginNote(brain.login) + restriction;
}

/** Whether the studio offers to start the brain's own login: it is installed and is not known to be connected (a brain with no way to say is offered it too). */
export function canConnect(brain: { installed: boolean; login: "loggedIn" | "notLoggedIn" | "unknown"; loginCommand: string }): boolean {
  return brain.installed && brain.login !== "loggedIn" && brain.loginCommand.trim().length > 0;
}
