import { Check, CircleDot, ExternalLink, GitPullRequest } from "lucide-react";
import type { Agent, Task } from "../lib/api";
import { cx } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { specProgress } from "../lib/issueOrder";
import { DROPPED, STAGES, deliveryOf, stageOf, ticketLabel, ticketsOf, type DeliverySummary, type Stage } from "../lib/specViews";

/* Pieces shared by the Issues page's Specs board (./SpecBoard) and a spec's own page, which opens with its Delivery timeline (`SpecPanel`). */

export const STAGE_TONE: Record<Stage, string> = { planned: "text-fg-3", delivery: "text-accent", ready: "text-warn", shipped: "text-merged", dropped: "text-fg-3" };
export const TONE_TEXT = { good: "text-good", accent: "text-accent", warn: "text-warn", muted: "text-fg-3" } as const;
export const TONE_BG = { good: "bg-good", accent: "bg-accent", warn: "bg-warn", muted: "bg-fg-3/40" } as const;

export type Ctx = { tasks: Task[]; deliveries: DeliverySummary[]; agentById: Map<string, Agent> };

/** Tickets done of all: the Delivery's own tickets when it has them (live), else the issue list's count. */
export function progressOf(spec: Task, d: DeliverySummary | null, tasks: Task[]) {
  const ts = d?.tickets ?? [];
  return ts.length ? { done: ts.filter((t) => t.state === "closed").length, total: ts.length } : specProgress(spec, tasks);
}

export function Badge({ stage, d }: { stage: Stage; d: DeliverySummary | null }) {
  const label = [...STAGES, DROPPED].find((s) => s.key === stage)!.label;
  const sub = stage === "delivery" && d?.state === "final_review" ? " · final review" : stage === "delivery" && d?.state === "final_fixes" ? " · fixing" : stage === "delivery" && d?.state === "needs_human" ? " · needs you" : "";
  return <span className={cx("chip shrink-0 text-[11px]", STAGE_TONE[stage], stage === "delivery" && d?.state === "needs_human" && "text-warn")}>{label}{sub}</span>;
}

export function Bar({ done, total, stage }: { done: number; total: number; stage: Stage }) {
  return (
    <div className="flex items-center gap-2">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/[0.08]" role="progressbar" aria-valuenow={done} aria-valuemax={total} aria-label="Tickets done">
        <div className={cx("h-full rounded-full", stage === "shipped" ? "bg-merged" : stage === "dropped" ? "bg-fg-3" : stage === "ready" ? "bg-warn" : "bg-accent")} style={{ width: `${total ? Math.round((done / total) * 100) : 0}%` }} />
      </div>
      <span className="shrink-0 font-mono text-[10.5px] text-fg-2">{done}/{total}</span>
    </div>
  );
}

export function Who({ d, agentById }: { d: DeliverySummary | null; agentById: Map<string, Agent> }) {
  const running = d?.tickets.filter((t) => t.agent_id) ?? [];
  if (!d || (!running.length && !d.waiting_for_lead?.length && !d.waiting_on_you?.length)) return null;
  return (
    <div className="space-y-0.5 text-[11.5px] text-fg-2">
      {running.map((t) => {
        const a = agentById.get(t.agent_id!);
        return (
          <div key={t.number} className="flex min-w-0 items-center gap-1.5">
            {a && <Sticker avatar={a.avatar} lead={a.role === "lead"} size={16} />}
            <span className="truncate"><span className="text-fg">{t.agent_name ?? "An agent"}</span> on #{t.number} {t.title}</span>
          </div>
        );
      })}
      {!!d.waiting_on_you?.length && <div className="text-warn">Waiting on you: {d.waiting_on_you.map((n) => `#${n}`).join(", ")}</div>}
      {!!d.waiting_for_lead?.length && <div className="text-warn">Waiting for the Lead: {d.waiting_for_lead.map((n) => `#${n}`).join(", ")}</div>}
    </div>
  );
}

const prUrl = (spec: Task, d: DeliverySummary | null) => (d?.pr && spec.github ? `https://github.com/${spec.github.repo}/pull/${d.pr}` : null);

function TicketLine({ t, current, open }: { t: DeliverySummary["tickets"][number]; current: boolean; open?: () => void }) {
  const l = ticketLabel(t.state);
  const row = "flex w-full items-center gap-2 py-0.5 text-left text-[12.5px]";
  const body = (
    <>
      <span className={cx("grid size-4 shrink-0 place-items-center rounded-full", TONE_BG[l.tone], l.tone === "muted" ? "text-fg-3" : "text-ink-950")}>
        {t.state === "closed" ? <Check className="size-3" /> : current ? <CircleDot className="size-3" /> : null}
      </span>
      <span className="font-mono text-[10.5px] text-fg-3">#{t.number}</span>
      <span className={cx("min-w-0 flex-1 truncate", t.state === "closed" ? "text-fg-3" : "text-fg")}>{t.title}</span>
      {Object.entries(t.verdicts ?? {}).map(([role, v]) => (
        <span key={role} className={cx("font-mono text-[10px]", v === "pass" ? "text-good" : "text-fg-3")} title={`${role}: ${v}`}>{role} {v === "pass" ? "✓" : "…"}</span>
      ))}
      <span className={cx("shrink-0 font-mono text-[10.5px]", TONE_TEXT[l.tone])}>{l.text}</span>
    </>
  );
  return <li>{open ? <button onClick={open} className={cx(row, "rounded-md hover:bg-white/[0.04]")}>{body}</button> : <div className={row}>{body}</div>}</li>;
}

/** The top of a spec's own page: its Delivery as a timeline, who has what, the PR and each ticket's verdicts. */
export function SpecPanel({ spec, openTicket, ...c }: Ctx & { spec: Task; openTicket: (id: string) => void }) {
  const d = deliveryOf(spec, c.deliveries), stage = stageOf(spec, d), p = progressOf(spec, d, c.tasks), url = prUrl(spec, d);
  const own = d?.tickets.length ? ticketsOf(spec, d, c.tasks) : []; // same order as d.tickets, each with its own issue
  const verdicts = (d?.tickets ?? []).filter((t) => t.state === "review");
  return (
    <section className="panel space-y-4 p-4" aria-label="Delivery">
      <div className="flex flex-wrap items-center gap-2">
        <div className="eyebrow">Delivery</div><Badge stage={stage} d={d} />
        {url && <a href={url} target="_blank" rel="noreferrer" className="chip gap-1 text-[11px] hover:text-fg"><GitPullRequest className="size-3" /> Spec PR #{d!.pr} <ExternalLink className="size-3" /></a>}
        {stage === "ready" && (d?.can_reopen
          ? <span className="text-[12px] text-warn">Done, but tickets published since are open: Reopen delivery builds them.</span>
          : <span className="text-[12px] text-warn">Marked ready: waiting for you to merge.</span>)}
      </div>
      {p && <Bar {...p} stage={stage} />}
      {!d || d.state === "planned" ? (
        <div className="text-[13px] text-fg-3">Not started: no ticket has been handed to an agent yet.</div>
      ) : (
        <>
          <Who d={d} agentById={c.agentById} />
          {verdicts.length > 0 && (
            <div className="space-y-1 rounded-xl border border-line bg-ink-950/40 p-3">
              <div className="eyebrow">In review now</div>
              {verdicts.map((t) => <div key={t.number} className="flex flex-wrap items-center gap-2 text-[12.5px]"><span className="font-mono text-[10.5px] text-fg-3">#{t.number}</span><span className="min-w-0 flex-1 truncate">{t.title}</span>{Object.entries(t.verdicts ?? {}).map(([r, v]) => <span key={r} className={cx("chip text-[11px]", v === "pass" ? "text-good" : "text-fg-3")}>{r}: {v}</span>)}</div>)}
            </div>
          )}
          <ol className="relative ml-2 border-l border-line pl-4">
            {d.tickets.map((t, i) => <TicketLine key={t.number} t={t} current={!!t.agent_id} open={own[i]?.taskId ? () => openTicket(own[i].taskId!) : undefined} />)}
          </ol>
        </>
      )}
    </section>
  );
}
