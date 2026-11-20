import { useCommands } from "../../lib/useCommands";
import { terminalNote } from "../../lib/commands";
import { cx } from "../../lib/format";
import type { Agent } from "../../lib/api";

const TAG: Record<string, string> = { loft: "Loft", passthrough: "Claude Code", terminal: "terminal only" };

/** Every command the agent's CLI offers, and Loft's own, with how each runs. */
export function CommandsPanel({ agent }: { agent: Agent }) {
  const commands = useCommands(agent.id, !!agent.running);
  return (
    <div className="space-y-1 p-3">
      <p className="px-1 pb-2 text-[12px] text-fg-3">Type / in the chat to run one. The list refreshes with every run; your own commands and skills from ~/.claude are in it.</p>
      {commands.map((c) => (
        <div key={c.name} title={c.tag === "terminal" ? terminalNote(c) : undefined} className={cx("rounded-lg px-2 py-1.5", c.tag === "terminal" && "opacity-50")}>
          <div className="flex items-baseline gap-2">
            <span className="font-mono text-[12.5px] text-accent">{c.name}</span>
            <span className="ml-auto shrink-0 text-[11px] text-fg-3">{TAG[c.tag]}</span>
          </div>
          {(c.description || c.reason) && <div className="text-[12px] text-fg-3">{c.tag === "terminal" ? c.reason : c.description}</div>}
        </div>
      ))}
    </div>
  );
}
