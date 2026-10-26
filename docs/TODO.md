# Security TODO

Open findings from the Security Agent audit (pass 1, 2026-10-21). Each item links to a Loft task of the same name; the full report with reproduction notes stays on the host Mac and is not published. Items are removed here when the fix merges.

## iMessage inbox trusts the sender handle only

**Severity:** Low (the inbox is off by default and nobody has enabled it yet)

**What it does today.** When iMessage replies are enabled, the backend polls the Messages database every few seconds and treats every incoming message whose sender handle matches the configured recipient as a command for the default agent. The agent's answer is texted back to that same thread. The match is on the handle alone (phone digits or email, compared loosely). The query does not check which service delivered the message, so a plain SMS counts the same as an iMessage, and there is no second factor beyond "the sender looks like you".

**Risk.** Phone numbers can be spoofed on SMS. An attacker who knows the configured phone number can send a text that appears to come from it. The backend would accept it, hand it to the default agent as if the user had sent it, and the agent would run with whatever tools and permissions it has on this Mac, then text the result back. In practice that means remote command of an agent on the machine from outside the tailnet.

What the attacker needs: the configured phone number, an SMS spoofing service, and the inbox turned on. If the recipient is an Apple ID email rather than a phone number the spoofing route mostly closes, since iMessage addresses are authenticated by Apple.

**Suggested fixes** (any combination; the first two are the cheap ones):

- [ ] Filter on the delivery service: only accept rows where the handle's service is iMessage, never SMS or RCS. Most of the risk goes away with this one line.
- [ ] Keep an explicit allow list of handles (today it is one recipient; make that a list and match exactly after normalising).
- [ ] Add a shared secret or confirmation step: a short code the user sets in Settings that every command must start with, or a one-time code the backend texts back that the user has to echo before the command runs.
- [ ] Do not act on messages from unknown senders at all, and do not reply to them either, so the backend never confirms the number is live.
- [ ] Run the reply agent on a narrow tool set (read-only, no shell) so a successful spoof is bounded.
- [ ] Rate limit inbound commands and log the sender handle and service for every accepted message.

**Where:** `backend/src/inbox.rs` (the query in `read_since` and the handle comparison in `normalize`), settings in `backend/src/settings.rs`.

**Reference:** local report "Security Agent audit 2026-10-21", finding 8.

## Other open findings from the same audit

Short list so the whole picture is visible. Details are in the local report and in the Loft tasks with the same names.

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | High | No authentication on the HTTP API and `/mcp`. Anyone who can reach the tailnet address can drive every agent, read every chat and change settings. The same-origin guard on writes stops browsers, not direct clients. | Open |
| 3 | High | Auto-update runs any commit that lands on `main`. The branch is unprotected, so a single compromised GitHub account or token becomes code running on this Mac. | Open |
| 5 | Medium | Agents can escalate their own permissions through the local API (approve their own permission prompts and edit their own tool rules). | Open |
| 6 | Medium | Chat history stores and serves secrets that agents read during a run (keys, tokens, config files), in plain text, readable by every other agent. | Open |
| 7 | Low | The iMessage "send test" endpoint is unauthenticated, so anyone on the tailnet can text any recipient from this Mac's Apple ID. Settings can also redirect all notifications to another number. | Open |
| 8 | Low | iMessage inbox trusts the sender handle only (this file, above). | Open |

Fixed or in review: #2 wildcard CORS and #4 path traversal in the workspace file listing (PR #32, plus the same-origin write guard now on `main`).
