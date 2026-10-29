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
