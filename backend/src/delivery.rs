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
            d.tickets.push(DeliveryTicket { number, title: title.into(), blocked_by, state: "open".into(), rounds: 0 });
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
         - When the commits are pushed and the checks pass, finish the ticket as your Planning instructions say.",
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
    let t = next_ticket(&d).cloned().ok_or_else(|| format!("Spec #{spec} has no ticket ready to start: every open ticket is blocked, or none was published."))?;
    if d.branch.is_empty() {
        return Err(format!("Spec #{spec} has no spec branch yet: call publish_spec again with the same title to make it."));
    }
    crate::github::request(state, reqwest::Method::POST, &format!("/repos/{repo}/issues/{}/labels", t.number), Some(json!({ "labels": ["in-progress"] })))
        .await
        .map_err(|e| format!("Couldn't label #{} `in-progress`, so the Delivery didn't start: {e}", t.number))?;
    let d = update(state, repo, spec, |d| {
        d.state = "running".into();
        d.current = Some(t.number);
        if let Some(x) = d.tickets.iter_mut().find(|x| x.number == t.number) {
            x.state = "started".into();
        }
    })
    .ok_or("The Delivery record couldn't be saved.")?;
    crate::agents::wake(state, &agent.id, dev_brief(&d, &t), "delivery");
    Ok(Started { ticket: t.number, agent, already: false })
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
