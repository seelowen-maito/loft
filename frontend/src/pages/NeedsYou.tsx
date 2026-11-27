import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { Check, ExternalLink, Inbox, Send } from "lucide-react";
import { api } from "../lib/api";
import { useLiveConnection, useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";
import { relTime } from "../lib/time";
import { isTyping } from "../lib/shortcuts";
import { ActionButton } from "../components/ActionButton";
import { ErrorLine } from "../components/Callout";
import { ConfirmDialog } from "../components/Dialog";
import { errorText, useAction } from "../lib/useAction";
import { answerBody, beingLookedAt, clearAll, clearTitle, closeTarget, escCloses, freshAfter, headline, idsChanged, itemView, kindTag, shortTitle, unseenIds, urgencyOf, type NeedsYouAction, type NeedsYouItem, type NeedsYouList } from "../lib/needsYou";

const TONE = { bad: "bg-bad", warn: "bg-warn", accent: "bg-accent", muted: "bg-fg-3/40" } as const;
const TAG = { bad: "border-bad/40 text-bad", warn: "border-warn/40 text-warn", info: "border-info/40 text-info", good: "border-good/40 text-good" } as const;

/** One item: why it waits, its text, where to act and what can be sent from here. One seen before this visit is less
 * prominent; its Done check takes it out of the list. */
function ItemCard({ item, fresh, onDone }: { item: NeedsYouItem; fresh: boolean; onDone: () => void }) {
  const view = itemView(item, window.location.hostname);
  const urgency = urgencyOf(item.urgency);
  const tag = kindTag(item.type);
  const title = shortTitle(item.title, item.project);
  const act = useAction();
  // Which button is working, by kind and label: the list of calls is the item's.
  const [doing, setDoing] = useState<string | null>(null);
  const keyOf = (a: NeedsYouAction) => `${a.kind}:${a.label}`;
  const busy = act.busy;
  const [msg, setMsg] = useState<string | null>(null);
  const [answering, setAnswering] = useState(false);
  const [answer, setAnswer] = useState("");

  const send = (a: NeedsYouAction, body?: Record<string, unknown>, done?: string) => {
    setMsg(null);
    setDoing(keyOf(a));
    act.run(async () => {
      const r = await api.needsYouCall(a, body);
      setAnswer("");
      setAnswering(false);
      // A reply on an issue nobody works on stays a note there.
      if (a.kind === "answer" && "delivered_to" in r && r.delivered_to === null) setMsg("Kept as a note on the issue: no agent is on it.");
      else if (done) setMsg(done);
      onDone();
    });
  };
  const pending = (a: NeedsYouAction | undefined) => busy && !!a && doing === keyOf(a);

  return (
    <li
      className={cx("panel flex gap-3 p-4 transition-opacity", !fresh && "opacity-60 hover:opacity-100 focus-within:opacity-100")}
      data-needs-you={item.type}
      data-seen={!fresh}
    >
      <span className={cx("mt-1.5 size-2 shrink-0 rounded-full", TONE[urgency.tone])} title={urgency.label} />
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
          <span className={cx("chip self-center !text-[10px] uppercase tracking-wide", TAG[tag.tone])}>{tag.label}</span>
          <h2 title={title.full} className={cx("min-w-0 text-[14px] [overflow-wrap:anywhere]", fresh ? "font-medium" : "font-normal text-fg-2")}>{title.text}</h2>
          {fresh && <span className="self-center rounded-full bg-accent/15 px-1.5 py-px font-mono text-[10px] font-medium uppercase tracking-wide text-accent">New</span>}
          <span className="font-mono text-[11px] text-fg-3">
            {[item.agent?.name, title.project, relTime(item.created_at)].filter(Boolean).join(" · ")}
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
            <ActionButton key={keyOf(a)} type="button" variant={a.kind === "deny" ? "secondary" : "primary"} pending={pending(a)} pendingLabel="Sending…" disabled={busy} onClick={() => send(a, undefined, `${a.label}: sent.`)}>
              {a.label}
            </ActionButton>
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
            <ActionButton
              type="button"
              variant="secondary"
              className="ml-auto inline-flex items-center gap-1.5 text-[12.5px] hover:border-good/50 hover:text-good"
              title="Done: take it off the list"
              data-needs-you-done
              icon={<Check className="size-3.5" />}
              pending={pending(view.dismiss)}
              pendingLabel="Saving…"
              disabled={busy}
              onClick={() => send(view.dismiss!)}
            >
              {view.dismiss.label}
            </ActionButton>
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
            <ActionButton type="submit" icon={<Send className="size-3.5" />} pending={pending(view.answer)} pendingLabel="Sending…" disabled={busy || !answer.trim()}>
              Send
            </ActionButton>
            <button type="button" className="btn-ghost text-[12px]" onClick={() => setAnswering(false)}>
              Cancel
            </button>
          </form>
        )}
        {msg && <p className="text-[12px] text-good">{msg}</p>}
        <ErrorLine error={act.error} />
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
  // The ids Clear all was opened with: what the confirm names and sends, whatever arrives while it is open.
  const [confirming, setConfirming] = useState<string[] | null>(null);
  const clearing = useAction();
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
      .catch((e) => setErr(errorText(e)));
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
        if (e.type === "resync" || (e.type === "needs_you" && idsChanged(shown.current, e.ids))) load();
      },
      [load],
    ),
  );

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex w-full max-w-[900px] flex-col gap-4 px-4 py-5 sm:px-5">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <h1 className="whitespace-nowrap text-[20px] font-medium tracking-tight">Needs you</h1>
          {list && list.count > 0 && (
            <span className="font-mono text-[12px] text-fg-3" data-needs-you-headline>
              {headline(list.count, list.items.filter((i) => fresh.has(i.id)).length)}
            </span>
          )}
          <div className="ml-auto flex items-center gap-2">
            {list && clearAll(list.items).clear > 0 && (
              <button type="button" className="btn-ghost inline-flex items-center gap-1.5 self-center whitespace-nowrap text-[12.5px]" data-needs-you-clear-all onClick={() => setConfirming(clearAll(list.items).ids)}>
                <Check className="size-3.5" /> Mark all done
              </button>
            )}
            <button type="button" onClick={close} title="Close (Esc)" className="btn-ghost self-center text-[12.5px]">
              Close
            </button>
          </div>
        </div>
        {confirming && (
          <ConfirmDialog
            title={clearTitle(confirming.length)}
            body="Each one is marked Done and leaves the list, on every device."
            confirmLabel="Mark all done"
            pendingLabel="Saving…"
            pending={clearing.busy}
            error={clearing.error}
            onConfirm={() =>
              clearing.run(async () => {
                await api.needsYouClearAll(confirming);
                setConfirming(null);
                load();
              })
            }
            onClose={() => setConfirming(null)}
          />
        )}
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
