/** The Issues page's spec views: which Delivery a spec has and where it stands. Pure, so the three views agree. */
import type { IssueDelivery, Task } from "./api";

/** `GET /api/deliveries/summary`: a spec's Delivery, with the spec it belongs to. */
export type DeliverySummary = Omit<IssueDelivery, "tickets" | "spec"> & {
  repo: string;
  spec: number;
  spec_id: string;
  local: boolean;
  tickets: { number: number; id?: string; title: string; state: string; skipped: string; verdicts?: Record<string, string> }[];
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

/* ---------- the board's tickets ---------- */

export type BoardTicket = { key: string; number: number | null; title: string; state: string; taskId: string | null; verdicts: Record<string, string>; current: boolean };

/** A spec's tickets: its Delivery's when it has them, else its sub-issues in Loft. Each knows its own issue, to open it. */
export function ticketsOf(spec: Task, d: DeliverySummary | null, tasks: Task[]): BoardTicket[] {
  if (d?.tickets.length)
    return d.tickets.map((t) => {
      const own = t.id ? tasks.find((x) => x.id === t.id) : tasks.find((x) => !!spec.github && x.github?.repo === spec.github.repo && x.github.number === t.number);
      return { key: `${t.number}`, number: own?.github?.number ?? t.number, title: t.title, state: t.state, taskId: own?.id ?? null, verdicts: t.verdicts ?? {}, current: d.current?.number === t.number };
    });
  return tasks
    .filter((x) => x.parent_id === spec.id && x.status !== "cancelled")
    .map((x) => ({ key: x.id, number: x.github?.number ?? null, title: x.title, state: x.status === "done" ? "closed" : x.status === "in_progress" ? "started" : ["blocked", "failed", "waiting_approval"].includes(x.status) ? "needs_human" : "open", taskId: x.id, verdicts: {}, current: false }));
}

/** The columns of a ticket: waiting, being built or in review, done. */
export type Lane = "open" | "doing" | "done";
export const LANES: { key: Lane; label: string }[] = [{ key: "open", label: "Open" }, { key: "doing", label: "In progress / review" }, { key: "done", label: "Done" }];
export const laneOf = (state: string): Lane => (state === "closed" ? "done" : ["started", "review", "closing", "needs_human"].includes(state) ? "doing" : "open");

/* ---------- the board's options, kept in the URL ---------- */

export type SortKey = "activity" | "progress";
export type BoardOptions = { stages: Stage[]; hideShipped: boolean; byProject: boolean; sort: SortKey; dense: boolean; needsMe: boolean };
export const DEFAULT_BOARD: BoardOptions = { stages: [], hideShipped: false, byProject: false, sort: "activity", dense: false, needsMe: false };

export function parseBoard(p: URLSearchParams): BoardOptions {
  const stages = (p.get("stage") ?? "").split(",").filter((s): s is Stage => STAGES.some((x) => x.key === s));
  return { stages, hideShipped: p.get("shipped") === "0", byProject: p.get("by") === "project", sort: p.get("sort") === "progress" ? "progress" : "activity", dense: p.get("dense") === "1", needsMe: p.get("needsme") === "1" };
}

/** `p` with the changed options written (and the defaults removed); everything else in it is kept. */
export function withBoard(p: URLSearchParams, change: Partial<BoardOptions>): URLSearchParams {
  const o = { ...parseBoard(p), ...change }, n = new URLSearchParams(p);
  const set = (k: string, v: string | null) => (v === null ? n.delete(k) : n.set(k, v));
  set("stage", o.stages.length ? o.stages.join(",") : null);
  set("shipped", o.hideShipped ? "0" : null);
  set("by", o.byProject ? "project" : null);
  set("sort", o.sort === "progress" ? "progress" : null);
  set("dense", o.dense ? "1" : null);
  set("needsme", o.needsMe ? "1" : null);
  return n;
}

/** Something the user must do on this spec: merge it, or a Delivery or ticket that needs a human. */
export const needsMe = (stage: Stage, d: DeliverySummary | null) => stage === "ready" || d?.state === "needs_human" || !!d?.tickets.some((t) => t.state === "needs_human");

/** The specs the options keep, in the order they ask for. */
export function arrange(specs: Task[], deliveries: DeliverySummary[], tasks: Task[], o: BoardOptions): Task[] {
  const info = (s: Task) => { const d = deliveryOf(s, deliveries); return { d, stage: stageOf(s, d) }; };
  const frac = (s: Task) => { const ts = ticketsOf(s, info(s).d, tasks); return ts.length ? ts.filter((t) => t.state === "closed").length / ts.length : 0; };
  return specs
    .filter((s) => {
      const { d, stage } = info(s);
      return (!o.stages.length || o.stages.includes(stage)) && !(o.hideShipped && stage === "shipped") && (!o.needsMe || needsMe(stage, d));
    })
    .sort((a, b) => (o.sort === "progress" ? frac(b) - frac(a) : 0) || b.updated_at.localeCompare(a.updated_at));
}
