import { useCallback, useEffect, useState } from "react";
import { Smartphone } from "lucide-react";
import { api } from "../lib/api";

type Request = { code: string; text: string };

/** A new browser or phone asks to pair: compare its code with the one on that device, then Allow or Deny. */
export function PairRequests() {
  const [list, setList] = useState<Request[]>([]);
  const refresh = useCallback(() => {
    api.pairRequests().then(setList).catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh]);
  const answer = (code: string, allow: boolean) => {
    setList((l) => l.filter((r) => r.code !== code));
    api.answerPairing(code, allow).catch(() => {}).finally(refresh);
  };
  if (!list.length) return null;
  return (
    <div className="border-b border-warn/30 bg-warn/10">
      {list.map((r) => (
        <div key={r.code} className="mx-auto flex max-w-[1600px] flex-wrap items-center gap-x-4 gap-y-2 px-5 py-2.5 max-md:px-4">
          <Smartphone className="size-4 text-warn" />
          <span className="text-[13px]">
            A new device wants to pair: <span className="font-mono text-[15px] tracking-[0.2em]">{r.code}</span>
            <span className="text-fg-3"> · {r.text.split(" · ")[0]}. Allow it only if this code matches the one on that device.</span>
          </span>
          <span className="ml-auto flex gap-2">
            <button className="btn-primary" onClick={() => answer(r.code, true)}>
              Allow
            </button>
            <button className="btn" onClick={() => answer(r.code, false)}>
              Deny
            </button>
          </span>
        </div>
      ))}
    </div>
  );
}
