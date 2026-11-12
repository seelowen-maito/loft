import { Fragment, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { ArrowDown, ArrowUp, ChevronRight, Loader2 } from "lucide-react";
import { api, type CleanItem, type CleanLog, type GitCheck, type LeftProcess, type WorkspaceSizes } from "../lib/api";
import { canCheck, cleanState, cleanType, cx, filterClean, fmtBytes, chosenRows, gitCheckText, inBatches, initialTicks, reasonLines, selectable, sortClean, ticksAfter, timeAgo, type CleanFilter, type CleanSort } from "../lib/format";
import { CleanLogView, CopyCommands, EQUIVALENT } from "./Settings";

/** Clean: every agent's workspace items in a full-screen table at `/settings/clean`, to filter, tick and delete. */
export default function CleanPage() {
  const [s, setS] = useState<WorkspaceSizes | null>(null);
  const [left, setLeft] = useState<Record<string, LeftProcess[]>>({});
  const [stopping, setStopping] = useState<string | null>(null);
  const [table, setTable] = useState<Picker | null>(null);
  const [shown, setShown] = useState<CleanLog | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [checking, setChecking] = useState(true);
  const name = (id: string) => s?.agents.find((a) => a.id === id)?.name ?? `Deleted agent ${id.slice(0, 8)}`;
  const checkSpace = async () => {
    setChecking(true);
    setErr(null);
    try {
      const [{ items }, l, sizes] = await Promise.all([api.cleanPreview(), api.leftRunning(), api.workspaces()]);
      setLeft(l);
      setS(sizes);
      setTable({ items, ticked: initialTicks(items) });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setChecking(false);
    }
  };
  useEffect(() => {
    checkSpace();
  }, []);
  const stopThem = async (id: string) => {
    setStopping(id);
    setErr(null);
    try {
      await api.stopLeftRunning(id);
      await checkSpace();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setStopping(null);
    }
  };
  const confirm = async (paths: string[]) => {
    if (!table) return;
    setShown({ at: null, what: "", title: "Cleaning…", running: true, lines: [], skipped: [], freed_bytes: 0, removed: 0, failed: 0 });
    // The log fills in as Loft removes each folder.
    let done = false;
    const poll = setInterval(() => api.cleanLog().then((l) => l.running && !done && setShown(l)).catch(() => {}), 500);
    try {
      const l = await api.clean(undefined, paths);
      done = true;
      setShown(l);
      // The table shows what is left.
      setTable(null);
      await checkSpace();
    } catch (e) {
      done = true;
      setShown(null);
      setErr((e as Error).message);
    } finally {
      clearInterval(poll);
    }
  };
  const tick = (paths: string[], on: boolean) =>
    setTable((t) => {
      if (!t) return t;
      const ticked = new Set(t.ticked);
      for (const p of paths) on ? ticked.add(p) : ticked.delete(p);
      return { ...t, ticked };
    });
  const busy = checking || shown?.running === true;
  return (
    <div className="flex h-full flex-col">
      <div className="mx-auto flex min-h-0 w-full max-w-[1400px] flex-1 flex-col gap-3 px-4 py-5 sm:px-5">
        <div className="flex items-center gap-3">
          <Link to="/settings#workspaces" className="btn-ghost shrink-0 text-[12.5px]">← Settings</Link>
          <h1 className="text-[20px] font-medium tracking-tight">Clean</h1>
          {checking && <Loader2 className="size-4 animate-spin text-fg-3" aria-label="Measuring workspaces" />}
        </div>
        {err && <p className="rounded-xl border border-bad/25 bg-bad/[0.06] px-3 py-2 text-[12.5px] text-bad">{err}</p>}
        {table && <CleanTable table={table} name={name} left={left} stopping={stopping} busy={busy} onTick={tick} onStop={stopThem} onConfirm={confirm} />}
      </div>
      {shown && <CleanLogView log={shown} name={name} onClose={() => setShown(null)} />}
    </div>
  );
}

/** The Clean table: the items listed and the paths ticked. */
type Picker = { items: CleanItem[]; ticked: Set<string> };

/** Every agent's top-level items in one table: agent, item, type, size, last change and status. Sort by size, agent or
 * last change; tick what to delete. A main clone opens up to its folders, loaded one level at a time. */
function CleanTable({ table, name, left, stopping, busy, onTick, onStop, onConfirm }: {
  table: Picker;
  name: (id: string) => string;
  left: Record<string, LeftProcess[]>;
  stopping: string | null;
  busy: boolean;
  onTick: (paths: string[], on: boolean) => void;
  onStop: (agent: string) => void;
  onConfirm: (paths: string[]) => void;
}) {
  // Folders listed inside opened items, by path; null while loading.
  const [inside, setInside] = useState<Map<string, CleanItem[] | null>>(new Map());
  const [err, setErr] = useState<string | null>(null);
  // Check results by path; null while checking. Git runs only when the user clicks, and nothing gets ticked.
  const [checks, setChecks] = useState<Map<string, GitCheck | null>>(new Map());
  const [sort, setSort] = useState<CleanSort>({ by: "size", desc: true });
  const [filter, setFilter] = useState<CleanFilter>({ agent: "", type: "", deletable: false });
  // Locked reasons opened to their full list, by path.
  const [more, setMore] = useState<Set<string>>(new Set());
  const { items, ticked } = table;
  const toggle = (path: string, on: boolean) => onTick([path], on);
  // Progress of Check git on all: done of total, null when idle.
  const [batch, setBatch] = useState<{ done: number; total: number } | null>(null);
  const allBox = useRef<HTMLInputElement>(null);
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
    // A clean check ticks the row; any other result unticks it.
    onTick([i.path], ticksAfter(c));
  };
  const checkAll = async () => {
    const todo = shown.filter((i) => canCheck(i, 0) && !cleanState(i, 0).locked && checks.get(i.path)?.state !== "safe");
    setBatch({ done: 0, total: todo.length });
    await inBatches(todo, 3, check, (done) => setBatch({ done, total: todo.length }));
    setBatch(null);
  };
  // Changing a filter drops the ticks on rows it hides.
  const changeFilter = (f: CleanFilter) => {
    setFilter(f);
    const keep = new Set(filterClean(items, f).flatMap((i) => [i.path, ...(inside.get(i.path) ?? []).map((c) => c.path)]));
    onTick([...ticked].filter((p) => !keep.has(p)), false);
  };
  const sortBy = (by: CleanSort["by"]) => setSort((s) => (s.by === by ? { by, desc: !s.desc } : { by, desc: by !== "agent" }));
  const head = (by: CleanSort["by"], label: string, cls = "") => (
    <th className={cx("sticky top-0 z-10 bg-ink-900 px-2 py-1.5 font-medium", cls)} aria-sort={sort.by === by ? (sort.desc ? "descending" : "ascending") : "none"}>
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
    const status = cleanState(i, depth, checked);
    return (
      <Fragment key={i.path}>
        <tr className="border-t border-line align-top max-sm:mx-2 max-sm:my-2 max-sm:block max-sm:rounded-xl max-sm:border max-sm:p-1">
          <td className="hidden px-2 py-1.5 text-fg-2 sm:table-cell">{depth === 0 ? name(i.agent) : ""}</td>
          <td className="px-2 py-1.5 max-sm:block">
            <div className="flex items-start gap-2" style={{ paddingLeft: depth * 18 }}>
              <input type="checkbox" className="mt-1 shrink-0" aria-label={`Select ${name(i.agent)} ${i.rel}`} checked={on} disabled={status.locked || busy} onChange={(e) => toggle(i.path, e.target.checked)} />
              <span className="min-w-0">
                <span className="flex items-center gap-1">
                  {i.opens && (
                    <button type="button" className="shrink-0 text-fg-3 hover:text-fg" aria-label={sub !== undefined ? `Close ${i.name}` : `Open ${i.name}`} onClick={() => open(i)}>
                      <ChevronRight className={cx("size-4 transition-transform", sub !== undefined && "rotate-90")} />
                    </button>
                  )}
                  <span className="break-words font-mono text-fg">{i.name}</span>
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
                <span className="block text-[11.5px] text-fg-3 sm:hidden">{depth === 0 && <>{name(i.agent)} · </>}{cleanType(i)} · {timeAgo(i.changed)}</span>
              </span>
            </div>
          </td>
          <td className="hidden whitespace-nowrap px-2 py-1.5 text-fg-3 sm:table-cell">{cleanType(i)}</td>
          <td className="whitespace-nowrap px-2 py-1.5 text-right tabular-nums max-sm:block max-sm:pl-[2.25rem] max-sm:text-left">{fmtBytes(i.bytes)}</td>
          <td className="hidden whitespace-nowrap px-2 py-1.5 text-fg-3 sm:table-cell">{timeAgo(i.changed)}</td>
          <td className={cx("px-2 py-1.5 max-sm:block max-sm:pl-[2.25rem]", status.locked ? "text-warn" : "text-good")}>
            <Reason status={status} blocked={i.blocked} open={more.has(i.path)} onToggle={() => setMore((m) => { const n = new Set(m); if (!n.delete(i.path)) n.add(i.path); return n; })} />
            {!status.locked && i.label && <span className="block text-[11.5px] text-fg-3">{i.label}</span>}
          </td>
        </tr>
        {sub === null && (
          <tr className="max-sm:block"><td colSpan={6} className="px-2 py-1 text-[12px] text-fg-3 max-sm:block" style={{ paddingLeft: (depth + 1) * 18 + 40 }}><Loader2 className="mr-2 inline size-3.5 animate-spin" />Measuring…</td></tr>
        )}
        {sub && sub.length === 0 && (
          <tr className="max-sm:block"><td colSpan={6} className="px-2 py-1 text-[12px] text-fg-3 max-sm:block" style={{ paddingLeft: (depth + 1) * 18 + 40 }}>No folders inside.</td></tr>
        )}
        {sub?.map((c) => row(c, depth + 1))}
      </Fragment>
    );
  };
  // An idle agent's leftover processes sit in the table before its first row.
  const shownAgents = new Set<string>();
  const shown = filterClean(items, filter);
  const sorted = sortClean(shown, sort, name);
  const chosen = chosenRows(shown, inside, ticked);
  const total = chosen.reduce((n, i) => n + i.bytes, 0);
  const pickable = selectable(shown, checks);
  const nTicked = pickable.filter((i) => ticked.has(i.path)).length;
  const all = pickable.length > 0 && nTicked === pickable.length;
  useEffect(() => {
    if (allBox.current) allBox.current.indeterminate = nTicked > 0 && !all;
  });
  const processes = (agent: string): React.ReactNode => {
    const ps = left[agent];
    if (shownAgents.has(agent) || !ps?.length || items.find((i) => i.agent === agent)?.blocked === "Working") return null;
    shownAgents.add(agent);
    return (
      <tr key={`ps-${agent}`} className="border-t border-line bg-ink-800/40 max-sm:block">
        <td colSpan={6} className="px-2 py-1.5 text-[12.5px] max-sm:block">
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
  const agents = [...new Set(items.map((i) => i.agent))];
  const types = [...new Set(items.map(cleanType))];
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      {err && <p className="rounded-xl border border-bad/25 bg-bad/[0.06] px-3 py-2 text-[12.5px] text-bad">{err}</p>}
      {items.length === 0 ? (
        <p className="text-[13px] text-fg-3">Nothing in any workspace.</p>
      ) : (
        <>
        <div className="flex flex-wrap items-center gap-2 text-[12.5px] text-fg-2">
          <select className="input min-w-0 flex-1 sm:w-48 sm:flex-none" aria-label="Agent" disabled={batch !== null} value={filter.agent} onChange={(e) => changeFilter({ ...filter, agent: e.target.value })}>
            <option value="">All agents</option>
            {agents.map((a) => <option key={a} value={a}>{name(a)}</option>)}
          </select>
          <select className="input min-w-0 flex-1 sm:w-48 sm:flex-none" aria-label="Type" disabled={batch !== null} value={filter.type} onChange={(e) => changeFilter({ ...filter, type: e.target.value })}>
            <option value="">All types</option>
            {types.map((x) => <option key={x} value={x}>{x}</option>)}
          </select>
          <label className="flex w-full items-center gap-1.5 sm:w-auto">
            <input type="checkbox" disabled={batch !== null} checked={filter.deletable} onChange={(e) => changeFilter({ ...filter, deletable: e.target.checked })} />
            Deletable only
          </label>
          <label className="flex items-center gap-1.5">
            <input ref={allBox} type="checkbox" aria-label="Select all shown" checked={all} disabled={busy || pickable.length === 0} onChange={(e) => onTick(pickable.map((i) => i.path), e.target.checked)} />
            Select all
          </label>
          <button type="button" className="btn px-2.5 py-1 text-[12px]" disabled={busy || batch !== null || !shown.some((i) => canCheck(i, 0) && !cleanState(i, 0).locked)} onClick={checkAll}>
            Check git on all
          </button>
          {batch && <span className="text-fg-3" role="status">Checking {Math.min(batch.done + 1, batch.total)} of {batch.total}…</span>}
          <label className="flex w-full items-center gap-2 sm:hidden">
            Sort by
            <select className="input flex-1" aria-label="Sort by" value={sort.by} onChange={(e) => setSort({ by: e.target.value as CleanSort["by"], desc: e.target.value !== "agent" })}>
              <option value="size">Size</option>
              <option value="agent">Agent</option>
              <option value="changed">Last change</option>
            </select>
          </label>
        </div>
        <div className="min-h-0 flex-1 overflow-auto rounded-xl border border-line">
          <table className="w-full text-left text-[12.5px] sm:min-w-[640px] max-sm:block">
            <thead className="text-[11.5px] uppercase tracking-wide text-fg-3 max-sm:hidden">
              <tr>
                {head("agent", "Agent", "hidden sm:table-cell")}
                <th className="sticky top-0 z-10 bg-ink-900 px-2 py-1.5 font-medium">Item</th>
                <th className="sticky top-0 z-10 hidden bg-ink-900 px-2 py-1.5 font-medium sm:table-cell">Type</th>
                {head("size", "Size", "text-right")}
                {head("changed", "Last change", "hidden sm:table-cell")}
                <th className="sticky top-0 z-10 bg-ink-900 px-2 py-1.5 font-medium">Status</th>
              </tr>
            </thead>
            <tbody className="max-sm:block">
              {sorted.map((i) => (
                <Fragment key={i.path}>
                  {processes(i.agent)}
                  {row(i, 0)}
                </Fragment>
              ))}
              {sorted.length === 0 && <tr className="max-sm:block"><td colSpan={6} className="px-3 py-4 text-[13px] text-fg-3 max-sm:block">Nothing matches these filters.</td></tr>}
            </tbody>
          </table>
        </div>
        </>
      )}
      {items.length > 0 && <p className="text-[12px] text-fg-3">{EQUIVALENT}</p>}
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-line pt-2">
        <span className="w-full min-w-0 text-[13px] text-fg-2 sm:w-auto sm:flex-1">
          {chosen.length === 0 ? "Nothing selected" : `${chosen.length} selected · ${fmtBytes(total)}`}
        </span>
        <CopyCommands items={chosen} name={name} />
        <button className="btn-primary" disabled={chosen.length === 0 || busy} onClick={() => onConfirm(chosen.map((i) => i.path))}>
          Delete selected
        </button>
      </div>
    </div>
  );
}

/** A status: a long locked reason shows its first line and how many more, the full list on expand. */
function Reason({ status, blocked, open, onToggle }: { status: { text: string; locked: boolean }; blocked: string; open: boolean; onToggle: () => void }) {
  const lines = status.locked && blocked ? reasonLines(blocked) : [];
  if (lines.length < 2 && status.text.length <= 60) return <>{status.text}</>;
  const all = lines.length > 1 ? lines : [blocked || status.text];
  return (
    <>
      {open ? (
        <ul className="list-disc space-y-0.5 pl-4">{all.map((l, n) => <li key={n} className="break-words">{l}</li>)}</ul>
      ) : (
        <span className="block max-w-[34ch] truncate" title={status.text}>
          {status.locked && !all[0].startsWith("Locked") ? "Locked: " : ""}{all[0]}
        </span>
      )}
      <button type="button" className="text-[11.5px] text-accent hover:underline" aria-expanded={open} onClick={onToggle}>
        {open ? "Show less" : all.length > 1 ? `Show all ${all.length}` : "Show all"}
      </button>
    </>
  );
}

