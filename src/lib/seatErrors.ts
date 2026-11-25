/**
 * The line the Seats page shows when its brain check failed: none when the page already says the same thing, which it does when no engine
 * is running to ask (28h z75).
 */
export function brainCheckError(error: string, pageErrors: string[]): string {
  return pageErrors.includes(error) ? "" : `brain check: ${error}`;
}
