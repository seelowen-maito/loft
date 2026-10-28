/** The chat header's context size warning. */

/** Past this many tokens in context a Session leaves the smart zone: the same limit grills compact at. */
export const CONTEXT_WARN_AT = 80_000;

/** Whether the context size shows in the warning color. */
export function contextWarns(tokens: number): boolean {
  return tokens >= CONTEXT_WARN_AT;
}
