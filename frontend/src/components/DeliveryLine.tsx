import { useEffect, useState } from "react";
import { ExternalLink } from "lucide-react";
import { cx } from "../lib/format";
import { deliveryStateLine, type DeliveryTone } from "../lib/deliveryState";

const DELIVERY_TONE: Record<DeliveryTone, string> = {
  running: "text-accent",
  ci: "text-accent",
  stalled: "text-bad",
  paused: "text-fg-3",
  needs_you: "text-warn",
  final: "text-accent",
  done: "text-good",
};

/** A started Delivery's state line. Waiting for CI counts its minutes (re-rendering only this line) and opens the
 *  Checks tab on GitHub. */
export function DeliveryLine({ d, className }: { d: Parameters<typeof deliveryStateLine>[0]; className?: string }) {
  const [now, setNow] = useState(Date.now());
  const ci = d?.state_line?.tone === "ci";
  useEffect(() => {
    if (!ci) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, [ci]);
  const line = deliveryStateLine(d, now);
  if (!line) return null;
  const cls = cx("text-[12.5px]", DELIVERY_TONE[line.tone], className);
  return line.href ? (
    <a href={line.href} target="_blank" rel="noreferrer" className={cx(cls, "inline-flex items-center gap-1 hover:underline")} data-delivery-state={line.tone} title="The CI checks on GitHub">
      {line.text} <ExternalLink className="size-3 shrink-0" />
    </a>
  ) : (
    <div className={cls} data-delivery-state={line.tone}>{line.text}</div>
  );
}
