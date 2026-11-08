# Only a paired client can change things through the API

Status: proposed (design for review; nothing is built yet)

## Context
An agent runs as the same Mac user and can `curl` Loft's API on localhost:7420. Loft can't tell that from the web app, the iOS app or a script, so today an agent could change the push keys and the relay address (`PUT /api/push`), approve its own permission requests (`POST /api/permissions/{id}`), approve comments, merge PRs, or delete agents. Agents don't need the REST API at all: everything they do goes through `/mcp/{agent_id}`.

## Threat model (what this protects, and what it can't)
Protects against an agent, or a prompt injected into one, that uses the API the way a person's client would: a `curl` from its shell. It does **not** protect against an agent that edits Loft's own files, reads the keys on disk (`AuthKey.p8` and the data folder are readable by the same Mac user) or the paired browser's profile. Closing that needs a separate OS user or sandbox, a different project.

## Decision
1. **Default deny for writes.** Every `POST/PUT/PATCH/DELETE` under `/api` needs a user token. One middleware, so a new endpoint is covered without anyone remembering to add it. Only `POST /api/pair` (below) stays open. `/mcp/*` is unchanged and never takes the token.
2. **Named sensitive reads** need it too: `GET /api/devices` and `GET /api/push` (device tokens, key status), `GET /api/settings`. Other reads stay as today: images, SSE and `<img>` loads can't send headers, and agents can already read each other's chat through MCP.
3. **Pairing needs a person at the Mac.** A client with no token calls `POST /api/pair`; Loft shows a native macOS dialog ("Allow a browser or phone to control Loft? Code 4821") and the client shows the same code. The user clicks Allow; the client, polling, receives its token once. No click within 2 minutes, or Deny: nothing is issued. One pairing waits at a time. This is the same shape as the relay sign-in code (ADR 0003): the code is shown only where an agent can't read it. A Mac nobody is looking at pairs nobody, which is the point.
4. **Token.** 32 random bytes, hex. One per paired client, stored only as a SHA-256 hash (table `paired_clients`: hash, name, created). Compared in constant time. Never logged, never in an agent's environment, files or prompt (agents get none of Loft's own env, see `agents.rs`).
5. **How clients carry it.** Web: an `HttpOnly; SameSite=Strict` cookie set by the pairing response, so it never reaches page scripts. iOS: the Keychain, sent as `Authorization: Bearer`. The existing `Sec-Fetch-Site` guard stays and covers cookie CSRF.
6. **Existing installs** pair on first use after the update: the web app and the iOS app show a "Pair this client" screen on a 401.

## User-only endpoints (all writes, plus the named reads)
Push and relay: `PUT /push`, `POST /push/signin`, `POST/PATCH/DELETE /devices…`, `POST /devices/{token}/test`, `GET /push`, `GET /devices`.
Approvals: `POST /permissions/{id}`, `POST /issues/{id}/comments/{cid}/approve|keep` (and the `/tasks` alias), `POST /pull-requests/{id}/merge`, `POST /deliveries/merge`.
Widening access: `PUT /settings` (GitHub token, `claude_bin`, comment approval mode), `POST/PATCH/DELETE /agents…` (instructions, tools, delete), `POST /restart`, `POST /system/mac-access`, `POST /simulator/input`, `POST /agents/{id}/open-editor`.
Everything else that writes (messages, issues, runs, memories…) is covered by the default-deny rule without being listed.

## Open questions for review
1. Is default-deny on all writes too wide? It makes the user's own message the only way a person speaks to an agent, which seems right, but it means a `curl` a user types in a terminal needs a token too.
2. `POST /devices` (a phone registering for push on every launch) comes after pairing on iOS, so it can be guarded; confirm.
3. Per-client tokens give revocation later (a Settings list) at no cost now; no revoke UI in v1.
4. The relay address is now one setting, `push.relay_url`, changed only by `PUT /api/push` (#112/#115); there is no separate portal address, so ticket #48's list shrinks to that.

## Consequences
- An agent's `curl` to any write is refused with 401; tests drive the API with and without a token.
- Every client needs a pairing step once; the iOS app and web app each get a small screen.
- Not a sandbox: see the threat model.
