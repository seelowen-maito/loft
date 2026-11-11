import { useEffect, useRef, useState } from "react";
import { BellRing, ExternalLink, KeyRound, Smartphone, Trash2 } from "lucide-react";
import { api, portalHome, type PushDevice, type PushStatus } from "../lib/api";
import { cx, timeAgo } from "../lib/format";

/** Settings → Push notifications: which path pushes take (the user's own APNs key, the Loft relay with a relay key, or none), the phones registered, and a test push. */
const CHIP: Record<PushStatus["path"], string> = { own_key: "Your own key", relay: "Using the Loft relay", none: "Sign in to get push" };
const WHERE: Record<PushStatus["path"], string> = {
  own_key: "Pushes go straight to Apple with your own key.",
  relay: "Pushes go through the Loft relay with your relay key.",
  none: "Nothing is pushed yet: sign in to get push (paste your relay key), or add your own APNs key.",
};

export function PushSettings() {
  const [status, setStatus] = useState<PushStatus | null>(null);
  const [devices, setDevices] = useState<PushDevice[]>([]);
  const [keyId, setKeyId] = useState("");
  const [teamId, setTeamId] = useState("");
  const [file, setFile] = useState<{ name: string; text: string } | null>(null);
  const [relayKey, setRelayKey] = useState("");
  const [relayAddr, setRelayAddr] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [portalLink, setPortalLink] = useState<string | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const section = useRef<HTMLElement>(null);

  const load = () => {
    api.pushStatus().then((s) => {
      setStatus(s);
      setKeyId(s.key_id);
      setTeamId(s.team_id);
      setRelayAddr(s.relay_address);
    });
    api.devices().then(setDevices);
  };
  useEffect(load, []);
  // `/settings#push` (Needs you's "Push is off") scrolls here once it shows.
  const shown = !!status;
  useEffect(() => {
    if (shown && window.location.hash === "#push") section.current?.scrollIntoView({ block: "start" });
  }, [shown]);
  // After Sign in the backend collects the key; look until it has it or the code is gone.
  const signingIn = status?.signing_in;
  useEffect(() => {
    if (!signingIn) return;
    const t = setInterval(load, 2000);
    return () => clearInterval(t);
  }, [signingIn]);
  useEffect(() => {
    if (status?.relay_key) setPortalLink(null);
  }, [status?.relay_key]);

  const signIn = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const { url } = await api.pushSignIn();
      setPortalLink(url);
      window.open(url, "_blank", "noopener");
      load();
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
      // A relay without a portal is remembered: Sign in goes away.
      load();
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const s = await api.configurePush({ key_id: keyId, team_id: teamId, key_p8: file?.text });
      setStatus(s);
      setFile(null);
      setMsg({ ok: true, text: s.path === "own_key" ? "Saved. Pushes go straight to Apple with your key." : "Saved." });
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const test = async (d: PushDevice) => {
    setMsg(null);
    try {
      await api.testPush(d.token);
      setMsg({ ok: true, text: `Sent to ${d.name || "the phone"}. It should arrive in a few seconds, even with the app closed.` });
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
      load();
    }
  };

  /** Sends a change to the push settings and says where pushes go now. */
  const change = async (body: Parameters<typeof api.configurePush>[0], done: string) => {
    setBusy(true);
    setMsg(null);
    try {
      const s = await api.configurePush(body);
      setStatus(s);
      setKeyId(s.key_id);
      setTeamId(s.team_id);
      setRelayKey("");
      setRelayAddr(s.relay_address);
      setMsg({ ok: true, text: `${done} ${WHERE[s.path]}` });
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const remove = async (d: PushDevice) => {
    await api.removeDevice(d.token).catch(() => {});
    load();
  };

  if (!status) return null;
  const changed = keyId !== status.key_id || teamId !== status.team_id || !!file;
  const off = status.path === "none";
  const addrChanged = relayAddr.trim() !== status.relay_address;
  const portal = portalHome(status);
  return (
    <section ref={section} id="push" className="panel scroll-mt-20 space-y-4 p-5">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-[15px] font-medium tracking-tight">Push notifications</h2>
        <span className={cx("chip text-[10.5px]", off ? "text-fg-3" : "text-good")}>{CHIP[status.path]}</span>
      </div>
      <p className="text-[12.5px] text-fg-2">Alerts reach the iPhone app even when it's closed. {WHERE[status.path]}</p>
      {off && status.devices > 0 && (
        <p role="alert" className="rounded-lg border border-warn/30 bg-warn/10 px-3 py-2 text-[12.5px] text-warn">
          Push is off: Loft drops every alert for {status.devices === 1 ? "your paired phone" : `your ${status.devices} paired phones`}. Sign in below to get push through the
          Loft relay, or add your own APNs key.
        </p>
      )}

      <div className="space-y-1.5">
        <span className="eyebrow">Loft relay</span>
        {status.relay_key ? (
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-[11.5px] text-fg-3">
              A relay key is stored on this Mac (owner-only, never shown).{!status.relay_url && " The relay isn't set up: add its address below."}
            </span>
            <button type="button" className="btn" disabled={busy} onClick={() => change({ remove_relay_key: true }, "Relay key removed.")}>
              Remove relay key
            </button>
            {portal && (
              <a className="text-[12px] text-fg-3 underline hover:text-fg" href={portal} target="_blank" rel="noreferrer">
                Manage keys
              </a>
            )}
          </div>
        ) : (
          <>
            {status.relay_refused && <p className="text-[12.5px] text-bad">Your relay key no longer works: Sign in again. Pushes through the relay are paused.</p>}
            <p className="text-[12px] text-fg-3">
              Sign in to get push: the relay sends your alerts to Apple for you, with no Apple developer account. Agent names and titles pass through it
              and are never stored. {portal ? "Press Sign in and allow it on the portal; or paste a relay key." : "Paste the relay key you were given."}
            </p>
            {portal && (
              <div className="flex flex-wrap items-center gap-2">
                <button type="button" className="btn-primary" disabled={busy} onClick={signIn}>
                  <ExternalLink className="size-3.5" /> Sign in
                </button>
                {status.signing_in && (portalLink || status.signin_code) && (
                  <span className="text-[12px] text-fg-3">
                    {status.signin_code ? (
                      <>
                        Type <span className="font-mono text-[13px] tracking-[0.2em] text-fg">{status.signin_code}</span> on the portal, then Allow.{" "}
                      </>
                    ) : (
                      "Waiting for you to allow it on the portal. "
                    )}
                    {portalLink && (
                      <a className="underline hover:text-fg" href={portalLink} target="_blank" rel="noreferrer">
                        Open the page again
                      </a>
                    )}
                  </span>
                )}
              </div>
            )}
            <div className="flex gap-2">
              <input
                className="input font-mono"
                type="password"
                autoComplete="off"
                placeholder="lrk_…"
                value={relayKey}
                onChange={(e) => setRelayKey(e.target.value.trim())}
                aria-label="Relay key"
              />
              <button type="button" className="btn-primary shrink-0" disabled={!relayKey || busy} onClick={() => change({ relay_key: relayKey }, "Relay key saved.")}>
                Save relay key
              </button>
            </div>
          </>
        )}
      </div>

      <div className="space-y-1.5">
        <p className="text-[12px] text-fg-3">
          Where the relay and its portal are: your relay key and the alerts' text go to this address, so only use one you trust. https only.
        </p>
        <label className="block space-y-1.5">
          <span className="eyebrow">Relay address</span>
          <input
            className="input font-mono"
            placeholder={status.relay_url && !status.relay_address ? `${status.relay_url} (default)` : "https://…"}
            value={relayAddr}
            onChange={(e) => setRelayAddr(e.target.value)}
          />
        </label>
        <button type="button" className="btn" disabled={!addrChanged || busy} onClick={() => change({ relay_url: relayAddr }, "Address saved.")}>
          Save address
        </button>
      </div>

      <div className="space-y-1.5 border-t border-line pt-4">
        <span className="eyebrow">Your own key</span>
        <p className="text-[12px] text-fg-3">
          Sends straight to Apple, without the relay. Apple Developer portal → Keys → +, tick “Apple Push Notifications service”. Apple lets you download
          the <span className="font-mono">.p8</span> file once: keep a copy in your password manager.
        </p>
      </div>

      <div className="space-y-1.5">
        <span className="eyebrow">Auth key (.p8)</span>
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className="btn" onClick={() => picker.current?.click()}>
            <KeyRound className="size-3.5" /> {status.key_present ? "Replace key…" : "Choose .p8 file…"}
          </button>
          <span className="font-mono text-[11.5px] text-fg-3">
            {file ? file.name : status.key_present ? "A key is stored on this Mac (owner-only, never shown)." : "No key yet."}
          </span>
          <input
            ref={picker}
            type="file"
            accept=".p8"
            hidden
            onChange={async (e) => {
              const f = e.target.files?.[0];
              e.target.value = "";
              if (f) setFile({ name: f.name, text: await f.text() });
              // Apple names it AuthKey_<KEYID>.p8: fill the key id from it.
              const id = f?.name.match(/AuthKey_([A-Z0-9]{10})\.p8$/i)?.[1];
              if (id && !keyId) setKeyId(id.toUpperCase());
            }}
          />
        </div>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="block space-y-1.5">
          <span className="eyebrow">Key ID</span>
          <input className="input font-mono" placeholder="10 characters" value={keyId} onChange={(e) => setKeyId(e.target.value.toUpperCase())} />
        </label>
        <label className="block space-y-1.5">
          <span className="eyebrow">Team ID</span>
          <input className="input font-mono" placeholder="Membership details" value={teamId} onChange={(e) => setTeamId(e.target.value.toUpperCase())} />
        </label>
      </div>
      <div className="flex items-center gap-3">
        <button type="button" className="btn-primary" disabled={!changed || busy} onClick={save}>Save</button>
        {status.key_present && (
          <button type="button" className="btn" disabled={busy} onClick={() => change({ key_id: "", team_id: "", remove_key: true }, "Key removed.")}>
            Remove key
          </button>
        )}
        {msg && <span className={cx("text-[12px]", msg.ok ? "text-good" : "text-bad")}>{msg.text}</span>}
      </div>

      <div className="space-y-2 border-t border-line pt-4">
        <span className="eyebrow">Phones</span>
        {devices.length === 0 ? (
          <p className="text-[12.5px] text-fg-3">None yet. Open the iPhone app and allow notifications; it registers itself.</p>
        ) : (
          devices.map((d) => (
            <div key={d.token} className="flex items-center gap-3 rounded-xl border border-line px-3 py-2">
              <Smartphone className="size-4 shrink-0 text-fg-3" />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px]">{d.name || "iPhone"}</div>
                <div className="truncate font-mono text-[10.5px] text-fg-3">
                  {d.environment === "production" ? "TestFlight / App Store" : "Xcode build (sandbox)"} · seen {timeAgo(d.last_seen_at)} · {d.kinds.length} kinds
                </div>
              </div>
              <button type="button" className="btn shrink-0" disabled={off} onClick={() => test(d)} title={off ? "Add a relay key or your own key first" : undefined}>
                <BellRing className="size-3.5" /> Send test push
              </button>
              <button type="button" className="btn-ghost shrink-0 text-fg-3 hover:text-bad" onClick={() => remove(d)} aria-label="Remove this phone" title="Remove this phone">
                <Trash2 className="size-3.5" />
              </button>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
