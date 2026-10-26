import { useCallback, useEffect, useSyncExternalStore } from "react";
import { Link } from "react-router";
import { CircleDot, X } from "lucide-react";
import { api, type Task } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";

/** Task ids in links: `…/tasks/<id>` (the web app) or `loft://task/<id>` (the iOS app). */
export const TASK_LINK = /(?:\/tasks\/|loft:\/\/task\/)([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i;

// One shared, live copy of the task list for chips and pickers across the chat.
let tasks: Task[] | null = null;
let loading = false;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

function load() {
  if (tasks || loading) return;
  loading = true;
  api.tasks().then((t) => {
    tasks = t;
    notify();
  }).finally(() => (loading = false));
}

/** All tasks, kept current by task events. Null until loaded. */
export function useTasks(): Task[] | null {
  useEffect(load, []);
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (!tasks) return;
      if (e.type === "task") {
        const i = tasks.findIndex((t) => t.id === e.task.id);
        tasks = i < 0 ? [e.task, ...tasks] : tasks.map((t) => (t.id === e.task.id ? e.task : t));
        notify();
      }
      if (e.type === "task_deleted") {
        tasks = tasks.filter((t) => t.id !== e.task_id);
        notify();
      }
    }, []),
  );
  return useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => tasks,
  );
}

export const STATUS_TONE: Record<string, string> = {
  waiting_approval: "text-warn",
  blocked: "text-warn",
  failed: "text-bad",
  in_progress: "text-accent",
  ready: "text-info",
  inbox: "text-fg-3",
  done: "text-good",
  cancelled: "text-fg-3",
};

/** A task as a small chip: status, title, link to its page. `onRemove` shows an × (the composer). */
export function TaskChip({ id, onRemove, className }: { id: string; onRemove?: () => void; className?: string }) {
  const all = useTasks();
  const t = all?.find((x) => x.id === id.toLowerCase());
  const label = t ? t.title : all ? "Deleted task" : "Task";
  return (
    <span className={cx("inline-flex max-w-full items-center gap-1.5 rounded-full border border-line-strong bg-ink-850 py-0.5 pl-2 text-[12px] align-middle", onRemove ? "pr-1" : "pr-2.5", className)}>
      <CircleDot className={cx("size-3 shrink-0", t ? STATUS_TONE[t.status] : "text-fg-3")} />
      <Link to={`/tasks/${id}`} className="truncate text-fg hover:underline" title={t ? `${t.title} · ${t.status.replace("_", " ")}` : undefined}>
        {label}
      </Link>
      {t && <span className="shrink-0 font-mono text-[10px] text-fg-3">{t.status === "waiting_approval" ? "waiting" : t.status.replace("_", " ")}</span>}
      {onRemove && (
        <button type="button" className="rounded-full p-0.5 text-fg-3 hover:bg-white/[0.08] hover:text-fg" onClick={onRemove} aria-label="Remove the attached task">
          <X className="size-3" />
        </button>
      )}
    </span>
  );
}
