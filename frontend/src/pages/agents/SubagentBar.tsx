import { useState } from "react";
import { Bot, ChevronDown } from "lucide-react";
import type { Agent } from "../../lib/api";
import { cx } from "../../lib/format";
import { RunTimer } from "../../components/RunTimer";

/** Claude Code's cap when the agent's "Subagents at once" is Default. */
export const DEFAULT_MAX_SUBAGENTS = 20;

/** Past this many, the bar shows only the count until tapped. */
export const SHOWN_SUBAGENTS = 3;

/** The bar above the composer while the agent's run has subagents running: how many, against its limit,
 * and one line each with description, type and running time, like Claude Code's agent list. Read-only. */
export function SubagentBar({ agent }: { agent: Pick<Agent, "subagents" | "max_subagents"> }) {
  const [open, setOpen] = useState(false);
  const subs = agent.subagents ?? [];
  if (!subs.length) return null;
  const collapsible = subs.length > SHOWN_SUBAGENTS;
  const shown = !collapsible || open;
  const count = `${subs.length} of ${agent.max_subagents ?? DEFAULT_MAX_SUBAGENTS} subagents`;
  return (
    <div className="mb-3 rounded-xl border border-accent/25 bg-accent/[0.05] px-3 py-2" role="status">
      {collapsible ? (
        <button className="flex w-full items-center gap-3 text-left" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
          <Bot className="size-4 shrink-0 text-accent" />
          <span className="min-w-0 flex-1 truncate text-[13px] text-fg">{count}</span>
          <ChevronDown className={cx("size-4 shrink-0 text-fg-3 transition", open && "rotate-180")} />
        </button>
      ) : (
        <div className="flex items-center gap-3">
          <Bot className="size-4 shrink-0 text-accent" />
          <span className="min-w-0 flex-1 truncate text-[13px] text-fg">{count}</span>
        </div>
      )}
      {shown && (
        <ul className="mt-1.5 space-y-1 pl-7">
          {subs.map((s) => (
            <li key={s.id} className="flex items-baseline gap-2 font-mono text-[11.5px]">
              <span className="min-w-0 flex-1 truncate text-fg-2">{s.description || "Subagent"}</span>
              {s.type && <span className="shrink-0 text-fg-3">{s.type}</span>}
              <RunTimer startedAt={s.started_at} className="shrink-0 text-fg-3" />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
