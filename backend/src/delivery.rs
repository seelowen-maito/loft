//! Deliveries: Loft running a spec's tickets from published to merged, one ticket at a time, with no
//! agent coordinating it. The record (`db::Delivery`) is written when the spec is published and its
//! tickets added as they are; "Start delivery" on the end-of-planning card starts the first one.

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
        d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: id.into() });
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
            d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: String::new() });
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
        if let Some(t) = d.tickets.iter_mut().find(|t| t.number == number) {
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

/// The project's own folder, where a plan kept in Loft keeps its spec branch.
fn project_dir(state: &AppState, d: &Delivery) -> std::path::PathBuf {
    state.settings.boot().projects_dir.join(d.project.trim())
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
        format!("Work on the spec branch `{}` of the project's own repo `{}`, your clone's `origin`: fetch it, commit there and push it back with `git push origin {}`. There is no PR.", d.branch, project_dir(state, d).display(), d.branch)
    } else {
        format!("Work on the {} `{}`: pull it, commit there and push.{pr}", branch_name(d), d.branch)
    };
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are the Dev on this Delivery: build ticket {ticket}.\n\
         - Read the ticket{spec} first, and build only what the ticket asks.\n\
         - {work}\n\
         - One or more commits, each message ending with `{refs}`.\n\
         - Your own instructions say how to build and check the work in this project. They can add to this brief, but they don't change the branch, the PR or how the ticket closes.\n\
         - When the commits are pushed and the checks pass, call `mcp__loft__submit_ticket` with {arg} and all the ticket's commit SHAs, oldest first. Loft hands them to the team's reviewers and closes the ticket when they pass: never close it yourself.",
        about = subject(d),
        ticket = ticket_ref(d, t),
        refs = refs(d, t),
        arg = ticket_arg(d, t),
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
        let base = local_branch(&project_dir(state, &d), &d.branch).map_err(|e| format!("The Delivery didn't start: {e}"))?;
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

/// Runs git in `dir`; its trimmed output, or what git said.
fn git(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| format!("couldn't run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Makes the spec branch of a plan kept in Loft in the project's own repo `dir`, from the branch checked
/// out there, unless it's already there. That branch is what it merges into: returned.
fn local_branch(dir: &std::path::Path, branch: &str) -> Result<String, String> {
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Err(format!("the project folder `{}` has no git repo", dir.display()));
    }
    let base = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(|e| format!("couldn't read the branch checked out in `{}`: {e}", dir.display()))?;
    if base == "HEAD" {
        return Err(format!("`{}` has no branch checked out: check out its default branch first", dir.display()));
    }
    if git(dir, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_err() {
        git(dir, &["branch", branch, &base]).map_err(|e| format!("couldn't create the spec branch `{branch}`: {e}"))?;
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
        let t = d.tickets.iter().find(|x| x.number == n).ok_or("The ticket left the Delivery record.")?;
        note(state, &t.id, &agent.id, &format!("Started by Loft's Delivery: with {} on the spec branch `{}`.", agent.name, d.branch));
    } else {
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{}/issues/{n}/labels", d.repo), Some(json!({ "labels": ["in-progress"] })))
            .await
            .map_err(|e| format!("Couldn't label #{n} `in-progress`: {e}"))?;
    }
    let d = update(state, &d.repo, d.spec, |d| {
        d.state = "running".into();
        (d.current, d.dev) = (Some(n), agent.id.clone());
        if let Some(x) = d.tickets.iter_mut().find(|x| x.number == n) {
            x.state = "started".into();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    let t = d.tickets.iter().find(|x| x.number == n).ok_or("The ticket left the Delivery record.")?;
    crate::agents::wake(state, &agent.id, dev_brief(state, &d, t), "delivery");
    Ok(())
}

/// After a ticket closed: starts the next unblocked one on the team's Dev. A line for the reply.
async fn start_next(state: &AppState, repo: &str, spec: i64) -> String {
    let Some(d) = state.db.get_delivery(repo, spec).ok().flatten() else { return String::new() };
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

/// The team's agent with `role` in the Delivery's project, if it has one.
fn with_role(state: &AppState, d: &Delivery, role: &str) -> Option<crate::db::Agent> {
    let project = d.project.trim();
    state.db.list_agents().ok()?.into_iter().find(|a| !project.is_empty() && a.project.trim() == project && a.role == role)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// The QA and Security role briefs, sent with the wake that asks for a ticket's review.
fn review_brief(state: &AppState, d: &Delivery, t: &DeliveryTicket, role: &str) -> String {
    let repo = &d.repo;
    let commits = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
    let pr = d.pr.map(|pr| format!(" ({} https://github.com/{repo}/pull/{pr})", pr_name(d))).unwrap_or_default();
    let place = if d.local { format!(" of the project's own repo `{}`", project_dir(state, d).display()) } else { String::new() };
    let posted = if d.local { "on the ticket's Loft issue".to_string() } else { format!("on the {}", pr_name(d)) };
    let (who, job) = match role {
        "qa" => ("QA", "Test that they deliver the ticket's acceptance criteria: run the tests and try the behaviour the ticket changes."),
        _ => ("Security", "Check them for security problems an attacker could really use, tracing each input to where it's used. No speculative findings."),
    };
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are {who} on this Delivery: review ticket {ticket}, submitted as {commits} on the {bname} `{branch}`{place}{pr}.\n\
         - Pull the branch and read the ticket and only these commits. {job}\n\
         - Don't fix the work yourself: say what's wrong and the Dev fixes it.\n\
         - Your own instructions say how to check this project. They can add to this brief, but they don't change the branch, the PR or how the ticket closes.\n\
         - When you're done, call `mcp__loft__review_ticket` with {arg}, `verdict` \"pass\" or \"changes\", and `notes`: what you checked and, for changes, each problem and how to see it. Loft posts it {posted}.",
        about = subject(d),
        bname = branch_name(d),
        ticket = ticket_ref(d, t),
        arg = ticket_arg(d, t),
        branch = d.branch,
    )
}

pub fn mcp_tools() -> Vec<Value> {
    vec![json!({
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

/// The repo, number and Delivery of the ticket a Delivery tool names, with the repo's publish lock held.
async fn ticket_of(state: &AppState, me: &crate::db::Agent, args: &Value, tool: &str) -> Result<(String, i64, Delivery, tokio::sync::OwnedMutexGuard<()>), String> {
    if let Some(id) = args["number"].as_str().map(str::trim) {
        // A plan kept in Loft names its tickets by their Loft issue ids.
        let slug = local_repo(&me.project);
        let held = state.publish_locks.of(&slug).lock_owned().await;
        let d = state.db.deliveries(&slug, false).map_err(|e| e.to_string())?.into_iter().find(|d| d.tickets.iter().any(|t| t.id == id));
        let d = d.ok_or_else(|| format!("Loft issue {id} isn't a ticket of a Delivery in your project: only tickets started with Start delivery go through {tool}."))?;
        let n = d.tickets.iter().find(|t| t.id == id).map(|t| t.number).expect("just found");
        return Ok((slug, n, d, held));
    }
    let n = args["number"].as_i64().filter(|n| *n > 0).ok_or("number is required: the ticket's GitHub issue number")?;
    let slug = crate::planning::github_home(state, me).await.map_err(|why| format!("{tool} works on tickets on GitHub, and {why}."))?;
    let held = state.publish_locks.of(&slug).lock_owned().await;
    let d = of_ticket(state, &slug, n).ok_or_else(|| format!("#{n} on {slug} isn't a ticket of a Delivery: only tickets started with Start delivery go through {tool}."))?;
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
    let t = d.tickets.iter().find(|t| t.number == n).expect("of_ticket found it");
    let name = tag(&d, t);
    if d.current != Some(n) || t.state != "started" {
        let why = match t.state.as_str() {
            "review" => "it's already submitted and waiting for its reviewers",
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
    let d = update(state, &slug, d.spec, |d| {
        if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
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
    let t = d.tickets.iter().find(|t| t.number == n).expect("just saved");
    for (role, a) in &reviewers {
        crate::agents::wake(state, &a.id, review_brief(state, &d, t, role), "delivery");
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
    if me.role == "code_reviewer" {
        return final_review(state, me, args, verdict, notes).await;
    }
    if args["tech_debt"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("Only the Code reviewer files tech_debt, in its review of the whole spec PR.".into());
    }
    let (slug, n, d, _held) = ticket_of(state, me, args, "review_ticket").await?;
    let role = me.role.as_str();
    let t = d.tickets.iter().find(|t| t.number == n).expect("of_ticket found it").clone();
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
            if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
                t.verdicts.insert(role.into(), "pass".into());
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let t = d.tickets.iter().find(|t| t.number == n).expect("just saved");
        if t.verdicts.values().any(|v| v != "pass") {
            return Ok(format!("Posted your pass on {on}.{labelled} {name} waits for its other reviewers."));
        }
        let closed = finish(state, &d, n).await?;
        return Ok(format!("Posted your pass on {on}.{labelled}{closed}"));
    }

    let rounds = t.rounds + 1;
    let stuck = rounds >= MAX_ROUNDS;
    update(state, &slug, d.spec, |d| {
        if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
            t.rounds = rounds;
            t.verdicts.clear();
            t.state = if stuck { "needs_human" } else { "started" }.into();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    if stuck {
        let why = needs_human(state, &d, Some(&t), rounds, notes).await;
        return Ok(format!("Posted your changes on {on}.{labelled} That's round {rounds} of changes on {name}, so it waits for the user now{why}."));
    }
    let brief = format!(
        "[Loft Delivery: {about}] This is not a message from the user. {who} ({label}) asked for changes on ticket {ticket} (round {rounds} of {MAX_ROUNDS}):\n\n{notes}\n\n\
         Fix it on the {bname} `{branch}` with new commits, each ending with `{refs}`, push, then call `mcp__loft__submit_ticket` again with {arg} and all the ticket's commits, oldest first.",
        about = subject(&d),
        bname = branch_name(&d),
        who = me.name,
        ticket = if d.local { ticket_ref(&d, &t) } else { format!("#{n} \"{}\"", t.title) },
        refs = refs(&d, &t),
        arg = ticket_arg(&d, &t),
        branch = d.branch,
    );
    crate::agents::wake(state, &d.dev, brief, "delivery");
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
        x.status = "blocked".into();
        x.error = format!("{rounds} rounds of changes on {what}: it needs you. Last notes: {}", notes.lines().next().unwrap_or(""));
        x.updated_at = crate::db::now();
        if state.db.save_task(&x).is_ok() {
            crate::tasks::emit(state, &x);
        }
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
    let t = d.tickets.iter().find(|t| t.number == n).ok_or("The ticket left the Delivery record.")?;
    if d.local {
        // A plan kept in Loft: its Loft issue is marked done, with the commits.
        let by = state.db.get_agent(&d.dev).ok().flatten();
        if let Ok(Some(mut x)) = state.db.get_task(&t.id) {
            let shas = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
            let author = by.as_ref().map_or("loft", |a| a.id.as_str());
            crate::tasks::log(state, &x, author, "comment", &format!("Done in {shas}, on the spec branch `{}`.", d.branch), json!({}));
            x.status = "done".into();
            x.completed_at = Some(crate::db::now());
            x.updated_at = crate::db::now();
            if state.db.save_task(&x).is_ok() {
                crate::tasks::emit(state, &x);
                crate::tasks::log_status(state, &x, author, "Closed by Loft: its reviewers passed it");
            }
        }
        let d = update(state, &d.repo, d.spec, |d| {
            if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
                t.state = "closed".into();
            }
            d.current = None;
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let after = crate::planning::after_tickets(state, &d).await.unwrap_or_default().unwrap_or_default();
        let next = start_next(state, &d.repo, d.spec).await;
        return Ok(format!("\n{} is done.{after}{next}", tag(&d, t)));
    }
    if d.standalone {
        // Its PR closes it when it merges: Loft doesn't close it now.
        let d = update(state, &d.repo, d.spec, |d| {
            if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
                t.state = "closed".into();
            }
            d.current = None;
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
        if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
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
    for d in state.db.deliveries(repo, false).unwrap_or_default().into_iter().filter(|d| d.state == "running") {
        for t in d.tickets.iter().filter(|t| t.state == "closing") {
            match finish(state, &d, t.number).await {
                Ok(line) => tracing::info!("delivery {repo}#{}: {}", d.spec, line.trim()),
                Err(e) => tracing::warn!("delivery {repo}#{}: {e}", d.spec),
            }
        }
        let d = state.db.get_delivery(repo, d.spec).ok().flatten().unwrap_or(d);
        let busy = d.tickets.iter().any(|t| matches!(t.state.as_str(), "started" | "review" | "closing" | "needs_human"));
        if d.current.is_none() && !busy && next_ticket(&d).is_some() {
            let line = start_next(state, repo, d.spec).await;
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
    if let Some(d) = of_ticket(&state, &repo, n).filter(|d| !d.standalone) {
        return Err(bad(format!("#{n} is a ticket of spec #{}: start the spec's Delivery from its end-of-planning card.", d.spec)));
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
        let planner = if t.created_by != "user" { t.created_by.as_str() } else { "" };
        record_standalone(&state, &repo, n, i["title"].as_str().unwrap_or(&t.title), &t.project, planner);
    }
    let s = start(&state, &repo, n, "").await.map_err(bad)?;
    let pr = state.db.get_delivery(&repo, n)?.and_then(|d| d.pr);
    Ok(Json(json!({ "ok": true, "duplicate": s.already, "ticket": n, "agent_id": s.agent.id, "agent_name": s.agent.name, "pr": pr })))
}

/// `GET /api/issues/{id}/delivery`: the standalone Delivery of a ticket's Loft issue, for its page: whether
/// it started, who has it and its PR. `null` when it has none.
pub async fn issue_delivery(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    let Some(d) = t.github.as_ref().and_then(|g| state.db.get_delivery(&g.repo, g.number).ok().flatten()).filter(|d| d.standalone) else {
        return Ok(Json(Value::Null));
    };
    let dev = state.db.get_agent(&d.dev).ok().flatten();
    Ok(Json(json!({
        "state": d.state,
        "started": d.state != "planned",
        "agent_id": dev.as_ref().map(|a| a.id.clone()),
        "agent_name": dev.as_ref().map(|a| a.name.clone()),
        "pr": d.pr,
    })))
}

/// `POST /api/agents/{id}/messages/{mid}/start-delivery`: the end-of-planning card's "Start delivery".
/// Starts the card's Delivery once and records on the card who got which ticket, so it shows as started
/// after a reload; a second press gets the card back without starting another ticket.
pub async fn start_from_card(State(state): State<AppState>, Path((id, mid)): Path<(String, i64)>) -> ApiResult<Value> {
    let m = state.db.get_message(&id, mid)?.ok_or_else(not_found)?;
    let card = &m.meta["grill"]["end_card"]["delivery"];
    let (Some(repo), Some(spec)) = (card["repo"].as_str(), card["spec"].as_i64()) else {
        return Err(bad("This card has no Delivery."));
    };
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
    crate::agents::wake(state, &cr.id, final_brief(state, &d), "delivery");
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
             - Your own instructions say how to review this project. They can add to this brief, but they don't change the branch or how it's merged.\n\
             - When you're done, call `mcp__loft__review_ticket` with \"{spec_id}\": `verdict` \"changes\" only for problems that must be fixed before merging (in `notes`, each with its file, line and fix), otherwise \"pass\", and `notes` saying what you checked. Put findings that can wait in `tech_debt`: Loft files each as a `tech-debt` Loft issue.",
            about = subject(d),
            spec_id = d.spec_id,
            branch = d.branch,
            dir = project_dir(state, d).display(),
            base = d.base,
        );
    }
    format!(
        "[Loft Delivery: {about}] This is not a message from the user. You are the Code reviewer on this Delivery: {done}, and you review the whole {pname} https://github.com/{repo}/pull/{pr} once, on the {bname} `{branch}`.\n\
         - Pull the branch and review its whole diff against the default branch: correctness first (logic errors, edge cases, error handling, data loss, races), then what only shows across tickets: pieces that don't fit together, duplication, gaps in the tests.{again}\n\
         - Don't fix the work yourself.\n\
         - Your own instructions say how to review this project. They can add to this brief, but they don't change the branch, the PR or how it's merged.\n\
         - When you're done, call `mcp__loft__review_ticket` with {pr}: `verdict` \"changes\" only for problems that must be fixed before merging (in `notes`, each with its file, line and fix), otherwise \"pass\", and `notes` saying what you checked. Put findings that can wait in `tech_debt`: Loft files each as a `tech-debt` issue.",
        about = subject(d),
        pname = pr_name(d),
        bname = branch_name(d),
        branch = d.branch,
    )
}

/// The Delivery whose spec PR (or spec) is #n on `slug`, or in a plan kept in Loft whose spec is the Loft
/// issue named, with the repo's publish lock held. The name to use in replies.
async fn final_of(state: &AppState, me: &crate::db::Agent, args: &Value) -> Result<(String, String, Option<Delivery>, tokio::sync::OwnedMutexGuard<()>), String> {
    if let Some(id) = args["number"].as_str().map(str::trim) {
        let slug = local_repo(&me.project);
        let held = state.publish_locks.of(&slug).lock_owned().await;
        let d = state.db.deliveries(&slug, false).map_err(|e| e.to_string())?.into_iter().find(|d| d.spec_id == id);
        return Ok((slug, format!("Loft issue {id}"), d, held));
    }
    let n = args["number"].as_i64().filter(|n| *n > 0).ok_or("number is required")?;
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
    crate::agents::wake(state, &cr.id, format!("{}\n- The fixes: {shas}.", final_brief(state, &d)), "delivery");
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
             Fix them on that branch with new commits, each ending with `Refs {spec_id}`, push them to `origin`, then call `mcp__loft__submit_ticket` with \"{spec_id}\" and the new commits, oldest first.",
            about = subject(&d),
            spec_id = d.spec_id,
            who = me.name,
            branch = d.branch,
        )
    } else {
        format!(
            "[Loft Delivery: {about}] This is not a message from the user. {who} (Code reviewer) asked for changes on the {pname} https://github.com/{slug}/pull/{pr} (round {rounds} of {MAX_ROUNDS}):\n\n{notes}\n\n\
             Fix them on the {bname} `{branch}` with new commits, each ending with `Refs #{spec}`, push, then call `mcp__loft__submit_ticket` with {pr} and the new commits, oldest first.",
            about = subject(&d),
            pname = pr_name(&d),
            bname = branch_name(&d),
            spec = d.spec,
            who = me.name,
            branch = d.branch,
        )
    };
    crate::agents::wake(state, &d.dev, brief, "delivery");
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

/// Merges a plan kept in Loft whose last review passed: its spec branch into the branch it started from, in
/// the project's own repo, only when that branch is checked out and the working tree is clean. Otherwise
/// says why and what to run. Its spec's Loft issue is then done.
pub async fn merge(state: &AppState, repo: &str, spec: i64) -> Result<String, String> {
    let _held = state.publish_locks.of(repo).lock_owned().await;
    let d = state.db.get_delivery(repo, spec).map_err(|e| e.to_string())?.filter(|d| d.local).ok_or("There's no plan kept in Loft with that Delivery.")?;
    match d.state.as_str() {
        "ready" => {}
        "merged" => return Ok(format!("`{}` is already merged into `{}`.", d.branch, d.base)),
        _ => return Err(format!("{} isn't ready to merge yet: its reviews haven't all passed.", subject(&d))),
    }
    let dir = project_dir(state, &d);
    let shown = dir.display();
    let on = git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(|e| format!("Couldn't read the branch checked out in `{shown}`: {e}"))?;
    if on != d.base {
        return Err(format!("`{}` isn't checked out in `{shown}` (`{on}` is). Run `git -C {shown} checkout {}`, then press Merge again.", d.base, d.base));
    }
    let dirty = git(&dir, &["status", "--porcelain"]).map_err(|e| format!("Couldn't read the state of `{shown}`: {e}"))?;
    if !dirty.is_empty() {
        return Err(format!("`{shown}` has uncommitted changes, so nothing was merged. Commit or stash them (`git -C {shown} stash -u`), then press Merge again."));
    }
    if let Err(e) = git(&dir, &["merge", "--no-ff", "-m", &format!("Merge {}", subject(&d)), &d.branch]) {
        let _ = git(&dir, &["merge", "--abort"]);
        return Err(format!("`{}` doesn't merge cleanly into `{}` ({e}), so nothing was merged. Run `git -C {shown} merge {}`, resolve the conflicts and commit.", d.branch, d.base, d.branch));
    }
    update(state, repo, spec, |d| d.state = "merged".into());
    if let Ok(Some(mut t)) = state.db.get_task(&d.spec_id) {
        t.status = "done".into();
        t.completed_at = Some(crate::db::now());
        t.updated_at = crate::db::now();
        if state.db.save_task(&t).is_ok() {
            crate::tasks::emit(state, &t);
            crate::tasks::log_status(state, &t, "user", &format!("Merged `{}` into `{}`", d.branch, d.base));
        }
    }
    Ok(format!("Merged `{}` into `{}` in `{shown}`.", d.branch, d.base))
}
