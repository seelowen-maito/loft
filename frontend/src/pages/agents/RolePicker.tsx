// The New agent dialog's Role field: a button showing the chosen role, and the command-palette panel it opens (#494).
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { BookOpen, ChevronDown, Crown, FlaskConical, Hammer, Inbox, Lock, Package, ScanEye, ShieldCheck, Sparkles, Workflow, type LucideIcon } from "lucide-react";
import { cx } from "../../lib/format";
import { pickLocked, pickNumber, pickerFooter, pickerKey, type NewAgentForm, type RolePick } from "../../lib/newAgent";
import { Dialog } from "../../components/Dialog";
import { ErrorLine } from "../../components/Callout";

const ICONS: Record<string, LucideIcon> = {
  lead: Crown,
  dev: Hammer,
  reviewer: ScanEye,
  tests: FlaskConical,
  security: ShieldCheck,
  custom: Sparkles,
  deps: Package,
  docs: BookOpen,
  triage: Inbox,
  ci: Workflow,
};

const iconOf = (p: RolePick) => ICONS[p.key] ?? Sparkles;

export function RoleField({ form, current, team, solo, error, onPick }: { form: NewAgentForm; current: RolePick; team: RolePick[]; solo: RolePick[]; error: string | null; onPick: (p: RolePick) => void }) {
  const [open, setOpen] = useState(false);
  const Icon = iconOf(current);
  return (
    <div className="space-y-1.5">
      <span className="eyebrow">Role</span>
      <button type="button" className="input flex items-center gap-3 text-left" aria-haspopup="dialog" onClick={() => setOpen(true)}>
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-accent/10 text-accent"><Icon className="size-4" /></span>
        <span className="min-w-0 flex-1">
          <span className="block text-[13.5px] text-fg">{current.label}</span>
          <span className="block truncate text-[11.5px] text-fg-3">{current.summary}</span>
        </span>
        <ChevronDown className="size-4 shrink-0 text-fg-3" />
      </button>
      {open && createPortal(
        <RolePanel form={form} current={current} team={team} solo={solo} error={error} onClose={() => setOpen(false)} onPick={(p) => {
          onPick(p);
          setOpen(false);
        }} />,
        document.body,
      )}
    </div>
  );
}

function RolePanel({ form, current, team, solo, error, onClose, onPick }: { form: NewAgentForm; current: RolePick; team: RolePick[]; solo: RolePick[]; error: string | null; onClose: () => void; onPick: (p: RolePick) => void }) {
  const rows = [...team, ...solo];
  const [at, setAt] = useState(() => Math.max(0, rows.findIndex((r) => r.key === current.key)));
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    list.current?.querySelector(`[data-row="${at}"]`)?.scrollIntoView({ block: "nearest" });
  }, [at]);
  const choose = (p: RolePick | undefined) => p && !pickLocked(p, form) && onPick(p);

  const onKey = (e: KeyboardEvent) => {
    if (/^[0-9]$/.test(e.key) && (e.metaKey || e.ctrlKey || e.altKey)) return;
    const next = pickerKey(at, rows, e.key, form);
    if (next || e.key === "Enter") e.preventDefault();
    if (next && "at" in next) setAt(next.at);
    else if (next) onPick(next.pick);
  };

  const section = (title: string, picks: RolePick[], from: number) => (
    <div className="py-1.5">
      <div className="eyebrow px-4 pb-1 pt-1.5">{title}</div>
      {picks.map((p, k) => {
        const i = from + k;
        const Icon = iconOf(p);
        const locked = pickLocked(p, form);
        return (
          <button
            key={p.key}
            type="button"
            data-row={i}
            role="option"
            aria-selected={i === at}
            aria-disabled={locked}
            tabIndex={-1}
            className={cx("flex w-full items-center gap-3 px-4 py-2 text-left transition", i === at ? "bg-ink-800" : "hover:bg-ink-850", locked && "opacity-60")}
            onMouseMove={() => setAt(i)}
            onClick={() => choose(p)}
          >
            <span className={cx("grid size-8 shrink-0 place-items-center rounded-lg", i === at ? "bg-accent/15 text-accent" : "bg-ink-800 text-fg-2")}><Icon className="size-4" /></span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5 text-[13.5px] text-fg">
                {p.label}
                {locked && <Lock className="size-3 text-fg-3" aria-label="Needs a group" />}
                {p.key === current.key && <span className="text-[11px] text-accent">current</span>}
              </span>
              <span className="block truncate text-[11.5px] text-fg-3">{p.summary}</span>
            </span>
            <kbd className="chip hidden min-w-6 justify-center font-mono text-[11px] sm:inline-flex">{pickNumber(i)}</kbd>
          </button>
        );
      })}
    </div>
  );

  return (
    <Dialog title="Choose a role" size="lg" top onClose={onClose} onKeyDown={onKey} className="scroll-pb-28">
      <div ref={list} role="listbox" aria-label="Roles">
        {section("Team", team, 0)}
        <div className="border-t border-line" />
        {section("On their own", solo, team.length)}
        {team.length === 0 && !error && <p className="px-4 pb-3 text-[12px] text-fg-3">Loading the team's templates…</p>}
        {error && <div className="px-4 pb-3"><ErrorLine error={error} /></div>}
      </div>
      <div className="sticky bottom-0 space-y-1.5 border-t border-line bg-ink-900 px-4 py-3">
        <p className="text-[12px] leading-snug text-fg-2">
          {pickerFooter(at, rows, form)}
        </p>
        <p className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-fg-3">
          <span><kbd className="font-mono">↑↓</kbd> navigate</span>
          <span><kbd className="font-mono">↵</kbd> choose</span>
          <span><kbd className="font-mono">1–{pickNumber(Math.min(rows.length, 10) - 1)}</kbd> jump</span>
          <span><kbd className="font-mono">esc</kbd> close</span>
        </p>
      </div>
    </Dialog>
  );
}
