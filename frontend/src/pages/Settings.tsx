import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { RotateCw, TriangleAlert } from "lucide-react";
import { api, type ActionsOwner, type ActionsOwners, type AgentDevice, type AgentsSummary, type BackupStatus, type CleanLog, type CleanSkip, type GithubBudget, type WorkspaceSizes, type Settings, type SettingsView } from "../lib/api";
import { monthName, parseQuota, quotaPercent, quotaTone } from "../lib/actionsQuota";
import { exactDollars } from "../lib/usage";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { PushSettings } from "./PushSettings";
import { MacAccess } from "./MacAccess";
import { FolderField } from "./FolderField";
import { HttpsAccess } from "./HttpsAccess";
import { VoiceOnThisMac } from "./VoiceOnThisMac";
import { SettingsCard } from "../components/SettingsCard";
import { SettingsField as Field } from "../components/SettingsField";
import { cardInView, keepPin, type Pin } from "../lib/settingsSpy";
import { Callout, ErrorLine } from "../components/Callout";
import { ActionButton } from "../components/ActionButton";
import { errorText, useAction } from "../lib/useAction";
import { FloatingBar } from "../components/FloatingBar";
import { ConfirmDialog, Dialog } from "../components/Dialog";
import { ClaudeLoginSection } from "./ClaudeLogin";
import { HerdrSetup } from "./HerdrSetup";
import { cleanLineText, cx, fmtBytes, oneLine } from "../lib/format";
import { fmtClock, fmtDateTime, relTime } from "../lib/time";
import { comboFromEvent, fixedShortcuts, formatCombo, isReserved, loadCombos, normalizeCombo, REBINDABLE, recording, saveCombo, SHORTCUTS_CHANGED, type Rebindable } from "../lib/shortcuts";
import { Keys } from "../components/Shortcuts";
import { usePoll } from "../hooks/usePoll";
import { useOffered } from "../hooks/useOffered";
import type { MacOnly } from "../lib/platform";

const LABELS: Record<string, string> = { projects_dir: "Projects folder" };

/** Window event fired after a save, with the saved Settings as detail, so the navbar can follow. */
export const SETTINGS_SAVED = "loft:settings-saved";

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

const SOURCE_LABELS: Record<string, string> = {
  prs: "Pull request cards",
  mirror: "Issue mirror",
  repos: "Repos page",
  inbox: "GitHub inbox",
  user: "Your clicks",
  agent: "Agents' Loft tools",
  actions: "Actions minutes",
  other: "Other",
};

/** What's left of GitHub's hourly limit (shared with the agents' own `gh`), where Loft's requests went, and each
 *  owner's Actions minutes this month against its quota. */
function GithubUsage() {
  const [b, setB] = useState<GithubBudget | null>(null);
  usePoll(() => api.githubBudget().then(setB).catch(() => {}), 15_000);
  const at = (s: number) => fmtClock(s);
  const rows = Object.entries(b?.by_source ?? {}).sort((x, y) => y[1].requests - x[1].requests);
  return (
    <SettingsCard id="github" title="GitHub API">
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
          <WorkflowScope scopes={b.scopes} />
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
      <ActionsQuotas />
    </SettingsCard>
  );
}

/** Whether Loft's own GitHub token can push changes to `.github/workflows/` (the `workflow` scope), which run with a
 *  repo's secrets once pushed to one of its branches. Git pushing over SSH or another credential isn't limited by it. */
function WorkflowScope({ scopes }: { scopes?: string[] | null }) {
  if (scopes === undefined) return null;
  const code = (s: string) => <code className="font-mono">{s}</code>;
  return (
    <p className="text-[12.5px] text-fg-2">
      {scopes === null ? (
        <>Workflows: GitHub lists no scopes for Loft's token (fine-grained or app), so check its Workflows permission on GitHub.</>
      ) : scopes.includes("workflow") ? (
        <>Workflows: Loft's token has the {code("workflow")} scope, so it can push changes to {code(".github/workflows/")}.</>
      ) : (
        <>Workflows: Loft's token has no {code("workflow")} scope, so it can't push changes to {code(".github/workflows/")} itself.</>
      )}{" "}
      Git pushes over SSH or with another credential aren't limited by it. Loft flags a PR that changes workflows or actions.
    </p>
  );
}

/** Each owner's GitHub Actions minutes this month, with its quota to edit: Needs you warns at 70% and 90% of it. */
function ActionsQuotas() {
  const [v, setV] = useState<ActionsOwners | null>(null);
  const [err, setErr] = useState<string | null>(null);
  usePoll(
    () =>
      api
        .actionsOwners()
        .then((v) => (setV(v), setErr(null)))
        .catch((e) => setErr(errorText(e))),
    60_000,
  );
  return (
    <div className="space-y-3 border-t border-line pt-4">
      <div className="eyebrow">Actions minutes{v ? ` · ${monthName(v.month)}` : ""}</div>
      {err && !v && <ErrorLine error={err} />}
      {!v && !err && <div className="skeleton h-12 rounded-xl" />}
      {v && v.owners.length === 0 && <p className="text-[12.5px] text-fg-3">No repo on GitHub read yet: Loft reads each project's repo every 15 minutes.</p>}
      {v?.owners.map((o) => <OwnerQuota key={o.owner} o={o} onSaved={setV} />)}
      <p className="text-[11.5px] text-fg-3">
        Per org or account: the sum of its private repos Loft knows, each job rounded up to the minute (public repos are free) and weighted as GitHub uses up included minutes (macOS jobs 10×, Windows 2×), or the org's own billed minutes when the token has <code className="font-mono">admin:org</code>. At 70% and 90% of a quota,
        Needs you tells you, once a month.
      </p>
    </div>
  );
}

function OwnerQuota({ o, onSaved }: { o: ActionsOwner; onSaved: (v: ActionsOwners) => void }) {
  const saved = o.quota === null ? "" : String(o.quota);
  const [draft, setDraft] = useState(saved);
  useEffect(() => setDraft(saved), [saved]);
  const save = useAction();
  const commit = () => {
    const q = parseQuota(draft);
    if (q === undefined) return setDraft(saved);
    if (q === o.quota) return setDraft(saved);
    save.run(async () => onSaved(await api.setActionsQuota(o.owner, q)));
  };
  const pct = quotaPercent(o.minutes, o.quota);
  const priv = o.repos.filter((r) => r.private).sort((a, b) => b.minutes - a.minutes);
  const where =
    o.source === "billing"
      ? "As GitHub bills the org"
      : priv.length === 0
        ? "No private repo Loft knows"
        : `${priv.length} private repo${priv.length === 1 ? "" : "s"}: ${priv
            .slice(0, 3)
            .map((r) => `${r.repo.split("/")[1]} ${r.minutes.toLocaleString()} min`)
            .join(", ")}${priv.length > 3 ? ", …" : ""}`;
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5 text-[13px]">
        <span className="min-w-0 [overflow-wrap:anywhere]">
          <span className="text-fg">{o.owner}</span>
          {o.kind && <span className="ml-1.5 text-[11.5px] text-fg-3">{o.kind === "Organization" ? "org" : "account"}</span>}
        </span>
        <span className="flex items-center gap-3">
          <span className="font-mono text-[12.5px] tabular-nums">
            {o.minutes.toLocaleString()} min{pct !== null && <span className="text-fg-3"> · {Math.round(pct)}%</span>}
          </span>
          <label className="flex items-center gap-1.5 text-[11.5px] text-fg-3">
            quota
            <input
              className="input w-24 py-1 text-right font-mono text-[12px] tabular-nums"
              inputMode="numeric"
              placeholder="none"
              aria-label={`Monthly Actions quota of ${o.owner}, in minutes`}
              value={draft}
              disabled={save.busy}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commit}
              onKeyDown={(e) => e.key === "Enter" && commit()}
            />
          </label>
        </span>
      </div>
      {pct !== null && (
        <div className="h-1.5 overflow-hidden rounded-full bg-ink-700">
          <div className={cx("h-full rounded-full", quotaTone(pct))} style={{ width: `${Math.min(100, pct)}%` }} />
        </div>
      )}
      <p className="text-[11.5px] text-fg-3 [overflow-wrap:anywhere]">{where}</p>
      {save.error && <ErrorLine error={save.error} />}
    </div>
  );
}

const chip = (on: boolean) => cx("chip text-[12px]", on && "border-accent/40 text-accent");

/** Backups of Loft's database: the folder (saved with the other settings), the last backup, its error, and Back up now. */
function Backups({ folder, onFolder, folderErr }: { folder: string; onFolder: (v: string) => void; folderErr: string | null }) {
  const [s, setS] = useState<BackupStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const backUp = useAction();
  useEffect(() => {
    api.backups().then(setS).catch((e) => setErr(errorText(e)));
  }, []);
  const now = () => {
    setErr(null);
    backUp.run(async () => {
      try {
        setS(await api.backUpNow());
      } catch (e) {
        // The failure is recorded: show the last good backup next to it.
        api.backups().then(setS).catch(() => {});
        throw e;
      }
    });
  };
  const last = s?.last;
  return (
    <SettingsCard
      id="backups"
      title="Backups"
      action={
        <ActionButton type="button" variant="secondary" className="shrink-0" pending={backUp.busy} pendingLabel="Backing up…" onClick={now}>
          Back up now
        </ActionButton>
      }
      error={backUp.error ?? err ?? (s?.error ? `The last backup failed ${relTime(s.error.at)}: ${s.error.message}` : null)}
      loading={!s && !err && !backUp.error}
    >
        <div className="flex flex-wrap items-center gap-3">
          <p className="min-w-0 flex-1 text-[13px] text-fg">
            {!s ? " " : last ? (
              <span title={last.path}>
                Last backup {relTime(last.at)} · {fmtDateTime(last.at)} · {(last.bytes / 1e6).toFixed(1)} MB
              </span>
            ) : "No backup yet."}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" className={chip(folder === "")} aria-pressed={folder === ""} onClick={() => onFolder("")}>This Mac</button>
          {s?.icloud && <button type="button" className={chip(folder === s.icloud)} aria-pressed={folder === s.icloud} onClick={() => onFolder(s.icloud!)}>iCloud Drive</button>}
        </div>
        {s?.icloud && folder === s.icloud && (
          <Callout tone="warn" icon={<TriangleAlert />}>
            Backups in iCloud Drive leave this Mac: all your chats and your GitHub token are then stored by Apple. Save to use it.
          </Callout>
        )}
        <Field label="Backups folder" hint="Loft backs up its database and settings once a day." help="Backups are owner-only; Loft keeps 7 daily and 4 weekly ones. Point the folder at an external disk to survive this Mac's disk. A cloud folder like iCloud Drive sends your chats and GitHub token off this Mac. To restore, see docs/backups.md.">
          {(id) => <input id={id} className="input font-mono" placeholder={s?.dir ?? ""} value={folder} onChange={(e) => onFolder(e.target.value)} />}
        </Field>
        {folderErr && <p className="text-[12.5px] text-bad [overflow-wrap:anywhere]">{folderErr}</p>}
    </SettingsCard>
  );
}

/** Disk: the free space, and Clean. Check space lists every agent's workspace items in one table to tick and
 * delete, and logs each removal; the last run's log, the daily sweep's included, stays here. */
function Workspaces({ idleHours, onIdleHours }: { idleHours: number; onIdleHours: (v: number) => void }) {
  const [s, setS] = useState<WorkspaceSizes | null>(null);
  const [devices, setDevices] = useState<AgentDevice[] | null>(null);
  // The log on screen: live while cleaning, then the summary.
  const [shown, setShown] = useState<CleanLog | null>(null);
  const [last, setLast] = useState<CleanLog | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const card = useRef<HTMLDivElement>(null);
  const load = () => Promise.all([api.workspaces().then(setS), api.cleanLog().then(setLast), api.agentDevices().then((d) => setDevices(d.devices))]);
  useEffect(() => {
    load().then(() => {
      // Needs you links here when the disk is almost full.
      if (window.location.hash === "#workspaces") setTimeout(() => card.current?.scrollIntoView({ block: "start" }));
    }).catch((e) => setErr(errorText(e)));
  }, []);
  const name = (id: string) => s?.agents.find((a) => a.id === id)?.name ?? `Deleted agent ${id.slice(0, 8)}`;
  const low = s?.free_bytes != null && s.free_bytes < s.low_disk_bytes;
  return (
    <div ref={card} id="workspaces">
      <SettingsCard title="Disk">
        <div className="flex flex-wrap items-center gap-3">
          <p className={cx("min-w-0 flex-1 text-[13px]", low ? "text-warn" : "text-fg")}>
            {!s ? " " : s.free_bytes == null ? "Free space unknown." : `${fmtBytes(s.free_bytes)} free${low ? " · disk almost full" : ""}`}
          </p>
          <Link to="/settings/clean" className="btn shrink-0">
            Check space
          </Link>
        </div>
        {s && (
          <p className="text-[12.5px] text-fg-2">
            {s.sccache.installed
              ? `Rust build cache (sccache, shared by agents): ${fmtBytes(s.sccache.bytes)} of ${fmtBytes(s.sccache.limit_bytes)}`
              : "Rust build cache: sccache isn't installed. Install it with brew install sccache and agents share compiled Rust dependencies."}
          </p>
        )}
        {err && <Callout tone="bad" icon={<TriangleAlert />}>{err}</Callout>}
        {last?.at && (
          <p className="flex flex-wrap items-center gap-x-2 text-[12.5px] text-fg-2">
            <span>
              {last.title || last.what} {relTime(last.at)} · freed {fmtBytes(last.freed_bytes)} from {last.removed} item{last.removed === 1 ? "" : "s"}
              {last.failed > 0 && <span className="text-bad"> · {last.failed} failed</span>}
            </span>
            <button type="button" className="text-accent hover:underline" onClick={() => setShown(last)}>
              View log
            </button>
          </p>
        )}
        {devices && devices.length > 0 && (
          <div className="space-y-1.5">
            <h4 className="text-[12px] font-medium uppercase tracking-wide text-fg-3">Simulator devices</h4>
            <ul className="space-y-1.5 text-[13px]">
              {devices.map((d) => (
                <li key={d.udid} className="flex items-center gap-3">
                  <span className="min-w-0 flex-1 truncate text-fg-2" title={d.udid}>{d.agent ? d.agent.name : `Orphaned · ${d.name}`}</span>
                  <span className={cx("shrink-0 text-[12px]", d.state === "Booted" ? "text-good" : "text-fg-3")}>{d.state === "Booted" ? "Booted" : "Shut down"}</span>
                  <span className="w-16 shrink-0 text-right tabular-nums text-fg">{fmtBytes(d.bytes)}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
        <Field label="Idle timeout (hours)" hint="After this long with no run, Loft stops what the agent left running." help="That means processes like a dev server, and the agent's Simulator device is shut down. Devices left under an agent's old name, or of a deleted agent, are deleted. Your own devices are never touched.">
          {(id) => (
            <input
              id={id}
              className="input w-24"
              type="number"
              min={1}
              value={idleHours}
              onChange={(e) => onIdleHours(Math.max(1, Math.round(Number(e.target.value)) || 1))}
            />
          )}
        </Field>
        <p className="text-[12px] text-fg-3">
          Check space lists what's in every agent's workspace; Delete selected removes what you tick. A clone, a main clone too, can be ticked once git says everything in it is pushed (the agent clones it again); notes and uploads stay unless you tick them. Loft also removes build output untouched for 3 days, daily, a ticket's scratch folder once the ticket is closed, and other scratch and test output in notes after 7 days.
        </p>
      </SettingsCard>
      {shown && <CleanLogView log={shown} name={name} onClose={() => setShown(null)} />}
    </div>
  );
}

/** Groups items by agent, in the order they come. */
function byAgent<T extends { agent: string }>(items: T[]): [string, T[]][] {
  const m = new Map<string, T[]>();
  for (const i of items) m.set(i.agent, [...(m.get(i.agent) ?? []), i]);
  return [...m];
}

/** Copies the equivalent `rm -rf` commands, by agent. */
export function CopyCommands({ items, name }: { items: { agent: string; command: string }[]; name: (id: string) => string }) {
  const [copied, setCopied] = useState(false);
  const text = byAgent(items)
    .map(([agent, xs]) => [`# ${oneLine(name(agent))}`, ...xs.map((x) => x.command)].join("\n"))
    .join("\n\n");
  return (
    <button
      type="button"
      className="btn-ghost text-[12.5px]"
      disabled={items.length === 0}
      onClick={() => navigator.clipboard.writeText(text + "\n").then(() => setCopied(true))}
    >
      {copied ? "Copied" : "Copy commands"}
    </button>
  );
}

/** Says the commands shown are what Loft's removal amounts to, not what it runs. */
export const EQUIVALENT = "Commands are the equivalent of each removal; Loft removes the paths itself.";

function Skipped({ skipped, name }: { skipped: CleanSkip[]; name: (id: string) => string }) {
  if (skipped.length === 0) return null;
  return (
    <div className="space-y-1">
      <p className="eyebrow">Skipped</p>
      <ul className="space-y-0.5 text-[12.5px] text-fg-3">
        {skipped.map((k, i) => (
          <li key={i} className="flex gap-2">
            <span className="min-w-0 flex-1 truncate">
              {name(k.agent)}
              {k.rel && <span className="font-mono"> / {k.rel}</span>}
            </span>
            <span className="shrink-0">{k.reason}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** A clean's log: each removal (or its error) as it happens, with the running total, then the summary. */
export function CleanLogView({ log, name, onClose }: { log: CleanLog; name: (id: string) => string; onClose: () => void }) {
  return (
    <Dialog title={log.running ? "Cleaning" : "Clean log"} size="xl" onClose={log.running ? undefined : onClose} className="flex flex-col gap-4 p-5">
      <div>
        <h3 className="text-[15px] font-medium">{log.running ? log.title || "Cleaning…" : `${log.title || log.what || "Clean"} ${log.at ? relTime(log.at) : ""}`}</h3>
        <p className="mt-1 text-[13px] text-fg-2">
          {log.running && log.lines.length === 0 ? "Removing…" : `Freed ${fmtBytes(log.freed_bytes)} · ${log.removed} removed`} · {log.skipped.length} skipped
          {log.failed > 0 && <span className="text-bad"> · {log.failed} failed</span>}
        </p>
      </div>
      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto">
        {byAgent(log.lines).map(([agent, lines]) => (
          <div key={agent} className="space-y-0.5">
            <p className="text-[13px] font-medium">{name(agent)}</p>
            <ul className="space-y-0.5 font-mono text-[12px]">
              {lines.map((l) => (
                <li key={l.path} className="break-all">
                  <span className={l.error ? "text-bad" : "text-fg-2"}>{cleanLineText(l)}</span>
                  {l.command && <span className="block text-[11.5px] text-fg-3">{l.command}</span>}
                </li>
              ))}
            </ul>
          </div>
        ))}
        {!log.running && log.lines.length === 0 && <p className="text-[13px] text-fg-3">Nothing was removed.</p>}
        <Skipped skipped={log.skipped} name={name} />
      </div>
      {log.lines.some((l) => l.command) && <p className="text-[12px] text-fg-3">{EQUIVALENT}</p>}
      <div className="flex justify-end gap-2">
        <CopyCommands items={log.lines.filter((l) => l.command)} name={name} />
        <span className="flex-1" />
        <button className="btn-primary" disabled={log.running} onClick={onClose}>
          {log.running ? "Cleaning…" : "Done"}
        </button>
      </div>
    </Dialog>
  );
}

/** The All-time spend: every run Loft has counted, which the home line no longer shows. */
function Spend() {
  const [all, setAll] = useState<AgentsSummary["all_time"] | null>(null);
  useEffect(() => {
    api.agentsSummary().then((r) => setAll(r.all_time)).catch(() => {});
  }, []);
  return (
    <SettingsCard id="spend" title="Spend">
      <p className="text-[13px] text-fg" title="Every run since Loft started counting, deleted agents and cleared chats included">
        {all ? `All time: ${all.runs.toLocaleString("en-US")} runs · est. ${exactDollars(all.cost_usd)}` : " "}
      </p>
      <p className="text-[11.5px] text-fg-3">Estimated at API list prices.</p>
    </SettingsCard>
  );
}

/** The keyboard shortcuts: ⌘K and ⌘J can be recorded anew or reset, kept in this browser and used at once (no Save needed); the rest are fixed. */
function ShortcutSettings() {
  const offers = useOffered();
  const [combos, setCombos] = useState(loadCombos);
  const [listening, setListening] = useState<Rebindable | null>(null);
  const [hint, setHint] = useState<string | null>(null);

  const use = (id: Rebindable, c: string) => {
    saveCombo(id, c);
    setCombos(loadCombos());
    setHint(null);
    window.dispatchEvent(new Event(SHORTCUTS_CHANGED));
  };

  useEffect(() => {
    if (!listening) return;
    recording.on = true;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.metaKey) return setListening(null);
      if (/^(Shift|Control|Alt|Meta|OS)$/.test(e.key)) return;
      const c = comboFromEvent(e);
      if (!c) return setHint("Hold ⌘, Ctrl or Alt with the key.");
      if (isReserved(c)) return setHint(`${formatCombo(c)} is the browser's (copy, paste, close tab…). Pick another.`);
      const other = REBINDABLE.find((r) => r.id !== listening)!;
      if (formatCombo(c) === formatCombo(loadCombos()[other.id])) return setHint(`${formatCombo(c)} already does “${other.label}”. Pick another.`);
      use(listening, normalizeCombo(c));
      setListening(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      recording.on = false;
      window.removeEventListener("keydown", onKey, true);
    };
  }, [listening]);

  return (
    <div className="space-y-3">
      {REBINDABLE.map((r) => (
        <div key={r.id} className="flex flex-wrap items-center gap-2">
          <span className="min-w-0 flex-1 basis-40 text-[13px] text-fg-2">{r.label}</span>
          <kbd className="chip min-w-12 justify-center font-mono text-[12px]">{listening === r.id ? "Press keys…" : formatCombo(combos[r.id])}</kbd>
          <button type="button" className="btn" aria-label={listening === r.id ? "Cancel" : `Record ${r.label}`} onClick={() => { setHint(null); setListening((l) => (l === r.id ? null : r.id)); }}>
            {listening === r.id ? "Cancel" : "Record"}
          </button>
          {combos[r.id] !== r.default && listening !== r.id && (
            <button type="button" className="btn-ghost" onClick={() => use(r.id, r.default)}>
              Reset to {formatCombo(r.default)}
            </button>
          )}
        </div>
      ))}
      <span className="block text-[11.5px] text-fg-3">
        {hint ?? "These two work from any page, even while typing. Press Record, then the new keys; they need ⌘, Ctrl or Alt. Kept in this browser."}
      </span>
      <ul className="space-y-2 border-t border-line pt-3">
        {fixedShortcuts(offers).map((f) => (
          <li key={f.keys} className="flex items-center justify-between gap-3 text-[13px]">
            <span className="min-w-0 text-fg-2">{f.label}</span>
            <Keys keys={f.keys} />
          </li>
        ))}
      </ul>
      <span className="block text-[11.5px] text-fg-3">Fixed. They don't fire while you type in a text field.</span>
    </div>
  );
}

/** The cards, in page order: what "On this page" and "Jump to…" list. Each id is its card's (`/settings#push`). */
const SECTIONS = [
  ["general", "General"], ["backups", "Backups"], ["workspaces", "Disk"], ["github", "GitHub API"], ["spend", "Spend"],
  ["updates", "Updates"], ["agents", "Agents"], ["navigation", "Navigation"], ["shortcuts", "Keyboard shortcuts"],
  ["herdr", "Herdr UI"], ["push", "Push notifications"], ["https", "Https on the tailnet"], ["voice", "Voice on this Mac"],
  ["claude-login", "Claude login"], ["mac-access", "Mac access"],
] as const;

/** The cards that are Mac-only features: listed only where the server offers the feature, like the cards themselves. */
const MAC_ONLY_CARDS: Partial<Record<(typeof SECTIONS)[number][0], MacOnly>> = { navigation: "simulator", herdr: "herdr", voice: "voice", "mac-access": "mac_access" };

const goTo = (id: string) => {
  document.getElementById(id)?.scrollIntoView({ block: "start" });
  history.replaceState(null, "", `#${id}`);
};

/** The card "On this page" highlights, from the page's scroll. The page's scroller isn't there yet while Settings loads,
 * and scroll events don't bubble: listen on the document, in the capture phase. `ready`: the cards are on the page.
 * A card clicked, or named by the URL's #hash, stays highlighted at the bottom of the page (where the last cards can't
 * reach the top) until the user scrolls by hand. */
function useCardInView(ready: boolean) {
  const [current, setCurrent] = useState<string>(SECTIONS[0][0]);
  // `top`: where the scroller was when a click pinned it; null for a #hash pin, which the page keeps scrolling to
  // while the cards above it grow, so only the user's own input drops it.
  const pinned = useRef<Pin | null>(null);
  const scroller = () => document.querySelector<HTMLElement>("[data-settings-scroll]");
  useEffect(() => {
    if (!ready) return;
    const hash = window.location.hash.slice(1);
    if (SECTIONS.some(([id]) => id === hash)) pinned.current = { id: hash, top: null };
    const update = () => {
      const el = scroller();
      if (!el) return;
      const tops: Record<string, number> = {};
      for (const [id] of SECTIONS) {
        const card = document.getElementById(id);
        if (card) tops[id] = card.getBoundingClientRect().top;
      }
      const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 2;
      pinned.current = keepPin(pinned.current, el.scrollTop);
      setCurrent(cardInView(SECTIONS.map(([id]) => id), tops, el.getBoundingClientRect().top + 96, atBottom, pinned.current?.id ?? null));
    };
    const byHand = () => (pinned.current = null);
    document.addEventListener("scroll", update, { capture: true, passive: true });
    for (const e of ["wheel", "touchmove", "keydown", "pointerdown"]) document.addEventListener(e, byHand, { passive: true });
    update();
    return () => {
      document.removeEventListener("scroll", update, { capture: true });
      for (const e of ["wheel", "touchmove", "keydown", "pointerdown"]) document.removeEventListener(e, byHand);
    };
  }, [ready]);
  /** Call after scrolling to the card: pins it at the scroller's new position. */
  const pin = (id: string) => {
    pinned.current = { id, top: scroller()?.scrollTop ?? null };
    setCurrent(id);
  };
  return { current, pin };
}

export default function SettingsPage() {
  const offers = useOffered();
  const sections = SECTIONS.filter(([id]) => {
    const feature = MAC_ONLY_CARDS[id];
    return !feature || offers[feature];
  });
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  // Write-only: undefined keeps the saved token, "" clears it.
  const [token, setToken] = useState<string | undefined>(undefined);
  const save = useAction();
  const [err, setErr] = useState<string | null>(null);
  const [folderErr, setFolderErr] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const restartAsk = useAction();

  const load = useCallback(() => api.settings().then((v) => {
    setView(v);
    return v;
  }), []);
  useEffect(() => {
    load().then((v) => setDraft(v.settings)).catch((e) => setErr(errorText(e)));
  }, [load]);
  // Keep the running-agents count fresh; it gates the restart.
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "agent_updated") load().catch(() => {});
    }, [load]),
  );

  const { current: inView, pin } = useCardInView(!!view && !!draft);

  if (!view || !draft) {
    return <div className="mx-auto max-w-3xl px-5 py-8">{err ? <ErrorLine error={err} /> : <div className="skeleton h-96 rounded-2xl" />}</div>;
  }

  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => setDraft({ ...draft, [k]: v });
  const dirty = JSON.stringify(draft) !== JSON.stringify(view.settings) || token !== undefined;
  const restartFields = view.restart_fields as string[];
  const dirtyRestart = restartFields.filter((f) => JSON.stringify(draft[f as keyof Settings]) !== JSON.stringify(view.settings[f as keyof Settings]));

  const saveAll = () => {
    setErr(null);
    setFolderErr(null);
    save.run(async () => {
      // The backups folder goes on its own, after the rest: a folder the server refuses fails only that field.
      let v = await api.saveSettings({ ...draft, github_token: token, backup_dir: undefined });
      if (draft.backup_dir !== v.settings.backup_dir) {
        try {
          // No github_token: the view's is blank, and blank would clear it.
          v = await api.saveSettings({ ...v.settings, github_token: undefined, backup_dir: draft.backup_dir });
        } catch (e) {
          setFolderErr(errorText(e));
        }
      }
      setView(v);
      setDraft({ ...v.settings, backup_dir: draft.backup_dir });
      setToken(undefined);
      window.dispatchEvent(new CustomEvent(SETTINGS_SAVED, { detail: v.settings }));
    });
  };

  const restart = async () => {
    setErr(null);
    const sent = await restartAsk.run(async () => (await api.restart(), true));
    setConfirming(false);
    if (!sent) return;
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
    <div className="h-full overflow-y-auto" data-settings-scroll>
      <div className="mx-auto max-w-3xl px-5 py-8 pb-28 lg:grid lg:max-w-5xl lg:grid-cols-[200px_minmax(0,1fr)] lg:gap-8">
        <nav aria-label="On this page" className="hidden lg:block">
          <div className="sticky top-6 space-y-0.5">
            <span className="eyebrow block px-2.5 pb-2">On this page</span>
            {sections.map(([id, title]) => (
              <a
                key={id}
                href={`#${id}`}
                aria-current={inView === id ? "true" : undefined}
                onClick={(e) => (e.preventDefault(), goTo(id), pin(id))}
                className={cx("block rounded-lg px-2.5 py-1 text-[13px] transition-colors", inView === id ? "bg-ink-800 text-fg" : "text-fg-2 hover:text-fg")}
              >
                {title}
              </a>
            ))}
          </div>
        </nav>
        <div className="min-w-0 space-y-5">
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="text-[22px] font-medium tracking-tight">Settings</h1>
          <select
            aria-label="Jump to"
            className="input ml-auto w-auto max-w-[60%] py-1.5 text-[13px] lg:hidden"
            value=""
            onChange={(e) => e.target.value && (goTo(e.target.value), pin(e.target.value))}
          >
            <option value="">Jump to…</option>
            {sections.map(([id, title]) => <option key={id} value={id}>{title}</option>)}
          </select>
        </div>

        {(view.pending_restart.length > 0 || restarting) && (
          <Callout
            tone="warn"
            icon={<RotateCw className={cx(restarting && "animate-spin")} />}
            action={!restarting && (
              <button className="btn shrink-0" disabled={running > 0} onClick={() => setConfirming(true)}>
                Restart now
              </button>
            )}
          >
            {restarting ? (
              "Restarting loft…"
            ) : (
              <>
                <span className="text-fg">Restart to apply</span> {view.pending_restart.map((f) => LABELS[f] ?? f).join(", ")}.
                {running > 0 && <> Waiting for {running} running agent{running > 1 ? "s" : ""} to finish.</>}
              </>
            )}
          </Callout>
        )}

        <SettingsCard id="general" title="General">
          <Field label="Projects folder" hint="Where your git repos live. Type a path or a folder's name to see suggestions." restart>
            {(id) => <FolderField id={id} value={draft.projects_dir} onChange={(v) => set("projects_dir", v)} />}
          </Field>
          <Field label="Claude command" hint="What agents run.">
            {(id) => <input id={id} className="input font-mono" value={draft.claude_bin} onChange={(e) => set("claude_bin", e.target.value)} />}
          </Field>
          <Field label="SSH host" hint="Leave empty if you only use Loft on this Mac." help="How other machines reach this one, e.g. loft. /code from them then opens VS Code over Remote-SSH (VS Code only).">
            {(id) => <input id={id} className="input font-mono" placeholder="none" value={draft.ssh_host} onChange={(e) => set("ssh_host", e.target.value)} />}
          </Field>
          <Field label="GitHub token" hint={view.github_token_set ? "A token is saved. Type a new one to replace it." : "Needed for private repos and merging PRs."} help={view.github_token_set ? undefined : "Without one, Loft uses gh auth token."}>
            {(id) => (
              <div className="flex gap-2">
                <input
                  id={id}
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
            )}
          </Field>
        </SettingsCard>

        <Backups folder={draft.backup_dir ?? ""} onFolder={(v) => set("backup_dir", v)} folderErr={folderErr} />

        <Workspaces idleHours={draft.idle_timeout_hours ?? 4} onIdleHours={(v) => set("idle_timeout_hours", v)} />

        <GithubUsage />
        <Spend />

        <SettingsCard id="updates" title="Updates">
          <Toggle
            on={draft.auto_update}
            onChange={(v) => set("auto_update", v)}
            label="Update automatically"
            hint="Checks for new commits every minute. Installs once no agent is running, then restarts."
          />
        </SettingsCard>

        <SettingsCard id="agents" title="Agents">
          <Toggle
            on={draft.messaging_rules}
            onChange={(v) => set("messaging_rules", v)}
            label="Messaging rules"
            hint="Tells every agent to message other agents only with a finding, report, verdict or request, never thanks or waiting notes."
          />
        </SettingsCard>

        {offers.simulator && (
          <SettingsCard id="navigation" title="Navigation">
            <Toggle
              on={draft.show_simulator}
              onChange={(v) => set("show_simulator", v)}
              label="Show Simulator"
              hint="Adds a tab to view and control a booted iOS Simulator on this Mac."
            />
          </SettingsCard>
        )}

        <SettingsCard id="shortcuts" title="Keyboard shortcuts">
          <ShortcutSettings />
        </SettingsCard>

        {offers.herdr && <HerdrSetup />}

        <PushSettings />

        <HttpsAccess />
        {offers.voice && <VoiceOnThisMac />}

        <ClaudeLoginSection />

        {offers.mac_access && <MacAccess />}

        {(dirty || err || save.error || restartAsk.error) && (
          <FloatingBar at="bottom">
            {err || save.error || restartAsk.error ? (
              <ErrorLine error={save.error ?? restartAsk.error ?? err} />
            ) : (
              <span className="text-fg-2">
                Unsaved changes{dirtyRestart.length > 0 && <> · {dirtyRestart.map((f) => LABELS[f] ?? f).join(", ")} needs a restart</>}
              </span>
            )}
            {dirty && (
              <>
                <button className="btn-ghost" onClick={() => { setDraft(view.settings); setToken(undefined); setErr(null); save.setError(null); }}>
                  Discard
                </button>
                <ActionButton pending={save.busy} pendingLabel="Saving…" onClick={saveAll}>
                  Save
                </ActionButton>
              </>
            )}
          </FloatingBar>
        )}
        </div>

        {confirming && (
          <ConfirmDialog
            title="Restart loft?"
            body={
              <>
                This applies {view.pending_restart.map((f) => LABELS[f] ?? f).join(", ")}. It takes a few seconds; open tabs reconnect on their own.
                {running > 0 && <span className="mt-2 block text-warn">{running} agent{running > 1 ? "s are" : " is"} running. Restart once they finish.</span>}
              </>
            }
            confirmLabel="Restart"
            pendingLabel="Restarting…"
            pending={restartAsk.busy}
            disabled={running > 0}
            onConfirm={restart}
            onClose={() => setConfirming(false)}
          />
        )}
      </div>
    </div>
  );
}
