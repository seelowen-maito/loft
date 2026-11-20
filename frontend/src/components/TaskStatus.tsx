import { cx } from "../lib/format";
import { taskStatus, toneText } from "../lib/taskStatus";

/** A task's status: its label in its colour (`text`), as a `chip`, or just a `dot` (one size everywhere). */
export function TaskStatus({ status, variant = "text", className }: { status: string | null | undefined; variant?: "text" | "chip" | "dot"; className?: string }) {
  const s = taskStatus(status);
  const tone = toneText(s.tone);
  if (variant === "dot") return <span className={cx("size-1.5 shrink-0 rounded-full bg-current", tone, className)} title={s.label} />;
  return <span className={cx(variant === "chip" && "chip", tone, className)}>{s.label}</span>;
}
