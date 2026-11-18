import { useRef } from "react";
import { CornerUpLeft, Reply, X } from "lucide-react";
import { cx } from "../../lib/format";
import { authorLabel, type Quote } from "../../lib/reply";

// Quoted text is shown as plain text: React escapes it, and nothing here parses Markdown or HTML.

/** The quote above the message box while composing a reply, with ✕ to remove it. */
export function QuoteChip({ q, onRemove }: { q: Quote; onRemove: () => void }) {
  return (
    <div className="flex items-start gap-2 rounded-xl border-l-2 border-accent/60 bg-white/[0.04] py-1.5 pl-2.5 pr-1">
      <CornerUpLeft className="mt-0.5 size-3.5 shrink-0 text-accent" aria-hidden />
      <div className="min-w-0 flex-1">
        <div className="font-mono text-[10.5px] uppercase tracking-wider text-fg-3">Replying to {authorLabel(q)}</div>
        <div className="truncate text-[12.5px] text-fg-2">{q.excerpt}</div>
      </div>
      <button type="button" className="rounded-full p-1 text-fg-3 hover:bg-white/[0.08] hover:text-fg" onClick={onRemove} aria-label="Remove the quote">
        <X className="size-3.5" />
      </button>
    </div>
  );
}

/** The quote above a sent reply: clicking it jumps to the original. */
export function QuoteBlock({ q, onJump }: { q: Quote; onJump: (id: number) => void }) {
  return (
    <button
      type="button"
      onClick={() => onJump(q.id)}
      className="flex max-w-[80%] flex-col items-start rounded-xl border-l-2 border-accent/50 bg-white/[0.03] px-3 py-1.5 text-left hover:bg-white/[0.06]"
      aria-label={`Replying to ${authorLabel(q)}: ${q.excerpt}. Show the message.`}
    >
      <span className="flex items-center gap-1 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
        <CornerUpLeft className="size-3" aria-hidden /> {authorLabel(q)}
      </span>
      <span className="line-clamp-2 [overflow-wrap:anywhere] text-[12.5px] text-fg-2">{q.excerpt}</span>
    </button>
  );
}

/** Reply on a message: shown on hover or keyboard focus; on a touch screen a long press replies instead. */
export function ReplyButton({ onReply, className }: { onReply: () => void; className?: string }) {
  return (
    <button
      type="button"
      onClick={onReply}
      className={cx("rounded-full p-1.5 text-fg-3 opacity-0 transition hover:bg-white/[0.08] hover:text-fg focus-visible:opacity-100 group-hover:opacity-100", className)}
      aria-label="Reply"
      title="Reply"
    >
      <Reply className="size-3.5" />
    </button>
  );
}

/** Pointer handlers that call `onPress` after a touch held still for half a second. */
export function useLongPress(onPress: () => void) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const start = useRef<{ x: number; y: number } | null>(null);
  const cancel = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  return {
    onPointerDown: (e: React.PointerEvent) => {
      if (e.pointerType !== "touch") return;
      start.current = { x: e.clientX, y: e.clientY };
      cancel();
      timer.current = setTimeout(() => {
        timer.current = null;
        onPress();
      }, 500);
    },
    onPointerMove: (e: React.PointerEvent) => {
      const s = start.current;
      if (s && Math.hypot(e.clientX - s.x, e.clientY - s.y) > 10) cancel();
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
    onPointerLeave: cancel,
  };
}
