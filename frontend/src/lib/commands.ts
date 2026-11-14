import type { AgentCommand } from "./api";

/** The commands matching the `/word` being typed: names that start with it first, then names that contain it,
 * then descriptions that do. A bare `/` lists them all. */
export function filterCommands(list: AgentCommand[], word: string): AgentCommand[] {
  const q = word.replace(/^\//, "").toLowerCase();
  const names = list.map((c) => c.name.slice(1).toLowerCase());
  const at = (i: number) => (names[i].startsWith(q) ? 0 : names[i].includes(q) ? 1 : list[i].description.toLowerCase().includes(q) ? 2 : 3);
  return list
    .map((c, i) => ({ c, rank: at(i) }))
    .filter((x) => x.rank < 3)
    .sort((a, b) => a.rank - b.rank)
    .map((x) => x.c);
}

/** What a command that can't run here says when picked. */
export function terminalNote(c: AgentCommand): string {
  return `${c.name} can't run here. ${c.reason ?? "Claude Code runs it only in its own terminal app."}`;
}
