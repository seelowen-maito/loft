import { useCallback, useEffect, useState, type ReactNode } from "react";
import { UNPAIRED } from "../lib/api";
import { cancelPairing, isPaired, pollPairing, startPairing, type Pairing } from "../lib/pairing";

/** Shows the app once this browser is paired; until then, the "Pair this browser" screen with its code. */
export default function PairGate({ children }: { children: ReactNode }) {
  const [paired, setPaired] = useState<boolean | null>(null);

  useEffect(() => {
    isPaired().then(setPaired, () => setPaired(true)); // backend down: the app shows its own offline state
    const lost = () => setPaired(false);
    window.addEventListener(UNPAIRED, lost);
    return () => window.removeEventListener(UNPAIRED, lost);
  }, []);

  if (paired === null) return <div className="h-full" />;
  return paired ? <>{children}</> : <PairScreen onPaired={() => setPaired(true)} />;
}

function PairScreen({ onPaired }: { onPaired: () => void }) {
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [round, setRound] = useState(0);

  useEffect(() => {
    let stop = false;
    let current: Pairing | null = null;
    (async () => {
      try {
        current = await startPairing();
        if (stop) return void cancelPairing(current.secret);
        setPairing(current);
        setNote(null);
        while (!stop) {
          const status = await pollPairing(current.secret);
          if (stop) return;
          if (status === "allowed") return onPaired();
          if (status === "denied") return setNote("This device was not allowed.");
          if (status === "gone") return setNote("The code expired.");
          await new Promise((r) => setTimeout(r, 1500));
        }
      } catch (e) {
        if (!stop) setNote((e as Error).message);
      }
    })();
    return () => {
      stop = true;
      if (current) void cancelPairing(current.secret);
    };
  }, [round, onPaired]);

  const again = useCallback(() => {
    setPairing(null);
    setRound((n) => n + 1);
  }, []);

  return (
    <div className="flex h-full items-center justify-center px-4">
      <div className="card w-full max-w-sm p-8 text-center">
        <h1 className="text-lg font-medium tracking-tight">Pair this browser</h1>
        <p className="mt-2 text-[13.5px] text-fg-2">
          Approve this code from a device that's already paired with Loft. For the first device, approve it at the Mac.
        </p>
        <div className="my-6 font-mono text-5xl tracking-[0.3em] text-fg" aria-label="Pairing code">
          {pairing && !note ? pairing.code : "····"}
        </div>
        {note ? (
          <>
            <p className="text-[13px] text-bad">{note}</p>
            <button className="btn mt-4" onClick={again}>
              Try again
            </button>
          </>
        ) : (
          <p className="text-[12.5px] text-fg-3">Waiting for approval…</p>
        )}
      </div>
    </div>
  );
}
