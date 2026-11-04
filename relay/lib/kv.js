// The relay's key-value store: Upstash Redis (Vercel KV) over its REST API, so the relay has no dependencies.
// It only ever holds hashed relay tokens and counters keyed by hashes: never a payload, a device token or an IP.

export function kv(env) {
  const url = env.KV_REST_API_URL;
  const token = env.KV_REST_API_TOKEN;
  if (!url || !token) throw new Error("KV_REST_API_URL and KV_REST_API_TOKEN must be set");
  const command = async (...args) => {
    const res = await fetch(url, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify(args),
    });
    if (!res.ok) throw new Error(`store answered ${res.status}`);
    return (await res.json()).result;
  };
  return {
    set: (key, value) => command("SET", key, value),
    exists: async (key) => (await command("EXISTS", key)) === 1,
    /** Adds one to `key` and returns the count; the key expires `seconds` after its first hit. */
    hit: async (key, seconds) => {
      const n = await command("INCR", key);
      if (n === 1) await command("EXPIRE", key, seconds);
      return n;
    },
  };
}
