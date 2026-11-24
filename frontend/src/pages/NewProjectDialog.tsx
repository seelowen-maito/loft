import { useEffect, useId, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { AlertTriangle, Loader2, Sparkles, X } from "lucide-react";
import { api, type Agent } from "../lib/api";
import { cx } from "../lib/format";
import { loadLastAgent } from "../lib/lastAgent";
import { teamOf } from "../lib/team";
import { Sticker } from "../components/Sticker";
import {
  TRY_IDEAS, createError, defaultLead, fixName, formView, leadsOf, loadProjectDraft, projectPath, saveProjectDraft, type NameCheck,
} from "../lib/newProject";

/** Repos' New project dialog (#448's design): a name and the idea, then Loft makes the folder and the Lead's grill starts. */
export function NewProjectDialog({ projectsDir, onClose }: { projectsDir?: string; onClose: () => void }) {
  const navigate = useNavigate();
  const ids = useId();
  const [draft] = useState(loadProjectDraft);
  const [name, setName] = useState(draft.name);
  const [idea, setIdea] = useState(draft.idea);
  const [agents, setAgents] = useState<Agent[] | null>(null);
  const [lead, setLead] = useState<string | null>(null);
  const [check, setCheck] = useState<NameCheck | null>(null);
  const [touched, setTouched] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const ideaRef = useRef<HTMLTextAreaElement>(null);

  const leads = leadsOf(agents ?? []);
  const finalName = fixName(name, false);
  const path = check?.name === finalName && finalName ? check.path : projectPath(projectsDir, finalName || "<name>");
  const view = formView({ name, idea, lead, check, touched, creating });

  useEffect(() => {
    api.agents().then((as) => {
      setAgents(as);
      setLead(defaultLead(leadsOf(as), loadLastAgent()));
    }).catch(() => setAgents([]));
  }, []);

  // The name is checked as you type, once typing pauses.
  useEffect(() => {
    if (!finalName) return;
    const t = setTimeout(() => api.checkProjectName(finalName).then(setCheck).catch(() => {}), 250);
    return () => clearTimeout(t);
  }, [finalName]);

  // Unsent text is kept as a Draft.
  useEffect(() => saveProjectDraft({ name, idea }), [name, idea]);

  const submit = async () => {
    setTouched(true);
    if (!view.canCreate || !lead) return;
    setCreating(true);
    setError(null);
    try {
      const r = await api.createProject({ name: finalName, idea: idea.trim(), lead_id: lead });
      saveProjectDraft({ name: "", idea: "" });
      const who = leads.find((l) => l.id === r.agent_id)?.name ?? "the Lead";
      // Say so only when opening the chat takes a moment.
      const slow = setTimeout(() => setOpening(`${r.project.name} created. Opening ${who}'s chat…`), 500);
      await api.grill(r.agent_id);
      clearTimeout(slow);
      navigate(`/agents/${r.agent_id}`);
    } catch (e) {
      setError(createError(path, (e as Error).message));
      setCreating(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !creating) onClose();
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        submit();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const nameHint = `${ids}-name-hint`;
  const nameErr = `${ids}-name-error`;
  const nameMessage = view.nameError ?? view.taken;
  const noLead = agents !== null && leads.length === 0;
  const one = leads.length === 1 ? leads[0] : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 max-sm:items-start max-sm:p-[18px]">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={() => !creating && onClose()} />
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${ids}-title`}
        className="relative max-h-full w-full max-w-[620px] animate-rise overflow-y-auto overflow-x-clip rounded-3xl border border-line-strong bg-ink-900 shadow-2xl max-sm:max-w-none"
      >
        <div className="pointer-events-none absolute -right-20 -top-24 h-64 w-80 rounded-full bg-[radial-gradient(closest-side,rgb(200_242_109/0.14),transparent)] blur-2xl" />
        <div className="relative flex items-start justify-between px-7 pt-7 max-sm:px-5 max-sm:pt-5">
          <div>
            <div className="eyebrow">New project</div>
            <h2 id={`${ids}-title`} className="mt-1.5 font-serif text-[34px] leading-none">What are we building?</h2>
          </div>
          <button className="btn-ghost" onClick={onClose} disabled={creating} aria-label="Close"><X className="size-4" /></button>
        </div>

        <fieldset disabled={creating} className="relative space-y-5 px-7 py-6 max-sm:px-5">
          <label className="block space-y-1.5">
            <span className="eyebrow">Name</span>
            <input
              autoFocus
              className={cx("input font-mono", nameMessage && "border-bad/60 focus:border-bad")}
              placeholder="on-call"
              value={name}
              spellCheck={false}
              autoCapitalize="off"
              aria-invalid={!!nameMessage}
              aria-describedby={nameMessage ? `${nameErr} ${nameHint}` : nameHint}
              onChange={(e) => setName(fixName(e.target.value))}
              onBlur={() => {
                setName(fixName(name, false));
                if (name) setTouched(true);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
                  e.preventDefault();
                  ideaRef.current?.focus();
                }
              }}
            />
            {nameMessage && <span id={nameErr} role="alert" className="block text-[12.5px] text-bad">{nameMessage}</span>}
            <span id={nameHint} aria-live="polite" className="block font-mono text-[11.5px] text-fg-3">Creates {path}</span>
          </label>

          <div className="space-y-1.5">
            <label htmlFor={`${ids}-idea`} className="eyebrow">The idea, in a sentence</label>
            <textarea
              id={`${ids}-idea`}
              ref={ideaRef}
              className="input min-h-[84px] resize-none"
              placeholder="A Java service with a Vite frontend that…"
              value={idea}
              onChange={(e) => setIdea(e.target.value)}
            />
            <div className="flex flex-wrap items-center gap-2 pt-1">
              <span className="eyebrow mr-1">Try</span>
              {TRY_IDEAS.map((t) => (
                <button key={t.label} type="button" className="btn text-[12px]" onClick={() => setIdea(t.idea)}>
                  <Sparkles className="size-3" /> {t.label}
                </button>
              ))}
            </div>
          </div>

          <div className="space-y-1.5">
            {leads.length > 1 ? (
              <>
                <label htmlFor={`${ids}-lead`} className="eyebrow">Who plans it</label>
                <select id={`${ids}-lead`} className="input" value={lead ?? ""} onChange={(e) => setLead(e.target.value)}>
                  {leads.map((l) => (
                    <option key={l.id} value={l.id}>{l.name}{teamOf(l) ? ` · ${teamOf(l)}` : ""}</option>
                  ))}
                </select>
              </>
            ) : (
              <>
                <span className="eyebrow">Who plans it</span>
                {one && (
                  <div className="flex items-center gap-3 rounded-xl border border-line bg-ink-950/40 px-3 py-2">
                    <Sticker avatar={one.avatar} lead size={32} />
                    <span className="text-[13.5px] text-fg">{one.name}</span>
                    {teamOf(one) && <span className="ml-auto rounded-full border border-line px-2 py-0.5 font-mono text-[10.5px] text-fg-3">{teamOf(one)}</span>}
                  </div>
                )}
                {agents === null && <div className="skeleton h-12 rounded-xl" />}
              </>
            )}
            {noLead && (
              <div role="status" className="flex items-start gap-3 rounded-2xl border border-warn/20 bg-warn/[0.06] px-4 py-3 text-[13px] text-fg-2">
                <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warn" />
                <span><span className="text-fg">No Lead yet.</span> A Lead plans new projects with you. Create one, then come back.</span>
              </div>
            )}
          </div>

          {error && (
            <div role="alert" className="flex items-start gap-3 rounded-2xl border border-bad/25 bg-bad/[0.06] px-4 py-3 text-[13px] text-fg-2">
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-bad" />
              <span>{error}</span>
            </div>
          )}
          {opening && <div role="status" className="text-[12.5px] text-fg-2">{opening}</div>}
        </fieldset>

        <div className="relative flex items-center justify-between gap-4 border-t border-line bg-ink-950/40 px-7 py-4 max-sm:flex-col max-sm:items-stretch max-sm:px-5">
          <span className="text-[12px] text-fg-3">Loft creates the folder, then the Lead starts asking. GitHub comes up in the first round.</span>
          <div className="flex shrink-0 items-center justify-end gap-2">
            <button className="btn" onClick={onClose} disabled={creating}>Cancel</button>
            <button className="btn-primary" onClick={submit} disabled={!view.canCreate || noLead}>
              {creating ? <><Loader2 className="size-3.5 animate-spin" /> Creating…</> : error ? "Try again" : "Create and start"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
