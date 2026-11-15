import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { api } from "../lib/api";
import { useCheck } from "../lib/useCheck";
import { SettingsCard } from "../components/SettingsCard";
import { herdrInstallCommand, isLocalOrigin } from "../lib/herdr";

/** Settings → Herdr UI: every agent as an agent in Herdr UI, on this machine or another one. */
export function HerdrSetup() {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ done: string[]; message: string } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const check = useCheck(api.herdrStatus);
  const status = check.data;
  useEffect(() => {
    if (status?.linked) setOpen(true);
  }, [status?.linked]);

  const setUpHere = async () => {
    setBusy(true);
    setErr(null);
    setResult(null);
    try {
      const r = await api.herdrSetup();
      setResult(r);
      await check.run();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const origin = window.location.origin;
  const command = herdrInstallCommand(origin);
  const copy = () => {
    navigator.clipboard.writeText(command).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const here = status?.herdr
    ? `Herdr ${status.herdr.version?.replace(/^herdr\s*/, "") ?? ""} is installed here${status.linked ? " and shows Loft's agents" : ""}.`
    : status
      ? "Herdr isn't installed on this machine."
      : " ";

  return (
    <SettingsCard
      id="herdr"
      title="Herdr UI"
      description="See every agent in Herdr UI, the terminal multiplexer for coding agents: one tab per agent with its chat, slash commands, approvals and this inspector as a sidebar. Herdr shows each one as an agent, working, idle or waiting for you."
      error={err ?? check.err}
      loading={check.loading}
    >
      {!open ? (
        <button type="button" className="btn-primary" onClick={() => setOpen(true)}>
          Set up Herdr UI
        </button>
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          <div className="space-y-3 rounded-xl border border-line p-4">
            <h3 className="text-[13.5px] font-medium">On this machine</h3>
            <p className="text-[12.5px] text-fg-3">{here}</p>
            <p className="text-[12px] text-fg-3">
              Builds the Herdr UI client, adds Loft's plugin to Herdr and opens a Loft workspace with your agents. The first build takes a minute.
            </p>
            <button type="button" className="btn" disabled={busy || (status !== null && !status.herdr)} onClick={setUpHere}>
              {busy ? "Setting up…" : status?.linked ? "Set up again" : "Set up on this machine"}
            </button>
            {status && !status.herdr && (
              <p className="text-[12px] text-fg-3">
                Install it from <a className="text-accent hover:underline" href="https://herdr.dev" target="_blank" rel="noreferrer">herdr.dev</a>, then come back.
              </p>
            )}
            {result && (
              <ul className="space-y-1 text-[12.5px]">
                {result.done.map((d) => (
                  <li key={d} className="flex gap-1.5 text-fg-2"><Check className="mt-0.5 size-3.5 shrink-0 text-good" /> {d}</li>
                ))}
                <li className="text-fg">{result.message}</li>
              </ul>
            )}
          </div>

          <div className="space-y-3 rounded-xl border border-line p-4">
            <h3 className="text-[13.5px] font-medium">On another machine</h3>
            <p className="text-[12.5px] text-fg-3">Run this in a terminal on the machine where you use Herdr:</p>
            <div className="flex items-start gap-2">
              <code className="min-w-0 flex-1 break-all rounded-lg bg-ink-950 px-3 py-2 font-mono text-[11.5px] text-fg-2">{command}</code>
              <button type="button" className="btn shrink-0" onClick={copy} aria-label="Copy command">
                {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
              </button>
            </div>
            {isLocalOrigin(origin) ? (
              <p className="text-[12px] text-warn">
                You opened Loft at {origin}, which other machines can't reach. Open it by an address they can, such as its Tailscale name, to get the right command.
              </p>
            ) : (
              <p className="text-[12px] text-fg-3">
                That machine talks to this Loft at {origin}. It downloads a ready-made build when it has the same system as this one
                {status ? ` (${status.platform})` : ""}; otherwise it builds one with cargo.
              </p>
            )}
          </div>
        </div>
      )}
    </SettingsCard>
  );
}
