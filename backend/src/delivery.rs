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
            created_at: crate::db::now(),
        },
        Err(e) => return tracing::warn!("delivery {repo}#{spec}: {e}"),
    };
    let _ = state.db.save_delivery(&d);
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
            d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0, commits: vec![], verdicts: Default::default() });
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

/// The Dev role brief, sent with the wake that hands a ticket to the Dev.
fn dev_brief(d: &Delivery, t: &DeliveryTicket) -> String {
    let repo = &d.repo;
    let pr = d.pr.map(|pr| format!(" Open no PR: the spec PR https://github.com/{repo}/pull/{pr} is the spec's only one.")).unwrap_or_default();
    format!(
        "[Loft Delivery: spec #{spec}] This is not a message from the user. You are the Dev on this Delivery: build ticket #{n} \"{title}\" (https://github.com/{repo}/issues/{n}).\n\
         - Read the ticket and its spec #{spec} first, and build only what the ticket asks.\n\
         - Work on the spec branch `{branch}`: pull it, commit there and push.{pr}\n\
         - One or more commits, each message ending with `Refs #{n}`.\n\
         - Your own instructions say how to build and check the work in this project. They can add to this brief, but they don't change the branch, the PR or how the ticket closes.\n\
         - When the commits are pushed and the checks pass, call `mcp__loft__submit_ticket` with {n} and all the ticket's commit SHAs, oldest first. Loft hands them to the team's reviewers and closes the ticket when they pass: never close it yourself.",
        spec = d.spec,
        n = t.number,
        title = t.title,
        branch = d.branch,
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
    if d.branch.is_empty() {
        return Err(format!("Spec #{spec} has no spec branch yet: call publish_spec again with the same title to make it."));
    }
    let n = next_ticket(&d).map(|t| t.number).ok_or_else(|| format!("Spec #{spec} has no ticket ready to start: every open ticket is blocked, or none was published."))?;
    begin(state, &d, n, &agent).await.map_err(|e| format!("The Delivery didn't start. {e}"))?;
    Ok(Started { ticket: n, agent, already: false })
}

/// Hands ticket #n of `d` to `agent`: labels it `in-progress`, records it as the one being built, and wakes
/// the agent with the Dev brief. The caller holds the repo's publish lock.
async fn begin(state: &AppState, d: &Delivery, n: i64, agent: &crate::db::Agent) -> Result<(), String> {
    crate::github::request(state, reqwest::Method::POST, &format!("/repos/{}/issues/{n}/labels", d.repo), Some(json!({ "labels": ["in-progress"] })))
        .await
        .map_err(|e| format!("Couldn't label #{n} `in-progress`: {e}"))?;
    let d = update(state, &d.repo, d.spec, |d| {
        d.state = "running".into();
        (d.current, d.dev) = (Some(n), agent.id.clone());
        if let Some(x) = d.tickets.iter_mut().find(|x| x.number == n) {
            x.state = "started".into();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    let t = d.tickets.iter().find(|x| x.number == n).ok_or("The ticket left the Delivery record.")?;
    crate::agents::wake(state, &agent.id, dev_brief(&d, t), "delivery");
    Ok(())
}

/// After a ticket closed: starts the next unblocked one on the team's Dev. A line for the reply.
async fn start_next(state: &AppState, repo: &str, spec: i64) -> String {
    let Some(d) = state.db.get_delivery(repo, spec).ok().flatten() else { return String::new() };
    let Some(n) = next_ticket(&d).map(|t| t.number) else {
        return format!("\nSpec #{spec} has no ticket left to start.");
    };
    let Some(agent) = builder(state, &d, &d.dev) else {
        return format!("\nNobody can build #{n}: the team has no Dev and the agent that planned the spec is gone.");
    };
    match begin(state, &d, n, &agent).await {
        Ok(()) => format!("\nNext: #{n} is with {}.", agent.name),
        Err(e) => {
            tracing::warn!("delivery {repo}#{spec}: {e}");
            format!("\n#{n} is next, but {e}: the mirror's next poll starts it.")
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
fn review_brief(d: &Delivery, t: &DeliveryTicket, role: &str) -> String {
    let repo = &d.repo;
    let commits = t.commits.iter().map(|c| format!("`{}`", short(c))).collect::<Vec<_>>().join(", ");
    let pr = d.pr.map(|pr| format!(" (spec PR https://github.com/{repo}/pull/{pr})")).unwrap_or_default();
    let (who, job) = match role {
        "qa" => ("QA", "Test that they deliver the ticket's acceptance criteria: run the tests and try the behaviour the ticket changes."),
        _ => ("Security", "Check them for security problems an attacker could really use, tracing each input to where it's used. No speculative findings."),
    };
    format!(
        "[Loft Delivery: spec #{spec}] This is not a message from the user. You are {who} on this Delivery: review ticket #{n} \"{title}\" (https://github.com/{repo}/issues/{n}), submitted as {commits} on the spec branch `{branch}`{pr}.\n\
         - Pull the branch and read the ticket and only these commits. {job}\n\
         - Don't fix the work yourself: say what's wrong and the Dev fixes it.\n\
         - Your own instructions say how to check this project. They can add to this brief, but they don't change the branch, the PR or how the ticket closes.\n\
         - When you're done, call `mcp__loft__review_ticket` with {n}, `verdict` \"pass\" or \"changes\", and `notes`: what you checked and, for changes, each problem and how to see it. Loft posts it on the spec PR.",
        spec = d.spec,
        n = t.number,
        title = t.title,
        branch = d.branch,
    )
}

pub fn mcp_tools() -> Vec<Value> {
    vec![json!({
        "name": "submit_ticket",
        "description": "As the Dev on a Delivery: hand in the ticket Loft gave you once its commits are pushed to the spec branch and the checks pass. Loft wakes the team's QA and Security to review them, and closes the ticket and starts the next one when they pass. On changes, Loft wakes you with their notes: fix with new commits and submit again.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": "integer", "description": "The ticket's GitHub issue number." },
            "commits": { "type": "array", "items": { "type": "string" }, "description": "The SHAs of all the ticket's commits, oldest first." }
        }, "required": ["number", "commits"] }
    }), json!({
        "name": "review_ticket",
        "description": "As QA or Security on a Delivery: give your verdict on the ticket Loft asked you to review. Loft posts it on the spec PR as a comment with your notes and a `qa-passed`, `security:passed` or `changes-requested` label. On changes it wakes the Dev with your notes; when every reviewer passed, it closes the ticket.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": "integer", "description": "The ticket's GitHub issue number." },
            "verdict": { "type": "string", "enum": ["pass", "changes"] },
            "notes": { "type": "string", "description": "What you checked; for changes, each problem and how to see it." }
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
    let (slug, n, d, _held) = ticket_of(state, me, args, "submit_ticket").await?;
    let t = d.tickets.iter().find(|t| t.number == n).expect("of_ticket found it");
    if d.current != Some(n) || t.state != "started" {
        let why = match t.state.as_str() {
            "review" => "it's already submitted and waiting for its reviewers",
            "closed" | "closing" => "it has passed review already",
            "needs_human" => "it's waiting for the user after too many changes rounds",
            _ => "it isn't the ticket being built",
        };
        return Err(format!("#{n} can't be submitted: {why}."));
    }
    if d.dev != me.id {
        let who = state.db.get_agent(&d.dev).ok().flatten().map_or("another agent".to_string(), |a| a.name);
        return Err(format!("#{n} is with {who}: only the agent building it submits it."));
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
        return Ok(format!("Submitted #{n}. The team has no QA or Security, so it passed as submitted.{closed}"));
    }
    let t = d.tickets.iter().find(|t| t.number == n).expect("just saved");
    for (role, a) in &reviewers {
        crate::agents::wake(state, &a.id, review_brief(&d, t, role), "delivery");
    }
    let names = reviewers.iter().map(|(r, a)| format!("{} ({})", a.name, crate::planning::role_label(r))).collect::<Vec<_>>().join(" and ");
    Ok(format!("Submitted #{n}: {names} are reviewing it. Loft wakes you if they ask for changes, and closes it when they pass."))
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
    let (slug, n, d, _held) = ticket_of(state, me, args, "review_ticket").await?;
    let role = me.role.as_str();
    let t = d.tickets.iter().find(|t| t.number == n).expect("of_ticket found it").clone();
    if t.state != "review" {
        return Err(format!("#{n} isn't waiting for a review: nothing was posted."));
    }
    match t.verdicts.get(role).map(String::as_str) {
        Some("pending") => {}
        Some(_) => return Err(format!("You already passed #{n} in this round: nothing was posted.")),
        None => return Err(format!("#{n} isn't waiting for a {} review: QA and Security review tickets.", crate::planning::role_label(role))),
    }
    let label = crate::planning::role_label(role);
    let sha = t.commits.last().map(|c| short(c).to_string()).unwrap_or_default();
    let (head, tag) = match (verdict, role) {
        ("pass", "qa") => (format!("**{label}** ({}) passed ticket #{n} at `{sha}`.", me.name), "qa-passed"),
        ("pass", _) => (format!("**{label}** ({}) passed ticket #{n} at `{sha}`.", me.name), "security:passed"),
        _ => (format!("**{label}** ({}) asks for changes on ticket #{n} at `{sha}`.", me.name), "changes-requested"),
    };
    let on = d.pr.unwrap_or(n);
    let body = if notes.is_empty() { head } else { format!("{head}\n\n{notes}") };
    crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{on}/comments"), Some(json!({ "body": body })))
        .await
        .map_err(|e| format!("Couldn't post the verdict on #{on}: {e}. Call review_ticket again."))?;
    let labelled = label_issue(state, &slug, on, tag).await.err().map(|e| format!(" The comment is posted, but {e}.")).unwrap_or_default();

    if verdict == "pass" {
        let d = update(state, &slug, d.spec, |d| {
            if let Some(t) = d.tickets.iter_mut().find(|t| t.number == n) {
                t.verdicts.insert(role.into(), "pass".into());
            }
        })
        .ok_or("The Delivery record couldn't be saved.")?;
        let t = d.tickets.iter().find(|t| t.number == n).expect("just saved");
        if t.verdicts.values().any(|v| v != "pass") {
            return Ok(format!("Posted your pass on #{on}.{labelled} #{n} waits for its other reviewers."));
        }
        let closed = finish(state, &d, n).await?;
        return Ok(format!("Posted your pass on #{on}.{labelled}{closed}"));
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
        let why = needs_human(state, &d, &t, rounds, notes).await;
        return Ok(format!("Posted your changes on #{on}.{labelled} That's round {rounds} of changes on #{n}, so it waits for the user now{why}."));
    }
    let brief = format!(
        "[Loft Delivery: spec #{spec}] This is not a message from the user. {who} ({label}) asked for changes on ticket #{n} \"{title}\" (round {rounds} of {MAX_ROUNDS}):\n\n{notes}\n\n\
         Fix it on the spec branch `{branch}` with new commits, each ending with `Refs #{n}`, push, then call `mcp__loft__submit_ticket` again with all the ticket's commits, oldest first.",
        spec = d.spec,
        who = me.name,
        title = t.title,
        branch = d.branch,
    );
    crate::agents::wake(state, &d.dev, brief, "delivery");
    Ok(format!("Posted your changes on #{on}.{labelled} The Dev is fixing #{n} (round {rounds} of {MAX_ROUNDS})."))
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

/// A ticket after its last changes round: labelled `needs-human` and its Loft issue blocked, which puts it
/// in Needs you and sends a push. Empty, or a note when GitHub refused the label.
async fn needs_human(state: &AppState, d: &Delivery, t: &DeliveryTicket, rounds: u32, notes: &str) -> String {
    let labelled = label_issue(state, &d.repo, t.number, "needs-human").await.err().map(|e| format!(" (and {e})")).unwrap_or_default();
    let mut task = state.db.list_tasks().unwrap_or_default().into_iter().find(|x| x.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(&d.repo) && g.number == t.number));
    if task.is_none() {
        // Not mirrored yet: bring it in now, so it reaches Needs you without waiting for the next poll.
        if let Ok(i) = crate::github::get(state, &format!("/repos/{}/issues/{}", d.repo, t.number)).await {
            task = crate::mirror::bring_in(state, d.project.trim(), &d.repo, &i).map(|(x, _)| x);
        }
    }
    if let Some(mut x) = task {
        x.status = "blocked".into();
        x.error = format!("{rounds} rounds of changes on #{}: it needs you. Last notes: {}", t.number, notes.lines().next().unwrap_or(""));
        x.updated_at = crate::db::now();
        if state.db.save_task(&x).is_ok() {
            crate::tasks::emit(state, &x);
        }
    }
    labelled
}

/// Every reviewer passed ticket #n: closes it on GitHub and starts the next one. A failed close is kept as
/// "closing" and retried on the mirror's next poll. The lines for the reply.
async fn finish(state: &AppState, d: &Delivery, n: i64) -> Result<String, String> {
    let t = d.tickets.iter().find(|t| t.number == n).ok_or("The ticket left the Delivery record.")?;
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
        let started = json!({ "ticket": s.ticket, "agent_id": s.agent.id, "agent_name": s.agent.name, "at": crate::db::now() });
        if card["started"].is_null() {
            state.db.mark_delivery_started(&id, mid, &started)?;
        }
    }
    let m = state.db.get_message(&id, mid)?.ok_or_else(not_found)?;
    state.emit(crate::state::Event::Message { message: json!(m) });
    Ok(Json(json!({ "ok": true, "duplicate": s.already, "message": m })))
}
