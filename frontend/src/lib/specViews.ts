/** The Issues page's spec views: which Delivery a spec has and where it stands. Pure, so the board, its URL options and a spec's own page agree. */
import type { IssueDelivery, Task, TaskStatus } from "./api";

/** `GET /api/deliveries/summary`: a spec's Delivery, with the spec it belongs to. */
export type DeliverySummary = Omit<IssueDelivery, "tickets" | "spec"> & {
  repo: string;
  spec: number;
  spec_id: string;
  local: boolean;
  tickets: { number: number; id?: string; title: string; state: string; skipped: string; waiting_on?: string[]; agent_id?: string | null; agent_name?: string | null; waiting?: string | null; verdicts?: Record<string, string> }[];
};

export type Stage = "planned" | "delivery" | "ready" | "shipped" | "dropped";
export const STAGES: { key: Stage; label: string }[] = [
  { key: "planned", label: "Planned" },
  { key: "delivery", label: "In Delivery" },
  { key: "ready", label: "Ready to merge" },
  { key: "shipped", label: "Shipped" },
];
/** Closed as not planned: never shown as Shipped, and out of the board unless asked for. */
export const DROPPED = { key: "dropped" as Stage, label: "Dropped" };

/** Where a spec comes from, so a spec mirrored from both a fork and its upstream reads as two: `owner/repo#12`, or the Loft project of a plan kept in Loft. */
export const specRef = (spec: Task) => (spec.github ? `${spec.github.repo}#${spec.github.number}` : `Loft${spec.project ? ` · ${spec.project}` : ""}`);

/** What a spec card's meta line says: where the spec lives, then its project, unless the ref already names it (a plan kept in Loft). */
export const specMeta = (spec: Task): { ref: string; project: string | null } => ({ ref: specRef(spec), project: spec.github && spec.project ? spec.project : null });

/** The Delivery of a spec's issue: a GitHub spec by repo and number, a plan kept in Loft by its issue. */
export function deliveryOf(spec: Task, all: DeliverySummary[]): DeliverySummary | null {
  return all.find((d) => (d.local ? d.spec_id === spec.id : !!spec.github && d.repo === spec.github.repo && d.spec === spec.github.number)) ?? null;
}

/** Where a spec stands: no Delivery or not started is Planned; Loft marked its PR ready (or found it so) and the issue is still open is Ready to merge; merged, or closed as completed, is Shipped; closed as not planned is Dropped; the rest is In Delivery. */
export function stageOf(spec: Task, d: DeliverySummary | null): Stage {
  if (d?.state === "merged") return "shipped";
  if (spec.state === "closed") return spec.status === "cancelled" ? "dropped" : "shipped";
  if (!d || d.state === "planned") return "planned";
  if (d.state === "ready" || d.state === "settled") return "ready";
  return "delivery";
}

/** A ticket's state in a few words and a tone. An open one waiting on tickets outside its spec says which (`owner/repo#n`). */
export function ticketLabel(state: string, waitingOn: string[] = []): { text: string; tone: "good" | "accent" | "warn" | "muted" } {
  if (state === "open" && waitingOn.length) return { text: `waiting on ${waitingOn.join(", ")}`, tone: "warn" };
  switch (state) {
    case "closed": return { text: "done", tone: "good" };
    case "started": return { text: "building", tone: "accent" };
    case "review": return { text: "in review", tone: "accent" };
    case "closing": return { text: "closing", tone: "accent" };
    case "needs_human": return { text: "Waiting on you", tone: "warn" };
    case "elsewhere": return { text: "elsewhere", tone: "muted" };
    default: return { text: "open", tone: "muted" };
  }
}

/* ---------- the board's tickets ---------- */

export type BoardTicket = { key: string; number: number | null; title: string; state: string; taskId: string | null; verdicts: Record<string, string>; current: boolean; agentId: string | null; waitingOn?: string[] };

/** A spec's tickets: its Delivery's when it has them, else its sub-issues in Loft. Each knows its own issue, to open it. */
export function ticketsOf(spec: Task, d: DeliverySummary | null, tasks: Task[]): BoardTicket[] {
  if (d?.tickets.length)
    return d.tickets.map((t) => {
      const own = t.id ? tasks.find((x) => x.id === t.id) : tasks.find((x) => !!spec.github && x.github?.repo === spec.github.repo && x.github.number === t.number);
      return { key: `${t.number}`, number: own?.github?.number ?? t.number, title: t.title, state: t.state, taskId: own?.id ?? null, verdicts: t.verdicts ?? {}, current: !!t.agent_id, agentId: t.agent_id ?? null, waitingOn: t.waiting_on };
    });
  return tasks
    .filter((x) => x.parent_id === spec.id && x.status !== "cancelled")
    .map((x) => ({ key: x.id, number: x.github?.number ?? null, title: x.title, state: x.status === "done" ? "closed" : x.status === "in_progress" ? "started" : ["blocked", "failed", "waiting_approval"].includes(x.status) ? "needs_human" : "open", taskId: x.id, verdicts: {}, current: false, agentId: null }));
}

/* ---------- the board's options, kept in the URL ---------- */

export type SortKey = "activity" | "progress";
/** Which columns show: the Issues column, the spec stages, or both. */
export type BoardType = "all" | "issues" | "specs";
export type BoardOptions = { type: BoardType; stages: Stage[]; hideShipped: boolean; showDropped: boolean; byProject: boolean; sort: SortKey; dense: boolean; needsMe: boolean };

export function parseBoard(p: URLSearchParams): BoardOptions {
  const stages = (p.get("stage") ?? "").split(",").filter((s): s is Stage => STAGES.some((x) => x.key === s));
  const type = p.get("type");
  return { type: type === "issues" || type === "specs" ? type : "all", stages, hideShipped: p.get("shipped") === "0", showDropped: p.get("dropped") === "1", byProject: p.get("by") === "project", sort: p.get("sort") === "progress" ? "progress" : "activity", dense: p.get("dense") === "1", needsMe: p.get("needsme") === "1" };
}

/** `p` with the changed options written (and the defaults removed); everything else in it is kept. */
export function withBoard(p: URLSearchParams, change: Partial<BoardOptions>): URLSearchParams {
  const o = { ...parseBoard(p), ...change }, n = new URLSearchParams(p);
  const set = (k: string, v: string | null) => (v === null ? n.delete(k) : n.set(k, v));
  set("type", o.type === "all" ? null : o.type);
  set("stage", o.stages.length ? o.stages.join(",") : null);
  set("shipped", o.hideShipped ? "0" : null);
  set("dropped", o.showDropped ? "1" : null);
  set("by", o.byProject ? "project" : null);
  set("sort", o.sort === "progress" ? "progress" : null);
  set("dense", o.dense ? "1" : null);
  set("needsme", o.needsMe ? "1" : null);
  return n;
}

/** `p` with the board's open pane set to issue `id` (`up`: the spec it was opened from, for the back arrow), or closed with null. */
export function withOpen(p: URLSearchParams, id: string | null, up?: string): URLSearchParams {
  const n = new URLSearchParams(p);
  n.delete("open");
  n.delete("up");
  if (id) n.set("open", id);
  if (id && up) n.set("up", up);
  return n;
}

/** Something the user must do on this spec: merge it, or a Delivery or ticket that needs a human. */
export const needsMe = (stage: Stage, d: DeliverySummary | null) => stage === "ready" || d?.state === "needs_human" || !!d?.tickets.some((t) => t.state === "needs_human");

/** Whether a spec is the agent's on the board: it owns the spec, or builds its Delivery. Any spec without an agent. */
export const specOfAgent = (spec: Task, deliveries: DeliverySummary[], agent: string) =>
  !agent || spec.agent_id === agent || deliveryOf(spec, deliveries)?.agent_id === agent;

/** The specs the options keep, in the order they ask for. */
export function arrange(specs: Task[], deliveries: DeliverySummary[], tasks: Task[], o: BoardOptions): Task[] {
  const info = (s: Task) => { const d = deliveryOf(s, deliveries); return { d, stage: stageOf(s, d) }; };
  const frac = (s: Task) => { const ts = ticketsOf(s, info(s).d, tasks); return ts.length ? ts.filter((t) => t.state === "closed").length / ts.length : 0; };
  return specs
    .filter((s) => {
      const { d, stage } = info(s);
      return (!o.stages.length || o.stages.includes(stage)) && !(o.hideShipped && stage === "shipped") && (o.showDropped || stage !== "dropped") && (!o.needsMe || needsMe(stage, d));
    })
    .sort((a, b) => (o.sort === "progress" ? frac(b) - frac(a) : 0) || b.updated_at.localeCompare(a.updated_at));
}

/* ---------- the Issues column ---------- */

/** Statuses that wait for the user. */
export const NEEDS_YOU: TaskStatus[] = ["waiting_approval", "blocked", "failed"];

/** A spec's ticket: it has a parent, names its spec in the template's `## Parent` section, or is in a Delivery's tickets. */
export const isSpecTicket = (t: Task, deliveries: DeliverySummary[]) =>
  !!t.parent_id || /^## Parent\s+#\d+/m.test(t.description) || deliveries.some((d) => d.tickets.some((x) => (x.id ? x.id === t.id : !!t.github && d.repo === t.github.repo && x.number === t.github.number)));

/** The Issues column: the issues (of `tasks`, already through the shared filters) that are no spec and no spec's ticket. Closed ones only when asked for, open ones first, then the most recently updated.
 * Whether an issue is a spec's ticket is only known once the Deliveries have loaded, so until then the column is empty. */
export function boardIssues(tasks: Task[], deliveries: DeliverySummary[], loaded: boolean, o: BoardOptions, showClosed: boolean): Task[] {
  if (!loaded) return [];
  const closed = (t: Task) => t.state === "closed";
  return tasks
    .filter((t) => !t.labels.some((l) => l.toLowerCase() === "spec") && !isSpecTicket(t, deliveries) && (showClosed || !closed(t)) && (!o.needsMe || NEEDS_YOU.includes(t.status)))
    .sort((a, b) => Number(closed(a)) - Number(closed(b)) || b.updated_at.localeCompare(a.updated_at));
}
