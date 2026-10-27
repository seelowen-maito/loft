import { useCallback, useEffect, useState } from "react";
import { NavLink, Navigate, Route, Routes } from "react-router";
import { Bot, CircleDot, GitBranch, ListTodo, Settings as SettingsIcon, ShieldAlert, Smartphone } from "lucide-react";
import { api } from "./lib/api";
import { useLiveConnection, useServerEvents, type ServerEvent } from "./lib/events";
import { cx } from "./lib/format";
import ReposPage from "./pages/Repos";
import AgentsPage from "./pages/agents/AgentsPage";
import IssuesPage from "./pages/Issues";
import SettingsPage from "./pages/Settings";
import SimulatorPage from "./pages/Simulator";
import TasksPage from "./pages/Tasks";
import { UpdateBanner } from "./components/UpdateBanner";
import { UpdateStatus } from "./components/UpdateStatus";
import { UsageMeter } from "./components/UsageMeter";
import { TempMeter } from "./components/TempMeter";

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

function PendingBadge() {
  const [count, setCount] = useState(0);
  const refresh = useCallback(() => {
    api.permissions().then((p) => setCount(p.filter((r) => r.status === "pending").length)).catch(() => {});
  }, []);
  useEffect(refresh, [refresh]);
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "permission") refresh();
    }, [refresh]),
  );
  if (!count) return null;
  return (
    <NavLink to="/agents" className="flex items-center gap-1.5 rounded-full border border-warn/30 bg-warn/10 px-2.5 py-1 font-mono text-[11px] text-warn">
      <ShieldAlert className="size-3.5" />
      {count}<span className="max-md:hidden"> permission{count > 1 ? "s" : ""} pending</span>
    </NavLink>
  );
}

export default function App() {
  const live = useLiveConnection();
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
              <Bot className="size-3.5" /> <span className="max-md:hidden">Agents</span>
            </NavLink>
            <NavLink to="/issues" className={tab} aria-label="Issues">
              <CircleDot className="size-3.5" /> <span className="max-md:hidden">Issues</span>
            </NavLink>
            <NavLink to="/tasks" className={tab} aria-label="Tasks">
              <ListTodo className="size-3.5" /> <span className="max-md:hidden">Tasks</span>
            </NavLink>
            <NavLink to="/simulator" className={tab} aria-label="Simulator">
              <Smartphone className="size-3.5" /> <span className="max-md:hidden">Simulator</span>
            </NavLink>
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
      <main className="min-h-0 flex-1">
        <Routes>
          <Route path="/" element={<Navigate to="/repos" replace />} />
          <Route path="/repos" element={<ReposPage />} />
          <Route path="/agents" element={<AgentsPage />} />
          <Route path="/agents/:id" element={<AgentsPage />} />
          <Route path="/issues" element={<IssuesPage />} />
          <Route path="/issues/:project/:number" element={<IssuesPage />} />
          <Route path="/tasks" element={<TasksPage />} />
          <Route path="/tasks/:id" element={<TasksPage />} />
          <Route path="/simulator" element={<SimulatorPage />} />
          <Route path="/settings" element={<SettingsPage />} />
        </Routes>
      </main>
    </div>
  );
}
