/** The Mac-only features (see the glossary): the web app shows each one only when `/api/info` lists it, which the
 * backend does when it runs on a Mac. No imports, so `node --test` can load it. */

export const MAC_ONLY = ["simulator", "voice", "mac_access", "terminal_login", "herdr"] as const;
export type MacOnly = (typeof MAC_ONLY)[number];
export type Offered = Record<MacOnly, boolean>;

/** Which Mac-only features the server offers. None until `/api/info` answers, so off the Mac they never flash in. */
export function offered(info: { features?: string[] } | null): Offered {
  const has = new Set(info?.features ?? []);
  return Object.fromEntries(MAC_ONLY.map((f) => [f, has.has(f)])) as Offered;
}
