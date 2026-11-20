import { Mic } from "lucide-react";
import { api } from "../lib/api";
import { useCheck } from "../lib/useCheck";
import { CheckButton, SettingsCard } from "../components/SettingsCard";

/** Settings → Voice on this Mac: whether Talk in the browser can be transcribed here, and where its cleanup runs. */
export function VoiceOnThisMac() {
  const { data: status, busy, err, run, loading } = useCheck(api.voiceStatus);
  return (
    <SettingsCard
      id="voice"
      title="Voice on this Mac"
      description="Talk in the browser sends your voice to this Mac, which turns it into text with Apple's speech model, then tidies it up."
      action={<CheckButton busy={busy} onClick={run} icon={Mic} />}
      error={err}
      loading={loading}
    >
      {/* Only one answer shows: a failed re-check, not it above the old status. */}
      {status && !err && (
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
    </SettingsCard>
  );
}
