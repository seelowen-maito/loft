import { useEffect, useState } from "react";
import { Sparkles, X } from "lucide-react";
import { api, ROLES, type Agent } from "../../lib/api";
import { cx } from "../../lib/format";
import { createPayload, EMPTY_FORM, FRESH_CONTEXT, offersFreshContext, TEMPLATES, withTeam } from "../../lib/newAgent";
import { FRESH_CONTEXT_HINT, INTERVALS, MODELS, roleHint } from "./Inspector";
import { GroupInput, ProjectsField } from "./ProjectsField";
import { teamOf } from "../../lib/team";

export function NewAgentDialog({ groups, projects, agents, onClose, onCreated }: { groups: string[]; projects: string[]; agents: Agent[]; onClose: () => void; onCreated: (a: Agent) => void }) {
  const [form, setForm] = useState(EMPTY_FORM);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // Set when creating it took the Lead role from another agent: the dialog stays open to say so, as Save does in the settings.
  const [moved, setMoved] = useState<{ agent: Agent; notice: string } | null>(null);
  const set = (k: keyof typeof form, v: string | number) => setForm((f) => ({ ...f, [k]: v }));
  const team = teamOf({ group: form.group, project: form.projects[0] ?? "" });
  const close = () => (moved ? onCreated(moved.agent) : onClose());

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const create = async () => {
    setBusy(true);
    setErr(null);
    try {
      const { notice, ...a } = await api.createAgent(createPayload(form));
      if (notice) {
        setMoved({ agent: a, notice });
        setBusy(false);
      } else {
        onCreated(a);
      }
    } catch (e) {
      setErr((e as Error).message);
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={close} />
      <div className="relative w-full max-w-[620px] animate-rise overflow-clip rounded-3xl border border-line-strong bg-ink-900 shadow-2xl">
        <div className="pointer-events-none absolute -right-20 -top-24 h-64 w-80 rounded-full bg-[radial-gradient(closest-side,rgb(200_242_109/0.14),transparent)] blur-2xl" />
        <div className="relative flex items-start justify-between px-7 pt-7">
          <div>
            <div className="eyebrow">New agent</div>
            <h2 className="mt-1.5 font-serif text-[34px] leading-none">Bring someone to life.</h2>
          </div>
          <button className="btn-ghost" onClick={close} aria-label="Close"><X className="size-4" /></button>
        </div>

        <div className="relative space-y-5 px-7 py-6">
          <div className="flex flex-wrap gap-2">
            <span className="eyebrow mr-1 self-center">Start from</span>
            {TEMPLATES.map((t) => (
              <button
                key={t.label}
                className={cx("btn text-[12px]", form.title === t.title && "border-accent/50 text-accent")}
                onClick={() => setForm((f) => ({ ...f, ...t }))}
              >
                <Sparkles className="size-3" /> {t.label}
              </button>
            ))}
          </div>

          <div className="grid grid-cols-2 gap-3">
            <label className="space-y-1.5">
              <span className="eyebrow">Name</span>
              <input autoFocus className="input" placeholder="e.g. Moss" value={form.name} onChange={(e) => set("name", e.target.value)} />
            </label>
            <label className="space-y-1.5">
              <span className="eyebrow">Title</span>
              <input className="input" placeholder="e.g. Repo janitor" value={form.title} onChange={(e) => set("title", e.target.value)} />
            </label>
          </div>
          <div className="space-y-2">
            <div className="grid grid-cols-2 gap-3">
              <label className="space-y-1.5">
                <span className="eyebrow">Group</span>
                <GroupInput value={form.group} groups={groups} onChange={(v) => setForm((f) => withTeam(f, { group: v }))} />
              </label>
              <label className="space-y-1.5">
                <span className="eyebrow">Role</span>
                <select className="input" value={form.role} onChange={(e) => set("role", e.target.value)}>
                  {ROLES.map((r) => (
                    <option key={r.value} value={r.value} disabled={r.value === "lead" && !team}>
                      {r.label}: {r.hint}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <div className="space-y-1.5">
              <span className="eyebrow">Projects</span>
              {/* Only an agent with a group can be Lead: clearing it drops it back to Custom. */}
              <ProjectsField value={form.projects} projects={projects} onChange={(v) => setForm((f) => withTeam(f, { projects: v }))} />
            </div>
            <p className="text-[11.5px] leading-snug text-fg-3">{roleHint(team)}</p>
          </div>
          {offersFreshContext(form, agents) && (
            <label className="flex items-center justify-between gap-3" title={FRESH_CONTEXT_HINT}>
              <span className="eyebrow">Fresh context for each Delivery ticket</span>
              <select className="input w-52" value={form.fresh_context} onChange={(e) => set("fresh_context", e.target.value)}>
                {FRESH_CONTEXT.map((f) => <option key={f.value} value={f.value}>{f.label}</option>)}
              </select>
            </label>
          )}
          <label className="block space-y-1.5">
            <span className="eyebrow">Personality</span>
            <textarea className="input min-h-[64px] resize-none" placeholder="How should it sound and behave?" value={form.personality} onChange={(e) => set("personality", e.target.value)} />
          </label>
          <label className="block space-y-1.5">
            <span className="eyebrow">Standing instructions</span>
            <textarea className="input min-h-[96px] resize-none" placeholder="What does this agent own? What should it do when it wakes up?" value={form.prompt} onChange={(e) => set("prompt", e.target.value)} />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="space-y-1.5">
              <span className="eyebrow">Model</span>
              <select className="input" value={form.model} onChange={(e) => set("model", e.target.value)}>
                {MODELS.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
              </select>
            </label>
            <label className="space-y-1.5">
              <span className="eyebrow">Autonomy</span>
              <select className="input" value={form.interval_minutes} onChange={(e) => set("interval_minutes", Number(e.target.value))}>
                {INTERVALS.map((i) => <option key={i.value} value={i.value}>{i.label}</option>)}
              </select>
            </label>
          </div>
          {err && <div className="text-[12.5px] text-bad">{err}</div>}
        </div>

        <div className="relative flex items-center justify-between border-t border-line bg-ink-950/40 px-7 py-4">
          {moved ? (
            <>
              <span className="text-[12px] text-fg-2">{moved.notice}</span>
              <button className="btn-primary" onClick={close}>Open {moved.agent.name}</button>
            </>
          ) : (
            <>
              <span className="text-[12px] text-fg-3">It gets its own workspace, memory, and pixel face.</span>
              <button className="btn-primary" disabled={busy} onClick={create}>{busy ? "Creating…" : "Create agent"}</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
