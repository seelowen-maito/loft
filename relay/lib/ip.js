// The caller's address, for the per-IP limits of the relay, pairing and the portal. The store only ever gets its hash.

/** The first x-forwarded-for address, which Vercel sets to the caller's; empty without one. */
export const ipOf = (request) => (request.headers.get("x-forwarded-for") ?? "").split(",")[0].trim();
