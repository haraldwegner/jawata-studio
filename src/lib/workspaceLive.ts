import type { RuntimeStatusRecord } from "./api/tauri";

/**
 * Whether a service phase counts as live: running, or starting (a cold import can take minutes, and a
 * starting workspace is as undeletable as a running one). The backend enforces the same rule where it
 * matters, in `manager_service::deletion_refusal`; this is the one place the screens read it from, so a
 * new phase or a fifth caller cannot make the screens disagree with each other (S19-10).
 */
export function isLivePhase(phase: string | undefined): boolean {
  return phase === "running" || phase === "starting";
}

/** True when any of the projects is live. The statuses are passed in, so a template that calls this re-runs when they change. */
export function anyLive(
  projects: ReadonlyArray<{ id: string }>,
  statuses: Record<string, RuntimeStatusRecord>,
): boolean {
  return projects.some((project) => isLivePhase(statuses[project.id]?.phase));
}
