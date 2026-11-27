import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { CircleHelp } from "lucide-react";

/** A (?) button that opens a popover with the long version of a hint. The popover sits under the nearest `relative`
 * ancestor (the label row), as wide as it, so it never leaves the card. Closed by a tap elsewhere, Esc or Tab away. */
export function HelpHint({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
  const popover = useId();
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => !root.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);
  return (
    <span ref={root} onBlur={(e) => !root.current?.contains(e.relatedTarget as Node | null) && setOpen(false)}>
      <button
        type="button"
        aria-label={`More about ${label}`}
        aria-expanded={open}
        aria-controls={open ? popover : undefined}
        className="-m-1.5 rounded-full p-1.5 text-fg-3 hover:text-fg focus-visible:outline-2 focus-visible:outline-accent"
        onClick={() => setOpen((o) => !o)}
      >
        <CircleHelp className="size-3.5" />
      </button>
      {open && (
        <span id={popover} className="absolute inset-x-0 top-full z-20 mt-2 max-w-sm rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[12px] font-normal normal-case leading-relaxed tracking-normal text-fg-2 shadow-xl">
          {children}
        </span>
      )}
    </span>
  );
}
