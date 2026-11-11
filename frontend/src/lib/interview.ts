import type { GrillSession } from "./api";

/** The full-screen interview of an agent's open grill, at its own URL. */
export const interviewPath = (agentId: string) => `/agents/${agentId}/interview`;

/** What the full-screen interview shows: the open round's questions, a wait for the next one, or no interview. */
export function interviewShows(s: GrillSession | null): "round" | "waiting" | "none" {
  if (!s || s.state === "ended") return "none";
  return s.round != null && !s.waiting ? "round" : "waiting";
}
