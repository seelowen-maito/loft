// One wording and colour for a task's status, and for "waiting on the user" everywhere.

export type Tone = "good" | "accent" | "info" | "warn" | "bad" | "muted";

/** What the app calls anything waiting on the user: a task, a ticket, an agent, a Deliveries card, a Needs you item. */
export const NEEDS_YOU_LABEL = "Needs you";

const STATUS: Record<string, { label: string; tone: Tone }> = {
  waiting_approval: { label: NEEDS_YOU_LABEL, tone: "warn" },
  blocked: { label: "Blocked", tone: "warn" },
  failed: { label: "Failed", tone: "bad" },
  in_progress: { label: "In progress", tone: "accent" },
  ready: { label: "Ready", tone: "info" },
  inbox: { label: "Inbox", tone: "muted" },
  done: { label: "Done", tone: "good" },
  cancelled: { label: "Cancelled", tone: "muted" },
};

/** A task status's label and tone. An unknown one reads as its own words, muted. */
export function taskStatus(s: string | null | undefined): { label: string; tone: Tone } {
  return STATUS[s ?? ""] ?? { label: (s ?? "").replace("_", " "), tone: "muted" };
}

const TEXT: Record<Tone, string> = {
  good: "text-good",
  accent: "text-accent",
  info: "text-info",
  warn: "text-warn",
  bad: "text-bad",
  muted: "text-fg-3",
};

/** The text colour class of a tone (a status icon, dot or label). */
export const toneText = (t: Tone): string => TEXT[t];
