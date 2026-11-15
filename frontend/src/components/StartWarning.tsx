import { useState } from "react";
import { Loader2, TriangleAlert } from "lucide-react";
import { api, type WaitsOn } from "../lib/api";
import { specsToStartFirst } from "../lib/waitsOn";
import { Callout } from "./Callout";

/** Shown instead of starting a Delivery whose tickets wait on open tickets in another spec: says what they wait on and
 *  offers to start the other spec first (or to start anyway: the waiting tickets stay waiting until those close). */
export function StartWarning({ waits, onStart, onCancel, onStarted }: { waits: WaitsOn[]; onStart: () => void; onCancel: () => void; onStarted: () => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const first = specsToStartFirst(waits);
  const startFirst = async (id: string, key: string) => {
    setBusy(key);
    setError(null);
    try {
      await api.startIssueDelivery(id);
      onStarted();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  return (
    <Callout tone="warn" icon={<TriangleAlert />} className="mt-3">
      <div className="space-y-2">
        <div>
          Tickets of this spec wait on <span className="font-mono text-fg">{waits.map((w) => w.blocker).join(", ")}</span>, which {waits.length === 1 ? "isn't" : "aren't"} done yet.
          {first.length > 0 ? " Start the spec that builds it first, or start this one and let those tickets wait." : " Start anyway and those tickets wait until it closes."}
        </div>
        <div className="flex flex-wrap gap-2">
          {first.map((f) => (
            <button key={`${f.repo}#${f.spec}`} className="btn-primary text-[12px]" disabled={busy !== null} onClick={() => startFirst(f.issueId, `${f.repo}#${f.spec}`)}>
              {busy === `${f.repo}#${f.spec}` && <Loader2 className="size-3.5 animate-spin" />} Start {f.repo}#{f.spec} first
            </button>
          ))}
          <button className="btn text-[12px]" disabled={busy !== null} onClick={onStart}>Start anyway</button>
          <button className="btn text-[12px]" disabled={busy !== null} onClick={onCancel}>Cancel</button>
        </div>
        {error && <div className="text-[12px] text-bad">{error}</div>}
      </div>
    </Callout>
  );
}
