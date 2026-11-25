import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { ArrowLeft, Plus } from "lucide-react";
import { api, TEAM_SETTINGS, type ProjectBranches, type ProjectMergeMethods, type ProjectSetting, type ProjectSettingsView, type ReviewerRow } from "../lib/api";
import { SettingsCard } from "../components/SettingsCard";
import { Sticker } from "../components/Sticker";
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
        {view?.settings.map((s) =>
          !TEAM_SETTINGS.includes(s.key) ? (
            <SettingCard key={s.key} project={project} setting={s} onSaved={setView} />
          ) : s.key === TEAM_SETTINGS[0] ? (
            <TeamCard key="team" project={project} settings={view.settings.filter((t) => TEAM_SETTINGS.includes(t.key))} onSaved={setView} />
          ) : null,
        )}
      </div>
    </div>
  );
}

/** One setting: its choices (the chosen one marked), what each does, Loft's default and who changed it last. */
function SettingCard({ project, setting: s, onSaved }: { project: string; setting: ProjectSetting; onSaved: (v: ProjectSettingsView) => void }) {
  const save = useAction();
  const choose = (value: string | string[] | number) => {
    if (value === s.value) return;
    save.run(async () => onSaved(await api.setProjectSetting(project, s.key, value)));
  };
  const label = (): string => {
    if (s.kind === "reviewers") {
      const on = s.reviewers.filter((r) => s.value.includes(r.role)).map((r) => r.label);
      return on.length > 1 ? `${on.slice(0, -1).join(", ")} and ${on[on.length - 1]}` : (on[0] ?? "no one");
    }
    if (s.kind === "number") return `${s.value} ${s.value === 1 ? s.unit.replace(/s$/, "") : s.unit}`;
    return s.kind === "branch" ? s.value || "the default branch" : (s.choices.find((c) => c.value === s.value)?.label ?? s.value);
  };
  return (
    <SettingsCard title={s.label} description={s.help} error={save.error}>
      {s.kind === "reviewers" ? (
        <Reviewers project={project} rows={s.reviewers} value={s.value} busy={save.busy} onChoose={choose} />
      ) : s.kind === "branch" ? (
        <BranchPicker project={project} setting={s} busy={save.busy} onChoose={choose} />
      ) : s.kind === "number" ? (
        <NumberPicker setting={s} busy={save.busy} onChoose={choose} />
      ) : s.kind === "merge_method" ? (
        <MergeMethodPicker project={project} setting={s} busy={save.busy} onChoose={choose} />
      ) : (
        <ChoiceGrid label={s.label} choices={s.choices} value={s.value} loftDefault={s.default} busy={save.busy} onChoose={choose} />
      )}
      <Changed setting={s} label={label()} />
    </SettingsCard>
  );
}

/** A setting's fixed choices as radio cards, the chosen one marked, Loft's default tagged. */
function ChoiceGrid({ label, choices, value, loftDefault, busy, onChoose }: { label: string; choices: ProjectSetting["choices"]; value: string; loftDefault: string; busy: boolean; onChoose: (v: string) => void }) {
  return (
    <div role="radiogroup" aria-label={label} className="grid gap-2 sm:grid-cols-2">
      {choices.map((c) => {
        const on = c.value === value;
        return (
          <button
            key={c.value}
            type="button"
            role="radio"
            aria-checked={on}
            disabled={busy}
            onClick={() => onChoose(c.value)}
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
                {c.value === loftDefault && <span className="chip py-0 text-[10px]">Loft's default</span>}
              </span>
              <span className="mt-0.5 block text-[11.5px] text-fg-3">{c.help}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** The merge method: only the methods the repo allows on GitHub, the one Merge uses marked (squash, or else one it
 *  allows, when the repo stopped allowing the saved one). When GitHub can't be read, every method, with why. */
function MergeMethodPicker({ project, setting: s, busy, onChoose }: { project: string; setting: ProjectSetting & { kind: "choice" | "branch" | "merge_method" }; busy: boolean; onChoose: (v: string) => void }) {
  const [repo, setRepo] = useState<ProjectMergeMethods | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    setErr(null);
    api.projectMergeMethods(project).then(setRepo).catch((e) => setErr(errorText(e)));
  }, [project, s.value]);
  const allowed = repo?.allowed ?? [];
  const choices = err ? s.choices : s.choices.filter((c) => allowed.includes(c.value));
  const value = err || !repo ? s.value : repo.method;
  return (
    <div className="space-y-2">
      {!repo && !err && <p className="text-[11.5px] text-fg-3">Reading the merge methods the repo allows on GitHub…</p>}
      {(repo || err) && <ChoiceGrid label={s.label} choices={choices} value={value} loftDefault={s.default} busy={busy} onChoose={onChoose} />}
      {repo && value !== s.value && (
        <p className="text-[11.5px] text-warn">{repo.repo} doesn't allow {s.choices.find((c) => c.value === s.value)?.label ?? s.value} now, so Merge uses {s.choices.find((c) => c.value === value)?.label ?? value}.</p>
      )}
      {err && <p role="alert" className="text-[12.5px] text-bad [overflow-wrap:anywhere]">Couldn't read which merge methods the repo allows: {err}</p>}
    </div>
  );
}

/** Who last changed a setting and when, or that it's still Loft's default. */
function Changed({ setting: s, label, what }: { setting: ProjectSetting; label: string; what?: string }) {
  return (
    <p className="font-mono text-[11px] text-fg-3">
      {s.changed ? (
        <>
          {what ? `${what} set to` : "Set to"} {label} by {s.changed.agent ? s.changed.agent.name : "you"}{" "}
          <span title={fmtDateTime(s.changed.at)}>{relTime(s.changed.at)}</span>
        </>
      ) : (
        <>Never changed: Loft's default</>
      )}
    </p>
  );
}

/** The team defaults in one card: the model, effort and subagent model new agents of the project start on, each a
 *  dropdown, and "Apply to the team now", which gives them to the agents the team has. Each agent stays editable. */
function TeamCard({ project, settings, onSaved }: { project: string; settings: ProjectSetting[]; onSaved: (v: ProjectSettingsView) => void }) {
  const save = useAction();
  const apply = useAction();
  const [applied, setApplied] = useState<{ id: string; name: string }[] | null>(null);
  const choose = (s: ProjectSetting, value: string) => {
    if (value === s.value) return;
    setApplied(null);
    save.run(async () => onSaved(await api.setProjectSetting(project, s.key, value)));
  };
  const set = settings.some((s) => s.value !== "");
  const latest = settings.filter((s) => s.changed).sort((a, b) => (b.changed!.at > a.changed!.at ? 1 : -1))[0];
  const short = (s: ProjectSetting) => s.label.replace(/^Team (\w)/, (_, c: string) => c.toUpperCase());
  const choiceLabel = (s: ProjectSetting) => s.choices.find((c) => c.value === s.value)?.label ?? String(s.value);
  return (
    <SettingsCard
      title="Team model"
      description="What new agents of the project start on. The agents the team has keep theirs until you apply these, and each agent stays editable on its own."
      error={save.error ?? apply.error}
    >
      <div className="grid gap-3 sm:grid-cols-3">
        {settings.map((s) => (
          <label key={s.key} className="block space-y-1.5" title={s.help}>
            <span className="eyebrow">{short(s)}</span>
            <select className="input w-full disabled:opacity-60" aria-label={s.label} value={String(s.value)} disabled={save.busy} onChange={(e) => choose(s, e.target.value)}>
              {s.choices.map((c) => (
                <option key={c.value} value={c.value}>{c.label}</option>
              ))}
            </select>
            <span className="block text-[11.5px] text-fg-3">{s.choices.find((c) => c.value === s.value)?.help}</span>
          </label>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          className="btn disabled:opacity-60"
          disabled={!set || apply.busy || save.busy}
          title={set ? "Gives the agents whose main project this is the defaults that are set" : "Pick a model, effort or subagent model first"}
          onClick={() => apply.run(async () => {
            const { applied, ...v } = await api.applyTeamDefaults(project);
            onSaved(v);
            setApplied(applied);
          })}
        >
          {apply.busy ? "Applying…" : "Apply to the team now"}
        </button>
        {applied && (
          <span role="status" className="text-[12px] text-fg-3 [overflow-wrap:anywhere]">
            {applied.length ? `Applied to ${applied.map((a) => a.name).join(", ")}.` : "The team already has them."}
          </span>
        )}
      </div>
      {latest ? <Changed setting={latest} what={short(latest)} label={choiceLabel(latest)} /> : <Changed setting={settings[0]} label="" />}
    </SettingsCard>
  );
}

/** A number setting: a short range (like rounds, 1 to 5) is one button per number, a long one (like minutes) a field
 *  saved on Enter or when it loses focus. Loft's default is marked either way. */
function NumberPicker({ setting: s, busy, onChoose }: { setting: ProjectSetting & { kind: "number" }; busy: boolean; onChoose: (v: number) => void }) {
  const [draft, setDraft] = useState(String(s.value));
  useEffect(() => setDraft(String(s.value)), [s.value]);
  if (s.max - s.min < 10) {
    const all = Array.from({ length: s.max - s.min + 1 }, (_, i) => s.min + i);
    return (
      <div className="space-y-1.5">
        <div role="radiogroup" aria-label={s.label} className="inline-flex overflow-hidden rounded-xl border border-line bg-ink-900">
          {all.map((n) => {
            const on = n === s.value;
            return (
              <button
                key={n}
                type="button"
                role="radio"
                aria-checked={on}
                disabled={busy}
                onClick={() => onChoose(n)}
                className={cx(
                  "min-w-11 border-l border-line px-3 py-2 text-[13.5px] tabular-nums transition-colors first:border-l-0 disabled:opacity-60",
                  on ? "bg-accent/[0.12] text-fg" : "text-fg-2 hover:bg-white/[0.04]",
                )}
              >
                {n}
              </button>
            );
          })}
        </div>
        <p className="text-[11.5px] text-fg-3">{s.unit[0].toUpperCase() + s.unit.slice(1)}. Loft's default: {s.default}.</p>
      </div>
    );
  }
  // A whole number in range, like the API takes: a decimal is put back like an out-of-range value.
  const n = Number(draft);
  const valid = draft.trim() !== "" && Number.isInteger(n) && n >= s.min && n <= s.max;
  const commit = () => (valid ? onChoose(n) : setDraft(String(s.value)));
  return (
    <div className="space-y-1.5">
      <label className="flex items-center gap-2 text-[13px] text-fg-2">
        <input
          className="input w-24 tabular-nums"
          type="number"
          aria-label={s.label}
          min={s.min}
          max={s.max}
          step={1}
          value={draft}
          disabled={busy}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
        {s.unit}
      </label>
      <p className="text-[11.5px] text-fg-3">
        From {s.min} to {s.max}. Loft's default: {s.default}.
      </p>
    </div>
  );
}

/** A branch setting: the repo's branches on GitHub, the empty choice standing for its default branch. A saved branch
 *  GitHub no longer lists stays in the list, so the picker never shows a value it doesn't have. */
function BranchPicker({ project, setting: s, busy, onChoose }: { project: string; setting: ProjectSetting & { kind: "choice" | "branch" | "merge_method" }; busy: boolean; onChoose: (v: string) => void }) {
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

/** The ticket reviewers: one compact row per role, its agent's sticker, name and role with a switch, or, when the team
 *  has no one in it, a dimmed row whose Create asks Loft's own agent to set one up. */
function Reviewers({ project, rows, value, busy, onChoose }: { project: string; rows: ReviewerRow[]; value: string[]; busy: boolean; onChoose: (v: string[]) => void }) {
  const flip = (role: string, on: boolean) => onChoose(on ? [...value, role] : value.filter((r) => r !== role));
  return (
    <ul className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-ink-900">
      {rows.map((r) => (
        <li key={r.role} className="flex items-center gap-3 px-3 py-2.5">
          {r.agent ? (
            <>
              <Sticker avatar={r.agent.avatar} size={30} />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                  <Link to={`/agents/${r.agent.id}`} className="truncate text-[13.5px] text-fg hover:underline">{r.agent.name}</Link>
                  <span className="chip py-0 text-[10px]">{r.label}</span>
                </div>
                <p className="mt-0.5 text-[11.5px] text-fg-3">{r.note}</p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={r.on}
                aria-label={`${r.agent.name} reviews each ticket`}
                title={r.on ? "Reviews each ticket" : "Doesn't review each ticket"}
                disabled={busy}
                onClick={() => flip(r.role, !r.on)}
                className={cx("flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors disabled:opacity-60", r.on ? "bg-accent" : "bg-ink-700")}
              >
                <span className={cx("size-4 rounded-full bg-ink-950 transition-transform", r.on && "translate-x-4")} />
              </button>
            </>
          ) : (
            <MissingRole project={project} row={r} />
          )}
        </li>
      ))}
    </ul>
  );
}

/** A reviewer role the team has no one in: dimmed, with Create, which asks Loft's own agent to set one up. */
function MissingRole({ project, row: r }: { project: string; row: ReviewerRow }) {
  const ask = useAction();
  const [asked, setAsked] = useState<{ id: string; name: string } | null>(null);
  return (
    <>
      <span className="size-[30px] shrink-0 rounded-full border border-dashed border-fg-3/50 opacity-60" />
      <div className="min-w-0 flex-1 opacity-60">
        <div className="text-[13.5px] text-fg-2">No {r.label} on the team</div>
        <p className="mt-0.5 text-[11.5px] text-fg-3">{r.note}</p>
        {ask.error && <p role="alert" className="mt-1 text-[12px] text-bad [overflow-wrap:anywhere]">{ask.error}</p>}
      </div>
      {asked ? (
        <Link to={`/agents/${asked.id}`} className="shrink-0 text-[12px] text-fg-3 hover:text-fg">
          Asked {asked.name}
        </Link>
      ) : (
        <button
          type="button"
          disabled={ask.busy}
          onClick={() => ask.run(async () => setAsked((await api.createReviewer(project, r.role)).agent))}
          className="btn shrink-0 whitespace-nowrap disabled:opacity-60"
        >
          <Plus className="size-3.5" /> Create
        </button>
      )}
    </>
  );
}
