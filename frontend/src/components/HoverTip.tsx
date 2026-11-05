import { useEffect, useRef, useState, type ReactNode } from "react";
import { cx } from "../lib/format";

/** A navbar readout with a tooltip: shown at once on mouse hover, toggled by a tap on touch, closed by a tap elsewhere. */
export function HoverTip({ tip, className, children }: { tip: ReactNode; className?: string; children: ReactNode }) {
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
        <span
          role="tooltip"
          className="pointer-events-none absolute right-0 top-full z-30 mt-2 flex flex-col items-start whitespace-nowrap rounded-lg border border-line-strong bg-ink-800/95 px-2 py-1 text-left font-mono text-[11px] shadow-xl backdrop-blur"
        >
          {tip}
        </span>
      )}
    </button>
  );
}
