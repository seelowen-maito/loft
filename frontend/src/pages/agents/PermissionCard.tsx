import { canAlwaysAllow } from "../../lib/permission";
import { useState } from "react";
import { Check, CheckCheck, MessageCircleQuestion, ShieldAlert, ShieldCheck, ShieldX, X } from "lucide-react";
import { api, type PermissionRequest } from "../../lib/api";
import { cx } from "../../lib/format";
import { relTime } from "../../lib/time";
import { useAction } from "../../lib/useAction";
import { ActionButton } from "../../components/ActionButton";
import { ErrorLine } from "../../components/Callout";
import { ASK_TOOL, answerFor, answersOf, questionsOf, toggle, typeOther, type Pick } from "../../lib/questions";

function describeInput(p: PermissionRequest) {
  const i = p.input ?? {};
  if (p.tool_name === ASK_TOOL) return questionsOf(i).map((q) => q.question).join("\n");
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

type Decision = "allow_once" | "allow_always" | "deny";

export function PermissionCard({ req, compact = false }: { req: PermissionRequest; compact?: boolean }) {
  if (req.source === "questions") return <QuestionsCard req={req} compact={compact} />;
  return <ToolCard req={req} compact={compact} />;
}

/** An agent's multiple-choice questions, once it may ask them: the picks go back as the tool's answers. */
function QuestionsCard({ req, compact }: { req: PermissionRequest; compact: boolean }) {
  const qs = questionsOf(req.input);
  const [picks, setPicks] = useState<Record<number, Pick>>({});
  const decide = useAction();
  const [choice, setChoice] = useState<"allow_once" | "deny" | null>(null);
  const busy = decide.busy;
  const pending = req.status === "pending";
  const answers = answersOf(qs, picks);
  const pick = (i: number) => picks[i] ?? { chosen: [], other: "" };
  const set = (i: number, p: Pick) => setPicks((ps) => ({ ...ps, [i]: p }));

  const act = (d: "allow_once" | "deny") => {
    if (busy) return;
    setChoice(d);
    return decide.run(() => api.decide(req.id, d, undefined, d === "deny" ? undefined : (answers ?? undefined)));
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
        <span className="ml-auto font-mono text-[11px] text-fg-3">{relTime(req.created_at)}</span>
      </div>
      <div className="mt-3 space-y-4">
        {qs.map((q, i) => {
          const multi = !!q.multiSelect;
          const p = pick(i);
          const answered = answerFor(req.input?.answers, q.question);
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
                answered && (
                  <div className="mt-1 flex items-center gap-1.5 text-[13px] text-good">
                    <Check className="size-3.5" /> {answered}
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
            <ActionButton icon={<Check className="size-3.5" />} pending={busy && choice === "allow_once"} pendingLabel="Sending…" disabled={busy || !answers} onClick={() => act("allow_once")}>
              Send answers
            </ActionButton>
            <ActionButton variant="ghost" className="text-fg-2" icon={<X className="size-3.5" />} pending={busy && choice === "deny"} pendingLabel="Skipping…" disabled={busy} onClick={() => act("deny")}>
              Skip
            </ActionButton>
          </div>
          <ErrorLine error={decide.error} />
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
  // A kill with no program to name has no rule to save: Allow once or Deny only. Its rule is Loft's, not editable.
  const noRule = !canAlwaysAllow(req);
  const decide = useAction();
  // Which button the running decision came from: it shows the spinner, the others just wait.
  const [choice, setChoice] = useState<Decision | null>(null);
  const pending = req.status === "pending";
  const detail = describeInput(req);

  const act = (d: Decision) => {
    if (decide.busy) return;
    setChoice(d);
    return decide.run(() => api.decide(req.id, d, rule));
  };
  const pendingOn = (d: Decision) => decide.busy && choice === d;

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
        <span className="ml-auto font-mono text-[11px] text-fg-3">{relTime(req.created_at)}</span>
      </div>
      {req.reason && <p className="mt-2 text-[13px] text-fg-2">“{req.reason}”</p>}
      {detail && !compact && (
        <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-line bg-ink-950 px-3 py-2 font-mono text-[12px] text-fg-2">{detail}</pre>
      )}
      {pending ? (
        <div className="mt-3 space-y-2.5">
          {!noRule && (
            <label className="flex items-center gap-2">
              <span className="eyebrow shrink-0">Rule</span>
              <input className="input py-1 font-mono text-[12px]" value={rule} readOnly={req.tool_name === "Kill"} onChange={(e) => setRule(e.target.value)} />
            </label>
          )}
          <div className="flex flex-wrap gap-2">
            {req.source === "prompt" && (
              <ActionButton variant="secondary" icon={<Check className="size-3.5" />} pending={pendingOn("allow_once")} pendingLabel="Allowing…" disabled={decide.busy} onClick={() => act("allow_once")}>
                Allow once
              </ActionButton>
            )}
            {!noRule && (
              <ActionButton icon={<CheckCheck className="size-3.5" />} pending={pendingOn("allow_always")} pendingLabel={req.source === "prompt" ? "Allowing…" : "Granting…"} disabled={decide.busy} onClick={() => act("allow_always")}>
                {req.source === "prompt" ? "Always allow" : "Grant"}
              </ActionButton>
            )}
            <ActionButton variant="ghost" className="text-bad hover:text-bad" icon={<X className="size-3.5" />} pending={pendingOn("deny")} pendingLabel="Denying…" disabled={decide.busy} onClick={() => act("deny")}>
              Deny
            </ActionButton>
          </div>
          <ErrorLine error={decide.error} />
        </div>
      ) : (
        res && (
          <div className={cx("mt-2 flex min-w-0 flex-wrap items-center gap-x-1.5 font-mono text-[11.5px]", res.cls)}>
            {res.icon} <span className="shrink-0">{res.label}</span>
            {req.status === "allowed_always" && <code className="max-w-full min-w-0 truncate text-fg-2" title={req.rule}>{req.rule}</code>}
          </div>
        )
      )}
    </div>
  );
}
