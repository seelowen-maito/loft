import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { Check, ExternalLink, Inbox, Send, X } from "lucide-react";
import { api } from "../lib/api";
import { useLiveConnection, useServerEvents, type ServerEvent } from "../lib/events";
import { cx, timeAgo } from "../lib/format";
import { isTyping } from "../lib/shortcuts";
import { answerBody, beingLookedAt, closeTarget, escCloses, freshAfter, headline, idsChanged, itemView, unseenIds, urgencyOf, type NeedsYouAction, type NeedsYouItem, type NeedsYouList } from "../lib/needsYou";

const TONE = { bad: "bg-bad", warn: "bg-warn", accent: "bg-accent", muted: "bg-fg-3/40" } as const;

/** One item: why it waits, its text, where to act and what can be sent from here. One seen before this visit is less
 * prominent; its Done check takes it out of the list. */
function ItemCard({ item, fresh, onDone }: { item: NeedsYouItem; fresh: boolean; onDone: () => void }) {
  const view = itemView(item, window.location.hostname);
  const urgency = urgencyOf(item.urgency);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [answering, setAnswering] = useState(false);
  const [answer, setAnswer] = useState("");

  const send = async (a: NeedsYouAction, body?: Record<string, unknown>, done?: string) => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await api.needsYouCall(a, body);
      setAnswer("");
      setAnswering(false);
      // A reply on an issue nobody works on stays a note there.
      if (a.kind === "answer" && "delivered_to" in r && r.delivered_to === null) setMsg({ ok: true, text: "Kept as a note on the issue: no agent is on it." });
      else if (done) setMsg({ ok: true, text: done });
      onDone();
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <li
      className={cx("panel flex gap-3 p-4 transition-opacity", !fresh && "opacity-60 hover:opacity-100 focus-within:opacity-100")}
      data-needs-you={item.type}
      data-seen={!fresh}
    >
      <span className={cx("mt-1.5 size-2 shrink-0 rounded-full", TONE[urgency.tone])} title={urgency.label} />
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
          {fresh && <span className="self-center rounded-full bg-accent/15 px-1.5 py-px font-mono text-[10px] font-medium uppercase tracking-wide text-accent">New</span>}
          <h2 className={cx("min-w-0 text-[14px] [overflow-wrap:anywhere]", fresh ? "font-medium" : "font-normal text-fg-2")}>{item.title}</h2>
          <span className="font-mono text-[11px] text-fg-3">
            {[item.agent?.name, item.project, timeAgo(item.created_at)].filter(Boolean).join(" · ")}
          </span>
        </div>
        <p className="text-[13px] text-fg-2 [overflow-wrap:anywhere]">{item.reason}</p>
        {item.text && (
          <details className="group">
            <summary className="cursor-pointer select-none font-mono text-[11px] text-fg-3 hover:text-fg">Details</summary>
            <div className="mt-2 max-h-72 overflow-y-auto whitespace-pre-wrap rounded-lg border border-line bg-white/[0.03] px-3 py-2 text-[12.5px] text-fg-2 [overflow-wrap:anywhere]">{item.text}</div>
          </details>
        )}
        <div className="flex flex-wrap items-center gap-2">
          {view.calls.map((a) => (
            <button key={`${a.kind}:${a.label}`} type="button" className={a.kind === "deny" ? "btn" : "btn-primary"} disabled={busy} onClick={() => send(a, undefined, `${a.label}: sent.`)}>
              {a.label}
            </button>
          ))}
          {view.answer && !answering && (
            <button type="button" className="btn" disabled={busy} onClick={() => setAnswering(true)}>
              {view.answer.label}
            </button>
          )}
          {view.open.map((o) =>
            "to" in o.place ? (
              <Link key={o.place.to} to={o.place.to} className="btn-ghost text-[12.5px]">
                {o.label}
              </Link>
            ) : (
              <a key={o.place.href} href={o.place.href} target="_blank" rel="noreferrer" className="btn-ghost inline-flex items-center gap-1 text-[12.5px]">
                {o.label} <ExternalLink className="size-3" />
              </a>
            ),
          )}
          {view.dismiss && (
            <button
              type="button"
              className="btn ml-auto inline-flex items-center gap-1.5 text-[12.5px] hover:border-good/50 hover:text-good"
              title="Done: take it off the list"
              data-needs-you-done
              disabled={busy}
              onClick={() => send(view.dismiss!)}
            >
              <Check className="size-3.5" /> {view.dismiss.label}
            </button>
          )}
        </div>
        {view.answer && answering && (
          <form
            className="flex items-end gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (answer.trim()) send(view.answer!, answerBody(view.answer!, answer.trim()), "Sent.");
            }}
          >
            <textarea
              autoFocus
              rows={2}
              value={answer}
              onChange={(e) => setAnswer(e.target.value)}
              placeholder={item.agent ? `Reply to ${item.agent.name}` : "Your reply"}
              className="min-w-0 flex-1 resize-y rounded-lg border border-line bg-ink-900 px-3 py-2 text-[13px] outline-none focus:border-line-strong"
            />
            <button type="submit" className="btn-primary" disabled={busy || !answer.trim()}>
              <Send className="size-3.5" /> Send
            </button>
            <button type="button" className="btn-ghost text-[12px]" onClick={() => setAnswering(false)}>
              Cancel
            </button>
          </form>
        )}
        {msg && <p className={cx("text-[12px]", msg.ok ? "text-good" : "text-bad")}>{msg.text}</p>}
      </div>
    </li>
  );
}

/** Closes Needs you: back to the page it was opened from, or home when it was opened by its URL. The top bar's icon, the
 * X and Esc all use it. */
export function useCloseNeedsYou(): () => void {
  const navigate = useNavigate();
  const { key } = useLocation();
  return useCallback(() => {
    if (closeTarget(key) === "back") navigate(-1);
    else navigate("/", { replace: true });
  }, [key, navigate]);
}

/** Needs you: everything waiting on the user, the same list the iPhone gets, kept live. */
export default function NeedsYouPage() {
  const live = useLiveConnection();
  const close = useCloseNeedsYou();
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const dialogOpen = !!document.querySelector('[role="dialog"]');
      if (!escCloses({ key: e.key, defaultPrevented: e.defaultPrevented, isComposing: e.isComposing, typing: isTyping(e.target), dialogOpen })) return;
      e.preventDefault();
      closeRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const [list, setList] = useState<NeedsYouList | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // Unseen when they appeared during this visit: shown as new until the page closes, though marked seen at once.
  const [fresh, setFresh] = useState<Set<string>>(new Set());
  const shown = useRef<string[]>([]);
  const toMark = useRef<string[]>([]);
  // What's on screen counts as seen everywhere, but not while the tab is in the background or its window lacks focus.
  const markSeen = useCallback(() => {
    if (!beingLookedAt(document) || toMark.current.length === 0) return;
    const ids = toMark.current;
    toMark.current = [];
    api.needsYouSeen(ids).catch(() => (toMark.current = [...ids, ...toMark.current]));
  }, []);
  const load = useCallback(() => {
    api
      .needsYouList()
      .then((l) => {
        shown.current = l.items.map((i) => i.id);
        setList(l);
        setFresh((f) => freshAfter(f, l.items));
        toMark.current = unseenIds(l.items);
        markSeen();
        setErr(null);
      })
      .catch((e) => setErr((e as Error).message));
  }, [markSeen]);
  useEffect(() => {
    document.addEventListener("visibilitychange", markSeen);
    window.addEventListener("focus", markSeen);
    return () => {
      document.removeEventListener("visibilitychange", markSeen);
      window.removeEventListener("focus", markSeen);
    };
  }, [markSeen]);
  // Events sent while offline are lost: reload on each (re)connect.
  useEffect(() => {
    if (live) load();
  }, [live, load]);
  useEffect(load, [load]);
  useServerEvents(
    useCallback(
      (e: ServerEvent) => {
        if (e.type === "needs_you" && idsChanged(shown.current, e.ids)) load();
      },
      [load],
    ),
  );

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-[900px] flex-col gap-4 px-4 py-5 sm:px-5">
        <div className="flex items-baseline gap-3">
          <h1 className="text-[20px] font-medium tracking-tight">Needs you</h1>
          {list && list.count > 0 && (
            <span className="font-mono text-[12px] text-fg-3" data-needs-you-headline>
              {headline(list.count, list.items.filter((i) => fresh.has(i.id)).length)}
            </span>
          )}
          <button type="button" onClick={close} title="Close (Esc)" aria-label="Close Needs you" className="ml-auto self-center rounded-full p-1.5 text-fg-3 transition-colors hover:bg-white/[0.06] hover:text-fg">
            <X className="size-4" />
          </button>
        </div>
        {err && <p className="text-[12.5px] text-bad">Couldn't load Needs you: {err}</p>}
        {list && list.items.length === 0 && (
          <div className="panel flex flex-col items-center gap-2 px-5 py-12 text-center">
            <Inbox className="size-6 text-fg-3" />
            <p className="text-[13.5px] text-fg-2">Nothing needs you.</p>
            <p className="text-[12px] text-fg-3">Questions, permissions, stalled Deliveries and anything else waiting on you show up here.</p>
          </div>
        )}
        {list && list.items.length > 0 && (
          <ul className="flex flex-col gap-2.5">
            {list.items.map((i) => (
              <ItemCard key={i.id} item={i} fresh={fresh.has(i.id)} onDone={load} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
