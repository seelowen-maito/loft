import { useState } from "react";
import { ShieldCheck } from "lucide-react";
import { api, type MacAccess as Check } from "../lib/api";
import { cx } from "../lib/format";

const STATE: Record<Check["items"][number]["state"], { text: string; cls: string }> = {
  allowed: { text: "allowed", cls: "text-good" },
  blocked: { text: "blocked", cls: "text-bad" },
  waiting: { text: "macOS is asking on the Mac", cls: "text-warn" },
  missing: { text: "not on this Mac", cls: "text-fg-3" },
};

/** Settings → Mac access: whether macOS lets Loft's agents into the folders it guards with a prompt. */
export function MacAccess() {
  const [check, setCheck] = useState<Check | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      setCheck(await api.macAccess());
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const fullDisk = check?.items.find((i) => i.id === "full_disk")?.state === "allowed";
  const needsHelp = check && !fullDisk && check.items.some((i) => i.state === "blocked" || i.state === "waiting");
  return (
    <section className="panel space-y-4 p-5">
      <div className="flex items-start gap-3">
        <div className="flex-1">
          <h2 className="text-[15px] font-medium tracking-tight">Mac access</h2>
          <p className="mt-1 text-[12.5px] text-fg-3">
            macOS asks before any app reads Desktop, Documents, Downloads or iCloud Drive. On a Mac with no screen nobody sees it, and an agent waits
            for an answer. Checking can show those prompts on the Mac, so check while you're at it or on Screen Sharing.
          </p>
        </div>
        <button className="btn shrink-0" disabled={busy} onClick={run}>
          <ShieldCheck className="size-3.5" /> {busy ? "Checking…" : check ? "Check again" : "Check"}
        </button>
      </div>
      {err && <p className="text-[12.5px] text-bad">{err}</p>}
      {check && (
        <>
          <ul className="divide-y divide-line rounded-xl border border-line">
            {check.items.map((i) => (
              <li key={i.id} className="flex items-center gap-3 px-3 py-2 text-[13px]">
                <span className="flex-1">
                  {i.label} <span className="font-mono text-[11px] text-fg-3">{i.folder}</span>
                </span>
                <span className={cx("font-mono text-[11.5px]", STATE[i.state].cls)}>{STATE[i.state].text}</span>
              </li>
            ))}
          </ul>
          {needsHelp && (
            <div className="space-y-2 text-[12.5px] text-fg-2">
              <p className="text-fg">To let agents in for good, give Loft Full Disk Access:</p>
              <ol className="list-decimal space-y-1 pl-5">
                <li>
                  Open the Mac's screen{check.host && <> with Screen Sharing: <a className="break-all font-mono text-accent underline" href={`vnc://${check.host}`}>vnc://{check.host}</a></>}.
                  A prompt that's waiting there can be answered with Allow.
                </li>
                <li>
                  System Settings → Privacy &amp; Security → Full Disk Access (
                  <span className="break-all font-mono text-[11.5px]">x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles</span>).
                </li>
                <li>
                  Press +, then ⇧⌘G and paste <span className="break-all font-mono text-[11.5px] text-fg">{check.binary ?? "the loft binary"}</span>. Turn it on.
                </li>
                <li>Come back and press Check again.</li>
              </ol>
            </div>
          )}
          {check.adhoc_signed && (
            <p className="text-[12px] text-warn">
              This Loft build isn't signed with a lasting identity, so macOS forgets what you allowed every time Loft updates.
            </p>
          )}
        </>
      )}
    </section>
  );
}
