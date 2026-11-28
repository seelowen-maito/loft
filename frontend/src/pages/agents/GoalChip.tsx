import { Target, X } from "lucide-react";
import { api, type Agent } from "../../lib/api";
import { useAction } from "../../lib/useAction";

/** The chat header's Goal active chip while the agent's Session has a `/goal`: its condition on hover, and a button
 * that sends `/goal clear` (into the run in progress when there is one). */
export function GoalChip({ agent }: { agent: Pick<Agent, "id" | "goal"> }) {
  const clear = useAction();
  if (!agent.goal) return null;
  return (
    <span className="flex shrink-0 items-center rounded-full border border-accent/40 bg-accent/[0.08] font-mono text-[10.5px] text-accent" title={clear.error ?? `Goal: ${agent.goal}`}>
      {/* On a phone the icon stands for it: the header has no room for the words. */}
      <span className="flex items-center gap-1 py-0.5 pl-2 pr-1" aria-label="Goal active">
        <Target className="size-3" aria-hidden /> <span className="max-sm:hidden">Goal active</span>
      </span>
      <button
        type="button"
        className="grid size-5 place-items-center rounded-full hover:bg-accent/[0.16] disabled:opacity-50"
        aria-label="Clear the goal"
        title="Clear the goal: sends /goal clear"
        disabled={clear.busy}
        onClick={() => void clear.run(() => api.clearGoal(agent.id))}
      >
        <X className="size-3" aria-hidden />
      </button>
    </span>
  );
}
