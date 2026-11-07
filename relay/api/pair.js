import { pair } from "../lib/pairing.js";
import { REQUIRED } from "../lib/portal.js";

// Without the portal no one can allow a code: Loft reads `portal: false` and offers only pasting a key.
export const POST = (request) =>
  REQUIRED.some((k) => !process.env[k]) ? Response.json({ error: "the push portal isn't configured", portal: false }, { status: 503 }) : pair(request);
