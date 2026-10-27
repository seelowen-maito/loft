/** Run timer text, shared by the chat's run indicator and the sidebar timer. */

import type { Message } from "./api";

/** "45s", "2m 23s", "1h 4m 9s". Negative spans (clock skew) read as 0s. */
export function fmtElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  if (h) return `${h}h ${m}m ${sec}s`;
  if (m) return `${m}m ${sec}s`;
  return `${sec}s`;
}

/** Time since the run's start (the server's `last_run_at`), so a reopened chat shows the right count. */
export function runElapsed(startedAt: string | null | undefined, now: number): number {
  const t = startedAt ? new Date(startedAt).getTime() : NaN;
  return Number.isNaN(t) ? 0 : now - t;
}

/** "Done in 2m 23s · Thu 16:23": the finished run's duration, then its end in the user's locale. */
export function fmtDone(durationMs: number, endedAt: string | Date, locale?: string): string {
  const end = new Date(endedAt);
  const day = end.toLocaleDateString(locale, { weekday: "short" });
  const time = end.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
  return `Done in ${fmtElapsed(durationMs)} · ${day} ${time}`;
}

type RunMessage = Pick<Message, "kind" | "run_id" | "created_at" | "meta">;

/** The run that ended last, for the "Done in" line; null after a /clear, or when the latest run ended
 * without a run_end (claude exited without a result). Measured run_start → run_end, like the live timer. */
export function lastFinishedRun(messages: RunMessage[]): { durationMs: number; endedAt: string } | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.kind === "clear" || m.kind === "run_start") return null;
    if (m.kind !== "run_end") continue;
    const start = messages.slice(0, i).findLast((s) => s.kind === "run_start" && s.run_id === m.run_id);
    const durationMs = start ? new Date(m.created_at).getTime() - new Date(start.created_at).getTime() : m.meta?.duration_ms;
    return durationMs == null ? null : { durationMs: Number(durationMs), endedAt: m.created_at };
  }
  return null;
}
