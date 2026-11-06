import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { Search } from "lucide-react";
import { api, type Agent } from "../lib/api";
import { cx } from "../lib/format";
import { teamOf } from "../lib/team";
import { loadCombo, loadRecent, matchesCombo, recording, switcherAgents, SWITCHER_KEY_CHANGED } from "../lib/quickSwitcher";
import { statusOf } from "../pages/agents/Status";
import { Sticker } from "./Sticker";

/** ⌘K (or the combination set in Settings) from any page, even while typing: a search box that opens an agent's chat. */
export function QuickSwitcher() {
  const [open, setOpen] = useState(false);
  const combo = useRef(loadCombo());

  useEffect(() => {
    const onChanged = () => (combo.current = loadCombo());
    // Capture, so a focused input can't swallow it first.
    const onKey = (e: KeyboardEvent) => {
      if (recording.on || !matchesCombo(e, combo.current)) return;
      e.preventDefault();
      setOpen((o) => !o);
    };
    window.addEventListener(SWITCHER_KEY_CHANGED, onChanged);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener(SWITCHER_KEY_CHANGED, onChanged);
      window.removeEventListener("keydown", onKey, true);
    };
  }, []);

  return open ? <Switcher onClose={() => setOpen(false)} /> : null;
}

function Switcher({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const [agents, setAgents] = useState<Agent[] | null>(null);
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const recent = useMemo(() => loadRecent(), []);

  useEffect(() => {
    api.agents().then(setAgents).catch(() => setAgents([]));
  }, []);

  const shown = useMemo(() => switcherAgents(agents ?? [], query, recent), [agents, query, recent]);
  useEffect(() => setSel(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-i="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  }, [sel]);

  const go = (a: Agent | undefined) => {
    if (!a) return;
    onClose();
    navigate(`/agents/${a.id}`);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center px-4 pt-[12vh]">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div role="dialog" aria-label="Open an agent" className="relative flex max-h-[70vh] w-full max-w-[520px] animate-rise flex-col overflow-clip rounded-2xl border border-line-strong bg-ink-900 shadow-2xl">
        <div className="flex items-center gap-2.5 border-b border-line px-4">
          <Search className="size-4 shrink-0 text-fg-3" />
          <input
            autoFocus
            className="min-w-0 flex-1 bg-transparent py-3.5 text-[14px] outline-none placeholder:text-fg-3"
            placeholder="Open an agent…"
            aria-label="Search agents"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                const n = shown.length;
                if (n) setSel((s) => (s + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
              } else if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault();
                go(shown[sel]);
              } else if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
          />
        </div>
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-1.5" role="listbox" aria-label="Agents">
          {agents && shown.length === 0 && <p className="px-3 py-4 text-[12.5px] text-fg-3">No agent matches.</p>}
          {shown.map((a, i) => {
            const s = statusOf(a);
            return (
              <button
                key={a.id}
                data-i={i}
                role="option"
                aria-selected={i === sel}
                onMouseMove={() => setSel(i)}
                onClick={() => go(a)}
                className={cx("flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left", i === sel && "bg-white/[0.06]")}
              >
                <div className="relative shrink-0">
                  <Sticker avatar={a.avatar} lead={a.role === "lead"} size={32} />
                  <span className={cx("absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full border-2 border-ink-900", s.dot)} />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13.5px]">{a.name}</div>
                  <div className="truncate font-mono text-[10.5px] text-fg-3">{[teamOf(a), a.title].filter(Boolean).join(" · ")}</div>
                </div>
                <span className={cx("shrink-0 font-mono text-[10.5px] uppercase tracking-wider", s.text)}>{s.label}</span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
