# Security TODO

Open findings from the Security Agent audit (pass 1, 2026-10-21). Each item links to a Loft task of the same name; the full report with reproduction notes stays on the host Mac and is not published. Items are removed here when the fix merges.

## Open findings

Short list so the whole picture is visible. Details are in the local report and in the Loft tasks with the same names.

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | High | No authentication on the HTTP API and `/mcp`. Anyone who can reach the tailnet address can drive every agent, read every chat and change settings. The same-origin guard on writes stops browsers, not direct clients. | Open |
| 3 | High | Auto-update runs any commit that lands on `main`. The branch is unprotected, so a single compromised GitHub account or token becomes code running on this Mac. | Open |
| 5 | Medium | Agents can escalate their own permissions through the local API (approve their own permission prompts and edit their own tool rules). | Open |
| 6 | Medium | Chat history stores and serves secrets that agents read during a run (keys, tokens, config files), in plain text, readable by every other agent. | Open |

Fixed or in review: #2 wildcard CORS and #4 path traversal in the workspace file listing (PR #32, plus the same-origin write guard now on `main`). #7 and #8 went away with the iMessage feature, which was removed.
