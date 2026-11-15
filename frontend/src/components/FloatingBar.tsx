import { useEffect, useState, type ReactNode } from "react";
import { cx } from "../lib/format";

const AT = { top: "top-[68px]", bottom: "bottom-5", toast: "bottom-20" };

/** A pill floating over the page: the update banner at the top, the Settings save bar at the bottom, and toasts just
 *  above it, so a toast never hides behind the save bar. Only the pill takes clicks, not the strip around it. */
export function FloatingBar({ at, accent, role, children }: { at: keyof typeof AT; accent?: boolean; role?: string; children: ReactNode }) {
  return (
    <div role={role} className={cx("pointer-events-none fixed inset-x-0 z-50 flex justify-center px-4", AT[at])}>
      <div
        className={cx(
          "pointer-events-auto flex min-h-[42px] max-w-full animate-rise items-center gap-3 rounded-full border bg-ink-850/95 py-1.5 pl-4 pr-1.5 text-[13px] shadow-2xl backdrop-blur",
          accent ? "border-accent/30" : "border-line-strong",
        )}
      >
        {children}
      </div>
    </div>
  );
}

const shown = new Set<(text: string) => void>();
const TOAST_MS = 2500;

/** Shows a short message at the bottom of the page for a moment, from anywhere. Needs a <Toaster/> on the page. */
export function toast(text: string) {
  shown.forEach((f) => f(text));
}

export function Toaster() {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    shown.add(setText);
    return () => void shown.delete(setText);
  }, []);
  useEffect(() => {
    if (!text) return;
    const t = setTimeout(() => setText(null), TOAST_MS);
    return () => clearTimeout(t);
  }, [text]);
  if (!text) return null;
  return (
    <FloatingBar at="toast" role="status">
      <span className="pr-2.5 text-fg-2">{text}</span>
    </FloatingBar>
  );
}
