# Loft issues mirror GitHub issues; Loft is the default home, GitHub is optional

Status: accepted (2026-10-27)

## Context
Loft had two lists of work: Tasks (Loft's own, agent-run) and Issues (PR #34, GitHub issues read live and never written). The same piece of work could show twice with two statuses. PR #49 weighed three options: link only (A), two cross-linked tabs (B), or sync issues into tasks (C, called too big).

## Decision
The Loft issue (formerly task) is the one work item. Its schema is issue-shaped (title, markdown description, labels, owner, open/closed) plus Loft-only fields (agent, execution status, priority, due date, thread). Projects without a repo or git keep plain Loft tasks.

In a project with a repo, open GitHub issues are mirrored into the tasks table (plus closed issues that already have a task). For the shared fields, an edit in Loft is written to GitHub right away; when both sides changed, GitHub wins. Comments from GitHub arrive freely. Comments from Loft (yours, an agent's, agent-to-agent, private notes) exist in the Loft thread at once but are posted to GitHub only after the user approves each one (Needs you: Approve, edit first, or Keep in Loft); a per-project setting offers auto-approve modes, off by default. Approved comments carry the agent's name in bold. This keeps internal agent talk from leaking to GitHub. Edits to title, description, labels, state and links go straight, whoever triggers them. Each synced comment keeps its GitHub comment id to avoid duplicates and echoes. Done closes the issue as completed, cancelled as not planned, and the reverse. `depends_on` and `parent_id` map to GitHub blocking links and sub-issues. GitHub reaches Loft by a 2-minute ETag poll plus on open, refresh and after publishing; no webhooks. A new Loft task stays Loft-only until published (a project setting can auto-publish). Planning's `publish_tickets` creates a linked Loft issue per ticket. The agent tools and API are renamed from task to issue, with the old names kept as aliases until iOS migrates.

Rejected: link-only (A) and two tabs (B), which keep the redundancy; webhooks, which don't fit a local app behind a tailnet.

## Consequences
- One list, one set of features (filters, push, iOS) instead of two.
- Moving a task to GitHub later is a straight copy.
- Loft now writes to GitHub on edits, comments and state changes, so it needs retry safety and conflict handling.
- A second copy of GitHub's data exists in SQLite; deleted or transferred issues must be reconciled.
- #34's read-only Issues tab is folded into Tasks and removed.
