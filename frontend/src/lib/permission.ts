import type { PermissionRequest } from "./api.ts";

/** Whether "Always allow" can save something. A Kill's rule is only ever one Loft made for a program it can name
 * (the backend checks it, and the answer can't post another), so an empty rule means none: only Allow once and Deny. */
export function canAlwaysAllow(req: Pick<PermissionRequest, "tool_name" | "rule">): boolean {
  return req.tool_name !== "Kill" || req.rule !== "";
}
