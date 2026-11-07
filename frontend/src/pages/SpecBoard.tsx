import { useState } from "react";
import { ChevronRight, GitPullRequest, Search } from "lucide-react";
import type { Agent, Task } from "../lib/api";
import { cx } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { STAGES, arrange, deliveryOf, parseBoard, stageOf, ticketLabel, ticketsOf, withBoard, LANES, laneOf, type BoardOptions, type BoardTicket, type DeliverySummary, type SortKey, type Stage } from "../lib/specViews";
import { Badge, Bar, STAGE_TONE, TONE_BG, TONE_TEXT, Who, progressOf, type Ctx } from "./SpecViews";

/* The spec board, three ways to see each spec's tickets on it (`?specView=b1|b2|b3`):
 *  b1 swimlanes: a row per spec, its tickets in Open / In progress / Done;
 *  b2 spec cards in the stage columns, each with its tickets inline (expandable);
 *  b3 a ticket board: columns are ticket states, grouped under their spec.
 * The filters and options sit in a bar on top and live in the URL. */

export type BoardVariant = "b1" | "b2" | "b3";
export const BOARD_VARIANTS: BoardVariant[] = ["b1", "b2", "b3"];

type Facets = { teams: string[]; projects: string[]; agents: Agent[] };
type Filters = { q: string; group: string; project: string; agent: string };

type View = Ctx & { specs: Task[]; o: BoardOptions; openSpec: (id: string) => void; openTicket: (id: string) => void };

const CAP = 12;

/** The specs, in sections by project when asked. */
const sections = (specs: Task[], byProject: boolean) =>
  byProject ? [...new Set(specs.map((s) => s.project || "No project"))].sort().map((label) => ({ label, specs: specs.filter((s) => (s.project || "No project") === label) })) : [{ label: null, specs }];

const Dot = ({ state }: { state: string }) => <span className={cx("inline-block size-2 shrink-0 rounded-full", TONE_BG[ticketLabel(state).tone])} title={ticketLabel(state).text} />;

function Verdicts({ v }: { v: Record<string, string> }) {
  return <>{Object.entries(v).map(([r, x]) => <span key={r} className={cx("rounded-full border border-line px-1.5 font-mono text-[10px]", x === "pass" ? "text-good" : "text-fg-3")} title={`${r}: ${x}`}>{r} {x === "pass" ? "✓" : "…"}</span>)}</>;
}

/** A ticket as a card: opens its own issue. */
function TicketCard({ t, d, dense, agentById, open }: { t: BoardTicket; d: DeliverySummary | null; dense: boolean; agentById: Map<string, Agent>; open: (id: string) => void }) {
  const l = ticketLabel(t.state), a = t.current && d?.agent_id ? agentById.get(d.agent_id) : undefined;
  const body = (
    <>
      <div className="flex items-start gap-1.5">
        <span className="font-mono text-[10.5px] text-fg-3">{t.number ? `#${t.number}` : "·"}</span>
        <span className={cx("min-w-0 flex-1 leading-snug", dense ? "truncate text-[12px]" : "text-[12.5px]", t.state === "closed" && "text-fg-3")}>{t.title}</span>
      </div>
      {(!dense || t.current) && (
        <div className="mt-1 flex flex-wrap items-center gap-1">
          <span className={cx("font-mono text-[10px]", TONE_TEXT[l.tone])}>{l.text}</span>
          <Verdicts v={t.verdicts} />
          {a && <span className="ml-auto inline-flex items-center gap-1 text-[10.5px] text-fg-2"><Sticker avatar={a.avatar} lead={a.role === "lead"} size={14} />{a.name}</span>}
        </div>
      )}
    </>
  );
  const cls = cx("block w-full rounded-lg border border-line bg-ink-950/60 text-left", dense ? "px-2 py-1" : "px-2.5 py-2", t.current && "border-accent/50", t.state === "needs_human" && "border-warn/60");
  return t.taskId ? <button onClick={() => open(t.taskId!)} className={cx(cls, "transition-colors hover:bg-white/[0.04]")}>{body}</button> : <div className={cls}>{body}</div>;
}

function SpecHead({ s, v, children }: { s: Task; v: View; children?: React.ReactNode }) {
  const d = deliveryOf(s, v.deliveries), stage = stageOf(s, d), p = progressOf(s, d, v.tasks);
  return (
    <button onClick={() => v.openSpec(s.id)} className="block w-full space-y-1.5 text-left">
      <div className="flex flex-wrap items-start gap-x-2 gap-y-1"><span className={cx("min-w-[9rem] flex-1 leading-snug", v.o.dense ? "truncate text-[12.5px]" : "text-[13px]")}>{s.title}</span><Badge stage={stage} d={d} /></div>
      {p && <Bar {...p} stage={stage} />}
      {!v.o.dense && <Who d={d} agentById={v.agentById} />}
      <div className="flex flex-wrap items-center gap-x-2 font-mono text-[10.5px] text-fg-3"><span>{s.github ? `#${s.github.number}` : "Loft"}</span>{s.project && <span>· {s.project}</span>}{d?.pr && <span className="inline-flex items-center gap-1"><GitPullRequest className="size-3" />#{d.pr}</span>}</div>
      {children}
    </button>
  );
}

const Empty = ({ text }: { text: string }) => <div className="px-2 py-4 text-center text-[12px] text-fg-3">{text}</div>;
const Label = ({ text }: { text: string }) => <div className="px-1 pb-1 pt-2 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">{text}</div>;

/** b1: a row per spec, its tickets in three columns. */
function Swimlanes(v: View) {
  return (
    <div className="min-h-0 flex-1 overflow-auto px-4 pb-4">
      <div className="min-w-[780px]">
        <div className="sticky top-0 z-10 grid grid-cols-[250px_repeat(3,minmax(0,1fr))] gap-2 bg-ink-950 py-2 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
          <div>Spec</div>{LANES.map((l) => <div key={l.key}>{l.label}</div>)}
        </div>
        {sections(v.specs, v.o.byProject).map((sec) => (
          <div key={sec.label ?? "all"}>
            {sec.label && <Label text={sec.label} />}
            {sec.specs.map((s) => {
              const d = deliveryOf(s, v.deliveries), ts = ticketsOf(s, d, v.tasks);
              return (
                <div key={s.id} className="mb-2 grid grid-cols-[250px_repeat(3,minmax(0,1fr))] gap-2 rounded-2xl border border-line bg-ink-900/60 p-2">
                  <div className="p-1"><SpecHead s={s} v={v} /></div>
                  {LANES.map((l) => {
                    const mine = ts.filter((t) => laneOf(t.state) === l.key);
                    return (
                      <div key={l.key} className="space-y-1.5 rounded-xl bg-ink-950/40 p-1.5">
                        {mine.map((t) => <TicketCard key={t.key} t={t} d={d} dense={v.o.dense} agentById={v.agentById} open={v.openTicket} />)}
                        {!mine.length && <div className="py-2 text-center font-mono text-[10.5px] text-fg-3/60">{ts.length ? "·" : l.key === "open" ? "No tickets yet" : ""}</div>}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

/** b2: spec cards in the stage columns; a strip of dots for the tickets, and the list when opened. */
function Cards(v: View) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setOpen((o) => { const n = new Set(o); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  return (
    <div className="grid min-h-0 flex-1 grid-cols-4 gap-3 overflow-x-auto px-4 pb-4 max-xl:grid-cols-2 max-md:flex max-md:flex-col max-md:overflow-y-auto">
      {STAGES.map((st) => {
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
                              <span className="flex flex-wrap gap-[3px]">{ts.map((t) => <Dot key={t.key} state={t.state} />)}</span>
                              <span className="ml-auto font-mono text-[10.5px]">{ts.length} tickets</span>
                            </button>
                            {isOpen && <div className="mt-2 space-y-1.5">{ts.map((t) => <TicketCard key={t.key} t={t} d={d} dense agentById={v.agentById} open={v.openTicket} />)}</div>}
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

const TICKET_COLS: { key: string; label: string; has: (state: string) => boolean }[] = [
  { key: "open", label: "Open", has: (s) => s === "open" || s === "elsewhere" },
  { key: "doing", label: "Building", has: (s) => ["started", "closing", "needs_human"].includes(s) },
  { key: "review", label: "In review", has: (s) => s === "review" },
  { key: "done", label: "Done", has: (s) => s === "closed" },
];

/** b3: the tickets themselves in columns by state, grouped under their spec. */
function TicketBoard(v: View) {
  const rows = v.specs.map((s) => { const d = deliveryOf(s, v.deliveries); return { s, d, ts: ticketsOf(s, d, v.tasks) }; });
  return (
    <div className="grid min-h-0 flex-1 grid-cols-4 gap-3 overflow-x-auto px-4 pb-4 max-xl:grid-cols-2 max-md:flex max-md:flex-col max-md:overflow-y-auto">
      {TICKET_COLS.map((col) => {
        const here = rows.map((r) => ({ ...r, ts: r.ts.filter((t) => col.has(t.state)) })).filter((r) => r.ts.length);
        return (
          <section key={col.key} className="flex min-h-0 min-w-[240px] flex-col rounded-2xl border border-line bg-ink-900/60 max-md:shrink-0">
            <div className="flex items-center justify-between px-3 py-2 font-mono text-[10.5px] uppercase tracking-wider"><span className="text-fg-2">{col.label}</span><span className="text-fg-3">{here.reduce((n, r) => n + r.ts.length, 0)}</span></div>
            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto max-md:overflow-visible px-2 pb-2">
              {sections(here.map((r) => r.s), v.o.byProject).map((sec) => (
                <div key={sec.label ?? "all"} className="space-y-3">
                  {sec.label && <Label text={sec.label} />}
                  {sec.specs.map((s) => {
                    const r = here.find((x) => x.s.id === s.id)!;
                    return (
                      <div key={s.id} className="rounded-xl border border-line bg-ink-950/40 p-2">
                        <button onClick={() => v.openSpec(s.id)} className="mb-1.5 flex w-full items-center gap-2 text-left">
                          <span className="min-w-0 flex-1 truncate text-[12px] text-fg-2">{s.title}</span>
                          <span className="shrink-0 font-mono text-[10px] text-fg-3">{r.ts.length}/{rows.find((x) => x.s.id === s.id)!.ts.length}</span>
                        </button>
                        <div className="space-y-1.5">{r.ts.map((t) => <TicketCard key={t.key} t={t} d={r.d} dense={v.o.dense} agentById={v.agentById} open={v.openTicket} />)}</div>
                      </div>
                    );
                  })}
                </div>
              ))}
              {!here.length && <Empty text="Nothing here" />}
            </div>
          </section>
        );
      })}
    </div>
  );
}

const chip = (on: boolean) => cx("chip text-[12px]", on && "border-accent/40 text-accent");

/** The filters and options, on top of the board; they all live in the URL. */
function Bar2({ variant, filters, facets, setFilter, query, setQuery, params, setParams, shown, total }: {
  variant: BoardVariant; filters: Filters; facets: Facets; setFilter: (c: Partial<Filters>) => void; query: string; setQuery: (q: string) => void;
  params: URLSearchParams; setParams: (n: URLSearchParams) => void; shown: number; total: number;
}) {
  const o = parseBoard(params);
  const set = (c: Partial<BoardOptions>) => setParams(withBoard(params, c));
  const view = (v: string | null) => { const n = new URLSearchParams(params); if (v) n.set("specView", v); else n.delete("specView"); setParams(n); };
  return (
    <div className="space-y-2 border-b border-line px-4 pb-3 pt-4">
      <div className="flex flex-wrap items-center gap-2">
        <div><div className="eyebrow">Specs</div><div className="font-mono text-[11px] text-fg-3">{shown === total ? `${total} specs` : `${shown} of ${total} specs`}</div></div>
        <div className="flex items-center gap-0.5 rounded-full border border-line bg-ink-900 p-0.5" role="group" aria-label="Board layout">
          {BOARD_VARIANTS.map((b) => <button key={b} onClick={() => view(b)} aria-pressed={variant === b} className={cx("rounded-full px-2.5 py-0.5 font-mono text-[12px]", variant === b ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>{b}</button>)}
          <button onClick={() => view(null)} className="rounded-full px-2.5 py-0.5 font-mono text-[12px] text-fg-3 hover:text-fg">off</button>
        </div>
        <label className="flex min-w-[160px] flex-1 items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5 sm:max-w-xs">
          <Search className="size-3.5 text-fg-3" />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search specs, or #42" aria-label="Search specs" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
        </label>
        <select value={filters.group} onChange={(e) => setFilter({ group: e.target.value, project: "" })} className="chip bg-ink-850 text-[12px]" aria-label="Group">
          <option value="">All groups</option>{facets.teams.map((g) => <option key={g} value={g}>{g}</option>)}
        </select>
        <select value={filters.project} onChange={(e) => setFilter({ project: e.target.value })} className="chip bg-ink-850 text-[12px]" aria-label="Project">
          <option value="">All projects</option>{facets.projects.map((p) => <option key={p} value={p}>{p}</option>)}
        </select>
        <select value={filters.agent} onChange={(e) => setFilter({ agent: e.target.value })} className="chip bg-ink-850 text-[12px]" aria-label="Agent">
          <option value="">All agents</option>{facets.agents.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
        </select>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Spec state">
          {STAGES.map((st: { key: Stage; label: string }) => {
            const on = o.stages.includes(st.key);
            return <button key={st.key} aria-pressed={on} onClick={() => set({ stages: on ? o.stages.filter((x) => x !== st.key) : [...o.stages, st.key] })} className={chip(on)}>{st.label}</button>;
          })}
        </div>
        <span className="mx-1 h-4 w-px bg-line" />
        <button className={chip(o.needsMe)} aria-pressed={o.needsMe} onClick={() => set({ needsMe: !o.needsMe })} title="Ready to merge, or something needs a human">Needs me</button>
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

/** The whole page of a board variant. `specs` are the spec issues the shared filters (search, group, project, agent) kept. */
export function BoardPage({ variant, specs, filters, facets, setFilter, query, setQuery, params, setParams, openSpec, openTicket, ...c }: Ctx & {
  variant: BoardVariant; specs: Task[]; filters: Filters; facets: Facets; setFilter: (c: Partial<Filters>) => void; query: string; setQuery: (q: string) => void;
  params: URLSearchParams; setParams: (n: URLSearchParams) => void; openSpec: (id: string) => void; openTicket: (id: string) => void;
}) {
  const o = parseBoard(params);
  const [all, setAll] = useState(false);
  const kept = arrange(specs, c.deliveries, c.tasks, o).filter((s) => !filters.agent || deliveryOf(s, c.deliveries)?.agent_id === filters.agent || s.agent_id === filters.agent);
  // Shipped specs pile up: the newest few show, the rest on request.
  const shipped = kept.filter((s) => stageOf(s, deliveryOf(s, c.deliveries)) === "shipped");
  const hidden = all ? [] : shipped.slice(CAP);
  const shown = kept.filter((s) => !hidden.includes(s));
  const v: View = { ...c, specs: shown, o, openSpec, openTicket };
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <Bar2 variant={variant} filters={filters} facets={facets} setFilter={setFilter} query={query} setQuery={setQuery} params={params} setParams={setParams} shown={kept.length} total={specs.length} />
      {!c.deliveries.length && <div className="px-4 pt-3 font-mono text-[11px] text-fg-3">Loading Deliveries…</div>}
      <div className="flex min-h-0 flex-1 flex-col pt-3">
        {variant === "b1" ? <Swimlanes {...v} /> : variant === "b2" ? <Cards {...v} /> : <TicketBoard {...v} />}
        {hidden.length > 0 && <button onClick={() => setAll(true)} className="mx-4 mb-3 mt-1 self-start rounded-full border border-line px-3 py-1 font-mono text-[11px] text-fg-3 hover:text-fg">Show {hidden.length} older shipped</button>}
      </div>
    </div>
  );
}
