import type { IssueDelivery } from "./api";

/** The tone of a Delivery card's state line: running, waiting for CI, stalled, paused, needs you, final review or done. */
export type DeliveryTone = "running" | "ci" | "stalled" | "paused" | "needs_you" | "final" | "done";

const TONES: DeliveryTone[] = ["running", "ci", "stalled", "paused", "needs_you", "final", "done"];

/** How long something has waited: "<1 min", "6 min", "1 h", "2 h 5 min". Negative spans (clock skew) read as "<1 min". */
export function fmtWait(ms: number): string {
  const m = Math.floor(Math.max(0, ms) / 60000);
  if (m < 1) return "<1 min";
  if (m < 60) return `${m} min`;
  return m % 60 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${m / 60} h`;
}

/** Why a Delivery stalled, as the card says it (only for an older server that sends no `state_line`). */
function stallWhy(reason: string | null | undefined): string {
  switch (reason) {
    case "run_failed":
      return "the run Loft woke for its step failed";
    case "restart":
      return "its run was lost when Loft restarted";
    default:
      return "the run Loft woke for its step ended without its call";
  }
}

/** A Delivery card's state line, or null before it started: the server's `state_line` as is (a tone it names
 *  that this build doesn't know shows as running). Waiting for CI adds how long since the server's `ci_since`
 *  (its earliest ticket waiting, at `now`), and links to the Checks tab on GitHub. An older server sends none, so it's worded here:
 *  waiting for the user (a ticket after too many changes rounds, or the final review) wins over running. */
export function deliveryStateLine(
  d: Pick<IssueDelivery, "state" | "started" | "stall_reason" | "needs_human" | "state_line" | "checks_url" | "ci_since"> | null,
  now = Date.now(),
): { tone: DeliveryTone; text: string; href?: string } | null {
  if (!d || !d.started) return null;
  if (d.state_line !== undefined) {
    if (!d.state_line) return null;
    const tone = TONES.find((t) => t === d.state_line?.tone) ?? "running";
    if (tone !== "ci") return { tone, text: d.state_line.text };
    const since = d.ci_since ? Date.parse(d.ci_since) : NaN;
    return { tone, text: Number.isNaN(since) ? d.state_line.text : `${d.state_line.text} · ${fmtWait(now - since)}`, ...(d.checks_url ? { href: d.checks_url } : {}) };
  }
  switch (d.state) {
    case "stalled":
      return { tone: "stalled", text: `Stalled: ${stallWhy(d.stall_reason)}.` };
    case "paused":
      return { tone: "paused", text: "Paused: Loft sends no wakes until you resume it." };
    case "needs_human":
      return { tone: "needs_you", text: "Needs you: too many rounds of changes in the final review." };
    case "final_review":
      return { tone: "final", text: "Final review: the Code reviewer is reviewing the whole PR." };
    case "final_fixes":
      return { tone: "final", text: "Final review: the Dev is fixing what the Code reviewer found." };
    case "ready":
    case "settled":
    case "merged":
      return { tone: "done", text: "Done: every review passed." };
  }
  if (d.needs_human) return { tone: "needs_you", text: "Needs you: a ticket had too many rounds of changes." };
  return { tone: "running", text: "Running." };
}

/** The buttons a started Delivery's card shows, each only when the server says it applies: Pause while it runs
 *  (final review included), Resume when it's stalled or paused, or a ticket or final review waits for the user,
 *  and Reopen delivery when it's done and tickets published since are open. */
export function deliveryButtons(d: Pick<IssueDelivery, "started" | "can_pause" | "can_resume" | "can_reopen"> | null): { pause: boolean; resume: boolean; reopen: boolean } {
  const started = !!d?.started;
  return { pause: started && !!d?.can_pause, resume: started && !!d?.can_resume, reopen: started && !!d?.can_reopen };
}
