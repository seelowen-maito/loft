import { useEffect, useState } from "react";
import { fmtDone, fmtElapsed, runElapsed } from "../lib/runTimer";
import { cx } from "../lib/format";

/** Time since `startedAt` (the agent's `last_run_at`), ticking every second. Only this span re-renders. */
export function RunTimer({ startedAt, className }: { startedAt: string | null | undefined; className?: string }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [startedAt]);
  return <span className={cx("tabular-nums", className)}>{fmtElapsed(runElapsed(startedAt, now))}</span>;
}

/** The chat's run line: `✻ 2m 23s · Reading CI logs` while running, `✻ Done in 2m 23s · Thu 16:23` after. */
export function RunIndicator(
  props: { running: true; startedAt: string | null | undefined; status?: string } | { running: false; durationMs: number; endedAt: string },
) {
  return (
    <div className="flex min-w-0 items-center gap-2 py-2 pl-1 font-mono text-[12px] text-fg-2" role="status">
      <span className={cx("inline-block shrink-0 text-[15px] leading-none text-accent", props.running && "motion-safe:animate-star")}>✻</span>
      {props.running ? (
        <span className="truncate">
          <RunTimer startedAt={props.startedAt} />
          {props.status && <span> · {props.status}</span>}
        </span>
      ) : (
        <span className="truncate text-fg-3">{fmtDone(props.durationMs, props.endedAt)}</span>
      )}
    </div>
  );
}
