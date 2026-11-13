import type { Agent, Task } from "./api";

type Teamed = Pick<Agent, "project"> & Partial<Pick<Agent, "group" | "projects">>;

/** The team an agent is in: its group ("" for none). A project alone makes no team. Mirrors `Agent::team` in the backend. */
export const teamOf = (a: Teamed) => a.group?.trim() || "";

/** The projects an agent works in, main first. A server without groups only sends `project`. */
export const projectsOf = (a: Teamed) => (a.projects?.length ? a.projects : [a.project?.trim() ?? ""].filter(Boolean));

/** The header badge: its group, main project and "+N" for the others; `compact` is the short form for a phone; `all` lists every project, main first. */
export function teamBadge(a: Teamed): { text: string; compact: string; all: string[] } {
  const all = projectsOf(a);
  const main = all[0] && all[0] !== a.group?.trim() ? all[0] : "";
  const more = all.length > 1 ? `+${all.length - 1}` : "";
  const text = [a.group?.trim(), main, more].filter(Boolean).join(" · ");
  return { text, compact: more, all };
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

/** `form` with the project name still typed in the add field taken in, so saving doesn't drop it. */
export function withTyped<T extends { projects: string[] }>(form: T, typed: string): T {
  return { ...form, projects: withProject(form.projects, typed) };
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

type GroupAgent = Pick<Agent, "id" | "project"> & Partial<Pick<Agent, "group" | "projects">>;

/** Whether an issue is in a group. A group is only a label on agents: its issues are those of the projects
 * its agents work in, and those assigned to one of them. */
export function inGroup(t: Pick<Task, "project" | "agent_id">, group: string, agents: GroupAgent[]): boolean {
  return agents.some((a) => teamOf(a) === group && ((!!t.project && projectsOf(a).includes(t.project)) || a.id === t.agent_id));
}

/** The projects of a group's agents, or every project in `known` without a group. */
export function groupProjects(group: string, agents: GroupAgent[], known: string[]): string[] {
  if (!group) return known;
  const mine = new Set(agents.filter((a) => teamOf(a) === group).flatMap(projectsOf));
  return known.filter((p) => mine.has(p));
}

/** The agents of a group (by their group label), or every agent without a group. */
export function groupAgents<A extends GroupAgent>(group: string, agents: A[]): A[] {
  return group ? agents.filter((a) => teamOf(a) === group) : agents;
}

/** The agent filter after switching to `group`: kept when the agent is in it, otherwise "" (all agents). */
export function agentInGroup(agentId: string, group: string, agents: GroupAgent[]): string {
  return agentId && groupAgents(group, agents).some((a) => a.id === agentId) ? agentId : "";
}

/** The order of every agent picker for an issue: those working in its `project` first, Devs first within each
 *  part, then by name. The first one is the picker's default, so pressing Start goes to an agent that builds. */
export function issuePickerOrder<T extends Teamed & Pick<Agent, "name"> & Partial<Pick<Agent, "role">>>(agents: T[], project = ""): T[] {
  const p = project.trim();
  const rank = (a: T) => (p && projectsOf(a).includes(p) ? 0 : 2) + (a.role === "dev" ? 0 : 1);
  return [...agents].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}
