import { useState } from "react";
import { Loader2, TriangleAlert } from "lucide-react";
import { api, type WaitsOn } from "../lib/api";
import { specsToStartFirst } from "../lib/waitsOn";

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
    <div role="alert" className="mt-3 space-y-2 rounded-xl border border-warn/40 bg-warn/[0.06] px-3 py-2 text-[12.5px] text-fg-2">
      <div className="flex items-start gap-2">
        <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warn" />
        <div>
          Tickets of this spec wait on <span className="font-mono text-fg">{waits.map((w) => w.blocker).join(", ")}</span>, which {waits.length === 1 ? "isn't" : "aren't"} done yet.
          {first.length > 0 ? " Start the spec that builds it first, or start this one and let those tickets wait." : " Start anyway and those tickets wait until it closes."}
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        {first.map((f) => (
          <button key={`${f.repo}#${f.spec}`} className="btn btn-primary text-[12px]" disabled={busy !== null} onClick={() => startFirst(f.issueId, `${f.repo}#${f.spec}`)}>
            {busy === `${f.repo}#${f.spec}` && <Loader2 className="size-3.5 animate-spin" />} Start {f.repo}#{f.spec} first
          </button>
        ))}
        <button className="btn text-[12px]" disabled={busy !== null} onClick={onStart}>Start anyway</button>
        <button className="btn text-[12px]" disabled={busy !== null} onClick={onCancel}>Cancel</button>
      </div>
      {error && <div className="text-[12px] text-bad">{error}</div>}
    </div>
  );
}
