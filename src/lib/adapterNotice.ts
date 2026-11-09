/** A brain the chat could run on if its adapter were installed (D54 (g)); what the studio's `missingAdapters` carries per brain. */
export interface MissingAdapter {
  brain: string;
  install: string;
}

/**
 * What the Seats page says about a brain whose adapter is not on this machine, in the three parts the page draws: the sentence before the
 * command, the command (shown as code, so a person can copy it), and the sentence after. The studio downloads no agent program, so what a person
 * can act on is this: the command that installs it, and where to look afterwards.
 */
export function adapterNotice(label: string, missing: MissingAdapter): { before: string; command: string; after: string } {
  return {
    before: `${label} is not available for the chat. To use it, run`,
    command: missing.install,
    after: "in a terminal, then open this tab again.",
  };
}
