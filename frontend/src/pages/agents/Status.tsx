import type { Agent } from "../../lib/api";
import { statusOf } from "../../lib/agentStatus";
import { cx } from "../../lib/format";
import { RunTimer } from "../../components/RunTimer";

export { statusOf };

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
