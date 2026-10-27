# Planning grills in rounds, on the agent's own model

Status: accepted (2026-10-26)

## Context

Loft's first grill posted questions one at a time from a queue. After every answer it ran the agent again on Haiku to drop or add queued questions, so the next question appeared instantly and each revision was cheap.

Planning (grill → spec → tickets) needs better questions than that: each with a recommended answer, ordered by what depends on what, and grounded in facts the agent looked up instead of asking the user.

## Decision

A grill works in rounds. The agent asks every question whose prerequisites are settled, each with a recommendation, then runs once more only after the user sends the round. Rounds run on the grilling agent's own model; fact-finding goes to its sub-agents, on the existing "Subagent model" setting. The queue, the per-answer Haiku runs and the old `grill_*` tools are removed, with no legacy mode kept.

## Consequences

- A grill costs about the same as before or more: fewer runs, but on a stronger model, plus the exploration sub-agents. We accept that for better questions. Cost is tuned with the agent's model and subagent model, not a grill-specific setting.
- Between rounds the user waits while the agent explores; within a round, questions are already there.
- Clients show a round as a whole on screen and one question at a time in voice, and send it in one go.
