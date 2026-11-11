//! Deliveries: Loft running a spec's tickets from published to merged, one ticket at a time, with no
//! agent coordinating it. The record (`db::Delivery`) is written when the spec is published and its
//! tickets added as they are; "Start delivery" on the end-of-planning card, or on a spec's or a standalone
//! ticket's issue page, starts the first one. A spec published earlier gets its record from GitHub there.

use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::api::{ApiResult, bad, not_found};
use crate::db::{Delivery, DeliveryTicket};
use crate::state::AppState;

/// Has Loft track the Delivery's PR, owned by its Dev or else its planner, so its card shows and checkers can
/// set verdicts on it. A PR Loft already tracks keeps its owner and is read again: the planner, when it was
/// opened before a Dev was assigned, which is the same team.
pub async fn track_pr(state: &AppState, d: &Delivery) {
    let Some(pr) = d.pr else { return };
    if let Some(known) = state.db.list_pull_requests(None).ok().and_then(|ps| ps.into_iter().find(|p| p.slug == d.repo && p.number == pr)) {
        let _ = crate::prs::refresh(state, &known.id).await;
        return;
    }
    let owner = [&d.dev, &d.planner].into_iter().find(|id| !id.is_empty() && matches!(state.db.get_agent(id), Ok(Some(_))));
    if let Some(owner) = owner
        && let Err(e) = crate::prs::register(state, owner, &d.repo, pr, None).await
    {
        tracing::warn!("delivery {}#{}: couldn't track PR #{pr}: {e}", d.repo, d.spec);
    }
}

/// On start-up: tracks the PRs of Deliveries not yet settled, opened before Loft tracked them.
pub async fn track_open_prs(state: AppState) {
    for s in ["planned", "running", "final_review", "final_fixes", "needs_human", "stalled", "paused"] {
        for d in state.db.deliveries_in_state(s).unwrap_or_default() {
            track_pr(&state, &d).await;
        }
    }
}

/// The Delivery of spec #spec on `repo`, made when the spec is published, with its branch. A record
/// already there keeps what it has, and gets the branch if it had none.
pub fn record_spec(state: &AppState, repo: &str, spec: i64, project: &str, planner: &str, branch: &str) {
    let d = match state.db.get_delivery(repo, spec) {
        Ok(Some(mut d)) => {
            if d.branch.is_empty() {
                d.branch = branch.into();
            }
            if d.project.is_empty() {
                (d.project, d.planner) = (project.into(), planner.into());
            }
            d
        }
        Ok(None) => Delivery {
            repo: repo.into(),
            spec,
            project: project.into(),
            planner: planner.into(),
            branch: branch.into(),
            pr: None,
            state: "planned".into(),
            resume_to: String::new(),
            stall_reason: String::new(),
            stalled_at: String::new(),
            stalled_on: vec![],
            done_at: String::new(),
            ready_retry: false,
            tickets: vec![],
            current: None,
            dev: String::new(),
            final_rounds: 0,
            last_notes: String::new(),
            migrated: false,
            standalone: false,
            local: false,
            spec_id: String::new(),
            base: String::new(),
            title: String::new(),
            created_at: crate::db::now(),
        },
        Err(e) => return tracing::warn!("delivery {repo}#{spec}: {e}"),
    };
    if state.db.save_delivery(&d).is_ok() {
        changed(state, &d);
    }
}

/// Tells the clients showing the Delivery that it changed.
fn changed(state: &AppState, d: &Delivery) {
    state.emit(crate::state::Event::Delivery { repo: d.repo.clone(), spec: d.spec });
}

/// The Delivery of standalone ticket #n on `repo`, its only ticket, on the branch `ticket/<N>-<slug>`.
/// One already there is left as it is. The branch and its PR are made when the Delivery starts.
pub fn record_standalone(state: &AppState, repo: &str, n: i64, title: &str, project: &str, planner: &str) {
    if !matches!(state.db.get_delivery(repo, n), Ok(None)) {
        return;
    }
    record_spec(state, repo, n, project, planner, &crate::planning::ticket_branch(n, title));
    update(state, repo, n, |d| d.standalone = true);
    record_ticket(state, repo, n, n, title, vec![]);
}

/// The `repo` key of a plan kept in Loft in `project`.
pub fn local_repo(project: &str) -> String {
    format!("loft:{}", project.trim())
}

/// The Delivery of the plan kept in Loft whose spec is Loft issue `spec_id`.
pub fn local_of(state: &AppState, project: &str, spec_id: &str) -> Option<Delivery> {
    state.db.deliveries(&local_repo(project), false).ok()?.into_iter().find(|d| d.spec_id == spec_id)
}

/// The Delivery of a plan kept in Loft, made when its spec is saved: the next number in the project, and
/// the spec branch `spec/<number>-<slug>` it gets in the project's own repo when it starts.
pub fn record_local_spec(state: &AppState, project: &str, spec_id: &str, title: &str, planner: &str) {
    if local_of(state, project, spec_id).is_some() {
        return;
    }
    let repo = local_repo(project);
    let n = state.db.deliveries(&repo, false).unwrap_or_default().iter().map(|d| d.spec).max().unwrap_or(0) + 1;
    record_spec(state, &repo, n, project.trim(), planner, &crate::planning::spec_branch(n, title));
    update(state, &repo, n, |d| (d.local, d.spec_id, d.title) = (true, spec_id.into(), title.into()));
}

/// Adds a ticket of a plan kept in Loft to its Delivery: numbered in the plan, blocked by the tickets with
/// the Loft issues `blocked_by`. One already there is left as it is.
pub fn record_local_ticket(state: &AppState, project: &str, spec_id: &str, id: &str, title: &str, blocked_by: &[String]) {
    let Some(d) = local_of(state, project, spec_id) else { return };
    update(state, &d.repo, d.spec, |d| {
        if d.tickets.iter().any(|t| t.id == id) {
            return;
        }
        let number = d.tickets.iter().map(|t| t.number).max().unwrap_or(0) + 1;
        let blocked_by = d.tickets.iter().filter(|t| blocked_by.contains(&t.id)).map(|t| t.number).collect();
        d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: id.into(), skipped: String::new(), agent: String::new(), notes: String::new() });
    });
}

/// Changes the Delivery of spec #spec on `repo`, if it has one, and saves it.
pub fn update(state: &AppState, repo: &str, spec: i64, f: impl FnOnce(&mut Delivery)) -> Option<Delivery> {
    let mut d = state.db.get_delivery(repo, spec).ok()??;
    f(&mut d);
    state.db.save_delivery(&d).ok()?;
    changed(state, &d);
    Some(d)
}

/// What a Delivery wake asks of its agent: the step, and the call that moves it on.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Hand the ready tickets out to Devs: ends with `assign_ticket`, or with nothing left to hand out.
    Assign,
    /// Build ticket #ticket, after `round` changes rounds: ends with `submit_ticket`.
    Build { ticket: i64, round: u32 },
    /// Review ticket #ticket as `role` in changes round `round`: ends with `review_ticket`.
    Review { ticket: i64, role: String, round: u32 },
    /// The final Code Review, after `round` rounds of fixes: ends with `review_ticket`.
    FinalReview { round: u32 },
    /// Fix what the final Code Review found in round `round`: ends with `submit_ticket`.
    FinalFixes { round: u32 },
}

impl Step {
    /// Whether `d` still waits on this step: its call never came.
    fn waiting(&self, d: &Delivery) -> bool {
        match self {
            // Tickets in flight keep the Delivery going, and their close wakes the Lead again.
            Step::Assign => d.state == "running" && !ready_tickets(d).is_empty() && none_in_flight(d),
            Step::Build { ticket, round } => d.ticket(*ticket).is_some_and(|t| t.state == "started" && t.rounds == *round),
            Step::Review { ticket, role, round } => d.ticket(*ticket).is_some_and(|t| t.state == "review" && t.rounds == *round && t.verdicts.get(role).is_some_and(|v| v == "pending")),
            // A pass whose ready call failed waits on Loft's retry, not on the Code reviewer.
            Step::FinalReview { round } => d.step_state() == "final_review" && d.final_rounds == *round && !d.ready_retry,
            Step::FinalFixes { round } => d.step_state() == "final_fixes" && d.final_rounds == *round,
        }
    }
}

/// A Delivery wake: the step of the Delivery of spec #spec on `repo` it woke an agent for. It goes with the
/// wake's prompt, queued or not, to the run that takes it (`agents::start_with`), checked when that run ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Wake {
    pub repo: String,
    pub spec: i64,
    pub step: Step,
}

fn wake_for(d: &Delivery, step: Step) -> Wake {
    Wake { repo: d.repo.clone(), spec: d.spec, step }
}

/// A run of `agent_id` ended (`failed`: claude reported an error, or exited without a result nobody asked
/// for). Each Delivery step it was woken for that still waits on its call stalls its Delivery, unless
/// another Delivery wake for the agent is still queued: the run that takes it carries on.
pub async fn run_ended(state: &AppState, agent_id: &str, failed: bool) {
    let ended = {
        let mut rt = state.rt.lock().unwrap();
        let ended = rt.delivery_running.remove(agent_id).unwrap_or_default();
        if let Some(q) = rt.delivery_queued.get_mut(agent_id).filter(|q| !q.is_empty()) {
            // The run that takes the queue carries on: it checks these steps too when it ends, whichever
            // Delivery its own wake is for.
            q.extend(ended.into_iter().map(|w| (w, None)));
            return;
        }
        ended
    };
    for w in ended {
        let _held = state.publish_locks.of(&w.repo).lock_owned().await;
        let Some(d) = state.db.get_delivery(&w.repo, w.spec).ok().flatten() else { continue };
        // A Dev whose ticket the Lead moved to another one has nothing left to submit.
        if let Step::Build { ticket, .. } = &w.step
            && d.ticket(*ticket).is_some_and(|t| builder_id(&d, t) != agent_id)
        {
            continue;
        }
        if w.step.waiting(&d) {
            let ticket = match &w.step {
                Step::Build { ticket, .. } | Step::Review { ticket, .. } => Some(*ticket),
                _ => None,
            };
            stall(state, &d, if failed { "run_failed" } else { "no_call" }, ticket.into_iter().collect());
        }
    }
}

/// Marks `d` stalled with `reason`, remembering the state to go back to. Only a Delivery whose step an
/// agent should be working on: never a paused one, nor one waiting for the user (`needs_human`).
fn stall(state: &AppState, d: &Delivery, reason: &str, tickets: Vec<i64>) -> Option<Delivery> {
    // Already stalled: another ticket's step joins the ones that stall it.
    if d.state == "stalled" && !tickets.is_empty() {
        return update(state, &d.repo, d.spec, |d| {
            for n in &tickets {
                if !d.stalled_on.contains(n) {
                    d.stalled_on.push(*n);
                }
            }
        });
    }
    if !matches!(d.state.as_str(), "running" | "final_review" | "final_fixes") {
        return None;
    }
    tracing::info!("delivery {}#{}: stalled ({reason})", d.repo, d.spec);
    update(state, &d.repo, d.spec, |d| {
        d.resume_to = std::mem::replace(&mut d.state, "stalled".into());
        (d.stall_reason, d.stalled_at) = (reason.into(), crate::db::now());
        d.stalled_on = tickets;
    })
}

/// The id of a stalled Delivery's Needs you item: a new stall is a new item, pushed again.
pub(crate) fn stalled_item_id(d: &Delivery) -> String {
    format!("stalled:{}:{}:{}", d.repo, d.spec, d.stalled_at)
}

/// At start-up, before any run: runs and their queues live only in memory, so no agent is still working on
/// any Delivery's step. Each Delivery with a ticket being built or in review, or in its final review or
/// fixes, stalls: its run was lost to the restart. A final review that passed and only waits on the ready
/// retry doesn't. Its Needs you item is news, so it's pushed even though the pusher's first look takes
/// what's already waiting as old.
pub fn stall_lost_runs(state: &AppState) {
    for s in ["running", "final_review", "final_fixes"] {
        for d in state.db.deliveries_in_state(s).unwrap_or_default() {
            let lost = match s {
                // Or the Lead's wake: it has tickets to hand out, and nothing is in flight to wake it later.
                "running" => d.tickets.iter().any(|t| matches!(t.state.as_str(), "started" | "review")) || waits_on_lead(state, &d),
                // A final review that passed only waits on Loft's ready retry, which survives the restart.
                "final_review" => !d.ready_retry,
                _ => true,
            };
            if lost
                && let Some(d) = stall(state, &d, "restart", d.tickets.iter().filter(|t| matches!(t.state.as_str(), "started" | "review")).map(|t| t.number).collect())
            {
                state.rt.lock().unwrap().news_at_start.insert(stalled_item_id(&d));
            }
        }
    }
}

/// The call a stalled Delivery waited on for ticket #n came after all (say the user nudged the agent): it
/// carries on, unless another ticket still stalls it.
fn unstalled(state: &AppState, d: Delivery, n: i64) -> Delivery {
    if d.state != "stalled" {
        return d;
    }
    update(state, &d.repo.clone(), d.spec, |d| d.clear_stall(n)).unwrap_or(d)
}

/// Adds a published ticket to its spec's Delivery, after the tickets that block it. One already there is left as it is.
pub fn record_ticket(state: &AppState, repo: &str, spec: i64, number: i64, title: &str, blocked_by: Vec<i64>) {
    update(state, repo, spec, |d| {
        if !d.tickets.iter().any(|t| t.number == number) {
            d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: String::new(), skipped: String::new(), agent: String::new(), notes: String::new() });
        }
    });
}

/// The Delivery that ticket #number on `repo` belongs to.
pub fn of_ticket(state: &AppState, repo: &str, number: i64) -> Option<Delivery> {
    state.db.deliveries(repo, false).ok()?.into_iter().find(|d| d.tickets.iter().any(|t| t.number == number))
}

/// Notes on its Delivery that ticket #number closed. The Delivery as it is now, if the ticket has one.
pub fn ticket_closed(state: &AppState, repo: &str, number: i64) -> Option<Delivery> {
    let d = of_ticket(state, repo, number)?;
    update(state, repo, d.spec, |d| {
        if let Some(t) = d.ticket_mut(number) {
            t.state = "closed".into();
        }
        if d.current == Some(number) {
            d.current = None;
        }
    })
}

/// The next ticket to build: the first open one whose blockers are all closed.
pub(crate) fn next_ticket(d: &Delivery) -> Option<&DeliveryTicket> {
    let closed = |n: &i64| d.tickets.iter().any(|t| t.number == *n && t.state == "closed");
    d.tickets.iter().find(|t| t.state == "open" && t.blocked_by.iter().all(closed))
}

/// No ticket is being built, reviewed or waiting for the user: nothing will wake the Lead by closing.
fn none_in_flight(d: &Delivery) -> bool {
    d.tickets.iter().all(|t| !matches!(t.state.as_str(), "started" | "review" | "closing" | "needs_human"))
}

/// A running Delivery of a team with a Lead that has tickets to hand out and nothing in flight: it waits on the Lead.
fn waits_on_lead(state: &AppState, d: &Delivery) -> bool {
    d.state == "running" && none_in_flight(d) && !ready_tickets(d).is_empty() && lead_of(state, d).is_some()
}

/// The tickets ready to hand out: open, with every blocker closed, and nobody on them.
fn ready_tickets(d: &Delivery) -> Vec<&DeliveryTicket> {
    let closed = |n: &i64| d.tickets.iter().any(|t| t.number == *n && t.state == "closed");
    d.tickets.iter().filter(|t| t.state == "open" && t.agent.is_empty() && t.blocked_by.iter().all(closed)).collect()
}

/// The agent building ticket `t`: the one it was handed to, else the Delivery's Dev.
fn builder_id<'a>(d: &'a Delivery, t: &'a DeliveryTicket) -> &'a str {
    if t.agent.is_empty() { &d.dev } else { &t.agent }
}

/// A Dev of the Delivery's team: its project's Dev role, in its main project or one of its projects.
fn is_team_dev(d: &Delivery, a: &crate::db::Agent) -> bool {
    let project = d.project.trim();
    !project.is_empty() && a.role == "dev" && (a.project.trim() == project || a.has_project(project))
}

/// The team's Lead, when the team also has a Dev: then the Lead hands the tickets out (`assign_ticket`).
fn lead_of(state: &AppState, d: &Delivery) -> Option<crate::db::Agent> {
    with_role(state, d, "lead").filter(|_| with_role(state, d, "dev").is_some())
}

/// How `Agent::delivery_ticket` names ticket #n of `d`, or with None the final review of its spec PR. A
/// ticket gets the key a message naming it gets (`agents::message_ticket_key`): `owner/repo#N` on GitHub,
/// its Loft issue id in a plan kept in Loft. So a hand-off by message after a Delivery wake on the same
/// ticket isn't a new one.
fn ticket_key(d: &Delivery, n: Option<i64>) -> String {
    let fallback = || format!("{}#{}/{}", d.repo, d.spec, n.map_or("final".to_string(), |n| n.to_string()));
    match n {
        Some(n) if d.local => d.tickets.iter().find(|t| t.number == n && !t.id.is_empty()).map_or_else(fallback, |t| t.id.clone()),
        Some(n) => format!("{}#{n}", d.repo),
        None => fallback(),
    }
}

/// Whether the Fresh context setting applies to `a`: to every agent but a Lead that only coordinates. A
/// Lead builds the tickets when its team (its group) has no Dev (`builder`).
pub fn fresh_context_applies(state: &AppState, a: &crate::db::Agent) -> bool {
    a.role != "lead" || !state.db.list_agents().unwrap_or_default().iter().any(|o| o.role == "dev" && !a.team().is_empty() && o.team() == a.team())
}

/// What a Delivery delivers: "spec #40", "ticket #7" for a standalone ticket, or `spec "Title"` for a
/// plan kept in Loft.
fn subject(d: &Delivery) -> String {
    if d.local {
        return format!("spec \"{}\"", d.title);
    }
    format!("{} #{}", if d.standalone { "ticket" } else { "spec" }, d.spec)
}

/// How a brief names a ticket: `#7 "Title" (its GitHub link)`, or `"Title" (Loft issue <id>)`.
fn ticket_ref(d: &Delivery, t: &DeliveryTicket) -> String {
    if d.local {
        format!("\"{}\" (Loft issue {})", t.title, t.id)
    } else {
        format!("#{n} \"{}\" (https://github.com/{}/issues/{n})", t.title, d.repo, n = t.number)
    }
}

/// What the Delivery tools take for a ticket: its number, or its Loft issue id.
fn ticket_arg(d: &Delivery, t: &DeliveryTicket) -> String {
    if d.local { format!("\"{}\"", t.id) } else { t.number.to_string() }
}

/// How a reply names a ticket: "#7", or its title in quotes.
fn tag(d: &Delivery, t: &DeliveryTicket) -> String {
    if d.local { format!("\"{}\"", t.title) } else { format!("#{}", t.number) }
}

/// The commit trailer naming a ticket: `Refs #7`, or `Refs <its Loft issue id>`.
fn refs(d: &Delivery, t: &DeliveryTicket) -> String {
    if d.local { format!("Refs {}", t.id) } else { format!("Refs #{}", t.number) }
}

/// The project's own folder, where a plan kept in Loft keeps its spec branch: resolved inside the projects
/// folder, never an agent's workspace, and a repo with its own `.git` folder, so Loft never runs git in a
/// repo an agent made.
fn project_dir(state: &AppState, d: &Delivery) -> Result<std::path::PathBuf, String> {
    crate::repos::project_dir(state, &d.project)
        .filter(|dir| crate::repos::own_git_dir(dir))
        .ok_or_else(|| format!("the project folder of {} isn't a git repo of its own in the projects folder, so Loft won't run git there", d.project.trim()))
}

/// The project's folder as briefs name it.
fn project_shown(state: &AppState, d: &Delivery) -> String {
    project_dir(state, d).unwrap_or_else(|_| state.settings.boot().projects_dir.join(d.project.trim())).display().to_string()
}

/// How Needs you and its push name a Delivery: the spec's (or standalone ticket's) title, else its subject.
pub(crate) fn title_of(d: &Delivery, issue_title: Option<&str>) -> String {
    let title = if d.local { Some(d.title.as_str()) } else if d.standalone { d.tickets.first().map(|t| t.title.as_str()) } else { issue_title };
    title.filter(|t| !t.trim().is_empty()).map_or_else(|| subject(d), str::to_string)
}

/// Why a Delivery stalled, mid-sentence.
fn stall_why(reason: &str) -> &'static str {
    match reason {
        "run_failed" => "the run Loft woke for its step failed",
        "restart" => "its run was lost when Loft restarted",
        _ => "the run Loft woke for its step ended without its call",
    }
}

/// The name of the agent building ticket `t`, else its id when the agent is gone.
fn builder_name(state: &AppState, d: &Delivery, t: &DeliveryTicket) -> String {
    let id = builder_id(d, t);
    state.db.get_agent(id).ok().flatten().map_or_else(|| id.to_string(), |a| a.name)
}

/// The Devs who built the spec's tickets, each once, in ticket order.
pub(crate) fn builders(state: &AppState, d: &Delivery) -> Vec<String> {
    let mut names: Vec<String> = vec![];
    for t in d.tickets.iter().filter(|t| t.skipped.is_empty() && t.state != "open" && !builder_id(d, t).is_empty()) {
        let n = builder_name(state, d, t);
        if !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

/// "A", "A and B", "A, B and C".
fn list_of(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Why a Delivery stalled and what waits, for Needs you: names and titles only.
pub(crate) fn stalled_reason(state: &AppState, d: &Delivery) -> String {
    let why = stall_why(&d.stall_reason);
    let why = format!("{}{}", why[..1].to_uppercase(), &why[1..]);
    // Every ticket in flight, with the agent on it.
    let in_flight: Vec<&DeliveryTicket> = d.tickets.iter().filter(|t| matches!(t.state.as_str(), "started" | "review" | "closing")).collect();
    let what = match d.resume_to.as_str() {
        "final_review" => "the final Code Review".to_string(),
        "final_fixes" => "the fixes from the final Code Review".to_string(),
        _ if in_flight.is_empty() => "its next step".to_string(),
        _ => list_of(&in_flight.iter().map(|t| match t.state.as_str() {
            "started" => format!("building {} on {}", tag(d, t), builder_name(state, d, t)),
            "closing" => format!("closing {}", tag(d, t)),
            _ => format!("the review of {}", tag(d, t)),
        }).collect::<Vec<_>>()),
    };
    format!("{why}. Stopped at: {what}.")
}

/// "spec PR", or "PR" for a standalone ticket.
fn pr_name(d: &Delivery) -> &'static str {
    if d.standalone { "PR" } else { "spec PR" }
}

/// How replies name what the final Code Review looks at: "spec PR #500", or the spec branch.
fn final_tag(d: &Delivery) -> String {
    if d.local { format!("the spec branch `{}`", d.branch) } else { format!("{} #{}", pr_name(d), d.pr.unwrap_or_default()) }
}

/// "spec branch", or "ticket branch" for a standalone ticket.
fn branch_name(d: &Delivery) -> &'static str {
    if d.standalone { "ticket branch" } else { "spec branch" }
}

/// The line every role brief ends its rules with: what the agent's own instructions can and can't change.
fn own_rules(what: &str) -> String {
    format!("Your own instructions can add anything to this brief, like how to {what} in this project, but they can't change the Delivery's tools, its branch or how the work closes or merges.")
}

/// The Dev role brief, sent with the wake that hands a ticket to the Dev.
fn dev_brief(state: &AppState, d: &Delivery, t: &DeliveryTicket) -> String {
    let repo = &d.repo;
    let whose = if d.standalone { "ticket" } else { "spec" };
    let pr = d.pr.map(|pr| format!(" Open no PR: the {} https://github.com/{repo}/pull/{pr} is the {whose}'s only one.", pr_name(d))).unwrap_or_default();
    let spec = match (d.standalone, d.local) {
        (true, _) => String::new(),
        (_, true) => format!(" and its spec (Loft issue {})", d.spec_id),
        _ => format!(" and its spec #{}", d.spec),
    };
    let work = if d.local {
        format!("Work on the spec branch `{}` of the project's own repo `{}`, your clone's `origin`: fetch it, commit there and push it back with `git push origin {}`. There is no PR.", d.branch, project_shown(state, d), d.branch)
    } else {
        format!("Work on the {} `{}`: pull it, commit there and push.{pr}", branch_name(d), d.branch)
    };
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are the Dev on this Delivery: build ticket {ticket}.\n\
         - Read the ticket{spec} first, and build only what the ticket asks.\n\
         - {work}\n\
         - Other Devs may push to the branch while you work: pull it again right before you push.\n\
         - One or more commits, each message ending with `{refs}`.\n\
         - {own}\n\
         - When the commits are pushed and the checks pass, call `mcp__loft__submit_ticket` with {arg} and all the ticket's commit SHAs, oldest first. Loft hands them to the team's reviewers and closes the ticket when they pass: never close it yourself.",
        about = subject(d),
        ticket = ticket_ref(d, t),
        refs = refs(d, t),
        arg = ticket_arg(d, t),
        own = own_rules("build and check the work"),
    )
}

/// Who builds the tickets: the team's Dev, or the agent that planned the spec when the team has none.
fn builder(state: &AppState, d: &Delivery, fallback: &str) -> Option<crate::db::Agent> {
    let all = state.db.list_agents().ok()?;
    let project = d.project.trim();
    let dev = all.iter().find(|a| !project.is_empty() && a.role == "dev" && (a.project.trim() == project || a.has_project(project)));
    let planner = || all.iter().find(|a| a.id == d.planner).or_else(|| all.iter().find(|a| a.id == fallback));
    dev.or_else(planner).cloned()
}

/// What Start delivery did: the ticket and the agent building it, and whether it had already started.
pub struct Started {
    pub ticket: i64,
    pub agent: crate::db::Agent,
    pub already: bool,
}

/// Starts the Delivery of spec #spec on `repo`: labels the first unblocked ticket `in-progress` and wakes
/// the team's Dev (or `fallback`, the agent that planned it, with no Dev) with the Dev brief. A Delivery
/// already started starts nothing.
pub async fn start(state: &AppState, repo: &str, spec: i64, fallback: &str) -> Result<Started, String> {
    let _held = state.publish_locks.of(repo).lock_owned().await;
    let d = state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.ok_or_else(|| format!("Spec #{spec} on {repo} has no Delivery."))?;
    let agent = builder(state, &d, fallback).ok_or("Nobody can build the tickets: the team has no Dev and the agent that planned the spec is gone.")?;
    if d.state != "planned" {
        return Ok(Started { ticket: d.current.unwrap_or_default(), agent, already: true });
    }
    let d = if d.local && d.base.is_empty() {
        let dir = project_dir(state, &d).map_err(|e| format!("The Delivery didn't start: {e}."))?;
        let base = local_branch(&dir, &d.branch).await.map_err(|e| format!("The Delivery didn't start: {e}."))?;
        update(state, repo, spec, |d| d.base = base).ok_or("The Delivery record couldn't be saved.")?
    } else if d.standalone && d.pr.is_none() {
        let title = d.tickets.first().map(|t| t.title.clone()).unwrap_or_default();
        crate::planning::branch_and_pr(state, repo, spec, &title, "ticket", None).await.map_err(|e| format!("The Delivery didn't start: {e}."))?;
        state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.ok_or("The Delivery record is gone.")?
    } else {
        d
    };
    if d.branch.is_empty() {
        return Err(format!("Spec #{spec} has no spec branch yet: call publish_spec again with the same title to make it."));
    }
    let n = next_ticket(&d).map(|t| t.number).ok_or_else(|| format!("Spec #{spec} has no ticket ready to start: every open ticket is blocked, or none was published."))?;
    if let Some(lead) = lead_of(state, &d) {
        // The Lead hands the tickets out (`assign`); the Delivery keeps the bookkeeping.
        let d = update(state, repo, spec, |d| d.state = "running".into()).ok_or("The Delivery record couldn't be saved.")?;
        wake_lead(state, &d, &lead, "The user started this Delivery: hand out its tickets.");
        return Ok(Started { ticket: 0, agent: lead, already: false });
    }
    begin(state, &d, n, &agent).await.map_err(|e| format!("The Delivery didn't start. {e}"))?;
    Ok(Started { ticket: n, agent, already: false })
}

/// The Lead's brief: the spec, its tickets with their state, blockers and agent, and what each Dev is doing.
fn lead_brief(state: &AppState, d: &Delivery, why: &str) -> String {
    let agents = state.db.list_agents().unwrap_or_default();
    let name = |id: &str| agents.iter().find(|a| a.id == id).map_or("someone", |a| a.name.as_str()).to_string();
    let tickets: Vec<String> = d
        .tickets
        .iter()
        .map(|t| {
            let blockers: Vec<String> = t.blocked_by.iter().filter(|b| d.ticket(**b).is_some_and(|b| b.state != "closed")).map(|b| format!("#{b}")).collect();
            let state = match t.state.as_str() {
                "open" if !blockers.is_empty() => format!("blocked by {}", blockers.join(", ")),
                "open" => "ready, nobody on it".to_string(),
                "elsewhere" => "built elsewhere".to_string(),
                s if !t.agent.is_empty() && s != "closed" => format!("{s}, with {}", name(&t.agent)),
                s => s.to_string(),
            };
            format!("  - {}: {state}", ticket_ref(d, t))
        })
        .collect();
    let devs: Vec<String> = agents
        .iter()
        .filter(|a| is_team_dev(d, a))
        .map(|a| {
            let on: Vec<String> = d.tickets.iter().filter(|t| t.agent == a.id && matches!(t.state.as_str(), "started" | "review" | "closing" | "needs_human")).map(|t| tag(d, t)).collect();
            format!("  - {}: {}", a.name, if on.is_empty() { "free".to_string() } else { format!("on {}", on.join(", ")) })
        })
        .collect();
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. {why}\n\
         Tickets:\n{tickets}\n\
         Devs:\n{devs}\n\
         - Hand out the ready tickets with `mcp__loft__assign_ticket` (ticket and the Dev's name). You may hand several ready tickets to different Devs at once; a blocked ticket is refused. Loft wakes the Dev with its brief, takes each ticket through review and closes it, and wakes you again when a ticket closes.\n\
         - Hand out only what is ready now, and do nothing else on this Delivery: Loft coordinates it.",
        about = subject(d),
        tickets = tickets.join("\n"),
        devs = devs.join("\n"),
    )
}

/// Wakes the team's Lead with the tickets to hand out. The caller holds the repo's publish lock.
fn wake_lead(state: &AppState, d: &Delivery, lead: &crate::db::Agent, why: &str) {
    let key = format!("{}#{}/lead", d.repo, d.spec);
    let step = Step::Assign;
    crate::agents::wake_on_ticket(state, &lead.id, &key, false, lead_brief(state, d, why), wake_for(d, step));
}

/// `assign_ticket`: the Lead hands ready ticket `ticket` to Dev `dev`; Loft wakes the Dev with its brief.
async fn assign(state: &AppState, me: &crate::db::Agent, args: &Value) -> Result<String, String> {
    if me.role != "lead" {
        return Err("Only the team's Lead hands out the tickets of a Delivery.".into());
    }
    // The Delivery of any of the Lead's projects (not only its main one) that has the ticket.
    let want = number_of(&args["ticket"]).ok_or("ticket is required: the ticket's GitHub issue number or its Loft issue id")?;
    let has = |d: &Delivery| d.tickets.iter().any(|t| match &want {
        TicketNumber::GitHub(n) => !d.local && t.number == *n,
        TicketNumber::Loft(id) => d.local && t.id == *id,
    });
    let mut found: Vec<Delivery> = state.db.all_deliveries().map_err(|e| e.to_string())?.into_iter().filter(|d| (me.project.trim() == d.project.trim() || me.has_project(&d.project)) && has(d)).collect();
    if found.len() > 1 {
        return Err("That ticket number is on more than one of your projects' Deliveries: name the ticket by its Loft issue id.".into());
    }
    let found = found.pop().ok_or("That isn't a ticket of a Delivery on your team: only tickets started with Start delivery go through assign_ticket.")?;
    if found.project.trim().is_empty() {
        // From before Deliveries had projects: it'd match any Lead whose main project is empty too.
        return Err("That ticket's Delivery has no project (it started before Deliveries had one), so Loft can't tell whose team it is: restart it with Start delivery.".into());
    }
    let _held = state.publish_locks.of(&found.repo).lock_owned().await;
    let d = state.db.get_delivery(&found.repo, found.spec).map_err(|e| e.to_string())?.ok_or("The Delivery record is gone.")?;
    refuse_paused(&d)?;
    let n = match &want {
        TicketNumber::GitHub(n) => *n,
        TicketNumber::Loft(id) => d.tickets.iter().find(|t| t.id == *id).map(|t| t.number).expect("just found"),
    };
    let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
    let name = tag(&d, t);
    let reassign = args["reassign"].as_bool() == Some(true) && t.state == "started" && !t.agent.is_empty();
    // A stalled Delivery (the Dev's run ended without submitting) can still move its ticket to another Dev.
    if d.state != "running" && !(reassign && d.state == "stalled" && d.resume_to == "running") {
        return Err(format!("{name} can't be assigned: its Delivery isn't running."));
    }
    if !reassign && (t.state != "open" || !t.agent.is_empty()) {
        return Err(format!("{name} can't be assigned: it's {}.", match (t.state.as_str(), t.agent.is_empty()) {
            (_, true) => "not open".to_string(),
            ("review", _) => "under review: reassign it once its round ends".to_string(),
            ("started", _) => "already with a Dev (pass `reassign: true` to move it to another)".to_string(),
            _ => "already with a Dev".to_string(),
        }));
    }
    let open: Vec<String> = t.blocked_by.iter().filter(|b| d.ticket(**b).is_some_and(|b| b.state != "closed")).map(|b| format!("#{b}")).collect();
    if !reassign && !open.is_empty() {
        return Err(format!("{name} is blocked by {}: assign it once they close.", open.join(", ")));
    }
    let key = args["dev"].as_str().unwrap_or("").trim();
    let agents = state.db.list_agents().map_err(|e| e.to_string())?;
    let dev = agents.iter().find(|a| a.id == key || a.name.eq_ignore_ascii_case(key)).filter(|a| is_team_dev(&d, a));
    let dev = dev.ok_or_else(|| format!("{key:?} isn't a Dev on your team."))?;
    if reassign {
        // Unlike a first assignment, this skips the blocked-by check on purpose: the ticket is already started.
        let old = t.agent.clone();
        if old == dev.id {
            return Err(format!("{name} is already with {}.", dev.name));
        }
        let d = update(state, &d.repo, d.spec, |d| {
            d.clear_stall(n);
            // `d.dev` is the Dev of the ticket in `current` (the final fixes and fallbacks read it); a parallel
            // ticket's Dev is only its `agent`, so moving that one must not take `d.dev` from the current ticket.
            if d.current == Some(n) {
                d.dev = dev.id.clone();
            }
            if let Some(x) = d.ticket_mut(n) {
                x.agent = dev.id.clone();
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
        if d.local {
            note(state, &t.id, &dev.id, &format!("Reassigned to {} by the Lead.", dev.name));
        }
        if let Some(old) = agents.iter().find(|a| a.id == old) {
            crate::agents::fyi_from_agent(state, me, old, &format!("{name} is now with {}: the Lead moved it. Don't submit it; your commits stay on the spec branch.", dev.name));
        }
        let step = Step::Build { ticket: n, round: t.rounds };
        let mut brief = dev_brief(state, &d, t);
        let notes = t.notes.trim();
        if t.rounds > 0 && !notes.is_empty() {
            brief.push_str(&format!("\n\nRound {} of changes was asked:\n\n{notes}", t.rounds));
        }
        crate::agents::wake_on_ticket(state, &dev.id, &ticket_key(&d, Some(n)), true, brief, wake_for(&d, step));
        return Ok(format!("{name} moved to {}: Loft woke them with the ticket and told the previous Dev.", dev.name));
    }
    begin(state, &d, n, dev).await?;
    Ok(format!("{name} is with {}: Loft woke them with the ticket.", dev.name))
}

/// [`crate::repos::git`] in `dir`: its trimmed output, or what git said.
async fn git(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    crate::repos::git(dir, args).await.map(|o| o.trim().to_string()).map_err(|e| e.to_string())
}

/// Makes the spec branch of a plan kept in Loft in the project's own repo `dir`, from the branch checked
/// out there, unless it's already there. That branch is what it merges into: returned.
async fn local_branch(dir: &std::path::Path, branch: &str) -> Result<String, String> {
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).await.is_err() {
        return Err(format!("the project folder `{}` has no git repo", dir.display()));
    }
    let base = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await.map_err(|e| format!("couldn't read the branch checked out in `{}`: {e}", dir.display()))?;
    if base == "HEAD" {
        return Err(format!("`{}` has no branch checked out: check out its default branch first", dir.display()));
    }
    if git(dir, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_err() {
        git(dir, &["branch", branch, &base]).await.map_err(|e| format!("couldn't create the spec branch `{branch}`: {e}"))?;
    }
    Ok(base)
}

/// Adds a note to Loft issue `id` as `author`'s.
fn note(state: &AppState, id: &str, author: &str, text: &str) {
    if let Ok(Some(t)) = state.db.get_task(id) {
        crate::tasks::log(state, &t, author, "comment", text, json!({}));
    }
}

/// Hands ticket #n of `d` to `agent`: labels it `in-progress` (in a plan kept in Loft, notes it on its
/// Loft issue), records it as the one being built, and wakes the agent with the Dev brief. The caller
/// holds the repo's publish lock.
async fn begin(state: &AppState, d: &Delivery, n: i64, agent: &crate::db::Agent) -> Result<(), String> {
    if d.local {
        let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
        note(state, &t.id, &agent.id, &format!("Started by Loft's Delivery: with {} on the spec branch `{}`.", agent.name, d.branch));
    } else {
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{}/issues/{n}/labels", d.repo), Some(json!({ "labels": ["in-progress"] })))
            .await
            .map_err(|e| format!("Couldn't label #{n} `in-progress`: {e}"))?;
    }
    let d = update(state, &d.repo, d.spec, |d| {
        d.state = "running".into();
        (d.current, d.dev) = (Some(n), agent.id.clone());
        if let Some(x) = d.ticket_mut(n) {
            (x.state, x.agent) = ("started".into(), agent.id.clone());
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
    let step = Step::Build { ticket: n, round: t.rounds };
    crate::agents::wake_on_ticket(state, &agent.id, &ticket_key(&d, Some(n)), true, dev_brief(state, &d, t), wake_for(&d, step));
    Ok(())
}

/// After a ticket closed: starts the next unblocked one on the team's Dev. A line for the reply.
async fn start_next(state: &AppState, repo: &str, spec: i64) -> String {
    let Some(d) = state.db.get_delivery(repo, spec).ok().flatten() else { return String::new() };
    if let Some(lead) = lead_of(state, &d) {
        let ready: Vec<String> = ready_tickets(&d).iter().map(|t| tag(&d, t)).collect();
        if ready.is_empty() {
            return format!("\nThe {} has no ticket ready to hand out.", subject(&d));
        }
        wake_lead(state, &d, &lead, &format!("Ready to hand out now: {}.", ready.join(", ")));
        return format!("\nReady: {}. {} hands them out.", ready.join(", "), lead.name);
    }
    let Some(t) = next_ticket(&d).cloned() else {
        return format!("\nThe {} has no ticket left to start.", subject(&d));
    };
    let (n, name) = (t.number, tag(&d, &t));
    let Some(agent) = builder(state, &d, &d.dev) else {
        return format!("\nNobody can build {name}: the team has no Dev and the agent that planned the spec is gone.");
    };
    match begin(state, &d, n, &agent).await {
        Ok(()) => format!("\nNext: {name} is with {}.", agent.name),
        Err(e) => {
            tracing::warn!("delivery {repo}#{spec}: {e}");
            format!("\n{name} is next, but {e}: the mirror's next poll starts it.")
        }
    }
}

/// The roles that review each ticket, in the order they are woken.
const TICKET_REVIEWERS: &[&str] = &["qa", "security"];

/// The agent with `role` that works in the Delivery's project (its main project or one of its projects), if any.
fn with_role(state: &AppState, d: &Delivery, role: &str) -> Option<crate::db::Agent> {
    state.db.agent_with_role(&d.project, role).ok()?
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// The QA and Security role briefs, sent with the wake that asks for a ticket's review.
fn review_brief(state: &AppState, d: &Delivery, t: &DeliveryTicket, role: &str) -> String {
    let repo = &d.repo;
    let commits = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
    let pr = d.pr.map(|pr| format!(" ({} https://github.com/{repo}/pull/{pr})", pr_name(d))).unwrap_or_default();
    let place = if d.local { format!(" of the project's own repo `{}`", project_shown(state, d)) } else { String::new() };
    let posted = if d.local { "on the ticket's Loft issue".to_string() } else { format!("on the {}", pr_name(d)) };
    let (who, job) = match role {
        "qa" => ("QA", "Test that they deliver the ticket's acceptance criteria: run the tests and try the behaviour the ticket changes."),
        _ => ("Security", "Check them for security problems an attacker could really use, tracing each input to where it's used. No speculative findings."),
    };
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are {who} on this Delivery: review ticket {ticket}, submitted as {commits} on the {bname} `{branch}`{place}{pr}.\n\
         - Pull the branch and read the ticket and only these commits. {job}\n\
         - Don't fix the work yourself: say what's wrong and the Dev fixes it.\n\
         - {own}\n\
         - When you're done, call `mcp__loft__review_ticket` with {arg}, `verdict` \"pass\" or \"changes\", and `notes`: what you checked and, for changes, each problem and how to see it. Loft posts it {posted}.",
        about = subject(d),
        bname = branch_name(d),
        ticket = ticket_ref(d, t),
        arg = ticket_arg(d, t),
        branch = d.branch,
        own = own_rules(if role == "qa" { "test it" } else { "check its security" }),
    )
}

pub fn mcp_tools() -> Vec<Value> {
    vec![json!({
        "name": "assign_ticket",
        "description": "As the Lead on a Delivery: hand a ready ticket to a Dev of the team. Loft sets the ticket's agent and wakes the Dev with its brief. Hand several ready tickets to different Devs at once; a ticket still blocked by an open ticket is refused.",
        "inputSchema": { "type": "object", "properties": {
            "ticket": { "type": ["integer", "string"], "description": "The ticket's GitHub issue number, or its Loft issue id in a plan kept in Loft." },
            "dev": { "type": "string", "description": "The Dev's name." },
            "reassign": { "type": "boolean", "description": "true to move a ticket that is already started (or back with its Dev for changes) to another Dev, keeping its commits and rounds. Refused while it's under review." }
        }, "required": ["ticket", "dev"] }
    }), json!({
        "name": "submit_ticket",
        "description": "As the Dev on a Delivery: hand in the ticket Loft gave you once its commits are pushed to the spec branch and the checks pass. Loft wakes the team's QA and Security to review them, and closes the ticket and starts the next one when they pass. On changes, Loft wakes you with their notes: fix with new commits and submit again. Fixes asked for in the final Code Review are submitted with the spec PR's number.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": ["integer", "string"], "description": "The ticket's GitHub issue number, or its Loft issue id in a plan kept in Loft; for final Code Review fixes, the spec PR's number, or the spec's Loft issue id." },
            "commits": { "type": "array", "items": { "type": "string" }, "description": "The SHAs of all the ticket's commits, oldest first." }
        }, "required": ["number", "commits"] }
    }), json!({
        "name": "review_ticket",
        "description": "As QA or Security on a Delivery: give your verdict on the ticket Loft asked you to review. As the Code reviewer: give your verdict on the whole spec PR, once every ticket is closed. Loft posts it on the spec PR as a comment with your notes and, for a ticket, a `qa-passed`, `security:passed` or `changes-requested` label. On changes it wakes the Dev with your notes. When every reviewer passed a ticket, Loft closes it; when the Code reviewer passes the spec PR, Loft marks it ready to merge.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": ["integer", "string"], "description": "The ticket's GitHub issue number, or its Loft issue id in a plan kept in Loft; for the Code reviewer, the spec PR's number, or the spec's Loft issue id." },
            "verdict": { "type": "string", "enum": ["pass", "changes"], "description": "\"changes\" for any finding; \"pass\" only when nothing is left to fix." },
            "notes": { "type": "string", "description": "What you checked; for changes, each problem and how to see it." }
        }, "required": ["number", "verdict", "notes"] }
    })]
}

/// Handle the Delivery tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &crate::db::Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    match name {
        "assign_ticket" => Some(assign(state, me, args).await),
        "submit_ticket" => Some(submit(state, me, args).await),
        "review_ticket" => Some(review(state, me, args).await),
        _ => None,
    }
}

/// What a Delivery tool's `number` names: a GitHub issue number, given as an integer or as a string of
/// digits (`"310"`, `"#310"`), or the Loft issue id of a ticket in a plan kept in Loft.
#[derive(Debug, PartialEq)]
enum TicketNumber {
    GitHub(i64),
    Loft(String),
}

/// None when `number` is missing, empty or not a positive issue number.
fn number_of(number: &Value) -> Option<TicketNumber> {
    if let Some(n) = number.as_i64() {
        return (n > 0).then_some(TicketNumber::GitHub(n));
    }
    let s = number.as_str()?.trim();
    let digits = s.strip_prefix('#').map(str::trim);
    if digits.is_some() || s.bytes().all(|b| b.is_ascii_digit()) {
        return digits.unwrap_or(s).parse().ok().filter(|n| *n > 0).map(TicketNumber::GitHub);
    }
    Some(TicketNumber::Loft(s.to_string()))
}

/// The repo, number and Delivery of the ticket a Delivery tool names, with the repo's publish lock held.
async fn ticket_of(state: &AppState, me: &crate::db::Agent, args: &Value, tool: &str) -> Result<(String, i64, Delivery, tokio::sync::OwnedMutexGuard<()>), String> {
    let n = match number_of(&args["number"]).ok_or("number is required: the ticket's GitHub issue number (85), or a Loft issue id in a plan kept in Loft")? {
        TicketNumber::GitHub(n) => n,
        TicketNumber::Loft(id) => {
            // A plan kept in Loft names its tickets by their Loft issue ids.
            let slug = local_repo(&me.project);
            let held = state.publish_locks.of(&slug).lock_owned().await;
            let d = state.db.deliveries(&slug, false).map_err(|e| e.to_string())?.into_iter().find(|d| d.tickets.iter().any(|t| t.id == id));
            let d = d.ok_or_else(|| format!("Loft issue {id} isn't a ticket of a Delivery in project {} (read as a Loft issue id: a GitHub issue number is only digits): only tickets started with Start delivery go through {tool}. {}", me.project.trim(), in_progress(state, &slug)))?;
            let n = d.tickets.iter().find(|t| t.id == id).map(|t| t.number).expect("just found");
            return Ok((slug, n, d, held));
        }
    };
    let slug = crate::planning::github_home(state, me).await.map_err(|why| format!("{tool} works on tickets on GitHub, and {why}."))?;
    let held = state.publish_locks.of(&slug).lock_owned().await;
    let d = of_ticket(state, &slug, n).ok_or_else(|| format!("#{n} on {slug} isn't a ticket of a Delivery (read as a GitHub issue number): only tickets started with Start delivery go through {tool}. {}", in_progress(state, &slug)))?;
    Ok((slug, n, d, held))
}

/// A paused Delivery takes no call: the user paused it, so it moves on, and wakes anyone, only once they
/// resume it. Resume then reads what was done meanwhile (commits on the branch) and wakes who it waits on.
fn refuse_paused(d: &Delivery) -> Result<(), String> {
    if d.state != "paused" {
        return Ok(());
    }
    Err(format!("The Delivery of {} is paused: the user paused it, so Loft took nothing in. Stop here: when they resume it, Loft wakes the agent its step waits on with what's already done.", subject(d)))
}

/// The PR Loft tracks for the Delivery, if it has one on GitHub.
fn tracked_pr(state: &AppState, d: &Delivery) -> Option<crate::db::PullRequest> {
    let n = d.pr.filter(|_| !d.local)?;
    state.db.list_pull_requests(None).ok()?.into_iter().find(|p| p.slug == d.repo && p.number == n)
}

/// Refuses a submission whose branch is behind the base of its PR, by GitHub's compare (Loft runs no git in
/// agents' clones), and keeps the count on the PR for its card and the Delivery's. When Loft can't tell (no
/// PR, or GitHub didn't answer) it lets the submission through.
async fn refuse_behind(state: &AppState, d: &Delivery) -> Result<(), String> {
    let Some(mut pr) = tracked_pr(state, d).filter(|p| !p.base_ref.is_empty() && !d.branch.is_empty()) else { return Ok(()) };
    let v = match crate::github::get(state, &format!("/repos/{}/compare/{}...{}", d.repo, pr.base_ref, d.branch)).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("delivery {}#{}: couldn't compare `{}` with `{}`: {e}", d.repo, d.spec, d.branch, pr.base_ref);
            return Ok(());
        }
    };
    let behind = v["behind_by"].as_i64().unwrap_or(0);
    if behind != pr.behind_by {
        pr.behind_by = behind;
        if state.db.save_pull_request(&pr).is_ok() {
            crate::prs::emit(state, &pr);
            changed(state, d);
        }
    }
    if behind == 0 {
        return Ok(());
    }
    let base = &pr.base_ref;
    Err(format!(
        "`{}` is {behind} commit{} behind origin/{base}, so Loft took nothing in: `git fetch origin`, `git merge origin/{base}` into it (merge, not rebase), resolve any conflicts, re-run the tests, push, then submit again.",
        d.branch,
        if behind == 1 { "" } else { "s" },
    ))
}

/// The tickets being built or in review on `slug`, for a refusal.
fn in_progress(state: &AppState, slug: &str) -> String {
    let ds = state.db.deliveries(slug, true).unwrap_or_default();
    let names: Vec<String> = ds.iter().flat_map(|d| d.tickets.iter().filter(|t| ["started", "review"].contains(&t.state.as_str())).map(move |t| tag(d, t))).collect();
    if names.is_empty() { "No ticket is being built there now.".into() } else { format!("Being built there now: {}.", names.join(", ")) }
}

/// `submit_ticket`: the Dev hands in its commits, and Loft wakes the team's reviewers (or closes the ticket
/// when the team has none).
async fn submit(state: &AppState, me: &crate::db::Agent, args: &Value) -> Result<String, String> {
    let commits = crate::planning::commits_in(args)?;
    if let Some(out) = final_submit(state, me, args, &commits).await {
        return out;
    }
    let (slug, n, d, _held) = ticket_of(state, me, args, "submit_ticket").await?;
    refuse_paused(&d)?;
    let t = d.ticket(n).expect("of_ticket found it");
    let name = tag(&d, t);
    if t.state != "started" {
        let why = match t.state.as_str() {
            "review" => "it's already submitted and waiting for its reviewers",
            "closed" | "closing" => "it has passed review already",
            "needs_human" => "it's waiting for the user after too many changes rounds",
            _ => "it isn't the ticket being built",
        };
        return Err(format!("{name} can't be submitted: {why}."));
    }
    if builder_id(&d, t) != me.id {
        let who = state.db.get_agent(builder_id(&d, t)).ok().flatten().map_or("another agent".to_string(), |a| a.name);
        return Err(format!("{name} is with {who}: only the agent building it submits it."));
    }
    refuse_behind(state, &d).await?;
    let reviewers: Vec<(&str, crate::db::Agent)> = TICKET_REVIEWERS.iter().filter_map(|r| with_role(state, &d, r).map(|a| (*r, a))).collect();
    let d = unstalled(state, d, n);
    let d = update(state, &slug, d.spec, |d| {
        if let Some(t) = d.ticket_mut(n) {
            t.state = "review".into();
            t.commits = commits.clone();
            t.verdicts = reviewers.iter().map(|(r, _)| (r.to_string(), "pending".to_string())).collect();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    if reviewers.is_empty() {
        let closed = finish(state, &d, n).await?;
        return Ok(format!("Submitted {name}. The team has no QA or Security, so it passed as submitted.{closed}"));
    }
    let t = d.ticket(n).expect("just saved");
    for (role, a) in &reviewers {
        let step = Step::Review { ticket: n, role: role.to_string(), round: t.rounds };
        crate::agents::wake_on_ticket(state, &a.id, &ticket_key(&d, Some(n)), false, review_brief(state, &d, t, role), wake_for(&d, step));
    }
    let names = reviewers.iter().map(|(r, a)| format!("{} ({})", a.name, crate::planning::role_label(r))).collect::<Vec<_>>().join(" and ");
    Ok(format!("Submitted {name}: {names} are reviewing it. Loft wakes you if they ask for changes, and closes it when they pass."))
}

/// Changes rounds after which a ticket waits for the user instead of going back to the Dev.
const MAX_ROUNDS: u32 = 3;

/// `review_ticket`: a reviewer's verdict, posted on the spec PR. Changes go back to the Dev (or, on the
/// third round, to the user); the last pass closes the ticket.
async fn review(state: &AppState, me: &crate::db::Agent, args: &Value) -> Result<String, String> {
    let verdict = args["verdict"].as_str().filter(|v| ["pass", "changes"].contains(v)).ok_or("verdict must be \"pass\" or \"changes\"")?;
    let notes = args["notes"].as_str().map(str::trim).unwrap_or("");
    if verdict == "changes" && notes.is_empty() {
        return Err("notes are required with changes: say what's wrong and how to see it, for the Dev.".into());
    }
    if args["tech_debt"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("tech_debt is gone: put every finding in `notes`, each with its file, line and fix, with verdict \"changes\". The Dev fixes them on the same PR.".into());
    }
    if me.role == "code_reviewer" {
        return final_review(state, me, args, verdict, notes).await;
    }
    let (slug, n, d, _held) = ticket_of(state, me, args, "review_ticket").await?;
    refuse_paused(&d)?;
    let role = me.role.as_str();
    let t = d.ticket(n).expect("of_ticket found it").clone();
    let name = tag(&d, &t);
    if t.state != "review" {
        return Err(format!("{name} isn't waiting for a review: nothing was posted."));
    }
    match t.verdicts.get(role).map(String::as_str) {
        Some("pending") => {}
        Some(_) => return Err(format!("You already passed {name} in this round: nothing was posted.")),
        None => return Err(format!("{name} isn't waiting for a {} review: QA and Security review tickets.", crate::planning::role_label(role))),
    }
    let label = crate::planning::role_label(role);
    let sha = t.commits.last().map(|c| short(c).to_string()).unwrap_or_default();
    let (head, label_tag) = match (verdict, role) {
        ("pass", "qa") => (format!("**{label}** ({}) passed ticket {name} at `{sha}`.", me.name), "qa-passed"),
        ("pass", _) => (format!("**{label}** ({}) passed ticket {name} at `{sha}`.", me.name), "security:passed"),
        _ => (format!("**{label}** ({}) asks for changes on ticket {name} at `{sha}`.", me.name), "changes-requested"),
    };
    let body = if notes.is_empty() { head } else { format!("{head}\n\n{notes}") };
    let (on, labelled) = if d.local {
        // A plan kept in Loft: the verdict is a note on the ticket's Loft issue.
        note(state, &t.id, &me.id, &body);
        ("its Loft issue".to_string(), String::new())
    } else {
        let on = d.pr.unwrap_or(n);
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{on}/comments"), Some(json!({ "body": body })))
            .await
            .map_err(|e| format!("Couldn't post the verdict on #{on}: {e}. Call review_ticket again."))?;
        let labelled = label_issue(state, &slug, on, label_tag).await.err().map(|e| format!(" The comment is posted, but {e}.")).unwrap_or_default();
        (format!("#{on}"), labelled)
    };

    if verdict == "pass" {
        let d = update(state, &slug, d.spec, |d| {
            if let Some(t) = d.ticket_mut(n) {
                t.verdicts.insert(role.into(), "pass".into());
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let t = d.ticket(n).expect("just saved");
        if t.verdicts.values().any(|v| v != "pass") {
            // A stalled Delivery stays stalled: the verdict it waits on may be the other reviewer's.
            return Ok(format!("Posted your pass on {on}.{labelled} {name} waits for its other reviewers."));
        }
        // The last verdict: the review step is done, so a stalled Delivery carries on.
        let d = unstalled(state, d, n);
        let closed = finish(state, &d, n).await?;
        return Ok(format!("Posted your pass on {on}.{labelled}{closed}"));
    }

    // Changes end the review step for every reviewer: the ticket goes back to the Dev (or the user).
    let d = unstalled(state, d, n);
    let rounds = t.rounds + 1;
    let stuck = rounds >= MAX_ROUNDS;
    update(state, &slug, d.spec, |d| {
        if let Some(t) = d.ticket_mut(n) {
            t.rounds = rounds;
            t.notes = notes.into();
            t.verdicts.clear();
            t.state = if stuck { "needs_human" } else { "started" }.into();
        }
        d.last_notes = notes.into();
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    if stuck {
        let why = needs_human(state, &d, Some(&t), rounds, notes).await;
        return Ok(format!("Posted your changes on {on}.{labelled} That's round {rounds} of changes on {name}, so it waits for the user now{why}."));
    }
    let brief = format!(
        "[Loft Delivery: {about}] This is not a message from the user. {who} ({label}) asked for changes on ticket {ticket} (round {rounds} of {MAX_ROUNDS}):\n\n{notes}\n\n\
         Fix it on the {bname} `{branch}` with new commits, each ending with `{refs}`, push, then call `mcp__loft__submit_ticket` again with {arg} and all the ticket's commits, oldest first.\n{own}",
        own = own_rules("build and check the work"),
        about = subject(&d),
        bname = branch_name(&d),
        who = me.name,
        ticket = if d.local { ticket_ref(&d, &t) } else { format!("#{n} \"{}\"", t.title) },
        refs = refs(&d, &t),
        arg = ticket_arg(&d, &t),
        branch = d.branch,
    );
    crate::agents::wake_on_ticket(state, builder_id(&d, &t), &ticket_key(&d, Some(n)), true, brief, wake_for(&d, Step::Build { ticket: n, round: rounds }));
    Ok(format!("Posted your changes on {on}.{labelled} The Dev is fixing {name} (round {rounds} of {MAX_ROUNDS})."))
}

/// Adds label `name` to issue or PR #n, creating it on the repo first if it's missing.
async fn label_issue(state: &AppState, slug: &str, n: i64, name: &str) -> Result<(), String> {
    let (color, about) = match name {
        "qa-passed" => ("0e8a16", "QA passed a ticket"),
        "security:passed" => ("1d76db", "Security passed a ticket"),
        "changes-requested" => ("d93f0b", "A reviewer asked for changes"),
        _ => ("b60205", "Stuck: needs the user"),
    };
    crate::planning::ensure_label(state, slug, name, color, about).await?;
    crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{n}/labels"), Some(json!({ "labels": [name] })))
        .await
        .map(|_| ())
        .map_err(|e| format!("couldn't label #{n} `{name}`: {e}"))
}

/// After the last changes round on ticket `t` (or, with None, on the final Code Review): the ticket (or the
/// spec PR) gets `needs-human`, and the ticket's (or the spec's) Loft issue is blocked, which puts it in
/// Needs you and sends a push. Empty, or a note when GitHub refused the label.
async fn needs_human(state: &AppState, d: &Delivery, t: Option<&DeliveryTicket>, rounds: u32, notes: &str) -> String {
    let what = match t {
        Some(t) => tag(d, t),
        None => final_tag(d),
    };
    let blocked = |mut x: crate::db::Task| {
        x.error = format!("{rounds} rounds of changes on {what}: it needs you. Last notes: {}", notes.lines().next().unwrap_or(""));
        crate::tasks::set_status(state, &mut x, "blocked", "loft", None);
    };
    if d.local {
        let id = t.map_or(d.spec_id.as_str(), |t| t.id.as_str());
        if let Ok(Some(x)) = state.db.get_task(id) {
            blocked(x);
        }
        return String::new();
    }
    let (labelled, issue) = match t {
        Some(t) => (t.number, t.number),
        None => (d.pr.unwrap_or(d.spec), d.spec),
    };
    let labelled = label_issue(state, &d.repo, labelled, "needs-human").await.err().map(|e| format!(" (and {e})")).unwrap_or_default();
    let mut task = state.db.list_tasks().unwrap_or_default().into_iter().find(|x| x.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(&d.repo) && g.number == issue));
    if task.is_none() {
        // Not mirrored yet: bring it in now, so it reaches Needs you without waiting for the next poll.
        if let Ok(i) = crate::github::get(state, &format!("/repos/{}/issues/{issue}", d.repo)).await {
            task = crate::mirror::bring_in(state, d.project.trim(), &d.repo, &i).map(|(x, _)| x);
        }
    }
    if let Some(x) = task {
        blocked(x);
    }
    labelled
}

/// Every reviewer passed ticket #n: closes it on GitHub and starts the next one. A failed close is kept as
/// "closing" and retried on the mirror's next poll. The lines for the reply.
async fn finish(state: &AppState, d: &Delivery, n: i64) -> Result<String, String> {
    let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
    if d.local {
        // A plan kept in Loft: its Loft issue is marked done, with the commits.
        let by = state.db.get_agent(builder_id(d, t)).ok().flatten();
        if let Ok(Some(mut x)) = state.db.get_task(&t.id) {
            let shas = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
            let author = by.as_ref().map_or("loft", |a| a.id.as_str());
            crate::tasks::log(state, &x, author, "comment", &format!("Done in {shas}, on the spec branch `{}`.", d.branch), json!({}));
            crate::tasks::set_status(state, &mut x, "done", author, Some("Closed by Loft: its reviewers passed it"));
        }
        let d = update(state, &d.repo, d.spec, |d| {
            if let Some(t) = d.ticket_mut(n) {
                t.state = "closed".into();
            }
            if d.current == Some(n) {
                d.current = None;
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let after = crate::planning::after_tickets(state, &d).await.unwrap_or_default().unwrap_or_default();
        let next = start_next(state, &d.repo, d.spec).await;
        return Ok(format!("\n{} is done.{after}{next}", tag(&d, t)));
    }
    if d.standalone {
        // Its PR closes it when it merges: Loft doesn't close it now.
        let d = update(state, &d.repo, d.spec, |d| {
            if let Some(t) = d.ticket_mut(n) {
                t.state = "closed".into();
            }
            if d.current == Some(n) {
                d.current = None;
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        if let Some(pr) = d.pr {
            let _ = crate::github::request(state, reqwest::Method::DELETE, &format!("/repos/{}/issues/{pr}/labels/changes-requested", d.repo), None).await;
        }
        let after = crate::planning::after_tickets(state, &d).await.unwrap_or_else(|e| Some(format!("\nBut {e}: Loft tries again on its next poll.")));
        let pr = d.pr.map(|pr| format!(" PR #{pr}")).unwrap_or_default();
        return Ok(format!("\n#{n} passed its reviews. It stays open until its{pr} merges.{}", after.unwrap_or_default()));
    }
    let by = state.db.get_agent(builder_id(d, t)).ok().flatten().ok_or("The agent that built the ticket is gone.")?;
    update(state, &d.repo, d.spec, |d| {
        if let Some(t) = d.ticket_mut(n) {
            t.state = "closing".into();
        }
    });
    let closed = match crate::planning::close_ticket(state, &d.project, &by, &d.repo, n, &t.commits).await {
        Ok(line) => line,
        // Closed by an earlier try whose answer was lost, or by hand.
        Err(e) if e.contains("is already closed") => {
            ticket_closed(state, &d.repo, n);
            format!("#{n} was already closed.")
        }
        Err(e) => return Ok(format!("\n#{n} passed, but {e}")),
    };
    if let Some(pr) = d.pr {
        // A pass after a changes round: the PR no longer waits for changes on this ticket.
        let _ = crate::github::request(state, reqwest::Method::DELETE, &format!("/repos/{}/issues/{pr}/labels/changes-requested", d.repo), None).await;
    }
    let next = start_next(state, &d.repo, d.spec).await;
    Ok(format!("\n{closed}{next}"))
}

/// The mirror's catch-up: closes the tickets whose close failed after they passed, then starts the next
/// ticket of a running Delivery that has none (its start failed).
pub(crate) async fn retry(state: &AppState, repo: &str) {
    let _held = state.publish_locks.of(repo).lock_owned().await;
    for d in state.db.deliveries_in(repo, "running").unwrap_or_default() {
        for t in d.tickets.iter().filter(|t| t.state == "closing") {
            match finish(state, &d, t.number).await {
                Ok(line) => tracing::info!("delivery {repo}#{}: {}", d.spec, line.trim()),
                Err(e) => tracing::warn!("delivery {repo}#{}: {e}", d.spec),
            }
        }
        let d = state.db.get_delivery(repo, d.spec).ok().flatten().unwrap_or(d);
        let busy = d.tickets.iter().any(|t| matches!(t.state.as_str(), "started" | "review" | "closing" | "needs_human"));
        if d.current.is_none() && !busy && next_ticket(&d).is_some() && lead_of(state, &d).is_none() {
            let line = start_next(state, repo, d.spec).await;
            tracing::info!("delivery {repo}#{}: {}", d.spec, line.trim());
        }
    }
}

/// Why a Delivery that isn't stalled or paused can't be resumed.
fn not_resumable(d: &Delivery) -> String {
    let what = subject(d);
    match d.state.as_str() {
        "planned" => format!("{what} hasn't started: Start delivery starts it."),
        "running" | "final_review" | "final_fixes" => format!("{what} isn't stalled or paused: an agent is working on its step, so there's nothing to resume."),
        _ => format!("{what} is done: every review passed, so there's nothing to resume."),
    }
}

/// Resume's first look at what was done while the Delivery stalled: its tickets closed by hand (on GitHub,
/// or Loft issues done or cancelled in a plan kept in Loft) are marked closed. The Delivery as it is now,
/// and those tickets. The caller holds the repo's publish lock.
async fn reconcile(state: &AppState, d: &Delivery) -> Result<(Delivery, Vec<i64>), String> {
    let open: Vec<i64> = d.tickets.iter().filter(|t| t.state != "closed").map(|t| t.number).collect();
    let mut closed = vec![];
    if open.is_empty() {
        return Ok((d.clone(), closed));
    }
    if d.local {
        for t in d.tickets.iter().filter(|t| open.contains(&t.number)) {
            if let Ok(Some(x)) = state.db.get_task(&t.id)
                && matches!(x.status.as_str(), "done" | "cancelled")
            {
                closed.push(t.number);
            }
        }
    } else if d.standalone {
        let i = crate::github::get(state, &format!("/repos/{}/issues/{}", d.repo, d.spec)).await.map_err(|e| format!("couldn't read #{} on GitHub: {e}", d.spec))?;
        if i["state"] == "closed" {
            closed.push(d.spec);
        }
    } else {
        let subs = crate::github::get(state, &format!("/repos/{}/issues/{}/sub_issues?per_page=100", d.repo, d.spec)).await.map_err(|e| format!("couldn't read spec #{}'s tickets on GitHub: {e}", d.spec))?;
        closed = subs.as_array().into_iter().flatten().filter(|x| x["state"] == "closed").filter_map(|x| x["number"].as_i64()).filter(|n| open.contains(n)).collect();
    }
    if closed.is_empty() {
        return Ok((d.clone(), closed));
    }
    let d = update(state, &d.repo, d.spec, |d| {
        for n in &closed {
            if let Some(t) = d.ticket_mut(*n) {
                t.state = "closed".into();
            }
        }
        if d.current.is_some_and(|n| closed.contains(&n)) {
            d.current = None;
        }
    })
    .ok_or("the Delivery record couldn't be saved")?;
    Ok((d, closed))
}

/// Whether a commit message carries `trailer` (`Refs #41`, not `Refs #410`).
fn has_trailer(message: &str, trailer: &str) -> bool {
    message.match_indices(trailer).any(|(i, _)| !message[i + trailer.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// The commits on the Delivery's branch whose message carries `trailer`, pushed after the newest of `known`
/// (the last submit's commits): "`abc1234` Subject" each, oldest first. Read on GitHub, or for a plan kept
/// in Loft in the project's own repo.
async fn branch_commits(state: &AppState, d: &Delivery, trailer: &str, known: &[String]) -> Result<Vec<String>, String> {
    let list: Vec<(String, String)> = if d.local {
        let dir = project_dir(state, d)?;
        let log = git(&dir, &["log", "-n", "200", "--format=%H%x1f%B%x1e", &format!("refs/heads/{}", d.branch)]).await?;
        log.split('\x1e').filter_map(|c| c.trim().split_once('\x1f').map(|(sha, m)| (sha.to_string(), m.to_string()))).collect()
    } else {
        let v = crate::github::get(state, &format!("/repos/{}/commits?sha={}&per_page=100", d.repo, d.branch)).await.map_err(|e| e.to_string())?;
        v.as_array().into_iter().flatten().filter_map(|c| Some((c["sha"].as_str()?.to_string(), c["commit"]["message"].as_str().unwrap_or("").to_string()))).collect()
    };
    let known = |sha: &str| {
        let sha = sha.to_lowercase();
        known.iter().any(|k| !k.is_empty() && (sha.starts_with(k.as_str()) || k.starts_with(&sha)))
    };
    // Newest first: back to the last submit.
    let mut found: Vec<String> = list.iter().take_while(|(sha, _)| !known(sha)).filter(|(_, m)| has_trailer(m, trailer)).map(|(sha, m)| format!("`{}` {}", short(sha), m.lines().next().unwrap_or("").trim())).collect();
    found.reverse();
    Ok(found)
}

/// The "this is a resume" section a Resume wake adds to the role brief: why the step stopped (`stopped`, a
/// clause), and what's already done.
fn resume_section(stopped: &str, done: &[String]) -> String {
    let done = done.iter().map(|l| format!("\n- {l}")).collect::<String>();
    format!("\n\nThis is a resume: {stopped}, and the user pressed Resume. Pick the work up where it stopped instead of starting over.{done}")
}

/// What a Resume wake to the Dev on ticket `t` says is done: its commits found on the branch, and its
/// changes rounds.
async fn build_done(state: &AppState, d: &Delivery, t: &DeliveryTicket) -> Vec<String> {
    let mut done = vec![];
    if t.rounds > 0 {
        let at = if d.local { "its Loft issue".to_string() } else { format!("the {}", pr_name(d)) };
        let last = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
        done.push(format!("Its reviewers asked for changes {} time(s); their notes are on {at}. You last submitted {last}.", t.rounds));
    }
    let since = if t.commits.is_empty() { "" } else { " since then" };
    done.push(match branch_commits(state, d, &refs(d, t), &t.commits).await {
        Ok(c) if c.is_empty() => format!("No commit with `{}` is on the {} `{}`{since} yet.", refs(d, t), branch_name(d), d.branch),
        Ok(c) => format!("Already on the {} `{}`{since}, with `{}`: {}. Check them, finish what's missing, then submit all the ticket's commits.", branch_name(d), d.branch, refs(d, t), c.join(", ")),
        Err(e) => format!("Loft couldn't read the {} ({e}): check what's already on it yourself.", branch_name(d)),
    });
    done
}

/// The Dev brief for the final Code Review's fixes, for a Resume: the Code reviewer's notes are on the spec
/// PR (or the spec's Loft issue), since Loft doesn't keep them.
fn fixes_brief(d: &Delivery) -> String {
    // None yet after a Resume on a final review that waited for the user: its rounds start over.
    let rounds = if d.final_rounds > 0 { format!(" (round {} of {MAX_ROUNDS})", d.final_rounds) } else { String::new() };
    if d.local {
        return format!(
            "[Loft Delivery: {about}] This is not a message from the user. You are the Dev on this Delivery: fix what the Code reviewer asked for on the spec branch `{branch}`{rounds}. Its notes are its last note on the spec's Loft issue {spec_id}.\n\
             Fix them on that branch with new commits, each ending with `Refs {spec_id}`, push them to `origin`, then call `mcp__loft__submit_ticket` with \"{spec_id}\" and the new commits, oldest first.\n{own}",
            about = subject(d),
            branch = d.branch,
            spec_id = d.spec_id,
            own = own_rules("build and check the work"),
        );
    }
    let pr = d.pr.unwrap_or_default();
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are the Dev on this Delivery: fix what the Code reviewer asked for on the {pname} https://github.com/{repo}/pull/{pr}{rounds}. Its notes are its last comment there.\n\
         Fix them on the {bname} `{branch}` with new commits, each ending with `Refs #{spec}`, push, then call `mcp__loft__submit_ticket` with {pr} and the new commits, oldest first.\n{own}",
        about = subject(d),
        pname = pr_name(d),
        repo = d.repo,
        bname = branch_name(d),
        branch = d.branch,
        spec = d.spec,
        own = own_rules("build and check the work"),
    )
}

/// The agent building the Delivery's tickets: its Dev, or whoever builds them now when it's gone.
fn dev_of(state: &AppState, d: &Delivery, agent: &str) -> Result<crate::db::Agent, String> {
    state.db.get_agent(agent).ok().flatten().or_else(|| builder(state, d, &d.dev)).ok_or_else(|| "Nothing was resumed: the team has no Dev and the agent that planned the spec is gone.".to_string())
}

/// Whether a ticket of `d`, or its final review, waits for the user after too many changes rounds: Resume
/// gives it a fresh set of rounds.
pub(crate) fn waits_for_user(d: &Delivery) -> bool {
    d.state == "needs_human" || (d.state == "running" && d.tickets.iter().any(|t| t.state == "needs_human"))
}

/// Whether the Loft issue `t` is the one blocked by `d`'s ticket (or final review) waiting for the user: its
/// Needs you item offers Resume.
pub(crate) fn blocked_by_rounds(d: &Delivery, t: &crate::db::Task) -> bool {
    if !waits_for_user(d) {
        return false;
    }
    let ticket = d.tickets.iter().find(|x| x.state == "needs_human");
    if d.local {
        return t.id == if d.state == "needs_human" { d.spec_id.as_str() } else { ticket.map_or("", |x| x.id.as_str()) };
    }
    let n = if d.state == "needs_human" { Some(d.spec) } else { ticket.map(|x| x.number) };
    t.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(&d.repo) && Some(g.number) == n)
}

/// Takes label `name` off issue or PR #n; one that's gone already is fine.
async fn unlabel_issue(state: &AppState, slug: &str, n: i64, name: &str) -> Result<(), String> {
    match crate::github::request(state, reqwest::Method::DELETE, &format!("/repos/{slug}/issues/{n}/labels/{name}"), None).await {
        Err(e) if !crate::github::is_gone(&e) => Err(format!("couldn't take `{name}` off #{n}: {e}")),
        _ => Ok(()),
    }
}

/// The ticket's (with None the spec's, or the standalone ticket's) Loft issue goes back to in progress, if
/// `needs_human` blocked it.
fn unblock(state: &AppState, d: &Delivery, t: Option<&DeliveryTicket>) {
    let x = if d.local {
        state.db.get_task(t.map_or(d.spec_id.as_str(), |t| t.id.as_str())).ok().flatten()
    } else {
        let n = t.map_or(d.spec, |t| t.number);
        state.db.list_tasks().unwrap_or_default().into_iter().find(|x| x.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(&d.repo) && g.number == n))
    };
    if let Some(mut x) = x.filter(|x| x.status == "blocked") {
        x.error.clear();
        crate::tasks::set_status(state, &mut x, "in_progress", "user", Some(&format!("Resumed by you: a fresh {MAX_ROUNDS} rounds of changes")));
    }
}

/// Resume on a ticket or final review waiting for the user after too many changes rounds, once they looked
/// at it: its rounds start over, `needs-human` comes off the ticket (or the spec PR), its Loft issue is
/// unblocked, and the Dev is woken with the last changes notes. The ticket is built again (the Delivery goes
/// back to its final fixes), and the Delivery carries on as usual. A ticket closed by hand meanwhile is
/// marked closed, and the next one starts. The line saying what it did.
async fn resume_needs_human(state: &AppState, d: Delivery) -> Result<String, String> {
    let (d, closed) = reconcile(state, &d).await.map_err(|e| format!("Nothing was resumed: {e}."))?;
    let (repo, spec) = (d.repo.clone(), d.spec);
    // The ticket waiting for the user, or with None the final review.
    let waiting = if d.state == "needs_human" { Some(None) } else { d.tickets.iter().find(|t| t.state == "needs_human").cloned().map(Some) };
    let Some(t) = waiting else {
        // Its ticket was closed by hand meanwhile: on to the next one.
        let names = closed.iter().filter_map(|n| d.ticket(*n)).map(|t| tag(&d, t)).collect::<Vec<_>>().join(", ");
        let line = if !d.tickets.is_empty() && d.tickets.iter().all(|t| t.state == "closed") {
            crate::planning::after_tickets(state, &d).await.unwrap_or_else(|e| Some(format!("\nBut {e}: Loft tries again on its next poll."))).unwrap_or_default()
        } else {
            start_next(state, &repo, spec).await
        };
        return Ok(format!("Resumed {}. Closed by hand meanwhile, so marked closed: {names}.{}", subject(&d), line.replace('\n', " ").trim_end()));
    };
    let dev = dev_of(state, &d, t.as_ref().map_or(&d.dev, |t| builder_id(&d, t)))?;
    let what = t.as_ref().map_or_else(|| final_tag(&d), |t| tag(&d, t));
    let rounds = t.as_ref().map_or(d.final_rounds, |t| t.rounds);
    // A ticket resumes with its own notes; `last_notes` is the Delivery's latest, whichever ticket it was on.
    let notes = t.as_ref().map_or(&d.last_notes, |t| &t.notes).trim().to_string();
    let saved = update(state, &repo, spec, |d| {
        match &t {
            Some(t) => {
                if let Some(t) = d.ticket_mut(t.number) {
                    (t.rounds, t.state) = (0, "started".into());
                    t.verdicts.clear();
                }
                d.current = Some(t.number);
            }
            None => (d.final_rounds, d.state) = (0, "final_fixes".into()),
        }
        d.dev = dev.id.clone();
    })
    .ok_or("Nothing was resumed: the Delivery record couldn't be saved.")?;
    let unlabelled = if d.local {
        String::new()
    } else {
        let on = t.as_ref().map_or(d.pr.unwrap_or(d.spec), |t| t.number);
        unlabel_issue(state, &repo, on, "needs-human").await.err().map(|e| format!(" But Loft {e}.")).unwrap_or_default()
    };
    unblock(state, &d, t.as_ref());

    // The brief: the role's, the resume section, and the notes that asked for the last changes.
    let at = match (&t, d.local) {
        (Some(t), true) => format!("its Loft issue {}", t.id),
        (None, true) => format!("the spec's Loft issue {}", d.spec_id),
        _ => format!("the {} https://github.com/{repo}/pull/{}", pr_name(&d), d.pr.unwrap_or_default()),
    };
    let stopped = format!("{what} waited for the user after {rounds} rounds of changes; they looked at it, so it gets a fresh {MAX_ROUNDS} rounds");
    let notes = if notes.is_empty() { format!("\n\nLoft doesn't have the last notes asking for changes: read them on {at}.") } else { format!("\n\nThe last notes asking for changes, also on {at}:\n\n{notes}") };
    let line = match &t {
        Some(t) => {
            let done = build_done(state, &d, t).await;
            let t = saved.ticket(t.number).expect("just saved");
            let brief = format!("{}{}{notes}", dev_brief(state, &saved, t), resume_section(&stopped, &done));
            crate::agents::resume_on_ticket(state, &dev.id, &ticket_key(&saved, Some(t.number)), brief, wake_for(&saved, Step::Build { ticket: t.number, round: 0 }));
            format!(" {} is fixing {what} again, with a fresh {MAX_ROUNDS} rounds of changes.", dev.name)
        }
        None => {
            let brief = format!("{}{}{notes}", fixes_brief(&saved), resume_section(&stopped, &[]));
            crate::agents::resume_on_ticket(state, &dev.id, &ticket_key(&saved, None), brief, wake_for(&saved, Step::FinalFixes { round: 0 }));
            format!(" {} is fixing what the final Code Review found, with a fresh {MAX_ROUNDS} rounds of changes; then the Code reviewer looks again.", dev.name)
        }
    };
    Ok(format!("Resumed {}.{line}{unlabelled}", subject(&saved)))
}

/// Resume on a stalled or paused Delivery. It first reconciles what was done meanwhile (tickets closed by hand), then
/// wakes only the role its step waits on, with its role brief and a "this is a resume" section: the Dev for a
/// ticket being built or the final fixes, the reviewers whose verdict is still pending, the Code reviewer for
/// the final review, or the next ticket's builder when nothing is in flight. The Delivery goes back to the
/// state it stalled or paused in, so a stall's Needs you item goes. A ticket or final review waiting for the user gets a fresh
/// set of rounds instead (`resume_needs_human`). Anything else is refused, saying why. The line saying what it
/// did.
pub async fn resume(state: &AppState, repo: &str, spec: i64) -> Result<String, String> {
    let _held = state.publish_locks.of(repo).lock_owned().await;
    let d = state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.ok_or_else(|| format!("Spec #{spec} on {repo} has no Delivery."))?;
    if waits_for_user(&d) {
        return resume_needs_human(state, d).await;
    }
    let stalled = match d.state.as_str() {
        "stalled" => format!("the Delivery stalled because {}", stall_why(&d.stall_reason)),
        "paused" => "the user paused the Delivery, and Loft stopped the work on its step".to_string(),
        _ => return Err(not_resumable(&d)),
    };
    let (d, closed) = reconcile(state, &d).await.map_err(|e| format!("Nothing was resumed: {e}."))?;
    let closed = match closed.as_slice() {
        [] => String::new(),
        ns => format!(" Closed by hand meanwhile, so marked closed: {}.", ns.iter().filter_map(|n| d.ticket(*n)).map(|t| tag(&d, t)).collect::<Vec<_>>().join(", ")),
    };
    let unstall = |f: &dyn Fn(&mut Delivery)| {
        update(state, repo, spec, |d| {
            d.unstall();
            f(d);
        })
        .ok_or("Nothing was resumed: the Delivery record couldn't be saved.")
    };
    let line = match d.resume_to.as_str() {
        "final_review" => {
            let cr = with_role(state, &d, "code_reviewer").ok_or("Nothing was resumed: the team has no Code reviewer any more, so nobody can do the final review.")?;
            let d = unstall(&|_| {})?;
            let brief = format!("{}{}", final_brief(state, &d), resume_section(&stalled, &[]));
            crate::agents::resume_on_ticket(state, &cr.id, &ticket_key(&d, None), brief, wake_for(&d, Step::FinalReview { round: d.final_rounds }));
            format!(" {} (Code reviewer) is doing the final review again.", cr.name)
        }
        "final_fixes" => {
            let dev = dev_of(state, &d, &d.dev)?;
            let trailer = if d.local { format!("Refs {}", d.spec_id) } else { format!("Refs #{}", d.spec) };
            let found = match branch_commits(state, &d, &trailer, &[]).await {
                Ok(c) if c.is_empty() => format!("No commit with `{trailer}` is on the {} yet.", branch_name(&d)),
                Ok(c) => format!("On the {} with `{trailer}` (this round's fixes, or earlier rounds'): {}.", branch_name(&d), c.join(", ")),
                Err(e) => format!("Loft couldn't read the {} ({e}): check what's already on it yourself.", branch_name(&d)),
            };
            let d = unstall(&|d| d.dev = dev.id.clone())?;
            let brief = format!("{}{}", fixes_brief(&d), resume_section(&stalled, &[found]));
            crate::agents::resume_on_ticket(state, &dev.id, &ticket_key(&d, None), brief, wake_for(&d, Step::FinalFixes { round: d.final_rounds }));
            format!(" {} is fixing what the final Code Review found again.", dev.name)
        }
        _ => {
            let inflight: Vec<DeliveryTicket> = d.tickets.iter().filter(|t| matches!(t.state.as_str(), "started" | "review" | "closing")).cloned().collect();
            let mut lines = String::new();
            for t in &inflight {
                lines += &resume_ticket(state, repo, spec, t, &stalled).await?;
            }
            if !inflight.is_empty() {
                lines
            } else {
                // Nothing in flight: the next ticket, or what comes after the last one.
                let d = unstall(&|_| {})?;
                if !d.tickets.is_empty() && d.tickets.iter().all(|t| t.state == "closed") {
                    crate::planning::after_tickets(state, &d).await.unwrap_or_else(|e| Some(format!("\nBut {e}: Loft tries again on its next poll."))).unwrap_or_default()
                } else {
                    start_next(state, repo, spec).await
                }
            }
        }
    };
    Ok(format!("Resumed {}.{closed}{}", subject(&d), line.replace('\n', " ").trim_end()))
}

/// Resume on one ticket of a stalled or paused Delivery that was in flight: wakes whoever its step waits on.
async fn resume_ticket(state: &AppState, repo: &str, spec: i64, t: &DeliveryTicket, stalled: &str) -> Result<String, String> {
    let d = state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.ok_or("Nothing was resumed: the Delivery record is gone.")?;
    let unstall = |f: &dyn Fn(&mut Delivery)| {
        update(state, repo, spec, |d| {
            d.unstall();
            f(d);
        })
        .ok_or("Nothing was resumed: the Delivery record couldn't be saved.")
    };
    let t = d.ticket(t.number).cloned().unwrap_or_else(|| t.clone());
    let line = if t.state == "started" {
                let dev = dev_of(state, &d, builder_id(&d, &t))?;
                let done = build_done(state, &d, &t).await;
                let d = unstall(&|d| d.dev = dev.id.clone())?;
                let brief = format!("{}{}", dev_brief(state, &d, &t), resume_section(stalled, &done));
                crate::agents::resume_on_ticket(state, &dev.id, &ticket_key(&d, Some(t.number)), brief, wake_for(&d, Step::Build { ticket: t.number, round: t.rounds }));
                format!(" {} is building {} again.", dev.name, tag(&d, &t))
    } else if t.state == "review" {
                let pending: Vec<&str> = TICKET_REVIEWERS.iter().copied().filter(|r| t.verdicts.get(*r).is_some_and(|v| v == "pending")).collect();
                let reviewers = pending
                    .iter()
                    .map(|r| with_role(state, &d, r).map(|a| (*r, a)).ok_or_else(|| format!("Nothing was resumed: {} waits for a {} verdict, and the team has no {0} any more.", tag(&d, &t), crate::planning::role_label(r))))
                    .collect::<Result<Vec<_>, _>>()?;
                let d = unstall(&|_| {})?;
                if reviewers.is_empty() {
                    // Every verdict is in: it only had to close.
                    finish(state, &d, t.number).await?
                } else {
                    let passed: Vec<&str> = t.verdicts.iter().filter(|(_, v)| *v == "pass").map(|(r, _)| crate::planning::role_label(r)).collect();
                    let done = if passed.is_empty() { vec![] } else { vec![format!("Verdicts already in: {} passed it.", passed.join(" and "))] };
                    for (role, a) in &reviewers {
                        let brief = format!("{}{}", review_brief(state, &d, &t, role), resume_section(stalled, &done));
                        crate::agents::resume_on_ticket(state, &a.id, &ticket_key(&d, Some(t.number)), brief, wake_for(&d, Step::Review { ticket: t.number, role: role.to_string(), round: t.rounds }));
                    }
                    let names = reviewers.iter().map(|(r, a)| format!("{} ({})", a.name, crate::planning::role_label(r))).collect::<Vec<_>>().join(" and ");
                    format!(" {names} {} reviewing {} again.", if reviewers.len() > 1 { "are" } else { "is" }, tag(&d, &t))
                }
    } else {
                let d = unstall(&|_| {})?;
                finish(state, &d, t.number).await?
    };
    Ok(line)
}

/// `POST /api/issues/{id}/resume-delivery`: Resume on the Delivery card of a spec's or standalone ticket's
/// issue (its Loft issue, in a plan kept in Loft). The Delivery as the card shows it, and what Resume did.
pub async fn resume_from_issue(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = delivery_of_issue(&state, &id)?;
    let message = resume(&state, &d.repo, d.spec).await.map_err(bad)?;
    card_reply(&state, &d, message)
}

/// The Delivery of a spec's or standalone ticket's issue `id` (its Loft issue, in a plan kept in Loft).
fn delivery_of_issue(state: &AppState, id: &str) -> Result<Delivery, crate::api::ApiError> {
    let t = state.db.get_task(id)?.ok_or_else(not_found)?;
    let d = match &t.github {
        Some(g) => state.db.get_delivery(&g.repo, g.number)?.filter(|d| !d.local),
        None => local_of(state, &t.project, &t.id),
    };
    d.ok_or_else(|| bad("This issue has no Delivery."))
}

/// A Delivery card button's answer: the Delivery as the card shows it now, and what the button did.
fn card_reply(state: &AppState, d: &Delivery, message: String) -> ApiResult<Value> {
    let mut v = delivery_json(state, &state.db.get_delivery(&d.repo, d.spec)?.ok_or_else(not_found)?);
    v["ok"] = json!(true);
    v["message"] = json!(message);
    Ok(Json(v))
}

/// Whether the user can pause `d`: an agent should be working on its step (building tickets, the final review
/// or its fixes), and no ticket waits for the user.
fn pausable(d: &Delivery) -> bool {
    matches!(d.state.as_str(), "running" | "final_review" | "final_fixes") && !waits_for_user(d)
}

/// Why a Delivery the user can't pause can't be.
fn not_pausable(d: &Delivery) -> String {
    let what = subject(d);
    match d.state.as_str() {
        "planned" => format!("{what} hasn't started, so there's nothing to pause."),
        "paused" => format!("{what} is already paused: Resume carries it on."),
        "stalled" => format!("{what} is stalled: no agent is working on it, so there's nothing to pause. Resume carries it on."),
        _ if waits_for_user(d) => format!("{what} waits for you after too many changes rounds, so no agent is working on it. Resume gives it a fresh set of rounds."),
        _ => format!("{what} is done: every review passed, so there's nothing to pause."),
    }
}

/// Pause on a running Delivery (its final review and fixes included): it's marked paused, keeping the state
/// to go back to, then each run working on one of its steps is stopped the way the agent's Stop does, and
/// its Delivery wakes still queued are dropped. That stop doesn't stall it, and while paused it starts no
/// ticket, takes no Delivery call and sends no wake, until Resume. Anything else is refused, saying why.
/// The line saying what it did.
pub async fn pause(state: &AppState, repo: &str, spec: i64) -> Result<String, String> {
    let _held = state.publish_locks.of(repo).lock_owned().await;
    let d = state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.ok_or_else(|| format!("Spec #{spec} on {repo} has no Delivery."))?;
    if !pausable(&d) {
        return Err(not_pausable(&d));
    }
    // Paused first: the stopped runs' ends then find nothing to stall.
    let d = update(state, repo, spec, |d| d.resume_to = std::mem::replace(&mut d.state, "paused".into())).ok_or("Nothing was paused: the Delivery record couldn't be saved.")?;
    tracing::info!("delivery {repo}#{spec}: paused");
    let (stopped, dropped) = drop_wakes(state, &d);
    for id in &stopped {
        for p in state.db.list_permissions(Some(id)).unwrap_or_default().into_iter().filter(|p| p.status == "pending") {
            let _ = crate::agents::decide(state, &p.id, "deny", None, Some("Run stopped: the user paused its Delivery.".into()));
        }
        crate::agents::stop(state, id);
    }
    let names = |ids: &[String]| ids.iter().map(|id| state.db.get_agent(id).ok().flatten().map_or_else(|| id.clone(), |a| a.name)).collect::<Vec<_>>().join(" and ");
    let mut line = format!("Paused {}.", subject(&d));
    if !stopped.is_empty() {
        line += &format!(" Stopped the run of {}.", names(&stopped));
    }
    if !dropped.is_empty() {
        line += &format!(" Dropped the Delivery wake queued for {}.", names(&dropped));
    }
    Ok(format!("{line} Loft sends no more wakes until you resume it."))
}

/// Pause's hold on the agents' runs: forgets the steps of `d` that runs are working on, and drops its wakes
/// still queued (their prompts too). The agents whose run to stop, and those whose queued wake was dropped.
fn drop_wakes(state: &AppState, d: &Delivery) -> (Vec<String>, Vec<String>) {
    let of_d = |w: &Wake| w.repo == d.repo && w.spec == d.spec;
    let mut rt = state.rt.lock().unwrap();
    let rt = &mut *rt;
    let mut stopped = vec![];
    for (id, ws) in rt.delivery_running.iter_mut() {
        if ws.iter().any(of_d) {
            ws.retain(|w| !of_d(w));
            stopped.push(id.clone());
        }
    }
    let mut dropped = vec![];
    for (id, ws) in rt.delivery_queued.iter_mut() {
        if !ws.iter().any(|(w, _)| of_d(w)) {
            continue;
        }
        for (_, prompt) in ws.iter().filter(|(w, _)| of_d(w)) {
            if let (Some(p), Some(q)) = (prompt, rt.queued.get_mut(id))
                && let Some(i) = q.iter().position(|x| x == p)
            {
                q.remove(i);
            }
        }
        ws.retain(|(w, _)| !of_d(w));
        if ws.is_empty() {
            // No queued wake left to start a new ticket.
            rt.fresh_before.remove(id);
        }
        dropped.push(id.clone());
    }
    rt.queued.retain(|_, q| !q.is_empty());
    rt.delivery_queued.retain(|_, ws| !ws.is_empty());
    rt.delivery_running.retain(|_, ws| !ws.is_empty());
    stopped.sort();
    dropped.sort();
    (stopped, dropped)
}

/// `POST /api/issues/{id}/pause-delivery`: Pause on the Delivery card of a spec's or standalone ticket's issue
/// (its Loft issue, in a plan kept in Loft). The Delivery as the card shows it, and what Pause did.
pub async fn pause_from_issue(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = delivery_of_issue(&state, &id)?;
    let message = pause(&state, &d.repo, d.spec).await.map_err(bad)?;
    card_reply(&state, &d, message)
}

/// `POST /api/deliveries/resume` with `{ repo, spec }`: the "Delivery stalled" Needs you item's Resume.
pub async fn resume_route(State(state): State<AppState>, Json(w): Json<Which>) -> ApiResult<Value> {
    let message = resume(&state, &w.repo, w.spec).await.map_err(bad)?;
    Ok(Json(json!({ "ok": true, "message": message })))
}

/// `POST /api/issues/{id}/start-delivery`: "Start delivery" on a standalone ticket's Loft issue. Makes its
/// Delivery the first time (only for an open `ready-for-agent` issue that isn't a spec's ticket), then
/// starts it like the end-of-planning card does: its branch, its draft PR with `Closes #N`, and the Dev.
pub async fn start_from_issue(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    let Some(g) = t.github.clone() else {
        return Err(bad("This issue isn't on GitHub: its Delivery needs a GitHub repo."));
    };
    let (repo, n) = (g.repo.clone(), g.number);
    if t.labels.iter().any(|l| l.eq_ignore_ascii_case("spec")) {
        return start_spec(&state, &t, &repo, n).await;
    }
    if let Some(d) = of_ticket(&state, &repo, n).filter(|d| !d.standalone) {
        return Err(bad(format!("#{n} is a ticket of spec #{}: start the spec's Delivery from the spec's page.", d.spec)));
    }
    if t.parent_id.is_some() {
        return Err(bad(format!("#{n} is part of a spec: its tickets go through the spec's Delivery, not one of their own.")));
    }
    if matches!(state.db.get_delivery(&repo, n), Ok(None)) {
        let i = crate::github::get(&state, &format!("/repos/{repo}/issues/{n}")).await.map_err(|e| bad(format!("Couldn't read #{n} on {repo}: {e}")))?;
        let ticket = i.get("pull_request").is_none() && i["labels"].as_array().into_iter().flatten().any(|l| l["name"] == "ready-for-agent");
        if !ticket || i["state"] != "open" {
            return Err(bad(format!("#{n} isn't an open ticket (an issue labelled `ready-for-agent`), so it has no Delivery.")));
        }
        if i["parent_issue_url"].is_string() {
            return Err(bad(format!("#{n} is part of a spec on GitHub: its tickets go through the spec's Delivery, not one of their own.")));
        }
        let planner = if t.created_by != "user" { t.created_by.as_str() } else { "" };
        record_standalone(&state, &repo, n, i["title"].as_str().unwrap_or(&t.title), &t.project, planner);
    }
    let s = start(&state, &repo, n, "").await.map_err(bad)?;
    // The Delivery as the card shows it, like Pause and Resume answer, with who started it.
    let mut v = delivery_json(&state, &state.db.get_delivery(&repo, n)?.ok_or_else(not_found)?);
    v["ok"] = json!(true);
    v["duplicate"] = json!(s.already);
    v["ticket"] = json!(n);
    v["agent_id"] = json!(s.agent.id);
    v["agent_name"] = json!(s.agent.name);
    Ok(Json(v))
}

/// Start delivery on spec #spec's Loft issue, for a spec published before this build or whose end-of-planning
/// card is gone. The first time (and while it hasn't started) Loft builds the Delivery from GitHub: the
/// spec's sub-issues are its tickets, with their blocking links; closed ones are done, and one labelled
/// `in-progress` or with an open PR of its own is built elsewhere, so it's never handed out. Then the spec
/// branch and draft spec PR, reused when there (a `spec/<N>-` PR from the repo itself, never a fork's), and
/// the first unblocked ticket on the team's Dev.
async fn start_spec(state: &AppState, t: &crate::db::Task, repo: &str, spec: i64) -> ApiResult<Value> {
    let planned = |d: &Delivery| d.state == "planned" && !d.local;
    if state.db.get_delivery(repo, spec)?.is_none_or(|d| planned(&d)) {
        let read = |path: String| async move { crate::github::get(state, &format!("/repos/{repo}{path}")).await.map_err(|e| bad(format!("Couldn't read spec #{spec} on {repo}: {e}"))) };
        let i = read(format!("/issues/{spec}")).await?;
        let is_spec = i["labels"].as_array().into_iter().flatten().any(|l| l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("spec")));
        if !is_spec || i["state"] != "open" || i.get("pull_request").is_some() {
            return Err(bad(format!("#{spec} isn't an open spec on GitHub (an issue labelled `spec`), so it has no Delivery.")));
        }
        let subs: Vec<Value> = read(format!("/issues/{spec}/sub_issues?per_page=100")).await?.as_array().cloned().unwrap_or_default();
        let subs: Vec<Value> = subs.into_iter().filter(|x| x.get("pull_request").is_none() && x["number"].is_i64()).collect();
        if !subs.iter().any(|x| x["state"] == "open") {
            return Err(bad(format!("Spec #{spec} has no open sub-issue on GitHub, so there is no ticket to deliver.")));
        }
        let numbers: Vec<i64> = subs.iter().filter_map(|x| x["number"].as_i64()).collect();
        let mut blockers = std::collections::HashMap::new();
        for x in subs.iter().filter(|x| x["issue_dependencies_summary"]["blocked_by"].as_i64().or(x["issue_dependencies_summary"]["total_blocked_by"].as_i64()).unwrap_or(0) > 0) {
            let n = x["number"].as_i64().unwrap_or_default();
            let v = read(format!("/issues/{n}/dependencies/blocked_by?per_page=100")).await?;
            // Only blockers among the spec's tickets: the Delivery can't tell when another issue closes.
            let by: Vec<i64> = v.as_array().into_iter().flatten().filter_map(|b| b["number"].as_i64()).filter(|b| numbers.contains(b) && *b != n).collect();
            blockers.insert(n, by);
        }
        let mut prs: Vec<Value> = vec![];
        for page in 1..=10 {
            let got = read(format!("/pulls?state=open&per_page=100&page={page}")).await?.as_array().cloned().unwrap_or_default();
            let last = got.len() < 100;
            prs.extend(got);
            if last {
                break;
            }
        }
        // Only PRs from the repo itself: a public repo's fork PRs are anyone's, so one must never become the
        // spec PR (agents would build and push to its head) nor mark a ticket built elsewhere.
        prs.retain(|p| p["head"]["repo"]["full_name"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(repo)));
        let closes = |p: &Value, n: i64| closes_issue(p["body"].as_str().unwrap_or(""), n);
        let spec_pr = prs.iter().find(|p| closes(p, spec) && p["head"]["ref"].as_str().is_some_and(|r| r.starts_with(&format!("spec/{spec}-"))));
        let own_pr = |n: i64| prs.iter().find(|p| closes(p, n) || p["head"]["ref"].as_str().is_some_and(|r| r.starts_with(&format!("ticket/{n}-")))).and_then(|p| p["number"].as_i64());

        let title = i["title"].as_str().unwrap_or(&t.title).to_string();
        let planner = if t.created_by != "user" { t.created_by.as_str() } else { "" };
        let _held = state.publish_locks.of(repo).lock_owned().await;
        let branch = spec_pr.and_then(|p| p["head"]["ref"].as_str()).map_or_else(|| crate::planning::spec_branch(spec, &title), String::from);
        record_spec(state, repo, spec, &t.project, planner, &branch);
        update(state, repo, spec, |d| {
            if d.pr.is_none() {
                d.pr = spec_pr.and_then(|p| p["number"].as_i64());
            }
            if d.state != "planned" {
                return;
            }
            for x in &subs {
                let n = x["number"].as_i64().unwrap_or_default();
                let skipped = if x["state"] != "open" {
                    None
                } else if x["labels"].as_array().into_iter().flatten().any(|l| l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("in-progress"))) {
                    Some("is labelled in-progress".to_string())
                } else {
                    own_pr(n).map(|pr| format!("has its own open PR #{pr}"))
                };
                let state = match (&skipped, x["state"] == "open") {
                    (_, false) => "closed",
                    (Some(_), _) => "elsewhere",
                    (None, _) => "open",
                };
                let blocked_by = blockers.remove(&n).unwrap_or_default();
                let title = x["title"].as_str().unwrap_or_default().to_string();
                match d.ticket_mut(n) {
                    Some(k) => (k.title, k.blocked_by, k.state, k.skipped) = (title, blocked_by, state.into(), skipped.unwrap_or_default()),
                    None => d.tickets.push(DeliveryTicket { number: n, title, blocked_by, state: state.into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: String::new(), skipped: skipped.unwrap_or_default(), agent: String::new(), notes: String::new() }),
                }
            }
            // A ticket taken off the spec on GitHub isn't the Delivery's any more.
            d.tickets.retain(|k| numbers.contains(&k.number));
        });
        if state.db.get_delivery(repo, spec)?.is_some_and(|d| d.pr.is_none()) {
            crate::planning::branch_and_pr(state, repo, spec, &title, "spec", None).await.map_err(|e| bad(format!("The Delivery didn't start: {e}.")))?;
        }
    }
    let s = start(state, repo, spec, "").await.map_err(bad)?;
    let mut v = delivery_json(state, &state.db.get_delivery(repo, spec)?.ok_or_else(not_found)?);
    v["ok"] = json!(true);
    v["duplicate"] = json!(s.already);
    v["ticket"] = json!(s.ticket);
    Ok(Json(v))
}

/// Whether a PR body closes issue #n with a GitHub closing keyword: "Closes #n", "fixes: #n" and so on.
pub(crate) fn closes_issue(body: &str, n: i64) -> bool {
    const KEYWORDS: &[&str] = &["close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved"];
    let body = body.to_lowercase();
    let at = format!("#{n}");
    body.match_indices(&at).any(|(i, _)| {
        let after = body[i + at.len()..].chars().next();
        let before = body[..i].trim_end().trim_end_matches(':').trim_end();
        let word = before.rsplit(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("");
        !after.is_some_and(|c| c.is_ascii_digit()) && KEYWORDS.contains(&word)
    })
}

/// A Delivery card's state line, `{ tone, text }`, or null before it started: the web and iOS cards show it as
/// is and only map its tone (running, stalled, paused, needs_you, final or done) to a colour. Waiting for the
/// user (a ticket after too many change rounds, or the final review) wins over running.
fn state_line(d: &Delivery) -> Value {
    let stalled;
    let (tone, text) = match d.state.as_str() {
        "planned" => return Value::Null,
        "stalled" => {
            stalled = format!("Stalled: {}.", stall_why(&d.stall_reason));
            ("stalled", stalled.as_str())
        }
        "paused" => ("paused", "Paused: Loft sends no wakes until you resume it."),
        "needs_human" => ("needs_you", "Needs you: too many rounds of changes in the final review."),
        "final_review" => ("final", "Final review: the Code reviewer is reviewing the whole PR."),
        "final_fixes" => ("final", "Final review: the Dev is fixing what the Code reviewer found."),
        "ready" | "settled" | "merged" => ("done", "Done: every review passed."),
        _ if d.tickets.iter().any(|t| t.state == "needs_human") => ("needs_you", "Needs you: a ticket had too many rounds of changes."),
        _ => ("running", "Running."),
    };
    json!({ "tone": tone, "text": text })
}

/// What an issue page shows of its Delivery: its state (with why it stalled, its state line, whether Pause and Resume apply, and
/// whether a ticket or its final review waits for the user), whether it started, who has it, its PR and, for a spec, its tickets.
pub(crate) fn delivery_json(state: &AppState, d: &Delivery) -> Value {
    let dev = state.db.get_agent(&d.dev).ok().flatten();
    let current = d.current.and_then(|n| d.ticket(n)).filter(|t| t.state != "needs_human");
    let mut v = json!({
        "state": d.state,
        "stall_reason": (d.state == "stalled").then_some(d.stall_reason.as_str()),
        "state_line": state_line(d),
        "can_pause": pausable(d),
        "can_resume": matches!(d.state.as_str(), "stalled" | "paused") || waits_for_user(d),
        "needs_human": d.state == "needs_human" || d.tickets.iter().any(|t| t.state == "needs_human"),
        "started": d.state != "planned",
        "agent_id": dev.as_ref().map(|a| a.id.clone()),
        "agent_name": dev.as_ref().map(|a| a.name.clone()),
        "pr": d.pr,
        "behind": tracked_pr(state, d).filter(|p| p.behind_by > 0 && p.state == "open").map(|p| json!({ "count": p.behind_by, "base": p.base_ref })),
    });
    if !d.standalone {
        v["spec"] = json!(true);
        v["current"] = json!(current.map(|t| json!({ "number": t.number, "title": t.title })));
        let agent = |t: &DeliveryTicket| state.db.get_agent(builder_id(d, t)).ok().flatten().filter(|_| matches!(t.state.as_str(), "started" | "review" | "closing"));
        v["tickets"] = json!(d.tickets.iter().map(|t| json!({ "number": t.number, "title": t.title, "state": t.state, "skipped": t.skipped, "agent_id": agent(t).map(|a| a.id), "agent_name": agent(t).map(|a| a.name) })).collect::<Vec<_>>());
        // Ready tickets nobody has yet, while the Lead hands them out.
        let waiting: Vec<i64> = if d.state == "running" && lead_of(state, d).is_some() { ready_tickets(d).iter().map(|t| t.number).collect() } else { vec![] };
        v["waiting_for_lead"] = json!(waiting);
        // A ticket after too many changes rounds waits on the user: no agent is running it.
        v["waiting_on_you"] = json!(d.tickets.iter().filter(|t| t.state == "needs_human").map(|t| t.number).collect::<Vec<_>>());
    }
    v
}

/// `GET /api/issues/{id}/delivery`: the Delivery of a standalone ticket's or a spec's Loft issue, for its
/// page: whether it started, who has it and its PR, and a spec's tickets. `null` when it has none.
pub async fn issue_delivery(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    let Some(d) = t.github.as_ref().and_then(|g| state.db.get_delivery(&g.repo, g.number).ok().flatten()).filter(|d| !d.local) else {
        return Ok(Json(Value::Null));
    };
    Ok(Json(delivery_json(&state, &d)))
}

/// `GET /api/deliveries/summary`: every spec's Delivery in one read, for the Issues page's spec views:
/// what `issue_delivery` shows, plus `repo` and `spec` (and `spec_id` for a plan kept in Loft) to tell
/// which issue it belongs to, and each ticket's review verdicts.
pub async fn summary_route(State(state): State<AppState>) -> ApiResult<Value> {
    let ds = state.db.all_deliveries()?;
    let out: Vec<Value> = ds
        .iter()
        .filter(|d| !d.standalone)
        .map(|d| {
            let mut v = delivery_json(&state, d);
            v["repo"] = json!(d.repo);
            v["spec"] = json!(d.spec);
            v["spec_id"] = json!(d.spec_id);
            v["local"] = json!(d.local);
            let tickets = v["tickets"].take();
            v["tickets"] = json!(d.tickets.iter().zip(tickets.as_array().into_iter().flatten()).map(|(t, j)| json!({ "number": t.number, "id": t.id, "title": t.title, "state": t.state, "skipped": t.skipped, "agent_id": j["agent_id"], "agent_name": j["agent_name"], "verdicts": t.verdicts })).collect::<Vec<_>>());
            v
        })
        .collect();
    Ok(Json(json!(out)))
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct StartQuery {
    /// The user said yes to setting up git in a project folder that has none.
    init_git: bool,
}

/// The project folder of a plan kept in Loft that hasn't started, when it has no git at all: a folder in
/// the projects folder, never an agent's workspace, with no `.git` (not even a file or a link).
fn without_git(state: &AppState, d: &Delivery) -> Option<std::path::PathBuf> {
    if !d.local || !d.base.is_empty() {
        return None;
    }
    let dir = crate::repos::project_dir(state, &d.project)?;
    std::fs::symlink_metadata(dir.join(".git")).is_err().then_some(dir)
}

/// `POST /api/agents/{id}/messages/{mid}/start-delivery`: the end-of-planning card's "Start delivery".
/// Starts the card's Delivery once and records on the card who got which ticket, so it shows as started
/// after a reload; a second press gets the card back without starting another ticket. A plan kept in Loft
/// whose folder has no git gets `needs_git` back instead, and `?init_git=true` sets git up first.
pub async fn start_from_card(State(state): State<AppState>, Path((id, mid)): Path<(String, i64)>, axum::extract::Query(q): axum::extract::Query<StartQuery>) -> ApiResult<Value> {
    let m = state.db.get_message(&id, mid)?.ok_or_else(not_found)?;
    let card = &m.meta["grill"]["end_card"]["delivery"];
    let (Some(repo), Some(spec)) = (card["repo"].as_str(), card["spec"].as_i64()) else {
        return Err(bad("This card has no Delivery."));
    };
    // A plan kept in Loft in a folder with no git: ask first, and set it up only on the user's yes.
    if let Some(dir) = state.db.get_delivery(repo, spec)?.and_then(|d| without_git(&state, &d)) {
        if !q.init_git {
            return Ok(Json(json!({ "ok": false, "needs_git": true, "dir": dir.display().to_string(), "message": m })));
        }
        crate::greenfield::init_folder(&dir).await.map_err(bad)?;
    }
    let s = start(&state, repo, spec, &id).await.map_err(bad)?;
    if s.ticket > 0 || s.agent.role == "lead" {
        let title = state.db.get_delivery(repo, spec)?.and_then(|d| d.tickets.into_iter().find(|t| t.number == s.ticket)).map(|t| t.title).unwrap_or_default();
        let started = json!({ "ticket": s.ticket, "title": title, "agent_id": s.agent.id, "agent_name": s.agent.name, "at": crate::db::now() });
        if card["started"].is_null() {
            state.db.mark_delivery_started(&id, mid, &started)?;
        }
    }
    let m = state.db.get_message(&id, mid)?.ok_or_else(not_found)?;
    state.emit(crate::state::Event::Message { message: json!(m) });
    Ok(Json(json!({ "ok": true, "duplicate": s.already, "message": m })))
}

/// Every ticket of a Delivery Loft ran is closed: wakes the team's Code reviewer to review the whole spec
/// PR once. None when the team has no Code reviewer (the PR is then marked ready right away).
pub(crate) fn begin_final_review(state: &AppState, d: &Delivery) -> Option<String> {
    let cr = with_role(state, d, "code_reviewer")?;
    // A ready call that failed before is moot: the Code reviewer's pass makes it again.
    let d = update(state, &d.repo, d.spec, |d| (d.state, d.ready_retry) = ("final_review".into(), false))?;
    crate::agents::wake_on_ticket(state, &cr.id, &ticket_key(&d, None), false, final_brief(state, &d), wake_for(&d, Step::FinalReview { round: d.final_rounds }));
    let pr = if d.local { format!(" spec branch `{}`", d.branch) } else { d.pr.map(|pr| format!(" {} #{pr}", pr_name(&d))).unwrap_or_default() };
    let done = if d.standalone { "It passed its reviews" } else { "It was the spec's last open ticket" };
    Some(format!("\n{done}: {} (Code reviewer) is reviewing the whole{pr} now.", cr.name))
}

/// The Code reviewer role brief, sent with the wake that asks for the final review of the spec PR.
fn final_brief(state: &AppState, d: &Delivery) -> String {
    let repo = &d.repo;
    let pr = d.pr.unwrap_or_default();
    let tickets = d.tickets.iter().map(|t| format!("#{}", t.number)).collect::<Vec<_>>().join(", ");
    let done = if d.standalone { format!("ticket #{} passed QA and Security", d.spec) } else { format!("every ticket of spec #{} is closed ({tickets})", d.spec) };
    let again = if d.final_rounds > 0 { "\n- The Dev pushed fixes for your last notes: look again, at those fixes first." } else { "" };
    if d.local {
        return format!(
            "[Loft Delivery: {about}] This is not a message from the user. You are the Code reviewer on this Delivery: every ticket of the spec (Loft issue {spec_id}) is done, and you review the whole spec branch `{branch}` once, in the project's own repo `{dir}`, against `{base}` (`git diff {base}...{branch}`).\n\
             - Review the whole diff: correctness first (logic errors, edge cases, error handling, data loss, races), then what only shows across tickets: pieces that don't fit together, duplication, gaps in the tests.{again}\n\
             - Don't fix the work yourself.\n\
             - {own}\n\
             - When you're done, call `mcp__loft__review_ticket` with \"{spec_id}\": `verdict` \"changes\" if you found anything, however small (every finding in `notes`, each with its file, line and fix: the Dev fixes them all on this PR), otherwise \"pass\", which means nothing is left to fix, and `notes` saying what you checked.",
            about = subject(d),
            spec_id = d.spec_id,
            branch = d.branch,
            dir = project_shown(state, d),
            base = d.base,
            own = own_rules("review it"),
        );
    }
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are the Code reviewer on this Delivery: {done}, and you review the whole {pname} https://github.com/{repo}/pull/{pr} once, on the {bname} `{branch}`.\n\
         - Pull the branch and review its whole diff against the default branch: correctness first (logic errors, edge cases, error handling, data loss, races), then what only shows across tickets: pieces that don't fit together, duplication, gaps in the tests.{again}\n\
         - Don't fix the work yourself.\n\
         - {own}\n\
         - When you're done, call `mcp__loft__review_ticket` with {pr}: `verdict` \"changes\" if you found anything, however small (every finding in `notes`, each with its file, line and fix: the Dev fixes them all on this PR), otherwise \"pass\", which means nothing is left to fix, and `notes` saying what you checked.",
        about = subject(d),
        pname = pr_name(d),
        bname = branch_name(d),
        branch = d.branch,
        own = own_rules("review it"),
    )
}

/// The Delivery whose spec PR (or spec) is #n on `slug`, or in a plan kept in Loft whose spec is the Loft
/// issue named, with the repo's publish lock held. The name to use in replies.
async fn final_of(state: &AppState, me: &crate::db::Agent, args: &Value) -> Result<(String, String, Option<Delivery>, tokio::sync::OwnedMutexGuard<()>), String> {
    let n = match number_of(&args["number"]).ok_or("number is required")? {
        TicketNumber::GitHub(n) => n,
        TicketNumber::Loft(id) => {
            let slug = local_repo(&me.project);
            let held = state.publish_locks.of(&slug).lock_owned().await;
            let d = state.db.deliveries(&slug, false).map_err(|e| e.to_string())?.into_iter().find(|d| d.spec_id == id);
            return Ok((slug, format!("Loft issue {id}"), d, held));
        }
    };
    let slug = crate::planning::github_home(state, me).await?;
    let held = state.publish_locks.of(&slug).lock_owned().await;
    let d = state.db.deliveries(&slug, false).map_err(|e| e.to_string())?.into_iter().find(|d| d.pr == Some(n) || d.spec == n).filter(|d| d.tickets.iter().all(|t| t.number != n) || d.pr == Some(n));
    Ok((slug, format!("#{n}"), d, held))
}

/// `submit_ticket` for fixes asked for in the final Code Review: wakes the Code reviewer to look again.
/// None when #number isn't a spec PR waiting for fixes, so it's a ticket.
async fn final_submit(state: &AppState, me: &crate::db::Agent, args: &Value, commits: &[String]) -> Option<Result<String, String>> {
    let (slug, n, d, _held) = final_of(state, me, args).await.ok()?;
    let d = d.filter(|d| d.step_state() == "final_fixes")?;
    if let Err(e) = refuse_paused(&d) {
        return Some(Err(e));
    }
    if d.dev != me.id {
        return Some(Err(format!("{n}'s fixes are with another agent: only the agent fixing it submits them.")));
    }
    if let Err(e) = refuse_behind(state, &d).await {
        return Some(Err(e));
    }
    let Some(cr) = with_role(state, &d, "code_reviewer") else {
        return Some(Err("The team has no Code reviewer any more: ask the user to look at the spec PR.".into()));
    };
    let Some(d) = update(state, &slug, d.spec, |d| {
        d.unstall();
        d.state = "final_review".into();
    }) else {
        return Some(Err("The Delivery record couldn't be saved.".into()));
    };
    let shas = commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
    let step = Step::FinalReview { round: d.final_rounds };
    crate::agents::wake_on_ticket(state, &cr.id, &ticket_key(&d, None), false, format!("{}\n- The fixes: {shas}.", final_brief(state, &d)), wake_for(&d, step));
    Some(Ok(format!("Submitted the fixes for {}: {} (Code reviewer) is looking again.", final_tag(&d), cr.name)))
}

/// `review_ticket` from the Code reviewer: its verdict on the whole spec PR. Any finding is changes, which go
/// back to the Dev (or, on the third round, to the user); a pass marks the
/// spec PR ready to merge.
async fn final_review(state: &AppState, me: &crate::db::Agent, args: &Value, verdict: &str, notes: &str) -> Result<String, String> {
    let (slug, n, d, _held) = final_of(state, me, args).await?;
    let d = d.filter(|d| d.step_state() == "final_review").ok_or_else(|| format!("{n} isn't a spec PR waiting for its Code Review: the Code reviewer reviews the whole spec PR once every ticket is closed."))?;
    refuse_paused(&d)?;
    let pr = d.pr.unwrap_or_default();
    if !d.local && pr == 0 {
        return Err("The Delivery has no spec PR.".into());
    }
    let what = if d.local { "spec branch".to_string() } else { pr_name(&d).to_string() };
    let head = if verdict == "pass" {
        format!("**Code reviewer** ({}) passed the {what}.", me.name)
    } else {
        format!("**Code reviewer** ({}) asks for changes on the {what}.", me.name)
    };
    let body = format!("{head}{}", if notes.is_empty() { String::new() } else { format!("\n\n{notes}") });
    let on = if d.local {
        note(state, &d.spec_id, &me.id, &body);
        "the spec's Loft issue".to_string()
    } else {
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{pr}/comments"), Some(json!({ "body": body })))
            .await
            .map_err(|e| format!("Couldn't post the verdict on #{pr}: {e}. Call review_ticket again."))?;
        format!("#{pr}")
    };
    let d = if d.state == "stalled" { update(state, &slug, d.spec, Delivery::unstall).unwrap_or(d) } else { d };
    if verdict == "pass" {
        if d.local {
            let ready = local_ready(state, &d);
            return Ok(format!("Posted your pass on {on}.{ready}"));
        }
        let _ = crate::github::request(state, reqwest::Method::DELETE, &format!("/repos/{slug}/issues/{pr}/labels/changes-requested"), None).await;
        let ready = match crate::planning::ready_if_done(state, &d).await {
            Ok(Some(_)) => format!(" Spec PR #{pr} is ready for review now, and the user is told it can be merged."),
            Ok(None) => format!(" Spec PR #{pr} was no longer a draft, so it was left as it is."),
            Err(e) => format!(" But {e}: Loft tries again on its next poll."),
        };
        return Ok(format!("Posted your pass on {on}.{ready}"));
    }
    let labelled = if d.local { String::new() } else { label_issue(state, &slug, pr, "changes-requested").await.err().map(|e| format!(" The comment is posted, but {e}.")).unwrap_or_default() };
    let rounds = d.final_rounds + 1;
    let stuck = rounds >= MAX_ROUNDS;
    update(state, &slug, d.spec, |d| {
        d.final_rounds = rounds;
        d.state = if stuck { "needs_human" } else { "final_fixes" }.into();
        d.last_notes = notes.into();
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    if stuck {
        let why = needs_human(state, &d, None, rounds, notes).await;
        return Ok(format!("Posted your changes on {on}.{labelled} That's round {rounds} of changes on the {what}, so it waits for the user now{why}."));
    }
    let brief = if d.local {
        format!(
            "[Loft Delivery: {about}] This is not a message from the user. {who} (Code reviewer) asked for changes on the spec branch `{branch}` (round {rounds} of {MAX_ROUNDS}):\n\n{notes}\n\n\
             Fix them on that branch with new commits, each ending with `Refs {spec_id}`, push them to `origin`, then call `mcp__loft__submit_ticket` with \"{spec_id}\" and the new commits, oldest first.\n{own}",
        own = own_rules("build and check the work"),
            about = subject(&d),
            spec_id = d.spec_id,
            who = me.name,
            branch = d.branch,
        )
    } else {
        format!(
            "[Loft Delivery: {about}] This is not a message from the user. {who} (Code reviewer) asked for changes on the {pname} https://github.com/{slug}/pull/{pr} (round {rounds} of {MAX_ROUNDS}):\n\n{notes}\n\n\
             Fix them on the {bname} `{branch}` with new commits, each ending with `Refs #{spec}`, push, then call `mcp__loft__submit_ticket` with {pr} and the new commits, oldest first.\n{own}",
        own = own_rules("build and check the work"),
            about = subject(&d),
            pname = pr_name(&d),
            bname = branch_name(&d),
            spec = d.spec,
            who = me.name,
            branch = d.branch,
        )
    };
    crate::agents::wake_on_ticket(state, &d.dev, &ticket_key(&d, None), true, brief, wake_for(&d, Step::FinalFixes { round: rounds }));
    Ok(format!("Posted your changes on {on}.{labelled} The Dev is fixing them (round {rounds} of {MAX_ROUNDS}); the {what} {}.", if d.local { "isn't merged" } else { "stays a draft" }))
}

/// A plan kept in Loft whose last review passed: ready for the user's Merge, which Needs you and the
/// end-of-planning card offer. The line for the reply.
pub(crate) fn local_ready(state: &AppState, d: &Delivery) -> String {
    update(state, &d.repo, d.spec, |d| d.state = "ready".into());
    done(state, &d.repo, d.spec);
    format!(" The spec branch `{}` is ready to merge into `{}`: the user is offered Merge.", d.branch, d.base)
}

/// The Delivery is done: Loft marked its spec PR ready, or set a plan kept in Loft to ready. Only the first
/// time (a retried ready call finds `done_at` set): Needs you gets its Delivery done item, and the agent that
/// planned it, else the team's Lead, is woken once with a summary, in a run whose reply is an Unread reply.
pub(crate) fn done(state: &AppState, repo: &str, spec: i64) {
    let mut first = false;
    let Some(d) = update(state, repo, spec, |d| {
        if d.done_at.is_empty() {
            (d.done_at, first) = (crate::db::now(), true);
        }
    }) else {
        return;
    };
    if !first {
        return;
    }
    let planner = Some(d.planner.as_str()).filter(|p| !p.is_empty()).and_then(|p| state.db.get_agent(p).ok().flatten());
    let (to, lead) = match planner {
        Some(a) => (a, false),
        None => match with_role(state, &d, "lead") {
            Some(a) => (a, true),
            None => return tracing::info!("delivery {repo}#{spec}: done, with nobody to tell but the user"),
        },
    };
    crate::agents::wake_with_reply(state, &to.id, done_brief(state, &d, lead));
}

/// The summary the done wake carries: the spec, its PR (or branch), and the tickets closed.
fn done_brief(state: &AppState, d: &Delivery, lead: bool) -> String {
    let issue = if d.local {
        state.db.get_task(&d.spec_id).ok().flatten()
    } else {
        state.db.list_tasks().unwrap_or_default().into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(&d.repo) && g.number == d.spec))
    };
    let title = title_of(d, issue.as_ref().map(|t| t.title.as_str()));
    let about = subject(d);
    // A plan kept in Loft's subject is its title already.
    let named = if d.local || title == about { about.clone() } else { format!("{about} \"{title}\"") };
    let lead = if lead { " You get this as the team's Lead: the agent that planned it is gone." } else { "" };
    let ready = if d.local {
        format!("Its spec branch `{}` is ready to merge into `{}`: the user is offered Merge.", d.branch, d.base)
    } else {
        format!("Loft marked its {} https://github.com/{}/pull/{} ready for review: the user merges it.", pr_name(d), d.repo, d.pr.unwrap_or_default())
    };
    let closed: Vec<String> = d.tickets.iter().filter(|t| t.state == "closed").map(|t| if d.local { format!("\"{}\"", t.title) } else { format!("#{} \"{}\"", t.number, t.title) }).collect();
    let closed = if d.standalone || closed.is_empty() { String::new() } else { format!("\n- Tickets closed: {}.", closed.join(", ")) };
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. The Delivery of {named} is done.{lead}\n- {ready}{closed}\n\
         Nothing is left to build: tell the user in a line or two."
    )
}

#[derive(serde::Deserialize)]
pub struct Which {
    repo: String,
    spec: i64,
}

/// `GET /api/deliveries/state?repo=&spec=`: where a Delivery stands, for the end-of-planning card's Merge.
pub async fn state_route(State(state): State<AppState>, axum::extract::Query(w): axum::extract::Query<Which>) -> ApiResult<Value> {
    let d = state.db.get_delivery(&w.repo, w.spec)?.ok_or_else(not_found)?;
    Ok(Json(json!({ "state": d.state, "local": d.local, "branch": d.branch, "base": d.base })))
}

/// `POST /api/deliveries/merge` with `{ repo, spec }`: Merge for a plan kept in Loft.
pub async fn merge_route(State(state): State<AppState>, Json(w): Json<Which>) -> ApiResult<Value> {
    let message = merge(&state, &w.repo, w.spec).await.map_err(bad)?;
    state.emit(crate::state::Event::Issues { project: w.repo.trim_start_matches("loft:").to_string() });
    Ok(Json(json!({ "ok": true, "message": message })))
}

/// The Dev's clone of the project's own repo `project`: a checkout in its workspace (a few levels deep)
/// whose `origin` is that folder.
fn dev_clone(state: &AppState, d: &Delivery, project: &std::path::Path) -> Option<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, depth: usize, project: &std::path::Path) -> Option<std::path::PathBuf> {
        let mut dirs: Vec<_> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for p in dirs {
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.') || n == "node_modules" || n == "target");
            if hidden || !p.is_dir() {
                continue;
            }
            if crate::repos::own_git_dir(&p) {
                let cfg = std::fs::read_to_string(p.join(".git").join("config")).unwrap_or_default();
                let origin = cfg.split("[remote \"origin\"]").nth(1).unwrap_or("").split("\n[").next().unwrap_or("");
                let url = origin.lines().find_map(|l| l.trim().strip_prefix("url").map(|r| r.trim_start().trim_start_matches('=').trim()));
                let to = url.map(|u| std::path::PathBuf::from(u.strip_prefix("file://").unwrap_or(u)));
                if to.and_then(|t| t.canonicalize().ok()).is_some_and(|t| t == project) {
                    return Some(p);
                }
            } else if depth < 3
                && let Some(found) = walk(&p, depth + 1, project)
            {
                return Some(found);
            }
        }
        None
    }
    if d.dev.is_empty() {
        return None;
    }
    walk(&crate::agents::workspace(state, &d.dev), 1, &project.canonicalize().ok()?)
}

/// Merges a plan kept in Loft whose last review passed: its spec branch, as the Dev last pushed it, into
/// the branch it started from, in the Dev's own clone. The project's folder is only read from, never
/// written to. Its spec's Loft issue is then done.
pub async fn merge(state: &AppState, repo: &str, spec: i64) -> Result<String, String> {
    let _held = state.publish_locks.of(repo).lock_owned().await;
    let d = state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.filter(|d| d.local).ok_or("There's no plan kept in Loft with that Delivery.")?;
    match d.state.as_str() {
        "ready" => {}
        "merged" => return Ok(format!("`{}` is already merged into `{}`.", d.branch, d.base)),
        _ => return Err(format!("{} isn't ready to merge yet: its reviews haven't all passed.", subject(&d))),
    }
    let project = project_dir(state, &d).map_err(|e| format!("Nothing was merged: {e}."))?;
    let dir = dev_clone(state, &d, &project).ok_or_else(|| format!("Nothing was merged: the Dev has no clone of `{}` in its workspace.", project.display()))?;
    let shown = dir.display();
    let (base_ref, spec_ref) = (format!("refs/heads/{}", d.base), format!("refs/remotes/origin/{}", d.branch));
    git(&dir, &["fetch", "-q", "origin", &format!("+refs/heads/{0}:{spec_ref}", d.branch), &format!("+refs/heads/{0}:refs/remotes/origin/{0}", d.base)])
        .await
        .map_err(|e| format!("Nothing was merged: couldn't fetch `{}` into `{shown}`: {e}", d.branch))?;
    let at = dir.as_path();
    let tip = |r: String| async move { git(at, &["rev-parse", "--verify", "--quiet", &format!("{r}^{{commit}}")]).await.ok() };
    // The clone's own base branch, or the project's when the clone has none yet.
    let base = match tip(base_ref.clone()).await {
        Some(b) => b,
        None => tip(format!("refs/remotes/origin/{}", d.base)).await.ok_or_else(|| format!("Nothing was merged: `{}` isn't in `{shown}`.", d.base))?,
    };
    let branch = tip(spec_ref).await.ok_or_else(|| format!("Nothing was merged: `{}` isn't in `{shown}`.", d.branch))?;
    let done = |how: String| {
        update(state, repo, spec, |d| d.state = "merged".into());
        if let Ok(Some(mut t)) = state.db.get_task(&d.spec_id) {
            crate::tasks::set_status(state, &mut t, "done", "user", Some(&format!("Merged `{}` into `{}`", d.branch, d.base)));
        }
        how
    };
    let merged = format!("Merged `{}` into `{}` in the Dev's clone `{shown}`.", d.branch, d.base);
    if git(&dir, &["merge-base", "--is-ancestor", &branch, &base]).await.is_ok() {
        if git(&dir, &["rev-parse", "--verify", "--quiet", &base_ref]).await.is_err() {
            git(&dir, &["branch", &d.base, &base]).await.map_err(|e| format!("Nothing was merged: {e}"))?;
        }
        return Ok(done(merged));
    }
    let tree = git(&dir, &["merge-tree", "--write-tree", "--no-messages", &base, &branch]).await;
    let Some(tree) = tree.ok().and_then(|t| t.lines().next().map(str::to_string)).filter(|t| !t.is_empty()) else {
        return Err(format!("`{}` doesn't merge cleanly into `{}`, so nothing was merged.", d.branch, d.base));
    };
    let message = format!("Merge {}", subject(&d));
    let commit = git(&dir, &["commit-tree", &tree, "-p", &base, "-p", &branch, "-m", &message]).await.map_err(|e| format!("Nothing was merged: {e}"))?;
    let on = git(&dir, &["symbolic-ref", "--quiet", "HEAD"]).await.unwrap_or_default();
    if on == base_ref {
        // The Dev has the base checked out: its files follow, and git refuses rather than lose its uncommitted work.
        git(&dir, &["merge", "-q", "--ff-only", &commit]).await.map_err(|e| format!("Nothing was merged: {e}"))?;
    } else {
        // Moves the branch only from the commit read above (or makes it), so a change since then fails instead of being lost.
        let old = if git(&dir, &["rev-parse", "--verify", "--quiet", &base_ref]).await.is_ok() { base.as_str() } else { "" };
        git(&dir, &["update-ref", "-m", &message, &base_ref, &commit, old]).await.map_err(|e| format!("Nothing was merged: {e}"))?;
    }
    Ok(done(merged))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys an agent's message naming the same ticket gets (`agents::message_ticket_key`), so Delivery and
    /// a hand-off by message share one ticket boundary.
    #[test]
    fn ticket_keys_match_a_message_naming_the_ticket() {
        let mut d: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": "running", "created_at": "" })).unwrap();
        assert_eq!(ticket_key(&d, Some(12)), crate::agents::message_ticket_key("#12", Some("o/r")).unwrap().unwrap());
        assert_eq!(ticket_key(&d, None), "o/r#9/final", "the final review is its own boundary");
        d.repo = local_repo("team");
        d.local = true;
        d.tickets.push(DeliveryTicket { number: 1, title: "A".into(), blocked_by: vec![], state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: "loft-issue-1".into(), skipped: String::new(), agent: String::new(), notes: String::new() });
        assert_eq!(ticket_key(&d, Some(1)), crate::agents::message_ticket_key("loft-issue-1", None).unwrap().unwrap());
    }

    /// One read lists every spec's Delivery with its tickets and verdicts, and leaves out a standalone ticket's.
    #[tokio::test]
    async fn summary_lists_every_spec_delivery_but_not_standalone_tickets() {
        let st = crate::planning::tests::state();
        let mut a: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": "running", "created_at": "" })).unwrap();
        a.tickets = vec![ticket(10, "", "review"), ticket(11, "", "open")];
        a.tickets[0].verdicts.insert("qa".into(), "pending".into());
        a.current = Some(10);
        let mut b = a.clone();
        b.spec = 12;
        b.state = "merged".into();
        let mut one = a.clone();
        one.spec = 20;
        one.standalone = true;
        for d in [&a, &b, &one] {
            st.db.save_delivery(d).unwrap();
        }
        let v = summary_route(State(st)).await.unwrap().0;
        let specs: Vec<(i64, &str)> = v.as_array().unwrap().iter().map(|d| (d["spec"].as_i64().unwrap(), d["state"].as_str().unwrap())).collect();
        assert_eq!(specs, vec![(12, "merged"), (9, "running")]);
        assert_eq!(v[1]["repo"], "o/r");
        assert_eq!(v[1]["current"]["number"], 10);
        assert_eq!(v[1]["tickets"][0]["verdicts"]["qa"], "pending");
        assert_eq!(v[1]["tickets"][0]["id"], "", "a ticket kept in Loft carries its issue id");
    }

    /// The card's state line comes from the server, for every state and stall reason: none before it started.
    #[tokio::test]
    async fn issue_delivery_carries_the_state_line() {
        let st = crate::planning::tests::state();
        let (mut t, _) = crate::tasks::create(&st, &serde_json::json!({ "title": "Spec" }), "user").unwrap();
        t.github = Some(crate::db::GithubLink { repo: "o/r".into(), number: 9, state: "open".into(), ..Default::default() });
        st.db.save_task(&t).unwrap();
        let line = |state: &str, reason: &str, tickets: Vec<DeliveryTicket>| {
            let mut d: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": state, "stall_reason": reason, "created_at": "" })).unwrap();
            d.tickets = tickets;
            st.db.save_delivery(&d).unwrap();
            let st = st.clone();
            let id = t.id.clone();
            async move { issue_delivery(State(st), Path(id)).await.unwrap().0["state_line"].clone() }
        };
        let says = |tone: &str, text: &str| serde_json::json!({ "tone": tone, "text": text });
        assert_eq!(line("planned", "", vec![]).await, Value::Null);
        assert_eq!(line("running", "", vec![ticket(10, "", "review")]).await, says("running", "Running."));
        assert_eq!(line("running", "", vec![ticket(10, "", "needs_human")]).await, says("needs_you", "Needs you: a ticket had too many rounds of changes."));
        assert_eq!(line("stalled", "run_failed", vec![]).await, says("stalled", "Stalled: the run Loft woke for its step failed."));
        assert_eq!(line("stalled", "no_call", vec![]).await, says("stalled", "Stalled: the run Loft woke for its step ended without its call."));
        assert_eq!(line("stalled", "restart", vec![ticket(10, "", "needs_human")]).await, says("stalled", "Stalled: its run was lost when Loft restarted."));
        assert_eq!(line("paused", "", vec![]).await, says("paused", "Paused: Loft sends no wakes until you resume it."));
        assert_eq!(line("needs_human", "", vec![]).await, says("needs_you", "Needs you: too many rounds of changes in the final review."));
        assert_eq!(line("final_review", "", vec![]).await, says("final", "Final review: the Code reviewer is reviewing the whole PR."));
        assert_eq!(line("final_fixes", "", vec![]).await, says("final", "Final review: the Dev is fixing what the Code reviewer found."));
        for done in ["ready", "settled", "merged"] {
            assert_eq!(line(done, "", vec![]).await, says("done", "Done: every review passed."), "{done}");
        }
    }

    /// With two tickets in flight, a stalled Delivery's reason names both and their agents, the Merge item
    /// names the Devs who built the spec, and the summary carries every started ticket's agent.
    #[tokio::test]
    async fn parallel_tickets_name_every_ticket_and_dev() {
        let st = crate::planning::tests::state();
        let a = crate::tasks::tests::agent(&st, "Backend Dev");
        let b = crate::tasks::tests::agent(&st, "Dev 2");
        let mut d: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": "stalled", "stall_reason": "restart", "created_at": "" })).unwrap();
        let (mut t1, mut t2, t3) = (ticket(1, "", "started"), ticket(2, "", "started"), ticket(3, "", "open"));
        (t1.agent, t2.agent) = (a.id.clone(), b.id.clone());
        d.tickets = vec![t1, t2, t3];
        assert_eq!(stalled_reason(&st, &d), "Its run was lost when Loft restarted. Stopped at: building #1 on Backend Dev and building #2 on Dev 2.");
        assert_eq!(builders(&st, &d), vec!["Backend Dev", "Dev 2"]);
        d.tickets.truncate(1);
        d.tickets[0].state = "closed".into();
        assert_eq!(stalled_reason(&st, &d), "Its run was lost when Loft restarted. Stopped at: its next step.");
        d.tickets[0].state = "started".into();
        d.tickets.push(ticket(2, "", "started"));
        d.tickets[1].agent = b.id.clone();
        d.state = "running".into();
        st.db.save_delivery(&d).unwrap();
        let v = summary_route(State(st)).await.unwrap().0;
        let agents: Vec<&str> = v[0]["tickets"].as_array().unwrap().iter().map(|t| t["agent_name"].as_str().unwrap()).collect();
        assert_eq!(agents, vec!["Backend Dev", "Dev 2"]);
    }

    /// A ticket waiting for the user isn't running: the summary gives it no agent and lists it as waiting on
    /// the user, a closing ticket isn't called a review, and Merge names builders only when there are some.
    #[tokio::test]
    async fn needs_human_ticket_waits_on_you_and_is_not_running() {
        let st = crate::planning::tests::state();
        let a = crate::tasks::tests::agent(&st, "Backend Dev");
        let mut d: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": "running", "created_at": "" })).unwrap();
        let (mut t1, mut t2) = (ticket(1, "", "needs_human"), ticket(2, "", "closing"));
        (t1.agent, t2.agent) = (a.id.clone(), a.id.clone());
        d.tickets = vec![t1, t2];
        d.current = Some(1);
        st.db.save_delivery(&d).unwrap();
        let v = summary_route(State(st.clone())).await.unwrap().0;
        assert_eq!((&v[0]["waiting_on_you"], &v[0]["current"], &v[0]["tickets"][0]["agent_id"]), (&json!([1]), &json!(null), &json!(null)));
        assert_eq!(v[0]["tickets"][1]["agent_name"], "Backend Dev");
        d.state = "stalled".into();
        d.stall_reason = "restart".into();
        assert_eq!(stalled_reason(&st, &d), "Its run was lost when Loft restarted. Stopped at: closing #2.");
        // Merge: no "Built by" without builders.
        let mut r: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 10, "state": "ready", "created_at": "", "branch": "spec/x", "base": "main" })).unwrap();
        assert_eq!(crate::needs_you::merge_reason(&st, &r), "Its reviews passed: merge `spec/x` into `main`.");
        r.tickets = vec![ticket(1, "", "closed")];
        r.tickets[0].agent = a.id.clone();
        assert_eq!(crate::needs_you::merge_reason(&st, &r), "Its reviews passed: merge `spec/x` into `main`. Built by Backend Dev.");
    }

    fn ticket(number: i64, id: &str, state: &str) -> DeliveryTicket {
        DeliveryTicket { number, title: format!("T{number}"), blocked_by: vec![], state: state.into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: id.into(), skipped: String::new(), agent: String::new(), notes: String::new() }
    }

    /// Agents often send a GitHub number as text ("85"): both tools take it like 85, a Loft issue id still
    /// picks a ticket of a plan kept in Loft, and a refusal names the repo, the number and what's being built.
    #[tokio::test]
    async fn ticket_number_as_integer_text_or_loft_issue_id() {
        let st = crate::planning::tests::state();
        let me = crate::planning::tests::planner_on(&st, "web", Some("https://github.com/o/r.git"));
        let mut d: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": "running", "created_at": "" })).unwrap();
        d.tickets = vec![ticket(85, "", "started"), ticket(86, "", "open")];
        st.db.save_delivery(&d).unwrap();
        let mut l: Delivery = serde_json::from_value(serde_json::json!({ "repo": local_repo("web"), "spec": 1, "state": "running", "created_at": "", "local": true })).unwrap();
        let id = "0b9c6a52-0c5e-4f43-9a8e-3f1e2b7c4d10";
        l.tickets = vec![ticket(1, id, "started")];
        st.db.save_delivery(&l).unwrap();

        for tool in ["submit_ticket", "review_ticket"] {
            for number in [serde_json::json!(85), serde_json::json!("85"), serde_json::json!(" 85 ")] {
                let (slug, n, d, _) = ticket_of(&st, &me, &serde_json::json!({ "number": number }), tool).await.unwrap();
                assert_eq!((slug.as_str(), n, d.spec), ("o/r", 85, 9), "{tool} {number}");
            }
            let (slug, n, _, _) = ticket_of(&st, &me, &serde_json::json!({ "number": id }), tool).await.unwrap();
            assert_eq!((slug, n), (local_repo("web"), 1), "{tool}: a Loft issue id");

            let no = ticket_of(&st, &me, &serde_json::json!({ "number": "87" }), tool).await.err().unwrap();
            assert!(no.contains("#87 on o/r") && no.contains(tool) && no.contains("Being built there now: #85."), "{no}");
            let no = ticket_of(&st, &me, &serde_json::json!({ "number": "not-a-ticket" }), tool).await.err().unwrap();
            assert!(no.contains("Loft issue not-a-ticket") && no.contains("project web") && no.contains("\"T1\""), "{no}");
            for empty in [serde_json::json!(""), serde_json::json!(0), serde_json::json!(null)] {
                assert!(ticket_of(&st, &me, &serde_json::json!({ "number": empty }), tool).await.err().unwrap().starts_with("number is required"));
            }
        }
    }

    /// Agents send `number` as an integer or a string; digits name a GitHub issue either way (#312).
    #[test]
    fn a_numeric_string_names_a_github_issue() {
        use serde_json::json;
        assert_eq!(number_of(&json!(310)), Some(TicketNumber::GitHub(310)));
        assert_eq!(number_of(&json!("310")), Some(TicketNumber::GitHub(310)));
        assert_eq!(number_of(&json!("#310")), Some(TicketNumber::GitHub(310)));
        assert_eq!(number_of(&json!(" # 310 ")), Some(TicketNumber::GitHub(310)));
        let id = "0b9c7a3e-5f1d-4c2b-9e8a-7d6f5e4c3b2a";
        assert_eq!(number_of(&json!(id)), Some(TicketNumber::Loft(id.into())), "a Loft issue id keeps the Loft path");
        assert_eq!(number_of(&json!("loft-issue-1")), Some(TicketNumber::Loft("loft-issue-1".into())));
        for bad in [json!(0), json!(-3), json!("0"), json!("#"), json!(""), json!("  "), json!(null), json!(true)] {
            assert_eq!(number_of(&bad), None, "{bad} names no ticket");
        }
    }
}
