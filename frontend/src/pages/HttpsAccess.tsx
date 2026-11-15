import { useEffect, useState } from "react";
import { Lock } from "lucide-react";
import { api, type Https } from "../lib/api";

/** Settings → Https: Loft's https address on the tailnet, which browsers need before they allow the microphone. */
export function HttpsAccess() {
  const [https, setHttps] = useState<Https | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      setHttps(await api.https());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    // Talk links here when the page isn't on https: the sections above load after this one, so scroll once it's checked.
    run().then(() => window.location.hash === "#https" && setTimeout(() => document.getElementById("https")?.scrollIntoView({ block: "start" }), 500));
  }, []);

  return (
    <section id="https" className="panel scroll-mt-4 space-y-4 p-5">
      <div className="flex items-start gap-3">
        <div className="flex-1">
          <h2 className="text-[15px] font-medium tracking-tight">Https on the tailnet</h2>
          <p className="mt-1 text-[12.5px] text-fg-3">
            Browsers allow the microphone only over https. Tailscale serves Loft at its https address, with a certificate, to your devices
            on the tailnet only.
          </p>
        </div>
        <button className="btn shrink-0" disabled={busy} onClick={run}>
          <Lock className="size-3.5" /> {busy ? "Checking…" : "Check again"}
        </button>
      </div>
      {err && <p className="text-[12.5px] text-bad">{err}</p>}
      {https?.url && (
        <p className="text-[13px]">
          On: <a className="break-all font-mono text-accent underline" href={https.url}>{https.url}</a>
        </p>
      )}
      {https && !https.url && (
        <div className="space-y-2 text-[12.5px] text-fg-2">
          <p className="text-fg">Off. To turn it on:</p>
          <ol className="list-decimal space-y-1 pl-5">
            <li>
              In the Tailscale admin console, DNS → turn on MagicDNS and HTTPS Certificates (
              <span className="break-all font-mono text-[11.5px]">login.tailscale.com/admin/dns</span>).
            </li>
            <li>
              On this Mac, run <span className="break-all font-mono text-[11.5px] text-fg">{https.command}</span>.
            </li>
            <li>
              Press Check again, then open Loft at its https address
              {https.host && <> (<span className="break-all font-mono text-[11.5px]">https://{https.host}</span>)</>}.
            </li>
          </ol>
        </div>
      )}
    </section>
  );
}
