import type { Agent } from "./api";

/** "@ho" at the end of the text: what's typed after the @, else undefined. */
export const atQuery = (text: string) => /(?:^|[\s([{"'])@([^\s@]*)$/.exec(text)?.[1];

export type MentionGroup = { title: string; agents: Agent[] };

/** Agents matching `query` (case-insensitive, at the start of the name or of one of its words): `project`'s first, then the rest. */
export function mentionGroups(agents: Agent[], project: string, query: string): MentionGroup[] {
  const q = query.toLowerCase();
  const hit = (a: Agent) => a.name.toLowerCase().split(/\s+/).some((w) => w.startsWith(q));
  const mine = agents.filter((a) => a.project.trim() === project.trim() && hit(a));
  const rest = agents.filter((a) => a.project.trim() !== project.trim() && hit(a));
  return [
    { title: project.trim() || "No project", agents: mine },
    { title: "Other agents", agents: rest },
  ].filter((g) => g.agents.length > 0);
}

/** Replaces the @… at the end of `text` with a mention of `agent`, in the form the backend resolves (the full name). */
export const insertMention = (text: string, agent: Agent) => text.replace(/@[^\s@]*$/, `@${agent.name} `);

/** The agents `text` @mentions, as the backend resolves them: longest name first, at the start of a word, ending at a word boundary (not "@Web Developer", not a path like "@qa/x"). */
export function mentionedIn(text: string, agents: Agent[]): Agent[] {
  const esc = (n: string) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  let rest = text;
  const found: Agent[] = [];
  for (const a of [...agents].filter((a) => a.name.trim()).sort((x, y) => y.name.length - x.name.length)) {
    const re = new RegExp(`(?<=^|[\\s([{"'])@${esc(a.name)}(?![\\w-]|/|\\.\\w)`, "gi");
    if (re.test(rest)) found.push(a);
    rest = rest.replace(re, (m) => " ".repeat(m.length)); // a longer name has used its @ already
  }
  return agents.filter((a) => found.includes(a));
}
