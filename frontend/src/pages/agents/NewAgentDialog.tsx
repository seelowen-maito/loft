import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { api, roleLabel, type Agent } from "../../lib/api";
import { applyPick, createPayload, currentPick, EMPTY_FORM, FRESH_CONTEXT, offersFreshContext, refillTemplate, rolePicks, withTeam, type AgentTemplate } from "../../lib/newAgent";
import { RoleField } from "./RolePicker";
import { FRESH_CONTEXT_HINT, INTERVALS, MODELS, PROJECTS_SUB, roleHint } from "./Inspector";
import { GroupInput, ProjectsField } from "./ProjectsField";
import { teamOf, withTyped } from "../../lib/team";
import { Dialog } from "../../components/Dialog";
import { ActionButton } from "../../components/ActionButton";
import { ErrorLine } from "../../components/Callout";
import { useAction } from "../../lib/useAction";

export function NewAgentDialog({ groups, projects, agents, onClose, onCreated }: { groups: string[]; projects: string[]; agents: Agent[]; onClose: () => void; onCreated: (a: Agent) => void }) {
  const [form, setForm] = useState(EMPTY_FORM);
  const [typed, setTyped] = useState("");
  const creating = useAction();
  // Set when creating it took the Lead role from another agent: the dialog stays open to say so, as Save does in the settings.
  const [moved, setMoved] = useState<{ agent: Agent; notice: string } | null>(null);
  const set = (k: keyof typeof form, v: string | number) => setForm((f) => ({ ...f, [k]: v }));
  const team = teamOf({ group: form.group, project: form.projects[0] ?? "" });
  // The main project's team model, when it has one: the agent starts on it unless the user picks a model here.
  const [teamModel, setTeamModel] = useState("");
  const [modelPicked, setModelPicked] = useState(false);
  const main = form.projects[0] ?? "";
  useEffect(() => {
    setTeamModel("");
    if (!main) return;
    let live = true;
    api.projectSettings(main)
      .then((v) => live && setTeamModel(String(v.settings.find((s) => s.key === "team_model")?.value ?? "")))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [main]);
  // The Agent templates, their instructions filled in for the main project: fetched again when it changes, and instructions
  // still a template's follow it.
  const [templates, setTemplates] = useState<AgentTemplate[]>([]);
  const [templatesError, setTemplatesError] = useState<string | null>(null);
  const shown = useRef<AgentTemplate[]>([]);
  useEffect(() => {
    let live = true;
    api.agentTemplates(main)
      .then((t) => {
        if (!live) return;
        setForm((f) => refillTemplate(f, shown.current, t));
        shown.current = t;
        setTemplates(t);
        setTemplatesError(null);
      })
      .catch((e) => live && setTemplatesError(String(e?.message ?? e)));
    return () => {
      live = false;
    };
  }, [main]);
  const { team: teamPicks, solo } = rolePicks(templates, roleLabel);
  const current = currentPick(form, [...teamPicks, ...solo]);
  const model = !modelPicked && teamModel ? teamModel : form.model;
  const customInterval = !INTERVALS.some((i) => i.value === form.interval_minutes);
  const close = () => (moved ? onCreated(moved.agent) : onClose());

  const create = () =>
    creating.run(async () => {
      const { notice, ...a } = await api.createAgent(createPayload(withTyped({ ...form, model }, typed)));
      if (notice) setMoved({ agent: a, notice });
      else onCreated(a);
    });

  return (
    <Dialog title="New agent" size="xl" onClose={close}>
      <div className="pointer-events-none absolute -right-20 -top-24 h-64 w-80 rounded-full bg-[radial-gradient(closest-side,rgb(200_242_109/0.14),transparent)] blur-2xl" />
      <div className="relative flex items-start justify-between px-7 pt-7">
        <div>
          <div className="eyebrow">New agent</div>
          <h2 className="mt-1.5 font-serif text-[34px] leading-none">Bring someone to life.</h2>
        </div>
        <button className="btn-ghost" onClick={close} aria-label="Close"><X className="size-4" /></button>
      </div>

      <div className="relative space-y-5 px-7 py-6">
        <RoleField form={form} current={current} team={teamPicks} solo={solo} error={templatesError} onPick={(p) => setForm((f) => applyPick(f, p, templates))} />

        <label className="block space-y-1.5">
          <span className="eyebrow">Name</span>
          <input autoFocus className="input" placeholder="e.g. Moss" value={form.name} onChange={(e) => set("name", e.target.value)} />
        </label>
        <div className="space-y-2">
          <label className="block space-y-1.5">
            <span className="eyebrow">Group</span>
            <GroupInput value={form.group} groups={groups} onChange={(v) => setForm((f) => withTeam(f, { group: v }))} />
          </label>
          <div className="space-y-1.5">
            <span className="eyebrow">Projects</span>
            <span className="block text-[11.5px] text-fg-3">{PROJECTS_SUB}</span>
            {/* Only an agent with a group can be Lead: clearing it drops it back to Custom. */}
            <ProjectsField value={form.projects} projects={projects} adding={typed} onAdding={setTyped} onChange={(v) => setForm((f) => withTeam(f, { projects: v }))} />
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
            <select className="input" value={model} title={model === teamModel && !modelPicked ? `${main}'s team model` : undefined} onChange={(e) => {
              setModelPicked(true);
              set("model", e.target.value);
            }}>
              {MODELS.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
            </select>
          </label>
          <label className="space-y-1.5">
            <span className="eyebrow">Autonomy</span>
            {/* A template's schedule (every 3 hours, weekly) is a Custom one, in minutes, as in the agent's settings. */}
            <div className="flex gap-2">
              <select className="input min-w-0" value={customInterval ? "custom" : form.interval_minutes} onChange={(e) => set("interval_minutes", e.target.value === "custom" ? 30 : Number(e.target.value))}>
                {INTERVALS.map((i) => <option key={i.value} value={i.value}>{i.label}</option>)}
                <option value="custom">Custom…</option>
              </select>
              {customInterval && (
                <div className="relative w-28 shrink-0">
                  <input type="number" min={1} aria-label="Minutes between check-ins" className="input pr-10" value={form.interval_minutes} onChange={(e) => set("interval_minutes", Math.max(1, Number(e.target.value)))} />
                  <span className="absolute right-3 top-1/2 -translate-y-1/2 font-mono text-[11px] text-fg-3">min</span>
                </div>
              )}
            </div>
          </label>
        </div>
        <ErrorLine error={creating.error} />
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
            <ActionButton pending={creating.busy} pendingLabel="Creating…" onClick={create}>Create agent</ActionButton>
          </>
        )}
      </div>
    </Dialog>
  );
}
