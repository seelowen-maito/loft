import { useEffect, useRef, useState, type ReactNode } from "react";
import { cx } from "../lib/format";

const BUBBLE = "pointer-events-none absolute top-full z-30 mt-2 flex whitespace-nowrap rounded-lg border border-line-strong bg-ink-800/95 px-2 py-1 text-left shadow-xl backdrop-blur";

/** A navbar readout with a tooltip: shown at once on mouse hover, toggled by a tap on touch, closed by a tap elsewhere. */
export function HoverTip({ tip, className, left, children }: { tip: ReactNode; className?: string; left?: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const pointer = useRef("mouse");
  const root = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);
  return (
    <button
      ref={root}
      type="button"
      className={cx("relative flex items-center gap-1 rounded hover:text-fg-2", className)}
      aria-expanded={open}
      onPointerDown={(e) => (pointer.current = e.pointerType)}
      onPointerEnter={(e) => e.pointerType === "mouse" && setOpen(true)}
      onPointerLeave={(e) => e.pointerType === "mouse" && setOpen(false)}
      onClick={() => setOpen((o) => !(o && pointer.current !== "mouse"))}
    >
      {children}
      {open && (
        <span role="tooltip" className={cx(BUBBLE, left ? "left-0" : "right-0", "flex-col items-start font-mono text-[11px]")}>
          {tip}
        </span>
      )}
    </button>
  );
}

/** HoverTip's bubble on an icon button or link that already acts on click: its `name`, and its shortcut `keys` (as
 *  "G A", one `<kbd>` per key). Shown on mouse hover and keyboard focus, not on touch; the control keeps its own aria-label. */
export function Tip({ name, keys, left, className, children }: { name: string; keys?: string; left?: boolean; className?: string; children: ReactNode }) {
  return (
    <span className={cx("group/tip relative inline-flex", className)}>
      {children}
      <span
        aria-hidden
        className={cx(
          BUBBLE,
          left ? "left-0" : "right-0",
          "invisible items-center gap-2 py-1.5 font-sans text-[12px] text-fg opacity-0 transition-opacity group-hover/tip:visible group-hover/tip:opacity-100 group-has-[:focus-visible]/tip:visible group-has-[:focus-visible]/tip:opacity-100",
        )}
      >
        {name}
        {keys && (
          <span className="flex gap-1">
            {keys.split(" ").map((k, i) => (
              <kbd key={i} className="rounded border border-b-2 border-line-strong bg-ink-850 px-1 font-mono text-[10.5px] text-fg-2">{k}</kbd>
            ))}
          </span>
        )}
      </span>
    </span>
  );
}
