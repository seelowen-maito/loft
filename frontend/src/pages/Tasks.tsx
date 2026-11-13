import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation, useNavigate, type To, useParams, useSearchParams } from "react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft, ChevronRight, CircleCheck, CircleDot, ExternalLink, MessageSquare, NotebookPen, Plus, RefreshCw, Search, X } from "lucide-react";
import { api, type MirrorSync, type Agent, type Task, type TaskComment } from "../lib/api";
import { parseFilters, resolveFrom, withFilters, type IssueFilters } from "../lib/issueFilters";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx, timeAgo } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { Origin } from "../components/Origin";
import { GithubUser } from "../components/GithubUser";
import { createdByMe, filterIssues, SOURCES } from "../lib/issueFilter";
import { isDeliverableSpec, isStandaloneTicket } from "../lib/standaloneTicket";
import { ClosedOnGithubCard, CommentBox, DeliveryCard, HandOffCard, NewTaskDialog, PublishCard, SpecDeliveryCard, StuckCard, TaskActions } from "./TaskActions";
import { isSpec } from "../lib/issueOrder";
import { SpecPanel } from "./SpecViews";
import { BoardPage, boardSpecs } from "./SpecBoard";
import { boardIssues, NEEDS_YOU, parseBoard, withOpen, type DeliverySummary } from "../lib/specViews";
import { agentInGroup, groupAgents, groupProjects, inGroup, projectsOf, teamOf } from "../lib/team";

const STATUS_TONE: Record<string, string> = {
  waiting_approval: "text-warn",
  blocked: "text-warn",
  failed: "text-bad",
  in_progress: "text-accent",
  ready: "text-info",
  inbox: "text-fg-3",
  done: "text-good",
  cancelled: "text-fg-3",
};

const statusLabel = (s: string) => (s === "waiting_approval" ? "waiting for you" : s.replace("_", " "));

/** Open or closed, as on GitHub: the green dot, or the purple check (grey when cancelled, like "not planned"). */
function StateIcon({ task: t, className }: { task: Task; className?: string }) {
  if (t.state === "open") return <CircleDot className={cx(className, STATUS_TONE[t.status])} aria-label="Open" />;
  return <CircleCheck className={cx(className, t.status === "cancelled" ? "text-fg-3" : "text-merged")} aria-label="Closed" />;
}

function LabelChip({ name }: { name: string }) {
  return <span className="inline-flex items-center rounded-full border border-line-strong bg-white/[0.04] px-2 py-px font-sans text-[11px] text-fg-2">{name}</span>;
}

export default function TasksPage() {
  const { id: routeId } = useParams();
  const navigate = useNavigate();
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [repos, setRepos] = useState<Record<string, string>>({});
  // The filters live in the URL (lib/issueFilters), so back/forward, reload and shared links keep them.
  // A click is a history step; typing replaces the entry, once it pauses.
  const [params, setParams] = useSearchParams();
  const filters = useMemo(() => parseFilters(params), [params]);
  // The board (see ./SpecBoard): its open issue is `open=<id>`, or the route of an old `/issues/:id` link. An old `?view=specs` is ignored.
  const setFilter = useCallback(
    (change: Partial<IssueFilters>, replace = false) => setParams((cur) => withFilters(cur, change), { replace }),
    [setParams],
  );
  const [deliveries, setDeliveries] = useState<DeliverySummary[]>([]);
  const [deliveriesLoaded, setDeliveriesLoaded] = useState(false);
  const loadDeliveries = useCallback(() => { api.deliverySummaries().then(setDeliveries).catch(() => {}).finally(() => setDeliveriesLoaded(true)); }, []);
  useEffect(() => { loadDeliveries(); }, [loadDeliveries]);
  const [query, setQuery] = useState(filters.q);
  // Back/forward to another search shows it in the box.
  useEffect(() => setQuery(filters.q), [filters.q]);
  useEffect(() => {
    if (query.trim() === filters.q) return;
    const id = setTimeout(() => setFilter({ q: query }, true), 300);
    return () => clearTimeout(id);
  }, [query, filters.q, setFilter]);
  const { source, mine, closed: showClosed } = filters;
  const [creating, setCreating] = useState(false);

  const [syncing, setSyncing] = useState(false);
  // The last read of GitHub, so the list can say why GitHub's issues aren't there (rate limit, no repo...).
  const [mirror, setMirror] = useState<MirrorSync | null>(null);
  // Opening the list, and the refresh button, read the projects' GitHub issues; changes arrive as events.
  const sync = useCallback(() => {
    setSyncing(true);
    api.syncIssues().then(setMirror).catch(() => {}).finally(() => setSyncing(false));
  }, []);
  useEffect(() => {
    api.tasks().then(setTasks);
    api.agents().then(setAgents);
    // Which projects have a GitHub repo: only their Loft issues can be published there.
    api.repos().then((rs) => setRepos(Object.fromEntries(rs.filter((r) => r.github).map((r) => [r.name, r.github!])))).catch(() => {});
    sync();
  }, [sync]);

  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "task")
        setTasks((ts) => {
          if (!ts) return ts;
          const i = ts.findIndex((t) => t.id === e.task.id);
          if (i < 0) return [e.task, ...ts];
          const next = [...ts];
          next[i] = e.task;
          return next;
        });
      // The board reloads the Deliveries once after a burst of changes.
      if (e.type === "task") {
        clearTimeout(reload.current);
        reload.current = setTimeout(loadDeliveries, 1500);
      }
      if (e.type === "task_deleted") setTasks((ts) => ts?.filter((t) => t.id !== e.task_id) ?? ts);
    }, [loadDeliveries]),
  );
  const reload = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(reload.current), []);

  const agentById = useMemo(() => new Map(agents.map((a) => [a.id, a])), [agents]);
  const projects = useMemo(() => [...new Set((tasks ?? []).map((t) => t.project).filter(Boolean))].sort(), [tasks]);
  const teams = useMemo(() => [...new Set(agents.map(teamOf).filter(Boolean))].sort((a, b) => a.localeCompare(b)), [agents]);
  // Coming from an agent (the Issues tab passes `from`): its group, in place of `from`, once.
  const from = params.get("from");
  useEffect(() => {
    if (!from || !agents.length) return;
    const groupOf = (id: string) => {
      const a = agentById.get(id);
      return a ? teamOf(a) : "";
    };
    setParams((cur) => resolveFrom(cur, groupOf) ?? cur, { replace: true });
  }, [from, agents.length, agentById, setParams]);
  // A group, agent or project the URL names that doesn't exist (any more) is ignored, once the lists are in.
  const group = agents.length && !teams.includes(filters.group) ? "" : filters.group;
  const groupProjectList = useMemo(() => groupProjects(group, agents, projects), [group, agents, projects]);
  const groupAgentList = useMemo(() => groupAgents(group, agents), [group, agents]);
  // An agent outside the picked group is ignored, like an unknown one.
  const agentId = agents.length ? agentInGroup(filters.agent, group, agents) : filters.agent;
  const project = tasks && !groupProjectList.includes(filters.project) ? "" : filters.project;
  // Every filter but the agent: the board matches an agent on its own (a spec's, or its Delivery's, see boardSpecs).
  const anyAgent = useMemo(() => {
    return filterIssues(tasks ?? [], filters.q, source).filter((t) => (!group || inGroup(t, group, agents)) && (!project || t.project === project) && (!mine || createdByMe(t, mirror?.login)));
  }, [tasks, filters.q, group, agents, project, mine, source, mirror?.login]);
  const filtered = useMemo(() => anyAgent.filter((t) => !agentId || t.agent_id === agentId), [anyAgent, agentId]);

  const openId = params.get("open") ?? routeId ?? null;
  const selected = tasks?.find((t) => t.id === openId) ?? null;
  const up = params.get("up");
  const openOnBoard = useCallback((id: string | null, from?: string) => navigate({ pathname: "/issues", search: withOpen(params, id, from).toString() }), [navigate, params]);
  const to = (id: string): To => ({ pathname: "/issues", search: withOpen(params, id).toString() });
  const detail = selected ? (
    <TaskDetail task={selected} tasks={tasks ?? []} agents={agents} agentById={agentById} repos={repos} deliveries={deliveries} to={to}
      openTicket={(id) => openOnBoard(id, selected.id)}
      onClose={() => openOnBoard(null)} onBack={up && tasks?.some((t) => t.id === up) ? () => openOnBoard(up) : undefined} />
  ) : openId && tasks ? (
    <div className="p-10 text-center text-[13px] text-fg-3">This Loft issue no longer exists.</div>
  ) : null;
  const unread = filtered.filter((t) => NEEDS_YOU.includes(t.status)).length;
  const specs = useMemo(() => anyAgent.filter(isSpec), [anyAgent]);
  const kept = useMemo(() => boardSpecs(specs, deliveries, tasks ?? [], parseBoard(params), agentId), [specs, deliveries, tasks, params, agentId]);
  const issues = useMemo(() => boardIssues(filtered, deliveries, deliveriesLoaded, parseBoard(params), showClosed), [filtered, deliveries, deliveriesLoaded, params, showClosed]);
  const count = `${issues.length} issues · ${kept.length === specs.length ? specs.length : `${kept.length} of ${specs.length}`} specs`;

  // The header with the filters the Issues and the specs share.
  const header = (
    <div className="border-b border-line px-4 pb-3 pt-5">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="eyebrow">Loft issues</div>
          <div className="mt-1 font-mono text-[11px] text-fg-3">
            {unread > 0 && <span className="text-warn">{unread} need you · </span>}
            {(tasks ?? []).filter((t) => !["done", "cancelled"].includes(t.status)).length} open
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button className="btn-ghost size-8 justify-center p-0" onClick={sync} aria-label="Refresh from GitHub" title="Refresh from GitHub">
            <RefreshCw className={cx("size-4", syncing && "animate-spin")} />
          </button>
          <button className="btn-primary size-8 justify-center p-0" onClick={() => setCreating(true)} aria-label="New Loft issue" title="New Loft issue">
            <Plus className="size-4" />
          </button>
        </div>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <label className="flex min-w-[160px] flex-1 items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5 sm:max-w-xs">
          <Search className="size-3.5 text-fg-3" />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search issues, or #42" aria-label="Search issues" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
        </label>
        <div className="flex items-center gap-0.5 rounded-full border border-line bg-ink-900 p-0.5" role="group" aria-label="Source">
          {SOURCES.map((s) => (
            <button key={s.key} onClick={() => setFilter({ source: s.key })} aria-pressed={source === s.key} className={cx("flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[12px]", source === s.key ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
              {s.key !== "all" && <Origin github={s.key === "github"} />} {s.label}
            </button>
          ))}
        </div>
        <select value={group} onChange={(e) => setFilter({ group: e.target.value, project: "", agent: agentInGroup(agentId, e.target.value, agents) })} className="chip bg-ink-850 text-[12px]" aria-label="Group">
          <option value="">All groups</option>
          {teams.map((g) => <option key={g} value={g}>{g}</option>)}
        </select>
        <select value={project} onChange={(e) => setFilter({ project: e.target.value })} className="chip bg-ink-850 text-[12px]" aria-label="Project">
          <option value="">All projects</option>
          {groupProjectList.map((p) => <option key={p} value={p}>{p}</option>)}
        </select>
        <select value={agentId} onChange={(e) => setFilter({ agent: e.target.value })} className="chip bg-ink-850 text-[12px]" aria-label="Agent">
          <option value="">All agents</option>
          {groupAgentList.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
        </select>
        <button className={cx("chip text-[12px]", mine && "border-accent/40 text-accent")} onClick={() => setFilter({ mine: !mine })} aria-pressed={mine}>
          Created by me
        </button>
        {tasks && <span className="font-mono text-[11px] text-fg-3">{count}</span>}
      </div>
    </div>
  );

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {header}
      <div className="min-h-0 flex-1 overflow-hidden">
        {tasks === null ? <div className="p-6 font-mono text-[12px] text-fg-3">Loading…</div> : (
          <BoardPage kept={kept} issues={issues} tasks={tasks} deliveries={deliveries} loaded={deliveriesLoaded} agentById={agentById} params={params} setParams={(n) => setParams(n)} openSpec={(id) => openOnBoard(id)} openTicket={(id) => openOnBoard(id)} />
        )}
      </div>
      {openId && <Modal onClose={() => openOnBoard(null)}>{detail}</Modal>}
      {creating && <NewTaskDialog agents={agents} projects={projects} onClose={() => setCreating(false)} />}
    </div>
  );
}

/** The board's large modal over the full-width board: background click, ✕ or Esc close it; Tab stays inside and focus goes back to the card on close. Full screen at phone width. */
function Modal({ onClose, children }: { onClose: () => void; children: React.ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const from = document.activeElement as HTMLElement | null;
    box.current?.focus();
    return () => { if (from?.isConnected) from.focus(); };
  }, []);
  const onKey = (e: React.KeyboardEvent) => {
    const el = e.target as HTMLElement;
    if (e.key === "Escape") {
      // Not while a field or another dialog (new issue, confirm…) has it.
      if (e.defaultPrevented || el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || document.querySelectorAll('[role="dialog"]').length > 1) return;
      e.stopPropagation();
      close.current();
    } else if (e.key === "Tab" && box.current) {
      const f = [...box.current.querySelectorAll<HTMLElement>('a[href],button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])')];
      if (f.length === 0) return e.preventDefault();
      const first = f[0], last = f[f.length - 1];
      if (e.shiftKey && (el === first || el === box.current)) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && el === last) { e.preventDefault(); first.focus(); }
    }
  };
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/60 backdrop-blur-[2px] sm:p-6" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div ref={box} role="dialog" aria-modal="true" aria-label="Issue" tabIndex={-1} onKeyDown={onKey}
        className="h-full w-full animate-rise overflow-y-auto bg-ink-950 shadow-2xl outline-none sm:h-auto sm:max-h-full sm:max-w-[960px] sm:rounded-2xl sm:border sm:border-line-strong">
        {children}
      </div>
    </div>
  );
}

/** An issue's page. In the board it's the modal over it: `onClose` is its ✕, `onBack` the arrow to the spec it was opened from. `to` is where a link to another issue goes. */
function TaskDetail({ task: t, tasks, agents, agentById, repos, deliveries, to, openTicket, onClose, onBack }: { task: Task; tasks: Task[]; agents: Agent[]; agentById: Map<string, Agent>; repos: Record<string, string>; deliveries: DeliverySummary[]; to: (id: string) => To; openTicket: (id: string) => void; onClose?: () => void; onBack?: () => void }) {
  const [thread, setThread] = useState<TaskComment[] | null>(null);
  const [editAsked, setEditAsked] = useState(0);
  // Links keep the list's filters (they're in the URL).
  const { search } = useLocation();
  useEffect(() => {
    setThread(null);
    api.taskComments(t.id).then(setThread).catch(() => setThread([]));
  }, [t.id]);
  // A finished issue's result was waiting in Needs you: opening it is looking at it.
  const done = t.status === "done";
  useEffect(() => {
    if (done) api.issueOpened(t.id).catch(() => {});
  }, [t.id, done]);
  useServerEvents(
    useCallback(
      (e: ServerEvent) => {
        if (e.type === "task_comment" && e.comment.task_id === t.id)
          // A comment approved or kept comes back with its new GitHub state: replace it in place.
          setThread((th) => (!th ? th : th.some((c) => c.id === e.comment.id) ? th.map((c) => (c.id === e.comment.id ? e.comment : c)) : [...th, e.comment]));
      },
      [t.id],
    ),
  );

  const agent = t.agent_id ? agentById.get(t.agent_id) : undefined;
  const creator = t.created_by === "user" ? "you" : t.created_by === "github" ? (t.github?.author ? `@${t.github.author} on GitHub` : "GitHub") : agentById.get(t.created_by)?.name ?? "an agent";
  const parent = t.parent_id ? tasks.find((x) => x.id === t.parent_id) : undefined;
  const subtasks = tasks.filter((x) => x.parent_id === t.id);
  const deps = t.depends_on.map((id) => tasks.find((x) => x.id === id)).filter((x): x is Task => !!x);
  const blocking = tasks.filter((x) => x.depends_on.includes(t.id));

  return (
    <div className="mx-auto max-w-[820px] space-y-6 px-6 py-6">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 empty:hidden">
        {onBack && (
          <button onClick={onBack} className="inline-flex items-center gap-1 font-mono text-[11px] text-fg-3 hover:text-fg">
            <ArrowLeft className="size-3" /> Back to the spec
          </button>
        )}
        {!onClose && (
          <Link to={{ pathname: "/issues", search }} className="inline-flex items-center gap-1 font-mono text-[11px] text-fg-3 hover:text-fg lg:hidden">
            <ArrowLeft className="size-3" /> All issues
          </Link>
        )}
        {onClose && (
          <button onClick={onClose} className="btn-ghost -mr-2 ml-auto size-7 justify-center p-0" aria-label="Close" title="Close (Esc)">
            <X className="size-4" />
          </button>
        )}
      </div>
      <header className="space-y-2">
        {parent && (
          <Link to={to(parent.id)} className="inline-flex items-center gap-1 font-mono text-[11px] text-fg-3 hover:text-fg">
            {isSpec(parent) && parent.github ? `Part of spec #${parent.github.number}: ${parent.title}` : `Part of ${parent.title}`} <ChevronRight className="size-3" />
          </Link>
        )}
        <h1 className="text-[22px] font-medium leading-snug tracking-tight">{t.title}</h1>
        <div className="flex flex-wrap items-center gap-2 font-mono text-[11px]">
          <span className={cx("inline-flex items-center gap-1 rounded-full px-2 py-0.5 font-sans text-[12px] font-medium text-ink-950", t.state === "open" ? "bg-good" : t.status === "cancelled" ? "bg-fg-3" : "bg-merged")}>
            {t.state === "open" ? <CircleDot className="size-3" /> : <CircleCheck className="size-3" />}
            {t.state === "open" ? "Open" : t.status === "cancelled" ? "Closed · not planned" : "Closed"}
          </span>
          <span className={cx("chip", STATUS_TONE[t.status])}>{statusLabel(t.status)}</span>
          {t.priority !== "normal" && <span className={cx("chip", (t.priority === "high" || t.priority === "urgent") && "text-warn")}>{t.priority}</span>}
          {t.project && <span className="chip">{t.project}</span>}
          {t.github && (
            <a href={t.github.url} target="_blank" rel="noreferrer" className="chip gap-1 hover:text-fg" title={`Mirrors ${t.github.repo}#${t.github.number} on GitHub (${t.github.state} there)`}>
              {t.github.repo}#{t.github.number} <ExternalLink className="size-3" />
            </a>
          )}
          <span className="text-fg-3">created by {creator} · {timeAgo(t.created_at)}</span>
          {t.due_at && <span className="text-fg-3">· due {new Date(t.due_at).toLocaleDateString()}</span>}
        </div>
        {!t.github && (t.labels.length > 0 || t.owner) && (
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2 pt-1">
            {t.labels.length > 0 && (
              <div className="flex flex-wrap items-center gap-1">
                <span className="eyebrow mr-1">Labels</span>
                {t.labels.map((l) => <LabelChip key={l} name={l} />)}
              </div>
            )}
            {t.owner && (
              <div className="flex items-center gap-1.5 text-[12.5px] text-fg-2">
                <span className="eyebrow mr-1">Owner</span>
                <GithubUser login={t.owner} size={16} />
              </div>
            )}
          </div>
        )}
        <div className="pt-2">
          <TaskActions task={t} agents={agents} edit={editAsked} projects={[...new Set([...Object.keys(repos), ...agents.flatMap(projectsOf)])].sort()} />
        </div>
      </header>

      {t.state === "open" && t.github?.state === "closed" && ["in_progress", "blocked", "waiting_approval"].includes(t.status) && <ClosedOnGithubCard task={t} />}
      {(t.status === "blocked" || t.status === "failed") && <StuckCard task={t} agents={agents} />}
      {isSpec(t) && <SpecPanel spec={t} tasks={tasks} deliveries={deliveries} agentById={agentById} openTicket={openTicket} />}
      {isStandaloneTicket(t) && <DeliveryCard key={t.id} task={t} />}
      {isDeliverableSpec(t) && <SpecDeliveryCard key={t.id} task={t} />}
      {t.state === "open" && !t.agent_id && !isSpec(t) && !isStandaloneTicket(t) && agents.length > 0 && <HandOffCard key={t.id} task={t} agents={agents} />}
      {t.state === "open" && !t.github && <PublishCard task={t} repo={repos[t.project]} onEdit={() => setEditAsked((n) => n + 1)} />}
      {t.github && <OwnedBy task={t} agent={agent} deps={deps} />}

      {agent && (
        <Link to={`/agents/${agent.id}`} className="panel flex items-center gap-3 p-3 transition-colors hover:bg-white/[0.03]">
          <Sticker avatar={agent.avatar} lead={agent.role === "lead"} size={36} />
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px]">{agent.name}</div>
            <div className="truncate font-mono text-[11px] text-fg-3">{agent.running ? agent.status_text || "Working…" : "Open conversation"}</div>
          </div>
          <MessageSquare className="size-4 text-fg-3" />
        </Link>
      )}

      {t.error && !["done", "blocked", "failed"].includes(t.status) && <Block title="Problem" tone="text-bad" text={t.error} />}
      {t.summary && <Block title="Summary" text={t.summary} />}
      {t.note && t.status !== "done" && <Block title="Latest progress" text={t.note} />}
      {t.description && <Block title="Description" text={t.description} />}

      {(subtasks.length > 0 || deps.length > 0 || blocking.length > 0) && (
        <div className="grid gap-4 sm:grid-cols-2">
          {subtasks.length > 0 && <TaskLinks title="Sub-issues" tasks={subtasks} to={to} />}
          {deps.length > 0 && <TaskLinks title="Blocked by" tasks={deps} to={to} />}
          {blocking.length > 0 && <TaskLinks title="Blocking" tasks={blocking} to={to} />}
        </div>
      )}

      <section>
        <div className="eyebrow mb-3">Thread</div>
        {thread === null ? (
          <div className="font-mono text-[12px] text-fg-3">Loading…</div>
        ) : thread.length === 0 ? (
          <div className="text-[13px] text-fg-3">Nothing yet. Comments, progress notes and status changes show up here.</div>
        ) : (
          <ol className="space-y-3">
            {thread.map((c) => <ThreadEntry key={c.id} c={c} agent={c.author === "user" ? undefined : agentById.get(c.author)} />)}
          </ol>
        )}
        <div className="mt-4">
          <CommentBox task={t} agent={agent} agents={agents} />
        </div>
      </section>
    </div>
  );
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 py-1.5 text-[13px]">
      <span className="text-fg-3">{label}</span>
      <span className="flex min-w-0 flex-wrap items-center justify-end gap-1.5 text-fg-2">{children}</span>
    </div>
  );
}

/** A mirrored issue's fields, split by who owns them: GitHub (edits go there) and Loft (never sent). */
function OwnedBy({ task: t, agent, deps }: { task: Task; agent?: Agent; deps: Task[] }) {
  const g = t.github!;
  const open = deps.filter((d) => d.state === "open").length;
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <section className="panel p-4">
        <div className="eyebrow mb-2">On GitHub · {g.repo}#{g.number}</div>
        <Fact label="State">{g.state === "open" ? <><CircleDot className="size-3.5 text-good" /> Open</> : <><CircleCheck className="size-3.5 text-merged" /> Closed</>}</Fact>
        <Fact label="Labels">{t.labels.length ? t.labels.map((l) => <LabelChip key={l} name={l} />) : <span className="text-fg-3">none</span>}</Fact>
        <Fact label="Owner">{t.owner ? <GithubUser login={t.owner} size={16} /> : <span className="text-fg-3">nobody</span>}</Fact>
        <Fact label="Blocked by">{deps.length ? `${open} open of ${deps.length}` : <span className="text-fg-3">nothing</span>}</Fact>
        <div className="mt-2 font-mono text-[10.5px] text-fg-3">Title, description, labels, owner, open/closed and links live on GitHub; edits here go there.</div>
      </section>
      <section className="panel p-4">
        <div className="eyebrow mb-2">In Loft</div>
        <Fact label="Status"><span className={STATUS_TONE[t.status]}>{statusLabel(t.status)}</span></Fact>
        <Fact label="Agent">{agent ? <><Sticker avatar={agent.avatar} lead={agent.role === "lead"} size={18} /> {agent.name}</> : <span className="text-fg-3">nobody yet</span>}</Fact>
        <Fact label="Priority">{t.priority}</Fact>
        <Fact label="When done">closes #{g.number} on GitHub</Fact>
        <div className="mt-2 font-mono text-[10.5px] text-fg-3">Who works on it, its progress, priority and notes stay in Loft.</div>
      </section>
    </div>
  );
}

function Block({ title, text, tone }: { title: string; text: string; tone?: string }) {
  return (
    <section>
      <div className={cx("eyebrow mb-2", tone)}>{title}</div>
      <div className="prose-chat text-fg-2">
        <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
      </div>
    </section>
  );
}

function TaskLinks({ title, tasks, to }: { title: string; tasks: Task[]; to: (id: string) => To }) {
  return (
    <section>
      <div className="eyebrow mb-2">{title}</div>
      <div className="space-y-1">
        {tasks.map((x) => (
          <Link key={x.id} to={to(x.id)} className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-[13px] hover:bg-white/[0.03]">
            <StateIcon task={x} className="size-3 shrink-0" />
            <span className="truncate">{x.title}</span>
          </Link>
        ))}
      </div>
    </section>
  );
}

function ThreadEntry({ c, agent }: { c: TaskComment; agent?: Agent }) {
  const name = c.author === "user" ? "You" : agent?.name ?? c.meta.author_name ?? "Agent";
  const gh = c.meta.github?.state;
  const when = new Date(c.created_at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  if (c.kind === "status") {
    const extra = c.meta.summary || c.meta.error;
    return (
      <li className="flex gap-3 pl-1">
        <span className={cx("mt-1.5 size-1.5 shrink-0 rounded-full bg-current", STATUS_TONE[c.meta.status ?? ""] ?? "text-fg-3")} />
        <div className="min-w-0 flex-1">
          <div className="font-mono text-[11.5px] text-fg-2">
            {c.text} <span className="text-fg-3">· {when}</span>
          </div>
          {extra && (
            <div className={cx("prose-chat mt-1 text-[13px]", c.meta.error ? "text-bad" : "text-fg-2")}>
              <Markdown remarkPlugins={[remarkGfm]}>{extra}</Markdown>
            </div>
          )}
        </div>
      </li>
    );
  }
  return (
    <li className="flex gap-3">
      {agent ? (
        <Sticker avatar={agent.avatar} lead={agent.role === "lead"} size={26} />
      ) : c.author === "github" ? (
        <span className="flex size-[26px] shrink-0 items-center justify-center rounded-full bg-white/[0.08] font-mono text-[10px] text-fg-2">gh</span>
      ) : (
        <span className="flex size-[26px] shrink-0 items-center justify-center rounded-full bg-accent/15 font-mono text-[11px] text-accent">you</span>
      )}
      <div className={cx("min-w-0 flex-1 rounded-xl border px-3 py-2", c.kind === "note" ? "border-line bg-transparent" : "border-line-strong bg-ink-850")}>
        <div className="mb-1 flex items-center gap-1.5 font-mono text-[10.5px] text-fg-3">
          {c.kind === "note" && <NotebookPen className="size-3" />}
          <span className="text-fg-2">{name}</span>
          <span>· {c.kind === "note" ? "progress note" : "comment"}</span>
          {c.meta.private && <span className="text-warn">· private</span>}
          {gh === "pending" && <span className="text-warn">· not on GitHub yet{c.meta.github?.auto ? " (auto-approved, posting)" : ""}</span>}
          {gh === "posted" && <span className="text-good">· on GitHub{c.meta.github?.auto ? ", auto-approved" : ""}</span>}
          {gh === "kept" && <span>· kept in Loft</span>}
          {gh === "from_github" && <span>· from GitHub</span>}
          <span>· {when}</span>
        </div>
        <div className="prose-chat text-[13.5px]">
          <Markdown remarkPlugins={[remarkGfm]}>{c.text}</Markdown>
        </div>
        {gh === "pending" && !c.meta.github?.auto && <Approval c={c} />}
      </div>
    </li>
  );
}

/** Approve (optionally after editing) or Keep in Loft, for a comment waiting to go to GitHub. */
function Approval({ c }: { c: TaskComment }) {
  const [editing, setEditing] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-2 space-y-2 border-t border-line pt-2">
      {editing !== null && (
        <textarea value={editing} onChange={(e) => setEditing(e.target.value)} rows={3} aria-label="Text to post" className="w-full resize-y rounded-lg border border-line-strong bg-ink-900 px-2.5 py-1.5 text-[13px] outline-none" />
      )}
      <div className="flex flex-wrap items-center gap-2">
        <button className="btn-primary" disabled={busy} onClick={() => run(() => api.approveComment(c.task_id, c.id, editing ?? undefined))}>
          {editing === null ? "Approve" : "Post edited"}
        </button>
        {editing === null && <button className="btn" disabled={busy} onClick={() => setEditing(c.text)}>Edit first</button>}
        <button className="btn-ghost" disabled={busy} onClick={() => run(() => api.keepComment(c.task_id, c.id))}>Keep in Loft</button>
        {error && <span className="text-[12px] text-bad">{error}</span>}
      </div>
    </div>
  );
}
