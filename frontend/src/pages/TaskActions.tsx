import { useEffect, useRef, useState, type ReactNode } from "react";
import { useNavigate } from "react-router";
import { ArrowUp, Check, CloudUpload, Link2, Pencil, Play, RotateCcw, Trash2, UserRoundPlus, X } from "lucide-react";
import { api, type Agent, type Task } from "../lib/api";
import { cx } from "../lib/format";
import { isSpec } from "../lib/issueOrder";
import { useAutoGrow } from "../lib/autoGrow";
import { atQuery, insertMention, mentionedIn, mentionGroups } from "../lib/mentionPicker";
import { MentionMenu } from "../components/MentionMenu";

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

const FIELD = "w-full rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[13px] outline-none placeholder:text-fg-3";

/** Labels typed as a comma-separated list, as GitHub names them. */
export const parseLabels = (s: string) => s.split(",").map((l) => l.trim()).filter(Boolean);

/** The issue fields shared by New and Edit: description, labels and owner. */
function IssueFields({ description, setDescription, labels, setLabels, owner, setOwner }: {
  description: string; setDescription: (v: string) => void;
  labels: string; setLabels: (v: string) => void;
  owner: string; setOwner: (v: string) => void;
}) {
  return (
    <>
      <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={5} placeholder="Description, in Markdown (optional)" className={cx(FIELD, "resize-y font-mono text-[12.5px]")} aria-label="Description" />
      <div className="flex gap-2">
        <input value={labels} onChange={(e) => setLabels(e.target.value)} placeholder="Labels, comma-separated" className={FIELD} aria-label="Labels" />
        <input value={owner} onChange={(e) => setOwner(e.target.value)} placeholder="Owner (GitHub login)" className={cx(FIELD, "font-mono")} aria-label="Owner" spellCheck={false} autoCapitalize="off" />
      </div>
    </>
  );
}

/** Edit a task's title, description, labels and owner. */
function EditDialog({ task: t, projects, onClose }: { task: Task; projects: string[]; onClose: () => void }) {
  const [title, setTitle] = useState(t.title);
  const [description, setDescription] = useState(t.description);
  const [labels, setLabels] = useState(t.labels.join(", "));
  const [owner, setOwner] = useState(t.owner);
  const [project, setProject] = useState(t.project);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.updateTask(t.id, { title: title.trim(), description, labels: parseLabels(labels), owner: owner.trim() || null, project: project.trim() });
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title="Edit Loft issue" onClose={onClose}>
      <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" className={FIELD} aria-label="Title" />
      <IssueFields {...{ description, setDescription, labels, setLabels, owner, setOwner }} />
      {/* Mirrored issues stay on their repo. */}
      {!t.github && (
        <>
          <input list="edit-projects" value={project} onChange={(e) => setProject(e.target.value)} placeholder="Project (its GitHub repo is where Publish goes)" className={FIELD} aria-label="Project" />
          <datalist id="edit-projects">{projects.map((p) => <option key={p} value={p} />)}</datalist>
        </>
      )}
      {ACTIVE.includes(t.status) && <p className="text-[12px] text-fg-3">The agent already has the old description: message it if the work changes.</p>}
      <div className="flex justify-end gap-2">
        <button className="btn-ghost" onClick={onClose}>Cancel</button>
        <button className="btn-primary" disabled={!title.trim() || busy} onClick={save}>Save</button>
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}
    </Modal>
  );
}

/** Why an open Loft-only issue can't be published yet, or null when it can. `repo` is its project's GitHub repo. */
export function publishBlocked(t: { project: string }, repo: string | undefined): string | null {
  if (!t.project.trim()) return "Pick a project first";
  if (!repo) return "This project has no GitHub repo";
  return null;
}

/** On an open issue nobody is working on: pick an agent and hand it over, from the issue itself. */
export function HandOffCard({ task: t, agents }: { task: Task; agents: Agent[] }) {
  // The project's agents first: they're on its repo.
  const sorted = [...agents].sort((a, b) => Number(b.project.trim() === t.project) - Number(a.project.trim() === t.project) || a.name.localeCompare(b.name));
  const [agentId, setAgentId] = useState(sorted[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.runTask(t.id, agentId);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="rounded-2xl border border-accent/25 bg-accent/[0.05] p-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[14px]">Hand this issue to an agent</div>
          <div className="mt-0.5 text-[12.5px] text-fg-3">
            {t.github ? `It works from #${t.github.number}'s text and reports progress here; GitHub keeps the title, labels and state.` : "It gets the description and reports progress here."}
          </div>
        </div>
        <select value={agentId} onChange={(e) => setAgentId(e.target.value)} className="chip bg-ink-850 text-[12px]" aria-label="Agent to hand it to">
          {sorted.map((a) => <option key={a.id} value={a.id}>{a.name}{a.project.trim() === t.project ? "" : ` · ${a.project || "no project"}`}</option>)}
        </select>
        <button className="btn-primary" disabled={!agentId || busy} onClick={start}>
          <Play className="size-3 fill-current" /> Start
        </button>
      </div>
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </section>
  );
}

/** On an open Loft-only issue: what Publish to GitHub would create, and the button, or why it can't yet. */
export function PublishCard({ task: t, repo, onEdit }: { task: Task; repo?: string; onEdit: () => void }) {
  const why = publishBlocked(t, repo);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const publish = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.publishTask(t.id);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="rounded-2xl border border-line-strong bg-ink-850 p-4">
      <div className="flex flex-wrap items-start gap-3">
        <CloudUpload className="mt-0.5 size-4 shrink-0 text-fg-2" />
        <div className="min-w-0 flex-1 space-y-1">
          <div className="text-[14px]">Publish as a GitHub issue</div>
          {why ? (
            <div className="text-[12.5px] text-fg-3">
              {t.project ? `${why}.` : "It needs a project with a GitHub repo."}{" "}
              <button className="text-accent underline decoration-accent/30 underline-offset-2" onClick={onEdit}>
                {t.project ? "Change the project" : "Pick a project"}
              </button>
            </div>
          ) : (
            <>
              <div className="font-mono text-[11.5px] text-fg-2">
                {repo} · {t.labels.length ? t.labels.join(", ") : "no labels"} · {t.owner ? `owner ${t.owner}` : "no owner"}
              </div>
              <div className="text-[12.5px] text-fg-3">The issue stays in Loft and links to it. From then on its title, description and labels are edited on GitHub too; the agent, progress and thread stay in Loft.</div>
            </>
          )}
        </div>
        <button className="btn" disabled={busy || !!why} onClick={publish} title={why ?? `Create it on ${repo}`}>
          <CloudUpload className="size-3.5" /> Publish to GitHub
        </button>
      </div>
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </section>
  );
}

/** Buttons on a task's page: run, reassign, done, reopen, cancel, edit, copy link, delete. */
export function TaskActions({ task: t, agents, projects = [], edit = 0 }: { task: Task; agents: Agent[]; projects?: string[]; edit?: number }) {
  const navigate = useNavigate();
  const [dialog, setDialog] = useState<"run" | "cancel" | "delete" | "edit" | null>(null);
  // The Publish card's "Pick a project" opens Edit here.
  useEffect(() => {
    if (edit) setDialog("edit");
  }, [edit]);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const active = ACTIVE.includes(t.status);
  const spec = isSpec(t);
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
        {/* A spec is split into tickets; each ticket goes to an agent, never the spec as one task. */}
        {spec && <span className="font-mono text-[11px] text-fg-3">A spec: its tickets go to agents.</span>}
        {!spec && !active && !finished && agent && (
          <button className="btn-primary" disabled={busy} onClick={() => act(() => api.runTask(t.id))}>
            <Play className="size-3 fill-current" /> Run with {agent.name}
          </button>
        )}
        {!spec && !active && (
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
            <X className="size-3.5" /> Cancel issue
          </button>
        )}
        <button className="btn-ghost" onClick={() => setDialog("edit")} title="Edit title, description, labels and owner">
          <Pencil className="size-3.5" /> Edit
        </button>
        <button className="btn-ghost" onClick={copyLink} title="Copy a link to this issue">
          <Link2 className="size-3.5" /> {copied ? "Copied" : "Copy link"}
        </button>
        {!active && (
          <button className="btn-ghost text-fg-3 hover:text-bad" onClick={() => setDialog("delete")} title="Delete this issue">
            <Trash2 className="size-3.5" />
          </button>
        )}
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}

      {dialog === "run" && <RunDialog task={t} agents={agents} onClose={() => setDialog(null)} />}
      {dialog === "edit" && <EditDialog task={t} projects={projects} onClose={() => setDialog(null)} />}
      {dialog === "cancel" && (
        <Modal title="Cancel this issue?" onClose={() => setDialog(null)}>
          <p className="text-[13px] text-fg-2">
            It's marked cancelled.{" "}
            {active && agent ? `${agent.name} is told to stop working on it; you can also stop its current run, which may be doing other work too.` : "Nobody is working on it right now."}
          </p>
          <div className="flex flex-wrap justify-end gap-2">
            <button className="btn-ghost" onClick={() => setDialog(null)}>Keep it</button>
            {active && agent?.running && (
              <button className="btn" disabled={busy} onClick={() => act(() => api.cancelTask(t.id, true))}>Cancel and stop {agent.name}</button>
            )}
            <button className="btn-primary" disabled={busy} onClick={() => act(() => api.cancelTask(t.id, false))}>Cancel issue</button>
          </div>
          {error && <div className="text-[12px] text-bad">{error}</div>}
        </Modal>
      )}
      {dialog === "delete" && (
        <Modal title="Delete this issue?" onClose={() => setDialog(null)}>
          <p className="text-[13px] text-fg-2">It's removed with its thread. Subtasks stay, without a parent. This can't be undone.</p>
          <div className="flex justify-end gap-2">
            <button className="btn-ghost" onClick={() => setDialog(null)}>Keep it</button>
            <button className="btn-primary" disabled={busy} onClick={() => act(async () => { await api.deleteTask(t.id); navigate({ pathname: "/tasks", search: window.location.search }); })}>Delete</button>
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

/** Closed on GitHub while an agent works on it: the work wasn't stopped; the user decides. */
export function ClosedOnGithubCard({ task: t }: { task: Task }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const n = t.github?.number;
  return (
    <section className="rounded-2xl border border-warn/30 bg-warn/[0.06] p-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[14px] text-fg">#{n} was closed on GitHub</div>
          <div className="mt-0.5 text-[12.5px] text-fg-2">It's still being worked on here. Cancel it, or keep working and reopen #{n} on GitHub.</div>
        </div>
        <button className="btn" disabled={busy} onClick={() => act(() => api.cancelTask(t.id, false))}>
          <X className="size-3.5" /> Cancel issue
        </button>
        <button className="btn-primary" disabled={busy} onClick={() => act(() => api.keepWorking(t.id))}>
          <RotateCcw className="size-3.5" /> Keep working
        </button>
      </div>
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </section>
  );
}

/** The comment box under a task's thread. */
export function CommentBox({ task: t, agent, agents }: { task: Task; agent?: Agent; agents: Agent[] }) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  useAutoGrow(box, text, 2);
  const [send, setSend] = useState(true);
  const [github, setGithub] = useState(true);
  const [reopen, setReopen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const finished = FINISHED.includes(t.status);
  // "@" in the box (comment or note) offers agents to mention; Escape closes it until the text changes.
  const [menuIndex, setMenuIndex] = useState(0);
  const [closedAt, setClosedAt] = useState<string | null>(null);
  const query = atQuery(text);
  const groups = query === undefined || closedAt === text ? [] : mentionGroups(agents, t.project, query);
  const flat = groups.flatMap((g) => g.agents);
  const marks: Record<string, string> = { ...(t.created_by ? { [t.created_by]: "created it" } : {}), ...(t.agent_id ? { [t.agent_id]: "on this issue" } : {}) };
  const told = mentionedIn(text, agents);
  const pick = (a: Agent) => {
    setText((v) => insertMention(v, a));
    box.current?.focus();
  };
  useEffect(() => setMenuIndex(0), [query]);

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
      const posting = !!t.github && github;
      const r = await api.commentTask(t.id, v, { send: sending, github: posting, reopen: sending && finished && reopen });
      setText("");
      setReopen(false);
      setNote(
        r.task ? `Reopened; ${agent!.name} is on it${r.queued ? " after its current run" : ""}.`
        : r.delivered_to ? `Sent to ${agent!.name}${r.queued ? "; it reads it after its current run" : ""}.`
        : posting ? "Saved; it goes to GitHub once approved."
        : "Saved as a private note.",
      );
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const label = !agent || !send ? (t.github && github ? "Comment" : "Add note") : finished && reopen ? "Reopen and send" : `Send to ${agent.name}`;
  return (
    <div className="space-y-2">
      {flat.length > 0 && <MentionMenu groups={groups} index={Math.min(menuIndex, flat.length - 1)} marks={marks} onHover={setMenuIndex} onPick={pick} />}
      <div className="rounded-2xl border border-line-strong bg-ink-850 p-2 pl-3 transition focus-within:border-accent/40">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (flat.length > 0) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                setMenuIndex((i) => (i + (e.key === "ArrowDown" ? 1 : flat.length - 1)) % flat.length);
                return;
              }
              if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                pick(flat[Math.min(menuIndex, flat.length - 1)]);
                return;
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setClosedAt(text);
                return;
              }
            }
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              post();
            }
          }}
          ref={box}
          rows={2}
          placeholder={agent && send ? `Comment, or ask ${agent.name} about this issue… (@ to mention an agent)` : "A note on this issue… (@ to mention an agent)"}
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
            <label className="flex items-center gap-1.5 font-mono text-[11px] text-fg-2" title="Run it again with this comment as the instruction">
              <input type="checkbox" checked={reopen} onChange={(e) => setReopen(e.target.checked)} className="accent-[var(--color-accent)]" />
              Reopen and run again
            </label>
          )}
          {t.github && (
            <label className="flex items-center gap-1.5 font-mono text-[11px] text-fg-2" title="After approval, per the comment setting in Settings">
              <input type="checkbox" checked={github} onChange={(e) => setGithub(e.target.checked)} className="accent-[var(--color-accent)]" />
              Post on GitHub
            </label>
          )}
          {(!agent || !send) && !(t.github && github) && (
            <span className="font-mono text-[11px] text-fg-3">
              {told.length > 0 ? `Private: only ${told.map((a) => `@${a.name}`).join(", ")} ${told.length > 1 ? "are" : "is"} told.` : "Private: no agent is told."}
            </span>
          )}
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

/** "New Loft issue" from the list. */
export function NewTaskDialog({ agents, projects, onClose }: { agents: Agent[]; projects: string[]; onClose: () => void }) {
  const navigate = useNavigate();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [labels, setLabels] = useState("");
  const [owner, setOwner] = useState("");
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
      const t = await api.createTask({ title: title.trim(), description, labels: parseLabels(labels), owner: owner.trim(), agent_id: agentId || null, priority, project });
      if (agentId && start) await api.runTask(t.id);
      onClose();
      navigate({ pathname: `/tasks/${t.id}`, search: window.location.search });
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const field = FIELD;
  return (
    <Modal title="New Loft issue" onClose={onClose}>
      <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What needs doing" className={field} />
      <IssueFields {...{ description, setDescription, labels, setLabels, owner, setOwner }} />
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
