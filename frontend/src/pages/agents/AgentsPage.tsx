import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useNavigate, useParams } from "react-router";
import { PanelRightClose, PanelRightOpen, Plus } from "lucide-react";
import { api, type Agent, type GrillSession, type Message, type PermissionRequest, type PullRequest } from "../../lib/api";
import { useServerEvents, type ServerEvent } from "../../lib/events";
import { cx, groupByProject, timeAgo } from "../../lib/format";
import { ProjectHeader, useCollapsed } from "../../components/ProjectGroup";
import { Sticker } from "../../components/Sticker";
import { RunTimer } from "../../components/RunTimer";
import { Chat } from "./Chat";
import { Inspector } from "./Inspector";
import { NewAgentDialog } from "./NewAgentDialog";
import { statusOf } from "./Status";
import { saveLastAgent, selectedAgent } from "../../lib/lastAgent";
import { groupUnread, shouldMarkRead, showsUnreadDot } from "../../lib/unread";

const COLLAPSED_KEY = "loft:collapsed-projects";
const INSPECTOR_KEY = "loft:inspector-hidden";
// Tailwind's xl: the inspector is a column at or above it, an overlay below it.
const WIDE = window.matchMedia("(min-width: 80rem)");
const onWideChange = (cb: () => void) => {
  WIDE.addEventListener("change", cb);
  return () => WIDE.removeEventListener("change", cb);
};
const onVisibilityChange = (cb: () => void) => {
  document.addEventListener("visibilitychange", cb);
  return () => document.removeEventListener("visibilitychange", cb);
};

export default function AgentsPage() {
  const { id: routeId } = useParams();
  const navigate = useNavigate();
  const [agents, setAgents] = useState<Agent[] | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [perms, setPerms] = useState<PermissionRequest[]>([]);
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [creating, setCreating] = useState(false);
  const [grill, setGrill] = useState<GrillSession | null>(null);
  // Project groups folded away in the roster, remembered per browser.
  const folded = useCollapsed(COLLAPSED_KEY);

  // Inspector: a remembered column on wide screens, an overlay opened on demand on narrow ones.
  const wide = useSyncExternalStore(onWideChange, () => WIDE.matches);
  const [panel, setPanel] = useState(() => {
    try {
      return localStorage.getItem(INSPECTOR_KEY) !== "1";
    } catch {
      return true;
    }
  });
  const [overlay, setOverlay] = useState(false);
  const inspectorOpen = wide ? panel : overlay;
  const toggleInspector = () => {
    if (!wide) return setOverlay((o) => !o);
    setPanel((p) => {
      try {
        localStorage.setItem(INSPECTOR_KEY, p ? "1" : "0");
      } catch {
        /* storage unavailable */
      }
      return !p;
    });
  };
  useEffect(() => {
    if (wide) setOverlay(false);
  }, [wide]);
  useEffect(() => {
    if (!overlay) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOverlay(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [overlay]);

  useEffect(() => {
    api.agents().then(setAgents);
    api.permissions().then(setPerms);
    api.pullRequests().then(setPrs);
  }, []);

  // `/agents` opens on the last agent viewed in this browser, or the first if it is gone.
  const selectedId = selectedAgent(routeId, agents);
  const selected = agents?.find((a) => a.id === selectedId) ?? null;
  useEffect(() => {
    if (selected) saveLastAgent(selected.id);
  }, [selected?.id]);

  // Seeing the open chat clears its Unread reply, also when the tab comes back or a reply lands while both hold.
  const visible = useSyncExternalStore(onVisibilityChange, () => document.visibilityState === "visible");
  useEffect(() => {
    if (!shouldMarkRead(selected, selectedId, visible)) return;
    setAgents((as) => as?.map((a) => (a.id === selectedId ? { ...a, unread_reply: false } : a)) ?? as);
    api.markRead(selectedId!).catch(() => {});
  }, [selected?.unread_reply, selectedId, visible]);

  const loadMessages = useCallback(() => {
    if (selectedId) api.messages(selectedId).then(setMessages);
  }, [selectedId]);

  useEffect(() => {
    setMessages([]);
    loadMessages();
  }, [loadMessages]);

  useEffect(() => {
    setGrill(null);
    if (selectedId) api.grill(selectedId).then(setGrill);
  }, [selectedId]);

  useServerEvents(
    useCallback(
      (e: ServerEvent) => {
        switch (e.type) {
          case "agent_updated":
            setAgents((as) => {
              if (!as) return as;
              const i = as.findIndex((a) => a.id === e.agent.id);
              if (i < 0) return [...as, e.agent];
              const next = [...as];
              next[i] = { ...next[i], ...e.agent };
              return next;
            });
            break;
          case "agent_deleted":
            setAgents((as) => as?.filter((a) => a.id !== e.agent_id) ?? as);
            if (e.agent_id === selectedId) navigate("/agents");
            break;
          case "message":
            if (e.message.agent_id === selectedId) {
              // A known id is that message updated (an end-of-planning card marked sent).
              setMessages((ms) => (ms.some((m) => m.id === e.message.id) ? ms.map((m) => (m.id === e.message.id ? e.message : m)) : [...ms, e.message]));
            }
            break;
          case "pull_request":
            setPrs((ps) => {
              const i = ps.findIndex((p) => p.id === e.pr.id);
              if (i < 0) return [e.pr, ...ps];
              const next = [...ps];
              next[i] = e.pr;
              return next;
            });
            break;
          case "grill":
            if (e.session.agent_id === selectedId) setGrill(e.session.state === "ended" ? null : e.session);
            break;
          case "permission":
            setPerms((ps) => {
              const i = ps.findIndex((p) => p.id === e.request.id);
              if (i < 0) return [e.request, ...ps];
              const next = [...ps];
              next[i] = e.request;
              return next;
            });
            break;
        }
      },
      [selectedId, navigate],
    ),
  );

  const permMap = useMemo(() => new Map(perms.map((p) => [p.id, p])), [perms]);
  const agentPerms = useMemo(() => perms.filter((p) => p.agent_id === selectedId), [perms, selectedId]);
  const agentPrs = useMemo(() => prs.filter((p) => p.agent_id === selectedId), [prs, selectedId]);
  const pendingBy = useMemo(() => {
    const m = new Map<string, number>();
    perms.forEach((p) => p.status === "pending" && m.set(p.agent_id, (m.get(p.agent_id) ?? 0) + 1));
    return m;
  }, [perms]);

  // Roster order, set by dragging a row (or Alt+↑/↓) within its project group.
  // The ref is read by drag handlers, which can fire before React re-renders with the new state.
  const dragging = useRef<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const saveOrder = (as: Agent[] | null) => as && api.reorderAgents(as.map((a) => a.id)).catch(() => api.agents().then(setAgents));

  const working = agents?.filter((a) => a.running).length ?? 0;
  const groups = useMemo(() => groupByProject(agents ?? []), [agents]);
  const projects = useMemo(() => groups.map((g) => g.project).filter(Boolean), [groups]);

  return (
    <div
      className={cx(
        "relative grid h-full overflow-hidden transition-[grid-template-columns] duration-300 ease-out motion-reduce:transition-none max-xl:grid-cols-[240px_minmax(0,1fr)] max-md:grid-cols-1",
        panel ? "grid-cols-[272px_minmax(0,1fr)_380px]" : "grid-cols-[272px_minmax(0,1fr)_0px]",
      )}
    >
      {/* roster */}
      <aside className="flex min-h-0 flex-col border-r border-line bg-ink-950 max-md:hidden">
        <div className="flex items-center justify-between px-4 pb-3 pt-5">
          <div>
            <div className="eyebrow">Agents</div>
            <div className="mt-1 font-mono text-[11px] text-fg-3">
              {agents?.length ?? 0} total · <span className={working ? "text-accent" : ""}>{working} working</span>
            </div>
          </div>
          <button className="btn-primary size-8 justify-center p-0" onClick={() => setCreating(true)} aria-label="New agent" title="New agent">
            <Plus className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-1 overflow-y-auto px-2 pb-4">
          {groups.map((g) => {
            const open = !projects.length || folded.isOpen(g.project);
            const groupWorking = g.agents.filter((a) => a.running).length;
            const groupPending = g.agents.reduce((n, a) => n + (pendingBy.get(a.id) ?? 0), 0);
            const groupUnreadCount = groupUnread(g.agents, selectedId);
            return (
              <div key={g.project} className="space-y-1">
                {projects.length > 0 && (
                  <ProjectHeader name={g.project || "Other"} open={open} onToggle={() => folded.toggle(g.project)}>
                    {!open && groupWorking > 0 && <span className="text-accent">{groupWorking} working</span>}
                    {!open && groupUnreadCount > 0 && <span className="text-accent">{groupUnreadCount} unread</span>}
                    {!open && groupPending > 0 && <span className="rounded-full bg-warn px-1.5 text-[10px] text-ink-950">{groupPending}</span>}
                    <span>{g.agents.length}</span>
                  </ProjectHeader>
                )}
                {open && g.agents.map((a, i) => {
                  const s = statusOf(a);
                  const pending = pendingBy.get(a.id);
                  return (
                    <button
                      key={a.id}
                      onClick={() => navigate(`/agents/${a.id}`)}
                      draggable
                      onDragStart={(e) => {
                        e.dataTransfer.effectAllowed = "move";
                        dragging.current = a.id;
                        setDragId(a.id);
                      }}
                      onDragOver={(e) => {
                        const id = dragging.current;
                        if (!id || agents?.find((x) => x.id === id)?.project.trim() !== a.project.trim()) return;
                        e.preventDefault();
                        setAgents((as) => as && moved(as, id, a.id));
                      }}
                      onDrop={(e) => e.preventDefault()}
                      onDragEnd={() => {
                        dragging.current = null;
                        setDragId(null);
                        saveOrder(agents);
                      }}
                      onKeyDown={(e) => {
                        if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
                        const target = g.agents[i + (e.key === "ArrowUp" ? -1 : 1)];
                        if (!target || !agents) return;
                        e.preventDefault();
                        const next = moved(agents, a.id, target.id);
                        setAgents(next);
                        saveOrder(next);
                      }}
                      title="Drag or Alt+↑/↓ to reorder"
                      className={cx(
                        "group flex w-full items-center gap-3 rounded-xl px-2.5 py-2.5 text-left transition",
                        a.id === selectedId ? "bg-white/[0.06]" : "hover:bg-white/[0.03]",
                        a.id === dragId && "opacity-40",
                      )}
                    >
                      <div className="relative">
                        <Sticker avatar={a.avatar} lead={a.lead} size={40} />
                        <span className={cx("absolute -bottom-0.5 -right-0.5 size-3 rounded-full border-2 border-ink-950", s.dot)} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-[13.5px]">{a.name}</span>
                          {showsUnreadDot(a, selectedId) && <span className="size-2 shrink-0 rounded-full bg-accent" aria-label="Unread reply" title="Unread reply" />}
                          {!!pending && <span className="rounded-full bg-warn px-1.5 font-mono text-[10px] text-ink-950">{pending}</span>}
                          {a.running && <RunTimer startedAt={a.last_run_at} className="ml-auto shrink-0 font-mono text-[10.5px] text-fg-3" />}
                        </div>
                        <div className="mt-0.5 truncate font-mono text-[10.5px] text-fg-3">
                          {a.running || a.status === "waiting" ? (
                            <span className={s.text}>{a.status_text || s.label}</span>
                          ) : (
                            <>{a.title || "agent"} · {a.last_run_at ? timeAgo(a.last_run_at) : "never run"}</>
                          )}
                        </div>
                      </div>
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      </aside>

      {/* main */}
      <section className="min-h-0 min-w-0 border-r border-line max-xl:border-r-0">
        {agents && agents.length === 0 ? (
          <Empty onCreate={() => setCreating(true)} />
        ) : selected ? (
          <Chat
            agent={selected}
            agents={agents ?? []}
            messages={messages}
            perms={permMap}
            prs={agentPrs}
            grill={grill}
            headerActions={
              <button
                className="btn-ghost"
                onClick={toggleInspector}
                aria-expanded={inspectorOpen}
                aria-controls="inspector"
                aria-label={inspectorOpen ? "Hide agent settings" : "Show agent settings"}
                title={inspectorOpen ? "Hide agent settings" : "Show agent settings"}
              >
                {inspectorOpen ? <PanelRightClose className="size-3.5" /> : <PanelRightOpen className="size-3.5" />}
              </button>
            }
          />
        ) : (
          <div className="skeleton m-6 h-40 rounded-2xl" />
        )}
      </section>

      {/* inspector; below xl a drawer over the chat, closed by the backdrop or Esc */}
      <div
        aria-hidden
        onClick={() => setOverlay(false)}
        className={cx(
          "absolute inset-0 z-10 bg-black/40 transition-opacity duration-300 motion-reduce:transition-none xl:hidden",
          overlay ? "opacity-100" : "pointer-events-none opacity-0",
        )}
      />
      <aside
        id="inspector"
        inert={!inspectorOpen}
        className={cx(
          "min-h-0 overflow-hidden bg-ink-900/60 motion-reduce:transition-none",
          "max-xl:absolute max-xl:inset-y-0 max-xl:right-0 max-xl:z-20 max-xl:w-[min(380px,calc(100%-3rem))] max-xl:border-l max-xl:border-line max-xl:bg-ink-900 max-xl:shadow-2xl max-xl:transition-transform max-xl:duration-300 max-xl:ease-out",
          !overlay && "max-xl:translate-x-full",
        )}
      >
        {/* Fixed width so the content is clipped, not reflowed, while the column animates. */}
        <div className="h-full w-[380px] max-xl:w-full">
          {selected && <Inspector key={selected.id} agent={selected} projects={projects} perms={agentPerms} onCleared={() => setMessages([])} />}
        </div>
      </aside>

      {creating && (
        <NewAgentDialog
          projects={projects}
          onClose={() => setCreating(false)}
          onCreated={(a) => {
            setCreating(false);
            setAgents((as) => (as?.some((x) => x.id === a.id) ? as : [...(as ?? []), a]));
            navigate(`/agents/${a.id}`);
          }}
        />
      )}
    </div>
  );
}

/** `as` with agent `id` moved into `targetId`'s slot; unchanged across project groups. */
function moved(as: Agent[], id: string, targetId: string): Agent[] {
  const from = as.findIndex((a) => a.id === id);
  const to = as.findIndex((a) => a.id === targetId);
  if (from < 0 || to < 0 || from === to || as[from].project.trim() !== as[to].project.trim()) return as;
  const next = [...as];
  next.splice(to, 0, ...next.splice(from, 1));
  return next;
}

function Empty({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="relative flex h-full flex-col items-center justify-center overflow-hidden px-6 text-center">
      <div className="hairline-grid absolute inset-0 [mask-image:radial-gradient(ellipse_at_center,black,transparent_70%)]" />
      <div className="relative">
        <div className="eyebrow">No agents yet</div>
        <h2 className="mt-3 font-serif text-[48px] leading-[1.02]">
          A small crew, <em className="text-accent">always on.</em>
        </h2>
        <p className="mx-auto mt-4 max-w-md text-[14px] text-fg-2">
          Agents are Claude Code workers with their own workspace, memory, schedule, and permissions. Give them a personality and something to own.
        </p>
        <button className="btn-primary mt-7 px-5 py-2" onClick={onCreate}>
          <Plus className="size-4" /> Create your first agent
        </button>
      </div>
    </div>
  );
}

