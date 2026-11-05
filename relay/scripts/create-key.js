// Creates a relay key and prints it once: `node scripts/create-key.js "<who it's for>"`, with KV_REST_API_URL and
// KV_REST_API_TOKEN set to the relay's store (Vercel → Storage → the store → .env.local). Only its hash is stored, so
// copy it now: it can't be shown again. The user pastes it in Loft → Settings → Push notifications.

import { createKey } from "../lib/relay.js";

const label = process.argv.slice(2).join(" ").trim();
if (!label) {
  console.error('Usage: node scripts/create-key.js "<who it\'s for>"');
  process.exit(1);
}
const key = await createKey(process.env, label);
console.log(`Relay key for ${label}:\n\n  ${key}\n\nIt is shown only this once.`);
