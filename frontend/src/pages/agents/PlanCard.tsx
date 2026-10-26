import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router";
import { CircleDot, GitPullRequest, ListChecks } from "lucide-react";
import { api, type Agent, type TaskPlan } from "../../lib/api";
import { useServerEvents, type ServerEvent } from "../../lib/events";
import { cx } from "../../lib/format";
import { Sticker } from "../../components/Sticker";

const TONE: Record<string, string> = {
  done: "text-good",
  in_progress: "text-accent",
  waiting_approval: "text-warn",
  blocked: "text-warn",
  failed: "text-bad",
  cancelled: "text-fg-3",
  ready: "text-info",
  inbox: "text-fg-3",
};

/** A delegated plan in the delegating agent's chat: each subtask, who's on it, its status and PRs, kept live. */
export function PlanCard({ taskId, agents }: { taskId: string; agents: Agent[] }) {
  const [plan, setPlan] = useState<TaskPlan | null>(null);
  const [missing, setMissing] = useState(false);
  const load = useCallback(() => {
    api.taskPlan(taskId).then(setPlan).catch(() => setMissing(true));
  }, [taskId]);
  useEffect(load, [load]);
  useServerEvents(
    useCallback(
      (e: ServerEvent) => {
        if (e.type === "task" && (e.task.id === taskId || e.task.parent_id === taskId)) load();
        if (e.type === "pull_request") load();
      },
      [taskId, load],
    ),
  );

  if (missing) return null;
  if (!plan) return <div className="h-24 animate-pulse rounded-2xl border border-line bg-ink-850" />;
  const p = plan.progress;
  const sticker = (id?: string) => agents.find((a) => a.id === id);
  return (
    <div className="rounded-2xl border border-line-strong bg-ink-850 p-4">
      <div className="flex items-center gap-2">
        <ListChecks className="size-4 text-accent" />
        <Link to={`/tasks/${plan.task.id}`} className="min-w-0 flex-1 truncate text-[14px] font-medium hover:underline">
          {plan.task.title}
        </Link>
        <span className="shrink-0 font-mono text-[11px] text-fg-3">
          {p.done}/{p.total} done
        </span>
      </div>
      <div className="mt-2 flex h-1.5 overflow-hidden rounded-full bg-white/[0.06]" role="progressbar" aria-valuenow={p.percent} aria-valuemin={0} aria-valuemax={100}>
        <div className="bg-good" style={{ width: `${(p.done / Math.max(1, p.total)) * 100}%` }} />
        <div className="bg-accent/70" style={{ width: `${(p.in_progress / Math.max(1, p.total)) * 100}%` }} />
        <div className="bg-warn/70" style={{ width: `${(p.waiting / Math.max(1, p.total)) * 100}%` }} />
        <div className="bg-bad/70" style={{ width: `${(p.failed / Math.max(1, p.total)) * 100}%` }} />
      </div>
      <div className="mt-1.5 flex flex-wrap gap-x-3 font-mono text-[10.5px] text-fg-3">
        {p.in_progress > 0 && <span className="text-accent">{p.in_progress} working</span>}
        {p.waiting > 0 && <span className="text-warn">{p.waiting} waiting or blocked</span>}
        {p.failed > 0 && <span className="text-bad">{p.failed} failed</span>}
        {p.todo > 0 && <span>{p.todo} not started</span>}
      </div>
      <ol className="mt-3 space-y-1">
        {plan.subtasks.map((s) => {
          const a = sticker(s.agent?.id);
          const line = s.status === "done" ? s.summary : ["failed", "blocked"].includes(s.status) ? s.error : s.note;
          return (
            <li key={s.id}>
              <Link to={`/tasks/${s.id}`} className="flex items-start gap-2.5 rounded-lg px-2 py-1.5 hover:bg-white/[0.04]">
                <CircleDot className={cx("mt-0.5 size-3.5 shrink-0", TONE[s.status])} />
                <div className="min-w-0 flex-1">
                  <div className={cx("truncate text-[13px]", s.status === "cancelled" && "text-fg-3 line-through")}>{s.title}</div>
                  <div className="truncate font-mono text-[10.5px] text-fg-3">
                    <span className={TONE[s.status]}>{s.status.replace("_", " ")}</span>
                    {s.agent && <> · {s.agent.name}</>}
                    {line && <> · {line.split("\n")[0]}</>}
                  </div>
                </div>
                {s.prs.map((pr) => (
                  <a key={pr.id} href={pr.url} target="_blank" rel="noopener noreferrer" onClick={(e) => e.stopPropagation()} className="chip shrink-0 gap-1 text-[10.5px]" title={pr.title}>
                    <GitPullRequest className="size-3" />#{pr.number}
                  </a>
                ))}
                {a && <Sticker avatar={a.avatar} lead={a.lead} size={20} />}
              </Link>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
