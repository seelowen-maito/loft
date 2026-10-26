# "Needs you" API (v1)

Everything waiting on the user, from all agents, for the apps' inbox and badge. The server only gathers
and orders it: by fixed urgency, then oldest first. Ranking and summarising with a model (the phone's
on-device model, iOS P-2) happen on the client.

Feature flag: `needs_you` in `GET /api/info` → `features`.

## `GET /api/needs-you`

```jsonc
{
  "version": 1,
  "count": 7,             // badge: all items
  "urgent": 4,            // items with urgency >= 2
  "generated_at": "2026-10-23T14:37:36Z",
  "items": [ Item, … ]    // most urgent first, then longest waiting first
}
```

### Item

| Field | Type | Notes |
|---|---|---|
| `id` | string | Stable while the situation is the same; **a new id when it changes** (new status, new commit, new question). Format `<type-ish>:<…>`; treat as opaque. |
| `type` | string | One of the types below. Unknown types: show them generically (title, reason, open). |
| `urgency` | 0–3 | 3 blocks an agent now · 2 waiting for your answer · 1 worth a look · 0 to review |
| `agent` | `{id, name}` \| null | |
| `task_id` | string \| null | |
| `title` | string | ≤ 120 chars, one line |
| `reason` | string | ≤ 160 chars, one plain line: why it's here |
| `text` | string | ≤ 2000 chars, the full text (may contain markdown characters and newlines) |
| `created_at` | RFC 3339 | When it started waiting |
| `age_seconds` | int | At `generated_at` |
| `links` | `[{label, url?, path?}]` | Kept apart from the text. `url`: external (GitHub); `path`: a Loft web path (`/tasks/<id>`) |
| `actions` | `[Action]` | What the user can do, in display order |
| `project` | string \| null | Grouping key: the GitHub repo's name for PR items (`pos`), else the task's or agent's project |
| `repo` | string \| null | PR items: the GitHub slug (`acme/shop`), the same as `pr.repo` |
| `pr` | object \| null | PR items only: `{repo: "owner/name", project, number, title, url, author, draft: false, checks: "passing"\|"none", checks_green: true, review_requested}` |

**All text is plain text.** Never render it as HTML. Anything that looks like a credential (known token
prefixes, JWTs, `Bearer …`, `password=…`, `token: …`) is replaced with `[redacted]` before it's sent.

### Types

| `type` | urgency | When | Actions |
|---|---|---|---|
| `permission` | 3 | A permission prompt is pending | approve, approve_always, deny, open |
| `task_approval` | 3 | Task in `waiting_approval` | answer, open, dismiss |
| `task_blocked` | 2 | Task `blocked` (reason = its error) | answer, open, dismiss |
| `question` | 2 | An idle agent's last reply ends with a question and you haven't written since (last 7 days) | answer, open, dismiss |
| `grill_question` | 2 (1 if paused) | An interview question is waiting | answer, skip, open |
| `stuck` | 2 | A run waits on background work, or has had no output, for ≥ 10 min | restart, open, dismiss |
| `task_failed` | 1 | Task `failed` | answer, retry (reopen and send), open, dismiss |
| `pr_ready` | 1 | Open, not a draft, mergeable, checks green. PRs agents opened here, plus the user's own ready PRs on GitHub in known repos | merge (tracked PRs) or open (GitHub), dismiss |
| `pr_review` | 1 | Someone else's open, non-draft PR with green checks, in a known repo, where the user's review is requested | open (GitHub), dismiss |
| `task_done` | 0 | A task **you** created finished in the last 48 h | answer (follow up = reopen and send), open, dismiss |

### Action

```jsonc
// An HTTP action: send `body` as JSON to `path` with `method`.
{ "kind": "approve", "label": "Allow once", "method": "POST", "path": "/api/permissions/p1", "body": { "decision": "allow_once" } }

// "answer" / "retry" carry a body with an empty "text": put the user's words there before sending.
{ "kind": "answer", "label": "Reply", "method": "POST", "path": "/api/agents/<id>/messages", "body": { "text": "" } }
{ "kind": "answer", "label": "Answer", "method": "POST", "path": "/api/agents/<id>/messages",
  "body": { "text": "", "meta": { "grill": { "role": "answer", "session_id": "…", "question_id": "…" } } } }
{ "kind": "answer", "label": "Reply on the task", "method": "POST", "path": "/api/tasks/<id>/comments", "body": { "text": "", "send": true } }

// Navigation, no request: `target` is { screen: "chat", agent_id } | { screen: "task", task_id } | { url }.
{ "kind": "open", "label": "Open chat", "target": { "screen": "chat", "agent_id": "…" } }
```

`kind` values: `approve`, `approve_always`, `deny`, `answer`, `retry`, `skip`, `merge`, `restart`, `open`, `dismiss`.
Use the `path` and `body` as given. Add `client_id` to message and comment bodies as the apps do today.

### Pull requests from GitHub

Besides the PRs agents opened here (tracked and polled every 30 s), the server searches GitHub every 5 minutes, with one GraphQL request, for open, non-draft PRs that are **the user's own** (agents push with the user's token, so theirs count) or that **request the user's review**. It keeps only PRs that:
- are in a known repo: the projects folder, the agents' workspaces, or a repo agents opened PRs in;
- have green checks, or no checks;
- are mergeable (for the user's own PRs).

Teammates' PRs only appear when they request the user's review. A PR already tracked here is listed once, with its Merge action. Group PR items by `pr.repo` or `project`.

## `POST /api/needs-you/{id}/dismiss`

Hides an item until it changes, so a changed item comes back with a new id. Dismissals are kept 30 days.
`permission` and `grill_question` items can't be dismissed (400): they block an agent until answered. Unknown ids get 404.

## Live updates

SSE event on `/api/events` when the list changes (checked every 5 s, sent only on a change):

```json
{ "type": "needs_you", "count": 7, "urgent": 4, "ids": ["permission:p1", "task:…:blocked", …] }
```

Refetch `GET /api/needs-you` when `ids` differ from what you show. The count alone is enough for a badge.

## Safety

- Same access as the rest of the API (the tailnet). Because this payload holds conversation text, **a browser request from another site is refused (403) even for this GET**: Loft's CORS is permissive, so other GETs don't get this protection. Non-browser clients (the iOS app, curl) send no `Sec-Fetch-Site` and are allowed.
- No tokens or secrets on purpose: credential-looking values are redacted in `title`, `reason` and `text`.

## Fixtures

`docs/fixtures/needs-you.json` is a full response with one item of every type except `stuck`, generated by the backend's own test (`NEEDS_YOU_FIXTURE=<path> cargo test needs_you`). A `stuck` item looks like:

```json
{
  "id": "stuck:<agent-id>:<run-start>", "type": "stuck", "urgency": 2,
  "agent": { "id": "…", "name": "Tablebook Dev" }, "task_id": null,
  "title": "Tablebook Dev may be stuck",
  "reason": "Waiting on a background task for 14 min: Run Maestro F9, F11, F12, F14",
  "text": "Waiting on: Run Maestro F9, F11, F12, F14",
  "created_at": "…", "age_seconds": 840, "links": [],
  "actions": [
    { "kind": "restart", "label": "Restart (keeps queued messages)", "method": "POST", "path": "/api/agents/<id>/restart", "body": {} },
    { "kind": "open", "label": "Open chat", "target": { "screen": "chat", "agent_id": "…" } },
    { "kind": "dismiss", "label": "Dismiss", "method": "POST", "path": "/api/needs-you/stuck%3A…/dismiss", "body": {} }
  ]
}
```
