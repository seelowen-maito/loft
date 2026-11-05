import type { Agent } from "./api";

type Teamed = Pick<Agent, "project"> & Partial<Pick<Agent, "group" | "projects">>;

/** The team an agent is in: its group, else its main project ("" for none). Mirrors `Agent::team` in the backend. */
export const teamOf = (a: Teamed) => a.group?.trim() || a.project?.trim() || "";

/** The projects an agent works in, main first. A server without groups only sends `project`. */
export const projectsOf = (a: Teamed) => (a.projects?.length ? a.projects : [a.project?.trim() ?? ""].filter(Boolean));

/** The header badge: its group, main project and "+N" for the others; `compact` is the short form for a phone; `all` lists every project, main first. */
export function teamBadge(a: Teamed): { text: string; compact: string; all: string[] } {
  const all = projectsOf(a);
  const main = all[0] && all[0] !== a.group?.trim() ? all[0] : "";
  const more = all.length > 1 ? `+${all.length - 1}` : "";
  const text = [a.group?.trim(), main, more].filter(Boolean).join(" · ");
  return { text, compact: more || text, all };
}

/** Agents grouped by team, named teams A–Z, then those without one. */
export function groupByTeam<T extends Teamed>(agents: T[]): { team: string; agents: T[] }[] {
  const m = new Map<string, T[]>();
  agents.forEach((a) => m.set(teamOf(a), [...(m.get(teamOf(a)) ?? []), a]));
  return [...m.entries()]
    .sort(([a], [b]) => (!a ? 1 : !b ? -1 : a.localeCompare(b, undefined, { sensitivity: "base" })))
    .map(([team, agents]) => ({ team, agents }));
}

/** `projects` with `p` added at the end, or made the main one (first) when `main`; each once. */
export function withProject(projects: string[], p: string, main = false): string[] {
  const name = p.trim();
  if (!name || (!main && projects.includes(name))) return projects;
  const rest = projects.filter((x) => x !== name);
  return main ? [name, ...rest] : [...rest, name];
}

/** The projects a new issue for `agent` can go to: its own (main first), then every other one known, dimmed in the form. */
export function projectChoices(agent: Teamed | undefined, known: string[]): { mine: string[]; others: string[] } {
  const mine = agent ? projectsOf(agent) : [];
  const others = [...new Set(known.map((p) => p.trim()).filter((p) => p && !mine.includes(p)))].sort((a, b) => a.localeCompare(b));
  return { mine, others };
}

/** The warning for giving `agent` an issue in `project` it doesn't work in; null when it does, or has no projects. */
export function outsideWarning(agent: (Teamed & { name: string }) | undefined, project: string): string | null {
  const p = project.trim();
  const mine = agent ? projectsOf(agent) : [];
  if (!agent || !p || !mine.length || mine.includes(p)) return null;
  return `${agent.name} doesn't work in ${p} (its projects: ${mine.join(", ")}). You can still create it; the issue's thread will say so.`;
}
