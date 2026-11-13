import { useState } from "react";
import { ChevronRight, CircleCheck, CircleDot, GitPullRequest } from "lucide-react";
import type { Agent, Task } from "../lib/api";
import { cx, timeAgo } from "../lib/format";
import { Origin } from "../components/Origin";
import { parseFilters, withFilters } from "../lib/issueFilters";
import { Sticker } from "../components/Sticker";
import { GithubUser } from "../components/GithubUser";
import { STAGES, arrange, deliveryOf, parseBoard, specOfAgent, stageOf, ticketLabel, ticketsOf, withBoard, DROPPED, specMeta, type BoardOptions, type BoardType, type BoardTicket, type DeliverySummary, type SortKey, type Stage } from "../lib/specViews";
import { Badge, Bar, STAGE_TONE, TONE_BG, TONE_TEXT, Who, progressOf, type Ctx } from "./SpecViews";

/* The Issues page: an Issues column (issues that are no spec and no spec's ticket), then spec cards in stage columns, each with its tickets inline (expandable).
 * The filters and options sit in a bar on top and live in the URL. */

type View = Ctx & { specs: Task[]; issues: Task[]; loaded: boolean; o: BoardOptions; openSpec: (id: string) => void; openTicket: (id: string) => void };

const CAP = 12;

/** The specs, in sections by project when asked. */
const sections = (specs: Task[], byProject: boolean) =>
  byProject ? [...new Set(specs.map((s) => s.project || "No project"))].sort().map((label) => ({ label, specs: specs.filter((s) => (s.project || "No project") === label) })) : [{ label: null, specs }];

/** Grid columns by how many columns show (at least the Issues column or a spec stage). */
const GRID = ["", "grid-cols-1", "grid-cols-2", "grid-cols-3", "grid-cols-4", "grid-cols-5", "grid-cols-6"];

const Dot = ({ state, waitingOn }: { state: string; waitingOn?: string[] }) => <span className={cx("inline-block size-2 shrink-0 rounded-full", TONE_BG[ticketLabel(state, waitingOn).tone])} title={ticketLabel(state, waitingOn).text} />;

function Verdicts({ v }: { v: Record<string, string> }) {
  return <>{Object.entries(v).map(([r, x]) => <span key={r} className={cx("rounded-full border border-line px-1.5 font-mono text-[10px]", x === "pass" ? "text-good" : "text-fg-3")} title={`${r}: ${x}`}>{r} {x === "pass" ? "✓" : "…"}</span>)}</>;
}

/** A ticket as a card: opens its own issue. */
function TicketCard({ t, dense, agentById, open }: { t: BoardTicket; dense: boolean; agentById: Map<string, Agent>; open: (id: string) => void }) {
  const l = ticketLabel(t.state, t.waitingOn), a = t.agentId ? agentById.get(t.agentId) : undefined;
  const body = (
    <>
      <div className="flex items-start gap-1.5">
        <span className="font-mono text-[10.5px] text-fg-3">{t.number ? `#${t.number}` : "·"}</span>
        <span className={cx("min-w-0 flex-1 leading-snug", dense ? "truncate text-[12px]" : "text-[12.5px]", t.state === "closed" && "text-fg-3")}>{t.title}</span>
      </div>
      {(!dense || t.current || !!t.waitingOn?.length) && (
        <div className="mt-1 flex flex-wrap items-center gap-1">
          <span className={cx("min-w-0 font-mono text-[10px] [overflow-wrap:anywhere]", TONE_TEXT[l.tone])}>{l.text}</span>
          <Verdicts v={t.verdicts} />
          {a && <span className="ml-auto inline-flex items-center gap-1 text-[10.5px] text-fg-2"><Sticker avatar={a.avatar} lead={a.role === "lead"} size={14} />{a.name}</span>}
        </div>
      )}
    </>
  );
  const cls = cx("block w-full rounded-lg border border-line bg-ink-950/60 text-left", dense ? "px-2 py-1" : "px-2.5 py-2", t.current && "border-accent/50", t.state === "needs_human" && "border-warn/60");
  return t.taskId ? <button onClick={() => open(t.taskId!)} className={cx(cls, "transition-colors hover:bg-white/[0.04]")}>{body}</button> : <div className={cls}>{body}</div>;
}

/** An issue as a card: opens in the modal. */
function IssueCard({ t, dense, agentById, open }: { t: Task; dense: boolean; agentById: Map<string, Agent>; open: (id: string) => void }) {
  const a = t.agent_id ? agentById.get(t.agent_id) : undefined, closed = t.state === "closed";
  return (
    <button onClick={() => open(t.id)} className={cx("panel block w-full text-left transition-colors hover:bg-white/[0.04]", dense ? "p-2" : "p-3")}>
      <div className="flex items-start gap-1.5">
        {closed ? <CircleCheck className="mt-0.5 size-3.5 shrink-0 text-merged" aria-label="Closed" /> : <CircleDot className="mt-0.5 size-3.5 shrink-0 text-good" aria-label="Open" />}
        <span className={cx("min-w-0 flex-1 leading-snug", dense ? "truncate text-[12.5px]" : "text-[13px] [overflow-wrap:anywhere]", closed && "text-fg-3")}>{t.title}</span>
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[10.5px] text-fg-3">
        <Origin github={!!t.github} />
        <span>{t.github ? `#${t.github.number}` : "Loft"}</span>
        {t.project && <span>{t.project}</span>}
        <span>{timeAgo(t.updated_at)}</span>
        {(t.owner || (!dense && a)) && (
          <span className="ml-auto inline-flex min-w-0 items-center gap-2 text-[10.5px] text-fg-2">
            {t.owner && <GithubUser login={t.owner} bare />}
            {!dense && a && <span className="inline-flex items-center gap-1"><Sticker avatar={a.avatar} lead={a.role === "lead"} size={14} />{a.name}</span>}
          </span>
        )}
      </div>
    </button>
  );
}

function SpecHead({ s, v, children }: { s: Task; v: View; children?: React.ReactNode }) {
  const d = deliveryOf(s, v.deliveries), stage = stageOf(s, d), p = progressOf(s, d, v.tasks), m = specMeta(s);
  return (
    <button onClick={() => v.openSpec(s.id)} className="block w-full space-y-1.5 text-left">
      <div className="flex flex-wrap items-start gap-x-2 gap-y-1"><span className={cx("min-w-[9rem] flex-1 leading-snug", v.o.dense ? "truncate text-[12.5px]" : "text-[13px]")}>{s.title}</span><Badge stage={stage} d={d} /></div>
      {p && <Bar {...p} stage={stage} />}
      {!v.o.dense && <Who d={d} agentById={v.agentById} />}
      <div className="flex flex-wrap items-center gap-x-2 font-mono text-[10.5px] text-fg-3"><span title="Where this spec lives">{m.ref}</span>{m.project && <span title="Project">{m.project}</span>}{d?.pr && <span className="inline-flex items-center gap-1"><GitPullRequest className="size-3" />#{d.pr}</span>}{s.owner && <span className="ml-auto text-fg-2"><GithubUser login={s.owner} bare /></span>}</div>
      {children}
    </button>
  );
}

const Empty = ({ text }: { text: string }) => <div className="px-2 py-4 text-center text-[12px] text-fg-3">{text}</div>;
const Label = ({ text }: { text: string }) => <div className="px-1 pb-1 pt-2 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">{text}</div>;

/** The Issues column and the spec cards in the stage columns; a strip of dots for the tickets, and the list when opened. */
function Cards(v: View) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setOpen((o) => { const n = new Set(o); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const cols = v.o.type === "issues" ? [] : [...STAGES, ...(v.o.showDropped ? [DROPPED] : [])].filter((c) => !(v.o.hideShipped && c.key === "shipped"));
  const withIssues = v.o.type !== "specs";
  return (
    <div className={cx("grid min-h-0 flex-1 gap-3 overflow-x-auto px-4 pb-4 max-xl:grid-cols-2 max-md:flex max-md:flex-col max-md:overflow-y-auto", GRID[cols.length + Number(withIssues)])}>
      {withIssues && (
        <section aria-label="Issues" className="flex min-h-0 min-w-[240px] flex-col rounded-2xl border border-line bg-ink-900/60 max-md:shrink-0">
          <div className="flex items-center justify-between px-3 py-2 font-mono text-[10.5px] uppercase tracking-wider"><span className="text-fg-2">Issues</span><span className="text-fg-3">{v.issues.length}</span></div>
          <div className="min-h-0 flex-1 overflow-y-auto max-md:overflow-visible px-2 pb-2">
            {sections(v.issues, v.o.byProject).map((sec) => (
              <div key={sec.label ?? "all"} className="space-y-2">
                {sec.label && <Label text={sec.label} />}
                {sec.specs.map((t) => <IssueCard key={t.id} t={t} dense={v.o.dense} agentById={v.agentById} open={v.openTicket} />)}
              </div>
            ))}
            {v.loaded && !v.issues.length && <Empty text="Nothing here" />}
          </div>
        </section>
      )}
      {cols.map((st) => {
        const specs = v.specs.filter((s) => stageOf(s, deliveryOf(s, v.deliveries)) === st.key);
        return (
          <section key={st.key} className="flex min-h-0 min-w-[240px] flex-col rounded-2xl border border-line bg-ink-900/60 max-md:shrink-0">
            <div className="flex items-center justify-between px-3 py-2 font-mono text-[10.5px] uppercase tracking-wider"><span className={STAGE_TONE[st.key]}>{st.label}</span><span className="text-fg-3">{specs.length}</span></div>
            <div className="min-h-0 flex-1 overflow-y-auto max-md:overflow-visible px-2 pb-2">
              {sections(specs, v.o.byProject).map((sec) => (
                <div key={sec.label ?? "all"} className="space-y-2">
                  {sec.label && <Label text={sec.label} />}
                  {sec.specs.map((s) => {
                    const d = deliveryOf(s, v.deliveries), ts = ticketsOf(s, d, v.tasks), isOpen = open.has(s.id);
                    return (
                      <div key={s.id} className={cx("panel", v.o.dense ? "p-2" : "p-3")}>
                        <SpecHead s={s} v={v} />
                        {ts.length > 0 && (
                          <>
                            <button onClick={() => toggle(s.id)} aria-expanded={isOpen} className="mt-2 flex w-full items-center gap-1.5 text-left text-[11.5px] text-fg-3 hover:text-fg">
                              <ChevronRight className={cx("size-3 shrink-0 transition-transform", isOpen && "rotate-90")} />
                              <span className="flex flex-wrap gap-[3px]">{ts.map((t) => <Dot key={t.key} state={t.state} waitingOn={t.waitingOn} />)}</span>
                              <span className="ml-auto font-mono text-[10.5px]">{ts.length} {ts.length === 1 ? "ticket" : "tickets"}</span>
                            </button>
                            {isOpen && <div className="mt-2 space-y-1.5">{ts.map((t) => <TicketCard key={t.key} t={t} dense agentById={v.agentById} open={v.openTicket} />)}</div>}
                          </>
                        )}
                      </div>
                    );
                  })}
                </div>
              ))}
              {!specs.length && <Empty text="Nothing here" />}
            </div>
          </section>
        );
      })}
    </div>
  );
}

const chip = (on: boolean) => cx("chip text-[12px]", on && "border-accent/40 text-accent");

/** The board's own options, under the page's shared filter bar; they all live in the URL. */
function Options({ params, setParams }: { params: URLSearchParams; setParams: (n: URLSearchParams) => void }) {
  const o = parseBoard(params);
  const set = (c: Partial<BoardOptions>) => setParams(withBoard(params, c));
  const closed = parseFilters(params).closed;
  return (
    <div className="border-b border-line px-4 pb-3 pt-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-center gap-0.5 rounded-full border border-line bg-ink-900 p-0.5" role="group" aria-label="Show">
          {([["all", "All"], ["issues", "Issues"], ["specs", "Specs"]] as [BoardType, string][]).map(([key, label]) => (
            <button key={key} onClick={() => set({ type: key })} aria-pressed={o.type === key} className={cx("rounded-full px-2.5 py-0.5 text-[12px]", o.type === key ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>{label}</button>
          ))}
        </div>
        <span className="mx-1 h-4 w-px bg-line" />
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Spec state">
          {STAGES.map((st: { key: Stage; label: string }) => {
            const on = o.stages.includes(st.key);
            return <button key={st.key} aria-pressed={on} onClick={() => set({ stages: on ? o.stages.filter((x) => x !== st.key) : [...o.stages, st.key] })} className={chip(on)}>{st.label}</button>;
          })}
        </div>
        <span className="mx-1 h-4 w-px bg-line" />
        <button className={chip(o.needsMe)} aria-pressed={o.needsMe} onClick={() => set({ needsMe: !o.needsMe })} title="Ready to merge, or something needs a human">Needs me</button>
        <button className={chip(closed)} aria-pressed={closed} onClick={() => setParams(withFilters(params, { closed: !closed }))} title="Closed issues in the Issues column">Show closed</button>
        <button className={chip(o.showDropped)} aria-pressed={o.showDropped} onClick={() => set({ showDropped: !o.showDropped })} title="Specs closed as not planned">Show dropped</button>
        <button className={chip(o.hideShipped)} aria-pressed={o.hideShipped} onClick={() => set({ hideShipped: !o.hideShipped })}>Hide shipped</button>
        <button className={chip(o.byProject)} aria-pressed={o.byProject} onClick={() => set({ byProject: !o.byProject })}>Group by project</button>
        <button className={chip(o.dense)} aria-pressed={o.dense} onClick={() => set({ dense: !o.dense })}>Compact</button>
        <select value={o.sort} onChange={(e) => set({ sort: e.target.value as SortKey })} className="chip bg-ink-850 text-[12px]" aria-label="Sort">
          <option value="activity">Sort: recent activity</option><option value="progress">Sort: progress</option>
        </select>
      </div>
    </div>
  );
}

/** The whole page of the board. `issues` are the Issues column (see `boardIssues`); `kept` are the specs the shared filters and the board's options let through (see `boardSpecs`). */
export function BoardPage({ kept, issues, loaded, params, setParams, openSpec, openTicket, ...c }: Ctx & {
  kept: Task[]; issues: Task[]; loaded: boolean; params: URLSearchParams; setParams: (n: URLSearchParams) => void; openSpec: (id: string) => void; openTicket: (id: string) => void;
}) {
  const o = parseBoard(params);
  const [all, setAll] = useState(false);
  // Shipped specs pile up: the newest few show, the rest on request.
  const shipped = kept.filter((s) => stageOf(s, deliveryOf(s, c.deliveries)) === "shipped");
  const hidden = all ? [] : shipped.slice(CAP);
  const shown = kept.filter((s) => !hidden.includes(s));
  const v: View = { ...c, specs: shown, issues, loaded, o, openSpec, openTicket };
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <Options params={params} setParams={setParams} />
      {!loaded && <div className="px-4 pt-3 font-mono text-[11px] text-fg-3">Loading Deliveries…</div>}
      <div className="flex min-h-0 flex-1 flex-col pt-3">
        <Cards {...v} />
        {hidden.length > 0 && o.type !== "issues" && <button onClick={() => setAll(true)} className="mx-4 mb-3 mt-1 self-start rounded-full border border-line px-3 py-1 font-mono text-[11px] text-fg-3 hover:text-fg">Show {hidden.length} older shipped</button>}
      </div>
    </div>
  );
}

/** The specs on the board: `specs` (already through the shared filters), arranged by the board's options, and by agent
 * (a spec's own, or its Delivery's): the shared agent filter doesn't apply to `specs` before this. */
export function boardSpecs(specs: Task[], deliveries: DeliverySummary[], tasks: Task[], o: BoardOptions, agent: string): Task[] {
  return arrange(specs, deliveries, tasks, o).filter((s) => specOfAgent(s, deliveries, agent));
}
