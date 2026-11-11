import { useEffect, useState } from "react";
import { Monitor, Smartphone } from "lucide-react";
import { api, type PairedClient } from "../lib/api";
import { timeAgo } from "../lib/format";

/** Settings → Paired devices: every browser and iPhone paired with Loft. Forget this device unpairs one; Unpair all every one but this. */
export function PairedClients() {
  const [clients, setClients] = useState<PairedClient[]>([]);
  const [confirming, setConfirming] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const load = () => {
    api.pairedClients().then(setClients, (e) => setErr((e as Error).message));
  };
  useEffect(load, []);

  // Forgetting this browser makes the reload a 401, which shows the pairing screen.
  const forget = (c: PairedClient) => api.forgetClient(c.id).catch((e) => setErr((e as Error).message)).finally(load);
  const unpairAll = () => {
    setConfirming(false);
    api.unpairAll().catch((e) => setErr((e as Error).message)).finally(load);
  };
  const others = clients.filter((c) => !c.this).length;

  return (
    <section className="panel space-y-4 p-5">
      <div>
        <h2 className="text-[15px] font-medium tracking-tight">Paired devices</h2>
        <p className="mt-1 text-[12.5px] text-fg-3">Browsers and phones that can change Loft. An unpaired one goes back to its pairing screen on its next request.</p>
      </div>
      <div className="space-y-2">
        {clients.map((c) => (
          <div key={c.id} className="flex items-center gap-3 rounded-xl border border-line px-3 py-2">
            {/iPhone|iOS/.test(c.name) ? <Smartphone className="size-4 shrink-0 text-fg-3" /> : <Monitor className="size-4 shrink-0 text-fg-3" />}
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px]">
                {c.name}
                {c.this && <span className="chip ml-2">This device</span>}
              </div>
              <div className="truncate font-mono text-[10.5px] text-fg-3">
                paired {timeAgo(c.paired_at)} · used {timeAgo(c.last_used_at)}
              </div>
            </div>
            <button type="button" className="btn-ghost shrink-0 text-fg-3 hover:text-bad" onClick={() => forget(c)}>
              Forget this device
            </button>
          </div>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {confirming ? (
          <>
            <span className="text-[13px] text-fg-2">
              Unpair {others} other device{others === 1 ? "" : "s"}?
            </span>
            <button type="button" className="btn text-bad" onClick={unpairAll}>
              Unpair
            </button>
            <button type="button" className="btn-ghost" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </>
        ) : (
          <button type="button" className="btn" disabled={others === 0} onClick={() => setConfirming(true)}>
            Unpair all
          </button>
        )}
        {err && <span className="text-[12px] text-bad">{err}</span>}
      </div>
    </section>
  );
}
