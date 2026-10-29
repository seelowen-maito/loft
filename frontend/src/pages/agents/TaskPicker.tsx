import { useEffect, useMemo, useRef, useState } from "react";
import { CircleDot, Search } from "lucide-react";
import type { Agent } from "../../lib/api";
import { filterIssues, SOURCES, type Source } from "../../lib/issueFilter";
import { ATTACH } from "../../lib/attach";
import { Origin } from "../../components/Origin";
import { cx, timeAgo } from "../../lib/format";
import { STATUS_TONE, useTasks } from "../../components/TaskChip";

/** Open tasks first, the ones waiting on you on top; then the most recently updated. */
const ORDER = ["waiting_approval", "blocked", "failed", "in_progress", "ready", "inbox", "done", "cancelled"];

/** "Attach an issue" above the composer: Loft and GitHub issues, search (or #42), pick one with the mouse or ↑↓ + Enter, Esc closes. */
export function TaskPicker({ agents, onPick, onClose }: { agents: Agent[]; onPick: (id: string) => void; onClose: () => void }) {
  const all = useTasks();
  const [q, setQ] = useState("");
  const [source, setSource] = useState<Source>("all");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const list = useMemo(() => {
    return filterIssues(all ?? [], q, source)
      .sort((a, b) => ORDER.indexOf(a.status) - ORDER.indexOf(b.status) || b.updated_at.localeCompare(a.updated_at))
      .slice(0, 8);
  }, [all, q, source]);
  useEffect(() => setIndex(0), [q, source]);
  const name = (id: string | null) => agents.find((a) => a.id === id)?.name;

  return (
    <div className="mb-2 overflow-hidden rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="dialog" aria-label={ATTACH}>
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
          placeholder={`${ATTACH}: search by title, or #42`}
          className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3"
        />
      </label>
      <div className="flex gap-0.5 border-b border-line px-2 py-1.5" role="group" aria-label="Source">
        {SOURCES.map((o) => (
          <button key={o.key} onClick={() => setSource(o.key)} aria-pressed={source === o.key} className={cx("flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[12px]", source === o.key ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
            {o.key !== "all" && <Origin github={o.key === "github"} />} {o.label}
          </button>
        ))}
      </div>
      <div role="listbox">
        {all === null && <div className="px-3 py-2 font-mono text-[11.5px] text-fg-3">Loading…</div>}
        {all !== null && list.length === 0 && <div className="px-3 py-2 text-[12.5px] text-fg-3">No issue matches.</div>}
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
            <Origin github={!!t.github} />
            <span className="min-w-0 flex-1 truncate text-[13px]">{t.title}</span>
            <span className="shrink-0 truncate font-mono text-[10.5px] text-fg-3">
              {[t.github && `#${t.github.number}`, t.status.replace("_", " "), name(t.agent_id), t.project, timeAgo(t.updated_at)].filter(Boolean).join(" · ")}
            </span>
          </button>
        ))}
      </div>
      <div className="border-t border-line px-3 py-1.5 font-mono text-[10.5px] text-fg-3">The agent gets the issue's title, status and latest note with your message.</div>
    </div>
  );
}
