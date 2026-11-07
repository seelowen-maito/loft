# Comments and actions on issues, and issues on the web: plan and TODO

> Historical plan. The product word is now "issue"; code in backticks keeps the old `task` API names.

**Status:** plan for review. Nothing built yet.

## Problem

- In the iOS app, an issue's page shows its status, the latest note, the summary and a history built from activity events. You can **run, re-run, cancel, mark done, edit or add a sub-issue**, but you **can't say anything about it**. To ask the agent about an issue, or tell it to change course, you copy the issue into the agent's chat by hand. The agent then has no link back to the issue, and the conversation isn't kept with the issue.
- Agents can only leave **one** progress note (`update_task note` overwrites the last), so an issue has no running thread.
- The **web app has no issues screen at all**: issues only appear in the iOS app and as rows in chats.

## What we want

1. **A thread on every issue:** your comments, the agent's comments and notes, and status changes, in one timeline.
2. **A comment reaches the agent** working on the issue, tagged with the issue, without copying anything. The agent's answer comes back to the same thread.
3. **One-tap actions** that carry the issue with them: ask about it, follow up on a finished issue, hand it to another agent, mention it in a chat.
4. **Issues on the web:** the same list, detail, thread and actions as the iOS app.

## Design

### An issue thread (backend)

- New table `task_comments`: `id, task_id, author ("user" or an agent id), kind (comment | note | status | system), text, meta, created_at`.
- **Written by:**
  - a user comment (`POST /api/tasks/{id}/comments`, with `client_id` for retries, like messages)
  - an agent comment (new MCP tool `comment_task`)
  - `update_task` with a `note`: the note is still the "latest progress" line, and each one is also appended to the thread, so notes are no longer lost
  - status changes, run, cancel, reassign: a short `status` entry ("Started by Web Dev", "Cancelled by you")
- `GET /api/tasks/{id}` includes the thread (latest 100, paged). An SSE event `task_comment` keeps open screens live.
- The existing activity-based "History" on iOS is replaced by this thread.

### A comment reaches the agent

When you comment on an issue that has an agent:

- **"Send to <agent>"** is on by default, and it delivers the comment to the agent's chat as a message tagged `task_id` + `task_role: "comment"`. The agent sees:
  `[Comment on task <id> "<title>" (status: in_progress)] <your text>` plus how to answer: call `comment_task` on that issue.
  The chat shows it as a compact issue row (like briefs and reports today) linking to the issue.
- If the agent is busy, it waits in the queue like any message. With the run watchdog it's delivered at the reply, not lost.
- **Turn the switch off** for a private note on the issue that the agent isn't told about.
- **On a finished issue** (done, failed, cancelled), the sheet offers **"Reopen and send"**: it reopens the issue and runs it again with your comment as the instruction (the existing `execute` with `note`).
- The agent's answer (`comment_task`) appears in the issue thread **and** in its chat, so either place works.
- Agents get `get_task` (issue + thread) to read the context before answering. The system prompt's Issues line says to answer issue comments with `comment_task`.

### Actions on an issue (iOS and web)

| Action | What it does |
|---|---|
| **Comment** | adds to the thread; sends to the agent unless switched off |
| **Ask <agent>…** | a comment framed as a question; the answer comes back to the thread |
| **Follow up** (finished issues) | reopen + run again with your note |
| **Run with another agent / reassign** | existing; now posts a status entry |
| **Mention in a chat** | opens any agent's chat with the issue **attached** to the composer as a chip. The message carries `task_id` and the agent gets the title, status, latest note and summary. This replaces copying the issue by hand. |
| **Copy link** | `loft://task/<id>` (iOS) / `<web>/tasks/<id>`. Issue links in any chat render as issue chips. |
| Mark done / cancel / edit / sub-issue | existing |

In chats, the composer gets **Attach → Issue** (a search picker) on iOS and web, so you can start from the chat too.

### Issues on the web: yes

Reasons: you triage and review at the desk as much as on the phone, agents create many sub-issues, and the web shows long summaries and threads better. Everything goes through the same API, so it's mostly UI.

- **`/tasks` page**, a new top-level tab:
  - a list grouped by status (Waiting on you · In progress · Ready · Inbox · Done)
  - filters: project, agent, priority, "created by me"
  - search, plus "New issue"
  - live via SSE
- **`/tasks/:id` detail:** title, status, assignment, parent and sub-issues, dependencies, latest note, summary, problem, the **thread** with a comment box (with the "Send to <agent>" switch), and the action buttons above.
- **In chats:** issue rows (brief, report, comment) link to the issue page, and the issue chips work as on iOS.
- Small screens: the page works at phone width too.

## TODO (one PR each)

Every PR: tests (one reproducing the current gap where it applies), a full test run, and a device check for iOS.

- [x] **T-1 Backend: issue thread.** *(done, #83)* `task_comments` table and migration; `POST/GET /api/tasks/{id}/comments`; notes and status changes append to it; SSE `task_comment`; the feature flag `task_comments` in `/api/info`.
- [x] **T-2 Backend: comments reach the agent.** *(done, #85)* Delivery to the agent's chat as a tagged message (queued while busy); "reopen and send" for finished issues; MCP `comment_task` and `get_task`; a system-prompt line. Tests: a comment on an in-progress issue reaches the agent with the issue tag; the agent's `comment_task` lands in the thread and the chat; a private note isn't delivered; agents can only comment on issues they're on or created.
- [x] **T-3 Backend: issue references in messages.** *(done, #116)* `meta.task_id` on user messages from any client adds a short issue header to the agent's prompt; issue links in text are recognised.
- [x] **T-4 Web: Issues page (list + detail, read-only actions first).** *(done, #86)* Nav tab, list with groups and filters, detail with the thread, live updates.
- [x] **T-5 Web: comment box and actions.** *(done, #87)* Comment (send / private), Ask, Follow up, Run / reassign, Mark done, Cancel, New issue, Copy link.
- [x] **T-6 Web: issues in chats.** *(done, #116)* Issue rows link to the issue; Attach → Issue in the composer; issue chips.
- [ ] **T-7 iOS: thread and comment box on the issue page** (replaces History), with the Send switch and "Reopen and send".
- [ ] **T-8 iOS: Mention in chat, Attach → Issue, issue chips, `loft://task/<id>` links.**
- [ ] **T-9 iOS (later, with the Apple Intelligence plan):** reply to an issue comment from its notification, and Siri "comment on the backup issue: …".

Order: T-1 → T-2 → (T-4, T-5, T-7 in parallel) → T-3, T-6, T-8. T-1 and T-2 already help the iOS app once T-7 lands; the web page (T-4/T-5) is useful from day one even before the thread.

## Open questions

1. Should a comment on an issue be **sent to the agent by default**, or kept as a private note by default? (Proposed: sent.)
2. Should the agent be allowed to **comment on issues it isn't assigned to** (e.g. Orchestrator commenting on its delegates' issues)? (Proposed: yes for issues it created or delegated.)
3. On the web, should Issues be **its own tab**, or a panel inside Agents? (Proposed: its own tab.)
4. Notifications for agent comments on your issues: on by default?
