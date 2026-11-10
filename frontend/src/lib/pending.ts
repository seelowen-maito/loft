import type { Agent, PermissionRequest } from "./api";

/** A pending permission request in the top bar: whose it is, what for, and the link to its card in that agent's chat. */
export type PendingItem = { id: string; agent: string; tool: string; href: string; title: string };

/** The pending requests, oldest first: the badge opens the first. An agent that's gone keeps the request, unnamed. */
export function pendingItems(perms: PermissionRequest[], agents: Pick<Agent, "id" | "name">[]): PendingItem[] {
  const names = new Map(agents.map((a) => [a.id, a.name]));
  return perms
    .filter((p) => p.status === "pending")
    .sort((a, b) => a.created_at.localeCompare(b.created_at))
    .map((p) => {
      const agent = names.get(p.agent_id) ?? "An agent";
      return { id: p.id, agent, tool: p.tool_name, href: `/agents/${p.agent_id}#perm-${p.id}`, title: `${agent} wants to use ${p.tool_name}` };
    });
}

/** The permission card a `#perm-<id>` link points at, inside the chat's transcript only (the Inspector shows a copy of it). */
export function permCard(transcript: ParentNode | null, hash: string): Element | null {
  if (!transcript || !hash.startsWith("#perm-")) return null;
  return transcript.querySelector(`[id="${hash.slice(1).replace(/["\\]/g, "\\$&")}"]`);
}
