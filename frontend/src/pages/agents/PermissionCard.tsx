import { useState } from "react";
import { Check, CheckCheck, MessageCircleQuestion, ShieldAlert, ShieldCheck, ShieldX, X } from "lucide-react";
import { api, type PermissionRequest } from "../../lib/api";
import { cx, timeAgo } from "../../lib/format";
import { ASK_TOOL, answersOf, questionsOf, toggle, typeOther, type Pick } from "../../lib/questions";

function describeInput(p: PermissionRequest) {
  const i = p.input ?? {};
  if (typeof i.command === "string") return i.command;
  if (typeof i.file_path === "string") return i.file_path;
  if (typeof i.url === "string") return i.url;
  const s = JSON.stringify(i);
  return s === "{}" ? "" : s;
}

const RESOLVED: Record<string, { label: string; icon: React.ReactNode; cls: string }> = {
  allowed_once: { label: "Allowed once", icon: <ShieldCheck className="size-3.5" />, cls: "text-good" },
  allowed_always: { label: "Always allowed · saved to config", icon: <ShieldCheck className="size-3.5" />, cls: "text-good" },
  denied: { label: "Denied", icon: <ShieldX className="size-3.5" />, cls: "text-bad" },
  expired: { label: "Expired", icon: <ShieldX className="size-3.5" />, cls: "text-fg-3" },
};

export function PermissionCard({ req, compact = false }: { req: PermissionRequest; compact?: boolean }) {
  if (req.source === "prompt" && req.tool_name === ASK_TOOL) return <QuestionsCard req={req} compact={compact} />;
  return <ToolCard req={req} compact={compact} />;
}

/** An agent's multiple-choice questions: answering them is the whole decision, so there's nothing to allow. */
function QuestionsCard({ req, compact }: { req: PermissionRequest; compact: boolean }) {
  const qs = questionsOf(req.input);
  const [picks, setPicks] = useState<Record<number, Pick>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const pending = req.status === "pending";
  const answered: Record<string, string> = req.input?.answers ?? {};
  const answers = answersOf(qs, picks);
  const pick = (i: number) => picks[i] ?? { chosen: [], other: "" };
  const set = (i: number, p: Pick) => setPicks((ps) => ({ ...ps, [i]: p }));

  const act = async (d: "allow_once" | "deny") => {
    setBusy(true);
    setErr(null);
    try {
      await api.decide(req.id, d, undefined, d === "deny" ? undefined : (answers ?? undefined));
    } catch (e) {
      setErr(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };

  const status = req.status === "denied" ? "Skipped" : req.status === "expired" ? "Expired" : null;
  return (
    <div
      id={`perm-${req.id}`}
      className={cx(
        "scroll-mt-4 rounded-2xl border p-4",
        pending ? "border-warn/30 bg-warn/[0.05] shadow-[0_0_0_4px_rgb(242_196_109/0.04)]" : "border-line bg-ink-900/60",
      )}
    >
      <div className="flex items-center gap-2 text-[13px]">
        <MessageCircleQuestion className={cx("size-4", pending ? "text-warn" : "text-fg-3")} />
        <span className="text-fg">{qs.length === 1 ? "Has a question for you" : "Has questions for you"}</span>
        <span className="ml-auto font-mono text-[11px] text-fg-3">{timeAgo(req.created_at)}</span>
      </div>
      <div className="mt-3 space-y-4">
        {qs.map((q, i) => {
          const multi = !!q.multiSelect;
          const p = pick(i);
          return (
            <div key={i}>
              {q.header && <div className="eyebrow mb-1">{q.header}</div>}
              <p className="text-[13.5px] text-fg">{q.question}</p>
              {pending ? (
                <div className="mt-2 space-y-1.5">
                  {(q.options ?? []).map((o) => {
                    const on = p.chosen.includes(o.label);
                    return (
                      <button
                        key={o.label}
                        type="button"
                        disabled={busy}
                        aria-pressed={on}
                        onClick={() => set(i, toggle(p, o.label, multi))}
                        className={cx(
                          "block w-full rounded-xl border px-3 py-2 text-left transition",
                          on ? "border-accent bg-accent/10" : "border-line bg-ink-950 hover:border-fg-3",
                        )}
                      >
                        <span className="flex items-center gap-2 text-[13px] text-fg">
                          {on && <Check className="size-3.5 shrink-0 text-accent" />}
                          {o.label}
                        </span>
                        {o.description && !compact && <span className="mt-0.5 block text-[12px] text-fg-2">{o.description}</span>}
                      </button>
                    );
                  })}
                  <input
                    className="input py-1.5 text-[13px]"
                    placeholder={multi ? "Something else, too…" : "Something else…"}
                    value={p.other}
                    disabled={busy}
                    onChange={(e) => set(i, typeOther(p, e.target.value, multi))}
                  />
                  {multi && <div className="text-[11.5px] text-fg-3">Pick any that apply.</div>}
                </div>
              ) : (
                answered[q.question] && (
                  <div className="mt-1 flex items-center gap-1.5 text-[13px] text-good">
                    <Check className="size-3.5" /> {answered[q.question]}
                  </div>
                )
              )}
            </div>
          );
        })}
      </div>
      {pending ? (
        <div className="mt-4 space-y-2">
          <div className="flex flex-wrap gap-2">
            <button className="btn-primary" disabled={busy || !answers} onClick={() => act("allow_once")}>
              <Check className="size-3.5" /> Send answers
            </button>
            <button className="btn-ghost text-fg-2" disabled={busy} onClick={() => act("deny")}>
              <X className="size-3.5" /> Skip
            </button>
          </div>
          {err && <div className="text-[12px] text-bad">{err}</div>}
        </div>
      ) : (
        status && (
          <div className="mt-2 flex items-center gap-1.5 font-mono text-[11.5px] text-fg-3">
            <X className="size-3.5" /> {status}
          </div>
        )
      )}
    </div>
  );
}

function ToolCard({ req, compact }: { req: PermissionRequest; compact: boolean }) {
  const [rule, setRule] = useState(req.rule);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const pending = req.status === "pending";
  const detail = describeInput(req);

  const act = async (d: "allow_once" | "allow_always" | "deny") => {
    setBusy(true);
    setErr(null);
    try {
      await api.decide(req.id, d, rule);
    } catch (e) {
      setErr(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };

  const res = RESOLVED[req.status];
  return (
    <div
      id={`perm-${req.id}`}
      className={cx(
        "scroll-mt-4 rounded-2xl border p-4",
        pending ? "border-warn/30 bg-warn/[0.05] shadow-[0_0_0_4px_rgb(242_196_109/0.04)]" : "border-line bg-ink-900/60",
      )}
    >
      <div className="flex items-center gap-2 text-[13px]">
        <ShieldAlert className={cx("size-4", pending ? "text-warn" : "text-fg-3")} />
        <span className="text-fg">
          {req.source === "request" ? "Requests permission" : "Wants to use"} <code className="font-mono text-[12.5px] text-accent">{req.tool_name}</code>
        </span>
        <span className="ml-auto font-mono text-[11px] text-fg-3">{timeAgo(req.created_at)}</span>
      </div>
      {req.reason && <p className="mt-2 text-[13px] text-fg-2">“{req.reason}”</p>}
      {detail && !compact && (
        <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-line bg-ink-950 px-3 py-2 font-mono text-[12px] text-fg-2">{detail}</pre>
      )}
      {pending ? (
        <div className="mt-3 space-y-2.5">
          <label className="flex items-center gap-2">
            <span className="eyebrow shrink-0">Rule</span>
            <input className="input py-1 font-mono text-[12px]" value={rule} onChange={(e) => setRule(e.target.value)} />
          </label>
          <div className="flex flex-wrap gap-2">
            {req.source === "prompt" && (
              <button className="btn" disabled={busy} onClick={() => act("allow_once")}>
                <Check className="size-3.5" /> Allow once
              </button>
            )}
            <button className="btn-primary" disabled={busy} onClick={() => act("allow_always")}>
              <CheckCheck className="size-3.5" /> {req.source === "prompt" ? "Always allow" : "Grant"}
            </button>
            <button className="btn-ghost text-bad hover:text-bad" disabled={busy} onClick={() => act("deny")}>
              <X className="size-3.5" /> Deny
            </button>
          </div>
          {err && <div className="text-[12px] text-bad">{err}</div>}
        </div>
      ) : (
        res && (
          <div className={cx("mt-2 flex items-center gap-1.5 font-mono text-[11.5px]", res.cls)}>
            {res.icon} {res.label}
            {req.status === "allowed_always" && <code className="ml-1 text-fg-2">{req.rule}</code>}
          </div>
        )
      )}
    </div>
  );
}
