# The Lead starts and resumes Deliveries, under a per-project option

Status: accepted (spec #9, ticket #12).

## Context
Only the user starts a Delivery (Start delivery) or resumes a stalled one (Needs you → Resume). The Lead gets the notices ("The Delivery stalled", "hand out its tickets") but can't act, so a usage limit, an API 529 or a spec waiting for a click leave the team idle for hours, with nothing for the user to decide. The cost is idle time, not tokens.

## Decision
- **The Lead exercises Start and Resume**, through two MCP tools (`start_delivery`, `resume_delivery`, later tickets), **only in a project whose option "The Lead can start and resume Deliveries" is on**. The user still owns the roadmap, publishing, Needs you, pausing and merging.
- **The option is per project, off by default**, with three limits, each under a fixed ceiling and at least 1: resumes per hour (default 2, ceiling 6), resumes per day (default 6, ceiling 24) and Deliveries running at once (default 2, ceiling 10). A project never changed has no row and reads as the defaults, so new and existing projects start off.
- **Only the user changes it**, through `PUT /api/projects/lead-delivery`, a user endpoint (ADR 0004). No MCP tool reads or writes it for an agent to change; a test checks that no tool mentions it. An agent's `curl` to that endpoint stays open, as ADR 0004 accepts for every user endpoint.
- **Every change is logged** (`lead_delivery_log`): who (`user`), when, and the values before and after; a request that changes nothing logs nothing. `GET /api/projects/lead-delivery?project=` returns the options, the ceilings and the latest 50 lines, which the project's settings show.

## Alternative dropped
**An agent only for starting and resuming Deliveries.** It would be one more agent to create, give permissions and wake, and it would need the same tools and the same limits; the Lead already gets the stall notice and knows the plan. A new agent adds a place where the protections can drift apart, for no gain.

## Consequences
- With the option off nothing changes for anyone.
- The brakes the limits feed (per hour, per day, repeated failure, running Deliveries) are built in the tickets that add the tools; this one only holds the settings.

## Brakes on the Lead's resumes

The Delivery keeps the Lead's resumes of the last day. `resume_delivery` is refused, and the Delivery stays stalled, when the hour's or the day's limit is used up, or when it stopped again for the same reason within 10 minutes of a Lead's resume. A refusal sets the Delivery's `brake`: Needs you shows a new stalled item saying which brake (so it is pushed again) and the spec thread gets a line. A resume within the limits only adds a thread line with the counts; it pushes nothing. The user's Resume is never braked.
