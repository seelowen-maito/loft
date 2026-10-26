import type { Agent } from "../../lib/api";
import { cx } from "../../lib/format";

export function statusOf(a: Agent): { label: string; dot: string; text: string } {
  if (a.status === "waiting") return { label: "Needs you", dot: "bg-warn", text: "text-warn" };
  if (a.running && a.waiting_on?.length) return { label: "Waiting", dot: "bg-info animate-pulse-soft", text: "text-info" };
  if (a.running || a.status === "running") return { label: "Working", dot: "bg-accent animate-pulse-soft", text: "text-accent" };
  if (a.status === "error") return { label: "Error", dot: "bg-bad", text: "text-bad" };
  if (a.interval_minutes > 0) return { label: "Sleeping", dot: "bg-info/70", text: "text-fg-2" };
  return { label: "Idle", dot: "bg-fg-3", text: "text-fg-3" };
}

/** A run that has gone quiet this long is shown as possibly stuck, with a restart. */
export const QUIET_MINUTES = 10;

/** Why a running agent may need a nudge: it replied and waits on background work, or nothing has come from it for a while. */
export function runWatch(a: Agent, lastMessageAt: string | undefined, now: number): { kind: "waiting" | "quiet"; minutes: number; text: string } | null {
  if (!a.running) return null;
  const mins = (t: string | null | undefined) => (t ? Math.max(0, Math.floor((now - new Date(t).getTime()) / 60000)) : 0);
  const tasks = a.waiting_on ?? [];
  if (tasks.length) {
    const more = tasks.length > 1 ? ` (+${tasks.length - 1} more)` : "";
    return { kind: "waiting", minutes: mins(a.waiting_since), text: `Waiting on a background task: ${tasks[0]}${more}` };
  }
  const last = [a.last_output_at, lastMessageAt].filter(Boolean).sort().pop();
  const m = mins(last);
  return last && m >= QUIET_MINUTES ? { kind: "quiet", minutes: m, text: `No activity for ${m} min` } : null;
}

export function StatusPill({ agent, withText = true }: { agent: Agent; withText?: boolean }) {
  const s = statusOf(agent);
  return (
    <span className="flex min-w-0 items-center gap-1.5 font-mono text-[11px]">
      <span className={cx("size-1.5 shrink-0 rounded-full", s.dot)} />
      <span className={cx("shrink-0 uppercase tracking-wider", s.text)}>{s.label}</span>
      {withText && agent.status_text && <span className="truncate text-fg-3">· {agent.status_text}</span>}
    </span>
  );
}
