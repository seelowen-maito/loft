import type { Agent } from "./api";

/** A run that has gone quiet this long is shown as possibly stuck, with a restart. */
export const QUIET_MINUTES = 10;

/** Why a running agent may need a nudge: it replied and waits on background work, or nothing has come from it for a while. */
export function runWatch(a: Pick<Agent, "running" | "waiting_on" | "waiting_since" | "last_output_at" | "last_run_at">, lastMessageAt: string | undefined, now: number): { kind: "waiting" | "quiet"; minutes: number; text: string } | null {
  if (!a.running) return null;
  const mins = (t: string | null | undefined) => (t ? Math.max(0, Math.floor((now - new Date(t).getTime()) / 60000)) : 0);
  const tasks = a.waiting_on ?? [];
  if (tasks.length) {
    const more = tasks.length > 1 ? ` (+${tasks.length - 1} more)` : "";
    return { kind: "waiting", minutes: mins(a.waiting_since), text: `Waiting on a background task: ${tasks[0]}${more}` };
  }
  // The run's start counts too: a quiet run (a Redraw, a grill) posts nothing to the chat.
  const last = [a.last_output_at, a.last_run_at, lastMessageAt].filter(Boolean).sort().pop();
  const m = mins(last);
  return last && m >= QUIET_MINUTES ? { kind: "quiet", minutes: m, text: `No activity for ${m} min` } : null;
}
