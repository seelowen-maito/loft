/** Replying to a message: the composer's quote, and the one a sent message carries in `meta.reply_to`. */

/** A quoted message: its id, who wrote it (null for the user) and a one-line excerpt. */
export type Quote = { id: number; author: string | null; excerpt: string };

const EXCERPT_CHARS = 200;

type Quotable = { id: number; role: string; content: string; meta: Record<string, any> | null };

/** Only chat messages can be quoted (as the server rules): text from the user, another agent or the agent. */
export const quotable = (m: { role: string; kind: string }) => m.kind === "text" && (m.role === "user" || m.role === "assistant");

/** The quote of `m` in `agentName`'s chat, as the composer shows it before sending. */
export function quoteOf(m: Quotable, agentName: string): Quote {
  const author = m.meta?.from_agent?.name ?? (m.role === "user" ? null : agentName);
  const line = m.content.split(/\s+/).filter(Boolean).join(" ");
  const chars = [...line];
  const excerpt = chars.length > EXCERPT_CHARS ? `${chars.slice(0, EXCERPT_CHARS).join("")}…` : line;
  return { id: m.id, author, excerpt };
}

/** The quote a sent message carries, if it's a reply. */
export function replyOf(meta: Record<string, any> | null | undefined): Quote | null {
  const r = meta?.reply_to;
  if (typeof r?.id !== "number") return null;
  return { id: r.id, author: typeof r.author === "string" ? r.author : null, excerpt: typeof r.excerpt === "string" ? r.excerpt : "" };
}

/** A quote's author as shown: the user is "You". */
export const authorLabel = (q: Quote) => q.author ?? "You";

/** The DOM id of a message's row, which a quote jumps to. */
export const messageAnchor = (id: number) => `msg-${id}`;
