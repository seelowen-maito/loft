import { useCallback, useEffect, useState } from "react";
import { MessageCircle, RotateCw, Send } from "lucide-react";
import { api, type Agent, type Settings, type SettingsView } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { PushSettings } from "./PushSettings";
import { cx } from "../lib/format";

const LABELS: Record<string, string> = { projects_dir: "Projects folder" };

const NOTICES: { key: keyof Settings["imessage"]; label: string; hint: string }[] = [
  { key: "permission", label: "An agent needs your input", hint: "A permission request is waiting for you." },
  { key: "scheduled_run", label: "A scheduled run finishes", hint: "With the agent's last reply, or the error." },
  { key: "manual_run", label: "A run you started finishes", hint: "Messages you send and Run now." },
  { key: "pr_opened", label: "An agent opens a pull request", hint: "With the link." },
  { key: "pr_merged", label: "A pull request is merged", hint: "Including ones you merge here." },
  { key: "update", label: "Loft updates", hint: "Auto-update installed or failed." },
];

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
  const [test, setTest] = useState<{ busy: boolean; msg?: string; ok?: boolean }>({ busy: false });
  const [confirming, setConfirming] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [agents, setAgents] = useState<Agent[]>([]);

  const load = useCallback(() => api.settings().then((v) => {
    setView(v);
    return v;
  }), []);
  useEffect(() => {
    load().then((v) => setDraft(v.settings)).catch((e) => setErr((e as Error).message));
    api.agents().then(setAgents).catch(() => {});
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
  const setMsg = <K extends keyof Settings["imessage"]>(k: K, v: Settings["imessage"][K]) => set("imessage", { ...draft.imessage, [k]: v });
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
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const sendTest = async () => {
    setTest({ busy: true });
    try {
      await api.testImessage(draft.imessage.to);
      setTest({ busy: false, ok: true, msg: "Sent. Check your phone." });
    } catch (e) {
      setTest({ busy: false, ok: false, msg: (e as Error).message });
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

        <Section title="Updates">
          <Toggle
            on={draft.auto_update}
            onChange={(v) => set("auto_update", v)}
            label="Update automatically"
            hint="Checks for new commits every minute. Installs once no agent is running, then restarts."
          />
        </Section>

        <Section title="iMessage notifications">
          <Toggle
            on={draft.imessage.enabled}
            onChange={(v) => setMsg("enabled", v)}
            label="Send notifications"
            hint="Sent from the Apple ID signed in to Messages on the Mac running loft."
          />
          <Field label="Send to" hint="Phone number or Apple ID email.">
            <div className="flex gap-2">
              <input className="input" placeholder="+1 555 0100 or you@icloud.com" value={draft.imessage.to} onChange={(e) => setMsg("to", e.target.value)} />
              <button type="button" className="btn shrink-0" disabled={test.busy || !draft.imessage.to.trim()} onClick={sendTest}>
                <Send className="size-3.5" /> {test.busy ? "Sending…" : "Send test"}
              </button>
            </div>
          </Field>
          {test.msg && <p className={cx("text-[12px]", test.ok ? "text-good" : "text-bad")}>{test.msg}</p>}
          <Field label="Replies go to" hint="Text back and this agent gets your message, then answers by iMessage. Needs Full Disk Access for loft on the Mac running it.">
            <select className="input" value={draft.imessage.agent_id} onChange={(e) => setMsg("agent_id", e.target.value)}>
              <option value="">Nobody (ignore replies)</option>
              {agents.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
              {draft.imessage.agent_id && !agents.some((a) => a.id === draft.imessage.agent_id) && <option value={draft.imessage.agent_id}>(deleted agent)</option>}
            </select>
          </Field>
          {view.imessage_inbox_error && <p className="text-[12px] text-bad">{view.imessage_inbox_error}</p>}
          <div className={cx("space-y-3 border-t border-line pt-4", !draft.imessage.enabled && "pointer-events-none opacity-40")}>
            <span className="eyebrow flex items-center gap-1.5">
              <MessageCircle className="size-3" /> Text me when
            </span>
            {NOTICES.map((n) => (
              <Toggle key={n.key} on={draft.imessage[n.key] as boolean} onChange={(v) => setMsg(n.key, v)} label={n.label} hint={n.hint} />
            ))}
          </div>
        </Section>

        <PushSettings />

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
