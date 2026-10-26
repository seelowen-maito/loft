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
A GitHub issue labelled `spec` that describes a feature from the user's perspective, written from a grill.
_Avoid_: PRD, plan, design doc

**Ticket**:
A GitHub issue under a spec, labelled `ready-for-agent`: one tracer-bullet slice of the spec, with native blocking links to the tickets it waits on.
_Avoid_: Task, story, sub-task

**Owner**:
The GitHub user a spec or ticket is assigned to. Always a person, never an agent: agents act on GitHub as the user who runs Loft.
_Avoid_: Assignee (GitHub's field name; the concept is ownership), claimer
