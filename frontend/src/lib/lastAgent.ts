/** The last agent viewed, remembered per browser so the Agents tab opens on it.
 * `storage` defaults to localStorage, read inside the try since it can throw when site data is blocked. */

type Store = Pick<Storage, "getItem" | "setItem">;

const KEY = "loft:last-agent";

export function loadLastAgent(storage?: Store): string | null {
  try {
    return (storage ?? localStorage).getItem(KEY);
  } catch {
    return null;
  }
}

export function saveLastAgent(id: string, storage?: Store) {
  try {
    (storage ?? localStorage).setItem(KEY, id);
  } catch {
    /* storage unavailable */
  }
}

/** The agent to open on `/agents`: the last one viewed if it still exists, else the first. */
export function openingAgent(agents: { id: string }[], last: string | null): string | null {
  return agents.find((a) => a.id === last)?.id ?? agents[0]?.id ?? null;
}
