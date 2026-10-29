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
