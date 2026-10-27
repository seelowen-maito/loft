# Tasks and GitHub issues: one list, with a visible origin

Status: proposal for discussion (2026-10-27). Nothing here is built.

Mockups are in `img/` (not committed yet: the repo keeps images out, so where they live is an open question below).

Terms follow `GLOSSARY.md` from the Planning PR (#34): **task**, **ticket**, **spec**, **owner**, **Lead**, **project**.

## Where we are

Loft has two lists of work in the web app's top nav once #34 lands:

| | **Tasks** tab (on `main`) | **Issues** tab (PR #34) |
|---|---|---|
| What it lists | Loft's own tasks: personal todos and work handed to agents | Every project's GitHub issues, open and closed |
| Stored | `tasks` table in Loft's SQLite, one JSON row per task | Not stored. Fetched live from GitHub (GraphQL, up to 1,000 per repo), refetched when Loft publishes |
| Writes | Create, edit, start, cancel, comment; agents report status with `update_task` | None. The tab is read-only ("the tab never writes") |
| Status | `inbox · ready · in_progress · blocked · waiting_approval · done · failed · cancelled` | `open · closed` (+ `state_reason`), labels, sub-issues, blocked by / blocking |
| Who | `agent_id` (an agent) and `created_by` (user or agent) | `assignees` on GitHub. The owner is always a person |
| Grouping | Needs you, In progress, Ready, Inbox, Done | By project, then open / closed |

How the two meet today:

- **Planning publishes to GitHub, not to tasks.** `publish_spec` and `publish_tickets` (`backend/src/planning.rs`) create the spec and its tickets as GitHub issues labelled `spec` and `ready-for-agent`, owned by the user, with native sub-issue and blocking links. `published_issues` remembers what was published (repo, kind, key → number) so retries don't duplicate. No task is created.
- **A task doesn't know about a ticket.** The glossary says "a task may point at a ticket", but the `tasks` row has no field for it. When the Lead hands a ticket to an agent, the link only lives in the task's text.
- So the same piece of work can show up twice, once as ticket #42 in Issues and once as a task in Tasks, with nothing tying them together and two different statuses.

## The problem

"We have issues and tasks." Two tabs that both look like "the work" make the user decide where to look. Neither tab can answer "is anyone working on #42?". The Issues tab can't show who is on it, and the Tasks tab can't show which ticket a task is for.

What we want:

1. One place to see all the work on a project, whatever its origin.
2. At a glance, what lives on GitHub and what lives only in Loft.
3. No second source of truth: GitHub stays the tracker for tickets, and Loft stays the place that runs agents.

## Options

### A. One list, origin visible (recommended)

The **Tasks** tab lists Loft tasks *and* the projects' open GitHub issues. Each row starts with an origin mark:

- the Loft house for a Loft-only task;
- the GitHub mark for an issue nobody has picked up yet;
- the house with a small GitHub badge for a task that points at a ticket, shown once, with its `#42`.

A source filter (**All · Loft · GitHub**) sits next to the project and agent filters, and search accepts `#42`. GitHub issues without a task get their own group, **On GitHub · no task yet**, so they never mix into "In progress". The Issues tab goes away. Its detail view (body, labels, sub-issues, blocking) becomes the detail of a GitHub row.

Two actions connect the two halves:

- **Hand to an agent** on a GitHub issue creates a task that points at the ticket and starts it.
- **Publish to GitHub** on a Loft task creates the ticket, then links the task to it.

![Recommended: one list, a task linked to ticket #42](img/linked.png)
*Mockup. The list mixes Loft tasks (house), GitHub issues (GitHub mark) and tasks for a ticket (house + badge). The detail splits what GitHub owns from what Loft owns.*

![Recommended: a GitHub issue with no task yet](img/issue.png)
*Mockup. A ticket nobody is working on, with "Hand this ticket to an agent". Its blocker shows the task working on it.*

![Recommended: publishing a Loft task as a ticket](img/publish.png)
*Mockup. "Publish to GitHub" on a Loft-only task. The task stays and points at the new issue.*

**For:** one place to look, and the origin shows on every row. It's a small model change (one link field). It reuses Pedro's GitHub fetching and issue view as they are.
**Against:** the Tasks list gets longer, since busy repos have many issues. Needs good defaults: open only, the "no task yet" group folded once it's over ~20, `ready-for-agent` first. Closed GitHub issues only show through their task, or behind a "closed" filter.

### B. Two tabs, cross-linked

Keep **Issues** and **Tasks** as separate tabs and add the same link field. An issue row shows a "task · in progress" chip with the agent, and the issue view shows a card that opens the task. A task shows its `#42` and opens the issue.

![Alternative: two tabs, cross-linked](img/two-tabs.png)
*Mockup. Pedro's Issues tab with task chips, a "without a task" filter and a card linking to the task.*

**For:** the smallest change, since #34 merges as is. Each tab stays simple and true to its source.
**Against:** the redundancy Gustavo pointed out stays. Two tabs, two status vocabularies, and the user still has to know where to look. Every new feature (filters, push, iOS) gets built twice.

### C. GitHub issues as a synced kind of task

Import every issue into the `tasks` table (`source: github`) and sync both ways: edit the title in Loft and it changes on GitHub, close on GitHub and the task closes.

**For:** one table, so every task feature works on issues for free.
**Against:** two-way sync is where the bugs live: conflicts, webhooks or polling, rate limits, deleted and transferred issues, and labels Loft doesn't know. It creates a second copy of GitHub's truth. It goes against #34's choice to read GitHub live and never write from the tab. Too big for what we get.

## Recommendation

**Option A.** One **Tasks** tab with a visible origin on every row, a source filter, and a task ↔ ticket link. GitHub stays the source of truth for tickets, and Loft for who works on what. Keep #34's live GitHub reads and issue view, and move them under Tasks instead of a tab of their own.

If we want #34 merged before the unified list exists, merge it with its Issues tab and build A in phases: B's cross-links come for free in phase 1.

## Origin model

**One new field on a task, no new table.** A task gets an optional `ticket`:

```json
"ticket": { "repo": "octo/api", "number": 42, "url": "https://github.com/octo/api/issues/42" }
```

- `source` is derived, not stored: a task with no `ticket` is **Loft**, a task with one is **ticket**. A GitHub issue no task points at is a **GitHub** row, and it isn't in the `tasks` table at all.
- At most one open task per ticket. **Hand to an agent** on an issue that already has an open task opens that task instead. A retried task (failed → ready) keeps its ticket.
- Tasks created by `publish_tickets` (if we decide the Lead's tickets should become tasks right away, see the questions) get `ticket` filled in from `published_issues`.

**Who owns what.**

| Field | Owner of the truth | In Loft |
|---|---|---|
| Title, description, labels | GitHub, once there's a ticket | Shown from GitHub. Editing opens GitHub, or Loft writes through the user's token (see questions) |
| Open / closed | GitHub | Shown. A task that's done doesn't close the ticket by itself |
| Owner (assignee) | GitHub. Always a person | Shown |
| Blocked by, sub-issues | GitHub | Shown. For a ticket they replace the task's `depends_on` and `parent_id` |
| Agent, execution status, progress note, summary, error | Loft | As today |
| Thread (comments, notes) | Loft | As today. Not mirrored to GitHub |
| Priority, due date | Loft | As today |

**Sync direction.**

- **GitHub → Loft:** read only and live, the way #34 already reads. The list refetches on open, on refresh, and on the `issues` event Loft sends after it publishes. No webhooks for now.
- **Loft → GitHub:** only on an explicit action by the user (Publish to GitHub, maybe Close ticket), or through Planning's existing `publish_spec` / `publish_tickets`. Never in the background.

**Publishing a task to GitHub.**

1. The user presses **Publish to GitHub** on a Loft-only task in a project that has a GitHub repo.
2. Loft creates the issue with the user's token, through the same path as `publish_tickets`: the task's title and description, `ready-for-agent` by default, owned by the user. A retry doesn't create a second issue (`published_issues`, keyed by the task id).
3. The task gets its `ticket` and keeps its agent, status, thread and priority.
4. From then on, the ticket's title, description and labels come from GitHub.

**Status and labels.**

| GitHub | Loft task (if linked) | Row shows |
|---|---|---|
| open, no task | — | "open" in **On GitHub · no task yet** |
| open | any open task status | the task's status, in the task's group |
| open | done | "done · ticket still open", with a **Close #42** button |
| closed | open task | a warning: "#42 was closed on GitHub", with **Cancel task** / **Keep working** |
| closed | done or cancelled | in **Done**, with "closed on GitHub" |
| label `ready-for-agent` | — | listed first among GitHub rows |
| label `spec` | — | a spec row with sub-issue progress (1/4). A spec is never handed to an agent as a single task |

## Open questions for tonight

1. **Name.** Keep **Tasks** for the merged list (matches the glossary) or call it something new, like **Work**?
2. **Should Planning create tasks?** When the Lead publishes tickets, should Loft also create a (not started) task per ticket, or only when someone hands a ticket to an agent?
3. **Closing tickets.** When a linked task is done, close the ticket automatically, only when its PR merges (`Closes #42`), or always leave it to the user?
4. **Editing.** Can the user edit a ticket's title and description from Loft (written to GitHub with their token), or does Loft send them to GitHub to edit?
5. **What GitHub rows to show.** Every open issue, or only `ready-for-agent` / `spec` plus anything assigned to the user? Busy repos could flood the list.
6. **Comments.** Mirror GitHub comments into the task thread (read only), or link to them?
7. **iOS.** The app has Tasks but no Issues. Does the unified list go to iOS in the same phase or after?
8. **#34.**  Merge it with the Issues tab first and fold the tab into Tasks later, or rework the nav inside #34 before it merges?
9. **Images in docs.** `.gitignore` keeps every image out of the repo. Allow `docs/plans/img/` (and PR screenshots), or host them somewhere else?

## Plan

Sizes: S = under a day, M = 1–2 days, L = 3–5 days of agent work, including tests.

| Phase | What | Size |
|---|---|---|
| 1. The link | `ticket` on tasks (backend, API, MCP `create_task` / `update_task`). The task row and detail show `#42` and open the issue. **Hand to an agent** on an issue creates a linked task. With #34's tab still in place, that's option B. | M |
| 2. One list | The Tasks page lists GitHub issues next to tasks: origin marks, **All · Loft · GitHub** filter, the "On GitHub · no task yet" group, `#42` search. The issue view moves under `/tasks/github/:project/:number`. The Issues tab goes away and its URLs redirect. | L |
| 3. Publish | **Publish to GitHub** on a Loft-only task, sharing `planning.rs`'s publishing path and retry safety. | M |
| 4. Statuses | The status and label rules above: done-but-open, closed-under-a-running-task, **Close #42**. "Needs you" items for the two mismatches. | S–M |
| 5. iOS | The same list and origin marks in the iOS Tasks tab. Hand to an agent from the phone. | L |

Phases 1 and 2 are the core. 3 and 4 can follow in either order, and 5 needs the backend from 1–2.
