import { useEffect, useRef, type HTMLAttributes, type ReactNode, type RefObject } from "react";
import { cx } from "../lib/format";
import { closesOnEscape } from "../lib/dialogKeys";

// The open dialogs, the top one last: Esc and Tab act on it only.
const open: RefObject<HTMLElement | null>[] = [];

const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';

/**
 * A dialog's keyboard and focus: Esc closes the top dialog (not from a field, nor while an IME composes), Tab stays
 * inside `box`, and focus goes back where it was on close. `onClose` undefined: Esc does nothing (a clean running).
 */
export function useDialogKeys(box: RefObject<HTMLElement | null>, onClose: (() => void) | undefined) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const me = box;
    // A dialog inside another goes above it, also when both open at once (the inner one's effect runs first).
    const inner = open.findIndex((o) => !!o.current && !!box.current?.contains(o.current));
    open.splice(inner < 0 ? open.length : inner, 0, me);
    const from = document.activeElement as HTMLElement | null;
    // A field with autoFocus already has it.
    if (!box.current?.contains(document.activeElement)) box.current?.focus();
    // Capture, so a page under it (the full-screen interview, Needs you) doesn't take the key first.
    const onKey = (e: KeyboardEvent) => {
      if (open.at(-1) !== me || !box.current) return;
      if (e.key === "Escape") {
        if (e.isComposing || !closesOnEscape(e) || !close.current) return;
        e.preventDefault();
        close.current();
      } else if (e.key === "Tab") {
        const f = [...box.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
        const at = document.activeElement;
        if (f.length === 0) return e.preventDefault();
        if (!box.current.contains(at)) {
          e.preventDefault();
          f[0].focus();
        } else if (e.shiftKey && (at === f[0] || at === box.current)) {
          e.preventDefault();
          f[f.length - 1].focus();
        } else if (!e.shiftKey && at === f[f.length - 1]) {
          e.preventDefault();
          f[0].focus();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      open.splice(open.indexOf(me), 1);
      if (from?.isConnected) from.focus();
    };
  }, [box]);
}

const CARD = "rounded-2xl border border-line-strong bg-ink-900";
const SIZE = {
  sm: `${CARD} max-w-sm`,
  md: `${CARD} max-w-md`,
  lg: `${CARD} max-w-[560px]`,
  xl: `${CARD} max-w-[640px]`,
  // The issue page over the board: full screen at phone width.
  page: "h-full bg-ink-950 sm:h-auto sm:max-h-full sm:max-w-[960px] sm:rounded-2xl sm:border sm:border-line-strong",
  // A panel on the right, full height.
  drawer: "h-full max-w-[640px] border-l border-line bg-ink-900",
};
const PLACE = { drawer: "justify-end", page: "items-center justify-center sm:p-6" };

/**
 * A modal: the backdrop closes it, as Esc does (see useDialogKeys); role=dialog, labelled by `title`. The children lay
 * out the inside, padding included. `onClose` undefined: it can't be closed for now. `top`: it hangs from near the top,
 * so a list that grows (search results) doesn't move it.
 */
export function Dialog({ title, onClose, size = "md", top = false, className, children, ...rest }: { title: string; onClose?: () => void; size?: keyof typeof SIZE; top?: boolean; children: ReactNode } & Omit<HTMLAttributes<HTMLDivElement>, "title">) {
  const box = useRef<HTMLDivElement>(null);
  useDialogKeys(box, onClose);
  return (
    <div className={cx("fixed inset-0 z-50 flex", size === "drawer" || size === "page" ? PLACE[size] : top ? "items-start justify-center px-4 pt-[12vh]" : "items-center justify-center p-4")}>
      <div className="absolute inset-0 bg-black/60 backdrop-blur-[2px]" onClick={onClose} />
      <div
        ref={box}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        {...rest}
        className={cx("relative w-full animate-rise overflow-y-auto shadow-2xl outline-none", SIZE[size], size !== "drawer" && size !== "page" && (top ? "max-h-[76vh]" : "max-h-[85vh]"), className)}
      >
        {children}
      </div>
    </div>
  );
}

/** Asks before an action: Cancel or `confirmLabel`, red when `danger`. `pending` disables it (while it runs, or when it can't yet). */
export function ConfirmDialog({ title, body, confirmLabel, cancelLabel = "Cancel", danger = false, pending = false, error, onConfirm, onClose }: { title: string; body: ReactNode; confirmLabel: string; cancelLabel?: string; danger?: boolean; pending?: boolean; error?: string | null; onConfirm: () => void; onClose: () => void }) {
  return (
    <Dialog title={title} onClose={onClose} size="sm" className="space-y-4 p-5">
      <h3 className="text-[15px] font-medium">{title}</h3>
      <div className="text-[13px] text-fg-2">{body}</div>
      <div className="flex justify-end gap-2">
        <button className="btn-ghost" onClick={onClose}>{cancelLabel}</button>
        <button className={danger ? DANGER : "btn-primary"} disabled={pending} onClick={onConfirm}>{confirmLabel}</button>
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}
    </Dialog>
  );
}

/** A destructive button: delete, cancel, end. */
export const DANGER = "btn border-bad/40 bg-bad/20 text-fg";
