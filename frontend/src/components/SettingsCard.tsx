import { useEffect, useRef, type ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

/** A Settings section: the title and description, an action on the right, one error line and a skeleton while it loads.
 *  Linking to `/settings#<id>` scrolls to it once it has loaded, and again while the cards above it grow for a few seconds. */
export function SettingsCard({ id, title, description, action, error, loading, children }: {
  id?: string;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  error?: string | null;
  loading?: boolean;
  children?: ReactNode;
}) {
  const card = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!id || loading || window.location.hash !== `#${id}`) return;
    const go = () => card.current?.scrollIntoView({ block: "start" });
    go();
    const grows = new ResizeObserver(go);
    grows.observe(card.current?.parentElement ?? document.body);
    const stop = setTimeout(() => grows.disconnect(), 3000);
    return () => {
      clearTimeout(stop);
      grows.disconnect();
    };
  }, [id, loading]);
  return (
    <section ref={card} id={id} className="panel scroll-mt-20 space-y-4 p-5">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-[15px] font-medium tracking-tight">{title}</h2>
          {description && <p className="mt-1 text-[12.5px] text-fg-3">{description}</p>}
        </div>
        {action}
      </div>
      {error && <p role="alert" className="text-[12.5px] text-bad [overflow-wrap:anywhere]">{error}</p>}
      {loading ? <div aria-busy className="h-14 animate-pulse rounded-xl bg-white/[0.05]" /> : children}
    </section>
  );
}

/** A card's "Check again" button; `label` is "Check" for a card that has not checked yet. */
export function CheckButton({ busy, onClick, icon: Icon, label = "Check again" }: { busy: boolean; onClick: () => void; icon: LucideIcon; label?: string }) {
  return (
    <button type="button" className="btn shrink-0" disabled={busy} onClick={onClick}>
      <Icon className="size-3.5" /> {busy ? "Checking…" : label}
    </button>
  );
}
