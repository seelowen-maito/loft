import { Loader2 } from "lucide-react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx } from "../lib/format";

/** A destructive button: delete, cancel, end. */
export const DANGER = "btn border-bad/40 bg-bad/20 text-fg";

const VARIANT = {
  primary: "btn-primary",
  secondary: "btn",
  ghost: "btn-ghost",
  danger: DANGER,
  // Inline text, as in the navbar.
  link: "inline-flex items-center gap-1 text-accent hover:underline disabled:opacity-50",
} as const;

/**
 * A button that starts an async action. While `pending` it shows a spinner and `pendingLabel` (its label otherwise) at
 * full strength, not faded like a disabled button, and can't be clicked. Its error goes under it: see ErrorLine.
 */
export function ActionButton({ variant = "primary", icon, pending = false, pendingLabel, disabled, className, children, ...rest }: {
  variant?: keyof typeof VARIANT;
  icon?: ReactNode;
  pending?: boolean;
  pendingLabel?: ReactNode;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button {...rest} className={cx(VARIANT[variant], pending && "disabled:opacity-100", className)} disabled={pending || disabled} aria-busy={pending}>
      {pending ? <Loader2 className="size-3.5 animate-spin" /> : icon}
      {pending ? (pendingLabel ?? children) : children}
    </button>
  );
}
