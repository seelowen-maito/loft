import { useState } from "react";
import { Terminal } from "lucide-react";
import { api } from "../lib/api";
import { useCheck } from "../lib/useCheck";
import { SettingsCard } from "../components/SettingsCard";
import { usePoll } from "../hooks/usePoll";

/** Settings → Claude login: whether the claude CLI is logged in and as whom, with Open Terminal to log in. */
export function ClaudeLoginSection() {
  const { data: s, err, run, loading } = useCheck(api.claudeLogin);
  const [openErr, setOpenErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Read again while the page is open, so the status turns once the login in Terminal is done.
  usePoll(run, 5000);
  const open = async () => {
    setBusy(true);
    setOpenErr(null);
    try {
      await api.claudeLoginTerminal();
    } catch (e) {
      setOpenErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <SettingsCard
      id="claude-login"
      title="Claude login"
      description="The claude CLI every agent runs on. Loft can't log in by itself: logging in needs a browser and a terminal."
      action={
        <button className="btn shrink-0" disabled={busy} onClick={open}>
          <Terminal className="size-3.5" /> Open Terminal
        </button>
      }
      error={openErr ?? err}
      loading={loading}
    >
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
    </SettingsCard>
  );
}
