import { useEffect, useRef, useState } from "react";
import { Minimize2, RotateCcw } from "lucide-react";
import { cx, fmtNum } from "../../lib/format";
import { contextWarns } from "../../lib/context";

/** The chat header's context size: warns past the smart zone, and opens Compact and Clear for the Session. */
export function ContextSize({ tokens, running, onCompact, onClear }: {
  tokens: number;
  running: boolean;
  onCompact: () => void;
  onClear: () => void;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
  // A tap anywhere else closes the menu.
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);
  const pick = (action: () => void) => {
    setOpen(false);
    action();
  };

  return (
    <span ref={root} className="relative shrink-0">
      <button
        type="button"
        className={cx("rounded font-mono text-[11px] tabular-nums hover:text-fg-2", contextWarns(tokens) ? "text-warn" : "text-fg-3")}
        aria-haspopup="menu"
        aria-expanded={open}
        title={`${tokens.toLocaleString()} tokens in context as of the last reply. Every turn resends it, so a big context uses your limits faster; Compact shrinks it.`}
        onClick={() => setOpen((o) => !o)}
      >
        context {fmtNum(tokens)}
      </button>
      {open && (
        <span
          role="menu"
          className="absolute left-0 top-full z-30 mt-2 flex w-60 flex-col rounded-lg border border-line-strong bg-ink-800/95 p-1 text-[12.5px] shadow-xl backdrop-blur"
        >
          {running && <span className="px-2.5 py-1.5 text-[11.5px] text-fg-3">Stop the agent first.</span>}
          <button type="button" role="menuitem" className="flex items-start gap-2 rounded-md px-2.5 py-1.5 text-left hover:bg-white/5 disabled:opacity-50 disabled:hover:bg-transparent" disabled={running} onClick={() => pick(onCompact)}>
            <Minimize2 className="mt-0.5 size-3.5 shrink-0" />
            <span>
              <span className="block text-fg">Compact</span>
              <span className="block text-[11.5px] text-fg-3">Summarize the conversation so later runs carry less.</span>
            </span>
          </button>
          <button type="button" role="menuitem" className="flex items-start gap-2 rounded-md px-2.5 py-1.5 text-left hover:bg-white/5 disabled:opacity-50 disabled:hover:bg-transparent" disabled={running} onClick={() => pick(onClear)}>
            <RotateCcw className="mt-0.5 size-3.5 shrink-0" />
            <span>
              <span className="block text-fg">Clear</span>
              <span className="block text-[11.5px] text-fg-3">Start a fresh Session. Memory is kept.</span>
            </span>
          </button>
        </span>
      )}
    </span>
  );
}
