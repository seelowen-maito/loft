import type { Agent } from "./api";
import { NEEDS_YOU_LABEL } from "./taskStatus.ts";

/** An agent's state label and colours. Working comes from the live run (`running`), never from the saved status alone. */
export function statusOf(a: Pick<Agent, "status" | "running" | "waiting_on" | "interval_minutes">): { label: string; dot: string; text: string } {
  if (a.status === "waiting") return { label: NEEDS_YOU_LABEL, dot: "bg-warn", text: "text-warn" };
  if (a.running && a.waiting_on?.length) return { label: "Waiting", dot: "bg-info animate-pulse-soft", text: "text-info" };
  if (a.running) return { label: "Working", dot: "bg-accent animate-pulse-soft", text: "text-accent" };
  if (a.status === "error") return { label: "Error", dot: "bg-bad", text: "text-bad" };
  if (a.interval_minutes > 0) return { label: "Sleeping", dot: "bg-info/70", text: "text-fg-2" };
  return { label: "Idle", dot: "bg-fg-3", text: "text-fg-3" };
}
