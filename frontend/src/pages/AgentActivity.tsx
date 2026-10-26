import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router";
import { AlertCircle, ArrowRight, Brain, GitMerge, GitPullRequest, Plus, ShieldAlert, Wrench } from "lucide-react";
import { api, type AgentSummary } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx, fmtSchedule, timeAgo } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { Tip } from "../components/Charts";
import { StatusPill } from "./agents/Status";

const USER = "var(--color-series-user)";
const AUTO = "var(--color-series-auto)";

function sum(a: number[]) {
  return a.reduce((x, y) => x + y, 0);
}

function dayLabel(offsetFromToday: number) {
  const d = new Date();
  d.setDate(d.getDate() - offsetFromToday);
  return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

function toolName(n: string) {
  return n.startsWith("mcp__loft__") ? n.slice(15) : n.replace(/^mcp__/, "").replace("__", " › ");
}

/** Stacked daily runs: on request (bottom) + autonomous (top), 2px gap between segments. */
function RunBars({ user, auto, height = 56 }: { user: number[]; auto: number[]; height?: number }) {
  const [hover, setHover] = useState<number | null>(null);
  const n = user.length;
  const max = Math.max(1, ...user.map((u, i) => u + auto[i]));
  const slot = 100 / n;
  const bw = slot * 0.62;
  const unit = (height - 4) / max;
  return (
    <div className="relative" onMouseLeave={() => setHover(null)}>
      <svg viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" className="block w-full" style={{ height }} role="img" aria-label="Runs per day">
        <line x1={0} x2={100} y1={height - 0.5} y2={height - 0.5} stroke="var(--color-line-strong)" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        {user.map((u, i) => {
          const a = auto[i];
          const x = i * slot + (slot - bw) / 2;
          const hu = u * unit;
          const ha = a * unit;
          const gap = u && a ? 2 : 0;
          const dim = hover !== null && hover !== i ? 0.4 : 1;
          return (
            <g key={i} onMouseEnter={() => setHover(i)} opacity={dim}>
              <rect x={i * slot} y={0} width={slot} height={height} fill="transparent" />
              {u > 0 && <rect x={x} y={height - hu} width={bw} height={hu} rx={0.8} fill={USER} />}
              {a > 0 && <rect x={x} y={height - hu - gap - ha} width={bw} height={ha} rx={0.8} fill={AUTO} />}
            </g>
          );
        })}
      </svg>
      {hover !== null && (
        <Tip x={hover >= n - 4 ? `${((hover + 1) / n) * 100}%` : `${((hover + 0.5) / n) * 100}%`} y={0} align={hover >= n - 4 ? "end" : "center"}>
          <span className="text-fg-3">{dayLabel(n - 1 - hover)} · </span>
          <span className="text-fg">{user[hover]} on request</span>
          <span className="text-fg-3"> · </span>
          <span className="text-fg">{auto[hover]} autonomous</span>
        </Tip>
      )}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: React.ReactNode; tone?: string }) {
  return (
    <div>
      <div className={cx("font-serif text-[24px] leading-none tabular-nums", tone)}>{value}</div>
      <div className="mt-1 font-mono text-[10px] uppercase tracking-wider text-fg-3">{label}</div>
    </div>
  );
}

function AgentCard({ s, index }: { s: AgentSummary; index: number }) {
  const a = s.agent;
  const runs = sum(s.daily_user) + sum(s.daily_auto);
  const reply = s.last_reply?.text.replace(/[*_`#>]/g, "").replace(/\s+/g, " ").trim();
  return (
    <Link
      to={`/agents/${a.id}`}
      className="panel group relative flex animate-rise flex-col p-5 transition hover:z-10 hover:border-white/15 hover:bg-ink-850"
      style={{ animationDelay: `${Math.min(index, 8) * 40}ms` }}
    >
      <div className="flex items-start gap-3">
        <Sticker avatar={a.avatar} size={44} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[15px] font-medium tracking-tight group-hover:text-accent">{a.name}</span>
            {s.pending_permissions > 0 && (
              <span className="flex shrink-0 items-center gap-1 rounded-full border border-warn/30 bg-warn/10 px-1.5 py-px font-mono text-[10px] text-warn">
                <ShieldAlert className="size-3" /> {s.pending_permissions} waiting
              </span>
            )}
          </div>
          <div className="mt-1">
            <StatusPill agent={a} />
          </div>
        </div>
        <ArrowRight className="size-4 shrink-0 text-fg-3 opacity-0 transition group-hover:translate-x-0.5 group-hover:opacity-100" />
      </div>

      <div className="mt-5 grid grid-cols-4 gap-3">
        <Stat label="Runs" value={runs} />
        <Stat label="Tool calls" value={s.tool_calls} />
        <Stat label="PRs" value={s.prs.total} />
        <Stat label="Spent" value={`$${s.cost_usd < 10 ? s.cost_usd.toFixed(2) : Math.round(s.cost_usd)}`} />
      </div>

      <div className="mt-5">
        <RunBars user={s.daily_user} auto={s.daily_auto} />
        <div className="mt-1.5 flex justify-between font-mono text-[10px] text-fg-3">
          <span>{s.days}d ago</span>
          <span>today</span>
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5">
        {s.prs.total > 0 && (
          <span className="chip">
            {s.prs.open > 0 ? <GitPullRequest className="size-3 text-good" /> : <GitMerge className="size-3 text-merged" />}
            {s.prs.open > 0 && <>{s.prs.open} open · </>}
            {s.prs.merged} merged
          </span>
        )}
        {s.last_error && (
          <span className="chip text-bad" title={s.last_error}>
            <AlertCircle className="size-3" /> error
          </span>
        )}
        <span className="chip">
          <Brain className="size-3" /> {s.memories} memor{s.memories === 1 ? "y" : "ies"}
        </span>
        {s.top_tools.slice(0, 2).map(([n, k]) => (
          <span key={n} className="chip">
            <Wrench className="size-3" /> {toolName(n)} <span className="text-fg-3">×{k}</span>
          </span>
        ))}
      </div>

      <div className="mt-auto pt-4">
        <div className="border-t border-line pt-3">
          {reply ? (
            <>
              <p className="line-clamp-2 text-[12.5px] leading-relaxed text-fg-2">{reply}</p>
              <div className="mt-1.5 flex justify-between font-mono text-[10.5px] text-fg-3">
                <span>last reply {timeAgo(s.last_reply!.at)}</span>
                <span>{fmtSchedule(a)}</span>
              </div>
            </>
          ) : (
            <p className="text-[12.5px] text-fg-3">No replies yet.</p>
          )}
        </div>
      </div>
    </Link>
  );
}

export function AgentActivity() {
  const [data, setData] = useState<AgentSummary[] | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = useCallback(() => {
    api.agentsSummary().then(setData).catch(() => setData((d) => d ?? []));
  }, []);
  useEffect(load, [load]);

  // Status changes patch in place; anything that moves the numbers triggers a debounced refetch.
  useServerEvents(
    useCallback(
      (e: ServerEvent) => {
        if (e.type === "agent_updated") {
          setData((d) => d?.map((s) => (s.agent.id === e.agent.id ? { ...s, agent: { ...s.agent, ...e.agent } } : s)) ?? d);
        }
        const moves =
          (e.type === "message" && ["run_start", "run_end", "error", "text"].includes(e.message.kind)) ||
          e.type === "pull_request" || e.type === "permission" || e.type === "memory" ||
          e.type === "agent_deleted" || (e.type === "agent_updated" && !data?.some((s) => s.agent.id === e.agent.id));
        if (moves) {
          if (timer.current) clearTimeout(timer.current);
          timer.current = setTimeout(load, 800);
        }
      },
      [load, data],
    ),
  );

  const totals = useMemo(() => {
    const d = data ?? [];
    return {
      working: d.filter((s) => s.agent.running).length,
      waiting: d.reduce((a, s) => a + s.pending_permissions, 0),
      runs: d.reduce((a, s) => a + sum(s.daily_user) + sum(s.daily_auto), 0),
      cost: d.reduce((a, s) => a + s.cost_usd, 0),
    };
  }, [data]);

  return (
    <section>
      <div className="mb-4 flex flex-wrap items-end gap-x-6 gap-y-2">
        <div className="mr-auto">
          <div className="eyebrow">Agents · last 14 days</div>
          <div className="mt-1 text-[15px]">
            {data === null ? (
              " "
            ) : (
              <>
                <span className={totals.working ? "text-accent" : "text-fg-2"}>{totals.working} working</span>
                {totals.waiting > 0 && <span className="text-warn"> · {totals.waiting} waiting on you</span>}
                <span className="text-fg-2"> · {totals.runs} runs · ${totals.cost.toFixed(2)} spent</span>
              </>
            )}
          </div>
        </div>
        <div className="flex items-center gap-4 font-mono text-[11px] text-fg-3">
          <span className="flex items-center gap-1.5">
            <span className="inline-block size-2.5 rounded-[2px]" style={{ background: USER }} /> on request
          </span>
          <span className="flex items-center gap-1.5">
            <span className="inline-block size-2.5 rounded-[2px]" style={{ background: AUTO }} /> autonomous
          </span>
          <Link to="/agents" className="btn ml-2">
            Open agents <ArrowRight className="size-3.5" />
          </Link>
        </div>
      </div>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        {data === null && [0, 1, 2].map((i) => <div key={i} className="skeleton h-[300px] rounded-2xl" />)}
        {data?.map((s, i) => <AgentCard key={s.agent.id} s={s} index={i} />)}
        {data?.length === 0 && (
          <Link to="/agents" className="panel flex flex-col items-center justify-center gap-2 border-dashed p-8 text-center text-fg-2 transition hover:border-accent/40 hover:text-fg sm:col-span-2 xl:col-span-3">
            <Plus className="size-5 text-accent" />
            <span className="text-[14px]">No agents yet. Create one and its activity shows up here.</span>
          </Link>
        )}
      </div>
    </section>
  );
}
