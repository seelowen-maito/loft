import { useEffect, useState } from "react";
import { Terminal } from "lucide-react";
import { api, type ClaudeLogin } from "../lib/api";

/** Settings → Claude login: whether the claude CLI is logged in and as whom, with Open Terminal to log in. */
export function ClaudeLoginSection() {
  const [s, setS] = useState<ClaudeLogin | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const load = () => api.claudeLogin().then(setS).catch((e) => setErr((e as Error).message));
  useEffect(() => void load(), []);
  const open = async () => {
    setBusy(true);
    setErr(null);
    try {
      await api.claudeLoginTerminal();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="panel space-y-3 p-5">
      <div className="flex items-start gap-3">
        <div className="flex-1">
          <h2 className="text-[15px] font-medium tracking-tight">Claude login</h2>
          <p className="mt-1 text-[12.5px] text-fg-3">The claude CLI every agent runs on. Loft can't log in by itself: logging in needs a browser and a terminal.</p>
        </div>
        <button className="btn shrink-0" disabled={busy} onClick={open}>
          <Terminal className="size-3.5" /> Open Terminal
        </button>
      </div>
      {s && (
        <p className={s.logged_in ? "text-[13px] text-good" : "text-[13px] text-bad"}>
          {s.logged_in ? `Logged in${s.account ? ` as ${s.account}` : ""}${s.plan ? ` · ${s.plan} plan` : ""}` : "Not logged in"}
        </p>
      )}
      {s && !s.logged_in && (
        <p className="text-[12.5px] text-fg-2">
          {s.note} {s.steps}
        </p>
      )}
      {err && <p className="text-[12.5px] text-bad">{err}</p>}
    </section>
  );
}
