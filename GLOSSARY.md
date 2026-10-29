# Loft

A local command center where a team of Claude Code agents works on the user's projects, followed from the web and the iOS app.

## Agents and work

**Agent**:
A long-lived Claude Code worker with its own name, instructions, memory and workspace.
_Avoid_: Bot, assistant

**Team**:
The agents that share a project.
_Avoid_: Group, squad

**Project**:
A folder under the user's projects directory. Its git `origin`, once it has one, is the project's repo; Loft can create the repo for a project that has none.
_Avoid_: Workspace (that is the agent's private folder)

**Lead**:
The one agent per team marked as lead: it plans with the user and hands ready tickets to the rest of the team. A team has at most one.
_Avoid_: Orchestrator, manager, planner

**Run**:
One stretch of an agent working, from what woke it (a message, a task, a schedule) until it replies. The chat shows how long it has been running and, at the end, how long it took.
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

**Subagent**:
A short-lived Claude Code worker an agent starts inside a run for a side job (a search, a self-contained task). It ends with the run at the latest and never talks to the user.
_Avoid_: Sub-agent, child agent, worker

**Agent message**:
A message one agent sends another through Loft, as opposed to a message from the user. It says whether the recipient must act on it (needs action) or only needs to know (FYI), and it follows the messaging rules Loft gives every agent.
_Avoid_: Ping, DM, hand-off

**Draft**:
A message typed in an agent's composer or a grill answer but not sent yet, with its pasted images and attached task. Each agent keeps its own until it is sent or the agent's context is cleared.
_Avoid_: Unsent message, pending message

**Task**:
An assignment of work to an agent, tracked in Loft's shared list. A task may point at a ticket.
_Avoid_: Ticket, job, issue

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

**Owner**:
The GitHub user a spec or ticket is assigned to. Always a person, never an agent: agents act on GitHub as the user who runs Loft.
_Avoid_: Assignee (GitHub's field name; the concept is ownership), claimer

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
