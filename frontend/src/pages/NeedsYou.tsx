import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { ExternalLink, Inbox, Send } from "lucide-react";
import { api } from "../lib/api";
import { useLiveConnection, useServerEvents, type ServerEvent } from "../lib/events";
import { cx, timeAgo } from "../lib/format";
import { answerBody, idsChanged, itemView, urgencyOf, type NeedsYouAction, type NeedsYouItem, type NeedsYouList } from "../lib/needsYou";

const TONE = { bad: "bg-bad", warn: "bg-warn", accent: "bg-accent", muted: "bg-fg-3/40" } as const;

/** One item: why it waits, its text, where to act and what can be sent from here. */
function ItemCard({ item, onDone }: { item: NeedsYouItem; onDone: () => void }) {
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
    <li className="panel flex gap-3 p-4" data-needs-you={item.type}>
      <span className={cx("mt-1.5 size-2 shrink-0 rounded-full", TONE[urgency.tone])} title={urgency.label} />
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
          <h2 className="min-w-0 text-[14px] font-medium [overflow-wrap:anywhere]">{item.title}</h2>
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
            <button type="button" className="btn-ghost ml-auto text-[12px] text-fg-3" disabled={busy} onClick={() => send(view.dismiss!)}>
              {view.dismiss.label}
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

/** Needs you: everything waiting on the user, the same list the iPhone gets, kept live. */
export default function NeedsYouPage() {
  const live = useLiveConnection();
  const [list, setList] = useState<NeedsYouList | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const shown = useRef<string[]>([]);
  const load = useCallback(() => {
    api
      .needsYouList()
      .then((l) => {
        shown.current = l.items.map((i) => i.id);
        setList(l);
        setErr(null);
      })
      .catch((e) => setErr((e as Error).message));
  }, []);
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
            <span className="font-mono text-[12px] text-fg-3">
              {list.count} waiting{list.urgent > 0 && `, ${list.urgent} urgent`}
            </span>
          )}
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
              <ItemCard key={i.id} item={i} onDone={load} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
