/** The Issues page's spec views: which Delivery a spec has and where it stands. Pure, so the three views agree. */
import type { IssueDelivery, Task } from "./api";

/** `GET /api/deliveries/summary`: a spec's Delivery, with the spec it belongs to. */
export type DeliverySummary = Omit<IssueDelivery, "tickets" | "spec"> & {
  repo: string;
  spec: number;
  spec_id: string;
  local: boolean;
  tickets: { number: number; title: string; state: string; skipped: string; verdicts?: Record<string, string> }[];
};

export type Stage = "planned" | "delivery" | "ready" | "shipped";
export const STAGES: { key: Stage; label: string }[] = [
  { key: "planned", label: "Planned" },
  { key: "delivery", label: "In Delivery" },
  { key: "ready", label: "Ready to merge" },
  { key: "shipped", label: "Shipped" },
];

/** The Delivery of a spec's issue: a GitHub spec by repo and number, a plan kept in Loft by its issue. */
export function deliveryOf(spec: Task, all: DeliverySummary[]): DeliverySummary | null {
  return all.find((d) => (d.local ? d.spec_id === spec.id : !!spec.github && d.repo === spec.github.repo && d.spec === spec.github.number)) ?? null;
}

/** Where a spec stands: no Delivery or not started is Planned; Loft marked its PR ready (or found it so) and the issue is still open is Ready to merge; merged or closed is Shipped; the rest is In Delivery. */
export function stageOf(spec: Task, d: DeliverySummary | null): Stage {
  if (d?.state === "merged" || spec.state === "closed") return "shipped";
  if (!d || d.state === "planned") return "planned";
  if (d.state === "ready" || d.state === "settled") return "ready";
  return "delivery";
}

/** A ticket's state in a few words and a tone. */
export function ticketLabel(state: string): { text: string; tone: "good" | "accent" | "warn" | "muted" } {
  switch (state) {
    case "closed": return { text: "done", tone: "good" };
    case "started": return { text: "building", tone: "accent" };
    case "review": return { text: "in review", tone: "accent" };
    case "closing": return { text: "closing", tone: "accent" };
    case "needs_human": return { text: "needs you", tone: "warn" };
    case "elsewhere": return { text: "elsewhere", tone: "muted" };
    default: return { text: "open", tone: "muted" };
  }
}
