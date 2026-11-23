import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft, Bookmark, MessageSquare, Search } from "lucide-react";
import { api, type SavedItem } from "../lib/api";
import { cx } from "../lib/format";
import { fmtDate } from "../lib/time";
import { errorText } from "../lib/useAction";
import { excerpt, sourceHref, tagOptions } from "../lib/saved";
import { ErrorLine } from "../components/Callout";

const PAGE = "mx-auto flex w-full max-w-[900px] flex-col gap-4 px-4 py-5 sm:px-5";

/** Saved: every kept message, newest first, with search and a tag filter. `/saved/:id` opens one. */
export default function SavedPage() {
  const { id } = useParams();
  return id ? <ItemPage id={Number(id)} /> : <ListPage />;
}

function ListPage() {
  const [q, setQ] = useState("");
  const [tag, setTag] = useState("");
  const [list, setList] = useState<{ items: SavedItem[]; tags: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    // Typing searches once it pauses.
    const t = setTimeout(() => {
      api.savedList(q.trim(), tag).then((l) => live && (setList(l), setError(null)), (e) => live && setError(errorText(e)));
    }, 200);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [q, tag]);
  const tags = tagOptions(list?.tags ?? []);
  return (
    <div className={PAGE}>
      <h1 className="flex items-center gap-2 text-[18px] font-medium tracking-tight">
        <Bookmark className="size-4 text-accent" /> Saved
      </h1>
      <div className="flex flex-wrap items-center gap-2">
        <label className="flex min-w-[160px] flex-1 items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5 sm:max-w-xs">
          <Search className="size-3.5 text-fg-3" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search titles and text" aria-label="Search saved" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
        </label>
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Tag">
          {["", ...tags].map((t) => (
            <button key={t || "all"} type="button" aria-pressed={tag === t} onClick={() => setTag(t)} className={cx("rounded-full px-2.5 py-0.5 text-[12px]", tag === t ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
              {t || "All"}
            </button>
          ))}
        </div>
      </div>
      <ErrorLine error={error} />
      {list && list.items.length === 0 && (
        <p className="py-10 text-center text-[13px] text-fg-3">
          {q || tag ? "Nothing saved matches." : "Nothing saved yet. Save a message from its footer in any chat."}
        </p>
      )}
      <ul className="flex flex-col gap-2">
        {list?.items.map((i) => (
          <li key={i.id}>
            <Link to={`/saved/${i.id}`} className="flex flex-col gap-1 rounded-xl border border-line bg-white/[0.02] px-4 py-3 transition-colors hover:bg-white/[0.05]">
              <div className="flex items-baseline gap-2">
                <span className="min-w-0 flex-1 truncate text-[14px] text-fg">{i.title}</span>
                {i.tag && <span className="chip shrink-0">{i.tag}</span>}
              </div>
              <span className="line-clamp-2 text-[12.5px] text-fg-3">{excerpt(i.content)}</span>
              <span className="font-mono text-[10.5px] text-fg-3">{i.agent_name} · {fmtDate(i.created_at)}</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ItemPage({ id }: { id: number }) {
  const [item, setItem] = useState<(SavedItem & { source_exists: boolean }) | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    api.savedItem(id).then((i) => live && setItem(i), (e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [id]);
  return (
    <div className={PAGE}>
      <Link to="/saved" className="flex w-fit items-center gap-1 text-[12.5px] text-fg-3 hover:text-fg">
        <ArrowLeft className="size-3.5" /> Saved
      </Link>
      <ErrorLine error={error} />
      {item && (
        <article className="flex flex-col gap-3">
          <h1 className="text-[20px] font-medium tracking-tight [overflow-wrap:anywhere]">{item.title}</h1>
          <div className="flex flex-wrap items-center gap-2 font-mono text-[11px] text-fg-3">
            {item.tag && <span className="chip">{item.tag}</span>}
            <span>From {item.agent_name} · {fmtDate(item.created_at)}</span>
            {item.source_exists ? (
              <Link to={sourceHref(item)} className="flex items-center gap-1 text-accent hover:underline">
                <MessageSquare className="size-3" /> Open in chat
              </Link>
            ) : (
              <span>· The source message is gone</span>
            )}
          </div>
          <div className="prose-chat rounded-2xl border border-line bg-white/[0.02] px-5 py-4 [overflow-wrap:anywhere]">
            <Markdown remarkPlugins={[remarkGfm]}>{item.content}</Markdown>
          </div>
        </article>
      )}
    </div>
  );
}
