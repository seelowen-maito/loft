import { useState } from "react";
import { Check, ChevronRight, CircleDot, ExternalLink, GitPullRequest } from "lucide-react";
import type { Agent, Task } from "../lib/api";
import { cx } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { specProgress } from "../lib/issueOrder";
import { STAGES, deliveryOf, stageOf, ticketLabel, type DeliverySummary, type Stage } from "../lib/specViews";

/* Three takes on the same data, behind `?specView=a|b|c` on the Issues page: (a) spec rows that expand to their tickets,
 * (b) a board by stage, (c) a spec's own page opening with its timeline. They share these pieces. */

const STAGE_TONE: Record<Stage, string> = { planned: "text-fg-3", delivery: "text-accent", ready: "text-warn", shipped: "text-merged" };
const TONE_TEXT = { good: "text-good", accent: "text-accent", warn: "text-warn", muted: "text-fg-3" } as const;
const TONE_BG = { good: "bg-good", accent: "bg-accent", warn: "bg-warn", muted: "bg-fg-3/40" } as const;

type Ctx = { tasks: Task[]; deliveries: DeliverySummary[]; agentById: Map<string, Agent> };

/** Tickets done of all: the Delivery's own tickets when it has them (live), else the issue list's count. */
function progressOf(spec: Task, d: DeliverySummary | null, tasks: Task[]) {
  const ts = d?.tickets ?? [];
  return ts.length ? { done: ts.filter((t) => t.state === "closed").length, total: ts.length } : specProgress(spec, tasks);
}

function Badge({ stage, d }: { stage: Stage; d: DeliverySummary | null }) {
  const label = STAGES.find((s) => s.key === stage)!.label;
  const sub = stage === "delivery" && d?.state === "final_review" ? " · final review" : stage === "delivery" && d?.state === "final_fixes" ? " · fixing" : stage === "delivery" && d?.state === "needs_human" ? " · needs you" : "";
  return <span className={cx("chip shrink-0 text-[11px]", STAGE_TONE[stage], stage === "delivery" && d?.state === "needs_human" && "text-warn")}>{label}{sub}</span>;
}

function Bar({ done, total, stage }: { done: number; total: number; stage: Stage }) {
  return (
    <div className="flex items-center gap-2">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/[0.08]" role="progressbar" aria-valuenow={done} aria-valuemax={total} aria-label="Tickets done">
        <div className={cx("h-full rounded-full", stage === "shipped" ? "bg-merged" : stage === "ready" ? "bg-warn" : "bg-accent")} style={{ width: `${total ? Math.round((done / total) * 100) : 0}%` }} />
      </div>
      <span className="shrink-0 font-mono text-[10.5px] text-fg-2">{done}/{total}</span>
    </div>
  );
}

function Who({ d, agentById }: { d: DeliverySummary | null; agentById: Map<string, Agent> }) {
  if (!d?.current) return null;
  const a = d.agent_id ? agentById.get(d.agent_id) : undefined;
  return (
    <div className="flex min-w-0 items-center gap-1.5 text-[11.5px] text-fg-2">
      {a && <Sticker avatar={a.avatar} lead={a.role === "lead"} size={16} />}
      <span className="truncate"><span className="text-fg">{d.agent_name ?? "An agent"}</span> on #{d.current.number} {d.current.title}</span>
    </div>
  );
}

const prUrl = (spec: Task, d: DeliverySummary | null) => (d?.pr && spec.github ? `https://github.com/${spec.github.repo}/pull/${d.pr}` : null);

function TicketLine({ t, current }: { t: DeliverySummary["tickets"][number]; current: boolean }) {
  const l = ticketLabel(t.state);
  return (
    <li className="flex items-center gap-2 py-0.5 text-[12.5px]">
      <span className={cx("grid size-4 shrink-0 place-items-center rounded-full", TONE_BG[l.tone], l.tone === "muted" ? "text-fg-3" : "text-ink-950")}>
        {t.state === "closed" ? <Check className="size-3" /> : current ? <CircleDot className="size-3" /> : null}
      </span>
      <span className="font-mono text-[10.5px] text-fg-3">#{t.number}</span>
      <span className={cx("min-w-0 flex-1 truncate", t.state === "closed" ? "text-fg-3" : "text-fg")}>{t.title}</span>
      {Object.entries(t.verdicts ?? {}).map(([role, v]) => (
        <span key={role} className={cx("font-mono text-[10px]", v === "pass" ? "text-good" : "text-fg-3")} title={`${role}: ${v}`}>{role} {v === "pass" ? "✓" : "…"}</span>
      ))}
      <span className={cx("shrink-0 font-mono text-[10.5px]", TONE_TEXT[l.tone])}>{l.text}</span>
    </li>
  );
}

const ORDER: Stage[] = ["delivery", "ready", "planned", "shipped"];
const byStage = (specs: Task[], c: Ctx) =>
  ORDER.map((stage) => ({ stage, label: STAGES.find((s) => s.key === stage)!.label, specs: specs.filter((s) => stageOf(s, deliveryOf(s, c.deliveries)) === stage) }));

/** (a) A list of specs in place of the issues: a progress bar and a Delivery badge each, opening to their tickets. */
export function SpecList({ specs, active, onOpen, ...c }: Ctx & { specs: Task[]; active: string | undefined; onOpen: (id: string) => void }) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setOpen((o) => { const n = new Set(o); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  return (
    <div className="space-y-4">
      {byStage(specs, c).filter((g) => g.specs.length).map((g) => (
        <section key={g.stage}>
          <div className="px-3 py-1 font-mono text-[10.5px] uppercase tracking-wider text-fg-3"><span className={STAGE_TONE[g.stage]}>{g.label}</span> · {g.specs.length}</div>
          {g.specs.slice(0, g.stage === "shipped" ? 10 : undefined).map((s) => {
            const d = deliveryOf(s, c.deliveries), p = progressOf(s, d, c.tasks), isOpen = open.has(s.id);
            return (
              <div key={s.id} className={cx("rounded-xl", s.id === active && "bg-white/[0.06]")}>
                <div className="flex items-start gap-1 px-1 py-2">
                  <button onClick={() => toggle(s.id)} aria-expanded={isOpen} aria-label={isOpen ? "Hide tickets" : "Show tickets"} className="mt-0.5 rounded p-1 text-fg-3 hover:text-fg">
                    <ChevronRight className={cx("size-3.5 transition-transform", isOpen && "rotate-90")} />
                  </button>
                  <button onClick={() => onOpen(s.id)} className="min-w-0 flex-1 space-y-1.5 text-left">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-[13.5px]">{s.title}</span>
                      <Badge stage={g.stage} d={d} />
                    </div>
                    {p && <Bar {...p} stage={g.stage} />}
                    <Who d={d} agentById={c.agentById} />
                    <div className="font-mono text-[10.5px] text-fg-3">{s.github ? `#${s.github.number}` : "Loft"}{s.project && ` · ${s.project}`}</div>
                  </button>
                </div>
                {isOpen && (
                  <ul className="mb-2 ml-9 mr-3 border-l border-line pl-3">
                    {d?.tickets.length ? d.tickets.map((t) => <TicketLine key={t.number} t={t} current={d.current?.number === t.number} />) : <li className="py-1 text-[12px] text-fg-3">No tickets yet.</li>}
                  </ul>
                )}
              </div>
            );
          })}
        </section>
      ))}
      {!specs.length && <div className="px-3 py-10 text-center text-[13px] text-fg-3">No specs match these filters.</div>}
    </div>
  );
}

/** (b) A board: Planned, In Delivery, Ready to merge, Shipped. */
export function SpecBoard({ specs, onOpen, ...c }: Ctx & { specs: Task[]; onOpen: (id: string) => void }) {
  const cols = STAGES.map((st) => ({ ...st, specs: specs.filter((s) => stageOf(s, deliveryOf(s, c.deliveries)) === st.key) }));
  return (
    <div className="flex h-full flex-col">
      <div className="px-6 pb-3 pt-5"><div className="eyebrow">Specs</div><div className="mt-1 font-mono text-[11px] text-fg-3">{specs.length} specs · {cols[1].specs.length} in Delivery · {cols[2].specs.length} ready to merge</div></div>
      <div className="grid min-h-0 flex-1 grid-cols-4 gap-3 overflow-x-auto px-4 pb-4 max-xl:grid-cols-2 max-md:grid-cols-1">
        {cols.map((col) => (
          <section key={col.key} className="flex min-h-0 min-w-[220px] flex-col rounded-2xl border border-line bg-ink-900/60">
            <div className="flex items-center justify-between px-3 py-2 font-mono text-[10.5px] uppercase tracking-wider"><span className={STAGE_TONE[col.key]}>{col.label}</span><span className="text-fg-3">{col.specs.length}</span></div>
            <div className="min-h-0 flex-1 space-y-2 overflow-y-auto px-2 pb-2">
              {col.specs.slice(0, col.key === "shipped" ? 20 : undefined).map((s) => {
                const d = deliveryOf(s, c.deliveries), p = progressOf(s, d, c.tasks);
                return (
                  <button key={s.id} onClick={() => onOpen(s.id)} className="panel block w-full space-y-2 p-3 text-left transition-colors hover:bg-white/[0.03]">
                    <div className="text-[13px] leading-snug">{s.title}</div>
                    {p && <Bar {...p} stage={col.key} />}
                    <Who d={d} agentById={c.agentById} />
                    <div className="flex flex-wrap items-center gap-x-2 font-mono text-[10.5px] text-fg-3">
                      <span>{s.github ? `#${s.github.number}` : "Loft"}</span>{s.project && <span>· {s.project}</span>}
                      {d?.pr && <span className="ml-auto inline-flex items-center gap-1"><GitPullRequest className="size-3" />#{d.pr}</span>}
                    </div>
                  </button>
                );
              })}
              {!col.specs.length && <div className="px-2 py-6 text-center text-[12px] text-fg-3">Nothing here</div>}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}

/** (c) The top of a spec's own page: its Delivery as a timeline, who has what, the PR and each ticket's verdicts. */
export function SpecPanel({ spec, ...c }: Ctx & { spec: Task }) {
  const d = deliveryOf(spec, c.deliveries), stage = stageOf(spec, d), p = progressOf(spec, d, c.tasks), url = prUrl(spec, d);
  const verdicts = (d?.tickets ?? []).filter((t) => t.state === "review");
  return (
    <section className="panel space-y-4 p-4" aria-label="Delivery">
      <div className="flex flex-wrap items-center gap-2">
        <div className="eyebrow">Delivery</div><Badge stage={stage} d={d} />
        {url && <a href={url} target="_blank" rel="noreferrer" className="chip gap-1 text-[11px] hover:text-fg"><GitPullRequest className="size-3" /> Spec PR #{d!.pr} <ExternalLink className="size-3" /></a>}
        {stage === "ready" && <span className="text-[12px] text-warn">Marked ready: waiting for you to merge.</span>}
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
            {d.tickets.map((t) => <TicketLine key={t.number} t={t} current={d.current?.number === t.number} />)}
          </ol>
        </>
      )}
    </section>
  );
}
