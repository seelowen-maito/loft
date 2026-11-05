// The relay's key-value store: Upstash Redis (Vercel KV) over its REST API, so the relay has no dependencies.
// It only ever holds hashed relay keys and counters keyed by those hashes: never a payload or a device token.

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
  return {
    set: (key, value) => command("SET", key, value),
    exists: async (key) => (await command("EXISTS", key)) === 1,
    /** Adds one to `key` and returns the count; the key expires `seconds` after its first hit. One round trip, so a
     *  counter never stays without its expiry. */
    hit: async (key, seconds) => {
      const [incr] = await call("/pipeline", [["INCR", key], ["EXPIRE", key, seconds, "NX"]]);
      return incr.result;
    },
  };
}
