import { useEffect, useId, useRef, useState } from "react";
import type { Message } from "../../lib/api";
import { cx } from "../../lib/format";
import { fyiTitle, isFyi } from "../../lib/agentMessage";

/** The FYI mark on an agent message: its note opens on hover with a mouse, or on tap/Enter elsewhere.
 * The note anchors to the nearest positioned ancestor (the header row) so it stays on screen on phones.
 * Renders nothing on any other message. */
export function FyiMark({ m, agentName }: { m: Message; agentName: string }) {
  const [open, setOpen] = useState(false);
  const pointer = useRef("mouse");
  const root = useRef<HTMLSpanElement>(null);
  const noteId = useId();
  // A tap anywhere else closes a tapped-open note.
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);
  if (!isFyi(m)) return null;

  return (
    <span ref={root} className="flex">
      <button
        type="button"
        className="rounded border border-line px-1 py-px text-[9.5px] uppercase text-fg-2 hover:text-fg"
        aria-expanded={open}
        aria-describedby={noteId}
        onPointerDown={(e) => (pointer.current = e.pointerType)}
        onPointerEnter={(e) => e.pointerType === "mouse" && setOpen(true)}
        onPointerLeave={(e) => e.pointerType === "mouse" && setOpen(false)}
        onClick={() => setOpen((o) => (o && pointer.current !== "mouse" ? false : true))}
        // Tabbing away closes a note opened from the keyboard.
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          pointer.current = "keyboard";
          if (e.key === "Escape") setOpen(false);
        }}
      >
        FYI
      </button>
      {/* Always in the DOM so screen readers get it as the mark's description; shown only when open. */}
      <span
        id={noteId}
        role="tooltip"
        className={cx(
          open
            ? "pointer-events-none absolute right-0 top-full z-30 mt-1.5 w-max max-w-[min(18rem,calc(100vw-5rem))] rounded-lg border border-line-strong bg-ink-800/95 px-2 py-1 text-left font-mono text-[11px] normal-case tracking-normal text-fg-2 shadow-xl backdrop-blur"
            : "sr-only",
        )}
      >
        {fyiTitle(agentName)}
      </span>
    </span>
  );
}
