/**
 * The busy flags (`isBusy`, `deployBusy`) disable the dashboard's buttons for as long as an action runs.
 * A flag that is set and never cleared leaves "Deploy to agents" and "Remove from agents" greyed for good
 * (Sprint 28h Stage 21, defect (h)3, seen in the installed 4.4.17).
 *
 * Every path in the store that cleared a flag did so after an `await` of a Tauri `invoke`. An `invoke` that
 * never answers (a command that panicked inside an async task, a command queued behind another that holds
 * the one UI thread, a download that stalls) neither resolves nor rejects, so neither the success path nor
 * the `catch` ever ran. This file holds the one rule that closes that: an action gets a budget, and when the
 * budget is spent the wait ends with an error that names the action. The store clears its flag in `finally`,
 * so the flag comes back whatever the action did.
 *
 * Pure and dependency free so that it is testable with the node runner (`npm test`).
 */

/** How long each kind of action may hold the buttons before the wait is given up (milliseconds). */
export const BUDGET_MS = {
  /** Reads and small writes: the dashboard, a rename, a settings save. */
  quick: 60_000,
  /** Fleet actions that wait on residents or the network: start all, reload, engine download. */
  fleet: 15 * 60_000,
  /** A deploy: config files, plus (VS Code) two extension downloads and installs. */
  deploy: 5 * 60_000
} as const;

/** The wait ended because the budget was spent, not because the action answered. */
export class BudgetSpentError extends Error {
  readonly what: string;
  readonly budgetMs: number;
  constructor(what: string, budgetMs: number) {
    super(
      `${what} did not answer within ${Math.round(budgetMs / 1000)} s. The buttons are released; ` +
        `the action may still be running in the background, so check before repeating it.`
    );
    this.name = "BudgetSpentError";
    this.what = what;
    this.budgetMs = budgetMs;
  }
}

export interface Clock {
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
}

const realClock: Clock = {
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>)
};

/**
 * Settles with `work`, or rejects with {@link BudgetSpentError} when `budgetMs` has passed first.
 * The timer is always cleared, so a prompt answer leaves nothing behind.
 */
export function withBudget<T>(
  work: Promise<T>,
  budgetMs: number,
  what: string,
  clock: Clock = realClock
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const handle = clock.setTimeout(() => reject(new BudgetSpentError(what, budgetMs)), budgetMs);
    work.then(
      (value) => {
        clock.clearTimeout(handle);
        resolve(value);
      },
      (error) => {
        clock.clearTimeout(handle);
        reject(error);
      }
    );
  });
}

/**
 * Runs one busy action: `begin` sets the flag, the call runs under its budget, a failure is handed to
 * `fail`, and `release` runs in `finally`, so every outcome (answer, error, no answer) clears the flag.
 */
export async function runBusy<T>(steps: {
  what: string;
  budgetMs: number;
  begin: () => void;
  call: () => Promise<T>;
  succeed?: (value: T) => void;
  fail: (error: unknown) => void;
  release: () => void;
  clock?: Clock;
}): Promise<void> {
  steps.begin();
  try {
    const value = await withBudget(steps.call(), steps.budgetMs, steps.what, steps.clock);
    steps.succeed?.(value);
  } catch (error) {
    steps.fail(error);
  } finally {
    steps.release();
  }
}
