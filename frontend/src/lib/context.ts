/** The chat header's context size warning. */

/** Past this many tokens in context a Session leaves the smart zone: the same limit grills compact at. */
export const CONTEXT_WARN_AT = 80_000;

/** Whether the context size shows in the warning color. */
export function contextWarns(tokens: number): boolean {
  return tokens >= CONTEXT_WARN_AT;
}

/** The size an agent's row in the list shows: only with a Session that has replied, as the chat header does. */
export function listedContext(a: { session_id: string | null; context_tokens?: number | null }): number | null {
  return a.session_id != null ? (a.context_tokens ?? null) : null;
}
