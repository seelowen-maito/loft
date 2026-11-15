import { useEffect, useState } from "react";
import { Mic } from "lucide-react";
import { api, type VoiceStatus } from "../lib/api";

/** Settings → Voice on this Mac: whether Talk in the browser can be transcribed here, and where its cleanup runs. */
export function VoiceOnThisMac() {
  const [status, setStatus] = useState<VoiceStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      setStatus(await api.voiceStatus());
    } catch (e) {
      // Only one answer shows: the failed re-check, not it above the old status.
      setStatus(null);
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    run();
  }, []);

  return (
    <section id="voice" className="panel scroll-mt-4 space-y-4 p-5">
      <div className="flex items-start gap-3">
        <div className="flex-1">
          <h2 className="text-[15px] font-medium tracking-tight">Voice on this Mac</h2>
          <p className="mt-1 text-[12.5px] text-fg-3">
            Talk in the browser sends your voice to this Mac, which turns it into text with Apple's speech model, then tidies it up.
          </p>
        </div>
        <button className="btn shrink-0" disabled={busy} onClick={run}>
          <Mic className="size-3.5" /> {busy ? "Checking…" : "Check again"}
        </button>
      </div>
      {err && <p className="text-[12.5px] text-bad [overflow-wrap:anywhere]">{err}</p>}
      {status && (
        <dl className="space-y-3 text-[13px] [overflow-wrap:anywhere]">
          <div>
            <dt className="text-fg-3">Speech recognition</dt>
            <dd className={status.speech.ready ? "text-good" : "text-bad"}>{status.speech.text}</dd>
          </div>
          <div>
            <dt className="text-fg-3">Cleanup</dt>
            <dd>
              <span className={status.cleanup.on_device ? "text-good" : "text-fg"}>On-device model: {status.cleanup.text}</span>
              <span className="block text-[12.5px] text-fg-3">
                {status.cleanup.on_device ? "Cleanup runs on this Mac." : "Cleanup uses Claude's Haiku instead; Talk still works."}
              </span>
            </dd>
          </div>
        </dl>
      )}
    </section>
  );
}
