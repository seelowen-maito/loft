// The relay's key-value store: Upstash Redis (Vercel KV) over its REST API, so the relay has no dependencies.
// It only ever holds hashed relay keys, when each was last used, counters keyed by those hashes and the portal's accounts:
// never a payload or a device token.

export function kv(env) {
  const url = env.KV_REST_API_URL;
  const token = env.KV_REST_API_TOKEN;
  if (!url || !token) throw new Error("KV_REST_API_URL and KV_REST_API_TOKEN must be set");
  const call = async (path, body) => {
    const res = await fetch(url + path, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`store answered ${res.status}`);
    return res.json();
  };
  const command = async (...args) => (await call("", args)).result;
  // A pipeline or transaction answers per command; one that failed must not read as an empty result.
  const each = (answers) =>
    answers.map((r) => {
      if (r.error) throw new Error(`store refused a command (${r.error})`);
      return r.result;
    });
  return {
    /** Sets `key`, expiring after `seconds` when given. */
    set: (key, value, seconds) => command("SET", key, value, ...(seconds ? ["EX", seconds] : [])),
    /** Sets `key` only when it isn't there yet. */
    setIfMissing: (key, value) => command("SET", key, value, "NX"),
    get: (key) => command("GET", key),
    /** Reads and deletes `key` in one step, so two callers never both get the value. */
    getdel: (key) => command("GETDEL", key),
    del: (key) => command("DEL", key),
    /** Sets `key`; it is gone after `seconds`. */
    setEx: (key, value, seconds) => command("SET", key, value, "EX", seconds),
    exists: async (key) => (await command("EXISTS", key)) === 1,
    /** Runs `commands` in one round trip and returns their results; not atomic, so for reads. */
    pipeline: async (commands) => each(await call("/pipeline", commands)),
    /** Runs `commands` as one transaction (MULTI/EXEC): all of them or none. */
    transaction: async (commands) => each(await call("/multi-exec", commands)),
    /** Adds one to `key` and returns the count; the key expires `seconds` after its first hit. One round trip, so a
     *  counter never stays without its expiry. */
    hit: async (key, seconds) => {
      const [incr] = await call("/pipeline", [["INCR", key], ["EXPIRE", key, seconds, "NX"]]);
      return incr.result;
    },
  };
}
