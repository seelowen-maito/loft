import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Brain, Code2, Copy, Dices, File, Folder, FolderGit2, Plus, Save, Server, Shield, SlidersHorizontal, Trash2, X,
} from "lucide-react";
import { api, type Agent, type FileEntry, type Memory, type PermissionRequest } from "../../lib/api";
import { useServerEvents, type ServerEvent } from "../../lib/events";
import { cx, timeAgo } from "../../lib/format";
import { Sticker } from "../../components/Sticker";
import { PermissionCard } from "./PermissionCard";
import { ProjectSelect } from "./ProjectSelect";
import { QuietHours } from "./QuietHours";

type Tab = "config" | "permissions" | "memory" | "workspace";

export const MODELS = [
  { value: "sonnet", label: "Sonnet · balanced" },
  { value: "opus", label: "Opus · most capable" },
  { value: "fable", label: "Fable" },
  { value: "haiku", label: "Haiku · fastest" },
];

export const INTERVALS = [
  { value: 0, label: "Manual only" },
  { value: 15, label: "Every 15 min" },
  { value: 60, label: "Hourly" },
  { value: 360, label: "Every 6 hours" },
  { value: 1440, label: "Daily" },
];

const PERMISSION_MODES = [
  { value: "auto", label: "Auto", hint: "Recommended. A safety classifier approves routine actions; risky ones are blocked or asked in chat." },
  { value: "default", label: "Ask", hint: "Anything outside the allowed list prompts you in chat." },
  { value: "acceptEdits", label: "Accept edits", hint: "File edits in the workspace are auto-approved." },
  { value: "plan", label: "Plan only", hint: "Read-only: the agent can explore and plan, not change anything." },
  { value: "bypassPermissions", label: "Bypass", hint: "Everything is allowed. Only for agents you fully trust." },
];

const TOOL_PRESETS = [
  "Bash", "Bash(npm *)", "Bash(cargo *)", "Bash(gh *)", "Bash(python3 *)", "WebFetch", "WebSearch", "Task", "NotebookEdit",
];

const MCP_TEMPLATES: Record<string, unknown> = {
  github: { type: "http", url: "https://api.githubcopilot.com/mcp/", headers: { Authorization: "Bearer ${GITHUB_TOKEN}" } },
  playwright: { command: "npx", args: ["@playwright/mcp@latest"] },
  filesystem: { command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "~/projects"] },
};

export function Inspector({ agent, projects, perms, onCleared }: { agent: Agent; projects: string[]; perms: PermissionRequest[]; onCleared: () => void }) {
  const [tab, setTab] = useState<Tab>("config");
  const pending = perms.filter((p) => p.status === "pending").length;
  const tabs: { id: Tab; label: string; icon: React.ReactNode; badge?: number }[] = [
    { id: "config", label: "Config", icon: <SlidersHorizontal className="size-3.5" /> },
    { id: "permissions", label: "Access", icon: <Shield className="size-3.5" />, badge: pending },
    { id: "memory", label: "Memory", icon: <Brain className="size-3.5" /> },
    { id: "workspace", label: "Files", icon: <Folder className="size-3.5" /> },
  ];
  return (
    <div className="flex h-full flex-col">
      <div className="flex gap-1 border-b border-line px-3 py-2.5">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={cx(
              "relative flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12.5px] transition",
              tab === t.id ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg",
            )}
          >
            {t.icon} {t.label}
            {!!t.badge && <span className="ml-0.5 rounded-full bg-warn px-1.5 font-mono text-[10px] text-ink-950">{t.badge}</span>}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {tab === "config" && <ConfigTab key={agent.id} agent={agent} projects={projects} onCleared={onCleared} />}
        {tab === "permissions" && <AccessTab agent={agent} perms={perms} />}
        {tab === "memory" && <MemoryTab agent={agent} />}
        {tab === "workspace" && <FilesTab agent={agent} />}
      </div>
    </div>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block space-y-1.5">
      <span className="eyebrow">{label}</span>
      {children}
      {hint && <span className="block text-[11.5px] text-fg-3">{hint}</span>}
    </label>
  );
}

type Draft = Pick<Agent, "name" | "title" | "project" | "personality" | "prompt" | "model" | "effort" | "subagent_model" | "max_subagents" | "planning" | "lead" | "role" | "interval_minutes" | "quiet_hours" | "permission_mode">;

function ConfigTab({ agent, projects, onCleared }: { agent: Agent; projects: string[]; onCleared: () => void }) {
  const initial = useMemo<Draft>(
    () => ({
      name: agent.name, title: agent.title, project: agent.project ?? "", personality: agent.personality, prompt: agent.prompt,
      model: agent.model, effort: agent.effort, subagent_model: agent.subagent_model ?? "", max_subagents: agent.max_subagents ?? null, planning: agent.planning ?? false, lead: agent.lead ?? false, role: agent.role ?? "", interval_minutes: agent.interval_minutes, quiet_hours: agent.quiet_hours ?? [], permission_mode: agent.permission_mode,
    }),
    [agent],
  );
  const [draft, setDraft] = useState<Draft>(initial);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const dirty = (Object.keys(initial) as (keyof Draft)[]).some((k) => JSON.stringify(initial[k]) !== JSON.stringify(draft[k]));
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => setDraft((d) => ({ ...d, [k]: v }));
  const customInterval = !INTERVALS.some((i) => i.value === draft.interval_minutes);

  const save = async () => {
    setSaving(true);
    setErr(null);
    setNotice(null);
    try {
      const saved = await api.updateAgent(agent.id, draft);
      setNotice(saved.notice ?? null);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-6 p-5">
      <div className="flex items-center gap-4">
        <Sticker avatar={agent.avatar} lead={agent.lead} size={88} />
        <div className="space-y-2">
          <button className="btn" onClick={() => api.randomizeAvatar(agent.id)}>
            <Dices className="size-3.5" /> Randomize
          </button>
          <p className="text-[11.5px] leading-snug text-fg-3">Or ask {agent.name} in chat to redraw itself. It draws its own sticker.</p>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Name">
          <input className="input" value={draft.name} onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field label="Role">
          <input className="input" value={draft.title} placeholder="e.g. Release captain" onChange={(e) => set("title", e.target.value)} />
        </Field>
      </div>

      <Field label="Project" hint="A git repo, or any name or tag. Agents that share one are grouped together in the list.">
        <ProjectSelect value={draft.project} projects={projects} onChange={(v) => set("project", v)} />
      </Field>

      <Field label="Personality" hint="Tone, voice, quirks. Injected into its system prompt.">
        <textarea className="input min-h-[80px] resize-y" value={draft.personality} onChange={(e) => set("personality", e.target.value)} />
      </Field>

      <Field label="Standing instructions" hint="What this agent owns. Autonomous wake-ups follow these.">
        <textarea className="input min-h-[120px] resize-y" value={draft.prompt} onChange={(e) => set("prompt", e.target.value)} />
      </Field>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Model">
          <select className="input" value={draft.model} onChange={(e) => set("model", e.target.value)}>
            {MODELS.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
            {!MODELS.some((m) => m.value === draft.model) && <option value={draft.model}>{draft.model}</option>}
          </select>
        </Field>
        <Field label="Effort">
          <select className="input" value={draft.effort} onChange={(e) => set("effort", e.target.value)}>
            <option value="">Default</option>
            {["low", "medium", "high", "xhigh", "max"].map((e) => <option key={e} value={e}>{e}</option>)}
          </select>
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Subagent model" hint="Every subagent it spawns, Explore included. A cheaper model here keeps searches and side jobs cheap.">
          <select className="input" value={draft.subagent_model} onChange={(e) => set("subagent_model", e.target.value)}>
            <option value="">Claude Code default</option>
            {MODELS.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
          </select>
        </Field>
        <Field label="Subagents at once" hint="How many can run in parallel. Off removes the Agent tool.">
          <select
            className="input"
            value={draft.max_subagents ?? ""}
            onChange={(e) => set("max_subagents", e.target.value === "" ? null : Number(e.target.value))}
          >
            <option value="">Default (20)</option>
            <option value={0}>Off</option>
            {[1, 2, 3, 5, 10].map((n) => <option key={n} value={n}>{n}</option>)}
          </select>
        </Field>
      </div>

      <Field label="Planning" hint="Lets it grill you on an idea, in rounds of questions that each come with a recommendation. Start one with /grill or “grill me”. Without it, that's an ordinary message.">
        <label className="flex items-center gap-2 text-[12.5px] text-fg">
          <input type="checkbox" checked={draft.planning || draft.lead} disabled={draft.lead} onChange={(e) => set("planning", e.target.checked)} className="accent-[var(--color-accent)]" />
          Can plan with you{draft.lead && <span className="text-fg-3">(a Lead always can)</span>}
        </label>
      </Field>

      <Field label="Lead" hint={draft.project.trim() ? `Leads the agents in ${draft.project.trim()} and wears the captain hat. One per project: marking it here moves the mark from any other.` : "Only an agent with a project can lead: set its project first."}>
        <label className={cx("flex items-center gap-2 text-[12.5px]", draft.project.trim() || draft.lead ? "text-fg" : "text-fg-3")}>
          <input
            type="checkbox"
            checked={draft.lead}
            disabled={!draft.project.trim() && !draft.lead}
            onChange={(e) => setDraft((d) => ({ ...d, lead: e.target.checked, planning: e.target.checked || d.planning }))}
            className="accent-[var(--color-accent)]"
          />
          Leads its team
        </label>
      </Field>

      <Field label="Team role" hint="What it does in its project's team. When planning ends, Loft offers to set up the team if no agent there is a Dev or a reviewer.">
        <select className="input" value={draft.role} onChange={(e) => set("role", e.target.value)}>
          <option value="">None</option>
          <option value="dev">Dev: builds the tickets</option>
          <option value="reviewer">Reviewer: code review, QA or security</option>
        </select>
      </Field>

      <Field label="Autonomy" hint="How often it wakes up on its own to work on its standing instructions.">
        <div className="flex gap-2">
          <select
            className="input"
            value={customInterval ? "custom" : draft.interval_minutes}
            onChange={(e) => set("interval_minutes", e.target.value === "custom" ? 30 : Number(e.target.value))}
          >
            {INTERVALS.map((i) => <option key={i.value} value={i.value}>{i.label}</option>)}
            <option value="custom">Custom…</option>
          </select>
          {customInterval && (
            <div className="relative w-32 shrink-0">
              <input type="number" min={1} className="input pr-10" value={draft.interval_minutes} onChange={(e) => set("interval_minutes", Math.max(1, Number(e.target.value)))} />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 font-mono text-[11px] text-fg-3">min</span>
            </div>
          )}
        </div>
      </Field>
      {draft.interval_minutes > 0 && (
        <QuietHours value={draft.quiet_hours} onChange={(v) => set("quiet_hours", v)} interval={draft.interval_minutes} />
      )}

      <Field label="Permission mode">
        <div className="grid grid-cols-2 gap-2">
          {PERMISSION_MODES.map((m) => (
            <button
              key={m.value}
              type="button"
              onClick={() => set("permission_mode", m.value)}
              className={cx(
                "rounded-xl border px-3 py-2 text-left transition",
                m.value === "auto" && "col-span-2",
                draft.permission_mode === m.value ? "border-accent/50 bg-accent/[0.06]" : "border-line hover:border-line-strong",
              )}
            >
              <div className={cx("text-[12.5px]", draft.permission_mode === m.value ? "text-accent" : "text-fg")}>{m.label}</div>
              <div className="mt-0.5 text-[11px] leading-snug text-fg-3">{m.hint}</div>
            </button>
          ))}
        </div>
      </Field>

      <div className="sticky bottom-0 -mx-5 flex items-center gap-2 border-t border-line bg-ink-900/95 px-5 py-3 backdrop-blur">
        <button className="btn-primary" disabled={!dirty || saving} onClick={save}>
          <Save className="size-3.5" /> {saving ? "Saving…" : "Save changes"}
        </button>
        {dirty && <button className="btn-ghost" onClick={() => setDraft(initial)}>Discard</button>}
        {err && <span className="text-[12px] text-bad">{err}</span>}
        {notice && !err && <span className="text-[12px] text-fg-2">{notice}</span>}
      </div>

      <DangerZone agent={agent} onCleared={onCleared} />
    </div>
  );
}

function DangerZone({ agent, onCleared }: { agent: Agent; onCleared: () => void }) {
  const [confirm, setConfirm] = useState(false);
  const [purge, setPurge] = useState(false);
  return (
    <div className="space-y-3 rounded-2xl border border-bad/20 p-4">
      <div className="eyebrow text-bad/80">Danger zone</div>
      <div className="flex flex-wrap gap-2">
        <button className="btn" onClick={() => api.clearMessages(agent.id).then(onCleared)}>Clear chat history</button>
        {agent.builtin ? (
          <span className="self-center text-[12.5px] text-fg-3">{agent.name} comes with Loft and can't be deleted.</span>
        ) : !confirm ? (
          <button className="btn border-bad/30 text-bad" onClick={() => setConfirm(true)}>
            <Trash2 className="size-3.5" /> Delete agent
          </button>
        ) : (
          <div className="w-full space-y-2 rounded-xl bg-bad/[0.06] p-3">
            <label className="flex items-center gap-2 text-[12.5px] text-fg-2">
              <input type="checkbox" checked={purge} onChange={(e) => setPurge(e.target.checked)} />
              Also delete its workspace folder
            </label>
            <div className="flex gap-2">
              <button className="btn border-bad/40 bg-bad/20 text-fg" onClick={() => api.deleteAgent(agent.id, purge)}>Delete {agent.name}</button>
              <button className="btn-ghost" onClick={() => setConfirm(false)}>Cancel</button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function ToolList({ title, hint, items, onChange, presets, tone }: {
  title: string; hint: string; items: string[]; onChange: (v: string[]) => void; presets?: string[]; tone: "good" | "bad";
}) {
  const [val, setVal] = useState("");
  const add = (v: string) => {
    const t = v.trim();
    if (t && !items.includes(t)) onChange([...items, t]);
    setVal("");
  };
  return (
    <div className="space-y-2">
      <div>
        <div className="eyebrow">{title}</div>
        <div className="mt-0.5 text-[11.5px] text-fg-3">{hint}</div>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {items.length === 0 && <span className="text-[12px] text-fg-3">None</span>}
        {items.map((t) => (
          <span key={t} className={cx("chip gap-1.5 bg-ink-850 py-1 pr-1", tone === "good" ? "border-good/20" : "border-bad/25")}>
            <span className={tone === "good" ? "text-fg" : "text-bad"}>{t}</span>
            <button className="rounded-full p-0.5 text-fg-3 hover:bg-white/10 hover:text-fg" onClick={() => onChange(items.filter((x) => x !== t))} aria-label={`Remove ${t}`}>
              <X className="size-3" />
            </button>
          </span>
        ))}
      </div>
      <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); add(val); }}>
        <input className="input py-1.5 font-mono text-[12px]" placeholder="e.g. Bash(npm test *) or mcp__github" value={val} onChange={(e) => setVal(e.target.value)} />
        <button className="btn shrink-0" type="submit"><Plus className="size-3.5" /></button>
      </form>
      {presets && (
        <div className="flex flex-wrap gap-1">
          {presets.filter((p) => !items.includes(p)).map((p) => (
            <button key={p} className="rounded-full border border-dashed border-line-strong px-2 py-0.5 font-mono text-[10.5px] text-fg-3 hover:border-accent/40 hover:text-accent" onClick={() => add(p)}>
              + {p}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function AccessTab({ agent, perms }: { agent: Agent; perms: PermissionRequest[] }) {
  const save = (patch: Partial<Agent>) => api.updateAgent(agent.id, patch);
  const [mcpText, setMcpText] = useState(() => JSON.stringify(agent.mcp_servers ?? {}, null, 2));
  const [mcpErr, setMcpErr] = useState<string | null>(null);
  const [mcpSaved, setMcpSaved] = useState(true);
  useEffect(() => {
    setMcpText(JSON.stringify(agent.mcp_servers ?? {}, null, 2));
    setMcpSaved(true);
  }, [agent.id, agent.mcp_servers]);

  const saveMcp = async () => {
    try {
      const parsed = JSON.parse(mcpText);
      await save({ mcp_servers: parsed });
      setMcpErr(null);
      setMcpSaved(true);
    } catch (e) {
      setMcpErr((e as Error).message);
    }
  };
  const addTemplate = (name: string) => {
    try {
      const cur = JSON.parse(mcpText || "{}");
      cur[name] = MCP_TEMPLATES[name];
      setMcpText(JSON.stringify(cur, null, 2));
      setMcpSaved(false);
    } catch {
      setMcpErr("Fix the JSON before adding a template.");
    }
  };
  const servers = Object.keys(agent.mcp_servers ?? {});
  const pending = perms.filter((p) => p.status === "pending");
  const history = perms.filter((p) => p.status !== "pending").slice(0, 20);

  return (
    <div className="space-y-7 p-5">
      {pending.length > 0 && (
        <div className="space-y-2">
          <div className="eyebrow text-warn">Waiting on you</div>
          {pending.map((p) => <PermissionCard key={p.id} req={p} />)}
        </div>
      )}

      <ToolList
        title="Allowed tools"
        hint="Used without asking. Anything else is asked in chat, and “Always allow” adds it here."
        items={agent.allowed_tools}
        onChange={(v) => save({ allowed_tools: v })}
        presets={TOOL_PRESETS}
        tone="good"
      />
      <ToolList
        title="Denied tools"
        hint="Always blocked, even when bypassing permissions."
        items={agent.disallowed_tools}
        onChange={(v) => save({ disallowed_tools: v })}
        tone="bad"
      />

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <div>
            <div className="eyebrow flex items-center gap-1.5"><Server className="size-3" /> MCP servers</div>
            <div className="mt-0.5 text-[11.5px] text-fg-3">Claude Code <code className="font-mono">mcpServers</code> format. Grant access with a rule like <code className="font-mono">mcp__name</code>.</div>
          </div>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <span className="chip border-accent/25 text-accent">loft · built-in</span>
          {servers.map((s) => (
            <span key={s} className={cx("chip", agent.allowed_tools.some((t) => t === `mcp__${s}` || t.startsWith(`mcp__${s}__`)) ? "text-good" : "text-fg-3")}>
              {s} {agent.allowed_tools.some((t) => t === `mcp__${s}` || t.startsWith(`mcp__${s}__`)) ? "· allowed" : "· asks"}
            </span>
          ))}
        </div>
        <textarea
          className="input min-h-[160px] resize-y font-mono text-[12px] leading-relaxed"
          spellCheck={false}
          value={mcpText}
          onChange={(e) => { setMcpText(e.target.value); setMcpSaved(false); }}
        />
        <div className="flex flex-wrap items-center gap-2">
          <button className="btn-primary" onClick={saveMcp} disabled={mcpSaved}><Save className="size-3.5" /> Save servers</button>
          <span className="text-[11px] text-fg-3">Templates:</span>
          {Object.keys(MCP_TEMPLATES).map((t) => (
            <button key={t} className="rounded-full border border-dashed border-line-strong px-2 py-0.5 font-mono text-[10.5px] text-fg-3 hover:text-accent" onClick={() => addTemplate(t)}>+ {t}</button>
          ))}
        </div>
        {mcpErr && <div className="text-[12px] text-bad">{mcpErr}</div>}
      </div>

      {history.length > 0 && (
        <div className="space-y-2">
          <div className="eyebrow">History</div>
          {history.map((p) => <PermissionCard key={p.id} req={p} compact />)}
        </div>
      )}
    </div>
  );
}

function MemoryTab({ agent }: { agent: Agent }) {
  const [mems, setMems] = useState<Memory[] | null>(null);
  const [text, setText] = useState("");
  const [q, setQ] = useState("");
  const load = useCallback(() => { api.memories(agent.id).then(setMems); }, [agent.id]);
  useEffect(load, [load]);
  useServerEvents(useCallback((e: ServerEvent) => { if (e.type === "memory" && e.agent_id === agent.id) load(); }, [agent.id, load]));
  const visible = (mems ?? []).filter((m) => !q || (m.content + m.tags).toLowerCase().includes(q.toLowerCase())).reverse();
  return (
    <div className="space-y-4 p-5">
      <p className="text-[12.5px] leading-relaxed text-fg-2">
        Long-term memory is private to {agent.name}. It survives new sessions and is loaded into every run. The agent writes here with its <code className="font-mono text-[11.5px]">remember</code> tool.
      </p>
      <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); if (text.trim()) api.addMemory(agent.id, text).then(() => setText("")); }}>
        <input className="input" placeholder="Teach it something…" value={text} onChange={(e) => setText(e.target.value)} />
        <button className="btn shrink-0" type="submit"><Plus className="size-3.5" /></button>
      </form>
      {(mems?.length ?? 0) > 6 && <input className="input py-1.5" placeholder="Search memories" value={q} onChange={(e) => setQ(e.target.value)} />}
      {!mems && <div className="skeleton h-24 rounded-xl" />}
      {mems?.length === 0 && <div className="py-6 text-center text-[12.5px] text-fg-3">Nothing remembered yet.</div>}
      <ul className="space-y-2">
        {visible.map((m) => (
          <li key={m.id} className="group rounded-xl border border-line bg-ink-850/60 px-3 py-2.5">
            <div className="text-[13px] leading-snug">{m.content}</div>
            <div className="mt-1.5 flex items-center gap-2 font-mono text-[10.5px] text-fg-3">
              <span>#{m.id}</span>
              <span>{timeAgo(m.created_at)}</span>
              {m.tags && m.tags.split(",").map((t) => <span key={t} className="chip py-0 text-[10px]">{t.trim()}</span>)}
              <button className="ml-auto opacity-0 transition hover:text-bad group-hover:opacity-100" onClick={() => api.deleteMemory(agent.id, m.id)} aria-label="Forget">
                <Trash2 className="size-3.5" />
              </button>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function FilesTab({ agent }: { agent: Agent }) {
  const [files, setFiles] = useState<FileEntry[] | null>(null);
  const [editorErr, setEditorErr] = useState<string | null>(null);
  const load = useCallback(() => { api.files(agent.id).then(setFiles); }, [agent.id]);
  useEffect(load, [load]);
  // refresh when a run finishes
  useEffect(() => { if (!agent.running) load(); }, [agent.running, load]);
  const fmtSize = (n: number) => (n > 1e6 ? `${(n / 1e6).toFixed(1)}M` : n > 1e3 ? `${(n / 1e3).toFixed(1)}k` : `${n}b`);
  return (
    <div className="space-y-4 p-5">
      <div>
        <div className="eyebrow mb-1.5">Workspace</div>
        <button
          className="group flex w-full items-center gap-2 rounded-xl border border-line bg-ink-950 px-3 py-2 text-left font-mono text-[11.5px] text-fg-2"
          onClick={() => agent.workspace && navigator.clipboard.writeText(agent.workspace)}
          title="Copy path"
        >
          <span className="truncate">{agent.workspace}</span>
          <Copy className="ml-auto size-3.5 shrink-0 text-fg-3 group-hover:text-fg" />
        </button>
        <button
          className="btn mt-2"
          onClick={() => api.openEditor(agent.id).then(() => setEditorErr(null), (e) => setEditorErr((e as Error).message))}
        >
          <Code2 className="size-3.5" /> Open in VS Code
        </button>
        {editorErr && <div className="mt-1.5 text-[12px] text-bad">{editorErr}</div>}
      </div>
      {!files && <div className="skeleton h-40 rounded-xl" />}
      <ul className="font-mono text-[12px]">
        {files?.map((f) => (
          <li key={f.path} className="flex items-center gap-2 rounded-md py-1 hover:bg-white/[0.03]" style={{ paddingLeft: f.depth * 16 + 4 }}>
            {f.is_repo ? <FolderGit2 className="size-3.5 text-accent" /> : f.dir ? <Folder className="size-3.5 text-fg-3" /> : <File className="size-3.5 text-fg-3" />}
            <span className={cx("truncate", f.dir ? "text-fg" : "text-fg-2")}>{f.path.split("/").pop()}</span>
            {!f.dir && <span className="ml-auto pr-1 text-[10.5px] text-fg-3">{fmtSize(f.size)}</span>}
          </li>
        ))}
      </ul>
      {files?.length === 0 && <div className="text-[12.5px] text-fg-3">Empty.</div>}
    </div>
  );
}
