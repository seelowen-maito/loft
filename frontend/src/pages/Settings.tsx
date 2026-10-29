import { useCallback, useEffect, useState } from "react";
import { RotateCw } from "lucide-react";
import { api, type ApprovalMode, type GithubBudget, type Settings, type SettingsView } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { PushSettings } from "./PushSettings";
import { MacAccess } from "./MacAccess";
import { cx } from "../lib/format";

const LABELS: Record<string, string> = { projects_dir: "Projects folder" };

/** Window event fired after a save, with the saved Settings as detail, so the navbar can follow. */
export const SETTINGS_SAVED = "loft:settings-saved";

function Field({ label, hint, restart, children }: { label: string; hint?: string; restart?: boolean; children: React.ReactNode }) {
  return (
    <label className="block space-y-1.5">
      <span className="flex items-center gap-2">
        <span className="eyebrow">{label}</span>
        {restart && <span className="chip text-[10px]">restart</span>}
      </span>
      {children}
      {hint && <span className="block text-[11.5px] text-fg-3">{hint}</span>}
    </label>
  );
}

export function Toggle({ on, onChange, label, hint }: { on: boolean; onChange: (v: boolean) => void; label: string; hint?: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} onClick={() => onChange(!on)} className="flex w-full items-start gap-3 text-left">
      <span className={cx("mt-0.5 flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors", on ? "bg-accent" : "bg-ink-700")}>
        <span className={cx("size-4 rounded-full bg-ink-950 transition-transform", on && "translate-x-4")} />
      </span>
      <span>
        <span className="block text-[13.5px] text-fg">{label}</span>
        {hint && <span className="block text-[11.5px] text-fg-3">{hint}</span>}
      </span>
    </button>
  );
}

const MODES: { value: ApprovalMode; label: string }[] = [
  { value: "ask", label: "Ask for every comment" },
  { value: "own", label: "Auto-approve my own comments" },
  { value: "all", label: "Auto-approve everything" },
];

function ModeSelect({ value, onChange }: { value: ApprovalMode; onChange: (m: ApprovalMode) => void }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value as ApprovalMode)} className="input">
      {MODES.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
    </select>
  );
}

const SOURCE_LABELS: Record<string, string> = {
  prs: "Pull request cards",
  mirror: "Issue mirror",
  repos: "Repos page",
  inbox: "GitHub inbox",
  user: "Your clicks",
  agent: "Agents' Loft tools",
  other: "Other",
};

/** What's left of GitHub's hourly limit (shared with the agents' own `gh`), and where Loft's requests went. */
function GithubUsage() {
  const [b, setB] = useState<GithubBudget | null>(null);
  useEffect(() => {
    const load = () => api.githubBudget().then(setB).catch(() => {});
    load();
    const id = setInterval(load, 15_000);
    return () => clearInterval(id);
  }, []);
  const at = (s: number) => new Date(s * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const rows = Object.entries(b?.by_source ?? {}).sort((x, y) => y[1].requests - x[1].requests);
  return (
    <Section title="GitHub API">
      {!b || Object.keys(b.resources).length === 0 ? (
        <p className="text-[12.5px] text-fg-3">No answer from GitHub yet since Loft started.</p>
      ) : (
        <div className="space-y-3">
          {Object.entries(b.resources).map(([name, r]) => (
            <div key={name} className="space-y-1">
              <div className="flex items-baseline justify-between text-[13px]">
                <span className="text-fg">{name === "core" ? "REST" : name === "graphql" ? "GraphQL" : name}: {r.remaining.toLocaleString()} of {r.limit.toLocaleString()} left</span>
                <span className="font-mono text-[11px] text-fg-3">resets {at(r.reset)}</span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-ink-700">
                <div className={cx("h-full rounded-full", r.remaining / r.limit < 0.1 ? "bg-bad" : r.remaining / r.limit < 0.3 ? "bg-warn" : "bg-good")} style={{ width: `${Math.max(0, Math.min(100, (r.remaining / r.limit) * 100))}%` }} />
              </div>
            </div>
          ))}
          <p className="text-[11.5px] text-fg-3">The limit is per GitHub account, so the agents' own <code className="font-mono">gh</code> commands use it too.</p>
          {rows.length > 0 && (
            <table className="w-full text-[12.5px]">
              <thead>
                <tr className="text-left font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
                  <th className="py-1 font-normal">Loft's requests this hour</th>
                  <th className="py-1 text-right font-normal">counted</th>
                  <th className="py-1 text-right font-normal">free (304)</th>
                </tr>
              </thead>
              <tbody>
                {rows.map(([s, c]) => (
                  <tr key={s} className="border-t border-line">
                    <td className="py-1.5 text-fg-2">{SOURCE_LABELS[s] ?? s}</td>
                    <td className="py-1.5 text-right font-mono">{c.requests}</td>
                    <td className="py-1.5 text-right font-mono text-fg-3">{c.not_modified}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </Section>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="panel space-y-4 p-5">
      <h2 className="text-[15px] font-medium tracking-tight">{title}</h2>
      {children}
    </section>
  );
}

export default function SettingsPage() {
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  // Write-only: undefined keeps the saved token, "" clears it.
  const [token, setToken] = useState<string | undefined>(undefined);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [restarting, setRestarting] = useState(false);

  const load = useCallback(() => api.settings().then((v) => {
    setView(v);
    return v;
  }), []);
  useEffect(() => {
    load().then((v) => setDraft(v.settings)).catch((e) => setErr((e as Error).message));
  }, [load]);
  // Keep the running-agents count fresh; it gates the restart.
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "agent_updated") load().catch(() => {});
    }, [load]),
  );

  if (!view || !draft) {
    return <div className="mx-auto max-w-3xl px-5 py-8">{err ? <p className="text-bad">{err}</p> : <div className="skeleton h-96 rounded-2xl" />}</div>;
  }

  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => setDraft({ ...draft, [k]: v });
  const dirty = JSON.stringify(draft) !== JSON.stringify(view.settings) || token !== undefined;
  const restartFields = view.restart_fields as string[];
  const dirtyRestart = restartFields.filter((f) => JSON.stringify(draft[f as keyof Settings]) !== JSON.stringify(view.settings[f as keyof Settings]));

  const save = async () => {
    setSaving(true);
    setErr(null);
    try {
      const v = await api.saveSettings({ ...draft, github_token: token });
      setView(v);
      setDraft(v.settings);
      setToken(undefined);
      window.dispatchEvent(new CustomEvent(SETTINGS_SAVED, { detail: v.settings }));
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const restart = async () => {
    setErr(null);
    try {
      await api.restart();
    } catch (e) {
      setConfirming(false);
      setErr((e as Error).message);
      return;
    }
    setConfirming(false);
    setRestarting(true);
    // Wait for the new process to answer.
    for (let i = 0; i < 60; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      try {
        const v = await load();
        if (!v.pending_restart.length) break;
      } catch {
        /* still restarting */
      }
    }
    setRestarting(false);
  };

  const running = view.running_agents;

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl space-y-5 px-5 py-8 pb-28">
        <h1 className="text-[22px] font-medium tracking-tight">Settings</h1>

        {(view.pending_restart.length > 0 || restarting) && (
          <div className="flex items-center gap-3 rounded-2xl border border-warn/25 bg-warn/[0.06] px-4 py-3 text-[13px]">
            <RotateCw className={cx("size-4 shrink-0 text-warn", restarting && "animate-spin")} />
            <div className="flex-1 text-fg-2">
              {restarting ? (
                "Restarting loft…"
              ) : (
                <>
                  <span className="text-fg">Restart to apply</span> {view.pending_restart.map((f) => LABELS[f] ?? f).join(", ")}.
                  {running > 0 && <> Waiting for {running} running agent{running > 1 ? "s" : ""} to finish.</>}
                </>
              )}
            </div>
            {!restarting && (
              <button className="btn" disabled={running > 0} onClick={() => setConfirming(true)}>
                Restart now
              </button>
            )}
          </div>
        )}

        <Section title="General">
          <Field label="Projects folder" hint="Where your git repos live." restart>
            <input className="input font-mono" value={draft.projects_dir} onChange={(e) => set("projects_dir", e.target.value)} />
          </Field>
          <Field label="Claude command" hint="What agents run.">
            <input className="input font-mono" value={draft.claude_bin} onChange={(e) => set("claude_bin", e.target.value)} />
          </Field>
          <Field label="SSH host" hint="How other machines reach this one, e.g. loft. /code from them then opens VS Code over Remote-SSH (VS Code only). Leave empty if you only use loft on this machine.">
            <input className="input font-mono" placeholder="none" value={draft.ssh_host} onChange={(e) => set("ssh_host", e.target.value)} />
          </Field>
          <Field label="GitHub token" hint={view.github_token_set ? "A token is saved. Type a new one to replace it." : "Needed for private repos and merging PRs. Without one, loft uses gh auth token."}>
            <div className="flex gap-2">
              <input
                className="input font-mono"
                type="password"
                autoComplete="off"
                placeholder={view.github_token_set && token === undefined ? "•••••••• saved" : "ghp_…"}
                value={token ?? ""}
                onChange={(e) => setToken(e.target.value)}
              />
              {view.github_token_set && (
                <button type="button" className="btn shrink-0" onClick={() => setToken("")}>
                  Remove
                </button>
              )}
            </div>
          </Field>
        </Section>

        <GithubUsage />

        <Section title="Updates">
          <Toggle
            on={draft.auto_update}
            onChange={(v) => set("auto_update", v)}
            label="Update automatically"
            hint="Checks for new commits every minute. Installs once no agent is running, then restarts."
          />
        </Section>

        <Section title="Agents">
          <Toggle
            on={draft.messaging_rules}
            onChange={(v) => set("messaging_rules", v)}
            label="Messaging rules"
            hint="Tells every agent to message other agents only with a finding, report, verdict or request, never thanks or waiting notes."
          />
        </Section>

        <Section title="GitHub issues">
          <Field label="Comments going to GitHub" hint="Every comment on a GitHub issue waits for your approval unless this says otherwise.">
            <ModeSelect value={draft.comment_approval_default} onChange={(m) => set("comment_approval_default", m)} />
          </Field>
          <Toggle
            on={draft.auto_publish_all}
            onChange={(v) => set("auto_publish_all", v)}
            label="Publish new Loft issues to GitHub automatically"
            hint="Each new Loft issue in a project with a GitHub repo becomes a GitHub issue right away. Off: it stays in Loft until you press Publish to GitHub."
          />
        </Section>

        <Section title="Navigation">
          <Toggle
            on={draft.show_simulator}
            onChange={(v) => set("show_simulator", v)}
            label="Show Simulator"
            hint="Adds a tab to view and control a booted iOS Simulator on this Mac."
          />
        </Section>

        <PushSettings />

        <MacAccess />

        {(dirty || err) && (
          <div className="fixed inset-x-0 bottom-5 z-40 flex justify-center px-4">
            <div className="flex items-center gap-3 rounded-full border border-line-strong bg-ink-850/95 py-1.5 pl-4 pr-1.5 text-[13px] shadow-2xl backdrop-blur">
              {err ? (
                <span className="text-bad">{err}</span>
              ) : (
                <span className="text-fg-2">
                  Unsaved changes{dirtyRestart.length > 0 && <> · {dirtyRestart.map((f) => LABELS[f] ?? f).join(", ")} needs a restart</>}
                </span>
              )}
              {dirty && (
                <>
                  <button className="btn-ghost" onClick={() => { setDraft(view.settings); setToken(undefined); setErr(null); }}>
                    Discard
                  </button>
                  <button className="btn-primary" disabled={saving} onClick={save}>
                    {saving ? "Saving…" : "Save"}
                  </button>
                </>
              )}
            </div>
          </div>
        )}

        {confirming && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={() => setConfirming(false)}>
            <div className="panel w-full max-w-sm space-y-4 bg-ink-900 p-5" onClick={(e) => e.stopPropagation()}>
              <h3 className="text-[15px] font-medium">Restart loft?</h3>
              <p className="text-[13px] text-fg-2">
                This applies {view.pending_restart.map((f) => LABELS[f] ?? f).join(", ")}. It takes a few seconds; open tabs reconnect on their own.
                {running > 0 && <span className="mt-2 block text-warn">{running} agent{running > 1 ? "s are" : " is"} running. Restart once they finish.</span>}
              </p>
              <div className="flex justify-end gap-2">
                <button className="btn-ghost" onClick={() => setConfirming(false)}>
                  Cancel
                </button>
                <button className="btn-primary" disabled={running > 0} onClick={restart}>
                  Restart
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
