import { ShieldCheck } from "lucide-react";
import { api, type MacAccess as Check } from "../lib/api";
import { useCheck } from "../lib/useCheck";
import { CheckButton, SettingsCard } from "../components/SettingsCard";
import { cx } from "../lib/format";

const STATE: Record<Check["items"][number]["state"], { text: string; cls: string }> = {
  allowed: { text: "allowed", cls: "text-good" },
  blocked: { text: "blocked", cls: "text-bad" },
  waiting: { text: "macOS is asking on the Mac", cls: "text-warn" },
  missing: { text: "not on this Mac", cls: "text-fg-3" },
};

/** Settings → Mac access: whether macOS lets Loft's agents into the folders it guards with a prompt. */
export function MacAccess() {
  const { data: check, busy, err, run, loading } = useCheck(api.macAccess, { manual: true }); // only on the button: checking can raise macOS prompts on the Mac
  const fullDisk = check?.items.find((i) => i.id === "full_disk")?.state === "allowed";
  const needsHelp = check && !fullDisk && check.items.some((i) => i.state === "blocked" || i.state === "waiting");
  return (
    <SettingsCard
      id="mac-access"
      title="Mac access"
      description="macOS asks before any app reads Desktop, Documents, Downloads or iCloud Drive. On a Mac with no screen nobody sees it, and an agent waits for an answer. Checking can show those prompts on the Mac, so check while you're at it or on Screen Sharing. Loft also watches macOS's privacy log for prompts in its own name (only those) and tells you in Needs you."
      action={<CheckButton busy={busy} onClick={run} icon={ShieldCheck} label={check ? "Check again" : "Check"} />}
      error={err}
      loading={loading}
    >
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
              This Loft build isn't signed with a lasting identity, so macOS forgets what you allowed every time Loft updates. To fix it,
              create a code-signing certificate named loft (Keychain Access → Certificate Assistant → Create a Certificate…, Certificate
              Type: Code Signing) and restart Loft.
            </p>
          )}
          {check.adhoc_signed === false && check.signed_by && (
            <p className="text-[12px] text-fg-3">Signed with “{check.signed_by}”, so macOS keeps what you allowed when Loft updates.</p>
          )}
        </>
      )}
    </SettingsCard>
  );
}
