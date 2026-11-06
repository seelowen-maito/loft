import { Loader2, Play } from "lucide-react";
import { cx } from "../lib/format";
import { startDeliveryLabel } from "../lib/startDelivery";

/** Start delivery, on the end-of-planning card and the issue page's delivery cards. While the request runs it shows
 *  a spinner and "Starting delivery…" at full strength (not faded like a disabled button), and can't be clicked. */
export function StartDeliveryButton({ pending, disabled, title, className, onClick }: {
  pending: boolean;
  disabled?: boolean;
  title?: string;
  className?: string;
  onClick: () => void;
}) {
  return (
    <button
      className={cx("btn-primary", pending && "disabled:opacity-100", className)}
      disabled={pending || disabled}
      aria-busy={pending}
      title={title}
      onClick={onClick}
    >
      {pending ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />} {startDeliveryLabel(pending)}
    </button>
  );
}
