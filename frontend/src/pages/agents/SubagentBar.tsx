import { Bot } from "lucide-react";
import type { Agent } from "../../lib/api";

/** Claude Code's cap when the agent's "Subagents at once" is Default. */
export const DEFAULT_MAX_SUBAGENTS = 20;

/** The bar above the composer while the agent's run has subagents running: how many, against its limit. */
export function SubagentBar({ agent }: { agent: Pick<Agent, "subagents" | "max_subagents"> }) {
  const n = agent.subagents?.length ?? 0;
  if (!n) return null;
  return (
    <div className="mb-3 flex items-center gap-3 rounded-xl border border-accent/25 bg-accent/[0.05] px-3 py-2" role="status">
      <Bot className="size-4 shrink-0 text-accent" />
      <div className="min-w-0 flex-1 truncate text-[13px] text-fg">
        {n} of {agent.max_subagents ?? DEFAULT_MAX_SUBAGENTS} subagents
      </div>
    </div>
  );
}
