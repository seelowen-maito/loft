import { useEffect, useRef, useState } from "react";
import { CornerDownLeft, Delete, House, Lock, Send } from "lucide-react";
import { api, type Simulator, type SimulatorInput } from "../lib/api";
import { cx } from "../lib/format";

type Devices = { interactive: boolean; devices: Simulator[] };
type Ripple = { id: number; x: number; y: number };

/** The iOS Simulator on the loft machine: live video, and touch/keyboard input when AXe is installed. */
export default function SimulatorPage() {
  const [data, setData] = useState<Devices | null>(null);
  const [udid, setUdid] = useState<string>("");

  useEffect(() => {
    const load = () => api.simulators().then(setData).catch(() => setData({ interactive: false, devices: [] }));
    load();
    const t = setInterval(load, 10_000);
    return () => clearInterval(t);
  }, []);

  const device = data?.devices.find((d) => d.udid === udid) ?? data?.devices[0];

  return (
    <div className="mx-auto flex h-full max-w-3xl flex-col items-center gap-4 px-4 py-5">
      <div className="flex w-full items-center gap-3">
        <h1 className="text-[15px] font-medium tracking-tight">Simulator</h1>
        {data && data.devices.length > 1 && (
          <select className="input ml-auto w-auto" value={device?.udid} onChange={(e) => setUdid(e.target.value)}>
            {data.devices.map((d) => (
              <option key={d.udid} value={d.udid}>{d.name} · {d.runtime}</option>
            ))}
          </select>
        )}
        {data?.devices.length === 1 && device && <span className="ml-auto font-mono text-[11px] text-fg-3">{device.name} · {device.runtime}</span>}
      </div>
      {data === null ? (
        <div className="skeleton aspect-[9/19.5] h-full max-h-[75vh] rounded-[2.5rem]" />
      ) : !device ? (
        <p className="text-fg-3">No simulator is running on this Mac. Ask an agent to build and run an iOS app, or boot one in Xcode.</p>
      ) : data.interactive ? (
        <LiveScreen key={device.udid} device={device} />
      ) : (
        <>
          <PolledScreen key={device.udid} device={device} />
          <p className="text-center text-[11.5px] text-fg-3">
            View only. To control it from here, install <a className="underline" href="https://github.com/cameroncooke/AXe" target="_blank" rel="noreferrer">AXe</a> on this Mac.
          </p>
        </>
      )}
    </div>
  );
}

function LiveScreen({ device }: { device: Simulator }) {
  const [attempt, setAttempt] = useState(0);
  const [down, setDown] = useState(false);
  const [ripples, setRipples] = useState<Ripple[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [text, setText] = useState("");
  const start = useRef<{ x: number; y: number; t: number } | null>(null);
  const img = useRef<HTMLImageElement>(null);

  const send = (input: SimulatorInput) =>
    api.simulatorInput(device.udid, input).then(() => setErr(null), (e) => setErr(e instanceof Error ? e.message : String(e)));

  // Where on the screen, as fractions; object-contain can letterbox the image inside its box.
  const at = (e: React.PointerEvent) => {
    const el = img.current!;
    const r = el.getBoundingClientRect();
    const ratio = el.naturalWidth && el.naturalHeight ? el.naturalWidth / el.naturalHeight : r.width / r.height;
    const w = Math.min(r.width, r.height * ratio);
    const h = w / ratio;
    const left = r.left + (r.width - w) / 2;
    const top = r.top + (r.height - h) / 2;
    return { x: (e.clientX - left) / w, y: (e.clientY - top) / h, px: e.clientX - r.left, py: e.clientY - r.top };
  };

  const onDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    const p = at(e);
    start.current = { x: p.x, y: p.y, t: performance.now() };
    setDown(true);
  };
  const onUp = (e: React.PointerEvent) => {
    const s = start.current;
    start.current = null;
    setDown(false);
    if (!s) return;
    const p = at(e);
    if (s.x < 0 || s.x > 1 || s.y < 0 || s.y > 1) return;
    const id = Date.now();
    setRipples((r) => [...r, { id, x: p.px, y: p.py }]);
    setTimeout(() => setRipples((r) => r.filter((x) => x.id !== id)), 500);
    const moved = Math.hypot(p.x - s.x, (p.y - s.y) * 2);
    if (moved < 0.02) send({ action: "tap", x: s.x, y: s.y });
    else send({ action: "swipe", x: s.x, y: s.y, x2: p.x, y2: p.y, duration: (performance.now() - s.t) / 1000 });
  };

  return (
    <>
      <div className="flex min-h-0 w-full flex-1 justify-center">
        <div className="relative h-full max-h-[75vh] max-w-full">
          <img
            ref={img}
            src={`/api/simulator/stream?udid=${device.udid}&n=${attempt}`}
            alt={`${device.name} screen`}
            draggable={false}
            onLoad={() => setErr(null)}
            onError={() => {
              setErr("Lost the video stream. Reconnecting…");
              setTimeout(() => setAttempt((n) => n + 1), 2000);
            }}
            onPointerDown={onDown}
            onPointerUp={onUp}
            onPointerCancel={() => { start.current = null; setDown(false); }}
            className={cx("h-full max-w-full cursor-pointer touch-none select-none rounded-[2.5rem] border border-line object-contain", down && "brightness-90")}
          />
          {ripples.map((r) => (
            <span
              key={r.id}
              className="pointer-events-none absolute size-8 -translate-x-1/2 -translate-y-1/2 animate-ping rounded-full bg-accent/60"
              style={{ left: r.x, top: r.y }}
            />
          ))}
        </div>
      </div>
      {err && <p className="text-[12px] text-bad">{err}</p>}
      <div className="flex w-full max-w-md flex-wrap items-center justify-center gap-2">
        <button className="btn" title="Home" onClick={() => send({ action: "button", name: "home" })}><House className="size-4" /></button>
        <button className="btn" title="Lock" onClick={() => send({ action: "button", name: "lock" })}><Lock className="size-4" /></button>
        <form
          className="flex min-w-0 flex-1 items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (text) send({ action: "type", text }).then(() => setText(""));
          }}
        >
          <input className="input min-w-0 flex-1" placeholder="Type into the focused field" value={text} onChange={(e) => setText(e.target.value)} />
          <button className="btn" title="Send text" disabled={!text}><Send className="size-4" /></button>
        </form>
        <button className="btn" title="Delete" onClick={() => send({ action: "key", code: 42 })}><Delete className="size-4" /></button>
        <button className="btn" title="Return" onClick={() => send({ action: "key", code: 40 })}><CornerDownLeft className="size-4" /></button>
      </div>
      <p className="text-[11.5px] text-fg-3">Tap or drag on the screen. Each action takes about a second to land.</p>
    </>
  );
}

/** Screenshots on a loop, when there's no video stream. */
function PolledScreen({ device }: { device: Simulator }) {
  const [frame, setFrame] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    let url: string | null = null;
    const tick = async () => {
      if (document.hidden) {
        timer = setTimeout(tick, 1000);
        return;
      }
      try {
        const res = await fetch(`/api/simulator/screenshot?udid=${device.udid}`, { cache: "no-store" });
        if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error ?? res.statusText);
        const next = URL.createObjectURL(await res.blob());
        if (stopped) return URL.revokeObjectURL(next);
        if (url) URL.revokeObjectURL(url);
        url = next;
        setFrame(next);
        setErr(null);
        timer = setTimeout(tick, 250);
      } catch (e) {
        if (stopped) return;
        setErr(e instanceof Error ? e.message : String(e));
        timer = setTimeout(tick, 3000);
      }
    };
    tick();
    return () => {
      stopped = true;
      clearTimeout(timer);
      if (url) URL.revokeObjectURL(url);
    };
  }, [device.udid]);

  return (
    <>
      {frame && <img src={frame} alt={`${device.name} screen`} className={cx("min-h-0 flex-1 rounded-[2.5rem] border border-line object-contain", err && "opacity-50")} />}
      {err && <p className="text-[12px] text-bad">{err}</p>}
    </>
  );
}
