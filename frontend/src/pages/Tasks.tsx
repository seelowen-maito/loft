import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft, ChevronRight, CircleCheck, CircleDot, ListTodo, MessageSquare, NotebookPen, Plus, Search, UserRound } from "lucide-react";
import { api, type Agent, type Task, type TaskComment, type TaskStatus } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx, timeAgo } from "../lib/format";
import { Sticker } from "../components/Sticker";
import { CommentBox, NewTaskDialog, TaskActions } from "./TaskActions";

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

const PRIORITY_RANK = { urgent: 0, high: 1, normal: 2, low: 3 } as const;

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
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const [agentId, setAgentId] = useState("");
  const [mine, setMine] = useState(false);
  const [showClosed, setShowClosed] = useState(false);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    api.tasks().then(setTasks);
    api.agents().then(setAgents);
  }, []);

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
    return (tasks ?? []).filter(
      (t) =>
        (!q || t.title.toLowerCase().includes(q) || t.description.toLowerCase().includes(q) || t.labels.some((l) => l.toLowerCase().includes(q)) || t.owner.toLowerCase() === q.replace(/^@/, "")) &&
        (!project || t.project === project) &&
        (!agentId || t.agent_id === agentId) &&
        (!mine || t.created_by === "user"),
    );
  }, [tasks, query, project, agentId, mine]);

  const groups = useMemo(
    () =>
      GROUPS.map((g) => ({
        ...g,
        tasks: filtered
          .filter((t) => g.statuses.includes(t.status))
          .sort((a, b) =>
            g.key === "closed"
              ? (b.completed_at ?? b.updated_at).localeCompare(a.completed_at ?? a.updated_at)
              : PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] || b.updated_at.localeCompare(a.updated_at),
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
              <div className="eyebrow">Tasks</div>
              <div className="mt-1 font-mono text-[11px] text-fg-3">
                {groups[0].tasks.length > 0 && <span className="text-warn">{groups[0].tasks.length} need you · </span>}
                {(tasks ?? []).filter((t) => !["done", "cancelled"].includes(t.status)).length} open
              </div>
            </div>
            <button className="btn-primary size-8 justify-center p-0" onClick={() => setCreating(true)} aria-label="New task" title="New task">
              <Plus className="size-4" />
            </button>
          </div>
          <label className="flex items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5">
            <Search className="size-3.5 text-fg-3" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search tasks" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
          </label>
          <div className="flex flex-wrap gap-2">
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
              <div className="text-[13px]">No tasks yet. Agents add them as they work, or add one with +.</div>
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
                    <TaskRow key={t.id} task={t} agent={t.agent_id ? agentById.get(t.agent_id) : undefined} active={t.id === routeId} onOpen={() => navigate(`/tasks/${t.id}`)} />
                  ))}
              </section>
            );
          })}
        </div>
      </aside>

      {/* detail */}
      <section className={cx("min-h-0 overflow-y-auto", !routeId && "max-lg:hidden")}>
        {selected ? (
          <TaskDetail task={selected} tasks={tasks ?? []} agents={agents} agentById={agentById} />
        ) : routeId && tasks ? (
          <div className="p-10 text-center text-[13px] text-fg-3">This task no longer exists.</div>
        ) : (
          <div className="flex h-full flex-col items-center justify-center p-10 text-center text-fg-3">
            <ListTodo className="mb-3 size-8" />
            <div className="text-[13px]">Pick a task to see its details and thread.</div>
          </div>
        )}
      </section>
      {creating && <NewTaskDialog agents={agents} projects={projects} onClose={() => setCreating(false)} />}
    </div>
  );
}

function TaskRow({ task: t, agent, active, onOpen }: { task: Task; agent?: Agent; active: boolean; onOpen: () => void }) {
  return (
    <button onClick={onOpen} className={cx("flex w-full items-start gap-3 rounded-xl px-3 py-2.5 text-left transition-colors", active ? "bg-white/[0.06]" : "hover:bg-white/[0.03]")}>
      <StateIcon task={t} className="mt-0.5 size-3.5 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className={cx("truncate text-[13.5px]", t.status === "cancelled" && "text-fg-3 line-through")}>{t.title}</span>
          {/* Two labels fit next to a title; the rest are counted and show on the task's page. */}
          {t.labels.slice(0, 2).map((l) => <span key={l} className="shrink-0"><LabelChip name={l} /></span>)}
          {t.labels.length > 2 && <span className="shrink-0 font-mono text-[10.5px] text-fg-3" title={t.labels.slice(2).join(", ")}>+{t.labels.length - 2}</span>}
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 truncate font-mono text-[10.5px] text-fg-3">
          <span className={STATUS_TONE[t.status]}>{statusLabel(t.status)}</span>
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

function TaskDetail({ task: t, tasks, agents, agentById }: { task: Task; tasks: Task[]; agents: Agent[]; agentById: Map<string, Agent> }) {
  const [thread, setThread] = useState<TaskComment[] | null>(null);
  useEffect(() => {
    setThread(null);
    api.taskComments(t.id).then(setThread).catch(() => setThread([]));
  }, [t.id]);
  useServerEvents(
    useCallback(
      (e: ServerEvent) => {
        if (e.type === "task_comment" && e.comment.task_id === t.id)
          setThread((th) => (th && !th.some((c) => c.id === e.comment.id) ? [...th, e.comment] : th));
      },
      [t.id],
    ),
  );

  const agent = t.agent_id ? agentById.get(t.agent_id) : undefined;
  const creator = t.created_by === "user" ? "you" : agentById.get(t.created_by)?.name ?? "an agent";
  const parent = t.parent_id ? tasks.find((x) => x.id === t.parent_id) : undefined;
  const subtasks = tasks.filter((x) => x.parent_id === t.id);
  const deps = t.depends_on.map((id) => tasks.find((x) => x.id === id)).filter((x): x is Task => !!x);

  return (
    <div className="mx-auto max-w-[820px] space-y-6 px-6 py-6">
      <Link to="/tasks" className="inline-flex items-center gap-1 font-mono text-[11px] text-fg-3 hover:text-fg lg:hidden">
        <ArrowLeft className="size-3" /> All tasks
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
          <TaskActions task={t} agents={agents} />
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

      {(subtasks.length > 0 || deps.length > 0) && (
        <div className="grid gap-4 sm:grid-cols-2">
          {subtasks.length > 0 && <TaskLinks title="Subtasks" tasks={subtasks} />}
          {deps.length > 0 && <TaskLinks title="Waits for" tasks={deps} />}
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
      {agent ? <Sticker avatar={agent.avatar} lead={agent.lead} size={26} /> : <span className="flex size-[26px] shrink-0 items-center justify-center rounded-full bg-accent/15 font-mono text-[11px] text-accent">you</span>}
      <div className={cx("min-w-0 flex-1 rounded-xl border px-3 py-2", c.kind === "note" ? "border-line bg-transparent" : "border-line-strong bg-ink-850")}>
        <div className="mb-1 flex items-center gap-1.5 font-mono text-[10.5px] text-fg-3">
          {c.kind === "note" && <NotebookPen className="size-3" />}
          <span className="text-fg-2">{name}</span>
          <span>· {c.kind === "note" ? "progress note" : "comment"}</span>
          {c.meta.private && <span className="text-warn">· private</span>}
          <span>· {when}</span>
        </div>
        <div className="prose-chat text-[13.5px]">
          <Markdown remarkPlugins={[remarkGfm]}>{c.text}</Markdown>
        </div>
      </div>
    </li>
  );
}
