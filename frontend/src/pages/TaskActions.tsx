import { useEffect, useState, type ReactNode } from "react";
import { useNavigate } from "react-router";
import { ArrowUp, Check, Link2, Play, RotateCcw, Trash2, UserRoundPlus, X } from "lucide-react";
import { api, type Agent, type Task } from "../lib/api";
import { cx } from "../lib/format";

const ACTIVE = ["in_progress", "blocked", "waiting_approval"];
const FINISHED = ["done", "failed", "cancelled"];

function Modal({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={onClose}>
      <div className="panel w-full max-w-md space-y-4 bg-ink-900 p-5" onClick={(e) => e.stopPropagation()} role="dialog" aria-label={title}>
        <h3 className="text-[15px] font-medium">{title}</h3>
        {children}
      </div>
    </div>
  );
}

function AgentSelect({ agents, value, onChange, placeholder }: { agents: Agent[]; value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} className="w-full rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[13px]">
      <option value="">{placeholder}</option>
      {agents.map((a) => <option key={a.id} value={a.id}>{a.name}{a.title ? ` · ${a.title}` : ""}</option>)}
    </select>
  );
}

/** Buttons on a task's page: run, reassign, done, reopen, cancel, copy link, delete. */
export function TaskActions({ task: t, agents }: { task: Task; agents: Agent[] }) {
  const navigate = useNavigate();
  const [dialog, setDialog] = useState<"run" | "cancel" | "delete" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const active = ACTIVE.includes(t.status);
  const finished = FINISHED.includes(t.status);
  const agent = agents.find((a) => a.id === t.agent_id);

  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
      setDialog(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(`${window.location.origin}/tasks/${t.id}`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setError("Couldn't copy the link.");
    }
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2">
        {!active && !finished && agent && (
          <button className="btn-primary" disabled={busy} onClick={() => act(() => api.runTask(t.id))}>
            <Play className="size-3 fill-current" /> Run with {agent.name}
          </button>
        )}
        {!active && (
          <button className="btn" disabled={busy} onClick={() => setDialog("run")}>
            <UserRoundPlus className="size-3.5" /> {agent ? (finished ? "Run again with…" : "Run with another agent…") : "Run with an agent…"}
          </button>
        )}
        {!active && t.status !== "done" && t.status !== "cancelled" && (
          <button className="btn" disabled={busy} onClick={() => act(() => api.updateTask(t.id, { status: "done" }))}>
            <Check className="size-3.5" /> Mark done
          </button>
        )}
        {finished && (
          <button className="btn" disabled={busy} onClick={() => act(() => api.updateTask(t.id, { status: "inbox" }))}>
            <RotateCcw className="size-3.5" /> Reopen
          </button>
        )}
        {!finished && (
          <button className="btn" disabled={busy} onClick={() => setDialog("cancel")}>
            <X className="size-3.5" /> Cancel task
          </button>
        )}
        <button className="btn-ghost" onClick={copyLink} title="Copy a link to this task">
          <Link2 className="size-3.5" /> {copied ? "Copied" : "Copy link"}
        </button>
        {!active && (
          <button className="btn-ghost text-fg-3 hover:text-bad" onClick={() => setDialog("delete")} title="Delete this task">
            <Trash2 className="size-3.5" />
          </button>
        )}
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}

      {dialog === "run" && <RunDialog task={t} agents={agents} onClose={() => setDialog(null)} />}
      {dialog === "cancel" && (
        <Modal title="Cancel this task?" onClose={() => setDialog(null)}>
          <p className="text-[13px] text-fg-2">
            It's marked cancelled.{" "}
            {active && agent ? `${agent.name} is told to stop working on it; you can also stop its current run, which may be doing other work too.` : "Nobody is working on it right now."}
          </p>
          <div className="flex flex-wrap justify-end gap-2">
            <button className="btn-ghost" onClick={() => setDialog(null)}>Keep it</button>
            {active && agent?.running && (
              <button className="btn" disabled={busy} onClick={() => act(() => api.cancelTask(t.id, true))}>Cancel and stop {agent.name}</button>
            )}
            <button className="btn-primary" disabled={busy} onClick={() => act(() => api.cancelTask(t.id, false))}>Cancel task</button>
          </div>
          {error && <div className="text-[12px] text-bad">{error}</div>}
        </Modal>
      )}
      {dialog === "delete" && (
        <Modal title="Delete this task?" onClose={() => setDialog(null)}>
          <p className="text-[13px] text-fg-2">It's removed with its thread. Subtasks stay, without a parent. This can't be undone.</p>
          <div className="flex justify-end gap-2">
            <button className="btn-ghost" onClick={() => setDialog(null)}>Keep it</button>
            <button className="btn-primary" disabled={busy} onClick={() => act(async () => { await api.deleteTask(t.id); navigate("/tasks"); })}>Delete</button>
          </div>
          {error && <div className="text-[12px] text-bad">{error}</div>}
        </Modal>
      )}
    </div>
  );
}

function RunDialog({ task: t, agents, onClose }: { task: Task; agents: Agent[]; onClose: () => void }) {
  const [agentId, setAgentId] = useState(t.agent_id ?? "");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      // A finished task goes back to ready first: the server only runs open tasks.
      if (FINISHED.includes(t.status)) await api.updateTask(t.id, { status: "ready" });
      await api.runTask(t.id, agentId, note);
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title={`Run “${t.title}”`} onClose={onClose}>
      <AgentSelect agents={agents} value={agentId} onChange={setAgentId} placeholder="Pick an agent" />
      <textarea
        value={note}
        onChange={(e) => setNote(e.target.value)}
        rows={3}
        placeholder="Anything to add for the agent (optional)"
        className="w-full resize-none rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[13px] outline-none placeholder:text-fg-3"
      />
      <div className="flex justify-end gap-2">
        <button className="btn-ghost" onClick={onClose}>Cancel</button>
        <button className="btn-primary" disabled={!agentId || busy} onClick={run}><Play className="size-3 fill-current" /> Run</button>
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}
    </Modal>
  );
}

/** The comment box under a task's thread. */
export function CommentBox({ task: t, agent }: { task: Task; agent?: Agent }) {
  const [text, setText] = useState("");
  const [send, setSend] = useState(true);
  const [reopen, setReopen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const finished = FINISHED.includes(t.status);

  useEffect(() => {
    setText("");
    setReopen(false);
    setError(null);
  }, [t.id]);
  useEffect(() => {
    if (!note) return;
    const id = setTimeout(() => setNote(null), 4000);
    return () => clearTimeout(id);
  }, [note]);

  const post = async () => {
    const v = text.trim();
    if (!v) return;
    setBusy(true);
    setError(null);
    try {
      const sending = !!agent && send;
      const r = await api.commentTask(t.id, v, { send: sending, reopen: sending && finished && reopen });
      setText("");
      setReopen(false);
      setNote(
        r.task ? `Reopened; ${agent!.name} is on it${r.queued ? " after its current run" : ""}.`
        : r.delivered_to ? `Sent to ${agent!.name}${r.queued ? "; it reads it after its current run" : ""}.`
        : "Saved as a private note.",
      );
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const label = !agent || !send ? "Add note" : finished && reopen ? "Reopen and send" : `Send to ${agent.name}`;
  return (
    <div className="space-y-2">
      <div className="rounded-2xl border border-line-strong bg-ink-850 p-2 pl-3 transition focus-within:border-accent/40">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              post();
            }
          }}
          rows={Math.min(8, Math.max(2, text.split("\n").length))}
          placeholder={agent && send ? `Comment, or ask ${agent.name} about this task…` : "A note on this task…"}
          className="w-full resize-none bg-transparent py-1 text-[13.5px] outline-none placeholder:text-fg-3"
        />
        <div className="flex flex-wrap items-center gap-3">
          {agent && (
            <label className="flex items-center gap-1.5 font-mono text-[11px] text-fg-2">
              <input type="checkbox" checked={send} onChange={(e) => setSend(e.target.checked)} className="accent-[var(--color-accent)]" />
              Send to {agent.name}
            </label>
          )}
          {agent && send && finished && (
            <label className="flex items-center gap-1.5 font-mono text-[11px] text-fg-2" title="Run the task again with this comment as the instruction">
              <input type="checkbox" checked={reopen} onChange={(e) => setReopen(e.target.checked)} className="accent-[var(--color-accent)]" />
              Reopen and run again
            </label>
          )}
          {(!agent || !send) && <span className="font-mono text-[11px] text-fg-3">Private: no agent is told.</span>}
          <button className={cx("ml-auto", agent && send ? "btn-primary" : "btn")} disabled={!text.trim() || busy} onClick={post}>
            <ArrowUp className="size-3.5" /> {label}
          </button>
        </div>
      </div>
      <div className="flex justify-between font-mono text-[10.5px] text-fg-3">
        <span>⌘↵ to send</span>
        {note && <span className="text-good">✓ {note}</span>}
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}
    </div>
  );
}

/** "New task" from the list. */
export function NewTaskDialog({ agents, projects, onClose }: { agents: Agent[]; projects: string[]; onClose: () => void }) {
  const navigate = useNavigate();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [agentId, setAgentId] = useState("");
  const [priority, setPriority] = useState<Task["priority"]>("normal");
  const [project, setProject] = useState("");
  const [start, setStart] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const t = await api.createTask({ title: title.trim(), description, agent_id: agentId || null, priority, project });
      if (agentId && start) await api.runTask(t.id);
      onClose();
      navigate(`/tasks/${t.id}`);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const field = "w-full rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[13px] outline-none placeholder:text-fg-3";
  return (
    <Modal title="New task" onClose={onClose}>
      <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What needs doing" className={field} />
      <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={4} placeholder="Details (optional)" className={cx(field, "resize-none")} />
      <AgentSelect agents={agents} value={agentId} onChange={setAgentId} placeholder="No agent (a to-do for you)" />
      <div className="flex gap-2">
        <select value={priority} onChange={(e) => setPriority(e.target.value as Task["priority"])} className={field} aria-label="Priority">
          {["low", "normal", "high", "urgent"].map((p) => <option key={p} value={p}>{p} priority</option>)}
        </select>
        <input list="task-projects" value={project} onChange={(e) => setProject(e.target.value)} placeholder="Project" className={field} />
        <datalist id="task-projects">{projects.map((p) => <option key={p} value={p} />)}</datalist>
      </div>
      {agentId && (
        <label className="flex items-center gap-1.5 font-mono text-[11px] text-fg-2">
          <input type="checkbox" checked={start} onChange={(e) => setStart(e.target.checked)} className="accent-[var(--color-accent)]" />
          Start now
        </label>
      )}
      <div className="flex justify-end gap-2">
        <button className="btn-ghost" onClick={onClose}>Cancel</button>
        <button className="btn-primary" disabled={!title.trim() || busy} onClick={create}>Create</button>
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}
    </Modal>
  );
}
