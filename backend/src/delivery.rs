//! Deliveries: Loft running a spec's tickets from published to merged, with no agent coordinating it. The
//! Dev builds one ticket at a time, and starts the next unblocked one while its last is in review. The
//! record (`db::Delivery`) is written when the spec is published and its tickets added as they are; "Start
//! delivery" on the end-of-planning card, or on a spec's or a standalone ticket's issue page, starts the
//! first one. A spec published earlier gets its record from GitHub there.

use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::api::{ApiResult, bad, not_found};
use crate::db::{Delivery, DeliveryTicket};
use crate::state::AppState;

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
            ready_retry: false,
            tickets: vec![],
            current: None,
            dev: String::new(),
            final_rounds: 0,
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
    let _ = state.db.save_delivery(&d);
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
        d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), submitted_at: String::new(), id: id.into(), skipped: String::new(), changes: String::new() });
    });
}

/// Changes the Delivery of spec #spec on `repo`, if it has one, and saves it.
pub fn update(state: &AppState, repo: &str, spec: i64, f: impl FnOnce(&mut Delivery)) -> Option<Delivery> {
    let mut d = state.db.get_delivery(repo, spec).ok()??;
    f(&mut d);
    state.db.save_delivery(&d).ok()?;
    Some(d)
}

/// Adds a published ticket to its spec's Delivery, after the tickets that block it. One already there is left as it is.
pub fn record_ticket(state: &AppState, repo: &str, spec: i64, number: i64, title: &str, blocked_by: Vec<i64>) {
    update(state, repo, spec, |d| {
        if !d.tickets.iter().any(|t| t.number == number) {
            d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), submitted_at: String::new(), id: String::new(), skipped: String::new(), changes: String::new() });
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
/// Lead builds the tickets when its team has no Dev (`builder`).
pub fn fresh_context_applies(state: &AppState, a: &crate::db::Agent) -> bool {
    a.role != "lead" || state.db.agent_with_role(&a.project, "dev").ok().flatten().is_none()
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
         - One or more commits, each message ending with `{refs}`.\n\
         - {own}\n\
         - When the commits are on GitHub and your local tests pass, call `mcp__loft__submit_ticket` with {arg} and all the ticket's commit SHAs, oldest first. Don't wait for the CI: Loft follows it. Loft hands the commits to the team's reviewers and closes the ticket when they pass and the CI is green: never close it yourself.",
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
    let dev = all.iter().find(|a| !project.is_empty() && a.project.trim() == project && a.role == "dev");
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
    begin(state, &d, n, &agent).await.map_err(|e| format!("The Delivery didn't start. {e}"))?;
    Ok(Started { ticket: n, agent, already: false })
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
            x.state = "started".into();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
    crate::agents::wake_on_ticket(state, &agent.id, &ticket_key(&d, Some(n)), true, dev_brief(state, &d, t));
    Ok(())
}

/// What the Dev does next, once it isn't building a ticket (it submitted it, or the ticket closed or went
/// back for changes): a fix waiting in its queue first, then the next unblocked ticket while at most one
/// of its tickets is in review. Nothing while it builds or fixes one, so a run is never interrupted, and
/// no new ticket while one waits for the user. The caller holds the repo's publish lock. A line for the reply.
async fn advance(state: &AppState, repo: &str, spec: i64) -> String {
    let Some(d) = state.db.get_delivery(repo, spec).ok().flatten() else { return String::new() };
    if d.tickets.iter().any(|t| t.state == "started") {
        return String::new();
    }
    if let Some(t) = d.tickets.iter().find(|t| t.state == "changes") {
        return fix(state, &d, t.number);
    }
    if let Some(t) = d.tickets.iter().find(|t| t.state == "needs_human") {
        return format!("\n{} waits for the user, so no new ticket starts.", tag(&d, t));
    }
    let in_review = d.tickets.iter().filter(|t| t.state == "review").count();
    if in_review > 1 {
        return String::new();
    }
    let Some(t) = next_ticket(&d).cloned() else {
        return if in_review > 0 { String::new() } else { format!("\nThe {} has no ticket left to start.", subject(&d)) };
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

/// Hands the Dev the fix waiting for ticket #n: the ticket is being built again, with the changes brief
/// queued when it went back. A line for the reply.
fn fix(state: &AppState, d: &Delivery, n: i64) -> String {
    let mut brief = String::new();
    let Some(d) = update(state, &d.repo, d.spec, |d| {
        d.current = Some(n);
        if let Some(t) = d.ticket_mut(n) {
            t.state = "started".into();
            brief = std::mem::take(&mut t.changes);
        }
    }) else {
        return String::new();
    };
    let Some(t) = d.ticket(n) else { return String::new() };
    crate::agents::wake_on_ticket(state, &d.dev, &ticket_key(&d, Some(n)), true, brief);
    format!("\nNext: the Dev fixes {} (round {} of {MAX_ROUNDS}).", tag(&d, t), t.rounds)
}

/// Sends ticket #n back to the Dev with `brief`, for changes round `rounds`: at once when the Dev isn't
/// building another ticket, otherwise once it submits that one, before any new ticket. The line for the reply.
async fn send_back(state: &AppState, d: &Delivery, n: i64, rounds: u32, brief: String) -> Result<String, String> {
    let d = update(state, &d.repo, d.spec, |d| {
        if let Some(t) = d.ticket_mut(n) {
            t.rounds = rounds;
            t.verdicts.clear();
            (t.state, t.changes) = ("changes".into(), brief);
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    let name = d.ticket(n).map(|t| tag(&d, t)).unwrap_or_default();
    if let Some(other) = d.tickets.iter().find(|t| t.state == "started") {
        return Ok(format!(" The Dev is building {} now: it fixes {name} (round {rounds} of {MAX_ROUNDS}) once it submits that one, before any new ticket.", tag(&d, other)));
    }
    advance(state, &d.repo, d.spec).await;
    Ok(format!(" The Dev is fixing {name} (round {rounds} of {MAX_ROUNDS})."))
}

/// The roles that review each ticket, in the order they are woken.
const TICKET_REVIEWERS: &[&str] = &["qa", "security"];

/// The team's agent with `role` in the Delivery's project, if it has one.
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
        "name": "submit_ticket",
        "description": "As the Dev on a Delivery: hand in the ticket Loft gave you once its commits are pushed to the spec branch and your local tests pass. Don't wait for the CI: Loft follows it. Loft wakes the team's QA and Security to review them, hands you the next unblocked ticket meanwhile, and closes the ticket when they pass and the CI is green. On changes or a red CI, Loft wakes you with their notes or the failure's log, after you submit the ticket you're building: fix with new commits and submit again. Fixes asked for in the final Code Review are submitted with the spec PR's number.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": ["integer", "string"], "description": "The ticket's GitHub issue number, or its Loft issue id in a plan kept in Loft; for final Code Review fixes, the spec PR's number, or the spec's Loft issue id." },
            "commits": { "type": "array", "items": { "type": "string" }, "description": "The SHAs of all the ticket's commits, oldest first." }
        }, "required": ["number", "commits"] }
    }), json!({
        "name": "review_ticket",
        "description": "As QA or Security on a Delivery: give your verdict on the ticket Loft asked you to review. As the Code reviewer: give your verdict on the whole spec PR, once every ticket is closed. Loft posts it on the spec PR as a comment with your notes and, for a ticket, a `qa-passed`, `security:passed` or `changes-requested` label. On changes it wakes the Dev with your notes. When every reviewer passed a ticket and its CI is green, Loft closes it; when the Code reviewer passes the spec PR, Loft marks it ready to merge.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": ["integer", "string"], "description": "The ticket's GitHub issue number, or its Loft issue id in a plan kept in Loft; for the Code reviewer, the spec PR's number, or the spec's Loft issue id." },
            "verdict": { "type": "string", "enum": ["pass", "changes"], "description": "\"changes\" only for problems that must be fixed first." },
            "notes": { "type": "string", "description": "What you checked; for changes, each problem and how to see it." },
            "tech_debt": { "type": "array", "description": "Code reviewer only: findings that can wait. Loft files each as a `tech-debt` issue assigned to the user.", "items": { "type": "object", "properties": {
                "title": { "type": "string" },
                "body": { "type": "string", "description": "Where it is, what's wrong, the suggested fix." }
            }, "required": ["title", "body"] } }
        }, "required": ["number", "verdict", "notes"] }
    })]
}

/// Handle the Delivery tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &crate::db::Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    match name {
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
    let n = match number_of(&args["number"]).ok_or("number is required: the ticket's GitHub issue number")? {
        TicketNumber::GitHub(n) => n,
        TicketNumber::Loft(id) => {
            // A plan kept in Loft names its tickets by their Loft issue ids.
            let slug = local_repo(&me.project);
            let held = state.publish_locks.of(&slug).lock_owned().await;
            let d = state.db.deliveries(&slug, false).map_err(|e| e.to_string())?.into_iter().find(|d| d.tickets.iter().any(|t| t.id == id));
            let d = d.ok_or_else(|| format!("Loft issue {id} isn't a ticket of a Delivery in your project (read as a Loft issue id: a GitHub issue number is only digits): only tickets started with Start delivery go through {tool}."))?;
            let n = d.tickets.iter().find(|t| t.id == id).map(|t| t.number).expect("just found");
            return Ok((slug, n, d, held));
        }
    };
    let slug = crate::planning::github_home(state, me).await.map_err(|why| format!("{tool} works on tickets on GitHub, and {why}."))?;
    let held = state.publish_locks.of(&slug).lock_owned().await;
    let d = of_ticket(state, &slug, n).ok_or_else(|| format!("#{n} on {slug} isn't a ticket of a Delivery (read as a GitHub issue number): only tickets started with Start delivery go through {tool}."))?;
    Ok((slug, n, d, held))
}

/// `submit_ticket`: the Dev hands in its commits, and Loft wakes the team's reviewers (or closes the ticket
/// when the team has none).
async fn submit(state: &AppState, me: &crate::db::Agent, args: &Value) -> Result<String, String> {
    let commits = crate::planning::commits_in(args)?;
    if let Some(out) = final_submit(state, me, args, &commits).await {
        return out;
    }
    let (slug, n, d, _held) = ticket_of(state, me, args, "submit_ticket").await?;
    let t = d.ticket(n).expect("of_ticket found it");
    let name = tag(&d, t);
    if d.current != Some(n) || t.state != "started" {
        let why = match t.state.as_str() {
            "review" => "it's already submitted and waiting for its reviewers",
            "changes" => "it has changes waiting, and Loft sends them to you once you submit the ticket you're building",
            "closed" | "closing" => "it has passed review already",
            "needs_human" => "it's waiting for the user after too many changes rounds",
            _ => "it isn't the ticket being built",
        };
        return Err(format!("{name} can't be submitted: {why}."));
    }
    if d.dev != me.id {
        let who = state.db.get_agent(&d.dev).ok().flatten().map_or("another agent".to_string(), |a| a.name);
        return Err(format!("{name} is with {who}: only the agent building it submits it."));
    }
    let reviewers: Vec<(&str, crate::db::Agent)> = TICKET_REVIEWERS.iter().filter_map(|r| with_role(state, &d, r).map(|a| (*r, a))).collect();
    // A plan kept in Loft has no GitHub CI to follow.
    let ci = !d.local;
    let d = update(state, &slug, d.spec, |d| {
        if let Some(t) = d.ticket_mut(n) {
            t.state = "review".into();
            t.commits = commits.clone();
            t.verdicts = reviewers.iter().map(|(r, _)| (r.to_string(), "pending".to_string())).collect();
            if ci {
                t.verdicts.insert("ci".into(), "pending".into());
            }
            t.submitted_at = crate::db::now();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    if reviewers.is_empty() && !ci {
        let closed = finish(state, &d, n).await?;
        return Ok(format!("Submitted {name}. The team has no QA or Security, so it passed as submitted.{closed}"));
    }
    // Free of #n: the Dev moves on while it's reviewed, in a run of its own.
    let mut next = advance(state, &slug, d.spec).await;
    if !next.is_empty() && state.db.get_delivery(&slug, d.spec).ok().flatten().is_some_and(|d| d.tickets.iter().any(|t| t.state == "started")) {
        next += " Loft wakes you with it once this run ends.";
    }
    if reviewers.is_empty() {
        let followed = follow_ci(state, &d).await.unwrap_or_else(|e| {
            tracing::warn!("delivery {slug}#{}: CI: {e}", d.spec);
            String::new()
        });
        let waits = if followed.is_empty() { " Loft closes it when the CI is green, and wakes you if it fails." } else { "" };
        return Ok(format!("Submitted {name}. The team has no QA or Security, so only its CI is waited for.{waits}{followed}{next}"));
    }
    let t = d.ticket(n).expect("just saved");
    for (role, a) in &reviewers {
        crate::agents::wake_on_ticket(state, &a.id, &ticket_key(&d, Some(n)), false, review_brief(state, &d, t, role));
    }
    let names = reviewers.iter().map(|(r, a)| format!("{} ({})", a.name, crate::planning::role_label(r))).collect::<Vec<_>>().join(" and ");
    let (and_ci, fails, green) = if ci { (", and Loft follows its CI", " or the CI fails", " and the CI is green") } else { ("", "", "") };
    Ok(format!("Submitted {name}: {names} are reviewing it{and_ci}. Loft wakes you if they ask for changes{fails}, and closes it when they pass{green}.{next}"))
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
    if me.role == "code_reviewer" {
        return final_review(state, me, args, verdict, notes).await;
    }
    if args["tech_debt"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("Only the Code reviewer files tech_debt, in its review of the whole spec PR.".into());
    }
    let (slug, n, d, _held) = ticket_of(state, me, args, "review_ticket").await?;
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
        if t.verdicts.iter().any(|(r, v)| r != "ci" && v != "pass") {
            return Ok(format!("Posted your pass on {on}.{labelled} {name} waits for its other reviewers."));
        }
        if t.verdicts.get("ci").is_some_and(|v| v != "pass") {
            // The last reviewer: the CI may well be done by now, so it's read at once.
            let followed = follow_ci(state, &d).await.unwrap_or_else(|e| {
                tracing::warn!("delivery {slug}#{}: CI: {e}", d.spec);
                String::new()
            });
            let waits = if followed.is_empty() { format!(" {name} waits for its CI: Loft closes it when the CI is green.") } else { String::new() };
            return Ok(format!("Posted your pass on {on}.{labelled}{waits}{followed}"));
        }
        let closed = finish(state, &d, n).await?;
        return Ok(format!("Posted your pass on {on}.{labelled}{closed}"));
    }

    let rounds = t.rounds + 1;
    if rounds >= MAX_ROUNDS {
        update(state, &slug, d.spec, |d| {
            if let Some(t) = d.ticket_mut(n) {
                (t.rounds, t.state) = (rounds, "needs_human".into());
                t.verdicts.clear();
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let why = needs_human(state, &d, Some(&t), rounds, notes).await;
        return Ok(format!("Posted your changes on {on}.{labelled} That's round {rounds} of changes on {name}, so it waits for the user now{why}."));
    }
    let brief = changes_brief(&d, &t, rounds, &format!("{} ({label})", me.name), notes);
    let back = send_back(state, &d, n, rounds, brief).await?;
    Ok(format!("Posted your changes on {on}.{labelled}{back}"))
}

/// The wake that sends ticket `t` back to the Dev for changes round `rounds`, asked for by `who` (a reviewer,
/// or the CI) with `notes`. It names the ticket and the commits it judged, so it stands on its own after a
/// cleared context.
fn changes_brief(d: &Delivery, t: &DeliveryTicket, rounds: u32, who: &str, notes: &str) -> String {
    let commits = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. {who} asked for changes on ticket {ticket} (round {rounds} of {MAX_ROUNDS}), submitted as {commits}:\n\n{notes}\n\n\
         Fix it on the {bname} `{branch}` with new commits, each ending with `{refs}`, push, then call `mcp__loft__submit_ticket` again with {arg} and all the ticket's commits, oldest first.\n{own}",
        own = own_rules("build and check the work"),
        about = subject(d),
        bname = branch_name(d),
        ticket = if d.local { ticket_ref(d, t) } else { format!("#{} \"{}\"", t.number, t.title) },
        refs = refs(d, t),
        arg = ticket_arg(d, t),
        branch = d.branch,
    )
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
        let by = state.db.get_agent(&d.dev).ok().flatten();
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
        let next = advance(state, &d.repo, d.spec).await;
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
    let by = state.db.get_agent(&d.dev).ok().flatten().ok_or("The agent that built the ticket is gone.")?;
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
    let next = advance(state, &d.repo, d.spec).await;
    Ok(format!("\n{closed}{next}"))
}

/// How often Loft reads the CI of the tickets waiting for it.
const CI_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// How long a commit with no check at all waits for one when the repo has workflows: a workflow's checks
/// show up seconds after a push, so none after this means none runs on the branch.
const CI_GRACE_MINUTES: i64 = 10;

/// The last lines of a failed job's log a changes round carries; the whole log is at the check's link.
const LOG_LINES: usize = 300;

/// Whether ticket `t` is submitted and waits for its CI.
fn waits_for_ci(t: &DeliveryTicket) -> bool {
    t.state == "review" && t.verdicts.get("ci").is_some_and(|v| v == "pending")
}

/// What the checks of a commit say.
enum Ci {
    /// Some still run, or none showed up yet.
    Running,
    Green,
    /// The repo has no CI, or none runs on the branch.
    Absent,
    /// The failed checks' names, and each one's failure with its log.
    Red(Vec<String>, String),
}

/// Follows the CI of `d`'s tickets that wait for it, on the newest commit of its branch on GitHub. For the
/// waiting tickets that commit includes, a green CI (or none at all) is their pass, and a red one is a
/// changes round with the failure's log for the oldest open ticket it includes. The caller holds the
/// repo's publish lock. The lines for the reply, empty when nothing changed.
async fn follow_ci(state: &AppState, d: &Delivery) -> Result<String, String> {
    if d.local || !d.tickets.iter().any(waits_for_ci) {
        return Ok(String::new());
    }
    let err = |e: anyhow::Error| e.to_string();
    let head = crate::github::get(state, &format!("/repos/{}/git/ref/heads/{}", d.repo, d.branch)).await.map_err(err)?;
    let head = head["object"]["sha"].as_str().ok_or_else(|| format!("GitHub gave the branch `{}` no commit", d.branch))?.to_string();
    // The tickets still open that the commit includes, oldest first.
    let mut included = vec![];
    for t in d.tickets.iter().filter(|t| matches!(t.state.as_str(), "started" | "review" | "changes" | "needs_human")) {
        let Some(last) = t.commits.last() else { continue };
        if includes(state, &d.repo, &head, last).await.map_err(err)? {
            included.push(t);
        }
    }
    let ready: Vec<&DeliveryTicket> = included.iter().copied().filter(|t| waits_for_ci(t)).collect();
    if ready.is_empty() {
        return Ok(String::new());
    }
    let since = ready.iter().map(|t| t.submitted_at.as_str()).max().unwrap_or_default();
    match ci_of(state, &d.repo, &head, since).await.map_err(err)? {
        Ci::Running => Ok(String::new()),
        ci @ (Ci::Green | Ci::Absent) => {
            let why = match ci {
                Ci::Green => format!("its CI is green on `{}`", short(&head)),
                _ => "the repo runs no CI on its branch".to_string(),
            };
            let mut out = String::new();
            for n in ready.iter().map(|t| t.number).collect::<Vec<_>>() {
                out += &ci_passed(state, &d.repo, d.spec, n, &why).await?;
            }
            Ok(out)
        }
        Ci::Red(names, log) => {
            // A ticket whose CI already passed on an earlier commit isn't what broke it.
            match included.iter().find(|t| t.verdicts.get("ci").is_none_or(|v| v != "pass")) {
                Some(t) if waits_for_ci(t) => ci_failed(state, d, t, &head, &names, &log).await,
                // Being fixed, waiting for its fix, or waiting for the user: its next commit runs the CI again.
                _ => Ok(String::new()),
            }
        }
    }
}

/// Whether commit `sha` (full or short) is `head` or one of its ancestors.
async fn includes(state: &AppState, repo: &str, head: &str, sha: &str) -> anyhow::Result<bool> {
    let sha = sha.to_ascii_lowercase();
    if head.starts_with(&sha) {
        return Ok(true);
    }
    match crate::github::get(state, &format!("/repos/{repo}/compare/{sha}...{head}")).await {
        Ok(v) => Ok(matches!(v["status"].as_str(), Some("ahead" | "identical"))),
        // A commit GitHub doesn't have isn't on the branch.
        Err(e) if crate::github::is_gone(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// The checks on commit `sha` of `repo`: its check runs and commit statuses. With none at all, a repo
/// with active workflows waits for them until `CI_GRACE_MINUTES` after `since`, the ticket's submit.
async fn ci_of(state: &AppState, repo: &str, sha: &str, since: &str) -> anyhow::Result<Ci> {
    let runs = crate::github::get(state, &format!("/repos/{repo}/commits/{sha}/check-runs?per_page=100")).await?;
    let statuses = crate::github::get(state, &format!("/repos/{repo}/commits/{sha}/status")).await?;
    let runs = runs["check_runs"].as_array().cloned().unwrap_or_default();
    let statuses = statuses["statuses"].as_array().cloned().unwrap_or_default();
    if runs.is_empty() && statuses.is_empty() {
        let workflows = match crate::github::get(state, &format!("/repos/{repo}/actions/workflows")).await {
            Ok(v) => v,
            Err(e) if crate::github::is_gone(&e) => Value::Null,
            Err(e) => return Err(e),
        };
        let active = workflows["workflows"].as_array().is_some_and(|w| w.iter().any(|w| w["state"] == "active"));
        let waited = chrono::DateTime::parse_from_rfc3339(since).map_or(true, |t| chrono::Utc::now().signed_duration_since(t) > chrono::Duration::minutes(CI_GRACE_MINUTES));
        return Ok(if active && !waited { Ci::Running } else { Ci::Absent });
    }
    if runs.iter().any(|c| c["status"] != "completed") || statuses.iter().any(|s| s["state"] == "pending") {
        return Ok(Ci::Running);
    }
    let failed_runs: Vec<&Value> = runs.iter().filter(|c| !matches!(c["conclusion"].as_str(), Some("success" | "neutral" | "skipped"))).collect();
    let failed_statuses: Vec<&Value> = statuses.iter().filter(|s| s["state"] != "success").collect();
    if failed_runs.is_empty() && failed_statuses.is_empty() {
        return Ok(Ci::Green);
    }
    let (mut names, mut log) = (vec![], String::new());
    for c in failed_runs {
        let name = c["name"].as_str().unwrap_or("a check");
        names.push(name.to_string());
        log += &format!("### `{name}`: {}\n", c["conclusion"].as_str().unwrap_or("failed"));
        if let Some(url) = c["details_url"].as_str() {
            log += &format!("{url}\n");
        }
        for part in ["title", "summary", "text"] {
            if let Some(x) = c["output"][part].as_str().map(str::trim).filter(|x| !x.is_empty()) {
                log += &format!("{x}\n");
            }
        }
        if c["app"]["slug"] == "github-actions"
            && let Some(id) = c["id"].as_i64()
        {
            match crate::github::get_text(state, &format!("/repos/{repo}/actions/jobs/{id}/logs")).await {
                Ok(text) => log += &format!("```\n{}\n```\n", log_tail(&text)),
                Err(e) => log += &format!("(Loft couldn't read its log: {e})\n"),
            }
        }
        log.push('\n');
    }
    for s in failed_statuses {
        let name = s["context"].as_str().unwrap_or("a status");
        names.push(name.to_string());
        log += &format!("### `{name}`: {}\n", s["state"].as_str().unwrap_or("failed"));
        for part in ["description", "target_url"] {
            if let Some(x) = s[part].as_str().map(str::trim).filter(|x| !x.is_empty()) {
                log += &format!("{x}\n");
            }
        }
        log.push('\n');
    }
    Ok(Ci::Red(names, log.trim_end().to_string()))
}

/// The last `LOG_LINES` lines of a GitHub Actions job log, without the timestamp GitHub starts each with.
fn log_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let cut = lines.len().saturating_sub(LOG_LINES);
    let strip = |l: &'_ str| -> String {
        match l.split_once(' ') {
            Some((stamp, rest)) if stamp.ends_with('Z') && chrono::DateTime::parse_from_rfc3339(stamp).is_ok() => rest.to_string(),
            _ => l.to_string(),
        }
    };
    let tail = lines[cut..].iter().map(|l| strip(l)).collect::<Vec<_>>().join("\n");
    if cut > 0 { format!("… ({cut} earlier lines cut)\n{tail}") } else { tail }
}

/// The CI of ticket #n passed (`why`): it closes when its reviewers passed too. The lines for the reply.
async fn ci_passed(state: &AppState, repo: &str, spec: i64, n: i64, why: &str) -> Result<String, String> {
    let d = update(state, repo, spec, |d| {
        if let Some(t) = d.ticket_mut(n) {
            t.verdicts.insert("ci".into(), "pass".into());
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    let t = d.ticket(n).ok_or("The ticket left the Delivery record.")?;
    let name = tag(&d, t);
    if t.verdicts.values().any(|v| v != "pass") {
        return Ok(format!("\n{name}: {why}. It waits for its reviewers."));
    }
    let closed = finish(state, &d, n).await?;
    Ok(format!("\n{name}: {why}.{closed}"))
}

/// The CI failed on commit `head`, which includes ticket `t`: a changes round for `t`, with the failure's
/// log for the Dev, counted against `MAX_ROUNDS` like a reviewer's. The lines for the reply.
async fn ci_failed(state: &AppState, d: &Delivery, t: &DeliveryTicket, head: &str, names: &[String], log: &str) -> Result<String, String> {
    let (n, name) = (t.number, tag(d, t));
    let checks = names.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ");
    let mut posted = String::new();
    if let Some(pr) = d.pr {
        let body = format!("**CI** failed on `{}`, which includes ticket {name}: {checks}.", short(head));
        posted = match crate::github::request(state, reqwest::Method::POST, &format!("/repos/{}/issues/{pr}/comments", d.repo), Some(json!({ "body": body }))).await {
            Ok(_) => label_issue(state, &d.repo, pr, "changes-requested").await.err().map(|e| format!(" Posted on #{pr}, but {e}.")).unwrap_or_default(),
            Err(e) => format!(" Couldn't post it on #{pr}: {e}."),
        };
    }
    let rounds = t.rounds + 1;
    if rounds >= MAX_ROUNDS {
        update(state, &d.repo, d.spec, |d| {
            if let Some(t) = d.ticket_mut(n) {
                (t.rounds, t.state) = (rounds, "needs_human".into());
                t.verdicts.clear();
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let why = needs_human(state, d, Some(t), rounds, &format!("the CI failed: {checks}")).await;
        return Ok(format!("\nThe CI failed on {name} ({checks}).{posted} That's round {rounds} of changes, so it waits for the user now{why}."));
    }
    let notes = format!("The CI failed on `{}`, the newest commit of the {} `{}`, which includes the ticket. Failed: {checks}.\n\n{log}", short(head), branch_name(d), d.branch);
    let back = send_back(state, d, n, rounds, changes_brief(d, t, rounds, "The CI", &notes)).await?;
    Ok(format!("\nThe CI failed on {name} ({checks}).{posted}{back}"))
}

/// Follows the CI of every running Delivery's tickets that wait for it, on a tick of its own: a ticket
/// closes, or goes back to the Dev, within a minute of its CI finishing, without waiting for the PR card's
/// poll or spending an agent's run.
pub async fn ci_poller(state: AppState) {
    let mut tick = tokio::time::interval(CI_EVERY);
    crate::github::as_source("delivery", async {
        loop {
            tick.tick().await;
            follow_all(&state).await;
        }
    })
    .await
}

/// One tick of `ci_poller`.
pub(crate) async fn follow_all(state: &AppState) {
    for d in state.db.deliveries_in_state("running").unwrap_or_default() {
        if d.local || !d.tickets.iter().any(waits_for_ci) {
            continue;
        }
        let _held = state.publish_locks.of(&d.repo).lock_owned().await;
        // Read again under the lock: a submit or a review may have changed it.
        let Some(d) = state.db.get_delivery(&d.repo, d.spec).ok().flatten() else { continue };
        match follow_ci(state, &d).await {
            Ok(line) if !line.is_empty() => tracing::info!("delivery {}#{}: {}", d.repo, d.spec, line.trim()),
            Ok(_) => {}
            Err(e) if e.starts_with("GitHub paused") => {}
            Err(e) => tracing::warn!("delivery {}#{}: CI: {e}", d.repo, d.spec),
        }
    }
}

/// The mirror's catch-up: closes the tickets whose close failed after they passed, then moves the Dev of a
/// running Delivery on when it builds nothing (a start that failed).
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
        let waits = |s: &str| d.tickets.iter().any(|t| t.state == s);
        let due = waits("changes") || !waits("needs_human") && next_ticket(&d).is_some();
        if due && !waits("started") {
            let line = advance(state, repo, d.spec).await;
            tracing::info!("delivery {repo}#{}: {}", d.spec, line.trim());
        }
    }
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
    let pr = state.db.get_delivery(&repo, n)?.and_then(|d| d.pr);
    Ok(Json(json!({ "ok": true, "duplicate": s.already, "ticket": n, "agent_id": s.agent.id, "agent_name": s.agent.name, "pr": pr })))
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
                    None => d.tickets.push(DeliveryTicket { number: n, title, blocked_by, state: state.into(), rounds: 0, commits: vec![], verdicts: Default::default(), submitted_at: String::new(), id: String::new(), skipped: skipped.unwrap_or_default(), changes: String::new() }),
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

/// What an issue page shows of its Delivery: whether it started, who has it, its PR and, for a spec, its tickets.
fn delivery_json(state: &AppState, d: &Delivery) -> Value {
    let dev = state.db.get_agent(&d.dev).ok().flatten();
    let current = d.current.and_then(|n| d.ticket(n));
    let mut v = json!({
        "state": d.state,
        "started": d.state != "planned",
        "agent_id": dev.as_ref().map(|a| a.id.clone()),
        "agent_name": dev.as_ref().map(|a| a.name.clone()),
        "pr": d.pr,
    });
    if !d.standalone {
        v["spec"] = json!(true);
        v["current"] = json!(current.map(|t| json!({ "number": t.number, "title": t.title })));
        v["tickets"] = json!(d.tickets.iter().map(|t| json!({ "number": t.number, "title": t.title, "state": t.state, "skipped": t.skipped })).collect::<Vec<_>>());
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
    if s.ticket > 0 {
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
    let d = update(state, &d.repo, d.spec, |d| d.state = "final_review".into())?;
    crate::agents::wake_on_ticket(state, &cr.id, &ticket_key(&d, None), false, final_brief(state, &d));
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
             - When you're done, call `mcp__loft__review_ticket` with \"{spec_id}\": `verdict` \"changes\" only for problems that must be fixed before merging (in `notes`, each with its file, line and fix), otherwise \"pass\", and `notes` saying what you checked. Put findings that can wait in `tech_debt`: Loft files each as a `tech-debt` Loft issue.",
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
         - When you're done, call `mcp__loft__review_ticket` with {pr}: `verdict` \"changes\" only for problems that must be fixed before merging (in `notes`, each with its file, line and fix), otherwise \"pass\", and `notes` saying what you checked. Put findings that can wait in `tech_debt`: Loft files each as a `tech-debt` issue.",
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
    let d = d.filter(|d| d.state == "final_fixes")?;
    if d.dev != me.id {
        return Some(Err(format!("{n}'s fixes are with another agent: only the agent fixing it submits them.")));
    }
    let Some(cr) = with_role(state, &d, "code_reviewer") else {
        return Some(Err("The team has no Code reviewer any more: ask the user to look at the spec PR.".into()));
    };
    let Some(d) = update(state, &slug, d.spec, |d| d.state = "final_review".into()) else {
        return Some(Err("The Delivery record couldn't be saved.".into()));
    };
    let shas = commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
    crate::agents::wake_on_ticket(state, &cr.id, &ticket_key(&d, None), false, format!("{}\n- The fixes: {shas}.", final_brief(state, &d)));
    Some(Ok(format!("Submitted the fixes for {}: {} (Code reviewer) is looking again.", final_tag(&d), cr.name)))
}

/// `review_ticket` from the Code reviewer: its verdict on the whole spec PR. Findings that can wait become
/// `tech-debt` issues; changes go back to the Dev (or, on the third round, to the user); a pass marks the
/// spec PR ready to merge.
async fn final_review(state: &AppState, me: &crate::db::Agent, args: &Value, verdict: &str, notes: &str) -> Result<String, String> {
    let (slug, n, d, _held) = final_of(state, me, args).await?;
    let d = d.filter(|d| d.state == "final_review").ok_or_else(|| format!("{n} isn't a spec PR waiting for its Code Review: the Code reviewer reviews the whole spec PR once every ticket is closed."))?;
    let pr = d.pr.unwrap_or_default();
    if !d.local && pr == 0 {
        return Err("The Delivery has no spec PR.".into());
    }
    let filed = file_tech_debt(state, &d, &me.id, args["tech_debt"].as_array().cloned().unwrap_or_default()).await;
    let what = if d.local { "spec branch".to_string() } else { pr_name(&d).to_string() };
    let head = if verdict == "pass" {
        format!("**Code reviewer** ({}) passed the {what}.", me.name)
    } else {
        format!("**Code reviewer** ({}) asks for changes on the {what}.", me.name)
    };
    let debt = if filed.0.is_empty() { String::new() } else { format!("\n\nFiled as tech debt: {}.", filed.0.join(", ")) };
    let body = format!("{head}{}{debt}", if notes.is_empty() { String::new() } else { format!("\n\n{notes}") });
    let on = if d.local {
        note(state, &d.spec_id, &me.id, &body);
        "the spec's Loft issue".to_string()
    } else {
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{pr}/comments"), Some(json!({ "body": body })))
            .await
            .map_err(|e| format!("Couldn't post the verdict on #{pr}: {e}. Call review_ticket again."))?;
        format!("#{pr}")
    };
    let filed_line = match (filed.0.is_empty(), filed.1.is_empty()) {
        (true, true) => String::new(),
        _ => format!(" Tech debt: {}{}.", filed.0.join(", "), filed.1),
    };
    if verdict == "pass" {
        if d.local {
            let ready = local_ready(state, &d);
            return Ok(format!("Posted your pass on {on}.{filed_line}{ready}"));
        }
        let _ = crate::github::request(state, reqwest::Method::DELETE, &format!("/repos/{slug}/issues/{pr}/labels/changes-requested"), None).await;
        let ready = match crate::planning::ready_if_done(state, &d).await {
            Ok(Some(_)) => format!(" Spec PR #{pr} is ready for review now, and the user is told it can be merged."),
            Ok(None) => format!(" Spec PR #{pr} was no longer a draft, so it was left as it is."),
            Err(e) => format!(" But {e}: Loft tries again on its next poll."),
        };
        return Ok(format!("Posted your pass on {on}.{filed_line}{ready}"));
    }
    let labelled = if d.local { String::new() } else { label_issue(state, &slug, pr, "changes-requested").await.err().map(|e| format!(" The comment is posted, but {e}.")).unwrap_or_default() };
    let rounds = d.final_rounds + 1;
    let stuck = rounds >= MAX_ROUNDS;
    update(state, &slug, d.spec, |d| {
        d.final_rounds = rounds;
        d.state = if stuck { "needs_human" } else { "final_fixes" }.into();
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    if stuck {
        let why = needs_human(state, &d, None, rounds, notes).await;
        return Ok(format!("Posted your changes on {on}.{labelled}{filed_line} That's round {rounds} of changes on the {what}, so it waits for the user now{why}."));
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
    crate::agents::wake_on_ticket(state, &d.dev, &ticket_key(&d, None), true, brief);
    Ok(format!("Posted your changes on {on}.{labelled}{filed_line} The Dev is fixing them (round {rounds} of {MAX_ROUNDS}); the {what} {}.", if d.local { "isn't merged" } else { "stays a draft" }))
}

/// A plan kept in Loft whose last review passed: ready for the user's Merge, which Needs you and the
/// end-of-planning card offer. The line for the reply.
pub(crate) fn local_ready(state: &AppState, d: &Delivery) -> String {
    update(state, &d.repo, d.spec, |d| d.state = "ready".into());
    format!(" The spec branch `{}` is ready to merge into `{}`: the user is offered Merge.", d.branch, d.base)
}

/// Files the Code reviewer's findings that can wait as `tech-debt` issues assigned to the user (Loft
/// issues in a plan kept in Loft), once each: a retry finds the one already filed. How each is named
/// ("#77", or "Loft issue <id>"), and a note on any that failed.
async fn file_tech_debt(state: &AppState, d: &Delivery, by: &str, items: Vec<Value>) -> (Vec<String>, String) {
    let (slug, mut out, mut failed) = (&d.repo, vec![], vec![]);
    if items.is_empty() {
        return (out, String::new());
    }
    if d.local {
        for i in items {
            let (title, body) = (i["title"].as_str().unwrap_or("").trim(), i["body"].as_str().unwrap_or("").trim());
            if title.is_empty() {
                continue;
            }
            let k = crate::planning::key(title);
            let there = state.db.list_tasks().unwrap_or_default().into_iter().find(|t| t.project == d.project && t.labels.iter().any(|l| l == "tech-debt") && crate::planning::key(&t.title) == k);
            let p = json!({ "title": title, "description": format!("Found in the Code Review of the spec branch `{}` ({}).\n\n{body}", d.branch, subject(d)), "labels": ["tech-debt"], "project": d.project, "status": "inbox" });
            match there.map(Ok).unwrap_or_else(|| crate::tasks::create(state, &p, by).map(|(t, _)| t)) {
                Ok(t) => out.push(format!("Loft issue {}", t.id)),
                Err(_) => failed.push(format!("\"{title}\"")),
            }
        }
        let note = if failed.is_empty() { String::new() } else { format!(" (not filed: {}; send them again)", failed.join(", ")) };
        return (out, note);
    }
    let ready = async {
        crate::planning::ensure_label(state, slug, "tech-debt", "fbca04", "Can wait: found in a Code Review").await?;
        crate::planning::login(state).await
    };
    let login = match ready.await {
        Ok(l) => l,
        Err(e) => return (out, format!(" (none filed: {e})")),
    };
    for i in items {
        let (title, body) = (i["title"].as_str().unwrap_or("").trim(), i["body"].as_str().unwrap_or("").trim());
        if title.is_empty() {
            continue;
        }
        let k = format!("{}/{}", d.spec, crate::planning::key(title));
        if let Ok(Some(n)) = state.db.published_issue(slug, "tech_debt", &k) {
            out.push(format!("#{n}"));
            continue;
        }
        let pr = d.pr.map(|pr| format!("{} #{pr}, ", pr_name(d))).unwrap_or_default();
        let body = format!("Found in the Code Review of {pr}{}.\n\n{body}", subject(d));
        let made = crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues"), Some(json!({ "title": title, "body": body, "labels": ["tech-debt"], "assignees": [login] }))).await;
        match made.ok().and_then(|v| v["number"].as_i64()) {
            Some(n) => {
                let _ = state.db.record_published_issue(slug, "tech_debt", &k, n);
                out.push(format!("#{n}"));
            }
            None => failed.push(format!("\"{title}\"")),
        }
    }
    let note = if failed.is_empty() { String::new() } else { format!(" (not filed: {}; send them again)", failed.join(", ")) };
    (out, note)
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

    #[test]
    fn a_job_log_keeps_its_last_lines_without_their_timestamps() {
        let log = (1..=LOG_LINES + 2).map(|i| format!("2026-11-11T10:00:00.1234567Z line {i}")).collect::<Vec<_>>().join("\n");
        let tail = log_tail(&format!("{log}\nno stamp here"));
        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines[0], "… (3 earlier lines cut)");
        assert_eq!((lines[1], lines[lines.len() - 2], lines[lines.len() - 1]), ("line 4", "line 302", "no stamp here"));
        assert_eq!(log_tail("short\nlog"), "short\nlog");
    }

    /// The keys an agent's message naming the same ticket gets (`agents::message_ticket_key`), so Delivery and
    /// a hand-off by message share one ticket boundary.
    #[test]
    fn ticket_keys_match_a_message_naming_the_ticket() {
        let mut d: Delivery = serde_json::from_value(serde_json::json!({ "repo": "o/r", "spec": 9, "state": "running", "created_at": "" })).unwrap();
        assert_eq!(ticket_key(&d, Some(12)), crate::agents::message_ticket_key("#12", Some("o/r")).unwrap().unwrap());
        assert_eq!(ticket_key(&d, None), "o/r#9/final", "the final review is its own boundary");
        d.repo = local_repo("team");
        d.local = true;
        d.tickets.push(DeliveryTicket { number: 1, title: "A".into(), blocked_by: vec![], state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), submitted_at: String::new(), id: "loft-issue-1".into(), skipped: String::new(), changes: String::new() });
        assert_eq!(ticket_key(&d, Some(1)), crate::agents::message_ticket_key("loft-issue-1", None).unwrap().unwrap());
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
