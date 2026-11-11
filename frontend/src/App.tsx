import { useCallback, useEffect, useRef, useState } from "react";
import { Link, NavLink, Navigate, Route, Routes, useLocation, useNavigate, useParams } from "react-router";
import { Bot, GitBranch, ListTodo, Settings as SettingsIcon, ShieldAlert, Smartphone } from "lucide-react";
import { api, type Agent, type Settings } from "./lib/api";
import { useLiveConnection, useServerEvents, type ServerEvent } from "./lib/events";
import { cx } from "./lib/format";
import { oldTasksTarget } from "./lib/oldTasks";
import { mergeLoaded, unreadBadge, unreadCount, unreadTitle } from "./lib/unread";
import { pendingItems, type PendingItem } from "./lib/pending";
import ReposPage from "./pages/Repos";
import AgentsPage from "./pages/agents/AgentsPage";
import SettingsPage, { SETTINGS_SAVED } from "./pages/Settings";
import CleanPage from "./pages/Clean";
import SimulatorPage from "./pages/Simulator";
import TasksPage from "./pages/Tasks";
import { UpdateBanner } from "./components/UpdateBanner";
import { UpdateStatus } from "./components/UpdateStatus";
import { UsageMeter } from "./components/UsageMeter";
import { TempMeter } from "./components/TempMeter";
import { Shortcuts } from "./components/Shortcuts";

function Clock() {
  const [now, setNow] = useState(new Date());
  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(t);
  }, []);
  return (
    <span className="font-mono text-[11px] tabular-nums text-fg-3">
      {now.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })}
    </span>
  );
}

function Logo() {
  // 8x8 pixel house
  const rows = ["...##...", "..####..", ".######.", "########", ".#....#.", ".#.##.#.", ".#.##.#.", ".######."];
  return (
    <svg viewBox="0 0 8 8" className="size-5" shapeRendering="crispEdges" aria-hidden>
      {rows.flatMap((r, y) => [...r].map((c, x) => (c === "#" ? <rect key={`${x}${y}`} x={x} y={y} width={1} height={1} fill="var(--color-accent)" /> : null)))}
    </svg>
  );
}

/** Pending permission requests: opens the oldest one's card in its agent's chat; with several, also lists them all. */
function PendingBadge() {
  const [items, setItems] = useState<PendingItem[]>([]);
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const navigate = useNavigate();
  const refresh = useCallback(() => {
    // Without the agents the requests still show, unnamed.
    Promise.all([api.permissions(), api.agents().catch(() => [])]).then(([p, a]) => setItems(pendingItems(p, a))).catch(() => {});
  }, []);
  useEffect(refresh, [refresh]);
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "permission") refresh();
    }, [refresh]),
  );
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  const count = items.length;
  if (!count) return null;
  const pill = "flex items-center gap-1.5 rounded-full border border-warn/30 bg-warn/10 px-2.5 py-1 font-mono text-[11px] text-warn";
  const label = (
    <>
      <ShieldAlert className="size-3.5" />
      {count}<span className="max-md:hidden"> permission{count > 1 ? "s" : ""} pending</span>
    </>
  );
  if (count === 1) {
    return <Link to={items[0].href} className={pill} title={items[0].title}>{label}</Link>;
  }
  return (
    <div ref={box} className="relative">
      <button
        type="button"
        className={pill}
        title={`Oldest: ${items[0].title}`}
        aria-expanded={open}
        onClick={() => {
          if (!open) navigate(items[0].href);
          setOpen(!open);
        }}
      >
        {label}
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-full z-50 mt-2 flex w-72 flex-col rounded-lg border border-line-strong bg-ink-800/95 p-1 shadow-xl backdrop-blur">
          {items.map((i) => (
            <Link key={i.id} to={i.href} role="menuitem" title={i.title} onClick={() => setOpen(false)} className="flex items-baseline gap-2 rounded-md px-2.5 py-1.5 text-[12.5px] hover:bg-white/5">
              <span className="shrink-0 text-fg">{i.agent}</span>
              <code className="min-w-0 truncate font-mono text-[11.5px] text-accent">{i.tool}</code>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

/** Agents with an Unread reply, kept live from any page; it also sets the browser tab title. Reloads on reconnect, as events sent while offline are lost. */
function useUnreadCount(live: boolean) {
  const [agents, setAgents] = useState<Agent[]>([]);
  // Updates that arrive while the list loads are newer than it (opening an unread chat on a fresh load marks it read meanwhile).
  const fresh = useRef(new Map<string, Agent | null>());
  useEffect(() => {
    if (!live) return;
    fresh.current.clear();
    api.agents().then((as) => setAgents(mergeLoaded(as, fresh.current))).catch(() => {});
  }, [live]);
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "agent_updated") {
        fresh.current.set(e.agent.id, e.agent);
        setAgents((as) => (as.some((a) => a.id === e.agent.id) ? as.map((a) => (a.id === e.agent.id ? { ...a, ...e.agent } : a)) : [...as, e.agent]));
      } else if (e.type === "agent_deleted") {
        fresh.current.set(e.agent_id, null);
        setAgents((as) => as.filter((a) => a.id !== e.agent_id));
      }
    }, []),
  );
  const count = unreadCount(agents);
  useEffect(() => {
    document.title = unreadTitle(count);
  }, [count]);
  return count;
}

/** The Agents tab's Unread reply dot. Its own component so agent events re-render only the dot, not App and the page under it. */
function UnreadDot({ live }: { live: boolean }) {
  const unread = useUnreadCount(live);
  if (!unread) return null;
  return (
    <span className="absolute -right-2 -top-2 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-accent px-0.5 font-mono text-[9px] font-medium leading-none text-ink-950" title={`${unread} Unread repl${unread > 1 ? "ies" : "y"}`}>
      {unreadBadge(unread)}
    </span>
  );
}

/** Whether the Simulator tab is on (Settings → Navigation). Off until the settings load. */
function useShowSimulator() {
  const [show, setShow] = useState(false);
  useEffect(() => {
    api.settings().then((v) => setShow(v.settings.show_simulator)).catch(() => {});
    const onSaved = (e: Event) => setShow((e as CustomEvent<Settings>).detail.show_simulator);
    window.addEventListener(SETTINGS_SAVED, onSaved);
    return () => window.removeEventListener(SETTINGS_SAVED, onSaved);
  }, []);
  return show;
}

/** `/issues/:project/:number` from the old Issues tab: the list, searched for `#number` in that project. */
function OldIssue() {
  const { project = "", number = "" } = useParams();
  return <Navigate to={`/issues?q=${encodeURIComponent(`#${number}`)}&project=${encodeURIComponent(project)}`} replace />;
}

function OldTask() {
  const { id } = useParams();
  return <Navigate to={oldTasksTarget(id, window.location.search)} replace />;
}

/** The Issues tab: from an agent's page, on that agent's group (the Issues page turns `from` into it). */
function issuesLink(path: string): string {
  const id = /^\/agents\/([^/]+)/.exec(path)?.[1];
  return id ? `/issues?from=${encodeURIComponent(id)}` : "/issues";
}

export default function App() {
  const location = useLocation();
  const live = useLiveConnection();
  const showSimulator = useShowSimulator();
  const tab = ({ isActive }: { isActive: boolean }) =>
    cx(
      "flex items-center gap-1.5 rounded-full px-3 py-1.5 text-[13px] transition-colors",
      isActive ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg",
    );
  return (
    <div className="flex h-full flex-col">
      <header className="sticky top-0 z-40 border-b border-line bg-ink-950/80 backdrop-blur-xl">
        <div className="mx-auto flex h-14 max-w-[1600px] items-center gap-6 px-5 max-md:gap-3 max-md:px-4">
          <NavLink to="/" className="flex items-center gap-2.5">
            <Logo />
            <span className="text-[15px] font-medium tracking-tight max-sm:hidden">loft</span>
          </NavLink>
          <nav className="flex items-center gap-1 rounded-full border border-line bg-ink-900 p-1">
            <NavLink to="/repos" className={tab} aria-label="Repos">
              <GitBranch className="size-3.5" /> <span className="max-md:hidden">Repos</span>
            </NavLink>
            <NavLink to="/agents" className={tab} aria-label="Agents">
              {/* The dot sits on the icon so it still shows on phones, where the label hides. */}
              <span className="relative">
                <Bot className="size-3.5" />
                <UnreadDot live={live} />
              </span>{" "}
              <span className="max-md:hidden">Agents</span>
            </NavLink>
            <NavLink to={issuesLink(location.pathname)} className={tab} aria-label="Issues">
              <ListTodo className="size-3.5" /> <span className="max-md:hidden">Issues</span>
            </NavLink>
            {showSimulator && (
              <NavLink to="/simulator" className={tab} aria-label="Simulator">
                <Smartphone className="size-3.5" /> <span className="max-md:hidden">Simulator</span>
              </NavLink>
            )}
          </nav>
          <div className="ml-auto flex items-center gap-4 max-md:gap-3">
            <PendingBadge />
            {/* The meters and the clock don't fit on a phone; the live dot and settings do. */}
            <div className="flex items-center gap-4 max-lg:hidden">
              <UpdateStatus live={live} />
              <UsageMeter live={live} />
              <TempMeter live={live} />
            </div>
            <span className="flex items-center gap-1.5 font-mono text-[11px] text-fg-3" title={live ? "Live event stream connected" : "Reconnecting to backend…"}>
              <span className={cx("size-1.5 rounded-full", live ? "bg-good animate-pulse-soft" : "bg-bad")} />
              <span className="max-sm:hidden">{live ? "live" : "offline"}</span>
            </span>
            <span className="max-lg:hidden">
              <Clock />
            </span>
            <NavLink to="/settings" title="Settings" className={({ isActive }) => cx("rounded-full p-1.5 transition-colors", isActive ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
              <SettingsIcon className="size-4" />
            </NavLink>
          </div>
        </div>
      </header>
      <UpdateBanner live={live} />
      <Shortcuts />
      <main className="min-h-0 flex-1">
        <Routes>
          <Route path="/" element={<Navigate to="/repos" replace />} />
          <Route path="/repos" element={<ReposPage />} />
          <Route path="/agents" element={<AgentsPage />} />
          <Route path="/agents/:id" element={<AgentsPage />} />
          <Route path="/agents/:id/interview" element={<AgentsPage />} />
          {/* The old GitHub Issues tab's links: each opens that issue, found in the list. */}
          <Route path="/issues" element={<TasksPage />} />
          <Route path="/issues/:id" element={<TasksPage />} />
          <Route path="/issues/:project/:number" element={<OldIssue />} />
          {/* Issues were once "tasks": old links keep working. */}
          <Route path="/tasks" element={<OldTask />} />
          <Route path="/tasks/:id" element={<OldTask />} />
          <Route path="/simulator" element={<SimulatorPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/settings/clean" element={<CleanPage />} />
        </Routes>
      </main>
    </div>
  );
}
