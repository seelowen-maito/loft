// Agent messages in a chat: who sent them and whether they woke the agent (#112).

type Msg = { role: string; meta?: { from_agent?: unknown; fyi?: unknown } | null };

/** An FYI from another agent: shown at once, read on the agent's next Run, without waking it. */
export function isFyi(m: Msg): boolean {
  return m.role === "user" && !!m.meta?.from_agent && m.meta.fyi === true;
}

/** What the FYI mark explains on hover or tap. */
export function fyiTitle(agentName: string): string {
  return `FYI: delivered without waking ${agentName}. It reads this at the start of its next run.`;
}

/** What a `message_agent` call shows under the message once it was queued behind the recipient's run (#185):
 * queued while any agent still holds the message id in `queued_messages`, then delivered. Null for any other
 * result: a message to an idle agent, an FYI or an error. */
export function sendState(result: string | undefined, agents: { queued_messages?: number[] }[]): string | null {
  const q = /^Queued: (.+) is busy and will read it when its current run ends \(message (\d+)\)\.$/.exec(result ?? "");
  if (!q) return null;
  const id = Number(q[2]);
  return agents.some((a) => a.queued_messages?.includes(id)) ? `Queued: ${q[1]} is busy` : "Delivered";
}

/** The mark on an agent message still waiting for the recipient's current run to end. */
export const QUEUED_MARK = "Queued until this run ends";
