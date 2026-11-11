# Only a paired client can change things through the API

Status: withdrawn. Pairing was built, then removed before it reached `main`: the user chose not to require it. The API is open again to anything that can reach it, agents' `curl` included; the threat model below describes what that leaves open.

## Context
An agent runs as the same Mac user and can `curl` Loft's API on localhost:7420. Loft can't tell that from the web app, the iOS app or a script, so today an agent could change the push keys and the relay address (`PUT /api/push`), approve its own permission requests (`POST /api/permissions/{id}`), approve comments, merge PRs, or delete agents. Agents don't need the REST API at all: everything they do goes through `/mcp/{agent_id}`.

## Threat model (what this protects, and what it can't)
Protects against an agent, or a prompt injected into one, that uses the API the way a person's client would: a `curl` from its shell. It does **not** protect against an agent that edits Loft's own files, reads the keys on disk (`AuthKey.p8` and the data folder are readable by the same Mac user) or the paired browser's profile. Closing that needs a separate OS user or sandbox, a different project.

## Decision
1. **Default deny for writes.** Every `POST/PUT/PATCH/DELETE` under `/api` needs a user token. One middleware, so a new endpoint is covered without anyone remembering to add it. Only `POST /api/pair` (below) stays open. `/mcp/*` is unchanged and never takes the token.
2. **Named sensitive reads** need it too: `GET /api/devices` and `GET /api/push` (device tokens, key status), `GET /api/settings`, and Clean's listings `GET /api/workspaces/clean/preview` and `GET /api/workspaces/clean/folders` (every path in the agents' workspaces) and its log `GET /api/workspaces/log` (the paths it removed). Other reads stay as today: images, SSE and `<img>` loads can't send headers, and agents can already read each other's chat through MCP.
3. **Pairing needs a person.** A client with no token calls `POST /api/pair` and gets a short display code plus a 32-byte secret, and shows the code. It polls `POST /api/pair/poll` with the secret, never the code, and receives its token once. Several pairings wait at once, in memory only (a restart drops them, and so does 2 minutes without an answer); the pair endpoint is rate-limited. A paired client allows or denies one by its code (`POST /api/pair/approve|deny`), after comparing it with the one the new device shows. The very first client has no paired device to ask, so the Mac does: a native dialog ("Allow a browser or phone to control Loft? Code 4821") or the code typed into the terminal Loft runs in. It is never trusted automatically, and the code is printed, not logged. Loopback requests are not trusted: Tailscale makes every request look local.
4. **Token.** 32 random bytes, hex. One per paired client, stored only as a SHA-256 hash (table `paired_clients`: hash, name, created, last used). Looked up by that hash in SQL, not compared byte by byte: a 256-bit random token can't be guessed from timing, so a constant-time compare adds nothing. Never logged, never in an agent's environment, files or prompt (agents get none of Loft's own env, see `agents.rs`).
5. **How clients carry it.** Web: a `__Host-loft` cookie (`Secure; HttpOnly; Path=/; SameSite=Strict`) set by the poll response, so it never reaches page scripts. `Secure` means Safari drops it on plain `http://localhost` (and the Vite dev server), where Chrome keeps it: web pairing there needs https, such as `tailscale serve`. iOS: the Keychain, sent as `Authorization: Bearer`. The existing `Sec-Fetch-Site` guard stays and covers cookie CSRF.
6. **Existing installs** pair on first use after the update: the web app and the iOS app show a "Pair this client" screen on a 401.

## User-only endpoints (all writes, plus the named reads)
Push and relay: `PUT /push`, `POST /push/signin`, `POST/PATCH/DELETE /devices…`, `POST /devices/{token}/test`, `GET /push`, `GET /devices`.
Approvals: `POST /permissions/{id}`, `POST /issues/{id}/comments/{cid}/approve|keep` (and the `/tasks` alias), `POST /pull-requests/{id}/merge`, `POST /deliveries/merge`.
Pairing: `GET /paired-clients` (the list, with `this` on the caller), `DELETE /paired-clients/{id}` (Forget this device), `POST /paired-clients/unpair-all` (every client but the caller), `POST /pair/approve|deny`.
Disk: `GET /workspaces/clean/preview`, `GET /workspaces/clean/folders` (every path in the agents' workspaces), `GET /workspaces/processes` (the commands an idle agent left running), `GET /workspaces/log` (the paths the last run removed; its file is `.loft/clean-log.json`, outside the agents' workspaces but not out of their reach, as the threat model above says); `POST /workspaces/clean` and `POST /workspaces/clean/check` (runs read-only git in an agent's clone, only on the user's click) and `POST /workspaces/{id}/stop-processes` (Clean's Stop them: signals an idle agent's own processes, a dev server it left running included, never an agent in a run) by the default-deny rule.
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
