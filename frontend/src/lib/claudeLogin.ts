import type { ClaudeLogin } from "./api";

/** What the login card shows after a check: a good check clears an earlier error; a failed one keeps the last status and shows its error. */
export function afterCheck(prev: { s: ClaudeLogin | null; err: string | null }, r: ClaudeLogin | Error): { s: ClaudeLogin | null; err: string | null } {
  return r instanceof Error ? { s: prev.s, err: r.message } : { s: r, err: null };
}
