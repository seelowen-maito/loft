# Loft

A local command center where a team of Claude Code agents works on the user's projects, followed from the web and the iOS app.

## Agents and work

**Agent**:
A long-lived Claude Code worker with its own name, instructions, memory and workspace.
_Avoid_: Bot, assistant

**Group**:
A label that keeps agents together, such as Dineout or Loft. It owns no projects, folder or main project, and is never a repo or a place to publish: projects belong to each agent.
_Avoid_: Workspace, organisation

**Team**:
The agents of one group. A team has at most one Lead.
_Avoid_: Squad, crew

**Project**:
One folder under the user's projects directory that is, or will become, one git repo: its `origin` is the project's repo, and Loft can create the repo for a project that has none. A folder holding several repos is never a project.
_Avoid_: Workspace (that is the agent's private folder), group

**Lead**:
The agent with the Lead role: it plans with the user (grill, spec, tickets) and answers questions. A team has at most one.
_Avoid_: Orchestrator, manager, planner

**Role**:
What an agent does in its team: Lead, Dev, Code reviewer, QA, Security or Custom. Loft wakes an agent when its role's step in the work comes up and sends it that role's brief with the wake; a Custom agent has no step and follows only its own instructions. An agent has exactly one role.
_Avoid_: Job, type, flag

**Delivery**:
Loft running a spec's tickets from published to merged: it starts each unblocked ticket on the Dev, hands its commits to the reviewers while it follows their CI on GitHub and the Dev starts the next unblocked ticket (with at most one of its tickets in review; a fix waits for the ticket being built and comes before any new one), closes it when they pass and the CI is green, and tells the user when the spec PR is ready or a ticket needs them. No agent coordinates it.
_Avoid_: Build, run, pipeline

**Stalled**:
A Delivery waiting on a step no agent is working on: the run woken for it ended, failed or was lost to a restart without the call its step expects. It stays stalled, and tells only the user, until the user resumes it.
_Avoid_: Stuck, failed, paused

**Paused**:
A Delivery the user paused: Loft stopped the run in flight and sends no more wakes until the user resumes it. Unlike a stalled one, it sends no push and adds nothing to Needs you.
_Avoid_: Stopped, cancelled, on hold

**Resume**:
The user's action that restarts a stalled or paused Delivery, or a ticket that needs the user after too many change rounds: Loft first checks GitHub for what is already done, then wakes only the agent the step still waits on.
_Avoid_: Retry, restart

**Role brief**:
The fixed instructions Loft writes for a role and sends with that role's wake in a Delivery. An agent's standing instructions add to it but can't change how the Delivery works.
_Avoid_: Role prompt, template

**Run**:
One stretch of an agent working, from what woke it (a message, an issue, a schedule) until it replies. The chat shows how long it has been running and, at the end, how long it took.
_Avoid_: Session, turn, job

**Session**:
The conversation an agent carries from run to run until the user clears it; Compact summarises it in place. Every model call in a run resends the whole session, mostly from the prompt cache.
_Avoid_: Conversation, thread

**Memory**:
A fact an agent keeps across sessions: a one-line title and a body. Every session starts with the titles of the agent's memories; the agent reads a body with `recall` when it needs it, and updates a memory instead of saving a near-copy.
_Avoid_: Note, knowledge

**Unread reply**:
The end of a run the user started (by a message or a schedule) that the user hasn't seen yet in that agent's chat. Runs started only by other agents never leave one.
_Avoid_: Notification, new message, unread message

**Agent process**:
A process started by one of an agent's runs, including servers it left running in the background. Each run starts in its own process session, so Loft knows which agent owns which process. An agent may stop its own processes freely; stopping any other process needs the user's approval.
_Avoid_: Agent job, child process

**Port range**:
The fixed set of local ports Loft gives one agent for its dev servers, the same across runs. An agent serves only on ports in its own range, so agents never collide.
_Avoid_: Port pool, port block

**Agent device**:
The iOS Simulator device Loft creates for one agent the first time it needs one, named after the agent. The agent installs and runs apps only on its own device; the user picks which agent's device to watch.
_Avoid_: Booted device, shared simulator

**Subagent**:
A short-lived Claude Code worker an agent starts inside a run for a side job (a search, a self-contained task). It ends with the run at the latest and never talks to the user.
_Avoid_: Sub-agent, child agent, worker

**Agent message**:
A message one agent sends another through Loft, as opposed to a message from the user. It says whether the recipient must act on it (needs action) or only needs to know (FYI), and it follows the messaging rules Loft gives every agent.
_Avoid_: Ping, DM, hand-off

**Mention**:
An `@` reference in a message: `@name` points at an agent, `@path` at a file or folder, read relative to the agent's project folder. As the user types after `@`, Loft suggests matching agents and files.
_Avoid_: Tag, reference

**Draft**:
A message typed in an agent's composer or a grill answer but not sent yet, with its pasted images and attached issue. Each agent keeps its own until it is sent or the agent's context is cleared.
_Avoid_: Unsent message, pending message

**Issue**:
A work item in Loft's list. It belongs to exactly one project; lists filter issues by group, but an issue never belongs to a group. An issue is either a GitHub issue or a Loft issue.
_Avoid_: Task (kept only in old API names), job, card

**GitHub issue**:
An issue mirrored two-way with its project's repo on GitHub: title, description, labels, owner and open or closed.
_Avoid_: Remote issue, synced task

**Loft issue**:
An issue that lives only in Loft, because its project has no repo yet or it isn't published; it can be published to GitHub later.
_Avoid_: Local task, draft issue

## Planning

**Planning**:
The flow that takes an idea to agent-ready tickets: grill, spec, tickets.
_Avoid_: Grill mode (grilling is one step of it)

**Round**:
The set of questions an agent asks at once during a grill: every question whose prerequisites are settled. The user answers them all, then sends the round.
_Avoid_: Batch, queue

**Recommendation**:
The agent's proposed answer to a question in a round, which the user can accept as-is.
_Avoid_: Default, suggestion

**Spec**:
An issue labelled `spec` that describes a feature from the user's perspective, written from a grill: on GitHub, or a Loft issue when the project has no GitHub repo.
_Avoid_: PRD, plan, design doc

**Ticket**:
An issue under a spec, labelled `ready-for-agent`: one tracer-bullet slice of the spec, with blocking links to the tickets it waits on. On GitHub (native sub-issue and blocking links), or a Loft issue under the spec when the project has no GitHub repo.
_Avoid_: Task, story, sub-task

**Spec branch**:
The one branch a spec is built on, `spec/<N>-<slug>`, which Loft creates when it publishes the spec on GitHub: from the grill's docs commits when there are any, otherwise from the default branch. Every ticket of the spec is one or more commits on it.
_Avoid_: Feature branch, shared branch

**Spec PR**:
The one pull request from a spec branch to the default branch, opened as a draft with `Closes #<spec>` when the spec is published. The spec ships when it merges.
_Avoid_: Feature PR, epic PR

**Owner**:
The GitHub user a spec or ticket is assigned to. Always a person, never an agent: agents act on GitHub as the user who runs Loft.
_Avoid_: Assignee (GitHub's field name; the concept is ownership), claimer

## Usage and spend

**Usage week**:
The 7-day window of the Claude account's plan usage, ending at its weekly reset (the "week" meter in the navbar). When Loft can't read the plan usage, it stands in with the last 7 days and says so.
_Avoid_: Billing week, 7 days, week

**All-time spend**:
The runs and estimated cost of every agent run since Loft started counting, kept as a stored running total: it never goes down, even when an agent is deleted or its history is cleared.
_Avoid_: Total cost, lifetime cost

## Push

**Push relay**:
A hosted service that holds Loft's Apple push key and forwards a Loft install's pushes to Apple, so installs run by other people never hold the key. It only forwards: which alerts a phone wants, quiet hours and rate limits stay in the install.
_Avoid_: Push server, push proxy, gateway

**Relay key**:
The secret an install uses to push through the push relay. The user gets it by signing in to the push portal with Apple; it belongs to that Apple ID and can be revoked there. There are no anonymous keys: without one, the install does not push through the relay.
_Avoid_: Relay token, account, login

**Push portal**:
A small web page on the relay's Vercel project, at the relay's own address, where a user signs in with Apple to create, name and revoke relay keys. It stores only Apple's user id and hashed keys.
_Avoid_: Dashboard, account page

## Threads

**Thread**:
A Loft issue's entries, oldest first: comments, notes and status entries.
_Avoid_: Timeline, history, log

**Comment**:
A thread entry someone wrote: the user, an agent, or someone on GitHub. On an issue that mirrors a GitHub issue, a Loft comment goes to GitHub only after the user approves it; GitHub's comments arrive in the thread.
_Avoid_: Reply, message

**Note**:
An agent's progress update on a Loft issue, written as it works. It stays in Loft.
_Avoid_: Comment, log entry

**Status entry**:
A thread entry Loft writes when something changes: created, started, done, reassigned, closed on GitHub. It stays in Loft.
_Avoid_: Event, activity
