import { type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { Search, X } from "lucide-react";
import { api, type Agent } from "../lib/api";
import { cx } from "../lib/format";
import { teamOf } from "../lib/team";
import {
  FIXED, formatCombo, isTyping, loadCombos, loadRecent, nextNeedsYou, REBINDABLE, recording, shortcutAction,
  SHORTCUTS_CHANGED, switcherAgents, type Combos,
} from "../lib/shortcuts";
import { statusOf } from "../pages/agents/Status";
import { Sticker } from "./Sticker";

const G_WINDOW_MS = 1500;

/** The web app's keyboard shortcuts, from any page: ⌘K switcher, ⌘J next agent that needs you, `g` + letter, `/` and `?`. */
export function Shortcuts() {
  const navigate = useNavigate();
  const [open, setOpen] = useState<"switcher" | "sheet" | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const combos = useRef<Combos>(loadCombos());
  const gAt = useRef(0);
  const returnTo = useRef<HTMLElement | null>(null);
  // Focus goes back to where it was, e.g. the chat box.
  const close = useCallback(() => {
    setOpen(null);
    returnTo.current?.focus();
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2500);
    return () => clearTimeout(t);
  }, [toast]);

  useEffect(() => {
    const onChanged = () => (combos.current = loadCombos());
    // Capture, so a focused input can't swallow ⌘K first.
    const onKey = (e: KeyboardEvent) => {
      if (recording.on || e.isComposing) return;
      const afterG = Date.now() - gAt.current < G_WINDOW_MS;
      gAt.current = 0;
      const a = shortcutAction(e, { combos: combos.current, typing: isTyping(e.target), afterG });
      if (!a) return;
      e.preventDefault();
      if (a.type === "switcher" || a.type === "sheet") {
        if (!document.querySelector('[role="dialog"][data-shortcuts]')) returnTo.current = document.activeElement as HTMLElement | null;
        setOpen((o) => (a.type === "switcher" && o === "switcher" ? null : a.type));
      }
      else if (a.type === "g") gAt.current = Date.now();
      else if (a.type === "go") navigate(a.path);
      else if (a.type === "focus-chat") document.querySelector<HTMLTextAreaElement>("[data-chat-input]")?.focus();
      else if (a.type === "next") {
        const current = window.location.pathname.match(/^\/agents\/([^/]+)/)?.[1] ?? null;
        api
          .needsYou()
          .then((r) => {
            const id = nextNeedsYou(r.items, current);
            if (id) {
              setOpen(null);
              navigate(`/agents/${id}`);
            } else setToast("Nothing needs you");
          })
          .catch(() => setToast("Couldn't load Needs you"));
      }
    };
    window.addEventListener(SHORTCUTS_CHANGED, onChanged);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener(SHORTCUTS_CHANGED, onChanged);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [navigate]);

  return (
    <>
      {open === "switcher" && <Switcher onClose={close} />}
      {open === "sheet" && <Sheet combos={combos.current} onClose={close} />}
      {toast && (
        <div role="status" className="fixed inset-x-0 bottom-6 z-50 flex justify-center px-4">
          <div className="rounded-full border border-line-strong bg-ink-850/95 px-4 py-2 text-[13px] text-fg-2 shadow-2xl backdrop-blur">{toast}</div>
        </div>
      )}
    </>
  );
}

/** Every shortcut with its current keys, for the sheet and Settings. */
export function shortcutRows(combos: Combos): { keys: string; label: string }[] {
  return [...REBINDABLE.map((r) => ({ keys: formatCombo(combos[r.id]), label: r.label })), ...FIXED];
}

export function Keys({ keys }: { keys: string }) {
  return (
    <span className="flex shrink-0 gap-1">
      {keys.split(" ").map((k, i) => (
        <kbd key={i} className="chip min-w-7 justify-center font-mono text-[12px]">{k}</kbd>
      ))}
    </span>
  );
}

/** A dialog's keyboard: Escape closes it from anywhere in it and Tab stays on `stay`. */
function useDialogKeys(onClose: () => void, stay: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "Tab") {
        e.preventDefault();
        stay.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, stay]);
}

function Sheet({ combos, onClose }: { combos: Combos; onClose: () => void }) {
  const closeRef = useRef<HTMLButtonElement>(null);
  useDialogKeys(onClose, closeRef);
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center px-4 pt-[12vh]">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div role="dialog" data-shortcuts aria-label="Keyboard shortcuts" className="relative max-h-[76vh] w-full max-w-[420px] animate-rise overflow-y-auto rounded-2xl border border-line-strong bg-ink-900 p-5 shadow-2xl">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-[15px] font-medium tracking-tight">Keyboard shortcuts</h2>
          <button ref={closeRef} autoFocus className="btn-ghost" onClick={onClose} aria-label="Close"><X className="size-4" /></button>
        </div>
        <ul className="space-y-2">
          {shortcutRows(combos).map((r) => (
            <li key={r.label} className="flex items-center justify-between gap-3 text-[13px]">
              <span className="min-w-0 text-fg-2">{r.label}</span>
              <Keys keys={r.keys} />
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

function Switcher({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const [agents, setAgents] = useState<Agent[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const recent = useMemo(() => loadRecent(), []);
  useDialogKeys(onClose, inputRef);

  useEffect(() => {
    api.agents().then(setAgents).catch(() => setFailed(true));
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
      <div role="dialog" data-shortcuts aria-label="Open an agent" className="relative flex max-h-[70vh] w-full max-w-[520px] animate-rise flex-col overflow-clip rounded-2xl border border-line-strong bg-ink-900 shadow-2xl">
        <div className="flex items-center gap-2.5 border-b border-line px-4">
          <Search className="size-4 shrink-0 text-fg-3" />
          <input
            ref={inputRef}
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
              }
            }}
          />
        </div>
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-1.5" role="listbox" aria-label="Agents">
          {failed && <p className="px-3 py-4 text-[12.5px] text-fg-3">Couldn't load agents.</p>}
          {agents && shown.length === 0 && <p className="px-3 py-4 text-[12.5px] text-fg-3">No agent matches.</p>}
          {shown.map((a, i) => {
            const s = statusOf(a);
            return (
              <button
                key={a.id}
                data-i={i}
                tabIndex={-1}
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
