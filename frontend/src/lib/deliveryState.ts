import type { IssueDelivery } from "./api";
import { fmtDuration } from "./time.ts";

/** The tone of a Delivery card's state line: running, waiting for CI, stalled, paused, needs you, final review or done. */
export type DeliveryTone = "running" | "ci" | "stalled" | "paused" | "needs_you" | "final" | "done";

const TONES: DeliveryTone[] = ["running", "ci", "stalled", "paused", "needs_you", "final", "done"];

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
 *  (its earliest ticket waiting, at `now`), and links to the Checks tab on GitHub; stalled by a Billing block, it links
 *  to the owner's billing page. An older server sends none, so it's worded here:
 *  waiting for the user (a ticket after too many changes rounds, or the final review) wins over running. */
export function deliveryStateLine(
  d: Pick<IssueDelivery, "state" | "started" | "stall_reason" | "needs_human" | "state_line" | "checks_url" | "ci_since" | "billing_url"> | null,
  now = Date.now(),
): { tone: DeliveryTone; text: string; href?: string } | null {
  if (!d || !d.started) return null;
  if (d.state_line !== undefined) {
    if (!d.state_line) return null;
    const tone = TONES.find((t) => t === d.state_line?.tone) ?? "running";
    if (tone === "stalled" && d.billing_url) return { tone, text: d.state_line.text, href: d.billing_url };
    if (tone !== "ci") return { tone, text: d.state_line.text };
    const since = d.ci_since ? Date.parse(d.ci_since) : NaN;
    return { tone, text: Number.isNaN(since) ? d.state_line.text : `${d.state_line.text} · ${fmtDuration(now - since)}`, ...(d.checks_url ? { href: d.checks_url } : {}) };
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

/** The card's line of GitHub Actions minutes its branch spent so far (the total once it's done), or null when the
 *  server sends none (before it started, a plan kept in Loft, an older server). */
export function actionsMinutesLine(d: Pick<IssueDelivery, "state" | "started" | "actions_minutes"> | null): string | null {
  if (!d?.started || typeof d.actions_minutes !== "number") return null;
  const done = ["ready", "settled", "merged"].includes(d.state);
  return `GitHub Actions: ${d.actions_minutes} min${done ? "" : " so far"}`;
}

/** The buttons a started Delivery's card shows, each only when the server says it applies: Pause while it runs
 *  (final review included), Resume when it's stalled or paused, or a ticket or final review waits for the user,
 *  and Reopen delivery when it's done and tickets published since are open. */
export function deliveryButtons(d: Pick<IssueDelivery, "started" | "can_pause" | "can_resume" | "can_reopen"> | null): { pause: boolean; resume: boolean; reopen: boolean } {
  const started = !!d?.started;
  return { pause: started && !!d?.can_pause, resume: started && !!d?.can_resume, reopen: started && !!d?.can_reopen };
}
