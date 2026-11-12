# Pairing was removed: what is open, and what still guards it

Status: withdrawn. Client pairing (a token for every write under `/api`, handed out after a person approved a code) was built and then removed in #405, which merged upstream's removal into this fork. The user chose not to require it. This record says what that leaves open and what still guards it.

## Context
An agent runs as the same Mac user and can `curl` Loft's API on localhost:7420. Loft can't tell that from the web app, the iOS app or a script, so an agent can change the push keys and the relay address (`PUT /api/push`), approve its own permission requests (`POST /api/permissions/{id}`), approve comments, merge PRs, or delete agents. Agents don't need the REST API at all: everything they do goes through `/mcp/{agent_id}`, but nothing stops them using the rest.

## What is open
Anything that can reach Loft's port and send a request. In particular:
- **An agent's `curl`** (and a prompt injected into one). The user accepts this: agents on the Mac are trusted. It does not protect against an agent that edits Loft's files or reads the keys on disk either; that needs a separate OS user or sandbox, a different project.
- **Another machine** on the network or tailnet that can open the port. Loft listens on the Mac; reaching it from outside is `tailscale serve` or the tailnet's own rules.
- **There is no token**, so a loopback request is not special: Tailscale makes every request look local.

## What still guards it
1. **`Sec-Fetch-Site` on writes** (`guard::same_origin_writes`). A browser marks a cross-site `POST/PUT/PATCH/DELETE`; it is refused with 403. Our own frontend is `same-origin`; non-browsers (iPhone app, curl, MCP) send nothing and pass.
2. **`same_site_read` on some private reads**: the device list and push status (`GET /api/devices`, `GET /api/push`), Needs you, and the herdr binary (`GET /api/herdr/bin/{platform}`, which builds when there is none). A cross-site read is refused. Other reads stay open: images, SSE and `<img>` loads can't send headers, agents can read each other's chat through MCP anyway, and the Host check below covers the rest from other sites.
3. **A `Host` check on `/api`, `/mcp` and `/kill`** (`guard::local_host`), against DNS rebinding: a page on another site can point its own domain at this Mac, after which the browser treats Loft as that page's own origin, so the two guards above both pass. The browser still sends the page's name as `Host`. Only these hosts are served, anything else gets 403: `localhost`, `127.0.0.1`, `::1`, this Mac's name (`<name>` and `<name>.local`), its Tailscale name, and any IP address (a rebound domain sends its name as `Host`, never an address, so LAN and tailnet addresses work). Others can be added with `LOFT_ALLOWED_HOSTS` (comma-separated names, without a port). The 403 names the refused host and this variable. If the Tailscale CLI is missing or slow when Loft starts, the iPhone's Tailscale name is refused until it answers (or is listed there). A request with no `Host` header isn't a browser and passes.

## Left behind by the removed pairing
Clients clear what the old pairing stored: the iPhone app deletes its Keychain item (`loft.client-token`) at launch, the server answers a request that carries the `__Host-loft` cookie with an expired one, and `loft-herdr` drops the `token` key from its `config.json`.

## Consequences
- An agent's `curl` to any write is accepted, as before pairing.
- A new endpoint that returns private data should call `same_site_read`; writes are covered by the middleware.
- A device that reaches Loft by a name not listed above (a custom domain in front of it) needs `LOFT_ALLOWED_HOSTS`.
