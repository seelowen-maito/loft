import { canAlwaysAllow } from "../../lib/permission";
import { useState } from "react";
import { Check, CheckCheck, ShieldAlert, ShieldCheck, ShieldX, X } from "lucide-react";
import { api, type PermissionRequest } from "../../lib/api";
import { cx, timeAgo } from "../../lib/format";

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
  const [rule, setRule] = useState(req.rule);
  // A kill with no program to name has no rule to save: Allow once or Deny only. Its rule is Loft's, not editable.
  const noRule = !canAlwaysAllow(req);
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
          {!noRule && (
            <label className="flex items-center gap-2">
              <span className="eyebrow shrink-0">Rule</span>
              <input className="input py-1 font-mono text-[12px]" value={rule} readOnly={req.tool_name === "Kill"} onChange={(e) => setRule(e.target.value)} />
            </label>
          )}
          <div className="flex flex-wrap gap-2">
            {req.source === "prompt" && (
              <button className="btn" disabled={busy} onClick={() => act("allow_once")}>
                <Check className="size-3.5" /> Allow once
              </button>
            )}
            {!noRule && (
              <button className="btn-primary" disabled={busy} onClick={() => act("allow_always")}>
                <CheckCheck className="size-3.5" /> {req.source === "prompt" ? "Always allow" : "Grant"}
              </button>
            )}
            <button className="btn-ghost text-bad hover:text-bad" disabled={busy} onClick={() => act("deny")}>
              <X className="size-3.5" /> Deny
            </button>
          </div>
          {err && <div className="text-[12px] text-bad">{err}</div>}
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
