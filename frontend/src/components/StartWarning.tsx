import { useState } from "react";
import { TriangleAlert } from "lucide-react";
import { api, type WaitsOn } from "../lib/api";
import { specsToStartFirst } from "../lib/waitsOn";
import { Callout, ErrorLine } from "./Callout";
import { ActionButton } from "./ActionButton";
import { useAction } from "../lib/useAction";
import { startDeliveryLabel } from "../lib/startDelivery";

/** Shown instead of starting a Delivery whose tickets wait on open tickets in another spec: says what they wait on and
 *  offers to start the other spec first (or to start anyway: the waiting tickets stay waiting until those close). */
export function StartWarning({ waits, onStart, startPending, onCancel, onStarted }: { waits: WaitsOn[]; onStart: () => void; startPending: boolean; onCancel: () => void; onStarted: () => void }) {
  const starting = useAction();
  // The spec being started, so its button shows the spinner.
  const [picked, setPicked] = useState<string | null>(null);
  const busy = starting.busy || startPending;
  const first = specsToStartFirst(waits);
  const startFirst = (id: string, key: string) => {
    if (busy) return;
    setPicked(key);
    return starting.run(async () => {
      await api.startIssueDelivery(id);
      onStarted();
    });
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
            <ActionButton
              key={`${f.repo}#${f.spec}`}
              className="text-[12px]"
              pending={busy && picked === `${f.repo}#${f.spec}`}
              pendingLabel={`Starting ${f.repo}#${f.spec}…`}
              disabled={busy}
              onClick={() => startFirst(f.issueId, `${f.repo}#${f.spec}`)}
            >
              Start {f.repo}#{f.spec} first
            </ActionButton>
          ))}
          <ActionButton variant="secondary" className="text-[12px]" pending={startPending} pendingLabel={startDeliveryLabel(true)} disabled={busy} onClick={onStart}>Start anyway</ActionButton>
          <button className="btn text-[12px]" disabled={busy} onClick={onCancel}>Cancel</button>
        </div>
        <ErrorLine error={starting.error} />
      </div>
    </Callout>
  );
}
