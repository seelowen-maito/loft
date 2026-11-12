import type { Agent } from "../../lib/api";
import { cx } from "../../lib/format";
import { RunTimer } from "../../components/RunTimer";

export function statusOf(a: Agent): { label: string; dot: string; text: string } {
  if (a.status === "waiting") return { label: "Needs you", dot: "bg-warn", text: "text-warn" };
  if (a.running && a.waiting_on?.length) return { label: "Waiting", dot: "bg-info animate-pulse-soft", text: "text-info" };
  if (a.running || a.status === "running") return { label: "Working", dot: "bg-accent animate-pulse-soft", text: "text-accent" };
  if (a.status === "error") return { label: "Error", dot: "bg-bad", text: "text-bad" };
  if (a.interval_minutes > 0) return { label: "Sleeping", dot: "bg-info/70", text: "text-fg-2" };
  return { label: "Idle", dot: "bg-fg-3", text: "text-fg-3" };
}

/** The state label, then the status text, or with `elapsed` the run time while running instead (not on a phone: the ✻ line shows it there). */
export function StatusPill({ agent, withText = true, elapsed = false }: { agent: Agent; withText?: boolean; elapsed?: boolean }) {
  const s = statusOf(agent);
  return (
    <span className={cx("flex min-w-0 items-center gap-1.5 font-mono text-[11px]", elapsed && "shrink-0")}>
      <span className={cx("size-1.5 shrink-0 rounded-full", s.dot)} />
      <span className={cx("shrink-0 uppercase tracking-wider", s.text)}>{s.label}</span>
      {elapsed
        ? agent.running && <span className="shrink-0 text-fg-3 max-md:hidden">· <RunTimer startedAt={agent.last_run_at} /></span>
        : withText && agent.status_text && <span className="truncate text-fg-3">· {agent.status_text}</span>}
    </span>
  );
}
