/**
 * What the Memory view tells the person about the store's reachability (Sprint 28h Stage 21 (h)1).
 *
 * With no workspace the view printed "No workspaces, or their services are not reachable." and, beside
 * it, "workspace starting - retrying automatically...". They contradicted each other: nothing was
 * starting, because nothing existed. The experience store is reached through a workspace's resident, so
 * with no workspace there is nothing to read, and the view says that, once. "Starting" is only said when
 * a workspace exists and its service does not answer yet.
 *
 * Pure and dependency free, so that it is testable with `npm test`.
 */

export interface StatusRow {
  reachable: boolean;
}

export interface MemoryNotice {
  /** The line shown where the store rows would be, when there are none. Null when rows are shown. */
  summary: string | null;
  /** The small hint beside the buttons. Null when there is nothing to wait for. */
  hint: string | null;
}

export const NO_WORKSPACE_SUMMARY =
  "No workspace exists yet, so there is no memory to show. The memory is read through a workspace's " +
  "service: add a project to a workspace on the Dashboard and its memory appears here.";

export const STARTING_HINT = "workspace starting — retrying automatically…";

export function memoryNotice(statuses: StatusRow[], statusReadOnce: boolean, reading: string): MemoryNotice {
  if (statuses.length === 0) {
    // Before the first answer the store has not been asked, so nothing is claimed about workspaces.
    return { summary: statusReadOnce ? NO_WORKSPACE_SUMMARY : reading, hint: null };
  }
  const waiting = statuses.some((s) => !s.reachable);
  return { summary: null, hint: waiting ? STARTING_HINT : null };
}
