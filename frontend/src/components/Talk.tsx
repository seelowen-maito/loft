import { useEffect, useReducer, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { Check, Loader2, Mic, MicOff, Search } from "lucide-react";
import { api, type Agent } from "../lib/api";
import { DRAFT_SET, loadDraft, saveDraft } from "../lib/draft";
import { cx } from "../lib/format";
import { loadLastAgent } from "../lib/lastAgent";
import { appendDraft, chipLabel, chips, micBlocked, next, OPEN_TALK, pcm16k, sections, START, starter } from "../lib/talk";

/** How often the audio so far goes to the Mac to be transcribed. */
const CHUNK_MS = 1000;

/** Talk in the browser, like the Mac's voice panel: one dialog for the whole page, opened by `openTalk` (the top
 * bar's mic, a composer's mic). The browser records; the Mac transcribes, tidies, and the text goes to the agent. */
export function Talk() {
  const [open, setOpen] = useState(false);
  const [wanted, setWanted] = useState<string | null>(null);
  useEffect(() => {
    const onOpen = (e: Event) => {
      setWanted((e as CustomEvent<string | null>).detail);
      setOpen(true);
    };
    window.addEventListener(OPEN_TALK, onOpen);
    return () => window.removeEventListener(OPEN_TALK, onOpen);
  }, []);
  return open ? <TalkDialog wanted={wanted} close={() => setOpen(false)} /> : null;
}

/** The microphone, turned into 16 kHz PCM blocks, and how loud it is. */
type Recorder = { take: () => Uint8Array; stop: () => void };

async function record(onLevel: (level: number) => void): Promise<Recorder> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  const ctx = new AudioContext();
  const source = ctx.createMediaStreamSource(stream);
  // ScriptProcessor is old but everywhere, and needs no worklet file; it outputs silence.
  const proc = ctx.createScriptProcessor(4096, 1, 1);
  let blocks: Uint8Array[] = [];
  proc.onaudioprocess = (e) => {
    const input = e.inputBuffer.getChannelData(0);
    blocks.push(pcm16k(input, ctx.sampleRate));
    let sum = 0;
    for (const v of input) sum += v * v;
    onLevel(Math.min(1, Math.sqrt(sum / input.length) * 6));
  };
  source.connect(proc);
  proc.connect(ctx.destination);
  return {
    take: () => {
      const out = new Uint8Array(blocks.reduce((n, b) => n + b.length, 0));
      let at = 0;
      for (const b of blocks) out.set(b, (at += b.length) - b.length);
      blocks = [];
      return out;
    },
    stop: () => {
      proc.disconnect();
      source.disconnect();
      stream.getTracks().forEach((t) => t.stop());
      ctx.close().catch(() => {});
    },
  };
}

function TalkDialog({ wanted, close }: { wanted: string | null; close: () => void }) {
  const navigate = useNavigate();
  const [step, dispatch] = useReducer(next, START);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [to, setTo] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const [query, setQuery] = useState("");
  const [level, setLevel] = useState(0);
  const [seconds, setSeconds] = useState(0);
  const [needsHttps, setNeedsHttps] = useState(false);
  // The recording in progress: its session on the Mac, the microphone, and the chunks sent one after another.
  const rec = useRef<{ session: string; mic: Recorder | null; timer: number; queue: Promise<string>; ended: boolean } | null>(null);

  useEffect(() => {
    api.agents().then((as) => {
      setAgents(as);
      setTo((t) => t ?? starter(as, wanted, loadLastAgent()));
    }).catch(() => {});
  }, [wanted]);

  /** Sends the audio recorded since the last chunk; the final one ends the session on the Mac. */
  const flush = (final: boolean) => {
    const r = rec.current;
    if (!r || r.ended) return r?.queue ?? Promise.resolve("");
    r.ended = final;
    // Taken now, so the chunks keep their order even while an earlier one is still being transcribed.
    const audio = r.mic?.take() ?? new Uint8Array();
    r.queue = r.queue.then(async () => {
      if (!final && audio.length === 0) return "";
      const { text } = await api.transcribe(r.session, audio, final);
      if (rec.current === r) dispatch({ type: "heard", text });
      return text;
    });
    return r.queue;
  };

  /** Stops the microphone; `discard` also deletes the session's audio on the Mac. */
  const stopRecording = (discard: boolean) => {
    const r = rec.current;
    if (!r) return;
    clearInterval(r.timer);
    r.mic?.stop();
    r.mic = null;
    if (discard) {
      if (!r.ended) api.transcribe(r.session, new Uint8Array(), true).catch(() => {});
      rec.current = null;
    }
  };

  // Preparing: ask for the microphone and start a session, on opening and on each Try again (`attempt`). The
  // recording stops when Talk closes or starts over, not when the step moves on.
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const blocked = micBlocked(window.isSecureContext, !!navigator.mediaDevices?.getUserMedia);
    setNeedsHttps(!window.isSecureContext);
    if (blocked) {
      dispatch({ type: "failed", message: blocked });
      return;
    }
    const r = { session: crypto.randomUUID(), mic: null as Recorder | null, timer: 0, queue: Promise.resolve(""), ended: false };
    rec.current = r;
    let cancelled = false;
    record(setLevel).then(
      (mic) => {
        if (cancelled) return mic.stop();
        r.mic = mic;
        const started = Date.now();
        r.timer = window.setInterval(() => {
          setSeconds(Math.floor((Date.now() - started) / 1000));
          flush(false).catch((e) => fail((e as Error).message));
        }, CHUNK_MS);
        dispatch({ type: "listening" });
      },
      (e: Error) => {
        if (cancelled) return;
        rec.current = null;
        const denied = e.name === "NotAllowedError" || e.name === "SecurityError";
        dispatch({ type: "failed", message: denied ? "The browser blocked the microphone. Allow it for this site, then try again." : e.message });
      },
    );
    return () => {
      cancelled = true;
      stopRecording(true);
    };
  }, [attempt]);

  const fail = (message: string) => {
    stopRecording(true);
    dispatch({ type: "failed", message });
  };

  /** Done talking: the last audio goes, and its final text comes back. */
  const done = () => {
    dispatch({ type: "stop" });
    const r = rec.current;
    const last = flush(true);
    stopRecording(false);
    last.then(
      (text) => {
        if (rec.current === r) dispatch({ type: "final", text });
        rec.current = null;
      },
      (e) => fail((e as Error).message),
    );
  };

  // Cleaning: Loft tidies the text, or keeps it as heard when it can't.
  const heard = step.kind === "cleaning" ? step.heard : null;
  useEffect(() => {
    if (heard == null) return;
    let live = true;
    api.voiceClean(heard).then(
      (r) => live && dispatch({ type: "cleaned", text: r.text }),
      () => live && dispatch({ type: "cleaned", text: heard }),
    );
    return () => {
      live = false;
    };
  }, [heard]);

  // Sending: the existing send path, then the agent's chat.
  const sending = step.kind === "sending" ? step.text : null;
  useEffect(() => {
    if (sending == null || !to) return;
    api.send(to, sending).then(
      () => {
        dispatch({ type: "sent" });
        navigate(`/agents/${to}`);
        close();
      },
      (e: Error) => dispatch({ type: "failed", message: `Couldn't send: ${e.message}` }),
    );
  }, [sending]);

  const cancel = () => {
    stopRecording(true);
    close();
  };

  const editInChat = () => {
    if (step.kind !== "review" || !to) return;
    const d = loadDraft(to);
    saveDraft(to, { ...d, text: appendDraft(d.text, step.text.trim()) });
    window.dispatchEvent(new CustomEvent(DRAFT_SET, { detail: to }));
    navigate(`/agents/${to}`);
    close();
  };

  // Esc closes More first, then cancels Talk: nothing is sent.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.isComposing) return;
      e.preventDefault();
      if (picking) setPicking(false);
      else cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const shown = chips(agents, to);
  const toName = agents.find((a) => a.id === to)?.name ?? "the agent";
  const retryText = step.kind === "failed" && step.text != null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={cancel}>
      <div role="dialog" aria-label="Talk" className="panel flex h-[420px] w-full max-w-[520px] flex-col gap-4 bg-ink-900 p-5" onClick={(e) => e.stopPropagation()}>
        <div className="relative flex items-center gap-2">
          <span className="text-[13px] text-fg-3">To</span>
          <div className="flex min-w-0 flex-1 gap-1.5 overflow-x-auto">
            {shown.map((a) => (
              <button
                key={a.id}
                onClick={() => setTo(a.id)}
                className={cx("shrink-0 rounded-full px-2.5 py-1 text-[12.5px]", a.id === to ? "bg-accent/25 font-medium text-fg" : "bg-white/[0.06] text-fg-2 hover:text-fg")}
              >
                {chipLabel(a, shown)}
              </button>
            ))}
          </div>
          <button className="btn-ghost shrink-0" onClick={() => setPicking((p) => !p)}>
            More
          </button>
          {picking && (
            <div className="absolute right-0 top-9 z-10 w-72 space-y-2 rounded-xl border border-line-strong bg-ink-850 p-2 shadow-2xl">
              <div className="relative">
                <Search className="absolute left-2.5 top-2.5 size-3.5 text-fg-3" />
                <input autoFocus className="input pl-8" placeholder="Name, role, group or project" value={query} onChange={(e) => setQuery(e.target.value)} />
              </div>
              <div className="max-h-60 overflow-y-auto">
                {sections(agents, query).map((s) => (
                  <div key={s.title}>
                    <div className="eyebrow px-2 pb-1 pt-2">{s.title}</div>
                    {s.agents.map((a) => (
                      <button
                        key={a.id}
                        className="flex w-full items-center rounded-lg px-2 py-1.5 text-left text-[13px] hover:bg-white/5"
                        onClick={() => {
                          setTo(a.id);
                          setPicking(false);
                        }}
                      >
                        <span className="flex-1">{a.name}</span>
                        {a.id === to && <Check className="size-3.5 text-accent" />}
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        <div className="flex min-h-0 flex-1 flex-col justify-center">
          {step.kind === "failed" ? (
            <div className="space-y-2 text-center">
              <MicOff className="mx-auto size-9 text-fg-3" />
              <p className="text-[15px] font-medium">{retryText ? "Not sent" : "Voice stopped"}</p>
              <p className="text-[13px] text-fg-2">{step.message}</p>
              {needsHttps && (
                <a className="text-[13px] text-accent underline" href="/settings#https">
                  Settings → Https on the tailnet
                </a>
              )}
            </div>
          ) : step.kind === "cleaning" || step.kind === "finishing" ? (
            <p className="flex items-center justify-center gap-2 text-[13px] text-fg-3">
              <Loader2 className="size-4 animate-spin" /> Tidying up…
            </p>
          ) : step.kind === "review" || step.kind === "sending" ? (
            <textarea
              aria-label="Text to send"
              className="input h-full resize-none text-[14px]"
              value={step.text}
              disabled={step.kind === "sending"}
              onChange={(e) => dispatch({ type: "edit", text: e.target.value })}
            />
          ) : (
            <div className="flex h-full flex-col gap-4">
              <p className={cx("flex-1 overflow-hidden text-[17px] font-medium", step.kind === "listening" && step.heard ? "text-fg" : "text-fg-3")}>
                {step.kind === "listening" && step.heard ? step.heard : step.kind === "preparing" ? "Getting ready…" : "Start talking…"}
              </p>
              <Bars level={step.kind === "listening" ? level : 0} />
              <p className="text-center font-mono text-[11px] text-fg-3">{step.kind === "listening" ? `Listening · ${seconds}s` : " "}</p>
            </div>
          )}
        </div>

        <div className="flex items-center gap-2">
          <button className="btn-ghost" onClick={cancel}>
            Cancel
          </button>
          <span className="flex-1" />
          {step.kind === "listening" && (
            <button className="btn-primary" onClick={done}>
              <Mic className="size-3.5" /> Done
            </button>
          )}
          {(step.kind === "review" || step.kind === "sending") && (
            <>
              <button className="btn" disabled={step.kind === "sending"} onClick={editInChat}>
                Edit in chat
              </button>
              <button className="btn-primary" disabled={step.kind === "sending" || !step.text.trim() || !to} onClick={() => dispatch({ type: "send" })}>
                {step.kind === "sending" ? "Sending…" : `Send to ${toName}`}
              </button>
            </>
          )}
          {step.kind === "failed" && !needsHttps && (
            <button
              className="btn-primary"
              onClick={() => {
                dispatch({ type: "retry" });
                if (!retryText) setAttempt((a) => a + 1);
              }}
            >
              {retryText ? "Retry sending" : "Try again"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/** Bars that rise with the voice. */
function Bars({ level }: { level: number }) {
  return (
    <div className="flex h-14 items-center justify-center gap-1" aria-hidden>
      {Array.from({ length: 21 }, (_, i) => {
        const centre = 1 - Math.abs(i - 10) / 12;
        return <span key={i} className="w-[5px] rounded-full bg-accent transition-[height] duration-100" style={{ height: 6 + 44 * level * centre, opacity: 0.55 + 0.45 * centre }} />;
      })}
    </div>
  );
}
