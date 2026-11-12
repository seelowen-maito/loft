import type { UpdateStatus } from "./api";

/** The navbar's Restart button is off while agents run, and while a restart is under way; a failed one frees it. */
export function restartDisabled(s: Pick<UpdateStatus, "running_agents" | "why">, busy: boolean): boolean {
  const failed = s.why?.startsWith("restart failed") ?? false;
  return (busy && !failed) || s.running_agents > 0;
}
