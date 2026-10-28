import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft, ChevronRight, CircleCheck, CircleDot, ExternalLink, ListTodo, MessageSquare, NotebookPen, Plus, RefreshCw, Search, UserRound } from "lucide-react";
import { api, type Agent, type Task, type TaskComment, type TaskStatus } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx, timeAgo } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { CommentBox, NewTaskDialog, TaskActions } from "./TaskActions";
import { compareOpen, specProgress } from "../lib/issueOrder";

/** The list's sections, in order; done and cancelled tasks are folded away. */
const GROUPS: { key: string; label: string; statuses: TaskStatus[] }[] = [
  { key: "you", label: "Needs you", statuses: ["waiting_approval", "blocked", "failed"] },
  { key: "progress", label: "In progress", statuses: ["in_progress"] },
  { key: "ready", label: "Ready", statuses: ["ready"] },
  { key: "inbox", label: "Inbox", statuses: ["inbox"] },
  { key: "closed", label: "Done", statuses: ["done", "cancelled"] },
];

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

type Source = "all" | "loft" | "github";
const SOURCES: { key: Source; label: string }[] = [
  { key: "all", label: "All" },
  { key: "loft", label: "Loft" },
  { key: "github", label: "GitHub" },
];

/** Where an issue lives: the Loft house for a Loft-only one, GitHub's mark for one mirroring GitHub. */
function Origin({ github }: { github: boolean }) {
  return github ? (
    <svg viewBox="0 0 16 16" className="size-3 shrink-0 text-fg-2" fill="currentColor" role="img" aria-label="On GitHub">
      <title>On GitHub</title>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  ) : (
    <svg viewBox="0 0 8 8" className="size-3 shrink-0 text-accent" shapeRendering="crispEdges" fill="currentColor" role="img" aria-label="Loft only">
      <title>Loft only</title>
      <path d="M3 0h2v1H3zM2 1h4v1H2zM1 2h6v1H1zM0 3h8v1H0zM1 4h1v4H1zM6 4h1v4H6zM2 7h4v1H2zM3 5h2v2H3z" />
    </svg>
  );
}

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
  // The old Issues tab's links arrive as ?source=github, or ?q=#42&project=api.
  const [params] = useSearchParams();
  const [query, setQuery] = useState(params.get("q") ?? "");
  const [project, setProject] = useState(params.get("project") ?? "");
  const [source, setSource] = useState<Source>((params.get("source") as Source) ?? "all");
  const [agentId, setAgentId] = useState("");
  const [mine, setMine] = useState(false);
  const [showClosed, setShowClosed] = useState(false);
  const [creating, setCreating] = useState(false);

  const [syncing, setSyncing] = useState(false);
  // Opening the list, and the refresh button, read the projects' GitHub issues; changes arrive as events.
  const sync = useCallback(() => {
    setSyncing(true);
    api.syncIssues().catch(() => {}).finally(() => setSyncing(false));
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
      if (e.type === "task_deleted") setTasks((ts) => ts?.filter((t) => t.id !== e.task_id) ?? ts);
    }, []),
  );

  const agentById = useMemo(() => new Map(agents.map((a) => [a.id, a])), [agents]);
  const projects = useMemo(() => [...new Set((tasks ?? []).map((t) => t.project).filter(Boolean))].sort(), [tasks]);
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    // "#42" finds the issue mirroring GitHub #42, and only it.
    const number = /^#\d+$/.test(q) ? Number(q.slice(1)) : null;
    return (tasks ?? []).filter(
      (t) =>
        (number !== null ? t.github?.number === number : !q || t.title.toLowerCase().includes(q) || t.description.toLowerCase().includes(q) || t.labels.some((l) => l.toLowerCase().includes(q)) || t.owner.toLowerCase() === q.replace(/^@/, "")) &&
        (source === "all" || (source === "github") === !!t.github) &&
        (!project || t.project === project) &&
        (!agentId || t.agent_id === agentId) &&
        (!mine || t.created_by === "user"),
    );
  }, [tasks, query, project, agentId, mine, source]);

  const groups = useMemo(
    () =>
      GROUPS.map((g) => ({
        ...g,
        tasks: filtered
          .filter((t) => g.statuses.includes(t.status))
          .sort((a, b) =>
            g.key === "closed"
              ? (b.completed_at ?? b.updated_at).localeCompare(a.completed_at ?? a.updated_at)
              : compareOpen(a, b),
          ),
      })),
    [filtered],
  );

  const selected = tasks?.find((t) => t.id === routeId) ?? null;

  return (
    <div className="grid h-full grid-cols-[400px_minmax(0,1fr)] overflow-hidden max-lg:grid-cols-1">
      {/* list */}
      <aside className={cx("flex min-h-0 flex-col border-r border-line bg-ink-950", routeId && "max-lg:hidden")}>
        <div className="space-y-3 px-4 pb-3 pt-5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <div className="eyebrow">Loft issues</div>
              <div className="mt-1 font-mono text-[11px] text-fg-3">
                {groups[0].tasks.length > 0 && <span className="text-warn">{groups[0].tasks.length} need you · </span>}
                {(tasks ?? []).filter((t) => !["done", "cancelled"].includes(t.status)).length} open
              </div>
            </div>
            <div className="flex items-center gap-1">
            <button className="btn-ghost size-8 justify-center p-0" onClick={sync} aria-label="Refresh from GitHub" title="Refresh from GitHub">
              <RefreshCw className={cx("size-4", syncing && "animate-spin")} />
            </button>
            <button className="btn-primary size-8 justify-center p-0" onClick={() => setCreating(true)} aria-label="New Loft issue" title="New Loft issue">
              <Plus className="size-4" />
            </button>
            </div>
          </div>
          <label className="flex items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5">
            <Search className="size-3.5 text-fg-3" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search issues, or #42" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
          </label>
          <div className="flex flex-wrap gap-2">
            <div className="flex items-center gap-0.5 rounded-full border border-line bg-ink-900 p-0.5" role="group" aria-label="Source">
              {SOURCES.map((s) => (
                <button key={s.key} onClick={() => setSource(s.key)} aria-pressed={source === s.key} className={cx("flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[12px]", source === s.key ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
                  {s.key !== "all" && <Origin github={s.key === "github"} />} {s.label}
                </button>
              ))}
            </div>
            <select value={project} onChange={(e) => setProject(e.target.value)} className="chip bg-ink-850 text-[12px]" aria-label="Project">
              <option value="">All projects</option>
              {projects.map((p) => <option key={p} value={p}>{p}</option>)}
            </select>
            <select value={agentId} onChange={(e) => setAgentId(e.target.value)} className="chip bg-ink-850 text-[12px]" aria-label="Agent">
              <option value="">All agents</option>
              {agents.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
            </select>
            <button className={cx("chip text-[12px]", mine && "border-accent/40 text-accent")} onClick={() => setMine((m) => !m)} aria-pressed={mine}>
              Created by me
            </button>
          </div>
        </div>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-2 pb-6">
          {tasks === null && <div className="px-3 font-mono text-[12px] text-fg-3">Loading…</div>}
          {tasks?.length === 0 && (
            <div className="flex flex-col items-center px-6 py-16 text-center text-fg-3">
              <ListTodo className="mb-3 size-8" />
              <div className="text-[13px]">No Loft issues yet. Agents add them as they work, or add one with +.</div>
            </div>
          )}
          {groups.map((g) => {
            if (!g.tasks.length) return null;
            const folded = g.key === "closed" && !showClosed;
            return (
              <section key={g.key}>
                <button
                  className="flex w-full items-center gap-1.5 px-3 py-1 text-left font-mono text-[10.5px] uppercase tracking-wider text-fg-3"
                  onClick={() => g.key === "closed" && setShowClosed((s) => !s)}
                  disabled={g.key !== "closed"}
                >
                  {g.key === "closed" && <ChevronRight className={cx("size-3 transition-transform", showClosed && "rotate-90")} />}
                  <span className={g.key === "you" ? "text-warn" : ""}>{g.label}</span>
                  <span>· {g.tasks.length}</span>
                </button>
                {!folded &&
                  g.tasks.slice(0, g.key === "closed" ? 50 : undefined).map((t) => (
                    <TaskRow key={t.id} task={t} progress={specProgress(t, tasks ?? [])} agent={t.agent_id ? agentById.get(t.agent_id) : undefined} active={t.id === routeId} onOpen={() => navigate(`/tasks/${t.id}`)} />
                  ))}
              </section>
            );
          })}
        </div>
      </aside>

      {/* detail */}
      <section className={cx("min-h-0 overflow-y-auto", !routeId && "max-lg:hidden")}>
        {selected ? (
          <TaskDetail task={selected} tasks={tasks ?? []} agents={agents} agentById={agentById} repos={repos} />
        ) : routeId && tasks ? (
          <div className="p-10 text-center text-[13px] text-fg-3">This Loft issue no longer exists.</div>
        ) : (
          <div className="flex h-full flex-col items-center justify-center p-10 text-center text-fg-3">
            <ListTodo className="mb-3 size-8" />
            <div className="text-[13px]">Pick a Loft issue to see its details and thread.</div>
          </div>
        )}
      </section>
      {creating && <NewTaskDialog agents={agents} projects={projects} onClose={() => setCreating(false)} />}
    </div>
  );
}

function TaskRow({ task: t, agent, active, onOpen, progress }: { task: Task; agent?: Agent; active: boolean; onOpen: () => void; progress: { done: number; total: number } | null }) {
  return (
    <button onClick={onOpen} className={cx("flex w-full items-start gap-3 rounded-xl px-3 py-2.5 text-left transition-colors", active ? "bg-white/[0.06]" : "hover:bg-white/[0.03]")}>
      <StateIcon task={t} className="mt-0.5 size-3.5 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <Origin github={!!t.github} />
          <span className={cx("truncate text-[13.5px]", t.status === "cancelled" && "text-fg-3 line-through")}>{t.title}</span>
          {/* Two labels fit next to a title; the rest are counted and show on the task's page. */}
          {t.labels.slice(0, 2).map((l) => <span key={l} className="shrink-0"><LabelChip name={l} /></span>)}
          {t.labels.length > 2 && <span className="shrink-0 font-mono text-[10.5px] text-fg-3" title={t.labels.slice(2).join(", ")}>+{t.labels.length - 2}</span>}
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 truncate font-mono text-[10.5px] text-fg-3">
          <span className={STATUS_TONE[t.status]}>{statusLabel(t.status)}</span>
          {t.github && <span className="text-fg-2">· #{t.github.number}</span>}
          {progress && <span className="text-fg-2" title="Tickets done of this spec's tickets">· {progress.done}/{progress.total} done</span>}
          {agent && <>· {agent.name}</>}
          {t.project && <>· {t.project}</>}
          {(t.priority === "high" || t.priority === "urgent") && <span className="text-warn">· {t.priority}</span>}
          <span>· {timeAgo(t.updated_at)}</span>
        </div>
        {t.note && t.status === "in_progress" && <div className="mt-1 truncate text-[12px] text-fg-2">{t.note}</div>}
      </div>
      {agent && <Sticker avatar={agent.avatar} lead={agent.lead} size={22} />}
    </button>
  );
}

function TaskDetail({ task: t, tasks, agents, agentById, repos }: { task: Task; tasks: Task[]; agents: Agent[]; agentById: Map<string, Agent>; repos: Record<string, string> }) {
  const [thread, setThread] = useState<TaskComment[] | null>(null);
  useEffect(() => {
    setThread(null);
    api.taskComments(t.id).then(setThread).catch(() => setThread([]));
  }, [t.id]);
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
  const creator = t.created_by === "user" ? "you" : t.created_by === "github" ? "GitHub" : agentById.get(t.created_by)?.name ?? "an agent";
  const parent = t.parent_id ? tasks.find((x) => x.id === t.parent_id) : undefined;
  const subtasks = tasks.filter((x) => x.parent_id === t.id);
  const deps = t.depends_on.map((id) => tasks.find((x) => x.id === id)).filter((x): x is Task => !!x);
  const blocking = tasks.filter((x) => x.depends_on.includes(t.id));

  return (
    <div className="mx-auto max-w-[820px] space-y-6 px-6 py-6">
      <Link to="/tasks" className="inline-flex items-center gap-1 font-mono text-[11px] text-fg-3 hover:text-fg lg:hidden">
        <ArrowLeft className="size-3" /> All issues
      </Link>
      <header className="space-y-2">
        {parent && (
          <Link to={`/tasks/${parent.id}`} className="inline-flex items-center gap-1 font-mono text-[11px] text-fg-3 hover:text-fg">
            Part of {parent.title} <ChevronRight className="size-3" />
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
        {(t.labels.length > 0 || t.owner) && (
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
                <UserRound className="size-3.5 text-fg-3" /> <span className="font-mono">{t.owner}</span>
              </div>
            )}
          </div>
        )}
        <div className="pt-2">
          <TaskActions task={t} agents={agents} repo={repos[t.project]} />
        </div>
      </header>

      {agent && (
        <Link to={`/agents/${agent.id}`} className="panel flex items-center gap-3 p-3 transition-colors hover:bg-white/[0.03]">
          <Sticker avatar={agent.avatar} lead={agent.lead} size={36} />
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px]">{agent.name}</div>
            <div className="truncate font-mono text-[11px] text-fg-3">{agent.running ? agent.status_text || "Working…" : "Open conversation"}</div>
          </div>
          <MessageSquare className="size-4 text-fg-3" />
        </Link>
      )}

      {t.error && t.status !== "done" && <Block title="Problem" tone="text-bad" text={t.error} />}
      {t.summary && <Block title="Summary" text={t.summary} />}
      {t.note && t.status !== "done" && <Block title="Latest progress" text={t.note} />}
      {t.description && <Block title="Description" text={t.description} />}

      {(subtasks.length > 0 || deps.length > 0 || blocking.length > 0) && (
        <div className="grid gap-4 sm:grid-cols-2">
          {subtasks.length > 0 && <TaskLinks title="Subtasks" tasks={subtasks} />}
          {deps.length > 0 && <TaskLinks title="Blocked by" tasks={deps} />}
          {blocking.length > 0 && <TaskLinks title="Blocking" tasks={blocking} />}
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
          <CommentBox task={t} agent={agent} />
        </div>
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

function TaskLinks({ title, tasks }: { title: string; tasks: Task[] }) {
  return (
    <section>
      <div className="eyebrow mb-2">{title}</div>
      <div className="space-y-1">
        {tasks.map((x) => (
          <Link key={x.id} to={`/tasks/${x.id}`} className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-[13px] hover:bg-white/[0.03]">
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
        <Sticker avatar={agent.avatar} lead={agent.lead} size={26} />
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
