import { taskText } from "./agentMessage.ts";

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

/** The footer's Reply label: it names who wrote the message. */
export function replyLabel(m: Quotable, agentName: string): string {
  const author = quoteOf(m, agentName).author;
  return author ? `Reply to ${author}` : "Reply to your message";
}

/** The box after Copy to box on a cancelled message: its text as the bubble shows it, after any draft already there. */
export function copiedToBox(draft: string, m: { content: string; meta: Record<string, any> | null }): string {
  const text = taskText(m);
  return draft.trim() ? `${draft}\n\n${text}` : text;
}

/** The DOM id of a message's row, which a quote jumps to. */
export const messageAnchor = (id: number) => `msg-${id}`;

/** What clicking a quote does: scroll to the original when it's shown, show the items before /clear when it may be
 * among them, else say it's gone (older than what the chat loaded). */
export const jumpStep = (shown: boolean, hiddenEarlier: number): "flash" | "show-earlier" | "gone" =>
  shown ? "flash" : hiddenEarlier > 0 ? "show-earlier" : "gone";

export const QUOTE_GONE_NOTE = "That message is no longer in the chat.";

/** The server's refusal of a reply whose quoted message is gone (a draft kept across a reload): the quote is dropped. */
export const quoteGone = (error: string) => error === "The quoted message isn't in this chat.";
