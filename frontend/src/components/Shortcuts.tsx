import { type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { CircleDot, ExternalLink, FileText, GitPullRequest, Search, X } from "lucide-react";
import { api, roleLabel, type Agent, type PullRequest, type Task } from "../lib/api";
import { isSpec, specProgress } from "../lib/issueOrder";
import { cx } from "../lib/format";
import { teamOf } from "../lib/team";
import {
  FIXED, formatCombo, isTyping, loadCombos, loadRecent, nextNeedsYou, OPEN_SHORTCUT, REBINDABLE, recording, shortcutAction,
  searchRows, SHORTCUTS_CHANGED, switcherAgents, type Combos, type Openable,
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
    const onOpen = (e: Event) => {
      returnTo.current = document.activeElement as HTMLElement | null;
      setOpen((e as CustomEvent<Openable>).detail);
    };
    // Capture, so a focused input can't swallow ⌘K first.
    const onKey = (e: KeyboardEvent) => {
      if (recording.on || e.isComposing) return;
      const afterG = Date.now() - gAt.current < G_WINDOW_MS;
      gAt.current = 0;
      const a = shortcutAction(e, { combos: combos.current, typing: isTyping(e.target), afterG });
      if (!a) return;
      const shown = document.querySelector<HTMLElement>('[role="dialog"][data-shortcuts]');
      // Plain keys don't act on the page behind an open dialog.
      if (shown && (a.type === "g" || a.type === "go" || a.type === "focus-chat" || a.type === "sheet")) return;
      e.preventDefault();
      if (a.type === "switcher" || a.type === "sheet") {
        if (a.type === "switcher" && shown?.getAttribute("aria-label") === SWITCHER) close();
        else {
          if (!shown) returnTo.current = document.activeElement as HTMLElement | null;
          setOpen(a.type);
        }
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
    window.addEventListener(OPEN_SHORTCUT, onOpen);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener(SHORTCUTS_CHANGED, onChanged);
      window.removeEventListener(OPEN_SHORTCUT, onOpen);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [navigate, close]);

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
      if (e.key === "Escape" && !e.isComposing) {
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

type Hit =
  | { kind: "agent"; key: string; agent: Agent }
  | { kind: "spec" | "issue"; key: string; task: Task; progress: { done: number; total: number } | null }
  | { kind: "pr"; key: string; pr: PullRequest };

const GROUPS = [
  { kind: "agent", label: "Agents" },
  { kind: "spec", label: "Specs" },
  { kind: "issue", label: "Issues" },
  { kind: "pr", label: "Pull requests" },
] as const;

const SWITCHER = "Search";

function Switcher({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const [agents, setAgents] = useState<Agent[] | null>(null);
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [prs, setPrs] = useState<PullRequest[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [tasksFailed, setTasksFailed] = useState(false);
  const [prsFailed, setPrsFailed] = useState(false);
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const recent = useMemo(() => loadRecent(), []);
  useDialogKeys(onClose, inputRef);

  // Loaded once when the switcher opens; typing searches what's here.
  useEffect(() => {
    api.agents().then(setAgents).catch(() => setFailed(true));
    api.tasks().then(setTasks).catch(() => setTasksFailed(true));
    api.pullRequests().then(setPrs).catch(() => setPrsFailed(true));
  }, []);

  const shown = useMemo<Hit[]>(() => {
    const found = switcherAgents(agents ?? [], query, recent, (a) => roleLabel(a.role));
    const agentHits = (query.trim() ? found.slice(0, 5) : found).map((a): Hit => ({ kind: "agent", key: `a:${a.id}`, agent: a }));
    if (!query.trim()) return agentHits;
    const issues = tasks ?? [];
    const row = (t: Task) => ({ title: t.title, number: t.github?.number ?? null, open: t.state === "open", project: t.project, task: t });
    const taskHits = (kind: "spec" | "issue") =>
      searchRows(issues.filter((t) => isSpec(t) === (kind === "spec")).map(row), query).map((r): Hit => ({
        kind, key: `t:${r.task.id}`, task: r.task, progress: kind === "spec" ? specProgress(r.task, issues) : null,
      }));
    // The same PR can be registered by more than one agent.
    const uniquePrs = [...new Map((prs ?? []).map((p) => [p.url, p])).values()];
    const prHits = searchRows(uniquePrs.map((p) => ({ title: p.title, number: p.number, open: p.state === "open" || p.state === "", project: p.slug.split("/").pop() ?? p.slug, pr: p })), query)
      .map((r): Hit => ({ kind: "pr", key: `p:${r.pr.url}`, pr: r.pr }));
    return [...agentHits, ...taskHits("spec"), ...taskHits("issue"), ...prHits];
  }, [agents, tasks, prs, query, recent]);
  useEffect(() => setSel(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-i="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  }, [sel]);

  const go = (h: Hit | undefined) => {
    if (!h) return;
    onClose();
    if (h.kind === "agent") navigate(`/agents/${h.agent.id}`);
    else if (h.kind === "pr") window.open(h.pr.url, "_blank", "noopener");
    else navigate(`/issues/${h.task.id}`);
  };

  const typed = query.trim() !== "";
  const loading = (tasks === null && !tasksFailed) || (prs === null && !prsFailed);
  const failedNote = tasksFailed && prsFailed ? "issues or pull requests" : tasksFailed ? "issues" : prsFailed ? "pull requests" : null;
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center px-4 pt-[12vh]">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div role="dialog" data-shortcuts aria-label={SWITCHER} className="relative flex max-h-[70vh] w-full max-w-[560px] animate-rise flex-col overflow-clip rounded-2xl border border-line-strong bg-ink-900 shadow-2xl">
        <div className="flex items-center gap-2.5 border-b border-line px-4">
          <Search className="size-4 shrink-0 text-fg-3" />
          <input
            ref={inputRef}
            autoFocus
            className="min-w-0 flex-1 bg-transparent py-3.5 text-[14px] outline-none placeholder:text-fg-3"
            placeholder="Search agents, specs, issues and pull requests…"
            aria-label="Search agents, specs, issues and pull requests"
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
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-1.5" role="listbox" aria-label="Results">
          {failed && <p className="px-3 py-4 text-[12.5px] text-fg-3">Couldn't load agents.</p>}
          {typed && failedNote && <p className="px-3 py-2 text-[12px] text-fg-3">Couldn't load {failedNote}.</p>}
          {agents && shown.length === 0 && <p className="px-3 py-4 text-[12.5px] text-fg-3">{!typed ? "No agents yet." : loading ? "Searching…" : "Nothing matches."}</p>}
          {GROUPS.map((g) => {
            const hits = shown.flatMap((h, i) => (h.kind === g.kind ? [{ h, i }] : []));
            if (!hits.length) return null;
            return (
              <div key={g.kind} role="group" aria-label={g.label}>
                {typed && <div className="px-2.5 pb-1 pt-2.5 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">{g.label}</div>}
                {hits.map(({ h, i }) => (
                  <button
                    key={h.key}
                    data-i={i}
                    tabIndex={-1}
                    role="option"
                    aria-selected={i === sel}
                    onMouseMove={() => setSel(i)}
                    onClick={() => go(h)}
                    className={cx("flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left", i === sel && "bg-white/[0.06]")}
                  >
                    <HitRow h={h} />
                  </button>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function HitRow({ h }: { h: Hit }) {
  if (h.kind === "agent") {
    const a = h.agent;
    const s = statusOf(a);
    return (
      <>
        <div className="relative shrink-0">
          <Sticker avatar={a.avatar} lead={a.role === "lead"} size={32} />
          <span className={cx("absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full border-2 border-ink-900", s.dot)} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px]">{a.name}</div>
          <div className="truncate font-mono text-[10.5px] text-fg-3">{[teamOf(a), roleLabel(a.role)].filter(Boolean).join(" · ")}</div>
        </div>
        <span className={cx("shrink-0 font-mono text-[10.5px] uppercase tracking-wider", s.text)}>{s.label}</span>
      </>
    );
  }
  const [Icon, title, number, state, project, url] =
    h.kind === "pr"
      ? [GitPullRequest, h.pr.title, h.pr.number, h.pr.draft && h.pr.state === "open" ? "draft" : h.pr.state || "open", h.pr.slug.split("/").pop(), true]
      : [h.kind === "spec" ? FileText : CircleDot, h.task.title, h.task.github?.number, h.task.state, h.task.project, false];
  const open = state === "open" || state === "draft";
  return (
    <>
      <Icon className={cx("size-4 shrink-0", open ? "text-fg-2" : "text-fg-3")} />
      <div className="min-w-0 flex-1">
        <div className={cx("truncate text-[13.5px]", !open && "text-fg-2")}>{title}</div>
        <div className="truncate font-mono text-[10.5px] text-fg-3">{[number ? `#${number}` : null, state, project].filter(Boolean).join(" · ")}</div>
      </div>
      {h.kind === "spec" && h.progress && <span className="chip shrink-0 font-mono text-[10.5px]">{h.progress.done}/{h.progress.total}</span>}
      {url && <ExternalLink className="size-3.5 shrink-0 text-fg-3" />}
    </>
  );
}
