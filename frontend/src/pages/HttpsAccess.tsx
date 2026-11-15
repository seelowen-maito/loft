import { Lock } from "lucide-react";
import { api } from "../lib/api";
import { useCheck } from "../lib/useCheck";
import { CheckButton, SettingsCard } from "../components/SettingsCard";

/** Settings → Https: Loft's https address on the tailnet, which browsers need before they allow the microphone. */
export function HttpsAccess() {
  const { data: https, busy, err, run, loading } = useCheck(api.https);
  return (
    <SettingsCard
      id="https"
      title="Https on the tailnet"
      description="Browsers allow the microphone only over https. Tailscale serves Loft at its https address, with a certificate, to your devices on the tailnet only."
      action={<CheckButton busy={busy} onClick={run} icon={Lock} />}
      error={err}
      loading={loading}
    >
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
    </SettingsCard>
  );
}
