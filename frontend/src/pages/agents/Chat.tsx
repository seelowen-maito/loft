import { Fragment, memo, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  AlertCircle, ArrowUp, Brain, ChevronRight, Clock, FileText, Folder, Hourglass, ImagePlus, Info, ListTodo, Loader2, Play, Plus, RefreshCw, RotateCcw, Square, Wrench, X,
} from "lucide-react";
import { api, type Agent, type GrillSession, type Message, type PermissionRequest, type PullRequest } from "../../lib/api";
import { cx, fmtNum, fmtQuiet, fmtSchedule, timeAgo } from "../../lib/format";
import { Sticker } from "../../components/Sticker";
import { PermissionCard } from "./PermissionCard";
import { PullRequestCard } from "./PullRequestCard";
import { GrillBar } from "./GrillBar";
import { GrillRound } from "./GrillRound";
import { PlanCard } from "./PlanCard";
import { EndCard } from "./EndCard";
import { TaskPicker } from "./TaskPicker";
import { TASK_LINK, TaskChip } from "../../components/TaskChip";
import { StatusPill, runWatch } from "./Status";
import { RunIndicator } from "../../components/RunTimer";
import { lastFinishedRun } from "../../lib/runTimer";
import { useAutoGrow } from "../../lib/autoGrow";
import { fyiTitle, isFyi } from "../../lib/agentMessage";

type Item =
  | { t: "msg"; m: Message }
  | { t: "tool"; use: Message; result?: Message }
  | { t: "perm"; m: Message }
  | { t: "dm"; use: Message; result?: Message };

function buildItems(messages: Message[]): Item[] {
  const results = new Map<string, Message>();
  for (const m of messages) if (m.kind === "tool_result" && m.meta?.tool_use_id) results.set(m.meta.tool_use_id, m);
  const items: Item[] = [];
  for (const m of messages) {
    if (m.kind === "tool_result") continue;
    // Claude Code's internal deferred-tool loader is noise in the transcript.
    if (m.kind === "tool_use" && m.meta?.name === "ToolSearch") continue;
    // A message this agent sent another agent reads as a message, unless sending failed.
    if (m.kind === "tool_use" && m.meta?.name === "mcp__loft__message_agent") {
      const result = results.get(m.meta?.id);
      if (!result?.meta?.is_error) {
        items.push({ t: "dm", use: m, result });
        continue;
      }
    }
    if (m.kind === "tool_use") items.push({ t: "tool", use: m, result: results.get(m.meta?.id) });
    else if (m.kind === "permission") items.push({ t: "perm", m });
    else items.push({ t: "msg", m });
  }
  return items;
}

// Agents' links point at GitHub, docs, previews: open them in a new tab so the chat stays put.
const LOOPBACK = /^(https?:\/\/)(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?=[:/]|$)/i;

/** From another device, `localhost` is that device: point loopback links at the host serving loft instead. */
function reachable(href: string | undefined) {
  const host = window.location.hostname;
  if (!href || LOOPBACK.test(`${window.location.protocol}//${host}`)) return href;
  return href.replace(LOOPBACK, `$1${host}`);
}

const MD_COMPONENTS: Components = {
  a: ({ node: _node, href: raw, children, ...rest }) => {
    const task = raw && TASK_LINK.exec(raw);
    if (task) return <TaskChip id={task[1]} />;
    const href = reachable(raw);
    const external = !!href && /^(https?:)?\/\//i.test(href);
    return (
      <a href={href} {...rest} {...(external ? { target: "_blank", rel: "noopener noreferrer" } : {})}>
        {children}
      </a>
    );
  },
};

type Command = { name: string; description: string };
const COMMANDS: Command[] = [
  { name: "/clear", description: "Clear the conversation context and start fresh. Memory is kept." },
  { name: "/compact", description: "Summarize the conversation so later runs carry less context. Add instructions after it to steer the summary." },
  { name: "/code", description: "Open this agent's workspace folder in VS Code." },
  { name: "/grill", description: "Plan an idea: the agent grills you on it in rounds of questions, each with a recommendation. Add the topic after it." },
];

const SUGGESTIONS = [
  "Introduce yourself and tell me what you can do.",
  "Redraw your avatar as a tiny astronaut with a green visor.",
  "Clone one of my projects into your workspace and summarize it.",
  "What do you remember about me?",
];

export function Chat({
  agent, agents, messages, perms, prs, grill, headerActions,
}: {
  agent: Agent;
  agents: Agent[];
  messages: Message[];
  perms: Map<string, PermissionRequest>;
  prs: PullRequest[];
  /** The agent's open interview, if any. */
  grill: GrillSession | null;
  /** Extra buttons at the end of the header, e.g. the inspector toggle. */
  headerActions?: React.ReactNode;
}) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [cmdError, setCmdError] = useState<string | null>(null);
  const [cmdNote, setCmdNote] = useState<string | null>(null);
  const [menuIndex, setMenuIndex] = useState(0);
  const [showEarlier, setShowEarlier] = useState(false);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const picker = useRef<HTMLInputElement>(null);
  // dragenter/dragleave fire for every child crossed; count them to know when the drag really left.
  const [dragDepth, setDragDepth] = useState(0);
  const scroller = useRef<HTMLDivElement>(null);
  const composer = useRef<HTMLTextAreaElement>(null);
  useAutoGrow(composer, text, 1);
  const stick = useRef(true);
  const items = useMemo(() => buildItems(messages), [messages]);
  // Everything before the latest /clear is hidden (not deleted) until asked for.
  const lastClear = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.t === "msg" && it.m.kind === "clear") return i;
    }
    return -1;
  }, [items]);
  const visible = showEarlier || lastClear < 0 ? items : items.slice(lastClear);
  const hiddenCount = showEarlier || lastClear < 0 ? 0 : lastClear;
  const fresh = visible.every((it) => it.t === "msg" && it.m.kind === "clear");
  const lastRun = useMemo(() => lastFinishedRun(messages), [messages]);

  const slash = text.startsWith("/") && !/\s/.test(text.trim());
  const matches = slash ? COMMANDS.filter((c) => c.name.startsWith(text.trim().toLowerCase())) : [];
  // "@ho" at the end of the text: suggest agents to mention (the message also goes to them), and the
  // entries of the folder typed so far, like a shell from the agent's project folder ("@../imole/pro").
  // Only agents in the same project: the backend doesn't deliver to others.
  const atToken = /(?:^|\s)@([^\s@]*)$/.exec(text)?.[1];
  const mentionQuery = atToken?.toLowerCase();
  const agentMatches =
    mentionQuery === undefined ? [] : agents.filter((a) => a.id !== agent.id && a.project.trim() === agent.project.trim() && a.name.toLowerCase().startsWith(mentionQuery)).slice(0, 6);
  const fileDir = atToken === undefined ? undefined : atToken.slice(0, atToken.lastIndexOf("/") + 1);
  const filePrefix = atToken?.slice(fileDir?.length ?? 0).toLowerCase() ?? "";
  const [listing, setListing] = useState<{ typed: string; dir: string; entries: { name: string; dir: boolean }[] }>();
  useEffect(() => {
    if (fileDir === undefined) return;
    let live = true;
    api.ls(agent.id, fileDir).then(
      (r) => live && setListing({ typed: fileDir, ...r }),
      () => live && setListing({ typed: fileDir, dir: "", entries: [] }),
    );
    return () => {
      live = false;
    };
  }, [agent.id, fileDir]);
  const fileMatches =
    listing && listing.typed === fileDir
      ? listing.entries.filter((e) => e.name.toLowerCase().startsWith(filePrefix) && (filePrefix.startsWith(".") || !e.name.startsWith("."))).slice(0, 50)
      : [];
  type Pick = { agent: Agent } | { file: { name: string; dir: boolean } };
  const mentionMatches: Pick[] = [...agentMatches.map((a) => ({ agent: a })), ...fileMatches.map((f) => ({ file: f }))];
  // A folder keeps the menu open on its contents, like tab completion in a shell.
  const mention = (p: Pick) =>
    setText((t) => t.replace(/@[^\s@]*$/, "agent" in p ? `@${p.agent.name} ` : `@${fileDir}${p.file.name}${p.file.dir ? "/" : " "}`));
  // PR cards render right below the tool call that opened them.
  const prsByTool = useMemo(() => {
    const m = new Map<string, PullRequest[]>();
    for (const pr of prs) {
      if (!pr.tool_use_id) continue;
      m.set(pr.tool_use_id, [...(m.get(pr.tool_use_id) ?? []), pr].sort((a, b) => a.number - b.number));
    }
    return m;
  }, [prs]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [items.length, agent.status_text, agent.running, prs.length]);

  useEffect(() => {
    stick.current = true;
    setShowEarlier(false);
    // Uploads belong to the agent they were pasted for.
    setAttachments((a) => (a.forEach((x) => URL.revokeObjectURL(x.preview)), []));
    setDragDepth(0);
    setCmdError(null);
    setCmdNote(null);
  }, [agent.id]);

  // Confirmations fade on their own.
  useEffect(() => {
    if (!cmdNote) return;
    const t = setTimeout(() => setCmdNote(null), 4000);
    return () => clearTimeout(t);
  }, [cmdNote]);

  // A new /clear collapses the history again.
  useEffect(() => {
    setShowEarlier(false);
  }, [lastClear]);

  useEffect(() => {
    setMenuIndex(0);
  }, [text]);

  const clearContext = async () => {
    setCmdError(null);
    try {
      await api.resetSession(agent.id);
      setText("");
      stick.current = true;
    } catch (e) {
      setCmdError((e as Error).message);
    }
  };

  // The end-of-planning card: a fresh session, with the suggested next message waiting in the composer, unsent.
  // Stable, so a grill event doesn't re-render every memoized MessageRow.
  const clearWith = useCallback(async (next: string) => {
    setCmdError(null);
    await api.resetSession(agent.id);
    setText(next);
    stick.current = true;
    composer.current?.focus();
  }, [agent.id]);

  const compactContext = async (instructions: string) => {
    setCmdError(null);
    try {
      await api.compact(agent.id, instructions);
      setText("");
      stick.current = true;
    } catch (e) {
      setCmdError((e as Error).message);
    }
  };

  const openEditor = async () => {
    setCmdError(null);
    try {
      const r = await api.openEditor(agent.id);
      setText("");
      setCmdNote(r.url ? `Opening ${r.path} in VS Code over Remote-SSH` : `Opened ${r.path} in VS Code`);
    } catch (e) {
      setCmdError((e as Error).message);
    }
  };

  const startGrill = async (topic: string) => {
    setSending(true);
    setCmdError(null);
    stick.current = true;
    try {
      await api.grillStart(agent.id, topic ? `Grill me on ${topic}` : "Grill me");
      setText("");
    } catch (e) {
      setCmdError((e as Error).message);
    } finally {
      setSending(false);
    }
  };

  const runCommand = async (name: string) => {
    setCmdNote(null);
    if (name === "/clear") return clearContext();
    if (name === "/compact") return compactContext("");
    if (name === "/code") return openEditor();
    if (name === "/grill") {
      // Picked from the menu: leave room for the topic; Enter again starts it.
      setText("/grill ");
      setCmdNote("Add what to plan, then press Enter.");
      return;
    }
    setCmdError(`Unknown command ${name}. Available: ${COMMANDS.map((c) => c.name).join(", ")}`);
  };

  const addImages = (all: File[]) => {
    const images = all.filter((f) => f.type.startsWith("image/"));
    const files = images.slice(0, MAX_IMAGES - attachments.length);
    setCmdError(
      files.length < images.length ? `Up to ${MAX_IMAGES} images per message.`
      : images.length < all.length ? "Only images can be attached." : null,
    );
    for (const f of files) {
      const key = crypto.randomUUID();
      setAttachments((a) => [...a, { key, preview: URL.createObjectURL(f) }]);
      const update = (u: Partial<Attachment>) => setAttachments((a) => a.map((x) => (x.key === key ? { ...x, ...u } : x)));
      prepareImage(f)
        .then((blob) => api.upload(agent.id, blob))
        .then((r) => update({ file: r.file }), (e) => update({ error: (e as Error).message }));
    }
  };

  const removeImage = (key: string) =>
    setAttachments((a) => a.filter((x) => (x.key === key ? (URL.revokeObjectURL(x.preview), false) : true)));

  const uploading = attachments.some((a) => !a.file && !a.error);
  const failed = attachments.some((a) => a.error);
  const images = attachments.flatMap((a) => (a.file ? [a.file] : []));

  const [taskId, setTaskId] = useState<string | null>(null);
  const [pickingTask, setPickingTask] = useState(false);
  useEffect(() => {
    setTaskId(null);
    setPickingTask(false);
  }, [agent.id]);

  const send = async (t = text) => {
    const v = t.trim();
    if (!v && images.length === 0) return;
    if (uploading || failed) return;
    if (images.length > 0) {
      setSending(true);
      setCmdError(null);
      stick.current = true;
      try {
        await api.send(agent.id, v, images, taskId ?? undefined);
        setText("");
        setTaskId(null);
        setAttachments((a) => (a.forEach((x) => URL.revokeObjectURL(x.preview)), []));
      } catch (e) {
        setCmdError((e as Error).message);
      } finally {
        setSending(false);
      }
      return;
    }
    const compact = /^\/compact(?:\s+([\s\S]*))?$/i.exec(v);
    if (compact) return compactContext(compact[1]?.trim() ?? "");
    const grillTopic = /^\/grill(?:\s+([\s\S]*))?$/i.exec(v);
    if (grillTopic) return startGrill(grillTopic[1]?.trim() ?? "");
    if (v.startsWith("/") && !/\s/.test(v)) return runCommand(v.toLowerCase());
    setSending(true);
    setCmdError(null);
    stick.current = true;
    try {
      await api.send(agent.id, v, [], taskId ?? undefined);
      setText("");
      setTaskId(null);
    } finally {
      setSending(false);
    }
  };

  const running = !!agent.running;
  // Re-render every 30 s so the "for N min" counts move while nothing else does.
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, [running]);
  const watch = runWatch(agent, messages[messages.length - 1]?.created_at, now);
  const [restarting, setRestarting] = useState(false);
  const restart = async () => {
    setRestarting(true);
    setCmdError(null);
    try {
      await api.restartRun(agent.id);
      stick.current = true;
    } catch (e) {
      setCmdError((e as Error).message);
    } finally {
      setRestarting(false);
    }
  };

  return (
    <div
      className="relative flex h-full min-w-0 flex-col"
      onDragEnter={(e) => e.dataTransfer.types.includes("Files") && setDragDepth((d) => d + 1)}
      onDragLeave={(e) => e.dataTransfer.types.includes("Files") && setDragDepth((d) => Math.max(0, d - 1))}
      onDragOver={(e) => e.dataTransfer.types.includes("Files") && e.preventDefault()}
      onDrop={(e) => {
        if (!e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        setDragDepth(0);
        addImages([...e.dataTransfer.files]);
      }}
    >
      {dragDepth > 0 && (
        <div className="pointer-events-none absolute inset-3 z-20 grid place-items-center rounded-2xl border-2 border-dashed border-accent/50 bg-ink-950/80 backdrop-blur-sm">
          <div className="flex flex-col items-center gap-2 text-fg-2">
            <ImagePlus className="size-7 text-accent" />
            <span className="text-[14px]">Drop images to attach</span>
          </div>
        </div>
      )}
      {/* header */}
      <div className="flex items-center gap-4 border-b border-line px-6 py-4 max-md:gap-3 max-md:px-4 max-md:py-3">
        <Sticker avatar={agent.avatar} lead={agent.lead} size={52} className="max-md:!size-9" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-[17px] font-medium tracking-tight">{agent.name}</h2>
            {agent.title && <span className="truncate text-[13px] text-fg-3 max-md:hidden">{agent.title}</span>}
            {agent.project && <span className="shrink-0 rounded-full border border-line px-2 py-0.5 font-mono text-[10.5px] text-fg-3">{agent.project}</span>}
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-3">
            <StatusPill agent={agent} />
            {/* Schedule and context size are details: on a phone the status line gets the room. */}
            <span
              className="flex shrink-0 items-center gap-1 font-mono text-[11px] text-fg-3 max-md:hidden"
              title={agent.interval_minutes && agent.quiet_hours?.length ? `Paused ${fmtQuiet(agent.quiet_hours)}` : undefined}
            >
              <Clock className="size-3" /> {fmtSchedule(agent)}
              {agent.next_run_at && agent.interval_minutes > 0 && <> · next {timeAgo(agent.next_run_at)}</>}
            </span>
            {agent.context_tokens != null && (
              <span
                className={cx("shrink-0 font-mono text-[11px] tabular-nums max-md:hidden", agent.context_tokens >= 200_000 ? "text-warn" : "text-fg-3")}
                title={`${agent.context_tokens.toLocaleString()} tokens in context as of the last reply. Every turn resends it, so a big context uses your limits faster; /compact shrinks it.`}
              >
                context {fmtNum(agent.context_tokens)}
              </span>
            )}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {running ? (
            <button className="btn" onClick={() => api.stop(agent.id)}>
              <Square className="size-3 fill-current" /> Stop
            </button>
          ) : (
            <button className="btn" onClick={() => api.run(agent.id)} title="Run an autonomous check-in now">
              <Play className="size-3 fill-current" /> Wake
            </button>
          )}
          <button
            className="btn-ghost"
            title="Clear context (/clear). Memory is kept."
            onClick={clearContext}
            disabled={running}
          >
            <RotateCcw className="size-3.5" />
          </button>
          {headerActions}
        </div>
      </div>

      {/* transcript */}
      <div
        ref={scroller}
        className="min-h-0 flex-1 overflow-y-auto"
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
        }}
      >
        <div className="mx-auto max-w-[820px] space-y-3 px-6 py-6">
          {hiddenCount > 0 && (
            <div className="flex justify-center">
              <button className="btn-ghost font-mono text-[11.5px]" onClick={() => setShowEarlier(true)}>
                Show {hiddenCount} earlier item{hiddenCount === 1 ? "" : "s"} from before /clear
              </button>
            </div>
          )}
          {visible.map((it) => {
            if (it.t === "tool") {
              const cards = prsByTool.get(it.use.meta?.id) ?? [];
              return (
                <Fragment key={it.use.id}>
                  <ToolRow use={it.use} result={it.result} />
                  {cards.map((pr) => <PullRequestCard key={pr.id} pr={pr} />)}
                </Fragment>
              );
            }
            if (it.t === "dm") return <SentToAgent key={it.use.id} use={it.use} result={it.result} agent={agent} agents={agents} />;
            if (it.t === "perm") {
              const req = perms.get(it.m.meta?.request_id);
              return req ? <PermissionCard key={it.m.id} req={req} /> : null;
            }
            // Only round cards read the session: other rows get null and skip re-rendering on grill events.
            const session = it.m.meta?.grill?.role === "round" ? grill : null;
            return <MessageRow key={it.m.id} m={it.m} agent={agent} agents={agents} grill={session} onClearWith={clearWith} />;
          })}
          {(items.length === 0 || fresh) && (
            <div className="flex flex-col items-center py-16 text-center">
              <Sticker avatar={agent.avatar} lead={agent.lead} size={112} className="animate-rise" />
              <div className="mt-5 font-serif text-[30px] leading-tight">Say hello to {agent.name}.</div>
              <p className="mt-2 max-w-md text-[13.5px] text-fg-2">
                It works inside its own workspace, keeps long-term memory, and asks you before it uses anything it hasn't been granted.
              </p>
              <div className="mt-6 flex max-w-xl flex-wrap justify-center gap-2">
                {SUGGESTIONS.map((s) => (
                  <button key={s} className="btn text-[12.5px] text-fg-2" onClick={() => send(s)}>
                    {s}
                  </button>
                ))}
              </div>
            </div>
          )}
          {running ? (
            <RunIndicator running startedAt={agent.last_run_at} status={agent.status_text} />
          ) : (
            lastRun && <RunIndicator running={false} durationMs={lastRun.durationMs} endedAt={lastRun.endedAt} />
          )}
        </div>
      </div>

      {/* composer */}
      <div className="border-t border-line px-6 py-4">
        <div className="mx-auto max-w-[820px]">
          {watch && (
            <div
              className={cx(
                "mb-3 flex items-center gap-3 rounded-xl border px-3 py-2",
                watch.kind === "waiting" ? "border-info/30 bg-info/[0.07]" : "border-warn/30 bg-warn/[0.07]",
              )}
              role="status"
            >
              <Hourglass className={cx("size-4 shrink-0", watch.kind === "waiting" ? "text-info" : "text-warn")} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] text-fg">
                  {watch.text}
                  {watch.kind === "waiting" && <span className="text-fg-3"> · {watch.minutes} min</span>}
                </div>
                <div className="truncate font-mono text-[11px] text-fg-3">
                  {watch.kind === "waiting" ? "Already replied. Sending a message stops the task." : "It may be stuck. Restart keeps queued messages."}
                </div>
              </div>
              <button className="btn shrink-0" onClick={restart} disabled={restarting} title="Stops this run. Queued messages are kept and run next.">
                <RefreshCw className={cx("size-3.5", restarting && "animate-spin")} /> Restart
              </button>
            </div>
          )}
          {grill && (grill.state === "active" || grill.state === "paused") && <GrillBar session={grill} />}
          {pickingTask && (
            <TaskPicker
              agents={agents}
              onPick={(id) => {
                setTaskId(id);
                setPickingTask(false);
              }}
              onClose={() => setPickingTask(false)}
            />
          )}
          {mentionMatches.length > 0 && (
            <div className="mb-2 max-h-80 overflow-y-auto rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="listbox" aria-label="Mention an agent or a file">
              {mentionMatches.map((p, i) => (
                <Fragment key={"agent" in p ? p.agent.id : p.file.name}>
                  {"file" in p && i === agentMatches.length && (
                    <div className={cx("px-3 pb-1 pt-1.5 font-mono text-[10.5px] text-fg-3", i > 0 && "border-t border-line")}>{listing?.dir ? `${listing.dir}/` : "./"}</div>
                  )}
                  <button
                    role="option"
                    aria-selected={i === menuIndex}
                    ref={(el) => {
                      if (i === menuIndex) el?.scrollIntoView({ block: "nearest" });
                    }}
                    onMouseEnter={() => setMenuIndex(i)}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      mention(p);
                    }}
                    className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", i === menuIndex && "bg-white/[0.06]")}
                  >
                    {"agent" in p ? (
                      <>
                        <Sticker avatar={p.agent.avatar} lead={p.agent.lead} size={20} />
                        <span className="text-[13px]">{p.agent.name}</span>
                        <span className="truncate text-[12px] text-fg-3">{p.agent.title}</span>
                      </>
                    ) : (
                      <>
                        {p.file.dir ? <Folder className="size-4 shrink-0 text-fg-3" /> : <FileText className="size-4 shrink-0 text-fg-3" />}
                        <span className="truncate font-mono text-[12.5px]">
                          {p.file.name}
                          {p.file.dir && "/"}
                        </span>
                      </>
                    )}
                  </button>
                  {"agent" in p && i === agentMatches.length - 1 && (
                    <div className="border-t border-line px-3 py-1.5 font-mono text-[10.5px] text-fg-3">
                      The message also goes to them, and their reply shows here. Start with @… to send it only to them.
                    </div>
                  )}
                </Fragment>
              ))}
            </div>
          )}
          {matches.length > 0 && (
            <div className="mb-2 overflow-hidden rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="listbox" aria-label="Commands">
              {matches.map((c, i) => (
                <button
                  key={c.name}
                  role="option"
                  aria-selected={i === menuIndex}
                  onMouseEnter={() => setMenuIndex(i)}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    runCommand(c.name);
                  }}
                  className={cx("flex w-full items-baseline gap-3 px-3 py-2 text-left", i === menuIndex && "bg-white/[0.06]")}
                >
                  <span className="font-mono text-[12.5px] text-accent">{c.name}</span>
                  <span className="truncate text-[12px] text-fg-3">{c.description}</span>
                </button>
              ))}
            </div>
          )}
          <div className="rounded-2xl border border-line-strong bg-ink-850 p-2 pl-2 transition focus-within:border-accent/40 focus-within:ring-4 focus-within:ring-accent/5">
            {taskId && (
              <div className="px-2 pb-2 pt-1">
                <TaskChip id={taskId} onRemove={() => setTaskId(null)} />
              </div>
            )}
            {attachments.length > 0 && (
              <div className="flex flex-wrap gap-2 px-2 pb-2 pt-1">
                {attachments.map((a) => <AttachmentTile key={a.key} a={a} onRemove={() => removeImage(a.key)} />)}
              </div>
            )}
            <div className="flex items-end gap-2">
              <button className="btn-ghost size-8 shrink-0 justify-center p-0" onClick={() => picker.current?.click()} aria-label="Attach images" title="Attach images">
                <Plus className="size-4" />
              </button>
              <button
                className={cx("btn-ghost size-8 shrink-0 justify-center p-0", (pickingTask || taskId) && "text-accent")}
                onClick={() => setPickingTask((p) => !p)}
                aria-label="Attach a task"
                title="Attach a task"
                aria-expanded={pickingTask}
              >
                <ListTodo className="size-4" />
              </button>
              <input
                ref={picker}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp"
                multiple
                hidden
                onChange={(e) => {
                  addImages([...(e.target.files ?? [])]);
                  // Let the same file be picked again.
                  e.target.value = "";
                }}
              />
              <textarea
                ref={composer}
                value={text}
                onChange={(e) => setText(e.target.value)}
                onPaste={(e) => {
                  const files = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"));
                  if (files.length === 0) return;
                  // Copying an image from a page also puts its address on the clipboard: take just the image.
                  e.preventDefault();
                  addImages(files);
                }}
                onKeyDown={(e) => {
                  if (mentionMatches.length > 0) {
                    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                      e.preventDefault();
                      setMenuIndex((i) => (i + (e.key === "ArrowDown" ? 1 : mentionMatches.length - 1)) % mentionMatches.length);
                      return;
                    }
                    if ((e.key === "Tab" || e.key === "Enter") && !e.shiftKey && !e.nativeEvent.isComposing) {
                      e.preventDefault();
                      mention(mentionMatches[Math.min(menuIndex, mentionMatches.length - 1)]);
                      return;
                    }
                  }
                  if (matches.length > 0) {
                    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                      e.preventDefault();
                      setMenuIndex((i) => (i + (e.key === "ArrowDown" ? 1 : matches.length - 1)) % matches.length);
                      return;
                    }
                    if (e.key === "Tab") {
                      e.preventDefault();
                      setText(matches[menuIndex].name);
                      return;
                    }
                    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                      e.preventDefault();
                      runCommand(matches[menuIndex].name);
                      return;
                    }
                  }
                  if (e.key === "Escape" && slash) {
                    setText("");
                    return;
                  }
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    send();
                  }
                }}
                rows={1}
                placeholder={
                  watch?.kind === "waiting" ? `${agent.name} is waiting on a background task. Sending stops it.`
                  : running ? `${agent.name} is working. Your message will be delivered next.`
                  : grill?.round != null && !grill.waiting && grill.state !== "ended" ? `Answer in the cards above. A message here goes to ${agent.name} as chat.`
                  : `Message ${agent.name}… (/ for commands, @ to mention an agent or a file)`
                }
                className="flex-1 resize-none bg-transparent py-1.5 text-[14px] outline-none placeholder:text-fg-3"
              />
              <button
                className="btn-primary size-8 justify-center p-0"
                disabled={(!text.trim() && images.length === 0) || uploading || failed || sending}
                onClick={() => send()}
                aria-label="Send"
                title={failed ? "Remove the image that failed to upload" : uploading ? "Waiting for images to upload" : undefined}
              >
                <ArrowUp className="size-4" />
              </button>
            </div>
          </div>
          {cmdError && <div className="mt-2 text-[12px] text-bad">{cmdError}</div>}
          {cmdNote && <div className="mt-2 truncate font-mono text-[11.5px] text-good">✓ {cmdNote}</div>}
          <div className="mt-2 flex justify-between font-mono text-[10.5px] text-fg-3">
            <span className="max-md:hidden">↵ send · ⇧↵ newline · / commands</span>
            <span>{agent.session_id ? `session ${agent.session_id.slice(0, 8)}` : "new session"} · ${agent.total_cost_usd.toFixed(2)} spent</span>
          </div>
        </div>
      </div>
    </div>
  );
}

/** A task message's text without the `[Task <id>]` tag and the bracketed instructions Loft adds for the agent. */
function taskText(m: Message) {
  if (!m.meta?.task_role && !m.meta?.task_id) return m.content;
  return m.content
    .replace(/^\[Task [0-9a-f-]{36}\]\s*/i, "")
    .replace(/\n\n\[(Report progress on this task|Answer on the task)[\s\S]*$/, "")
    .trim();
}

/** Chips for the tasks a message is about: attached, linked, or the task it briefs or reports on. */
function MessageTasks({ m }: { m: Message }) {
  const ids: string[] = m.meta?.task_refs ?? (m.meta?.task_id ? [m.meta.task_id] : []);
  if (!ids.length) return null;
  return (
    <div className="flex max-w-[80%] flex-wrap justify-end gap-1.5">
      {ids.map((id) => <TaskChip key={id} id={id} />)}
    </div>
  );
}

/** The agent a tool call or message names, by id or (case-insensitively) by name. */
function findAgent(agents: Agent[], key: string | undefined) {
  if (!key) return undefined;
  return agents.find((a) => a.id === key) ?? agents.find((a) => a.name.toLowerCase() === key.trim().toLowerCase());
}

/** The FYI mark on an agent message: its note opens on hover with a mouse, or on tap/Enter elsewhere.
 * The note anchors to the nearest positioned ancestor (the header row) so it stays on screen on phones. */
function FyiMark({ agentName }: { agentName: string }) {
  const [open, setOpen] = useState(false);
  const pointer = useRef("mouse");
  const root = useRef<HTMLSpanElement>(null);
  const noteId = useId();
  // A tap anywhere else closes a tapped-open note.
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);

  return (
    <span ref={root} className="flex">
      <button
        type="button"
        className="rounded border border-line px-1 py-px text-[9.5px] uppercase text-fg-2 hover:text-fg"
        aria-expanded={open}
        aria-describedby={noteId}
        onPointerDown={(e) => (pointer.current = e.pointerType)}
        onPointerEnter={(e) => e.pointerType === "mouse" && setOpen(true)}
        onPointerLeave={(e) => e.pointerType === "mouse" && setOpen(false)}
        onClick={() => setOpen((o) => (o && pointer.current !== "mouse" ? false : true))}
        onKeyDown={(e) => {
          pointer.current = "keyboard";
          if (e.key === "Escape") setOpen(false);
        }}
      >
        FYI
      </button>
      {/* Always in the DOM so screen readers get it as the mark's description; shown only when open. */}
      <span
        id={noteId}
        role="tooltip"
        className={cx(
          open
            ? "pointer-events-none absolute right-0 top-full z-30 mt-1.5 w-max max-w-[min(18rem,calc(100vw-5rem))] rounded-lg border border-line-strong bg-ink-800/95 px-2 py-1 text-left font-mono text-[11px] normal-case tracking-normal text-fg-2 shadow-xl backdrop-blur"
            : "sr-only",
        )}
      >
        {fyiTitle(agentName)}
      </span>
    </span>
  );
}

const MessageRow = memo(function MessageRow({ m, agent, agents, grill, onClearWith }: {
  m: Message;
  agent: Agent;
  agents: Agent[];
  grill: GrillSession | null;
  onClearWith: (next: string) => Promise<void>;
}) {
  if (m.kind === "clear") {
    return (
      <div className="flex items-center gap-3 py-2">
        <span className="h-px flex-1 bg-gradient-to-r from-transparent to-accent/30" />
        <span className="font-mono text-[10.5px] uppercase tracking-wider text-accent/80">
          /clear · context cleared, memory kept · {new Date(m.created_at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
        </span>
        <span className="h-px flex-1 bg-gradient-to-l from-transparent to-accent/30" />
      </div>
    );
  }
  if (m.kind === "compact") {
    const k = (n: unknown) => (typeof n === "number" ? `${Math.round(n / 1000)}k` : "?");
    return (
      <div className="flex items-center gap-3 py-2">
        <span className="h-px flex-1 bg-gradient-to-r from-transparent to-accent/30" />
        <span className="font-mono text-[10.5px] uppercase tracking-wider text-accent/80">
          {m.meta?.trigger === "auto" ? "auto-compacted" : "/compact"} · context {k(m.meta?.pre_tokens)} → {k(m.meta?.post_tokens)} tokens · {new Date(m.created_at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
        </span>
        <span className="h-px flex-1 bg-gradient-to-l from-transparent to-accent/30" />
      </div>
    );
  }
  if (m.kind === "run_start") {
    const label = m.meta?.trigger === "compact" ? "compacting context" : m.meta?.trigger === "schedule" ? "autonomous wake-up" : m.meta?.trigger === "manual" ? "manual wake-up" : m.meta?.trigger === "agent" ? "message from an agent" : "run";
    return (
      <div className="flex items-center gap-3 pt-3">
        <span className="h-px flex-1 bg-line" />
        <span className="font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
          {label} · {new Date(m.created_at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
        </span>
        <span className="h-px flex-1 bg-line" />
      </div>
    );
  }
  if (m.kind === "run_end") {
    if (m.meta?.stopped) return null;
    const secs = m.meta?.duration_ms ? (m.meta.duration_ms / 1000).toFixed(1) : null;
    return (
      <div className="flex justify-end gap-2 font-mono text-[10.5px] text-fg-3">
        {secs && <span>{secs}s</span>}
        {m.meta?.num_turns != null && <span>· {m.meta.num_turns} turns</span>}
        {m.meta?.cost_usd != null && <span>· ${Number(m.meta.cost_usd).toFixed(3)}</span>}
      </div>
    );
  }
  if (m.kind === "error") {
    return (
      <div className="flex items-start gap-2 rounded-xl border border-bad/25 bg-bad/[0.06] px-3 py-2 text-[13px] text-fg-2">
        <AlertCircle className="mt-0.5 size-4 shrink-0 text-bad" />
        <pre className="whitespace-pre-wrap font-sans">{m.content}</pre>
      </div>
    );
  }
  if (m.kind === "notice" && m.meta?.plan_task_id) return <PlanCard taskId={m.meta.plan_task_id} agents={agents} />;
  if (m.kind === "notice") {
    return (
      <div className="flex items-center justify-center gap-1.5 font-mono text-[11px] text-fg-3">
        <Info className="size-3" /> {m.content}
      </div>
    );
  }
  if (m.kind === "thinking") return <Thinking m={m} />;
  if (m.meta?.grill?.role === "round") return <GrillRound m={m} agentId={agent.id} session={grill} />;
  if (m.role === "user" && m.meta?.from_agent) {
    const from = findAgent(agents, m.meta.from_agent.id);
    return (
      <div className="flex justify-end gap-3 pt-2">
        <div className="flex max-w-[80%] flex-col items-end">
          <div className="relative mb-1 flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
            {isFyi(m) && (
              <FyiMark agentName={agent.name} />
            )}
            {from?.name ?? m.meta.from_agent.name} → {agent.name}
          </div>
          <div className="whitespace-pre-wrap rounded-2xl rounded-tr-md border border-line bg-white/[0.04] px-4 py-2.5 text-[14px]">{taskText(m)}</div>
          <div className="mt-1.5">
            <MessageTasks m={m} />
          </div>
        </div>
        {from && <Sticker avatar={from.avatar} lead={from.lead} size={26} className="mt-5" />}
      </div>
    );
  }
  if (m.role === "user") {
    const images: string[] = m.meta?.images ?? [];
    return (
      <div className="flex flex-col items-end gap-1.5 pt-2">
        {images.length > 0 && (
          <div className="flex max-w-[80%] flex-wrap justify-end gap-1.5">
            {images.map((f) => (
              <a key={f} href={api.uploadUrl(m.agent_id, f)} target="_blank" rel="noopener noreferrer">
                <img
                  src={api.uploadUrl(m.agent_id, f)}
                  alt="Attached image"
                  loading="lazy"
                  className="max-h-48 max-w-full rounded-xl border border-line object-contain"
                />
              </a>
            ))}
          </div>
        )}
        {m.content && (
          <div className="max-w-[80%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-white/[0.07] px-4 py-2.5 text-[14px]">{taskText(m)}</div>
        )}
        <MessageTasks m={m} />
        {m.meta?.mentions?.length > 0 && (
          <div className="font-mono text-[10.5px] text-fg-3">→ {m.meta.mentions_only ? "sent to" : "also sent to"} {m.meta.mentions.map((x: { name: string }) => x.name).join(", ")}</div>
        )}
        {m.meta?.mentioned_in && <div className="font-mono text-[10.5px] text-fg-3">mentioned in {m.meta.mentioned_in.name}'s chat</div>}
      </div>
    );
  }
  if (m.meta?.grill?.end_card) {
    return (
      <div className="space-y-2">
        <MessageRow m={{ ...m, meta: { ...m.meta, grill: { ...m.meta.grill, end_card: null } } }} agent={agent} agents={agents} grill={grill} onClearWith={onClearWith} />
        <EndCard agentId={m.agent_id} messageId={m.id} card={m.meta.grill.end_card} onClear={onClearWith} />
      </div>
    );
  }
  // A mentioned agent's reply, copied into this chat.
  const replier = m.meta?.mention_reply ? findAgent(agents, m.meta.from_agent?.id) : undefined;
  return (
    <div className="flex gap-3 pt-1">
      <Sticker avatar={(replier ?? agent).avatar} lead={(replier ?? agent).lead} size={26} className="mt-0.5" />
      <div className={cx("prose-chat min-w-0 flex-1", m.meta?.subagent && "border-l border-line pl-3 text-fg-2", replier && "border-l-2 border-accent/30 pl-3")}>
        {m.meta?.mention_reply && (
          <div className="mb-1 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">{replier?.name ?? m.meta.from_agent?.name} · answering your mention</div>
        )}
        <Markdown remarkPlugins={[remarkGfm]} components={MD_COMPONENTS}>{m.content}</Markdown>
      </div>
    </div>
  );
});

function SentToAgent({ use, result, agent, agents }: { use: Message; result?: Message; agent: Agent; agents: Agent[] }) {
  const to = findAgent(agents, use.meta?.input?.agent);
  return (
    <div className="flex gap-3 pt-1">
      <Sticker avatar={agent.avatar} lead={agent.lead} size={26} className="mt-5" />
      <div className="flex min-w-0 max-w-[80%] flex-col items-start">
        <div className={cx("mb-1 flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-fg-3", !result && "animate-pulse-soft")}>
          {agent.name} → {to && <Sticker avatar={to.avatar} lead={to.lead} size={16} />} {to?.name ?? use.meta?.input?.agent}
        </div>
        <div className="whitespace-pre-wrap rounded-2xl rounded-tl-md border border-line bg-white/[0.04] px-4 py-2.5 text-[14px]">{use.meta?.input?.text}</div>
      </div>
    </div>
  );
}

function Thinking({ m }: { m: Message }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="pl-9">
      <button className="flex items-center gap-1.5 font-mono text-[11.5px] text-fg-3 hover:text-fg-2" onClick={() => setOpen(!open)}>
        <Brain className="size-3.5" /> Reasoning
        <ChevronRight className={cx("size-3 transition", open && "rotate-90")} />
      </button>
      {open && (
        <div className="mt-1.5 whitespace-pre-wrap border-l border-line pl-3 text-[13px] italic leading-relaxed text-fg-3">{m.content}</div>
      )}
    </div>
  );
}

function ToolRow({ use, result }: { use: Message; result?: Message }) {
  const [open, setOpen] = useState(false);
  const name: string = use.meta?.name ?? "tool";
  const isErr = result?.meta?.is_error;
  const pending = !result;
  const shortName = name.startsWith("mcp__loft__") ? name.slice(15) : name.replace(/^mcp__/, "").replace("__", " › ");
  return (
    <div className="pl-9">
      <button
        onClick={() => setOpen(!open)}
        className={cx(
          "group flex w-full items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left transition",
          open ? "border-line-strong bg-ink-850" : "border-transparent hover:border-line hover:bg-ink-900",
        )}
      >
        <Wrench className={cx("size-3.5 shrink-0", pending ? "animate-pulse-soft text-accent" : isErr ? "text-bad" : "text-fg-3")} />
        <span className="shrink-0 font-mono text-[11.5px] text-fg-2">{shortName}</span>
        <span className="truncate text-[12.5px] text-fg-3">{use.content}</span>
        {use.meta?.subagent && <span className="chip ml-1 shrink-0 py-0 text-[10px]">subagent</span>}
        <ChevronRight className={cx("ml-auto size-3 shrink-0 text-fg-3 transition", open && "rotate-90")} />
      </button>
      {open && (
        <div className="mt-1 space-y-2 rounded-lg border border-line bg-ink-950 p-3">
          <div>
            <div className="eyebrow mb-1">Input</div>
            <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-all font-mono text-[11.5px] text-fg-2">
              {JSON.stringify(use.meta?.input, null, 2)}
            </pre>
          </div>
          {result && (
            <div>
              <div className={cx("eyebrow mb-1", isErr && "text-bad")}>{isErr ? "Error" : "Result"}</div>
              <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-all font-mono text-[11.5px] text-fg-2">{result.content || "(empty)"}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** A pasted image: shown at once from `preview`, sendable once uploaded (`file`). */
type Attachment = { key: string; preview: string; file?: string; error?: string };

function AttachmentTile({ a, onRemove }: { a: Attachment; onRemove: () => void }) {
  const [loaded, setLoaded] = useState(false);
  const busy = !a.file && !a.error;
  return (
    <div
      className={cx("group relative size-24 overflow-hidden rounded-xl border bg-ink-950", a.error ? "border-bad/50" : "border-line")}
      title={a.error}
    >
      <img
        src={a.preview}
        alt="Pasted image"
        onLoad={() => setLoaded(true)}
        className={cx("size-full object-contain transition duration-300", loaded ? "opacity-100" : "opacity-0", busy && "opacity-40 blur-[1px]")}
      />
      {busy && (
        <div className="absolute inset-0 grid place-items-center" role="status" aria-label="Uploading image">
          <Loader2 className="size-5 animate-spin text-fg-2" />
        </div>
      )}
      {a.error && (
        <div className="absolute inset-0 grid place-items-center bg-bad/10">
          <AlertCircle className="size-5 text-bad" />
        </div>
      )}
      <button
        onClick={onRemove}
        aria-label="Remove image"
        className="absolute right-1 top-1 grid size-5 place-items-center rounded-full bg-ink-950/80 text-fg-2 opacity-0 transition hover:text-fg focus-visible:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-100"
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

/** The API's limits: 3.75 MB raw (5 MB base64), and it scales anything bigger than 1568 px down anyway. */
const MAX_IMAGE_BYTES = 3_750_000;
const MAX_IMAGE_EDGE = 1568;
const MAX_IMAGES = 10;
const SENDABLE = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** The image as-is when the API takes it, else redrawn to fit: PNG keeps screenshots sharp, JPEG if that's still too big. */
async function prepareImage(f: File): Promise<Blob> {
  const bmp = await createImageBitmap(f).catch(() => {
    throw new Error("Couldn't read this image");
  });
  const scale = Math.min(1, MAX_IMAGE_EDGE / Math.max(bmp.width, bmp.height));
  if (scale === 1 && f.size <= MAX_IMAGE_BYTES && SENDABLE.includes(f.type)) return f;
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(bmp.width * scale);
  canvas.height = Math.round(bmp.height * scale);
  canvas.getContext("2d")!.drawImage(bmp, 0, 0, canvas.width, canvas.height);
  const encode = (type: string, q?: number) => new Promise<Blob | null>((r) => canvas.toBlob(r, type, q));
  const png = await encode("image/png");
  if (png && png.size <= MAX_IMAGE_BYTES) return png;
  const jpeg = await encode("image/jpeg", 0.85);
  if (jpeg && jpeg.size <= MAX_IMAGE_BYTES) return jpeg;
  throw new Error("Image is too large");
}
