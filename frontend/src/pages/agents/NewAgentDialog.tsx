import { useEffect, useState } from "react";
import { Sparkles, X } from "lucide-react";
import { api, type Agent } from "../../lib/api";
import { cx } from "../../lib/format";
import { INTERVALS, MODELS } from "./Inspector";
import { ProjectSelect } from "./ProjectSelect";

const TEMPLATES = [
  {
    label: "Repo janitor",
    name: "Moss",
    title: "Repo janitor",
    personality: "Calm, tidy, slightly dry. Speaks in short, precise sentences. Loves a clean working tree.",
    prompt:
      "Keep an eye on my repositories. On each wake-up, check for uncommitted work, stale branches, and repos that are behind their remote. Report anything worth my attention in a short bulleted list. Never push or modify my repos directly.",
    interval_minutes: 360,
  },
  {
    label: "Research scout",
    name: "Wren",
    title: "Research scout",
    personality: "Curious and enthusiastic, but ruthless about signal over noise. Always cites sources.",
    prompt:
      "Track new developments relevant to the languages and frameworks used in my projects. Keep running notes in notes/ in your workspace, and remember what you've already reported so you never repeat yourself.",
    interval_minutes: 1440,
  },
  {
    label: "Pair programmer",
    name: "Bolt",
    title: "Pair programmer",
    personality: "Direct, pragmatic senior engineer. Proposes a plan before big changes and explains trade-offs briefly.",
    prompt: "Help me with coding tasks on request. Clone repos into your workspace, make changes on a branch, run the tests, and summarize the diff.",
    interval_minutes: 0,
  },
];

export function NewAgentDialog({ projects, onClose, onCreated }: { projects: string[]; onClose: () => void; onCreated: (a: Agent) => void }) {
  const [form, setForm] = useState({ name: "", title: "", project: "", personality: "", prompt: "", model: "sonnet", interval_minutes: 0 });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const set = (k: keyof typeof form, v: string | number) => setForm((f) => ({ ...f, [k]: v }));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const create = async () => {
    setBusy(true);
    setErr(null);
    try {
      const a = await api.createAgent({ ...form, name: form.name.trim() || "Unnamed" });
      onCreated(a);
    } catch (e) {
      setErr((e as Error).message);
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div className="relative w-full max-w-[620px] animate-rise overflow-clip rounded-3xl border border-line-strong bg-ink-900 shadow-2xl">
        <div className="pointer-events-none absolute -right-20 -top-24 h-64 w-80 rounded-full bg-[radial-gradient(closest-side,rgb(200_242_109/0.14),transparent)] blur-2xl" />
        <div className="relative flex items-start justify-between px-7 pt-7">
          <div>
            <div className="eyebrow">New agent</div>
            <h2 className="mt-1.5 font-serif text-[34px] leading-none">Bring someone to life.</h2>
          </div>
          <button className="btn-ghost" onClick={onClose} aria-label="Close"><X className="size-4" /></button>
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
              <span className="eyebrow">Role</span>
              <input className="input" placeholder="e.g. Repo janitor" value={form.title} onChange={(e) => set("title", e.target.value)} />
            </label>
          </div>
          <label className="block space-y-1.5">
            <span className="eyebrow">Project</span>
            <ProjectSelect value={form.project} projects={projects} onChange={(v) => set("project", v)} />
          </label>
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
          <span className="text-[12px] text-fg-3">It gets its own workspace, memory, and pixel face.</span>
          <button className="btn-primary" disabled={busy} onClick={create}>{busy ? "Creating…" : "Create agent"}</button>
        </div>
      </div>
    </div>
  );
}
