import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";

/** The hashed entry bundle a page was built with, e.g. "/assets/index-AbC123.js". */
function entryOf(html: string): string | null {
  return html.match(/\/assets\/index-[\w-]+\.js/)?.[0] ?? null;
}

function currentEntry(): string | null {
  const s = document.querySelector<HTMLScriptElement>('script[type="module"][src*="/assets/index-"]');
  return s ? new URL(s.src).pathname : null;
}

/**
 * Long-lived tabs keep running whatever bundle they loaded. In production builds,
 * compare our entry bundle with the server's current index.html and offer a reload
 * when they differ. (The Vite dev server hot-reloads on its own.)
 */
export function UpdateBanner({ live }: { live: boolean }) {
  const [stale, setStale] = useState(false);
  const wasLive = useRef(live);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    const mine = currentEntry();
    if (!mine) return;
    const check = async () => {
      try {
        const html = await (await fetch("/", { cache: "no-store" })).text();
        const theirs = entryOf(html);
        if (theirs && theirs !== mine) setStale(true);
      } catch {
        /* backend down; we'll check again */
      }
    };
    check();
    const t = setInterval(check, 60_000);
    const onFocus = () => check();
    window.addEventListener("focus", onFocus);
    (window as any).__loftCheckUpdate = check;
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // A backend restart (live stream reconnecting) is the usual moment a new build lands.
  useEffect(() => {
    if (live && !wasLive.current) (window as any).__loftCheckUpdate?.();
    wasLive.current = live;
  }, [live]);

  if (!stale) return null;
  return (
    <div className="fixed top-[68px] left-1/2 z-50 -translate-x-1/2 animate-rise">
      <div className="flex items-center gap-3 rounded-full border border-accent/30 bg-ink-850/95 py-1.5 pl-4 pr-1.5 text-[13px] shadow-2xl backdrop-blur">
        <span className="size-1.5 rounded-full bg-accent" />
        <span className="text-fg-2">Loft was updated. This tab is running an older version.</span>
        <button className="btn-primary" onClick={() => location.reload()}>
          <RefreshCw className="size-3.5" /> Reload
        </button>
      </div>
    </div>
  );
}
