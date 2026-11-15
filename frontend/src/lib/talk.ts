/** Talk in the browser: the same steps and recipient rules as the Mac's voice panel (ios/LoftMac/Talk.swift and
 * LoftKit's TalkRecipients). The browser records; the Loft Mac transcribes (`/api/voice/transcribe`) and tidies
 * (`/api/voice/clean`). Pure, so the tests run it without a browser. */

/** What Talk needs of an agent. */
export type TalkAgent = { id: string; name: string; role?: string; group?: string; project?: string; projects?: string[]; last_run_at: string | null };

/** Opens Talk from anywhere (the top bar, a chat's composer): `detail` is the agent to start with, if any. */
export const OPEN_TALK = "loft:talk";

export function openTalk(agentId?: string) {
  window.dispatchEvent(new CustomEvent(OPEN_TALK, { detail: agentId ?? null }));
}

/** Who Talk starts with: the given agent, else whoever you talked to last, else a Lead, else the first agent. */
export function starter(agents: TalkAgent[], wanted: string | null, last: string | null): string | null {
  const has = (id: string | null) => id != null && agents.some((a) => a.id === id);
  if (has(wanted)) return wanted;
  if (has(last)) return last;
  return (agents.find((a) => a.role === "lead") ?? agents[0])?.id ?? null;
}

const ran = (a: TalkAgent) => (a.last_run_at ? Date.parse(a.last_run_at) : 0);

/** The To chips: `current`, then every Lead, then the others, each most recently run first. The others fill the row
 * up to `limit` chips; Leads are all shown, even past it. */
export function chips(agents: TalkAgent[], current: string | null, limit = 5): TalkAgent[] {
  const recent = [...agents].sort((a, b) => ran(b) - ran(a));
  const first = recent.filter((a) => a.id === current);
  const leads = recent.filter((a) => a.id !== current && a.role === "lead");
  const others = recent.filter((a) => a.id !== current && a.role !== "lead");
  return [...first, ...leads, ...others.slice(0, Math.max(0, limit - first.length - leads.length))];
}

/** A chip's name, with its group when another chip has the same name ("Lead · Dineout"). */
export function chipLabel(a: TalkAgent, shown: TalkAgent[]): string {
  const twin = shown.some((o) => o.id !== a.id && o.name === a.name);
  return twin && a.group ? `${a.name} · ${a.group}` : a.name;
}

/** More: the agents matching `query` (name, role, group or project), in sections by group (A to Z, no group last),
 * names A to Z. */
export function sections(agents: TalkAgent[], query: string): { title: string; agents: TalkAgent[] }[] {
  const q = query.trim().toLowerCase();
  const hits = agents.filter((a) => !q || [a.name, a.role ?? "", a.group ?? "", a.project ?? "", ...(a.projects ?? [])].some((f) => f.toLowerCase().includes(q)));
  const groups = new Map<string, TalkAgent[]>();
  for (const a of hits) {
    const g = (a.group ?? "").trim();
    groups.set(g, [...(groups.get(g) ?? []), a]);
  }
  return [...groups.keys()]
    .sort((a, b) => (a ? 0 : 1) - (b ? 0 : 1) || a.toLowerCase().localeCompare(b.toLowerCase()))
    .map((g) => ({ title: g || "No group", agents: groups.get(g)!.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" })) }));
}

/** Talk's steps, as on the Mac panel. `heard` is the text so far while listening. A failure keeps what retrying
 * needs: the text for a failed send, nothing to start over. */
export type Step =
  | { kind: "preparing" }
  | { kind: "listening"; heard: string }
  | { kind: "finishing"; heard: string }
  | { kind: "cleaning"; heard: string }
  | { kind: "review"; text: string }
  | { kind: "sending"; text: string }
  | { kind: "sent" }
  | { kind: "failed"; message: string; text?: string };

export type TalkEvent =
  | { type: "listening" }
  | { type: "heard"; text: string }
  | { type: "stop" }
  | { type: "final"; text: string }
  | { type: "cleaned"; text: string }
  | { type: "edit"; text: string }
  | { type: "send" }
  | { type: "sent" }
  | { type: "failed"; message: string }
  | { type: "retry" };

export const START: Step = { kind: "preparing" };

/** The next step. Events that don't fit the step (a late answer after Stop, say) leave it as it is. */
export function next(step: Step, e: TalkEvent): Step {
  switch (e.type) {
    case "listening":
      return step.kind === "preparing" ? { kind: "listening", heard: "" } : step;
    case "heard":
      return step.kind === "listening" || step.kind === "finishing" ? { ...step, heard: e.text } : step;
    case "stop":
      return step.kind === "listening" ? { kind: "finishing", heard: step.heard } : step;
    case "final":
      if (step.kind !== "finishing") return step;
      return e.text.trim() ? { kind: "cleaning", heard: e.text } : { kind: "failed", message: "Loft didn't hear anything. Try again closer to the microphone." };
    case "cleaned":
      return step.kind === "cleaning" ? { kind: "review", text: e.text } : step;
    case "edit":
      return step.kind === "review" ? { kind: "review", text: e.text } : step;
    case "send":
      return step.kind === "review" && step.text.trim() ? { kind: "sending", text: step.text.trim() } : step;
    case "sent":
      return step.kind === "sending" ? { kind: "sent" } : step;
    case "failed":
      // A failed send keeps the text, so Retry sends it again.
      return { kind: "failed", message: e.message, ...(step.kind === "sending" ? { text: step.text } : {}) };
    case "retry":
      if (step.kind !== "failed") return step;
      return step.text != null ? { kind: "sending", text: step.text } : START;
  }
}

/** Why the microphone is off here, or null when the browser may use it: browsers allow it only in a secure context
 * (https, or localhost). */
export function micBlocked(secure: boolean, hasMic: boolean): string | null {
  if (!secure) return "The microphone works only when Loft is open over https. Turn on https on the tailnet in Settings, then open Loft at its https address.";
  if (!hasMic) return "This browser doesn't offer a microphone.";
  return null;
}

/** `input` (mono floats at `rate` Hz) as 16 kHz 16-bit PCM, little-endian, as the transcribe endpoint takes it. */
export function pcm16k(input: Float32Array, rate: number): Uint8Array {
  const n = Math.floor((input.length * 16000) / rate);
  const out = new DataView(new ArrayBuffer(n * 2));
  for (let i = 0; i < n; i++) {
    // Averages the input samples this output sample covers (a box filter: enough for speech).
    const from = Math.floor((i * rate) / 16000);
    const to = Math.max(from + 1, Math.floor(((i + 1) * rate) / 16000));
    let sum = 0;
    for (let j = from; j < to && j < input.length; j++) sum += input[j];
    const v = Math.max(-1, Math.min(1, sum / (to - from)));
    out.setInt16(i * 2, Math.round(v * 0x7fff), true);
  }
  return new Uint8Array(out.buffer);
}

/** The text to put in a message box that already holds `existing`. */
export function appendDraft(existing: string, text: string): string {
  return existing.trim() ? `${existing.trimEnd()}\n${text}` : text;
}
