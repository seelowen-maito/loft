/** Pairing this browser with Loft (ADR 0004). The token only ever lives in an HttpOnly cookie the server sets. */

export type Pairing = { secret: string; code: string };
export type Polled = "pending" | "allowed" | "denied" | "gone";

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error((data as any).error ?? `${res.status} ${res.statusText}`);
  return data as T;
}

/** Whether this browser is paired: a request that needs a token (the settings) is not refused. */
export async function isPaired(): Promise<boolean> {
  const res = await fetch("/api/settings");
  return res.status !== 401;
}

/** A short name for this browser, shown to whoever approves it. */
export function browserName(ua: string = navigator.userAgent): string {
  const b = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "Browser";
  const os = /iPhone|iPad/.test(ua) ? "iOS" : /Android/.test(ua) ? "Android" : /Mac OS X/.test(ua) ? "Mac" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "";
  return os ? `${b} on ${os}` : b;
}

export const startPairing = () => post<Pairing>("/pair", { name: browserName() });
export const pollPairing = (secret: string) => post<{ status: Polled }>("/pair/poll", { secret, cookie: true }).then((r) => r.status);
export const cancelPairing = (secret: string) => post("/pair/cancel", { secret }).catch(() => {});
