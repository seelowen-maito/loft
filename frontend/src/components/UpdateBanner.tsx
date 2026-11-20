import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { usePoll } from "../hooks/usePoll";
import { FloatingBar } from "./FloatingBar";

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

  const [mine] = useState(() => (import.meta.env.DEV ? null : currentEntry()));
  const check = async () => {
    try {
      const html = await (await fetch("/", { cache: "no-store" })).text();
      const theirs = entryOf(html);
      if (theirs && theirs !== mine) setStale(true);
    } catch {
      /* backend down; we'll check again */
    }
  };
  usePoll(mine ? check : null, 60_000);
  useEffect(() => {
    if (!mine) return;
    window.addEventListener("focus", check);
    (window as any).__loftCheckUpdate = check;
    return () => window.removeEventListener("focus", check);
  }, [mine]);

  // A backend restart (live stream reconnecting) is the usual moment a new build lands.
  useEffect(() => {
    if (live && !wasLive.current) (window as any).__loftCheckUpdate?.();
    wasLive.current = live;
  }, [live]);

  if (!stale) return null;
  return (
    <FloatingBar at="top" accent>
      <span className="size-1.5 shrink-0 rounded-full bg-accent" />
      <span className="text-fg-2">Loft was updated. This tab is running an older version.</span>
      <button className="btn-primary shrink-0" onClick={() => location.reload()}>
        <RefreshCw className="size-3.5" /> Reload
      </button>
    </FloatingBar>
  );
}
