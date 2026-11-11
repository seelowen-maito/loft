import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, ChevronRight, Loader2, RotateCw } from "lucide-react";
import { api, type AgentDevice, type AgentsSummary, type ApprovalMode, type BackupStatus, type CleanItem, type CleanLog, type CleanSkip, type GitCheck, type GithubBudget, type LeftProcess, type WorkspaceSizes, type Settings, type SettingsView } from "../lib/api";
import { exactDollars } from "../lib/usage";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { PushSettings } from "./PushSettings";
import { PairedClients } from "./PairedClients";
import { MacAccess } from "./MacAccess";
import { HerdrSetup } from "./HerdrSetup";
import { canCheck, cleanLineText, cleanStatus, cleanType, cx, fmtBytes, gitCheckText, oneLine, sortClean, timeAgo, type CleanSort } from "../lib/format";
import { comboFromEvent, FIXED, formatCombo, isReserved, loadCombos, normalizeCombo, REBINDABLE, recording, saveCombo, SHORTCUTS_CHANGED, type Rebindable } from "../lib/shortcuts";
import { Keys } from "../components/Shortcuts";

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

const chip = (on: boolean) => cx("chip text-[12px]", on && "border-accent/40 text-accent");

/** Backups of Loft's database: the folder (saved with the other settings), the last backup, its error, and Back up now. */
function Backups({ folder, onFolder, folderErr }: { folder: string; onFolder: (v: string) => void; folderErr: string | null }) {
  const [s, setS] = useState<BackupStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const card = useRef<HTMLDivElement>(null);
  useEffect(() => {
    api.backups().then((v) => {
      setS(v);
      // Needs you links here when backups fail.
      if (window.location.hash === "#backups") setTimeout(() => card.current?.scrollIntoView({ block: "start" }));
    }).catch((e) => setErr((e as Error).message));
  }, []);
  const now = async () => {
    setBusy(true);
    setErr(null);
    try {
      setS(await api.backUpNow());
    } catch (e) {
      setErr((e as Error).message);
      // The failure is recorded: show the last good backup next to it.
      api.backups().then(setS).catch(() => {});
    } finally {
      setBusy(false);
    }
  };
  const last = s?.last;
  return (
    <div ref={card} id="backups">
      <Section title="Backups">
        <div className="flex flex-wrap items-center gap-3">
          <p className="min-w-0 flex-1 text-[13px] text-fg">
            {!s ? " " : last ? (
              <span title={last.path}>
                Last backup {timeAgo(last.at)} · {new Date(last.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })} · {(last.bytes / 1e6).toFixed(1)} MB
              </span>
            ) : "No backup yet."}
          </p>
          <button type="button" className="btn shrink-0" disabled={busy} onClick={now}>
            {busy ? "Backing up…" : "Back up now"}
          </button>
        </div>
        {(s?.error || err) && (
          <p className="rounded-xl border border-bad/25 bg-bad/[0.06] px-3 py-2 text-[12.5px] text-bad">
            {err ?? `The last backup failed ${timeAgo(s!.error!.at)}: ${s!.error!.message}`}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <button type="button" className={chip(folder === "")} aria-pressed={folder === ""} onClick={() => onFolder("")}>This Mac</button>
          {s?.icloud && <button type="button" className={chip(folder === s.icloud)} aria-pressed={folder === s.icloud} onClick={() => onFolder(s.icloud!)}>iCloud Drive</button>}
        </div>
        {s?.icloud && folder === s.icloud && (
          <p className="rounded-xl border border-warn/30 bg-warn/10 px-3 py-2 text-[12.5px] text-warn">
            Backups in iCloud Drive leave this Mac: all your chats and your GitHub token are then stored by Apple. Save to use it.
          </p>
        )}
        <Field label="Backups folder" hint="Loft backs up its database and settings once a day, owner-only, and keeps 7 daily and 4 weekly backups. Point it at an external disk to survive this Mac's disk. A cloud folder like iCloud Drive sends your chats and GitHub token off this Mac. To restore, see docs/backups.md.">
          <input className="input font-mono" placeholder={s?.dir ?? "~/Library/Application Support/Loft/backups"} value={folder} onChange={(e) => onFolder(e.target.value)} />
        </Field>
        {folderErr && <p className="text-[12.5px] text-bad">{folderErr}</p>}
      </Section>
    </div>
  );
}

/** Disk: the free space, and Clean. Check space lists every agent's workspace items in one table to tick and
 * delete, and logs each removal; the last run's log, the daily sweep's included, stays here. */
function Workspaces({ idleHours, onIdleHours }: { idleHours: number; onIdleHours: (v: number) => void }) {
  const [s, setS] = useState<WorkspaceSizes | null>(null);
  const [devices, setDevices] = useState<AgentDevice[] | null>(null);
  // What idle agents left running, by agent id, and the agent whose Stop them is working.
  const [left, setLeft] = useState<Record<string, LeftProcess[]>>({});
  const [stopping, setStopping] = useState<string | null>(null);
  // The table: every agent's items and the paths ticked; null until Check space.
  const [table, setTable] = useState<Picker | null>(null);
  // The log on screen: live while cleaning, then the summary.
  const [shown, setShown] = useState<CleanLog | null>(null);
  const [last, setLast] = useState<CleanLog | null>(null);
  const [checking, setChecking] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const card = useRef<HTMLDivElement>(null);
  const load = () => Promise.all([api.workspaces().then(setS), api.cleanLog().then(setLast), api.agentDevices().then((d) => setDevices(d.devices)), api.leftRunning().then(setLeft)]);
  useEffect(() => {
    load().then(() => {
      // Needs you links here when the disk is almost full.
      if (window.location.hash === "#workspaces") setTimeout(() => card.current?.scrollIntoView({ block: "start" }));
    }).catch((e) => setErr((e as Error).message));
  }, []);
  const name = (id: string) => s?.agents.find((a) => a.id === id)?.name ?? `Deleted agent ${id.slice(0, 8)}`;
  const checkSpace = async () => {
    setChecking(true);
    setErr(null);
    try {
      const [{ items }, l] = await Promise.all([api.cleanPreview(), api.leftRunning()]);
      setLeft(l);
      setTable({ items, ticked: new Set() });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setChecking(false);
    }
  };
  const stopThem = async (id: string) => {
    setStopping(id);
    setErr(null);
    try {
      await api.stopLeftRunning(id);
      await load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setStopping(null);
    }
  };
  const confirm = async () => {
    if (!table) return;
    const paths = [...table.ticked];
    setShown({ at: null, what: "", title: "Cleaning…", running: true, lines: [], skipped: [], freed_bytes: 0, removed: 0, failed: 0 });
    // The log fills in as Loft removes each folder.
    let done = false;
    const poll = setInterval(() => api.cleanLog().then((l) => l.running && !done && setShown(l)).catch(() => {}), 500);
    try {
      const l = await api.clean(undefined, paths);
      done = true;
      setShown(l);
      setLast(l);
      // The table shows what is left.
      setTable(null);
      await Promise.all([load(), checkSpace()]);
    } catch (e) {
      done = true;
      setShown(null);
      setErr((e as Error).message);
    } finally {
      clearInterval(poll);
    }
  };
  const low = s?.free_bytes != null && s.free_bytes < s.low_disk_bytes;
  const busy = !s || checking || shown?.running === true;
  return (
    <div ref={card} id="workspaces">
      <Section title="Disk">
        <div className="flex flex-wrap items-center gap-3">
          <p className={cx("min-w-0 flex-1 text-[13px]", low ? "text-warn" : "text-fg")}>
            {!s ? " " : s.free_bytes == null ? "Free space unknown." : `${fmtBytes(s.free_bytes)} free${low ? " · disk almost full" : ""}`}
          </p>
          <button type="button" className="btn shrink-0" disabled={busy} onClick={checkSpace}>
            {checking ? "Checking…" : "Check space"}
          </button>
        </div>
        {err && <p className="rounded-xl border border-bad/25 bg-bad/[0.06] px-3 py-2 text-[12.5px] text-bad">{err}</p>}
        {checking && !table && (
          <p className="flex items-center gap-2 py-2 text-[13px] text-fg-2">
            <Loader2 className="size-4 animate-spin" /> Measuring workspaces…
          </p>
        )}
        {table && <CleanTable table={table} name={name} left={left} stopping={stopping} busy={busy} onChange={setTable} onStop={stopThem} onConfirm={confirm} />}
        {last?.at && (
          <p className="flex flex-wrap items-center gap-x-2 text-[12.5px] text-fg-2">
            <span>
              {last.title || last.what} {timeAgo(last.at)} · freed {fmtBytes(last.freed_bytes)} from {last.removed} item{last.removed === 1 ? "" : "s"}
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
        <Field label="Idle timeout (hours)" hint="After this long with no run, Loft stops the processes the agent left running (like a dev server) and shuts its Simulator device down. Devices left under an agent's old name, or of a deleted agent, are deleted. Your own devices are never touched.">
          <input
            className="input w-24"
            type="number"
            min={1}
            value={idleHours}
            onChange={(e) => onIdleHours(Math.max(1, Math.round(Number(e.target.value)) || 1))}
          />
        </Field>
        <p className="text-[12px] text-fg-3">
          Check space lists what's in every agent's workspace; Delete selected removes what you tick. A clone, a main clone too, can be ticked once git says everything in it is pushed (the agent clones it again); notes and uploads stay unless you tick them. Loft also removes build output untouched for 3 days, daily.
        </p>
      </Section>
      {shown && <CleanLogView log={shown} name={name} onClose={() => setShown(null)} />}
    </div>
  );
}

function Modal({ onClose, children }: { onClose?: () => void; children: React.ReactNode }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={onClose}>
      <div className="panel flex max-h-[85vh] w-full max-w-2xl flex-col gap-4 bg-ink-900 p-5" onClick={(e) => e.stopPropagation()}>
        {children}
      </div>
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
function CopyCommands({ items, name }: { items: { agent: string; command: string }[]; name: (id: string) => string }) {
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
const EQUIVALENT = "Commands are the equivalent of each removal; Loft removes the paths itself.";

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

/** The Clean table: the items listed and the paths ticked. */
type Picker = { items: CleanItem[]; ticked: Set<string> };

/** Every agent's top-level items in one table: agent, item, type, size, last change and status. Sort by size, agent or
 * last change; tick what to delete. A main clone opens up to its folders, loaded one level at a time. */
function CleanTable({ table, name, left, stopping, busy, onChange, onStop, onConfirm }: {
  table: Picker;
  name: (id: string) => string;
  left: Record<string, LeftProcess[]>;
  stopping: string | null;
  busy: boolean;
  onChange: (p: Picker) => void;
  onStop: (agent: string) => void;
  onConfirm: () => void;
}) {
  // Folders listed inside opened items, by path; null while loading.
  const [inside, setInside] = useState<Map<string, CleanItem[] | null>>(new Map());
  const [err, setErr] = useState<string | null>(null);
  // Check results by path; null while checking. Git runs only when the user clicks, and nothing gets ticked.
  const [checks, setChecks] = useState<Map<string, GitCheck | null>>(new Map());
  const [sort, setSort] = useState<CleanSort>({ by: "size", desc: true });
  const { items, ticked } = table;
  const known = [...items, ...[...inside.values()].flatMap((xs) => xs ?? [])];
  const chosen = known.filter((i) => ticked.has(i.path));
  const total = chosen.reduce((n, i) => n + i.bytes, 0);
  const toggle = (path: string, on: boolean) => {
    const t = new Set(ticked);
    if (on) t.add(path);
    else t.delete(path);
    onChange({ ...table, ticked: t });
  };
  const open = async (i: CleanItem) => {
    const m = new Map(inside);
    if (m.has(i.path)) {
      m.delete(i.path);
      return setInside(m);
    }
    setInside(new Map(m).set(i.path, null));
    try {
      const { items } = await api.cleanFolders(i.agent, i.rel);
      setInside((cur) => new Map(cur).set(i.path, items));
    } catch (e) {
      setInside((cur) => { const n = new Map(cur); n.delete(i.path); return n; });
      setErr((e as Error).message);
    }
  };
  const check = async (i: CleanItem) => {
    setChecks((cur) => new Map(cur).set(i.path, null));
    let c: GitCheck;
    try {
      c = await api.cleanCheck(i.agent, i.rel);
    } catch (e) {
      c = { state: "unknown", lines: [`Can't check: ${(e as Error).message}`] };
    }
    setChecks((cur) => new Map(cur).set(i.path, c));
  };
  const sortBy = (by: CleanSort["by"]) => setSort((s) => (s.by === by ? { by, desc: !s.desc } : { by, desc: by !== "agent" }));
  const head = (by: CleanSort["by"], label: string, cls = "") => (
    <th className={cx("px-2 py-1.5 font-medium", cls)} aria-sort={sort.by === by ? (sort.desc ? "descending" : "ascending") : "none"}>
      <button type="button" className="inline-flex items-center gap-1 hover:text-fg" onClick={() => sortBy(by)}>
        {label}
        {sort.by === by && (sort.desc ? <ArrowDown className="size-3" /> : <ArrowUp className="size-3" />)}
      </button>
    </th>
  );
  const row = (i: CleanItem, depth: number): React.ReactNode => {
    const sub = inside.get(i.path);
    const checked = checks.get(i.path);
    const on = ticked.has(i.path);
    const status = cleanStatus(i, depth);
    return (
      <Fragment key={i.path}>
        <tr className="border-t border-line align-top">
          <td className="hidden px-2 py-1.5 text-fg-2 min-[400px]:table-cell">{depth === 0 ? name(i.agent) : ""}</td>
          <td className="px-2 py-1.5">
            <div className="flex items-start gap-2" style={{ paddingLeft: depth * 18 }}>
              <input type="checkbox" className="mt-1 shrink-0" aria-label={`Select ${name(i.agent)} ${i.rel}`} checked={on} disabled={status.locked || busy} onChange={(e) => toggle(i.path, e.target.checked)} />
              <span className="min-w-0">
                <span className="flex items-center gap-1">
                  {i.opens && (
                    <button type="button" className="shrink-0 text-fg-3 hover:text-fg" aria-label={sub !== undefined ? `Close ${i.name}` : `Open ${i.name}`} onClick={() => open(i)}>
                      <ChevronRight className={cx("size-4 transition-transform", sub !== undefined && "rotate-90")} />
                    </button>
                  )}
                  <span className="break-all font-mono text-fg">{i.name}</span>
                </span>
                {on && i.warning && !checked && <span className="block text-[11.5px] text-warn">{i.warning}</span>}
                {checked === null && (
                  <span className="flex items-center gap-1.5 text-[11.5px] text-fg-3">
                    <Loader2 className="size-3 animate-spin" /> Checking git…
                  </span>
                )}
                {checked && <span className={cx("block text-[11.5px]", checked.state === "safe" ? "text-good" : checked.state === "unsafe" ? "text-warn" : "text-fg-3")}>{gitCheckText(checked)}</span>}
                {canCheck(i, depth) && checked !== null && (
                  <button type="button" className="text-[11.5px] text-accent hover:underline" onClick={() => check(i)}>
                    {checked ? "Check again" : "Check git"}
                  </button>
                )}
                {i.note && <span className="block text-[11.5px] text-fg-2">{i.note}</span>}
                <span className="block text-[11.5px] text-fg-3 sm:hidden">{depth === 0 && <span className="min-[400px]:hidden">{name(i.agent)} · </span>}{cleanType(i)} · {timeAgo(i.changed)}</span>
              </span>
            </div>
          </td>
          <td className="hidden whitespace-nowrap px-2 py-1.5 text-fg-3 sm:table-cell">{cleanType(i)}</td>
          <td className="whitespace-nowrap px-2 py-1.5 text-right tabular-nums">{fmtBytes(i.bytes)}</td>
          <td className="hidden whitespace-nowrap px-2 py-1.5 text-fg-3 sm:table-cell">{timeAgo(i.changed)}</td>
          <td className={cx("px-2 py-1.5", status.locked ? "text-warn" : "text-good")}>
            {status.text}
            {!status.locked && i.label && <span className="block text-[11.5px] text-fg-3">{i.label}</span>}
          </td>
        </tr>
        {sub === null && (
          <tr><td colSpan={6} className="px-2 py-1 text-[12px] text-fg-3" style={{ paddingLeft: (depth + 1) * 18 + 40 }}><Loader2 className="mr-2 inline size-3.5 animate-spin" />Measuring…</td></tr>
        )}
        {sub && sub.length === 0 && (
          <tr><td colSpan={6} className="px-2 py-1 text-[12px] text-fg-3" style={{ paddingLeft: (depth + 1) * 18 + 40 }}>No folders inside.</td></tr>
        )}
        {sub?.map((c) => row(c, depth + 1))}
      </Fragment>
    );
  };
  // An idle agent's leftover processes sit in the table before its first row.
  const shownAgents = new Set<string>();
  const sorted = sortClean(items, sort, name);
  const processes = (agent: string): React.ReactNode => {
    const ps = left[agent];
    if (shownAgents.has(agent) || !ps?.length || items.find((i) => i.agent === agent)?.blocked === "Working") return null;
    shownAgents.add(agent);
    return (
      <tr key={`ps-${agent}`} className="border-t border-line bg-ink-800/40">
        <td colSpan={6} className="px-2 py-1.5 text-[12.5px]">
         <div className="w-0 min-w-full space-y-1">
          <div className="flex items-center gap-3">
            <span className="min-w-0 flex-1 text-fg-2">
              {name(agent)}: {ps.length} process{ps.length === 1 ? "" : "es"} still running
            </span>
            <button type="button" className="btn shrink-0 px-2.5 py-1 text-[12px]" disabled={stopping !== null || busy} onClick={() => onStop(agent)}>
              {stopping === agent ? "Stopping…" : "Stop them"}
            </button>
          </div>
          <ul className="space-y-0.5 text-fg-3">
            {ps.map((p) => (
              <li key={p.pid} className="flex gap-3">
                <span className="min-w-0 flex-1 truncate font-mono" title={p.command}>{p.command}</span>
                <span className="shrink-0 tabular-nums">{timeAgo(Date.now() / 1000 - p.age_secs).replace(" ago", "")}</span>
              </li>
            ))}
          </ul>
         </div>
        </td>
      </tr>
    );
  };
  return (
    <div className="space-y-2">
      {err && <p className="rounded-xl border border-bad/25 bg-bad/[0.06] px-3 py-2 text-[12.5px] text-bad">{err}</p>}
      {items.length === 0 ? (
        <p className="text-[13px] text-fg-3">Nothing in any workspace.</p>
      ) : (
        <>
        <label className="flex items-center gap-2 text-[12.5px] text-fg-2 sm:hidden">
          Sort by
          <select className="input flex-1" aria-label="Sort by" value={sort.by} onChange={(e) => setSort({ by: e.target.value as CleanSort["by"], desc: e.target.value !== "agent" })}>
            <option value="size">Size</option>
            <option value="agent">Agent</option>
            <option value="changed">Last change</option>
          </select>
        </label>
        <div className="overflow-x-auto rounded-xl border border-line">
          <table className="w-full text-left sm:min-w-[600px] text-[12.5px]">
            <thead className="text-[11.5px] uppercase tracking-wide text-fg-3">
              <tr>
                {head("agent", "Agent", "hidden min-[400px]:table-cell")}
                <th className="px-2 py-1.5 font-medium">Item</th>
                <th className="hidden px-2 py-1.5 font-medium sm:table-cell">Type</th>
                {head("size", "Size", "text-right")}
                {head("changed", "Last change", "hidden sm:table-cell")}
                <th className="px-2 py-1.5 font-medium">Status</th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((i) => (
                <Fragment key={i.path}>
                  {processes(i.agent)}
                  {row(i, 0)}
                </Fragment>
              ))}
            </tbody>
          </table>
        </div>
        </>
      )}
      {items.length > 0 && <p className="text-[12px] text-fg-3">{EQUIVALENT}</p>}
      <div className="flex flex-wrap items-center justify-end gap-2">
        <span className="min-w-0 flex-1 text-[13px] text-fg-2">
          {chosen.length === 0 ? "Nothing selected" : `${chosen.length} selected · ${fmtBytes(total)}`}
        </span>
        <CopyCommands items={chosen} name={name} />
        <button className="btn-primary" disabled={chosen.length === 0 || busy} onClick={onConfirm}>
          Delete selected
        </button>
      </div>
    </div>
  );
}

/** A clean's log: each removal (or its error) as it happens, with the running total, then the summary. */
function CleanLogView({ log, name, onClose }: { log: CleanLog; name: (id: string) => string; onClose: () => void }) {
  return (
    <Modal onClose={log.running ? undefined : onClose}>
      <div>
        <h3 className="text-[15px] font-medium">{log.running ? log.title || "Cleaning…" : `${log.title || log.what || "Clean"} ${log.at ? timeAgo(log.at) : ""}`}</h3>
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
    </Modal>
  );
}

/** The All-time spend: every run Loft has counted, which the home line no longer shows. */
function Spend() {
  const [all, setAll] = useState<AgentsSummary["all_time"] | null>(null);
  useEffect(() => {
    api.agentsSummary().then((r) => setAll(r.all_time)).catch(() => {});
  }, []);
  return (
    <Section title="Spend">
      <p className="text-[13px] text-fg" title="Every run since Loft started counting, deleted agents and cleared chats included">
        {all ? `All time: ${all.runs.toLocaleString("en-US")} runs · est. ${exactDollars(all.cost_usd)}` : " "}
      </p>
      <p className="text-[11.5px] text-fg-3">Estimated at API list prices.</p>
    </Section>
  );
}

/** The keyboard shortcuts: ⌘K and ⌘J can be recorded anew or reset, kept in this browser and used at once (no Save needed); the rest are fixed. */
function ShortcutSettings() {
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
        {FIXED.map((f) => (
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
  const [folderErr, setFolderErr] = useState<string | null>(null);
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
    setFolderErr(null);
    try {
      // The backups folder goes on its own, after the rest: a folder the server refuses fails only that field.
      let v = await api.saveSettings({ ...draft, github_token: token, backup_dir: undefined });
      if (draft.backup_dir !== v.settings.backup_dir) {
        try {
          // No github_token: the view's is blank, and blank would clear it.
          v = await api.saveSettings({ ...v.settings, github_token: undefined, backup_dir: draft.backup_dir });
        } catch (e) {
          setFolderErr((e as Error).message);
        }
      }
      setView(v);
      setDraft({ ...v.settings, backup_dir: draft.backup_dir });
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

        <Backups folder={draft.backup_dir ?? ""} onFolder={(v) => set("backup_dir", v)} folderErr={folderErr} />

        <Workspaces idleHours={draft.idle_timeout_hours ?? 4} onIdleHours={(v) => set("idle_timeout_hours", v)} />

        <GithubUsage />
        <Spend />

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

        <Section title="Keyboard shortcuts">
          <ShortcutSettings />
        </Section>

        <PairedClients />
        <HerdrSetup />

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
