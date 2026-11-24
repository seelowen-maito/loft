import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { ArrowLeft } from "lucide-react";
import { api, type ProjectBranches, type ProjectSetting, type ProjectSettingsView } from "../lib/api";
import { SettingsCard } from "../components/SettingsCard";
import { ErrorLine } from "../components/Callout";
import { errorText, useAction } from "../lib/useAction";
import { cx } from "../lib/format";
import { fmtDateTime, relTime } from "../lib/time";

/** A project's own settings, opened from its repo panel: each starts at Loft's default, and shows who changed it and when. */
export default function ProjectSettingsPage() {
  const project = useParams().name ?? "";
  const [view, setView] = useState<ProjectSettingsView | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    setView(null);
    setErr(null);
    api.projectSettings(project).then(setView).catch((e) => setErr(errorText(e)));
  }, [project]);

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl space-y-5 px-4 py-8 pb-28 sm:px-5">
        <div>
          <Link to="/repos" className="inline-flex items-center gap-1.5 text-[12.5px] text-fg-3 hover:text-fg">
            <ArrowLeft className="size-3.5" /> Repos
          </Link>
          <div className="eyebrow mt-4">Project settings</div>
          <h1 className="mt-1 font-serif text-[34px] leading-none tracking-tight [overflow-wrap:anywhere]">{project}</h1>
        </div>
        {err && <ErrorLine error={err} />}
        {!view && !err && <div className="skeleton h-60 rounded-2xl" />}
        {view?.settings.map((s) => (
          <SettingCard key={s.key} project={project} setting={s} onSaved={setView} />
        ))}
      </div>
    </div>
  );
}

/** One setting: its choices (the chosen one marked), what each does, Loft's default and who changed it last. */
function SettingCard({ project, setting: s, onSaved }: { project: string; setting: ProjectSetting; onSaved: (v: ProjectSettingsView) => void }) {
  const save = useAction();
  const choose = (value: string) => {
    if (value === s.value) return;
    save.run(async () => onSaved(await api.setProjectSetting(project, s.key, value)));
  };
  const label = (v: string) => (s.kind === "branch" ? v || "the default branch" : (s.choices.find((c) => c.value === v)?.label ?? v));
  return (
    <SettingsCard title={s.label} description={s.help} error={save.error}>
      {s.kind === "branch" ? (
        <BranchPicker project={project} setting={s} busy={save.busy} onChoose={choose} />
      ) : (
        <div role="radiogroup" aria-label={s.label} className="grid gap-2 sm:grid-cols-2">
          {s.choices.map((c) => {
            const on = c.value === s.value;
            return (
              <button
                key={c.value}
                type="button"
                role="radio"
                aria-checked={on}
                disabled={save.busy}
                onClick={() => choose(c.value)}
                className={cx(
                  "flex items-start gap-3 rounded-xl border px-3.5 py-3 text-left transition-colors disabled:opacity-60",
                  on ? "border-accent/50 bg-accent/[0.07]" : "border-line bg-ink-900 hover:border-white/15",
                )}
              >
                <span className={cx("mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border", on ? "border-accent" : "border-fg-3")}>
                  {on && <span className="size-2 rounded-full bg-accent" />}
                </span>
                <span className="min-w-0">
                  <span className="flex flex-wrap items-center gap-2 text-[13.5px] text-fg">
                    {c.label}
                    {c.value === s.default && <span className="chip py-0 text-[10px]">Loft's default</span>}
                  </span>
                  <span className="mt-0.5 block text-[11.5px] text-fg-3">{c.help}</span>
                </span>
              </button>
            );
          })}
        </div>
      )}
      <p className="font-mono text-[11px] text-fg-3">
        {s.changed ? (
          <>
            Set to {label(s.value)} by {s.changed.agent ? s.changed.agent.name : "you"}{" "}
            <span title={fmtDateTime(s.changed.at)}>{relTime(s.changed.at)}</span>
          </>
        ) : (
          <>Never changed: Loft's default</>
        )}
      </p>
    </SettingsCard>
  );
}

/** A branch setting: the repo's branches on GitHub, the empty choice standing for its default branch. A saved branch
 *  GitHub no longer lists stays in the list, so the picker never shows a value it doesn't have. */
function BranchPicker({ project, setting: s, busy, onChoose }: { project: string; setting: ProjectSetting; busy: boolean; onChoose: (v: string) => void }) {
  const [list, setList] = useState<ProjectBranches | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    setList(null);
    setErr(null);
    api.projectBranches(project).then(setList).catch((e) => setErr(errorText(e)));
  }, [project]);
  const names = list ? list.branches : [];
  const shown = s.value && !names.includes(s.value) ? [s.value, ...names] : names;
  return (
    <div className="space-y-2">
      <select
        aria-label={s.label}
        value={s.value}
        disabled={busy || !list}
        onChange={(e) => onChoose(e.target.value)}
        className="input w-full font-mono text-[13px] disabled:opacity-60 sm:w-80"
      >
        <option value="">{list ? `${list.default_branch} (default branch)` : "The default branch"}</option>
        {shown.filter((b) => b !== list?.default_branch || b === s.value).map((b) => (
          <option key={b} value={b}>{b}</option>
        ))}
      </select>
      {!list && !err && <p className="text-[11.5px] text-fg-3">Reading the branches on GitHub…</p>}
      {err && <p role="alert" className="text-[12.5px] text-bad [overflow-wrap:anywhere]">{err}</p>}
    </div>
  );
}
