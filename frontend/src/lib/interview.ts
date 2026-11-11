import type { GrillSession } from "./api";

/** The full-screen interview of an agent's open grill, at its own URL. */
export const interviewPath = (agentId: string) => `/agents/${agentId}/interview`;

/** What the full-screen interview shows: the open round's questions, a wait for the next one, or no interview. */
export function interviewShows(s: GrillSession | null): "round" | "waiting" | "none" {
  if (!s || s.state === "ended") return "none";
  return s.round != null && !s.waiting ? "round" : "waiting";
}

/** Where the open round is: its first unanswered question (1-based `n` of `total`), or null when every one has an answer. */
export function nextQuestion(s: GrillSession | null): { n: number; total: number; id: string } | null {
  const qs = interviewShows(s) === "round" ? (s!.questions ?? []) : [];
  const i = qs.findIndex((q) => q.state === "open");
  return i < 0 ? null : { n: i + 1, total: qs.length, id: qs[i].id };
}

/** The attribute that marks an open round's question card, so the round bar can find it. */
export const questionAnchor = (session: string, round: number, question: string) => `${session}:${round}:${question}`;
