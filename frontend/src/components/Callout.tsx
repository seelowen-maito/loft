import type { ReactNode } from "react";
import { cx } from "../lib/format";

const TONES = {
  bad: "border-bad/30 bg-bad/[0.06] text-bad",
  warn: "border-warn/30 bg-warn/[0.06] text-fg-2",
  accent: "border-accent/30 bg-accent/[0.06] text-fg-2",
} as const;
const ICON = { bad: "text-bad", warn: "text-warn", accent: "text-accent" } as const;

/** An error, warning or note box: one tint per tone, an optional icon on the left and an action on the right.
 *  Errors and warnings are announced (`alert`), notes are not. */
export function Callout({ tone, icon, action, className, children }: {
  tone: keyof typeof TONES;
  icon?: ReactNode;
  action?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div role={tone === "accent" ? "status" : "alert"} className={cx("flex gap-2 rounded-xl border px-3 py-2 text-[12.5px] [overflow-wrap:anywhere]", action ? "items-center" : "items-start", TONES[tone], className)}>
      {icon && <span className={cx("shrink-0 [&>svg]:size-3.5", !action && "mt-0.5", ICON[tone])}>{icon}</span>}
      <div className="min-w-0 flex-1">{children}</div>
      {action}
    </div>
  );
}

/** The one-line result of an action, under its button: green when it worked, red when it didn't. */
export function ResultLine({ said, className }: { said: { ok: boolean; text: string } | null; className?: string }) {
  if (!said) return null;
  return (
    <div role={said.ok ? "status" : "alert"} className={cx("text-[12px]", said.ok ? "text-good" : "text-bad", className)}>
      {said.ok && "✓ "}{said.text}
    </div>
  );
}
