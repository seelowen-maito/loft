import { useEffect, useMemo, useRef, useState } from "react";
import { CircleDot, Search } from "lucide-react";
import type { Agent } from "../../lib/api";
import { cx, timeAgo } from "../../lib/format";
import { STATUS_TONE, useTasks } from "../../components/TaskChip";

/** Open tasks first, the ones waiting on you on top; then the most recently updated. */
const ORDER = ["waiting_approval", "blocked", "failed", "in_progress", "ready", "inbox", "done", "cancelled"];

/** "Attach a task" above the composer: search, pick one with the mouse or ↑↓ + Enter, Esc closes. */
export function TaskPicker({ agents, onPick, onClose }: { agents: Agent[]; onPick: (id: string) => void; onClose: () => void }) {
  const all = useTasks();
  const [q, setQ] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const list = useMemo(() => {
    const term = q.trim().toLowerCase();
    return (all ?? [])
      .filter((t) => !term || t.title.toLowerCase().includes(term) || t.project.toLowerCase().includes(term))
      .sort((a, b) => ORDER.indexOf(a.status) - ORDER.indexOf(b.status) || b.updated_at.localeCompare(a.updated_at))
      .slice(0, 8);
  }, [all, q]);
  useEffect(() => setIndex(0), [q]);
  const name = (id: string | null) => agents.find((a) => a.id === id)?.name;

  return (
    <div className="mb-2 overflow-hidden rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="dialog" aria-label="Attach a task">
      <label className="flex items-center gap-2 border-b border-line px-3 py-2">
        <Search className="size-3.5 text-fg-3" />
        <input
          ref={input}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") onClose();
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              setIndex((i) => (list.length ? (i + (e.key === "ArrowDown" ? 1 : list.length - 1)) % list.length : 0));
            }
            if (e.key === "Enter" && list[index]) {
              e.preventDefault();
              onPick(list[index].id);
            }
          }}
          placeholder="Attach a task: search by title or project"
          className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3"
        />
      </label>
      <div role="listbox">
        {all === null && <div className="px-3 py-2 font-mono text-[11.5px] text-fg-3">Loading…</div>}
        {all !== null && list.length === 0 && <div className="px-3 py-2 text-[12.5px] text-fg-3">No task matches.</div>}
        {list.map((t, i) => (
          <button
            key={t.id}
            role="option"
            aria-selected={i === index}
            onMouseEnter={() => setIndex(i)}
            onMouseDown={(e) => {
              e.preventDefault();
              onPick(t.id);
            }}
            className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", i === index && "bg-white/[0.06]")}
          >
            <CircleDot className={cx("size-3.5 shrink-0", STATUS_TONE[t.status])} />
            <span className="min-w-0 flex-1 truncate text-[13px]">{t.title}</span>
            <span className="shrink-0 truncate font-mono text-[10.5px] text-fg-3">
              {[t.status.replace("_", " "), name(t.agent_id), t.project, timeAgo(t.updated_at)].filter(Boolean).join(" · ")}
            </span>
          </button>
        ))}
      </div>
      <div className="border-t border-line px-3 py-1.5 font-mono text-[10.5px] text-fg-3">The agent gets the task's title, status and latest note with your message.</div>
    </div>
  );
}
