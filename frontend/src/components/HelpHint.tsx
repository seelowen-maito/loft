import { useEffect, useRef, useState, type ReactNode } from "react";
import { CircleHelp } from "lucide-react";

/** A (?) button that opens a popover with the long version of a hint: closed by a tap elsewhere or Esc. */
export function HelpHint({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
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
    <span ref={root} className="relative inline-flex">
      <button
        type="button"
        aria-label={`More about ${label}`}
        aria-expanded={open}
        className="rounded-full text-fg-3 hover:text-fg"
        onClick={(e) => {
          e.preventDefault(); // inside a <label>: don't focus the field
          setOpen((o) => !o);
        }}
      >
        <CircleHelp className="size-3.5" />
      </button>
      {open && (
        <span role="tooltip" className="absolute left-0 top-full z-20 mt-2 w-72 max-w-[calc(100vw-3rem)] rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[12px] font-normal normal-case leading-relaxed tracking-normal text-fg-2 shadow-xl">
          {children}
        </span>
      )}
    </span>
  );
}
