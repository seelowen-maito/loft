//! Publishing a plan as GitHub issues: Loft itself creates them with the user's token, on the repo
//! of the agent's project, and records what it published so a retry never creates a second one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::api::{ApiResult, bad, not_found};
use crate::db::Agent;
use crate::grill::IssueLink;
use crate::state::AppState;

/// An agent's role in its team, as stored in `Agent::role`, with how the user sees it.
pub const ROLES: &[(&str, &str)] = &[("lead", "Lead"), ("dev", "Dev"), ("code_reviewer", "Code reviewer"), ("qa", "QA"), ("security", "Security"), ("custom", "Custom")];

/// The roles that implement a plan: the end-of-planning team check looks for each in the team.
pub const TEAM_ROLES: &[&str] = &["dev", "code_reviewer", "qa", "security"];

pub fn is_role(r: &str) -> bool {
    ROLES.iter().any(|(k, _)| *k == r)
}

/// How the user sees `role`; an unknown one as it is.
pub fn role_label(role: &str) -> &str {
    ROLES.iter().find(|(k, _)| *k == role).map_or(role, |(_, l)| l)
}

/// Labels publishing relies on: name, color, description. Created when the repo lacks them.
const LABELS: &[(&str, &str, &str)] = &[
    ("spec", "5319e7", "What to build, split into tickets"),
    ("ready-for-agent", "0e8a16", "A ticket an agent can pick up"),
];

const MAX_TITLE: usize = 200;
const MAX_TICKETS: usize = 50;

pub fn mcp_tools() -> Vec<Value> {
    vec![json!({
        "name": "publish_spec",
        "description": "Publish the spec as a GitHub issue on your project's repo, labelled `spec` and assigned to the user. Loft creates it with the user's token and creates missing labels. When the project has no GitHub repo (no git, no GitHub `origin`, or GitHub not connected), it is saved as a Loft issue labelled `spec` instead, and nothing goes to GitHub. On GitHub it also creates the spec branch `spec/<N>-<slug>`, from `docs_branch` when given or else with an empty \"Start spec #N\" commit on the default branch, and opens a draft spec PR from it to the default branch, with `Closes #<spec>`; if that fails the spec stays published and the result says why. Calling it again with the same title returns the spec already published instead of creating another and only makes the missing branch or PR, so a retry is safe; a spec branch already there gets `docs_branch` merged in, and the result says if that couldn't be done.",
        "inputSchema": { "type": "object", "properties": {
            "title": { "type": "string", "description": "The spec's title: what gets built, in a few words." },
            "project": { "type": "string", "description": "When you work in several projects: the one this is for (its repo gets it). Defaults to your starred main project." },
            "body": { "type": "string", "description": "The spec in Markdown, following the spec template." },
            "docs_branch": { "type": "string", "description": "In a grill that committed glossary or ADR changes: the docs branch, pushed to GitHub first. The spec branch starts from it, or gets it merged in when it already exists, so the spec PR carries the docs and no Docs PR is opened. Leave it out when there are no docs commits." }
        }, "required": ["title", "body"] }
    }), json!({
        "name": "publish_tickets",
        "description": "Publish tickets as GitHub issues, each on its project's repo (the spec's unless the ticket names another), only after the user approved the breakdown. Loft creates them blockers first as sub-issues of the spec, labelled `ready-for-agent` and assigned to the user, and links each blocker as a GitHub issue dependency. Under a spec with a spec PR, Loft adds a \"## Branch\" section to each ticket naming the spec branch and the spec PR. When the project has no GitHub repo they become Loft issues under the spec instead, with the blockers as dependencies, and nothing goes to GitHub. Without `spec` it publishes one standalone ticket. Calling it again with the same tickets skips what is already published, so a retry after a failure is safe.",
        "inputSchema": { "type": "object", "properties": {
            "spec": { "type": ["integer", "string"], "description": "The spec from publish_spec: its GitHub issue number, or its Loft issue id when the plan is kept in Loft. Leave it out only for a single standalone ticket." },
            "project": { "type": "string", "description": "When you work in several projects: the spec's project. Defaults to your starred main project." },
            "tickets": { "type": "array", "description": "The tickets, in any order.", "items": { "type": "object", "properties": {
                "title": { "type": "string", "description": "What the slice delivers, in a few words." },
                "body": { "type": "string", "description": "The ticket in Markdown, following the ticket template." },
                "project": { "type": "string", "description": "The project whose repo gets this ticket, one of yours: its Repo line. Defaults to the spec's project. On GitHub it stays a sub-issue of the spec and keeps its blockers across repos." },
                "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "Titles of the tickets in this list that must land before this one." }
            }, "required": ["title", "body"] } }
        }, "required": ["tickets"] }
    }), json!({
        "name": "publish_issue",
        "description": "Publish to GitHub: create the GitHub issue for a Loft issue you're on or created, in a project with a GitHub repo, and link them. Only when the user asked for it. Calling it again returns the issue already published.",
        "inputSchema": { "type": "object", "properties": {
            "id": { "type": "string", "description": "The Loft issue's id." }
        }, "required": ["id"] }
    }), json!({
        "name": "planning_guide",
        "description": "The templates and rules for a spec and its tickets. Call it before writing either outside a grill (a grill's instructions already include them).",
        "inputSchema": { "type": "object", "properties": {} }
    })]
}

/// Handle the planning tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    if name == "planning_guide" {
        return Some(Ok(format!("{}\n\n{}", crate::grill::SPEC, crate::grill::TICKETS)));
    }
    if name == "publish_issue" {
        let id = args["id"].as_str().unwrap_or("").trim();
        let mine = state.db.get_task(id).ok().flatten().is_some_and(|t| t.agent_id.as_deref() == Some(&me.id) || t.created_by == me.id);
        if !mine {
            return Some(Err("That Loft issue isn't assigned to you or created by you.".into()));
        }
        return Some(crate::mirror::publish(state, id, &me.id).await.map(|t| {
            let g = t.github.unwrap_or_default();
            format!("Published to GitHub as #{}: {}", g.number, g.url)
        }));
    }
    if name != "publish_spec" && name != "publish_tickets" {
        return None;
    }
    // Everything below plans for one project: `me` as if that were its only one. A ticket can name
    // another of the agent's projects, checked against `all` (the agent as it is).
    let all = me;
    let me = &match for_project(me, args) {
        Ok(a) => a,
        Err(e) => return Some(Err(e)),
    };
    let out = if name == "publish_tickets" {
        publish_tickets(state, all, me, args).await
    } else {
        let docs = args["docs_branch"].as_str().map(str::trim).filter(|d| !d.is_empty());
        publish_spec(state, me, args["title"].as_str().unwrap_or(""), args["body"].as_str().unwrap_or(""), docs).await.map(|(msg, slug, spec)| {
            crate::grill::record_published(state, &me.id, &slug, Some(spec), vec![]);
            msg
        })
    };
    // Even a failure can leave issues published, so clients hear about it either way, in every project
    // a ticket went to.
    let mut projects = vec![me.project.trim().to_string()];
    for t in args["tickets"].as_array().into_iter().flatten() {
        if t["project"].as_str().is_some_and(|p| !p.trim().is_empty())
            && let Ok(a) = for_project(all, t)
            && !projects.contains(&a.project)
        {
            projects.push(a.project);
        }
    }
    for p in projects {
        state.emit(crate::state::Event::Issues { project: p.clone() });
        // What was published shows up as Loft issues without waiting for the next poll.
        crate::mirror::spawn_sync(state, &p);
    }
    Some(out)
}

/// The agent as if `project` were its only project, for planning: the `project` in `args`, which must be
/// one of its projects, or else its starred main project (the first of its projects), never its group.
pub(crate) fn for_project(me: &Agent, args: &Value) -> Result<Agent, String> {
    let own: Vec<String> = if me.projects.is_empty() { Some(me.project.trim().to_string()).filter(|p| !p.is_empty()).into_iter().collect() } else { me.projects.clone() };
    let list = || own.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ");
    let project = match args["project"].as_str().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) if own.iter().any(|o| o == p) => p.to_string(),
        Some(p) if own.is_empty() => return Err(format!("You have no projects, so you can't plan for {p}.")),
        Some(p) => return Err(format!("You don't work in {p}. Pass `project` as one of your projects: {}.", list())),
        None => own.first().cloned().unwrap_or_default(),
    };
    let mut a = me.clone();
    a.project = project;
    Ok(a)
}

/// `POST /api/agents/{id}/messages/{mid}/team-setup`: the end-of-planning card's "Set up the team with
/// loft". Sends the card's setup message to loft once and records `sent_at` on the card, so it shows as
/// sent after a reload and a second click (or another device) gets the card back instead of a resend.
pub async fn send_team_setup(State(state): State<AppState>, Path((id, mid)): Path<(String, i64)>) -> ApiResult<Value> {
    let m = state.db.get_message(&id, mid)?.ok_or_else(not_found)?;
    let setup = m.meta["grill"]["end_card"]["setup"].clone();
    let (Some(to), Some(text)) = (setup["agent_id"].as_str(), setup["message"].as_str()) else {
        return Err(bad("This card has no team setup."));
    };
    if !state.db.mark_team_setup_sent(&id, mid)? {
        return Ok(Json(json!({ "ok": true, "duplicate": true, "message": m })));
    }
    crate::agents::enqueue_with(&state, to, Some(text.to_string()), json!({}), "user");
    let m = state.db.get_message(&id, mid)?.ok_or_else(not_found)?;
    state.emit(crate::state::Event::Message { message: json!(m) });
    Ok(Json(json!({ "ok": true, "message": m })))
}

/// The team check: the roles no agent of `team` (a group) fills, going by their stored roles.
pub fn missing_roles(state: &AppState, team: &str) -> Vec<&'static str> {
    let name = team.trim();
    let team: Vec<String> = state.db.list_agents().unwrap_or_default().into_iter().filter(|a| !name.is_empty() && a.team() == name).map(|a| a.role).collect();
    TEAM_ROLES.iter().copied().filter(|r| !team.iter().any(|t| t == r)).collect()
}

/// The words of an agent's name and title, lowercased.
fn words(name: &str, title: &str) -> Vec<String> {
    format!("{name} {title}").to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// Which reviewer an agent with the old "reviewer" role is, from its name and title: QA or test → QA,
/// security → Security, anything else → Code reviewer.
pub fn reviewer_role(name: &str, title: &str) -> &'static str {
    let words = words(name, title);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    if has(&["qa", "test", "tests", "tester", "testing"]) {
        "qa"
    } else if has(&["security", "seguranca", "segurança"]) {
        "security"
    } else {
        "code_reviewer"
    }
}

/// A role for an agent saved before roles existed, from the words in its name and title. Reviewing
/// words win, so a "QA engineer" is QA.
pub fn infer_role(name: &str, title: &str) -> &'static str {
    let words = words(name, title);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    if has(&["reviewer", "review", "revisor", "revisora", "qa", "test", "tests", "tester", "testing", "auditor", "security"]) {
        reviewer_role(name, title)
    } else if has(&["dev", "developer", "engineer", "programmer", "coder", "desenvolvedor", "desenvolvedora"]) {
        "dev"
    } else {
        "custom"
    }
}

/// The role an agent stored before the six roles gets, from its raw JSON: the `lead` flag becomes Lead,
/// the old "reviewer" is split by name and title, "dev" stays Dev, no role becomes Custom, and an agent
/// from before roles has one inferred. None when it already has one of the six and no `lead` flag.
pub fn migrated_role(v: &Value) -> Option<&'static str> {
    let role = v.get("role").map(|r| r.as_str().unwrap_or(""));
    if role.is_some_and(is_role) && v.get("lead").is_none() {
        return None;
    }
    let (name, title) = (v["name"].as_str().unwrap_or(""), v["title"].as_str().unwrap_or(""));
    Some(match role {
        _ if v["lead"] == true => "lead",
        None => infer_role(name, title),
        Some("reviewer") => reviewer_role(name, title),
        Some(r) => ROLES.iter().map(|(k, _)| *k).find(|k| *k == r).unwrap_or("custom"),
    })
}

/// One lock per repo, held for a whole publish: the check for what is already published and the
/// create are several awaits apart, so two overlapping calls (two planners on one project, parallel
/// tool calls, a retry while a slow run is still going) would otherwise both create the same issue.
#[derive(Default)]
pub struct Locks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl Locks {
    pub(crate) fn of(&self, slug: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).entry(slug.into()).or_default().clone()
    }
}

/// Lookups every publish and retry would otherwise repeat: the login of each token, and the repos
/// whose labels are known to exist.
#[derive(Default)]
pub struct Known {
    logins: Mutex<HashMap<String, String>>,
    labelled: Mutex<std::collections::HashSet<String>>,
}

/// Same spec, same key: case and spacing don't make a new one.
pub(crate) fn key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Where a plan goes: `owner/repo` from the `origin` of the agent's project folder, with GitHub
/// connected. Err says why it stays in Loft instead: no project, no git, no GitHub remote, no token.
pub(crate) async fn github_home(state: &AppState, a: &Agent) -> Result<String, String> {
    if a.project.trim().is_empty() {
        return Err("you have no project".into());
    }
    let slug = crate::repos::github_repo(state, &a.project).await?;
    if crate::github::token(state).is_none() {
        return Err("GitHub isn't connected (no token in Settings, no `gh auth login`)".into());
    }
    Ok(slug)
}

/// Publishing for a project with no GitHub repo: Loft issues in the agent's project, with no GitHub calls.
mod in_loft {
    use super::*;
    use crate::db::Task;

    fn in_project(state: &AppState, me: &Agent) -> Vec<Task> {
        state.db.list_tasks().unwrap_or_default().into_iter().filter(|t| t.project == me.project.trim() && t.github.is_none() && t.status != "cancelled").collect()
    }

    fn make(state: &AppState, me: &Agent, p: Value) -> Result<Task, String> {
        crate::tasks::create(state, &p, &me.id).map(|(t, _)| t)
    }

    /// The spec as a Loft issue labelled `spec`; the same title again returns the one already saved.
    pub(super) async fn spec(state: &AppState, me: &Agent, title: &str, body: &str, why: &str) -> Result<(String, String, IssueLink), String> {
        let project = me.project.trim().to_string();
        let _held = state.publish_locks.of(&format!("loft:{project}")).lock_owned().await;
        let k = key(title);
        let found = in_project(state, me).into_iter().find(|t| t.labels.iter().any(|l| l == "spec") && key(&t.title) == k);
        let (t, msg) = match found {
            Some(t) => {
                let msg = format!("The spec \"{title}\" is already saved as Loft issue {}. Nothing new was created and its text wasn't changed.", t.id);
                (t, msg)
            }
            None => {
                let t = make(state, me, json!({ "title": title, "description": body.trim(), "labels": ["spec"], "project": project, "status": "inbox" }))?;
                let msg = format!("Saved the spec as a Loft issue labelled `spec`, id {}: {why}, so the plan stays in Loft and nothing went to GitHub. Pass `spec: \"{}\"` to publish_tickets.", t.id, t.id);
                (t, msg)
            }
        };
        crate::delivery::record_local_spec(state, &project, &t.id, &t.title, &me.id);
        Ok((msg, String::new(), IssueLink { number: 0, title: t.title.clone(), id: t.id.clone(), repo: String::new() }))
    }

    /// The tickets as Loft issues labelled `ready-for-agent`: subtasks of the spec, blockers first,
    /// each depending on the tickets that block it. Ones already saved under the spec are skipped.
    pub(super) async fn tickets(state: &AppState, me: &Agent, spec: Option<&str>, ts: &[Ticket], order: &[usize], why: &str) -> Result<String, String> {
        let project = me.project.trim().to_string();
        let spec = match spec {
            Some(id) => {
                let s = state.db.get_task(id).map_err(|e| e.to_string())?.filter(|s| s.project == project && s.labels.iter().any(|l| l == "spec"));
                Some(s.ok_or_else(|| format!("There's no spec with the Loft issue id {id} in your project: pass the id publish_spec gave."))?)
            }
            None => None,
        };
        let _held = state.publish_locks.of(&format!("loft:{project}")).lock_owned().await;
        // A spec's tickets are under it in whichever project they went to.
        let siblings: Vec<Task> = match &spec {
            Some(s) => state.db.list_tasks().unwrap_or_default().into_iter().filter(|t| t.parent_id.as_deref() == Some(&s.id) && t.github.is_none() && t.status != "cancelled").collect(),
            None => in_project(state, me).into_iter().filter(|t| t.parent_id.is_none() && t.state() == "open").collect(),
        };
        let siblings: Vec<Task> = siblings.into_iter().filter(|t| t.labels.iter().any(|l| l == "ready-for-agent")).collect();
        let mut ids: Vec<Option<String>> = vec![None; ts.len()];
        let mut lines = vec![];
        let mut fresh = 0;
        for (pos, &i) in order.iter().enumerate() {
            let t = &ts[i];
            let blockers: Vec<String> = t.blocked_by.iter().map(|&b| ids[b].clone().expect("blockers are saved first")).collect();
            let (made, new) = match siblings.iter().find(|s| key(&s.title) == key(&t.title)) {
                Some(s) => (s.clone(), false),
                None => {
                    let made = make(state, me, json!({
                        "title": t.title, "description": t.body, "labels": ["ready-for-agent"], "project": t.home.project.trim(), "status": "ready",
                        "parent_id": spec.as_ref().map(|s| s.id.clone()), "depends_on": blockers,
                    }))?;
                    fresh += 1;
                    (made, true)
                }
            };
            if let Some(s) = &spec {
                crate::delivery::record_local_ticket(state, &project, &s.id, &made.id, &t.title, &blockers);
            }
            let spec_link = spec.as_ref().map(|s| IssueLink { number: 0, title: s.title.clone(), id: s.id.clone(), repo: String::new() });
            crate::grill::record_published(state, &me.id, "", spec_link, vec![IssueLink { number: pos as i64 + 1, title: t.title.clone(), id: made.id.clone(), repo: String::new() }]);
            let by: Vec<String> = t.blocked_by.iter().map(|&b| format!("\"{}\"", ts[b].title)).collect();
            let by = if by.is_empty() { String::new() } else { format!(", blocked by {}", by.join(", ")) };
            let was = if new { "" } else { ", already saved" };
            lines.push(format!("- {} (Loft issue {}){by}{was}", t.title, made.id));
            ids[i] = Some(made.id);
        }
        let head = match &spec {
            Some(s) => format!("Saved {} ticket{} as Loft issues under the spec \"{}\", blockers first", ts.len(), if ts.len() == 1 { "" } else { "s" }, s.title),
            None => "Saved a standalone ticket as a Loft issue".to_string(),
        };
        let none_new = if fresh == 0 { "\nNothing new was created: every ticket was already saved." } else { "" };
        Ok(format!("{head}, labelled `ready-for-agent`: {why}, so nothing went to GitHub.\n{}{none_new}", lines.join("\n")))
    }
}

/// Creates the labels the repo doesn't have yet; existing ones are left as they are.
/// Creates label `name` on `slug` unless Loft already made sure of it there; one GitHub says exists counts.
pub(crate) async fn ensure_label(state: &AppState, slug: &str, name: &str, color: &str, description: &str) -> Result<(), String> {
    let key = format!("{slug} {name}");
    if state.publish_known.labelled.lock().unwrap_or_else(|e| e.into_inner()).contains(&key) {
        return Ok(());
    }
    let made = crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/labels"), Some(json!({ "name": name, "color": color, "description": description }))).await;
    match made {
        Err(e) if !e.to_string().contains("already_exists") => return Err(format!("couldn't create the `{name}` label on {slug}: {e}")),
        _ => {}
    }
    state.publish_known.labelled.lock().unwrap_or_else(|e| e.into_inner()).insert(key);
    Ok(())
}

async fn ensure_labels(state: &AppState, slug: &str) -> Result<(), String> {
    if state.publish_known.labelled.lock().unwrap_or_else(|e| e.into_inner()).contains(slug) {
        return Ok(());
    }
    let have = crate::github::get(state, &format!("/repos/{slug}/labels?per_page=100")).await.map_err(|e| format!("Couldn't read the labels of {slug}: {e}"))?;
    let have: Vec<String> = have.as_array().map(|ls| ls.iter().filter_map(|l| l["name"].as_str()).map(str::to_lowercase).collect()).unwrap_or_default();
    for (name, color, description) in LABELS {
        if have.iter().any(|h| h == name) {
            continue;
        }
        let made = crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/labels"), Some(json!({ "name": name, "color": color, "description": description }))).await;
        match made {
            // Created meanwhile, or beyond the first page of labels: it exists, which is all we need.
            Err(e) if e.to_string().contains("already_exists") => {}
            Err(e) => return Err(format!("Couldn't create the `{name}` label on {slug}: {e}")),
            Ok(_) => {}
        }
    }
    state.publish_known.labelled.lock().unwrap_or_else(|e| e.into_inner()).insert(slug.into());
    Ok(())
}

/// The login of the token's user: everything Loft publishes is assigned to them.
pub(crate) async fn login(state: &AppState) -> Result<String, String> {
    let token = crate::github::token(state).unwrap_or_default();
    if let Some(l) = state.publish_known.logins.lock().unwrap_or_else(|e| e.into_inner()).get(&token) {
        return Ok(l.clone());
    }
    let me_gh = crate::github::get(state, "/user").await.map_err(|e| format!("Couldn't read the GitHub user of Loft's token: {e}"))?;
    let l = me_gh["login"].as_str().ok_or("GitHub didn't say who the token belongs to.")?.to_string();
    state.publish_known.logins.lock().unwrap_or_else(|e| e.into_inner()).insert(token, l.clone());
    Ok(l)
}

/// The reply, the repo and the spec published (or already there). On GitHub the reply also gives the
/// spec branch and spec PR, or says why they couldn't be made: the spec stays published either way.
async fn publish_spec(state: &AppState, me: &Agent, title: &str, body: &str, docs: Option<&str>) -> Result<(String, String, IssueLink), String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || body.trim().is_empty() {
        return Err("title and body are required".into());
    }
    if let Some(d) = docs.filter(|d| !is_branch_name(d)) {
        return Err(format!("\"{d}\" isn't a branch name: pass the docs branch as you pushed it, like `docs/<topic>`"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(format!("the title is at most {MAX_TITLE} characters"));
    }
    let slug = match github_home(state, me).await {
        Ok(slug) => slug,
        Err(why) => return in_loft::spec(state, me, &title, body, &why).await,
    };
    let _held = state.publish_locks.of(&slug).lock_owned().await;
    let k = key(&title);
    let (msg, n) = spec_issue(state, me, &slug, &title, &k, body).await?;
    crate::delivery::record_spec(state, &slug, n, me.project.trim(), &me.id, &spec_branch(n, title.as_str()));
    let pr = match spec_pr(state, &slug, n, &title, docs).await {
        Ok(line) => line,
        Err(e) => {
            let same = if docs.is_some() { " and `docs_branch`" } else { "" };
            format!("The spec is published, but {e}. Call publish_spec again with the same title{same} to retry only the spec branch and PR.")
        }
    };
    Ok((format!("{msg}\n{pr}"), slug, IssueLink { number: n, title, id: String::new(), repo: String::new() }))
}

/// The spec's issue on `slug`: the reply and its number, published now or found already there.
async fn spec_issue(state: &AppState, me: &Agent, slug: &str, title: &str, k: &str, body: &str) -> Result<(String, i64), String> {
    let already = |n: i64| (format!("The spec \"{title}\" is already published as #{n}: https://github.com/{slug}/issues/{n}. Nothing new was created and its text wasn't changed."), n);
    if let Ok(Some(n)) = state.db.published_issue(slug, "spec", k) {
        return Ok(already(n));
    }
    let login = login(state).await?;
    ensure_labels(state, slug).await?;
    // Created before but never recorded (the reply was lost): adopt it rather than publish it twice.
    let existing = crate::github::get(state, &format!("/repos/{slug}/issues?labels=spec&state=all&per_page=100"))
        .await
        .map_err(|e| format!("Couldn't check {slug} for an existing spec: {e}"))?;
    let found = existing.as_array().into_iter().flatten().find(|i| i.get("pull_request").is_none() && i["title"].as_str().is_some_and(|t| key(t) == k));
    if let Some(n) = found.and_then(|i| i["number"].as_i64()) {
        let _ = state.db.record_published_issue(slug, "spec", k, n);
        return Ok(already(n));
    }
    let made = crate::github::request(
        state,
        reqwest::Method::POST,
        &format!("/repos/{slug}/issues"),
        Some(json!({ "title": title, "body": body.trim(), "labels": ["spec"], "assignees": [login] })),
    )
    .await
    .map_err(|e| format!("GitHub rejected the spec on {slug}: {e}"))?;
    let n = made["number"].as_i64().ok_or("GitHub created the spec but didn't return its number.")?;
    state.db.record_published_issue(slug, "spec", k, n).map_err(|e| e.to_string())?;
    crate::mirror::bring_in(state, me.project.trim(), slug, &made);
    Ok((format!("Published the spec as #{n}, assigned to {login}: https://github.com/{slug}/issues/{n}"), n))
}

/// The spec branch of spec #n: `spec/<N>-<slug>`, the slug from the first words of its title.
pub(crate) fn spec_branch(n: i64, title: &str) -> String {
    branch_of("spec", n, title)
}

/// A standalone ticket's branch: `ticket/<N>-<slug>`.
pub(crate) fn ticket_branch(n: i64, title: &str) -> String {
    branch_of("ticket", n, title)
}

/// `<kind>/<N>-<slug>`, the slug from the first words of the title.
fn branch_of(kind: &str, n: i64, title: &str) -> String {
    let mut slug = String::new();
    for w in title.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        if !slug.is_empty() && slug.len() + w.len() >= 40 {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w.to_lowercase());
    }
    if slug.is_empty() { format!("{kind}/{n}") } else { format!("{kind}/{n}-{slug}") }
}

/// A branch name as git takes it: letters, digits and `-_./`, no `..`, not starting or ending with `/`.
fn is_branch_name(b: &str) -> bool {
    !b.is_empty() && b.len() <= 200 && b.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) && !b.contains("..") && !b.starts_with('/') && !b.ends_with('/') && !b.ends_with(".lock")
}

/// Makes sure spec #n has its spec branch and a draft spec PR to the default branch, all through the
/// GitHub API. The branch starts at the head of the grill's docs branch when there is one, so the
/// docs commits ship in the spec PR; otherwise at an empty commit on the default branch. Whatever is already there (recorded, or found on GitHub) is kept, so a retry only makes
/// what's missing; a spec branch already there gets the docs branch merged in instead. The PR is recorded on the spec's Delivery, for its tickets to find. Ok is the
/// reply's line naming both; Err says what failed.
async fn spec_pr(state: &AppState, slug: &str, n: i64, title: &str, docs: Option<&str>) -> Result<String, String> {
    let branch = spec_branch(n, title);
    let line = |pr: i64| format!("Spec branch: `{branch}`. Spec PR (draft, to the default branch): https://github.com/{slug}/pull/{pr}");
    if let Some(pr) = state.db.get_delivery(slug, n).ok().flatten().and_then(|d| d.pr) {
        return Ok(format!("{}{}", line(pr), docs_onto(state, slug, &branch, docs).await));
    }
    let (pr, docs_note) = branch_and_pr(state, slug, n, title, "spec", docs).await?;
    Ok(format!("{}{docs_note}", line(pr)))
}

/// Makes issue #n's `<kind>/<N>-<slug>` branch (a spec's, or a standalone ticket's) and its draft PR to
/// the default branch with `Closes #n`, keeping whatever is already there, and records the PR on the
/// Delivery of #n. The PR and a line about the docs branch (see [`spec_pr`]).
pub(crate) async fn branch_and_pr(state: &AppState, slug: &str, n: i64, title: &str, kind: &str, docs: Option<&str>) -> Result<(i64, String), String> {
    let branch = branch_of(kind, n, title);
    let gh = |path: String| async move { crate::github::get(state, &format!("/repos/{slug}{path}")).await };
    let post = |path: &'static str, body: Value| async move { crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}{path}"), Some(body)).await };
    let repo = gh(String::new()).await.map_err(|e| format!("couldn't read {slug} for its default branch: {e}"))?;
    let base = repo["default_branch"].as_str().filter(|b| !b.is_empty()).ok_or_else(|| format!("GitHub didn't give the default branch of {slug}"))?.to_string();
    let mut docs_note = String::new();
    match gh(format!("/git/ref/heads/{branch}")).await {
        Ok(_) => docs_note = docs_onto(state, slug, &branch, docs).await,
        Err(e) if crate::github::is_gone(&e) => {
            let failed = |e: anyhow::Error| format!("the {kind} branch `{branch}` couldn't be created: {e}");
            let start = match docs {
                Some(d) => {
                    let head = gh(format!("/git/ref/heads/{d}")).await.map_err(|e| {
                        if crate::github::is_gone(&e) { format!("the docs branch `{d}` isn't on GitHub, so the {kind} branch `{branch}` wasn't created: push it first") } else { failed(e) }
                    })?;
                    head["object"]["sha"].clone()
                }
                None => {
                    let head = gh(format!("/git/ref/heads/{base}")).await.map_err(failed)?;
                    let sha = head["object"]["sha"].as_str().unwrap_or_default().to_string();
                    let commit = gh(format!("/git/commits/{sha}")).await.map_err(failed)?;
                    // An empty commit: the tree of the default branch, so the branch has something to open a PR for.
                    let made = post("/git/commits", json!({ "message": format!("Start {kind} #{n}"), "tree": commit["tree"]["sha"], "parents": [sha] })).await.map_err(failed)?;
                    made["sha"].clone()
                }
            };
            post("/git/refs", json!({ "ref": format!("refs/heads/{branch}"), "sha": start })).await.map_err(failed)?;
        }
        Err(e) => return Err(format!("couldn't check {slug} for the {kind} branch `{branch}`: {e}")),
    }
    // Opened before but never recorded: adopt it rather than open a second one.
    let owner = slug.split('/').next().unwrap_or_default();
    let open = gh(format!("/pulls?head={owner}:{branch}&state=all")).await.map_err(|e| format!("couldn't check {slug} for the {kind} PR: {e}"))?;
    let pr = match open.as_array().and_then(|ps| ps.first()).and_then(|p| p["number"].as_i64()) {
        Some(pr) => pr,
        None => {
            let made = post("/pulls", json!({ "title": title, "head": branch, "base": base, "body": format!("Closes #{n}"), "draft": true }))
                .await
                .map_err(|e| format!("the draft {kind} PR from `{branch}` couldn't be opened: {e}"))?;
            made["number"].as_i64().ok_or_else(|| format!("GitHub opened the {kind} PR but didn't return its number"))?
        }
    };
    if let Some(d) = crate::delivery::update(state, slug, n, |d| d.pr = Some(pr)) {
        crate::delivery::track_pr(state, &d).await;
    }
    Ok((pr, docs_note))
}

/// Merges the grill's docs branch into a spec branch made before it (a retry, say), so its glossary
/// and ADR commits still ship in the spec PR. Empty when there's no docs branch or the spec branch
/// already has them; otherwise a line for the reply saying what happened.
async fn docs_onto(state: &AppState, slug: &str, branch: &str, docs: Option<&str>) -> String {
    let Some(d) = docs else { return String::new() };
    let body = json!({ "base": branch, "head": d, "commit_message": format!("Merge the grill's docs from {d}") });
    match crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/merges"), Some(body)).await {
        // 204: nothing to merge.
        Ok(Value::Null) => String::new(),
        Ok(v) => format!("\nThe spec branch was already there: merged the docs branch `{d}` into it ({}).", v["sha"].as_str().map(|s| &s[..s.len().min(7)]).unwrap_or("merge commit")),
        Err(e) if crate::github::is_gone(&e) => format!("\nThe docs branch `{d}` isn't on GitHub, so its commits aren't on `{branch}`: push it, then call publish_spec again with the same title and `docs_branch`."),
        Err(e) if e.to_string().starts_with("GitHub 409") => format!("\nThe docs branch `{d}` conflicts with `{branch}`, so its commits aren't on it: merge `{d}` into `{branch}` by hand."),
        Err(e) => format!("\nThe docs branch `{d}` couldn't be merged into `{branch}`: {e}. Call publish_spec again with the same title and `docs_branch` to retry."),
    }
}

/// The "## Branch" section of a ticket under spec #spec, when the spec has a spec PR: where to commit
/// and where it ships, from its Delivery. None for a spec without one (published before spec PRs, or
/// its PR failed). Only a Delivery migrated from before branches were recorded reads its branch from GitHub, once.
async fn branch_section(state: &AppState, slug: &str, spec: i64) -> Result<Option<String>, String> {
    let Some(d) = state.db.get_delivery(slug, spec).ok().flatten() else { return Ok(None) };
    let Some(pr) = d.pr else { return Ok(None) };
    let branch = if d.branch.is_empty() {
        let p = crate::github::get(state, &format!("/repos/{slug}/pulls/{pr}")).await.map_err(|e| format!("Couldn't read spec PR #{pr} on {slug} for its branch: {e}"))?;
        let b = p["head"]["ref"].as_str().ok_or_else(|| format!("GitHub didn't give the branch of spec PR #{pr} on {slug}"))?.to_string();
        crate::delivery::update(state, slug, spec, |d| d.branch = b.clone());
        b
    } else {
        d.branch
    };
    Ok(Some(format!("## Branch\n\nCommit on the spec branch `{branch}`. It ships in the spec PR https://github.com/{slug}/pull/{pr}.")))
}

/// The ticket's body with this "## Branch" section at the end, in place of one the body already had.
fn with_branch(body: &str, section: &str) -> String {
    let mut kept = vec![];
    let mut skipping = false;
    for line in body.lines() {
        if line.starts_with("## ") {
            skipping = line.trim_end() == "## Branch";
        }
        if !skipping {
            kept.push(line);
        }
    }
    format!("{}\n\n{section}", kept.join("\n").trim_end())
}

/// The spec PR a ticket's "## Branch" section links to, if it has one.
fn spec_pr_in(body: &str, slug: &str) -> Option<i64> {
    let at = format!("https://github.com/{slug}/pull/");
    let section = body.lines().skip_while(|l| l.trim_end() != "## Branch").skip(1).take_while(|l| !l.starts_with("## "));
    section.filter_map(|l| l.split_once(&at)).find_map(|(_, rest)| rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
}

/// The commit SHAs in `args["commits"]`, lowercased: each 7 to 40 hex characters.
pub(crate) fn commits_in(args: &Value) -> Result<Vec<String>, String> {
    let commits: Vec<String> = args["commits"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).map(|c| c.trim().to_lowercase()).collect();
    if commits.is_empty() {
        return Err("commits is required: the SHAs of the ticket's commits".into());
    }
    if let Some(bad) = commits.iter().find(|c| !(7..=40).contains(&c.len()) || !c.chars().all(|ch| ch.is_ascii_hexdigit())) {
        return Err(format!("\"{bad}\" isn't a commit SHA: give 7 to 40 hex characters"));
    }
    Ok(commits)
}

/// Closes ticket #n on `slug` once its reviewers passed it: comments "Done in `<sha…>`, part of #<spec PR>",
/// closes it as completed so the tickets it blocks can start, and marks its Loft issue (in `project`) done,
/// logged as `by`'s work. Then marks the spec PR ready when it was the spec's last ticket. The caller holds
/// the repo's publish lock. Ok is the line saying what was done.
pub(crate) async fn close_ticket(state: &AppState, project: &str, by: &Agent, slug: &str, n: i64, commits: &[String]) -> Result<String, String> {
    let slug = slug.to_string();
    let i = crate::github::get(state, &format!("/repos/{slug}/issues/{n}")).await.map_err(|e| format!("Couldn't read #{n} on {slug}: {e}"))?;
    let is_ticket = i.get("pull_request").is_none() && i["labels"].as_array().into_iter().flatten().any(|l| l["name"] == "ready-for-agent");
    if !is_ticket {
        return Err(format!("#{n} on {slug} isn't a ticket (an issue labelled `ready-for-agent`), so it wasn't closed."));
    }
    if i["state"] != "open" {
        return Err(format!("#{n} on {slug} is already closed: nothing was done."));
    }
    let pr = crate::delivery::of_ticket(state, &slug, n).and_then(|d| d.pr).or_else(|| spec_pr_in(i["body"].as_str().unwrap_or(""), &slug));
    let shas: Vec<String> = commits.iter().map(|c| format!("`{}`", &c[..7])).collect();
    let part = pr.map(|p| format!(", part of #{p}")).unwrap_or_default();
    let comment = format!("Done in {}{part}.", shas.join(", "));
    // Posted this same comment but the close failed: the retry doesn't comment twice. Keyed by the
    // comment so the reply always names a comment that is on the ticket. A record older than the
    // ticket's last close (by hand on GitHub, then reopened) belongs to an earlier round.
    let key = format!("{n} {comment}");
    let rfc = |t: &str| chrono::DateTime::parse_from_rfc3339(t).ok();
    let last_closed = i["closed_at"].as_str().and_then(rfc);
    let posted = state.db.published_issue_at(&slug, "close_comment", &key).ok().flatten().and_then(|t| rfc(&t));
    if !posted.is_some_and(|p| last_closed.is_none_or(|c| p > c)) {
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{n}/comments"), Some(json!({ "body": comment })))
            .await
            .map_err(|e| format!("Couldn't comment on #{n}, so it wasn't closed: {e}"))?;
        let _ = state.db.record_published_issue(&slug, "close_comment", &key, n);
    }
    let closed = crate::github::request(state, reqwest::Method::PATCH, &format!("/repos/{slug}/issues/{n}"), Some(json!({ "state": "closed", "state_reason": "completed" })))
        .await
        .map_err(|e| format!("Commented on #{n} but couldn't close it: {e}. Loft tries the close again on its next poll."))?;
    // Closed: a close after a reopen comments again, whichever comments the failed tries left.
    let _ = state.db.forget_published_issues_of(&slug, "close_comment", n);
    // The mirror leaves an issue someone is working on open when GitHub closes it; this close is the work's end.
    if let Some((mut t, _)) = crate::mirror::bring_in(state, project.trim(), &slug, &closed)
        && t.status != "done"
    {
        crate::tasks::set_status(state, &mut t, "done", &by.id, Some(&format!("Closed by Loft: {}'s work passed review", by.name)));
    }
    state.emit(crate::state::Event::Issues { project: project.trim().to_string() });
    let ready = match ready_when_last(state, &slug, n, i["body"].as_str().unwrap_or("")).await {
        Ok(line) => line.unwrap_or_default(),
        Err(e) => format!("\nThe ticket is closed, but {e}."),
    };
    Ok(format!("Closed #{n} as completed with the comment \"{comment}\": https://github.com/{slug}/issues/{n}{ready}"))
}

/// After ticket #number closed: notes it on its Delivery (found by the ticket, or for a ticket the
/// record doesn't list, by the spec PR its "## Branch" section links to), then moves the Delivery on
/// when it was the spec's last ticket ([`after_tickets`]). Some(line) for the reply when it did. The
/// caller holds the repo's publish lock.
pub(crate) async fn ready_when_last(state: &AppState, slug: &str, number: i64, ticket_body: &str) -> Result<Option<String>, String> {
    let d = crate::delivery::ticket_closed(state, slug, number).or_else(|| {
        let pr = spec_pr_in(ticket_body, slug)?;
        state.db.deliveries(slug, false).ok()?.into_iter().find(|d| d.pr == Some(pr))
    });
    match d {
        Some(d) if !waits(&d) => after_tickets(state, &d).await,
        _ => Ok(None),
    }
}

/// A Delivery past its tickets, or waiting on someone: nothing to move on.
fn waits(d: &crate::db::Delivery) -> bool {
    matches!(d.state.as_str(), "ready" | "merged" | "settled" | "final_review" | "final_fixes" | "needs_human" | "stalled" | "paused")
}

/// The mirror's catch-up for ready calls that were missed: on every poll the Deliveries whose call
/// failed, and on a full read every one not settled yet (Loft may have been down when the last ticket
/// closed). A Delivery whose record still has an open ticket costs no GitHub read (#197), and neither
/// does one with no tickets yet (a spec published before its tickets) on a full read: a ticket closing on
/// it still moves it on. The caller holds the repo's publish lock.
pub(crate) async fn retry_ready(state: &AppState, slug: &str, full: bool) {
    let open = state.db.deliveries(slug, true).unwrap_or_default();
    let unticketed = |d: &crate::db::Delivery| !d.migrated && d.tickets.is_empty();
    // A final review that passed but whose ready call failed waits on nobody but this retry.
    let retried_pass = |d: &crate::db::Delivery| d.ready_retry && d.state == "final_review";
    for d in open.into_iter().filter(|d| d.pr.is_some() && (d.ready_retry || full && !unticketed(d)) && (!waits(d) || retried_pass(d))) {
        match after_tickets(state, &d).await {
            Ok(Some(line)) => tracing::info!("mirror {slug}: a missed call caught up: {}", line.trim()),
            Ok(None) => {}
            Err(e) => tracing::warn!("mirror {slug}: {e}"),
        }
    }
}

/// When every ticket of the Delivery is closed: a Delivery Loft ran goes to the team's Code reviewer for
/// one review of the whole spec PR, and otherwise (no Code reviewer, or tickets closed outside a
/// Delivery) its spec PR is marked ready for review. The record says whether the tickets are closed; only
/// one migrated from before Deliveries asks GitHub. Some(line) for the reply.
pub(crate) async fn after_tickets(state: &AppState, d: &crate::db::Delivery) -> Result<Option<String>, String> {
    if !d.migrated && d.tickets.iter().any(|t| t.state != "closed") {
        return Ok(None);
    }
    if d.state == "running" && !d.tickets.is_empty()
        && let Some(line) = crate::delivery::begin_final_review(state, d)
    {
        return Ok(Some(line));
    }
    if d.local {
        return Ok(Some(format!("\nIt was the spec's last open ticket.{}", crate::delivery::local_ready(state, d))));
    }
    let slug = &d.repo;
    Ok(ready_if_done(state, d).await?.map(|pr| format!("\nIt was the spec's last open ticket: spec PR #{pr} is now ready for review: https://github.com/{slug}/pull/{pr}")))
}

/// [`mark_ready`], noting a call that failed for the mirror to retry on its next poll; a PR found ready,
/// merged or closed is settled and never asked about again.
pub(crate) async fn ready_if_done(state: &AppState, d: &crate::db::Delivery) -> Result<Option<i64>, String> {
    let out = mark_ready(state, d).await;
    let failed = out.is_err();
    crate::delivery::update(state, &d.repo, d.spec, |d| d.ready_retry = failed);
    out
}

/// Marks the Delivery's spec PR ready for review when it's still an open draft, and has Loft watch it, so
/// it reaches Needs you (with a push) once it can be merged. The caller knows every ticket is closed; a
/// Delivery migrated from before Deliveries (or with no tickets listed) checks the spec's sub-issues on
/// GitHub instead.
async fn mark_ready(state: &AppState, d: &crate::db::Delivery) -> Result<Option<i64>, String> {
    let (slug, spec) = (d.repo.as_str(), d.spec);
    let Some(pr) = d.pr else { return Ok(None) };
    let settle = |to: &str| {
        let to = to.to_string();
        crate::delivery::update(state, slug, spec, |d| d.state = to);
    };
    let p = crate::github::get(state, &format!("/repos/{slug}/pulls/{pr}")).await.map_err(|e| format!("couldn't read spec PR #{pr}: {e}"))?;
    if p["state"] != "open" || p["draft"] != true {
        settle("settled");
        return Ok(None);
    }
    if d.migrated || d.tickets.is_empty() {
        let subs = crate::github::get(state, &format!("/repos/{slug}/issues/{spec}/sub_issues?per_page=100")).await.map_err(|e| format!("couldn't read the tickets of spec #{spec}: {e}"))?;
        let subs = subs.as_array().cloned().unwrap_or_default();
        if subs.is_empty() || subs.iter().any(|s| s["state"] != "closed") {
            return Ok(None);
        }
    }
    let query = "mutation($id: ID!) { markPullRequestReadyForReview(input: { pullRequestId: $id }) { pullRequest { isDraft } } }";
    let v = crate::github::request(state, reqwest::Method::POST, "/graphql", Some(json!({ "query": query, "variables": { "id": p["node_id"] } })))
        .await
        .map_err(|e| format!("couldn't mark spec PR #{pr} ready for review: {e}"))?;
    if let Some(e) = v["errors"].as_array().and_then(|es| es.first()) {
        return Err(format!("couldn't mark spec PR #{pr} ready for review: {}", e["message"].as_str().unwrap_or("GitHub refused")));
    }
    settle("ready");
    // Watched like a PR an agent opened: Needs you lists it, with a push and a Merge button, once GitHub
    // says it can be merged.
    crate::delivery::track_pr(state, d).await;
    // Done now, whatever GitHub says of its checks or conflicts.
    crate::delivery::done(state, slug, spec);
    Ok(Some(pr))
}

struct Ticket {
    title: String,
    body: String,
    /// Positions in the list of the tickets that block this one.
    blocked_by: Vec<usize>,
    /// The planner as if the ticket's project were its only one: the `project` the ticket names, one of
    /// the planner's own (`all`), else the spec's (`me`).
    home: Agent,
}

fn tickets_in(all: &Agent, me: &Agent, args: &Value) -> Result<Vec<Ticket>, String> {
    let raw = args["tickets"].as_array().ok_or("tickets is required")?;
    if raw.is_empty() {
        return Err("tickets is empty".into());
    }
    if raw.len() > MAX_TICKETS {
        return Err(format!("at most {MAX_TICKETS} tickets at a time"));
    }
    let titles: Vec<String> = raw.iter().map(|t| t["title"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    let keys: Vec<String> = titles.iter().map(|t| key(t)).collect();
    let mut out = vec![];
    for (i, t) in raw.iter().enumerate() {
        let title = titles[i].clone();
        let body = t["body"].as_str().unwrap_or("").trim().to_string();
        if title.is_empty() || body.is_empty() {
            return Err(format!("ticket {} needs a title and a body", i + 1));
        }
        if title.chars().count() > MAX_TITLE {
            return Err(format!("\"{title}\": a title is at most {MAX_TITLE} characters"));
        }
        if keys[..i].contains(&keys[i]) {
            return Err(format!("two tickets are titled \"{title}\": every ticket needs its own title"));
        }
        let home = match t["project"].as_str().map(str::trim) {
            Some(p) if !p.is_empty() => for_project(all, t).map_err(|e| format!("\"{title}\": {e}"))?,
            _ => me.clone(),
        };
        let mut blocked_by = vec![];
        for b in t["blocked_by"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            match keys.iter().position(|k| *k == key(b)) {
                Some(j) if j == i => return Err(format!("\"{title}\" can't block itself")),
                Some(j) if !blocked_by.contains(&j) => blocked_by.push(j),
                Some(_) => {}
                None => return Err(format!("\"{title}\" is blocked by \"{b}\", which isn't one of these tickets")),
            }
        }
        out.push(Ticket { title, body, blocked_by, home });
    }
    Ok(out)
}

/// Positions in publishing order: every ticket after the ones that block it, otherwise as listed.
fn blockers_first(ts: &[Ticket]) -> Result<Vec<usize>, String> {
    let mut order: Vec<usize> = vec![];
    while order.len() < ts.len() {
        let next = (0..ts.len()).find(|i| !order.contains(i) && ts[*i].blocked_by.iter().all(|b| order.contains(b)));
        match next {
            Some(i) => order.push(i),
            None => {
                let stuck: Vec<String> = (0..ts.len()).filter(|i| !order.contains(i)).map(|i| format!("\"{}\"", ts[i].title)).collect();
                return Err(format!("these tickets block each other in a loop: {}", stuck.join(", ")));
            }
        }
    }
    Ok(order)
}

/// An issue on GitHub as publishing needs it: repo (`home` unless GitHub names another), number,
/// database id (what links take), title key.
fn issue_ref(i: &Value, home: &str) -> Option<(String, i64, i64, String)> {
    Some((repo_of(i, home), i["number"].as_i64()?, i["id"].as_i64()?, key(i["title"].as_str()?)))
}

/// The repo of an issue GitHub listed: from its `repository_url`, else `home`, where it was listed. GitHub
/// may spell it in another case than a git origin, so compare it with `same_repo`.
fn repo_of(i: &Value, home: &str) -> String {
    let prefix = format!("{}/repos/", crate::github::api_base().trim_end_matches('/'));
    i["repository_url"].as_str().and_then(|u| u.strip_prefix(&prefix)).unwrap_or(home).to_string()
}

/// Two `owner/repo` slugs name the same repo: GitHub ignores case in them.
fn same_repo(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The database id of issue #n, which sub-issue and dependency links take instead of its number.
async fn issue_id(state: &AppState, slug: &str, n: i64, known: Option<i64>) -> Result<i64, String> {
    if let Some(id) = known {
        return Ok(id);
    }
    let v = crate::github::get(state, &format!("/repos/{slug}/issues/{n}")).await.map_err(|e| format!("Couldn't read #{n} on {slug}: {e}."))?;
    v["id"].as_i64().ok_or_else(|| format!("GitHub didn't return the id of #{n} on {slug}."))
}

/// Issue #n of `repo` as seen from `home`: `n` there, `owner/repo#n` from another repo. What the
/// records of a publish are keyed by, so a plan in one repo keeps the keys it always had.
fn seen_from(home: &str, repo: &str, n: i64) -> String {
    if repo == home { n.to_string() } else { format!("{repo}#{n}") }
}

/// Publishes the tickets, each in its own project's repo, noting each one on the running grill as soon
/// as it's on GitHub, so the end-of-planning card lists it even if a later step fails and the grill ends
/// without a retry.
async fn publish_tickets(state: &AppState, all: &Agent, me: &Agent, args: &Value) -> Result<String, String> {
    let ts = tickets_in(all, me, args)?;
    let given = !args["spec"].is_null();
    if !given && ts.len() > 1 {
        return Err("Several tickets belong to a spec: publish it with `publish_spec` first and pass it as `spec`. Only a single ticket goes without one.".into());
    }
    // A standalone ticket goes to its own project.
    let me = if given { me } else { &ts[0].home };
    let order = blockers_first(&ts)?;
    let slug = match github_home(state, me).await {
        Ok(slug) => slug,
        Err(why) => {
            let id = args["spec"].as_str().map(str::trim);
            if given && id.is_none_or(str::is_empty) {
                return Err(format!("This plan is kept in Loft ({why}): pass the spec's Loft issue id from publish_spec as `spec`, not a number."));
            }
            return in_loft::tickets(state, me, id, &ts, &order, &why).await;
        }
    };
    let spec = args["spec"].as_i64();
    if given && spec.is_none() {
        return Err(format!("This project's plan goes to GitHub ({slug}): pass the spec's GitHub issue number as `spec`."));
    }
    // Each ticket's repo. A plan on GitHub keeps all its tickets there.
    let mut slugs = vec![];
    for t in &ts {
        slugs.push(if t.home.project == me.project {
            slug.clone()
        } else {
            github_home(state, &t.home).await.map_err(|why| format!("\"{}\" is for {}, which has no GitHub repo ({why}), so it can't join spec #{} on {slug}.", t.title, t.home.project, spec.unwrap_or(0)))?
        });
    }
    // The Delivery only knows the tickets in the spec's repo, so one there can't wait on one elsewhere:
    // it would start before its blocker landed.
    for (i, t) in ts.iter().enumerate() {
        if slugs[i] == slug
            && let Some(&b) = t.blocked_by.iter().find(|&&b| slugs[b] != slug)
        {
            return Err(format!(
                "\"{}\" is in {slug} but blocked by \"{}\" in {}: a ticket in the spec's repo can only be blocked by tickets in that repo. Swap the blocker, or name the other repo as the primary one.",
                t.title, ts[b].title, slugs[b]
            ));
        }
    }
    let _held = state.publish_locks.of(&slug).lock_owned().await;
    let login = login(state).await?;
    let mut labelled: Vec<&String> = vec![];
    for s in std::iter::once(&slug).chain(&slugs) {
        if !labelled.contains(&s) {
            ensure_labels(state, s).await?;
            labelled.push(s);
        }
    }
    // What GitHub already has: the spec's sub-issues (from any repo), or the open tickets for a standalone
    // one. A ticket created before but never recorded (the reply was lost) is adopted, not published twice.
    let listed = match spec {
        Some(s) => crate::github::get(state, &format!("/repos/{slug}/issues/{s}/sub_issues?per_page=100")).await.map_err(|e| format!("Couldn't read spec #{s} on {slug}: {e}"))?,
        None => crate::github::get(state, &format!("/repos/{slug}/issues?labels=ready-for-agent&state=open&per_page=100"))
            .await
            .map_err(|e| format!("Couldn't check {slug} for an existing ticket: {e}"))?,
    };
    let listed: Vec<(String, i64, i64, String)> = listed.as_array().into_iter().flatten().filter(|i| i.get("pull_request").is_none()).filter_map(|i| issue_ref(i, &slug)).collect();
    let find = |repo: &str, n: i64| listed.iter().find(|l| same_repo(&l.0, repo) && l.1 == n);
    let branch = match spec {
        Some(s) => branch_section(state, &slug, s).await?,
        None => None,
    };
    let scope = spec.map_or("standalone".to_string(), |s| format!("#{s}"));
    let retry = "What was published so far stays: call publish_tickets again with the same tickets to finish without duplicates.";
    let shown = |repo: &str, n: i64| if repo == slug { format!("#{n}") } else { format!("{repo}#{n}") };
    // Each published ticket: its repo, number and, once known, database id.
    let mut made: Vec<Option<(String, i64, Option<i64>)>> = vec![None; ts.len()];
    // GitHub's answer for each ticket created now, to bring it in without reading it again.
    let mut created: HashMap<(String, i64), Value> = HashMap::new();
    let mut lines = vec![];
    let mut fresh = 0;
    for &i in &order {
        let (t, repo) = (&ts[i], slugs[i].as_str());
        // Recorded on the ticket's repo; one in another repo than the spec's names the spec's repo too.
        let k = if repo == slug { format!("{scope}/{}", key(&t.title)) } else { format!("{slug}{scope}/{}", key(&t.title)) };
        // A standalone ticket counts as published only while it's still open: once closed, the same
        // request is new work. The database id is only read from GitHub when a link still needs it.
        let recorded = state.db.published_issue(repo, "ticket", &k).ok().flatten().filter(|n| spec.is_some() || find(repo, *n).is_some());
        let (n, mut id, new) = if let Some(n) = recorded {
            (n, find(repo, n).map(|l| l.2), false)
        } else if let Some(l) = listed.iter().find(|l| same_repo(&l.0, repo) && l.3 == key(&t.title)) {
            state.db.record_published_issue(repo, "ticket", &k, l.1).map_err(|e| e.to_string())?;
            (l.1, Some(l.2), false)
        } else {
            // The spec branch is in the spec's repo: only tickets there go on it.
            let body = match branch.as_deref() {
                Some(b) if repo == slug => with_branch(&t.body, b),
                _ => t.body.clone(),
            };
            let v = crate::github::request(state, reqwest::Method::POST, &format!("/repos/{repo}/issues"), Some(json!({ "title": t.title, "body": body, "labels": ["ready-for-agent"], "assignees": [login] })))
                .await
                .map_err(|e| format!("GitHub rejected the ticket \"{}\" on {repo}: {e}. {retry}", t.title))?;
            let (Some(n), Some(id)) = (v["number"].as_i64(), v["id"].as_i64()) else {
                return Err(format!("GitHub created the ticket \"{}\" but didn't return its number. {retry}", t.title));
            };
            state.db.record_published_issue(repo, "ticket", &k, n).map_err(|e| e.to_string())?;
            fresh += 1;
            created.insert((repo.to_string(), n), v);
            (n, Some(id), true)
        };
        let spec_link = spec.map(|number| IssueLink { number, title: String::new(), id: String::new(), repo: String::new() });
        let other = if repo == slug { String::new() } else { repo.to_string() };
        crate::grill::record_published(state, &me.id, &slug, spec_link, vec![IssueLink { number: n, title: t.title.clone(), id: String::new(), repo: other }]);
        if let Some(s) = spec {
            let lk = format!("{s}/{}", seen_from(&slug, repo, n));
            if !matches!(state.db.published_issue(&slug, "sub_issue", &lk), Ok(Some(_))) {
                if find(repo, n).is_none() {
                    let id = *id.insert(issue_id(state, repo, n, id).await.map_err(|e| format!("{e} {retry}"))?);
                    crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{s}/sub_issues"), Some(json!({ "sub_issue_id": id })))
                        .await
                        .map_err(|e| format!("Couldn't make {} a sub-issue of spec #{s}: {e}. {retry}", shown(repo, n)))?;
                }
                state.db.record_published_issue(&slug, "sub_issue", &lk, n).map_err(|e| e.to_string())?;
            }
        }
        // The Delivery builds the tickets in the spec's repo; one in another repo is published and linked only.
        match spec {
            Some(s) if repo == slug => {
                let blocker_numbers = t.blocked_by.iter().filter_map(|&b| made[b].as_ref().filter(|m| m.0 == slug).map(|m| m.1)).collect();
                crate::delivery::record_ticket(state, &slug, s, n, &t.title, blocker_numbers)
            }
            Some(_) => {}
            None => crate::delivery::record_standalone(state, &slug, n, &t.title, me.project.trim(), &me.id),
        }
        let mut blockers = vec![];
        for &b in &t.blocked_by {
            // Blockers go first, so every one of them is on GitHub by now.
            let (brepo, bn, bid) = made[b].clone().expect("blockers are published first");
            blockers.push(shown(&brepo, bn));
            let dk = format!("{n}/{}", seen_from(repo, &brepo, bn));
            if matches!(state.db.published_issue(repo, "blocked_by", &dk), Ok(Some(_))) {
                continue;
            }
            let bid = issue_id(state, &brepo, bn, bid).await.map_err(|e| format!("{e} {retry}"))?;
            made[b] = Some((brepo.clone(), bn, Some(bid)));
            let path = format!("/repos/{repo}/issues/{n}/dependencies/blocked_by");
            if let Err(e) = crate::github::request(state, reqwest::Method::POST, &path, Some(json!({ "issue_id": bid }))).await {
                // Linked before but never recorded: GitHub turns the second link down, which is fine.
                let have = crate::github::get(state, &format!("{path}?per_page=100")).await.unwrap_or(Value::Null);
                if !have.as_array().into_iter().flatten().any(|d| d["number"].as_i64() == Some(bn) && same_repo(&repo_of(d, repo), &brepo)) {
                    return Err(format!("Couldn't mark {} as blocked by {}: {e}. {retry}", shown(repo, n), shown(&brepo, bn)));
                }
            }
            state.db.record_published_issue(repo, "blocked_by", &dk, bn).map_err(|e| e.to_string())?;
        }
        made[i] = Some((repo.to_string(), n, id));
        let by = if blockers.is_empty() { String::new() } else { format!(", blocked by {}", blockers.join(", ")) };
        let was = if new { "" } else { ", already published" };
        lines.push(format!("- {} {}{by}{was}: https://github.com/{repo}/issues/{n}", shown(repo, n), t.title));
    }
    loft_issues_for(state, me, &slug, spec, &ts, &order, &made, &created).await;
    let head = match (spec, ts.len()) {
        (Some(s), c) => format!("Published {c} ticket{} as sub-issues of spec #{s}, blockers first", if c == 1 { "" } else { "s" }),
        (None, _) => "Published a standalone ticket".to_string(),
    };
    let none_new = if fresh == 0 { "\nNothing new was created: every ticket was already published." } else { "" };
    let msg = format!("{head}, labelled `ready-for-agent` and assigned to {login}:\n{}{none_new}", lines.join("\n"));
    Ok(msg)
}

/// The published tickets as Loft issues right away, as in a project without a repo: each in its own
/// project, linked to its GitHub issue, ready and unassigned, under the spec's Loft issue, depending on its
/// blockers'. A ticket that already has its Loft issue (a retry) is left as it is.
#[allow(clippy::too_many_arguments)]
async fn loft_issues_for(state: &AppState, me: &Agent, slug: &str, spec: Option<i64>, ts: &[Ticket], order: &[usize], made: &[Option<(String, i64, Option<i64>)>], created: &HashMap<(String, i64), Value>) {
    let bring = |project: String, repo: String, n: i64| async move {
        let i = match created.get(&(repo.clone(), n)) {
            Some(i) => i.clone(),
            None => crate::github::get(state, &format!("/repos/{repo}/issues/{n}")).await.ok()?,
        };
        crate::mirror::bring_in(state, project.trim(), &repo, &i)
    };
    let parent = match spec {
        Some(s) => bring(me.project.clone(), slug.to_string(), s).await.map(|(t, _)| t.id),
        None => None,
    };
    let mut ids: HashMap<usize, String> = HashMap::new();
    for &i in order {
        let Some((repo, n, _)) = made[i].clone() else { continue };
        let Some((mut t, new)) = bring(ts[i].home.project.clone(), repo, n).await else { continue };
        ids.insert(i, t.id.clone());
        if !new {
            continue;
        }
        t.status = "ready".into();
        t.created_by = me.id.clone();
        t.parent_id = parent.clone();
        t.depends_on = ts[i].blocked_by.iter().filter_map(|b| ids.get(b).cloned()).collect();
        if state.db.save_task(&t).is_ok() {
            crate::tasks::emit(state, &t);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::github::tests::{Fake, fake_github, own_fake, state_with_token};
    use crate::grill::tests::planner;

    /// A test state with a GitHub token whose projects folder is its own temp dir, read at boot like Loft does.
    pub(crate) fn state() -> AppState {
        let st = state_with_token();
        let dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        let projects = dir.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(dir.join("settings.json"), json!({ "projects_dir": projects, "github_token": "test-token" }).to_string()).unwrap();
        AppState { settings: std::sync::Arc::new(crate::settings::Store::load(&dir).unwrap()), ..st }
    }

    /// A planner whose project folder is a git repo with this `origin`.
    pub(crate) fn planner_on(st: &AppState, name: &str, origin: Option<&str>) -> Agent {
        let mut a = planner(st, name);
        (a.project, a.group) = (name.into(), name.into());
        st.db.save_agent(&a).unwrap();
        let dir = st.settings.boot().projects_dir.join(name);
        assert!(dir.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        if let Some(o) = origin {
            git(&["remote", "add", "origin", o]);
        }
        a
    }

    /// A repo on the fake with these labels, no spec yet, and issue creation answering #number.
    fn repo(fake: &Fake, slug: &str, labels: &[&str], number: i64) {
        fake.route("GET", "/user", 200, json!({ "login": "octo-me" }));
        fake.route("GET", &format!("/repos/{slug}/labels"), 200, json!(labels.iter().map(|l| json!({ "name": l })).collect::<Vec<_>>()));
        fake.route("POST", &format!("/repos/{slug}/labels"), 201, json!({}));
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, json!({ "number": number }));
        spec_pr_on(fake, slug);
    }

    /// What making the spec branch and spec PR on the fake needs: no spec branch or PR yet, and the PR
    /// opened as #500, from `spec/the-branch` when its tickets read it.
    fn spec_pr_on(fake: &Fake, slug: &str) {
        fake.route("GET", &format!("/repos/{slug}"), 200, json!({ "default_branch": "main" }));
        fake.route("GET", &format!("/repos/{slug}/git/ref/heads/main"), 200, json!({ "object": { "sha": "main-sha" } }));
        fake.route("GET", &format!("/repos/{slug}/git/commits/main-sha"), 200, json!({ "sha": "main-sha", "tree": { "sha": "main-tree" } }));
        fake.route("POST", &format!("/repos/{slug}/git/commits"), 201, json!({ "sha": "start-sha" }));
        fake.route("POST", &format!("/repos/{slug}/git/refs"), 201, json!({}));
        fake.route("GET", &format!("/repos/{slug}/pulls"), 200, json!([]));
        fake.route("POST", &format!("/repos/{slug}/pulls"), 201, json!({ "number": 500 }));
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "head": { "ref": "spec/the-branch" } }));
    }

    async fn publish(st: &AppState, a: &Agent, title: &str) -> Result<String, String> {
        mcp_call(st, a, "publish_spec", &json!({ "title": title, "body": "## Problem Statement\n\nIt hurts." })).await.unwrap()
    }

    /// Bodies POSTed to exactly `/repos/{slug}/{what}`, oldest first.
    fn posts(fake: &Fake, slug: &str, what: &str) -> Vec<Value> {
        let path = format!("/repos/{slug}/{what}");
        fake.seen(&path).into_iter().filter(|s| s.method == "POST" && s.path == path).map(|s| s.body).collect()
    }

    #[tokio::test]
    async fn publishes_the_spec_labelled_and_assigned_creating_missing_labels() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-new", Some("git@github.com:octo/spec-new.git"));
        repo(&fake, "octo/spec-new", &["bug", "Ready-For-Agent"], 40);
        let mut events = st.events.subscribe();

        let out = publish(&st, &a, "  Planning   in rounds ").await.unwrap();
        assert_eq!(out, "Published the spec as #40, assigned to octo-me: https://github.com/octo/spec-new/issues/40\nSpec branch: `spec/40-planning-in-rounds`. Spec PR (draft, to the default branch): https://github.com/octo/spec-new/pull/500");
        // The Issues tab hears about it and refetches.
        let evs: Vec<Value> = std::iter::from_fn(|| events.try_recv().ok()).map(|e| serde_json::to_value(e).unwrap()).collect();
        assert!(evs.contains(&json!({ "type": "issues", "project": "spec-new" })), "{evs:?}");

        let labels = posts(&fake, "octo/spec-new", "labels");
        assert_eq!(labels.len(), 1, "only the missing label is created: {labels:?}");
        assert_eq!(labels[0]["name"], "spec");
        assert_eq!(posts(&fake, "octo/spec-new", "issues"), [json!({
            "title": "Planning in rounds", "body": "## Problem Statement\n\nIt hurts.", "labels": ["spec"], "assignees": ["octo-me"],
        })]);
    }

    #[tokio::test]
    async fn a_published_spec_is_a_loft_issue_at_once_and_stays_after_a_poll_and_a_full_read() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/spec-shows";
        let a = planner_on(&st, "spec-shows", Some("git@github.com:octo/spec-shows.git"));
        repo(&fake, slug, &["spec"], 60);
        let made = json!({
            "number": 60, "id": 6000, "title": "Planning in rounds", "state": "open", "body": "## Problem Statement\n\nIt hurts.",
            "html_url": format!("https://github.com/{slug}/issues/60"), "updated_at": "2026-11-04T19:12:20Z",
            "labels": [{ "name": "spec" }], "assignees": [{ "login": "octo-me" }], "user": { "login": "octo-me" },
        });
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, made.clone());
        publish(&st, &a, "Planning in rounds").await.unwrap();

        let listed = || async {
            let Json(v) = crate::tasks::list(State(st.clone()), axum::extract::Query(serde_json::from_value(json!({})).unwrap())).await.unwrap();
            v.as_array().unwrap().iter().filter(|t| t["github"]["number"] == 60).cloned().collect::<Vec<_>>()
        };
        let check = |v: Vec<Value>, when: &str| {
            assert_eq!(v.len(), 1, "{when}: {v:?}");
            assert_eq!((&v[0]["project"], &v[0]["labels"], &v[0]["state"], &v[0]["parent_id"]), (&json!("spec-shows"), &json!(["spec"]), &json!("open"), &Value::Null), "{when}");
        };
        check(listed().await, "right after publishing");
        // GitHub now lists it, with sub-issues added since.
        let mut later = made.clone();
        later["updated_at"] = json!("2026-11-04T19:26:04Z");
        later["sub_issues_summary"] = json!({ "total": 2, "completed": 0 });
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([later]));
        fake.route("GET", &format!("/repos/{slug}/issues/60/sub_issues"), 200, json!([]));
        crate::mirror::sync_project(&st, "spec-shows", false).await.unwrap();
        crate::mirror::sync_project(&st, "spec-shows", false).await.unwrap();
        check(listed().await, "after a poll");
        crate::mirror::sync_project(&st, "spec-shows", true).await.unwrap();
        let v = listed().await;
        check(v.clone(), "after a full read");
        assert_eq!(v[0]["github"]["sub_total"], 2);
    }

    #[tokio::test]
    async fn publishing_starts_the_spec_branch_and_opens_a_draft_spec_pr() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-branch", Some("git@github.com:octo/spec-branch.git"));
        repo(&fake, "octo/spec-branch", &["spec", "ready-for-agent"], 186);

        let out = publish(&st, &a, "One PR per spec: branch & ticket closing, ready on the last close").await.unwrap();
        assert!(out.ends_with("Spec branch: `spec/186-one-pr-per-spec-branch-ticket-closing`. Spec PR (draft, to the default branch): https://github.com/octo/spec-branch/pull/500"), "{out}");
        // An empty commit on top of the default branch, then the branch pointing at it.
        assert_eq!(posts(&fake, "octo/spec-branch", "git/commits"), [json!({ "message": "Start spec #186", "tree": "main-tree", "parents": ["main-sha"] })]);
        assert_eq!(posts(&fake, "octo/spec-branch", "git/refs"), [json!({ "ref": "refs/heads/spec/186-one-pr-per-spec-branch-ticket-closing", "sha": "start-sha" })]);
        assert_eq!(posts(&fake, "octo/spec-branch", "pulls"), [json!({
            "title": "One PR per spec: branch & ticket closing, ready on the last close", "head": "spec/186-one-pr-per-spec-branch-ticket-closing",
            "base": "main", "body": "Closes #186", "draft": true,
        })]);

        // The retry finds both recorded and asks GitHub nothing about them.
        let before = fake.seen("/repos/octo/spec-branch").len();
        let again = publish(&st, &a, "one pr per spec: branch & ticket closing, ready on the last close").await.unwrap();
        assert!(again.contains("already published as #186") && again.ends_with("/pull/500"), "{again}");
        assert_eq!(fake.seen("/repos/octo/spec-branch").len(), before);
    }

    #[tokio::test]
    async fn the_spec_pr_is_tracked_for_its_planner_so_a_checker_of_the_team_can_set_its_verdict() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/spec-tracked";
        let a = planner_on(&st, "spec-tracked", Some("git@github.com:octo/spec-tracked.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 31);
        publish(&st, &a, "Tracked spec PRs").await.unwrap();
        assert!(st.db.find_pull_request(&a.id, slug, 500).unwrap().is_some(), "tracked as soon as it's opened");

        let mut qa = crate::tasks::tests::agent(&st, "Tester spec-tracked");
        (qa.project, qa.group, qa.role) = ("spec-tracked".into(), "spec-tracked".into(), "qa".into());
        st.db.save_agent(&qa).unwrap();
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "state": "open", "draft": true, "head": { "ref": "spec/31-tracked-spec-prs", "sha": "abc1234def" }, "base": { "ref": "main", "repo": {} }, "user": { "login": "octo" } }));
        fake.route("GET", &format!("/repos/{slug}/commits/abc1234def/check-runs"), 200, json!({ "check_runs": [] }));
        fake.route("GET", &format!("/repos/{slug}/commits/abc1234def/status"), 200, json!({ "statuses": [] }));
        fake.route("POST", &format!("/repos/{slug}/issues/500/labels"), 200, json!([{ "name": "qa:passed" }]));
        fake.route("DELETE", &format!("/repos/{slug}/issues/500/labels/qa%3Afailed"), 404, json!({ "message": "Label does not exist" }));
        let said = crate::prs::set_verdict(&st, &qa, &format!("https://github.com/{slug}/pull/500"), "qa", true, "abc1234").await.unwrap();
        assert!(said.contains("qa:passed"), "{said}");
    }

    #[tokio::test]
    async fn on_start_up_the_pr_of_a_delivery_not_yet_settled_is_tracked() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/spec-startup";
        let a = planner_on(&st, "spec-startup", None);
        crate::delivery::record_spec(&st, slug, 4, "spec-startup", &a.id, "spec/4-x");
        crate::delivery::update(&st, slug, 4, |d| d.pr = Some(7));
        fake.route("GET", &format!("/repos/{slug}/pulls/7"), 200, json!({ "number": 7 }));
        assert!(st.db.find_pull_request(&a.id, slug, 7).unwrap().is_none());

        crate::delivery::track_open_prs(st.clone()).await;
        assert!(st.db.find_pull_request(&a.id, slug, 7).unwrap().is_some());
        crate::delivery::track_open_prs(st.clone()).await;
        assert_eq!(st.db.list_pull_requests(None).unwrap().iter().filter(|p| p.slug == slug).count(), 1, "once");
    }

    async fn publish_with_docs(st: &AppState, a: &Agent, title: &str, docs: &str) -> Result<String, String> {
        mcp_call(st, a, "publish_spec", &json!({ "title": title, "body": "## Problem Statement\n\nIt hurts.", "docs_branch": docs })).await.unwrap()
    }

    #[tokio::test]
    async fn a_grills_docs_branch_starts_the_spec_branch() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/spec-docs";
        let a = planner_on(&st, "spec-docs", Some("git@github.com:octo/spec-docs.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 12);

        // Not pushed yet: the spec is published, the branch waits for a retry.
        let out = publish_with_docs(&st, &a, "Rounds", "docs/rounds").await.unwrap();
        assert!(out.starts_with("Published the spec as #12"), "{out}");
        assert!(out.ends_with("The spec is published, but the docs branch `docs/rounds` isn't on GitHub, so the spec branch `spec/12-rounds` wasn't created: push it first. Call publish_spec again with the same title and `docs_branch` to retry only the spec branch and PR."), "{out}");
        assert!(posts(&fake, slug, "git/refs").is_empty() && posts(&fake, slug, "pulls").is_empty());

        fake.route("GET", &format!("/repos/{slug}/git/ref/heads/docs/rounds"), 200, json!({ "object": { "sha": "docs-sha" } }));
        let out = publish_with_docs(&st, &a, "Rounds", "docs/rounds").await.unwrap();
        assert!(out.ends_with("Spec branch: `spec/12-rounds`. Spec PR (draft, to the default branch): https://github.com/octo/spec-docs/pull/500"), "{out}");
        // The spec branch points at the docs head, so its history holds the docs commits: no empty commit.
        assert_eq!(posts(&fake, slug, "git/refs"), [json!({ "ref": "refs/heads/spec/12-rounds", "sha": "docs-sha" })]);
        assert!(fake.seen(&format!("/repos/{slug}/git/commits")).is_empty(), "no empty commit");
        assert_eq!(posts(&fake, slug, "pulls")[0]["base"], "main");
        assert_eq!(posts(&fake, slug, "issues").len(), 1, "never a second spec");
    }

    /// Spec #12's branch `spec/12-rounds` already on the fake (a retry after a failed PR), and the docs branch pushed.
    fn spec_branch_there(fake: &Fake, slug: &str) {
        fake.route("GET", &format!("/repos/{slug}/git/ref/heads/spec/12-rounds"), 200, json!({ "object": { "sha": "start-sha" } }));
        fake.route("GET", &format!("/repos/{slug}/git/ref/heads/docs/rounds"), 200, json!({ "object": { "sha": "docs-sha" } }));
    }

    #[tokio::test]
    async fn a_docs_branch_is_merged_into_a_spec_branch_already_there() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/spec-docs-merge";
        let a = planner_on(&st, "spec-docs-merge", Some("git@github.com:octo/spec-docs-merge.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 12);
        spec_branch_there(&fake, slug);
        fake.route("POST", &format!("/repos/{slug}/merges"), 201, json!({ "sha": "merge1234abcd" }));

        let out = publish_with_docs(&st, &a, "Rounds", "docs/rounds").await.unwrap();
        assert!(out.ends_with("Spec PR (draft, to the default branch): https://github.com/octo/spec-docs-merge/pull/500\nThe spec branch was already there: merged the docs branch `docs/rounds` into it (merge12)."), "{out}");
        assert_eq!(posts(&fake, slug, "merges"), [json!({ "base": "spec/12-rounds", "head": "docs/rounds", "commit_message": "Merge the grill's docs from docs/rounds" })]);
        assert!(posts(&fake, slug, "git/refs").is_empty(), "the branch isn't made again");

        // A retry once the PR is recorded: GitHub has nothing left to merge (204) and the reply adds nothing.
        fake.route("POST", &format!("/repos/{slug}/merges"), 204, Value::Null);
        let out = publish_with_docs(&st, &a, "Rounds", "docs/rounds").await.unwrap();
        assert!(out.ends_with("https://github.com/octo/spec-docs-merge/pull/500"), "{out}");
        assert_eq!(posts(&fake, slug, "merges").len(), 2);
    }

    #[tokio::test]
    async fn docs_that_cant_be_merged_are_named_and_the_spec_stays_published() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/spec-docs-conflict";
        let a = planner_on(&st, "spec-docs-conflict", Some("git@github.com:octo/spec-docs-conflict.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 12);
        spec_branch_there(&fake, slug);
        fake.route("POST", &format!("/repos/{slug}/merges"), 409, json!({ "message": "Merge conflict" }));

        let out = publish_with_docs(&st, &a, "Rounds", "docs/rounds").await.unwrap();
        assert!(out.starts_with("Published the spec as #12"), "{out}");
        assert!(out.ends_with("/pull/500\nThe docs branch `docs/rounds` conflicts with `spec/12-rounds`, so its commits aren't on it: merge `docs/rounds` into `spec/12-rounds` by hand."), "{out}");
        assert_eq!(posts(&fake, slug, "pulls").len(), 1, "the spec PR is still opened");

        fake.route("POST", &format!("/repos/{slug}/merges"), 404, json!({ "message": "Head does not exist" }));
        let out = publish_with_docs(&st, &a, "Rounds", "docs/rounds").await.unwrap();
        assert!(out.ends_with("\nThe docs branch `docs/rounds` isn't on GitHub, so its commits aren't on `spec/12-rounds`: push it, then call publish_spec again with the same title and `docs_branch`."), "{out}");
    }

    #[tokio::test]
    async fn a_docs_branch_that_isnt_a_branch_name_is_turned_down() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-docs-bad", Some("git@github.com:octo/spec-docs-bad.git"));
        repo(&fake, "octo/spec-docs-bad", &["spec", "ready-for-agent"], 1);
        for bad in ["docs/../main", "docs rounds", "/docs", "docs/"] {
            let err = publish_with_docs(&st, &a, "Rounds", bad).await.unwrap_err();
            assert_eq!(err, format!("\"{bad}\" isn't a branch name: pass the docs branch as you pushed it, like `docs/<topic>`"));
        }
        assert!(fake.seen("/repos/octo/spec-docs-bad").is_empty(), "nothing asked of GitHub");
    }

    #[tokio::test]
    async fn a_failed_spec_branch_or_pr_leaves_the_spec_published_and_a_retry_makes_only_whats_missing() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-pr-fail", Some("git@github.com:octo/spec-pr-fail.git"));
        repo(&fake, "octo/spec-pr-fail", &["spec", "ready-for-agent"], 8);
        let retry = "Call publish_spec again with the same title to retry only the spec branch and PR.";

        // No push rights: the branch is turned down.
        fake.route("POST", "/repos/octo/spec-pr-fail/git/refs", 403, json!({ "message": "Resource not accessible by integration" }));
        let out = publish(&st, &a, "Ship it").await.unwrap();
        assert_eq!(out, format!("Published the spec as #8, assigned to octo-me: https://github.com/octo/spec-pr-fail/issues/8\nThe spec is published, but the spec branch `spec/8-ship-it` couldn't be created: GitHub 403: Resource not accessible by integration. {retry}"));
        assert!(posts(&fake, "octo/spec-pr-fail", "pulls").is_empty());

        // The branch goes up now, then the draft PR is turned down.
        fake.route("POST", "/repos/octo/spec-pr-fail/git/refs", 201, json!({}));
        fake.route("POST", "/repos/octo/spec-pr-fail/pulls", 422, json!({ "message": "Validation Failed", "errors": [{ "message": "Draft pull requests are not supported in this repository." }] }));
        let out = publish(&st, &a, "Ship it").await.unwrap();
        assert!(out.starts_with("The spec \"Ship it\" is already published as #8"), "{out}");
        assert!(out.ends_with(&format!("but the draft spec PR from `spec/8-ship-it` couldn't be opened: GitHub 422: Validation Failed (Draft pull requests are not supported in this repository.). {retry}")), "{out}");

        // The branch exists now: only the PR is made.
        fake.route("GET", "/repos/octo/spec-pr-fail/git/ref/heads/spec/8-ship-it", 200, json!({ "object": { "sha": "start-sha" } }));
        fake.route("POST", "/repos/octo/spec-pr-fail/pulls", 201, json!({ "number": 9 }));
        let out = publish(&st, &a, "Ship it").await.unwrap();
        assert!(out.ends_with("Spec branch: `spec/8-ship-it`. Spec PR (draft, to the default branch): https://github.com/octo/spec-pr-fail/pull/9"), "{out}");

        assert_eq!(posts(&fake, "octo/spec-pr-fail", "issues").len(), 1, "never a second spec");
        assert_eq!(posts(&fake, "octo/spec-pr-fail", "git/refs").len(), 2, "one refused, one created");
        assert_eq!(posts(&fake, "octo/spec-pr-fail", "pulls").len(), 2, "one refused, one opened");
    }

    #[tokio::test]
    async fn a_spec_pr_opened_but_never_recorded_is_adopted() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-pr-lost", Some("git@github.com:octo/spec-pr-lost.git"));
        repo(&fake, "octo/spec-pr-lost", &["spec", "ready-for-agent"], 4);
        fake.route("GET", "/repos/octo/spec-pr-lost/git/ref/heads/spec/4-lost", 200, json!({ "object": { "sha": "start-sha" } }));
        fake.route("GET", "/repos/octo/spec-pr-lost/pulls", 200, json!([{ "number": 77 }]));

        let out = publish(&st, &a, "Lost").await.unwrap();
        assert!(out.ends_with("https://github.com/octo/spec-pr-lost/pull/77"), "{out}");
        assert_eq!(fake.seen("/repos/octo/spec-pr-lost/pulls?").iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["/repos/octo/spec-pr-lost/pulls?head=octo:spec/4-lost&state=all"]);
        assert!(posts(&fake, "octo/spec-pr-lost", "pulls").is_empty() && posts(&fake, "octo/spec-pr-lost", "git/refs").is_empty());
    }

    #[tokio::test]
    async fn a_retry_returns_the_published_spec_inside_or_outside_a_grill() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-retry", Some("https://github.com/octo/spec-retry"));
        repo(&fake, "octo/spec-retry", &["spec", "ready-for-agent"], 7);

        publish(&st, &a, "Rounds").await.unwrap();
        // No grill is running: the record is Loft's, keyed on the repo and title.
        assert!(crate::grill::active(&st, &a.id).is_none());
        let again = publish(&st, &a, "rounds").await.unwrap();
        assert!(again.contains("already published as #7"), "{again}");
        assert_eq!(posts(&fake, "octo/spec-retry", "issues").len(), 1);
        assert!(posts(&fake, "octo/spec-retry", "labels").is_empty(), "existing labels are left alone");
    }

    #[tokio::test]
    async fn a_retry_after_a_failure_creates_nothing_twice() {
        let st = state();
        let fake = own_fake(&st);
        let a = planner_on(&st, "spec-partial", Some("git@github.com:octo/spec-partial.git"));
        repo(&fake, "octo/spec-partial", &[], 3);

        // The labels go up, then GitHub turns the spec down.
        fake.route("POST", "/repos/octo/spec-partial/issues", 422, json!({ "message": "Validation Failed", "errors": [{ "message": "could not add assignee" }] }));
        let err = publish(&st, &a, "Specs").await.unwrap_err();
        assert_eq!(err, "GitHub rejected the spec on octo/spec-partial: GitHub 422: Validation Failed (could not add assignee)");
        assert_eq!(posts(&fake, "octo/spec-partial", "labels").len(), 2);

        // The retry knows the labels and the login already, and creates the spec once.
        fake.route("POST", "/repos/octo/spec-partial/issues", 201, json!({ "number": 3 }));
        assert!(publish(&st, &a, "Specs").await.unwrap().starts_with("Published the spec as #3"));
        assert_eq!(posts(&fake, "octo/spec-partial", "labels").len(), 2);
        assert_eq!(fake.seen("/repos/octo/spec-partial/labels?").len(), 1, "labels are read once");
        assert_eq!(fake.seen("/user").len(), 1, "the login is read once");
        assert_eq!(posts(&fake, "octo/spec-partial", "issues").len(), 2, "one refused, one created");
    }

    #[tokio::test]
    async fn a_spec_created_but_never_recorded_is_adopted() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-lost", Some("git@github.com:octo/spec-lost.git"));
        repo(&fake, "octo/spec-lost", &["spec", "ready-for-agent"], 99);
        fake.route("GET", "/repos/octo/spec-lost/issues", 200, json!([
            { "number": 5, "title": "Rounds", "pull_request": {} },
            { "number": 6, "title": "Rounds" },
        ]));

        let out = publish(&st, &a, "Rounds").await.unwrap();
        assert!(out.contains("already published as #6"), "{out}");
        assert!(posts(&fake, "octo/spec-lost", "issues").is_empty());
        assert!(publish(&st, &a, "Rounds").await.unwrap().contains("#6"));
        assert_eq!(fake.seen("/repos/octo/spec-lost/issues?labels=spec").len(), 1, "the second retry trusts Loft's record");
    }

    #[tokio::test]
    async fn overlapping_publishes_of_one_spec_create_it_once() {
        let (fake, st) = (fake_github(), state());
        // Two planners on one repo, both publishing the same spec at once.
        let a = planner_on(&st, "spec-race", Some("git@github.com:octo/spec-race.git"));
        let b = planner_on(&st, "spec-race-too", Some("https://github.com/octo/spec-race"));
        repo(&fake, "octo/spec-race", &["spec", "ready-for-agent"], 7);

        let (x, y) = tokio::join!(publish(&st, &a, "Rounds"), publish(&st, &b, "rounds"));
        assert_eq!(posts(&fake, "octo/spec-race", "issues").len(), 1, "{x:?} / {y:?}");
        assert!([x.unwrap(), y.unwrap()].iter().any(|o| o.contains("already published")));
    }

    #[tokio::test]
    async fn a_label_github_says_already_exists_counts_as_there() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-many-labels", Some("git@github.com:octo/spec-many-labels.git"));
        // `spec` lies beyond the first page of labels, so creating it is refused as a duplicate.
        repo(&fake, "octo/spec-many-labels", &["ready-for-agent"], 12);
        fake.route("POST", "/repos/octo/spec-many-labels/labels", 422, json!({
            "message": "Validation Failed", "errors": [{ "resource": "Label", "code": "already_exists", "field": "name" }],
        }));

        let out = publish(&st, &a, "Many labels").await.unwrap();
        assert!(out.starts_with("Published the spec as #12"), "{out}");
        assert_eq!(posts(&fake, "octo/spec-many-labels", "labels").len(), 1);
        assert_eq!(posts(&fake, "octo/spec-many-labels", "issues").len(), 1);
    }

    #[tokio::test]
    async fn a_token_github_turns_down_is_explained() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-bad-token", Some("git@github.com:octo/spec-bad-token.git"));
        // `/user` is shared by every test on the fake, so the 401 comes from this repo's labels.
        repo(&fake, "octo/spec-bad-token", &[], 1);
        fake.route("GET", "/repos/octo/spec-bad-token/labels", 401, json!({ "message": "Bad credentials" }));

        let err = publish(&st, &a, "Bad token").await.unwrap_err();
        assert_eq!(err, "Couldn't read the labels of octo/spec-bad-token: GitHub 401: Bad credentials");
        assert!(posts(&fake, "octo/spec-bad-token", "issues").is_empty());
    }

    /// Ticket #n on `slug` as GitHub gives it.
    fn gh_ticket(slug: &str, n: i64, state: &str, body: &str) -> Value {
        json!({
            "number": n, "id": n * 100, "title": format!("Ticket {n}"), "body": body, "state": state, "state_reason": if state == "closed" { json!("completed") } else { Value::Null },
            "labels": [{ "name": "ready-for-agent" }], "html_url": format!("https://github.com/{slug}/issues/{n}"), "updated_at": format!("2026-10-31T00:00:0{}Z", if state == "closed" { 1 } else { 0 }),
        })
    }

    /// What closing a ticket whose reviewers all passed does, as `a`'s work.
    async fn close(st: &AppState, a: &Agent, n: i64, commits: &[&str]) -> Result<String, String> {
        let slug = github_home(st, a).await?;
        let commits = commits_in(&json!({ "commits": commits }))?;
        close_ticket(st, &a.project, a, &slug, n, &commits).await
    }

    #[tokio::test]
    async fn close_ticket_comments_the_commits_and_spec_pr_then_closes_it_completed() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-one";
        let a = planner_on(&st, "close-one", Some("git@github.com:octo/close-one.git"));
        let body = "## What to build\n\nStore.\n\n## Branch\n\nCommit on the spec branch `spec/40-rounds`. It ships in the spec PR https://github.com/octo/close-one/pull/500.";
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "open", body));
        fake.route("POST", &format!("/repos/{slug}/issues/41/comments"), 201, json!({}));
        fake.route("PATCH", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "closed", body));
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "state": "open", "draft": false }));
        // Its Loft issue is being worked on, which a close from GitHub alone would leave open.
        let (mut t, _) = crate::mirror::bring_in(&st, "close-one", slug, &gh_ticket(slug, 41, "open", body)).unwrap();
        t.status = "in_progress".into();
        st.db.save_task(&t).unwrap();

        let out = close(&st, &a, 41, &["ABC1234def5678abc1234def5678abc1234def56", "def5678"]).await.unwrap();
        assert_eq!(out, "Closed #41 as completed with the comment \"Done in `abc1234`, `def5678`, part of #500.\": https://github.com/octo/close-one/issues/41");
        assert_eq!(posts(&fake, slug, "issues/41/comments"), [json!({ "body": "Done in `abc1234`, `def5678`, part of #500." })]);
        let patched: Vec<Value> = fake.seen(&format!("/repos/{slug}/issues/41")).into_iter().filter(|s| s.method == "PATCH").map(|s| s.body).collect();
        assert_eq!(patched, [json!({ "state": "closed", "state_reason": "completed" })]);
        let t = st.db.get_task(&t.id).unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.github.unwrap().state.as_str()), ("done", "closed"));
    }

    /// Spec #40 on the fake: its draft spec PR #500 and tickets #41 and #42, both open, ready to close.
    /// Spec #40's Delivery: spec PR #500 on `spec/40-rounds`, tickets #41 and #42.
    fn delivery_40(st: &AppState, slug: &str) {
        crate::delivery::record_spec(st, slug, 40, "", "", "spec/40-rounds");
        crate::delivery::update(st, slug, 40, |d| d.pr = Some(500));
        for n in [41, 42] {
            crate::delivery::record_ticket(st, slug, 40, n, "", vec![]);
        }
    }

    fn spec_with_tickets(st: &AppState, fake: &Fake, slug: &str) -> String {
        delivery_40(st, slug);
        let body = format!("## What to build\n\nX.\n\n## Branch\n\nCommit on the spec branch `spec/40-rounds`. It ships in the spec PR https://github.com/{slug}/pull/500.");
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true, "body": "Closes #40" }));
        for n in [41, 42] {
            fake.route("GET", &format!("/repos/{slug}/issues/{n}"), 200, gh_ticket(slug, n, "open", &body));
            fake.route("POST", &format!("/repos/{slug}/issues/{n}/comments"), 201, json!({}));
            fake.route("PATCH", &format!("/repos/{slug}/issues/{n}"), 200, gh_ticket(slug, n, "closed", &body));
        }
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));
        body
    }

    /// Spec #40's tickets #41, #42… in these states on GitHub only.
    fn subs_on_github(fake: &Fake, slug: &str, states: &[&str]) {
        let list: Vec<Value> = states.iter().enumerate().map(|(i, s)| json!({ "number": 41 + i as i64, "state": s })).collect();
        fake.route("GET", &format!("/repos/{slug}/issues/40/sub_issues"), 200, json!(list));
    }

    /// Spec #40's tickets #41, #42… in these states, on GitHub and on its Delivery record.
    fn subs(st: &AppState, fake: &Fake, slug: &str, states: &[&str]) {
        subs_on_github(fake, slug, states);
        crate::delivery::update(st, slug, 40, |d| {
            for (t, s) in d.tickets.iter_mut().zip(states) {
                t.state = s.to_string();
            }
        });
    }

    #[tokio::test]
    async fn closing_the_last_ticket_marks_the_spec_pr_ready_and_not_before() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-last";
        let a = planner_on(&st, "ready-last", Some("git@github.com:octo/ready-last.git"));
        spec_with_tickets(&st, &fake, slug);

        subs(&st, &fake, slug, &["closed", "open"]);
        let out = close(&st, &a, 41, &["abc1234"]).await.unwrap();
        assert!(!out.contains("ready for review"), "{out}");
        assert!(fake.seen("/graphql").is_empty(), "#42 is still open");

        subs(&st, &fake, slug, &["closed", "closed"]);
        let out = close(&st, &a, 42, &["def5678"]).await.unwrap();
        assert!(out.ends_with("\nIt was the spec's last open ticket: spec PR #500 is now ready for review: https://github.com/octo/ready-last/pull/500"), "{out}");
        let calls = fake.seen("/graphql");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].body["query"].as_str().unwrap().contains("markPullRequestReadyForReview(input: { pullRequestId: $id })"));
        assert_eq!(calls[0].body["variables"], json!({ "id": "PR_500" }));
    }

    #[tokio::test]
    async fn a_refused_ready_for_review_call_leaves_the_ticket_closed_and_says_so() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-refused";
        let a = planner_on(&st, "ready-refused", Some("git@github.com:octo/ready-refused.git"));
        spec_with_tickets(&st, &fake, slug);
        subs(&st, &fake, slug, &["closed", "closed"]);
        // GraphQL turns a mutation down with a 200 and `errors`.
        fake.route("POST", "/graphql", 200, json!({ "errors": [{ "message": "Resource not accessible by integration" }] }));

        let out = close(&st, &a, 42, &["def5678"]).await.unwrap();
        assert!(out.ends_with("\nThe ticket is closed, but couldn't mark spec PR #500 ready for review: Resource not accessible by integration."), "{out}");
        assert!(!out.contains("now ready for review"), "{out}");
        let patched: Vec<Value> = fake.seen(&format!("/repos/{slug}/issues/42")).into_iter().filter(|s| s.method == "PATCH").map(|s| s.body).collect();
        assert_eq!(patched, [json!({ "state": "closed", "state_reason": "completed" })]);
        assert_eq!(fake.seen("/graphql").len(), 1);
    }

    #[tokio::test]
    async fn a_spec_pr_already_ready_or_merged_is_left_alone() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-done";
        let a = planner_on(&st, "ready-done", Some("git@github.com:octo/ready-done.git"));
        spec_with_tickets(&st, &fake, slug);
        subs(&st, &fake, slug, &["closed", "closed"]);

        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": false, "body": "Closes #40" }));
        assert!(!close(&st, &a, 41, &["abc1234"]).await.unwrap().contains("ready for review"));
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "closed", "merged": true, "draft": false, "body": "Closes #40" }));
        assert!(!close(&st, &a, 42, &["def5678"]).await.unwrap().contains("ready for review"));
        assert!(fake.seen("/graphql").is_empty());
    }

    #[tokio::test]
    async fn a_last_ticket_closed_on_github_by_hand_readies_the_spec_pr_on_the_next_poll() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-hand";
        planner_on(&st, "ready-hand", Some("git@github.com:octo/ready-hand.git"));
        let body = spec_with_tickets(&st, &fake, slug);
        subs(&st, &fake, slug, &["closed", "open"]);
        let mut spec = gh_ticket(slug, 40, "open", "Spec.");
        spec["labels"] = json!([{ "name": "spec" }]);
        fake.in_turn("GET", &format!("/repos/{slug}/issues"), vec![
            (200, json!([spec.clone(), gh_ticket(slug, 42, "open", &body)])),
            // #42, the last open ticket, closed by hand on GitHub.
            (200, json!([gh_ticket(slug, 42, "closed", &body)])),
            (200, json!([gh_ticket(slug, 42, "closed", &body)])),
        ]);

        crate::mirror::sync_project(&st, "ready-hand", true).await.unwrap();
        assert!(fake.seen("/graphql").is_empty(), "nothing closed yet");
        // The poll sees #42 closed and notes it on the record itself.
        subs_on_github(&fake, slug, &["closed", "closed"]);
        crate::mirror::sync_project(&st, "ready-hand", false).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 1, "the last ticket closed: the PR is ready");
        assert_eq!(fake.seen("/graphql")[0].body["variables"], json!({ "id": "PR_500" }));
        // Seeing it closed again isn't a new close.
        crate::mirror::sync_project(&st, "ready-hand", false).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 1);
    }

    #[tokio::test]
    async fn a_migrated_delivery_asks_github_whether_its_tickets_are_closed() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-migrated";
        planner_on(&st, "ready-migrated", Some("git@github.com:octo/ready-migrated.git"));
        let body = spec_with_tickets(&st, &fake, slug);
        // As migrated from a spec PR recorded before Deliveries: every ticket "open" on the record, though
        // #41 closed long ago.
        crate::delivery::update(&st, slug, 40, |d| d.migrated = true);
        subs_on_github(&fake, slug, &["closed", "open"]);
        let mut spec = gh_ticket(slug, 40, "open", "Spec.");
        spec["labels"] = json!([{ "name": "spec" }]);
        fake.in_turn("GET", &format!("/repos/{slug}/issues"), vec![
            (200, json!([spec.clone(), gh_ticket(slug, 42, "open", &body)])),
            (200, json!([gh_ticket(slug, 42, "closed", &body)])),
        ]);
        crate::mirror::sync_project(&st, "ready-migrated", true).await.unwrap();
        assert!(fake.seen("/graphql").is_empty(), "#42 is still open on GitHub");

        // #42 closed by hand between polls: GitHub says both are closed now.
        subs_on_github(&fake, slug, &["closed", "closed"]);
        crate::mirror::sync_project(&st, "ready-migrated", false).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 1);
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!((d.state.as_str(), d.tickets[0].state.as_str()), ("ready", "open"), "#41's state on the record didn't matter");
    }

    /// Spec #40's draft spec PR #500 recorded as Loft's, on a repo whose issue list is empty.
    fn recorded_spec_pr(st: &AppState, fake: &Fake, slug: &str) {
        delivery_40(st, slug);
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
    }

    #[tokio::test]
    async fn a_failed_ready_call_is_retried_on_the_next_poll() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-retry";
        let a = planner_on(&st, "ready-retry", Some("git@github.com:octo/ready-retry.git"));
        spec_with_tickets(&st, &fake, slug);
        recorded_spec_pr(&st, &fake, slug);
        subs(&st, &fake, slug, &["closed", "open"]);
        crate::mirror::sync_project(&st, "ready-retry", true).await.unwrap();

        subs(&st, &fake, slug, &["closed", "closed"]);
        fake.in_turn("POST", "/graphql", vec![
            (502, json!({ "message": "Server Error" })),
            (200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } })),
        ]);
        let out = close(&st, &a, 42, &["def5678"]).await.unwrap();
        assert!(out.contains("couldn't mark spec PR #500 ready for review"), "{out}");

        // A change poll, not a full read: the failed call alone brings it back.
        crate::mirror::sync_project(&st, "ready-retry", false).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 2, "retried and done");
        crate::mirror::sync_project(&st, "ready-retry", false).await.unwrap();
        crate::mirror::sync_project(&st, "ready-retry", true).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 2, "a ready PR isn't asked again");
    }

    #[tokio::test]
    async fn a_full_read_readies_a_spec_pr_whose_last_close_loft_missed() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-missed";
        planner_on(&st, "ready-missed", Some("git@github.com:octo/ready-missed.git"));
        spec_with_tickets(&st, &fake, slug);
        recorded_spec_pr(&st, &fake, slug);
        // Every ticket closed while Loft was down.
        subs(&st, &fake, slug, &["closed", "closed"]);

        crate::mirror::sync_project(&st, "ready-missed", true).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 1);
        assert_eq!(fake.seen("/graphql")[0].body["variables"], json!({ "id": "PR_500" }));
    }

    #[tokio::test]
    async fn the_catch_up_leaves_ready_merged_and_closed_spec_prs_alone() {
        for (i, pr) in [json!({ "state": "open", "draft": false }), json!({ "state": "closed", "merged": true, "draft": false }), json!({ "state": "closed", "draft": true })].into_iter().enumerate() {
            let st = state();
            let fake = own_fake(&st);
            let (name, slug) = (format!("ready-settled-{i}"), format!("octo/ready-settled-{i}"));
            planner_on(&st, &name, Some(&format!("git@github.com:{slug}.git")));
            spec_with_tickets(&st, &fake, &slug);
            recorded_spec_pr(&st, &fake, &slug);
            subs(&st, &fake, &slug, &["closed", "closed"]);
            fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "body": "Closes #40", "state": pr["state"], "draft": pr["draft"] }));

            crate::mirror::sync_project(&st, &name, true).await.unwrap();
            crate::mirror::sync_project(&st, &name, true).await.unwrap();
            assert!(fake.seen("/graphql").is_empty(), "{pr}");
            assert_eq!(fake.seen(&format!("/repos/{slug}/pulls/500")).len(), 1, "settled after one look: {pr}");
        }
    }

    #[tokio::test]
    async fn the_catch_up_skips_a_spec_with_an_open_ticket() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-open";
        planner_on(&st, "ready-open", Some("git@github.com:octo/ready-open.git"));
        spec_with_tickets(&st, &fake, slug);
        recorded_spec_pr(&st, &fake, slug);
        subs(&st, &fake, slug, &["closed", "open"]);

        crate::mirror::sync_project(&st, "ready-open", true).await.unwrap();
        crate::mirror::sync_project(&st, "ready-open", false).await.unwrap();
        assert!(fake.seen("/graphql").is_empty());
        // The record says #42 is open: neither a full read nor a change poll asks GitHub (#197).
        assert!(fake.seen("/repos/octo/ready-open/pulls/500").is_empty());
        assert!(fake.seen("/repos/octo/ready-open/issues/40/sub_issues").is_empty());
    }

    #[tokio::test]
    async fn the_catch_up_skips_a_spec_with_no_tickets_yet() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-none";
        planner_on(&st, "ready-none", Some("git@github.com:octo/ready-none.git"));
        // A spec published with its PR, before its tickets.
        crate::delivery::record_spec(&st, slug, 40, "", "", "");
        crate::delivery::update(&st, slug, 40, |d| d.pr = Some(500));
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        subs(&st, &fake, slug, &[]);

        crate::mirror::sync_project(&st, "ready-none", true).await.unwrap();
        assert!(fake.seen(&format!("/repos/{slug}/pulls/500")).is_empty(), "a full read doesn't ask GitHub (#237)");
        assert!(fake.seen(&format!("/repos/{slug}/issues/40/sub_issues")).is_empty());

        // A call that failed is still retried.
        crate::delivery::update(&st, slug, 40, |d| d.ready_retry = true);
        crate::mirror::sync_project(&st, "ready-none", false).await.unwrap();
        assert_eq!(fake.seen(&format!("/repos/{slug}/pulls/500")).len(), 1);
    }

    #[tokio::test]
    async fn close_ticket_turns_down_what_isnt_an_open_ticket() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-no";
        let a = planner_on(&st, "close-no", Some("git@github.com:octo/close-no.git"));
        fake.route("GET", &format!("/repos/{slug}/issues/1"), 200, gh_ticket(slug, 1, "closed", ""));
        let mut pr = gh_ticket(slug, 2, "open", "");
        pr["pull_request"] = json!({});
        fake.route("GET", &format!("/repos/{slug}/issues/2"), 200, pr);
        let mut spec = gh_ticket(slug, 3, "open", "");
        spec["labels"] = json!([{ "name": "spec" }]);
        fake.route("GET", &format!("/repos/{slug}/issues/3"), 200, spec);

        assert_eq!(close(&st, &a, 1, &["abc1234"]).await.unwrap_err(), "#1 on octo/close-no is already closed: nothing was done.");
        assert_eq!(close(&st, &a, 2, &["abc1234"]).await.unwrap_err(), "#2 on octo/close-no isn't a ticket (an issue labelled `ready-for-agent`), so it wasn't closed.");
        assert_eq!(close(&st, &a, 3, &["abc1234"]).await.unwrap_err(), "#3 on octo/close-no isn't a ticket (an issue labelled `ready-for-agent`), so it wasn't closed.");
        assert!(close(&st, &a, 9, &["abc1234"]).await.unwrap_err().starts_with("Couldn't read #9 on octo/close-no: GitHub 404"));
        assert!(fake.seen(&format!("/repos/{slug}")).iter().all(|s| s.method == "GET"), "nothing was written");
    }

    #[tokio::test]
    async fn a_close_ticket_retry_after_a_failed_close_comments_once() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-retry";
        let a = planner_on(&st, "close-retry", Some("git@github.com:octo/close-retry.git"));
        // A standalone ticket: no Branch section, so no spec PR in the comment.
        fake.route("GET", &format!("/repos/{slug}/issues/5"), 200, gh_ticket(slug, 5, "open", "## What to build\n\nBlue."));
        fake.route("POST", &format!("/repos/{slug}/issues/5/comments"), 201, json!({}));
        fake.in_turn("PATCH", &format!("/repos/{slug}/issues/5"), vec![(502, json!({ "message": "Server Error" })), (200, gh_ticket(slug, 5, "closed", ""))]);

        let err = close(&st, &a, 5, &["abc1234"]).await.unwrap_err();
        assert_eq!(err, "Commented on #5 but couldn't close it: GitHub 502: Server Error. Loft tries the close again on its next poll.");
        assert!(close(&st, &a, 5, &["abc1234"]).await.unwrap().starts_with("Closed #5 as completed with the comment \"Done in `abc1234`.\""));
        assert_eq!(posts(&fake, slug, "issues/5/comments").len(), 1);
    }

    #[tokio::test]
    async fn a_ticket_closed_reopened_and_closed_again_gets_a_second_comment() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-reopen";
        let a = planner_on(&st, "close-reopen", Some("git@github.com:octo/close-reopen.git"));
        fake.route("GET", &format!("/repos/{slug}/issues/6"), 200, gh_ticket(slug, 6, "open", "## What to build\n\nGreen."));
        fake.route("POST", &format!("/repos/{slug}/issues/6/comments"), 201, json!({}));
        fake.route("PATCH", &format!("/repos/{slug}/issues/6"), 200, gh_ticket(slug, 6, "closed", ""));

        assert!(close(&st, &a, 6, &["abc1234"]).await.unwrap().starts_with("Closed #6 as completed with the comment \"Done in `abc1234`.\""));
        // Reopened on GitHub (the fake still serves it open), fixed, and closed again, even with the same commits.
        let out = close(&st, &a, 6, &["def5678"]).await.unwrap();
        assert!(out.starts_with("Closed #6 as completed with the comment \"Done in `def5678`.\""), "{out}");
        assert!(close(&st, &a, 6, &["def5678"]).await.unwrap().starts_with("Closed #6 as completed with the comment \"Done in `def5678`.\""));
        assert_eq!(posts(&fake, slug, "issues/6/comments"), [json!({ "body": "Done in `abc1234`." }), json!({ "body": "Done in `def5678`." }), json!({ "body": "Done in `def5678`." })]);
    }

    #[tokio::test]
    async fn a_retry_with_other_commits_after_a_failed_close_comments_what_it_says() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-other";
        let a = planner_on(&st, "close-other", Some("git@github.com:octo/close-other.git"));
        fake.route("GET", &format!("/repos/{slug}/issues/7"), 200, gh_ticket(slug, 7, "open", "## What to build\n\nRed."));
        fake.route("POST", &format!("/repos/{slug}/issues/7/comments"), 201, json!({}));
        fake.in_turn("PATCH", &format!("/repos/{slug}/issues/7"), vec![(502, json!({ "message": "Server Error" })), (200, gh_ticket(slug, 7, "closed", ""))]);

        close(&st, &a, 7, &["abc1234"]).await.unwrap_err();
        let out = close(&st, &a, 7, &["abc1234", "def5678"]).await.unwrap();
        assert!(out.starts_with("Closed #7 as completed with the comment \"Done in `abc1234`, `def5678`.\""), "{out}");
        assert_eq!(posts(&fake, slug, "issues/7/comments"), [json!({ "body": "Done in `abc1234`." }), json!({ "body": "Done in `abc1234`, `def5678`." })]);
    }

    #[tokio::test]
    async fn a_close_after_a_reopen_comments_whatever_earlier_tries_left() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-stale";
        let a = planner_on(&st, "close-stale", Some("git@github.com:octo/close-stale.git"));
        fake.route("GET", &format!("/repos/{slug}/issues/8"), 200, gh_ticket(slug, 8, "open", "## What to build\n\nPink."));
        fake.route("POST", &format!("/repos/{slug}/issues/8/comments"), 201, json!({}));
        fake.in_turn("PATCH", &format!("/repos/{slug}/issues/8"), vec![(502, json!({ "message": "Server Error" })), (200, gh_ticket(slug, 8, "closed", ""))]);

        // Fails with abc, succeeds with abc and def, then reopened and closed with abc alone.
        close(&st, &a, 8, &["abc1234"]).await.unwrap_err();
        close(&st, &a, 8, &["abc1234", "def5678"]).await.unwrap();
        assert!(close(&st, &a, 8, &["abc1234"]).await.unwrap().starts_with("Closed #8 as completed with the comment \"Done in `abc1234`.\""));
        assert_eq!(posts(&fake, slug, "issues/8/comments").len(), 3);
    }

    #[tokio::test]
    async fn a_close_after_a_failed_close_and_a_close_by_hand_comments_again() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/close-hand";
        let a = planner_on(&st, "close-hand", Some("git@github.com:octo/close-hand.git"));
        fake.route("GET", &format!("/repos/{slug}/issues/9"), 200, gh_ticket(slug, 9, "open", "## What to build\n\nGrey."));
        fake.route("POST", &format!("/repos/{slug}/issues/9/comments"), 201, json!({}));
        fake.in_turn("PATCH", &format!("/repos/{slug}/issues/9"), vec![(502, json!({ "message": "Server Error" })), (200, gh_ticket(slug, 9, "closed", ""))]);

        close(&st, &a, 9, &["abc1234"]).await.unwrap_err();
        // Closed by hand on GitHub after that, then reopened: GitHub keeps the last close's time.
        let mut reopened = gh_ticket(slug, 9, "open", "## What to build\n\nGrey.");
        reopened["closed_at"] = json!(crate::db::now());
        fake.route("GET", &format!("/repos/{slug}/issues/9"), 200, reopened);
        assert!(close(&st, &a, 9, &["abc1234"]).await.unwrap().starts_with("Closed #9 as completed with the comment \"Done in `abc1234`.\""));
        assert_eq!(posts(&fake, slug, "issues/9/comments").len(), 2);
    }

    #[test]
    fn the_planning_prompt_offers_the_spec_and_asks_the_seams() {
        let p = crate::grill::brief();
        for want in ["Turn this into a spec and tickets?", "Test seams", "round question", "## Problem Statement", "## Testing Decisions", "## Out of Scope", "mcp__loft__publish_spec"] {
            assert!(p.contains(want), "missing {want}");
        }
    }

    #[tokio::test]
    async fn without_a_github_repo_the_spec_stays_in_loft_and_github_is_never_called() {
        let st = state();
        let fake = own_fake(&st);
        let mut a = planner(&st, "spec-nowhere");
        let cases: Vec<(Agent, &str)> = {
            let no_project = a.clone();
            a.project = "spec-nowhere".into();
            st.db.save_agent(&a).unwrap();
            vec![
                (no_project, "you have no project"),
                (a.clone(), "isn't a git repo"),
                (planner_on(&st, "spec-no-origin", None), "has no `origin` remote"),
                (planner_on(&st, "spec-gitlab", Some("https://gitlab.com/octo/x.git")), "isn't on GitHub"),
            ]
        };
        for (who, why) in &cases {
            let out = publish(&st, who, "X").await.unwrap();
            assert!(out.starts_with("Saved the spec as a Loft issue labelled `spec`") && out.contains(why) && out.contains("nothing went to GitHub"), "{out}");
        }
        // A repo, but GitHub isn't connected: also kept in Loft.
        let b = planner_on(&st, "spec-no-token", Some("https://github.com/octo/spec-no-token"));
        let mut s = st.settings.get();
        s.github_token = String::new();
        st.settings.save(s).unwrap();
        let out = publish(&st, &b, "X").await.unwrap();
        assert!(out.contains("GitHub isn't connected"), "{out}");
        assert!(fake.seen("/").is_empty(), "no GitHub request at all: {:?}", fake.seen("/"));

        let specs: Vec<_> = st.db.list_tasks().unwrap().into_iter().filter(|t| t.labels == ["spec"]).collect();
        assert_eq!(specs.len(), 5, "one per project, the project-less one included");
        assert!(specs.iter().all(|t| t.github.is_none() && t.title == "X" && t.description == "## Problem Statement\n\nIt hurts." && t.status == "inbox"));
    }

    #[tokio::test]
    async fn a_lead_with_two_projects_files_each_plan_in_the_project_it_names() {
        let st = state();
        let fake = own_fake(&st);
        // Two projects with git and no GitHub origin: plans become Loft issues in the project they're for.
        let mut lead = planner_on(&st, "multi-web", None);
        planner_on(&st, "multi-api", None);
        (lead.role, lead.group, lead.projects) = ("lead".into(), "Acme".into(), vec!["multi-web".into(), "multi-api".into()]);
        st.db.save_agent(&lead).unwrap();
        let one = |title: &str| json!([ticket(title, &[])]);

        for project in ["multi-web", "multi-api"] {
            let out = mcp_call(&st, &lead, "publish_tickets", &json!({ "project": project, "tickets": one(&format!("Ticket for {project}")) })).await.unwrap().unwrap();
            assert!(out.starts_with("Saved a standalone ticket as a Loft issue"), "{out}");
            let t = st.db.list_tasks().unwrap().into_iter().find(|t| t.title == format!("Ticket for {project}")).unwrap();
            assert_eq!(t.project, project);
        }
        let spec = mcp_call(&st, &lead, "publish_spec", &json!({ "project": "multi-api", "title": "Api spec", "body": "## Problem Statement\n\nIt hurts." })).await.unwrap().unwrap();
        assert!(spec.contains("Loft issue"), "{spec}");
        assert_eq!(st.db.list_tasks().unwrap().into_iter().find(|t| t.title == "Api spec").unwrap().project, "multi-api");

        // A spec's tickets each go to the project they name, the spec's by default, all under the spec.
        let spec_id = st.db.list_tasks().unwrap().into_iter().find(|t| t.title == "Api spec").unwrap().id;
        let both = json!([ticket("Endpoint", &[]), { "project": "multi-web", "title": "Screen", "body": "## What to build\n\nScreen.", "blocked_by": ["Endpoint"] }]);
        let out = mcp_call(&st, &lead, "publish_tickets", &json!({ "project": "multi-api", "spec": spec_id, "tickets": both })).await.unwrap().unwrap();
        assert!(out.starts_with("Saved 2 tickets as Loft issues under the spec \"Api spec\""), "{out}");
        let by = |title: &str| st.db.list_tasks().unwrap().into_iter().find(|t| t.title == title).unwrap();
        let (endpoint, screen) = (by("Endpoint"), by("Screen"));
        assert_eq!((endpoint.project.as_str(), screen.project.as_str()), ("multi-api", "multi-web"));
        assert_eq!((endpoint.parent_id.as_deref(), screen.parent_id.as_deref()), (Some(spec_id.as_str()), Some(spec_id.as_str())));
        assert_eq!(screen.depends_on, std::slice::from_ref(&endpoint.id));
        let again = mcp_call(&st, &lead, "publish_tickets", &json!({ "project": "multi-api", "spec": spec_id, "tickets": both })).await.unwrap().unwrap();
        assert!(again.contains("Nothing new was created"), "{again}");

        // A plan or a ticket in a project outside its list: refused, listing its projects, and nothing is made.
        let before = st.db.list_tasks().unwrap().len();
        let err = mcp_call(&st, &lead, "publish_spec", &json!({ "project": "elsewhere", "title": "No", "body": "x" })).await.unwrap().unwrap_err();
        assert_eq!(err, "You don't work in elsewhere. Pass `project` as one of your projects: `multi-web`, `multi-api`.");
        let outside = json!([{ "project": "elsewhere", "title": "Lost", "body": "x" }]);
        let err = mcp_call(&st, &lead, "publish_tickets", &json!({ "tickets": outside })).await.unwrap().unwrap_err();
        assert_eq!(err, "\"Lost\": You don't work in elsewhere. Pass `project` as one of your projects: `multi-web`, `multi-api`.");
        assert_eq!(st.db.list_tasks().unwrap().len(), before);

        // None named: the starred main project (the first), never the group.
        mcp_call(&st, &lead, "publish_tickets", &json!({ "tickets": one("Where?") })).await.unwrap().unwrap();
        assert_eq!(by("Where?").project, "multi-web");

        // create_issue follows the same list: an issue in each of its projects, none outside.
        let find = |k: &str| Err::<Agent, _>(k.to_string());
        for project in ["multi-web", "multi-api"] {
            assert!(crate::tasks::mcp_call(&st, &lead, "create_issue", &json!({ "title": format!("Issue in {project}"), "project": project }), find).unwrap().is_ok());
        }
        assert!(crate::tasks::mcp_call(&st, &lead, "create_issue", &json!({ "title": "Outside", "project": "elsewhere" }), find).unwrap().is_err());

        // An agent with one project needs no `project`, as before.
        let solo = planner_on(&st, "multi-solo", None);
        assert!(mcp_call(&st, &solo, "publish_tickets", &json!({ "tickets": one("Solo ticket") })).await.unwrap().is_ok());
        assert!(fake.seen("/").is_empty(), "no GitHub request at all");
    }

    #[tokio::test]
    async fn a_spec_files_each_ticket_in_its_own_repo_as_sub_issues_with_cross_repo_blockers() {
        let (fake, st) = (fake_github(), state());
        let (api, web) = ("octo/cross-api", "octo/cross-web");
        let mut lead = planner_on(&st, "cross-api", Some("git@github.com:octo/cross-api.git"));
        planner_on(&st, "cross-web", Some("git@github.com:octo/cross-web.git"));
        (lead.role, lead.group, lead.projects) = ("lead".into(), "Acme".into(), vec!["cross-api".into(), "cross-web".into()]);
        st.db.save_agent(&lead).unwrap();
        crate::grill::tests::round(&st, &lead, &["Turn this into a spec and tickets?"]).unwrap();
        crate::grill::tests::send(&st, &lead).unwrap();
        tickets_repo(&fake, api, 40, &[41]);
        tickets_repo(&fake, web, 0, &[7]);
        let screen = |blocked_by: &[&str]| json!({ "project": "cross-web", "title": "Screen", "body": "## What to build\n\nScreen.", "blocked_by": blocked_by });
        let tickets = json!([screen(&["Endpoint"]), ticket("Endpoint", &[])]);

        let out = mcp_call(&st, &lead, "publish_tickets", &json!({ "spec": 40, "tickets": tickets })).await.unwrap().unwrap();
        assert_eq!(out, "Published 2 tickets as sub-issues of spec #40, blockers first, labelled `ready-for-agent` and assigned to octo-me:\n\
            - #41 Endpoint: https://github.com/octo/cross-api/issues/41\n\
            - octo/cross-web#7 Screen, blocked by #41: https://github.com/octo/cross-web/issues/7");
        // Each ticket in its own repo, both sub-issues of the spec, the web one blocked by the api one.
        assert_eq!(posts(&fake, api, "issues").iter().map(|i| i["title"].as_str().unwrap()).collect::<Vec<_>>(), ["Endpoint"]);
        assert_eq!(posts(&fake, web, "issues").iter().map(|i| i["title"].as_str().unwrap()).collect::<Vec<_>>(), ["Screen"]);
        assert_eq!(posts(&fake, api, "issues/40/sub_issues"), [json!({ "sub_issue_id": 4100 }), json!({ "sub_issue_id": 700 })]);
        assert_eq!(posts(&fake, web, "issues/7/dependencies/blocked_by"), [json!({ "issue_id": 4100 })]);

        // A retry finds both under the spec, the web one by its repo, and creates nothing twice.
        fake.route("GET", &format!("/repos/{api}/issues/40/sub_issues"), 200, json!([
            { "number": 41, "id": 4100, "title": "Endpoint", "repository_url": format!("{}/repos/{api}", crate::github::api_base()) },
            // GitHub's own spelling of the repo can differ in case from the git origin.
            { "number": 7, "id": 700, "title": "Screen", "repository_url": format!("{}/repos/octo/Cross-Web", crate::github::api_base()) },
        ]));
        let again = mcp_call(&st, &lead, "publish_tickets", &json!({ "spec": 40, "tickets": tickets })).await.unwrap().unwrap();
        assert!(again.contains("Nothing new was created") && again.matches(", already published").count() == 2, "{again}");
        assert_eq!((posts(&fake, api, "issues").len(), posts(&fake, web, "issues").len(), posts(&fake, api, "issues/40/sub_issues").len()), (1, 1, 2));

        // The end-of-planning card links the web ticket to its own repo.
        crate::grill::mcp_call(&st, &lead, "grill_end", &json!({ "summary": "Split." })).unwrap().unwrap();
        let urls: Vec<String> = summary(&st, &lead).meta["grill"]["end_card"]["tickets"].as_array().unwrap().iter().map(|t| t["url"].as_str().unwrap().to_string()).collect();
        assert_eq!(urls, ["https://github.com/octo/cross-api/issues/41", "https://github.com/octo/cross-web/issues/7"]);

        // A ticket in the spec's repo can't wait on one elsewhere: the Delivery would start it too early.
        let before = (posts(&fake, api, "issues").len(), posts(&fake, web, "issues").len());
        let flipped = json!([screen(&[]), ticket("Endpoint v2", &["Screen"])]);
        let err = mcp_call(&st, &lead, "publish_tickets", &json!({ "spec": 40, "tickets": flipped })).await.unwrap().unwrap_err();
        assert_eq!(err, "\"Endpoint v2\" is in octo/cross-api but blocked by \"Screen\" in octo/cross-web: a ticket in the spec's repo can only be blocked by tickets in that repo. Swap the blocker, or name the other repo as the primary one.");
        assert_eq!((posts(&fake, api, "issues").len(), posts(&fake, web, "issues").len()), before, "nothing was created");

        // A ticket for a project without a GitHub repo can't join a spec on GitHub.
        planner_on(&st, "cross-local", None);
        lead.projects.push("cross-local".into());
        let err = mcp_call(&st, &lead, "publish_tickets", &json!({ "spec": 40, "tickets": [{ "project": "cross-local", "title": "Offline", "body": "x" }] })).await.unwrap().unwrap_err();
        assert!(err.starts_with("\"Offline\" is for cross-local, which has no GitHub repo"), "{err}");
    }

    #[tokio::test]
    async fn a_plan_without_a_repo_becomes_linked_loft_issues_with_no_github_requests() {
        let st = state();
        let fake = own_fake(&st);
        let a = planner_on(&st, "plan-local", None);
        crate::grill::tests::round(&st, &a, &["Turn this into a spec and tickets?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();

        let out = publish(&st, &a, "Offline mode").await.unwrap();
        let spec = st.db.list_tasks().unwrap().into_iter().find(|t| t.labels == ["spec"]).unwrap();
        assert!(out.contains(&format!("spec: \"{}\"", spec.id)), "{out}");
        assert_eq!(publish(&st, &a, " offline  MODE ").await.unwrap(), format!("The spec \"offline MODE\" is already saved as Loft issue {}. Nothing new was created and its text wasn't changed.", spec.id));

        let tickets = json!([ticket("UI", &["Store"]), ticket("Store", &[]), ticket("Sync", &["Store", "UI"])]);
        let out = mcp_call(&st, &a, "publish_tickets", &json!({ "spec": spec.id, "tickets": tickets })).await.unwrap().unwrap();
        assert!(out.starts_with("Saved 3 tickets as Loft issues under the spec \"Offline mode\", blockers first, labelled `ready-for-agent`"), "{out}");
        let all = st.db.list_tasks().unwrap();
        let by = |title: &str| all.iter().find(|t| t.title == title).unwrap().clone();
        let (store, ui, sync) = (by("Store"), by("UI"), by("Sync"));
        for t in [&store, &ui, &sync] {
            assert_eq!((t.parent_id.as_deref(), t.labels.as_slice(), t.status.as_str(), t.project.as_str()), (Some(spec.id.as_str()), &["ready-for-agent".to_string()][..], "ready", "plan-local"));
            assert!(t.github.is_none() && t.agent_id.is_none() && t.created_by == a.id);
        }
        assert_eq!((store.depends_on.clone(), ui.depends_on.clone(), sync.depends_on.clone()), (vec![], vec![store.id.clone()], vec![store.id.clone(), ui.id.clone()]));

        // The same tickets again: nothing new. A number for a Loft spec, or a spec from elsewhere, is refused.
        let again = mcp_call(&st, &a, "publish_tickets", &json!({ "spec": spec.id, "tickets": tickets })).await.unwrap().unwrap();
        assert!(again.contains("Nothing new was created") && again.matches(", already saved").count() == 3, "{again}");
        assert_eq!(st.db.list_tasks().unwrap().len(), 4);
        assert!(mcp_call(&st, &a, "publish_tickets", &json!({ "spec": 7, "tickets": tickets })).await.unwrap().unwrap_err().contains("not a number"));
        assert!(mcp_call(&st, &a, "publish_tickets", &json!({ "spec": "nope", "tickets": tickets })).await.unwrap().unwrap_err().starts_with("There's no spec with the Loft issue id nope"));
        // A standalone ticket needs no spec.
        let one = mcp_call(&st, &a, "publish_tickets", &json!({ "tickets": [ticket("Docs", &[])] })).await.unwrap().unwrap();
        assert!(one.starts_with("Saved a standalone ticket as a Loft issue"), "{one}");

        // The end-of-planning card links to the Loft issues.
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Offline it is." })).unwrap().unwrap();
        let card = &summary(&st, &a).meta["grill"]["end_card"];
        assert_eq!(card["repo"], "");
        assert_eq!(card["spec"], json!({ "number": 0, "title": "Offline mode", "url": format!("/issues/{}", spec.id) }));
        assert_eq!(card["tickets"].as_array().unwrap().iter().map(|t| t["url"].as_str().unwrap().to_string()).collect::<Vec<_>>()[..3], [format!("/issues/{}", store.id), format!("/issues/{}", ui.id), format!("/issues/{}", sync.id)]);
        // Its Delivery, started from the card like a spec on GitHub.
        let d = crate::delivery::local_of(&st, &a.project, &spec.id).unwrap();
        assert_eq!((&card["next_message"], &card["delivery"]), (&json!(""), &json!({ "repo": d.repo, "spec": d.spec, "local": true })));
        assert!(fake.seen("/").is_empty(), "no GitHub request at all: {:?}", fake.seen("/"));
    }

    /// What GitHub answers to creating issue #n.
    fn created(slug: &str, n: i64, title: &str, label: &str) -> Value {
        json!({
            "number": n, "id": n * 100, "title": title, "body": format!("## What to build\n\n{title}."), "state": "open",
            "html_url": format!("https://github.com/{slug}/issues/{n}"), "updated_at": "2026-10-28T10:00:00Z",
            "labels": [{ "name": label }], "assignees": [{ "login": "octo-me" }],
        })
    }

    #[tokio::test]
    async fn published_tickets_become_linked_loft_issues_right_away() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/tickets-loft";
        let a = planner_on(&st, "tickets-loft", Some("https://github.com/octo/tickets-loft"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 0);
        fake.in_turn("POST", &format!("/repos/{slug}/issues"), vec![
            (201, created(slug, 40, "Offline mode", "spec")),
            (201, created(slug, 41, "Store", "ready-for-agent")),
            (201, created(slug, 42, "UI", "ready-for-agent")),
        ]);
        fake.route("GET", &format!("/repos/{slug}/issues/40"), 200, created(slug, 40, "Offline mode", "spec"));
        fake.route("GET", &format!("/repos/{slug}/issues/40/sub_issues"), 200, json!([]));
        fake.route("POST", &format!("/repos/{slug}/issues/40/sub_issues"), 201, json!({}));
        fake.route("POST", &format!("/repos/{slug}/issues/42/dependencies/blocked_by"), 201, json!({}));

        publish(&st, &a, "Offline mode").await.unwrap();
        publish_tickets(&st, &a, Some(40), vec![ticket("UI", &["Store"]), ticket("Store", &[])]).await.unwrap();
        let by = |n: i64| st.db.list_tasks().unwrap().into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == n)).unwrap();
        let (spec, store, ui) = (by(40), by(41), by(42));
        for t in [&store, &ui] {
            assert_eq!((t.status.as_str(), t.agent_id.as_deref(), t.parent_id.as_deref(), t.created_by.as_str()), ("ready", None, Some(spec.id.as_str()), a.id.as_str()));
            assert_eq!((t.labels.as_slice(), t.project.as_str()), (&["ready-for-agent".to_string()][..], "tickets-loft"));
        }
        assert_eq!(ui.depends_on, std::slice::from_ref(&store.id));
        assert_eq!(spec.labels, ["spec"]);

        // A retry makes no second Loft issue.
        fake.route("GET", &format!("/repos/{slug}/issues/40/sub_issues"), 200, json!([created(slug, 41, "Store", "ready-for-agent"), created(slug, 42, "UI", "ready-for-agent")]));
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, created(slug, 41, "Store", "ready-for-agent"));
        fake.route("GET", &format!("/repos/{slug}/issues/42"), 200, created(slug, 42, "UI", "ready-for-agent"));
        publish_tickets(&st, &a, Some(40), vec![ticket("UI", &["Store"]), ticket("Store", &[])]).await.unwrap();
        assert_eq!(st.db.list_tasks().unwrap().len(), 3);

        // The Lead hands one to an agent: a Loft-only change, so nothing is written to GitHub.
        let dev = crate::tasks::tests::agent(&st, "Dev");
        let writes_before = fake.seen("/repos/").iter().filter(|r| r.method != "GET").count();
        let Json(v) = crate::tasks::update(State(st.clone()), Path(store.id.clone()), Json(json!({ "agent_id": dev.id }))).await.unwrap();
        assert_eq!(v["agent_id"], json!(dev.id));
        assert_eq!(fake.seen("/repos/").iter().filter(|r| r.method != "GET").count(), writes_before);
    }

    #[tokio::test]
    async fn with_a_repo_the_plan_still_goes_to_github() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/plan-remote";
        let a = planner_on(&st, "plan-remote", Some("https://github.com/octo/plan-remote"));
        tickets_repo(&fake, slug, 60, &[60, 61]);
        let out = publish(&st, &a, "Remote mode").await.unwrap();
        assert!(out.starts_with("Published the spec as #60"), "{out}");
        assert!(mcp_call(&st, &a, "publish_tickets", &json!({ "spec": "60", "tickets": [ticket("Store", &[])] })).await.unwrap().unwrap_err().contains("pass the spec's GitHub issue number"));
        let out = publish_tickets(&st, &a, Some(60), vec![ticket("Store", &[])]).await.unwrap();
        assert!(out.contains("#61 Store"), "{out}");
        assert!(st.db.list_tasks().unwrap().iter().all(|t| !t.labels.iter().any(|l| l == "spec")), "no Loft-only spec was made");
        assert_eq!(posts(&fake, slug, "issues").len(), 2);
    }

    /// A repo with both labels and spec #spec, whose issue creations answer these numbers in turn (id = number * 100).
    fn tickets_repo(fake: &Fake, slug: &str, spec: i64, numbers: &[i64]) {
        repo(fake, slug, &["spec", "ready-for-agent"], 0);
        fake.in_turn("POST", &format!("/repos/{slug}/issues"), numbers.iter().map(|n| (201, json!({ "number": n, "id": n * 100 }))).collect());
        fake.route("GET", &format!("/repos/{slug}/issues/{spec}/sub_issues"), 200, json!([]));
        fake.route("POST", &format!("/repos/{slug}/issues/{spec}/sub_issues"), 201, json!({}));
        for n in numbers {
            fake.route("POST", &format!("/repos/{slug}/issues/{n}/dependencies/blocked_by"), 201, json!({}));
        }
    }

    fn ticket(title: &str, blocked_by: &[&str]) -> Value {
        json!({ "title": title, "body": format!("## What to build\n\n{title}."), "blocked_by": blocked_by })
    }

    async fn publish_tickets(st: &AppState, a: &Agent, spec: Option<i64>, tickets: Vec<Value>) -> Result<String, String> {
        mcp_call(st, a, "publish_tickets", &json!({ "spec": spec, "tickets": tickets })).await.unwrap()
    }

    #[tokio::test]
    async fn publishes_tickets_blockers_first_as_sub_issues_with_dependencies() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-new";
        let a = planner_on(&st, "tickets-new", Some("git@github.com:octo/tickets-new.git"));
        tickets_repo(&fake, slug, 40, &[41, 42, 43]);

        // Listed out of order: the API needs the store, the UI needs both.
        let out = publish_tickets(&st, &a, Some(40), vec![ticket("API", &["Store"]), ticket("Store", &[]), ticket("UI", &["api", "Store"])]).await.unwrap();
        assert_eq!(out, "Published 3 tickets as sub-issues of spec #40, blockers first, labelled `ready-for-agent` and assigned to octo-me:\n\
            - #41 Store: https://github.com/octo/tickets-new/issues/41\n\
            - #42 API, blocked by #41: https://github.com/octo/tickets-new/issues/42\n\
            - #43 UI, blocked by #42, #41: https://github.com/octo/tickets-new/issues/43");

        let issues = posts(&fake, slug, "issues");
        assert_eq!(issues.iter().map(|i| i["title"].as_str().unwrap()).collect::<Vec<_>>(), ["Store", "API", "UI"]);
        for i in &issues {
            assert_eq!((&i["labels"], &i["assignees"]), (&json!(["ready-for-agent"]), &json!(["octo-me"])));
        }
        assert_eq!(issues[1]["body"], "## What to build\n\nAPI.");
        assert_eq!(posts(&fake, slug, "issues/40/sub_issues"), [json!({ "sub_issue_id": 4100 }), json!({ "sub_issue_id": 4200 }), json!({ "sub_issue_id": 4300 })]);
        assert_eq!(posts(&fake, slug, "issues/42/dependencies/blocked_by"), [json!({ "issue_id": 4100 })]);
        assert_eq!(posts(&fake, slug, "issues/43/dependencies/blocked_by"), [json!({ "issue_id": 4200 }), json!({ "issue_id": 4100 })]);
        assert!(posts(&fake, slug, "issues/41/dependencies/blocked_by").is_empty());
        assert!(posts(&fake, slug, "labels").is_empty());
    }

    #[tokio::test]
    async fn tickets_under_a_spec_with_a_spec_pr_say_which_branch_they_go_on() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-branch";
        let a = planner_on(&st, "tickets-branch", Some("git@github.com:octo/tickets-branch.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 40);
        publish(&st, &a, "Rounds").await.unwrap();
        tickets_repo(&fake, slug, 40, &[41, 42]);
        // The branch comes from the Delivery: a read of the spec PR would fail the publish.
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 502, json!({ "message": "Server Error" }));

        // A body that already has a Branch section gets Loft's in its place, at the end.
        let stale = json!({ "title": "API", "body": "## What to build\n\nAPI.\n\n## Branch\n\nmain\n\n## Acceptance criteria\n\n- [ ] Works", "blocked_by": ["Store"] });
        publish_tickets(&st, &a, Some(40), vec![ticket("Store", &[]), stale.clone()]).await.unwrap();
        let section = "## Branch\n\nCommit on the spec branch `spec/40-rounds`. It ships in the spec PR https://github.com/octo/tickets-branch/pull/500.";
        let bodies: Vec<Value> = posts(&fake, slug, "issues").into_iter().skip(1).map(|i| i["body"].clone()).collect();
        assert_eq!(bodies, [
            json!(format!("## What to build\n\nStore.\n\n{section}")),
            json!(format!("## What to build\n\nAPI.\n\n## Acceptance criteria\n\n- [ ] Works\n\n{section}")),
        ]);

        // The retry leaves the published tickets as they are.
        let again = publish_tickets(&st, &a, Some(40), vec![ticket("Store", &[]), stale]).await.unwrap();
        assert!(again.contains("Nothing new was created"), "{again}");
        assert_eq!(posts(&fake, slug, "issues").len(), 3, "the spec and two tickets");
        assert!(fake.seen(&format!("/repos/{slug}/issues/4")).iter().all(|s| s.method != "PATCH"), "no ticket is edited");
    }

    #[tokio::test]
    async fn a_migrated_delivery_reads_its_branch_from_github_once() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-no-pr";
        let a = planner_on(&st, "tickets-no-pr", Some("git@github.com:octo/tickets-no-pr.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 40);
        publish(&st, &a, "Rounds").await.unwrap();
        tickets_repo(&fake, slug, 40, &[41, 42]);
        // As migrated from a spec PR recorded before Deliveries: no branch.
        crate::delivery::update(&st, slug, 40, |d| d.branch.clear());
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 502, json!({ "message": "Server Error" }));

        let err = publish_tickets(&st, &a, Some(40), vec![ticket("Store", &[])]).await.unwrap_err();
        assert_eq!(err, "Couldn't read spec PR #500 on octo/tickets-no-pr for its branch: GitHub 502: Server Error");
        assert_eq!(posts(&fake, slug, "issues").len(), 1, "no ticket goes up without its branch");

        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "head": { "ref": "spec/40-old-name" } }));
        publish_tickets(&st, &a, Some(40), vec![ticket("Store", &[])]).await.unwrap();
        // Kept on the Delivery: the next publish doesn't read it again.
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 502, json!({ "message": "Server Error" }));
        publish_tickets(&st, &a, Some(40), vec![ticket("API", &[])]).await.unwrap();
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().branch, "spec/40-old-name");
        assert!(posts(&fake, slug, "issues")[2]["body"].as_str().unwrap().contains("`spec/40-old-name`"));
    }

    #[tokio::test]
    async fn a_retry_after_a_partial_failure_creates_nothing_twice() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-partial";
        let a = planner_on(&st, "tickets-partial", Some("https://github.com/octo/tickets-partial"));
        tickets_repo(&fake, slug, 9, &[10]);
        // The first ticket goes up, then GitHub turns the second one down.
        fake.in_turn("POST", &format!("/repos/{slug}/issues"), vec![(201, json!({ "number": 10, "id": 1000 })), (502, json!({ "message": "Server Error" }))]);
        let tickets = || vec![ticket("First", &[]), ticket("Second", &["First"]), ticket("Third", &["Second"])];

        let err = publish_tickets(&st, &a, Some(9), tickets()).await.unwrap_err();
        assert!(err.starts_with("GitHub rejected the ticket \"Second\" on octo/tickets-partial: GitHub 502: Server Error. What was published so far stays"), "{err}");

        // GitHub now lists #10 under the spec; the retry publishes the rest only.
        fake.route("GET", &format!("/repos/{slug}/issues/9/sub_issues"), 200, json!([{ "number": 10, "id": 1000, "title": "First" }]));
        fake.in_turn("POST", &format!("/repos/{slug}/issues"), vec![(201, json!({ "number": 11, "id": 1100 })), (201, json!({ "number": 12, "id": 1200 }))]);
        for n in [11, 12] {
            fake.route("POST", &format!("/repos/{slug}/issues/{n}/dependencies/blocked_by"), 201, json!({}));
        }
        let out = publish_tickets(&st, &a, Some(9), tickets()).await.unwrap();
        assert!(out.contains("- #10 First, already published:") && out.contains("- #11 Second, blocked by #10:") && out.contains("- #12 Third, blocked by #11:"), "{out}");
        assert_eq!(posts(&fake, slug, "issues").iter().map(|i| i["title"].as_str().unwrap()).collect::<Vec<_>>(), ["First", "Second", "Second", "Third"]);
        assert_eq!(posts(&fake, slug, "issues/9/sub_issues"), [json!({ "sub_issue_id": 1000 }), json!({ "sub_issue_id": 1100 }), json!({ "sub_issue_id": 1200 })]);
        assert_eq!(posts(&fake, slug, "issues/11/dependencies/blocked_by"), [json!({ "issue_id": 1000 })]);

        // Everything is recorded: a third call creates and links nothing.
        let again = publish_tickets(&st, &a, Some(9), tickets()).await.unwrap();
        assert!(again.ends_with("Nothing new was created: every ticket was already published."), "{again}");
        assert_eq!(posts(&fake, slug, "issues").len(), 4);
        assert_eq!(posts(&fake, slug, "issues/9/sub_issues").len(), 3);
        assert_eq!(posts(&fake, slug, "issues/12/dependencies/blocked_by").len(), 1);
    }

    #[tokio::test]
    async fn overlapping_publishes_of_the_same_tickets_create_them_once() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-race";
        let a = planner_on(&st, "tickets-race", Some("git@github.com:octo/tickets-race.git"));
        tickets_repo(&fake, slug, 20, &[21, 22, 23, 24]);
        let tickets = || vec![ticket("Store", &[]), ticket("API", &["Store"])];

        // A retry that arrives while the first call is still publishing.
        let (x, y) = tokio::join!(publish_tickets(&st, &a, Some(20), tickets()), publish_tickets(&st, &a, Some(20), tickets()));
        assert_eq!(posts(&fake, slug, "issues").len(), 2, "{x:?} / {y:?}");
        assert_eq!(posts(&fake, slug, "issues/20/sub_issues").len(), 2);
        assert_eq!(posts(&fake, slug, "issues/22/dependencies/blocked_by").len(), 1);
        assert!([x.unwrap(), y.unwrap()].iter().any(|o| o.ends_with("every ticket was already published.")));
    }

    #[tokio::test]
    async fn tickets_and_links_made_but_never_recorded_are_adopted() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-lost";
        let a = planner_on(&st, "tickets-lost", Some("git@github.com:octo/tickets-lost.git"));
        tickets_repo(&fake, slug, 1, &[3]);
        // #2 is already under the spec; #3's link to it went through but the reply was lost.
        fake.route("GET", &format!("/repos/{slug}/issues/1/sub_issues"), 200, json!([{ "number": 2, "id": 200, "title": "Lost" }]));
        fake.route("POST", &format!("/repos/{slug}/issues/3/dependencies/blocked_by"), 422, json!({ "message": "Issue is already blocked by this issue" }));
        fake.route("GET", &format!("/repos/{slug}/issues/3/dependencies/blocked_by"), 200, json!([{ "number": 2, "id": 200 }]));

        let out = publish_tickets(&st, &a, Some(1), vec![ticket("lost", &[]), ticket("Next", &["Lost"])]).await.unwrap();
        assert!(out.contains("- #2 lost, already published:") && out.contains("- #3 Next, blocked by #2:"), "{out}");
        assert_eq!(posts(&fake, slug, "issues").len(), 1, "only Next is created");
        assert_eq!(posts(&fake, slug, "issues/1/sub_issues"), [json!({ "sub_issue_id": 300 })]);

        // A link GitHub refuses and doesn't have is an error the agent can retry.
        fake.route("GET", &format!("/repos/{slug}/issues/1/sub_issues"), 200, json!([{ "number": 2, "id": 200, "title": "Lost" }, { "number": 3, "id": 300, "title": "Next" }]));
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, json!({ "number": 4, "id": 400 }));
        fake.route("POST", &format!("/repos/{slug}/issues/4/dependencies/blocked_by"), 422, json!({ "message": "Validation Failed" }));
        let err = publish_tickets(&st, &a, Some(1), vec![ticket("Lost", &[]), ticket("Next", &["Lost"]), ticket("Last", &["Next"])]).await.unwrap_err();
        assert!(err.starts_with("Couldn't mark #4 as blocked by #3: GitHub 422: Validation Failed. What was published so far stays"), "{err}");
        assert_eq!(posts(&fake, slug, "issues").len(), 2);
    }

    #[tokio::test]
    async fn publishes_one_standalone_ticket_without_a_spec() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-alone";
        let a = planner_on(&st, "tickets-alone", Some("git@github.com:octo/tickets-alone.git"));
        repo(&fake, slug, &[], 0);
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, json!({ "number": 5, "id": 500 }));

        let out = publish_tickets(&st, &a, None, vec![ticket("Blue button", &[])]).await.unwrap();
        assert_eq!(out, "Published a standalone ticket, labelled `ready-for-agent` and assigned to octo-me:\n- #5 Blue button: https://github.com/octo/tickets-alone/issues/5");
        assert_eq!(posts(&fake, slug, "labels").len(), 2, "missing labels are created");
        assert_eq!(posts(&fake, slug, "issues")[0]["labels"], json!(["ready-for-agent"]));
        assert_eq!(posts(&fake, slug, "issues")[0]["body"], "## What to build\n\nBlue button.", "no Branch section");
        assert!(fake.seen(&format!("/repos/{slug}/issues/")).is_empty(), "no parent, no links");
        assert!(fake.seen(&format!("/repos/{slug}/issues?labels=ready-for-agent&state=open")).len() == 1);

        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([{ "number": 5, "id": 500, "title": "Blue button" }]));
        let again = publish_tickets(&st, &a, None, vec![ticket("blue  button", &[])]).await.unwrap();
        assert!(again.contains("#5 blue button, already published"), "{again}");
        assert_eq!(posts(&fake, slug, "issues").len(), 1);

        // Once #5 is closed it no longer lists as open: the same request is new work.
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, json!({ "number": 8, "id": 800 }));
        let later = publish_tickets(&st, &a, None, vec![ticket("Blue button", &[])]).await.unwrap();
        assert!(later.contains("- #8 Blue button: "), "{later}");
        assert_eq!(posts(&fake, slug, "issues").len(), 2);
    }

    #[tokio::test]
    async fn a_sub_issue_link_that_failed_is_made_on_the_retry() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-unlinked";
        let a = planner_on(&st, "tickets-unlinked", Some("git@github.com:octo/tickets-unlinked.git"));
        tickets_repo(&fake, slug, 20, &[21]);
        fake.route("POST", &format!("/repos/{slug}/issues/20/sub_issues"), 500, json!({ "message": "Server Error" }));

        let err = publish_tickets(&st, &a, Some(20), vec![ticket("Only", &[])]).await.unwrap_err();
        assert!(err.starts_with("Couldn't make #21 a sub-issue of spec #20: GitHub 500: Server Error. What was published so far stays"), "{err}");

        // The retry knows #21 but not its id, which links need: it reads it from GitHub.
        fake.route("POST", &format!("/repos/{slug}/issues/20/sub_issues"), 201, json!({}));
        fake.route("GET", &format!("/repos/{slug}/issues/21"), 200, json!({ "number": 21, "id": 2100, "title": "Only" }));
        let out = publish_tickets(&st, &a, Some(20), vec![ticket("Only", &[])]).await.unwrap();
        assert!(out.contains("- #21 Only, already published:"), "{out}");
        assert_eq!(posts(&fake, slug, "issues").len(), 1);
        assert_eq!(posts(&fake, slug, "issues/20/sub_issues"), [json!({ "sub_issue_id": 2100 }), json!({ "sub_issue_id": 2100 })]);
        // Read twice: once for the id the link needs, once to bring #21 in as a Loft issue (#74).
        assert_eq!(fake.seen(&format!("/repos/{slug}/issues/21")).len(), 2);
    }

    #[tokio::test]
    async fn turns_down_tickets_it_cant_publish() {
        let (_fake, st) = (fake_github(), state());
        let a = planner_on(&st, "tickets-bad", Some("git@github.com:octo/tickets-bad.git"));
        let err = |tickets: Vec<Value>, spec: Option<i64>| {
            let (st, a) = (st.clone(), a.clone());
            async move { publish_tickets(&st, &a, spec, tickets).await.unwrap_err() }
        };
        assert!(err(vec![ticket("A", &[]), ticket("B", &[])], None).await.starts_with("Several tickets belong to a spec"));
        assert_eq!(err(vec![], Some(1)).await, "tickets is empty");
        assert_eq!(err((0..51).map(|i| ticket(&format!("T{i}"), &[])).collect(), Some(1)).await, "at most 50 tickets at a time");
        assert_eq!(err(vec![json!({ "title": "A", "body": "  " })], Some(1)).await, "ticket 1 needs a title and a body");
        assert!(err(vec![ticket(&"x".repeat(201), &[])], Some(1)).await.ends_with("a title is at most 200 characters"));
        assert_eq!(err(vec![ticket("A", &[]), ticket("a", &[])], Some(1)).await, "two tickets are titled \"a\": every ticket needs its own title");
        assert_eq!(err(vec![ticket("A", &["Z"])], Some(1)).await, "\"A\" is blocked by \"Z\", which isn't one of these tickets");
        assert_eq!(err(vec![ticket("A", &["A"])], Some(1)).await, "\"A\" can't block itself");
        assert_eq!(err(vec![ticket("A", &["B"]), ticket("B", &["A"]), ticket("C", &[])], Some(1)).await, "these tickets block each other in a loop: \"A\", \"B\"");
    }

    #[test]
    fn the_planning_prompt_slices_tickets_and_covers_direct_requests() {
        let p = crate::grill::brief();
        for want in ["Publish these N tickets?", "Vertical slices", "tracer bullet", "expand–contract", "## Acceptance criteria", "mcp__loft__publish_tickets"] {
            assert!(p.contains(want), "missing {want}");
        }
        let always = crate::grill::prompt_section();
        for want in ["# Planning: direct requests", "Never start a grill unasked", "at most one short question", "/grill", "single standalone ticket", "mcp__loft__planning_guide"] {
            assert!(always.contains(want), "missing {want}");
        }
    }

    #[test]
    fn no_system_prompt_carries_the_implementation_rules() {
        let st = state();
        // Whatever its role: the rules come only with a Delivery wake, in its role's brief.
        for role in ["dev", "qa", "security", "code_reviewer", "lead", "custom"] {
            let mut a = crate::tasks::tests::agent(&st, role);
            a.role = role.into();
            let p = crate::agents::system_prompt(&st, &a);
            for gone in ["# Planning: implementation", "submit_ticket", "review_ticket", "Refs #", "spec branch", "`ticket/<N>-<slug>`", "and on how a ticket is implemented"] {
                assert!(!p.contains(gone), "{role}'s prompt still has {gone:?}");
            }
            assert!(p.contains("override your standing instructions on how specs and tickets are written, labelled, linked and published"));
            assert!(p.contains("When Loft wakes you for a step of a Delivery, the wake brings your role's brief and says what your instructions can add to it."));
        }
        assert!(crate::grill::prompt_section().len() < 2000, "{} bytes", crate::grill::prompt_section().len());
    }

    #[tokio::test]
    async fn the_planning_guide_gives_the_spec_and_ticket_templates() {
        let st = state();
        let a = crate::tasks::tests::agent(&st, "guide");
        let g = mcp_call(&st, &a, "planning_guide", &json!({})).await.unwrap().unwrap();
        assert!(g.contains("## Problem Statement") && g.contains("## Acceptance criteria") && !g.contains("# Planning: grilling"), "{g}");
        // Each ticket names its repo, the breakdown shows it, and the spec lists the repos it touches.
        assert!(g.contains("Repo: <project>") && g.contains("its repo, what it delivers") && g.contains("## Repos") && g.contains("one ticket per repo"), "{g}");
    }

    /// The summary message `grill_end` just posted.
    fn summary(st: &AppState, a: &Agent) -> crate::db::Message {
        st.db.list_messages(&a.id, 50, None).unwrap().into_iter().rev().find(|m| m.meta["grill"]["final"] == true).unwrap()
    }

    #[tokio::test]
    async fn a_grill_that_published_ends_with_the_card() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/card-team";
        let a = planner_on(&st, "card-team", Some("git@github.com:octo/card-team.git"));
        tickets_repo(&fake, slug, 40, &[40, 41, 42]);
        // The team has reviewers but no developer.
        for (name, role) in [("Checker", "code_reviewer"), ("Tester", "qa"), ("Guard", "security")] {
            let mut rev = crate::tasks::tests::agent(&st, name);
            (rev.group, rev.role) = ("card-team".into(), role.into());
            st.db.save_agent(&rev).unwrap();
        }

        crate::grill::tests::round(&st, &a, &["Turn this into a spec and tickets?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        publish(&st, &a, "Planning in rounds").await.unwrap();
        publish_tickets(&st, &a, Some(40), vec![ticket("API", &["Store"]), ticket("Store", &[])]).await.unwrap();
        // A retry adds nothing to the card.
        fake.route("GET", &format!("/repos/{slug}/issues/40/sub_issues"), 200, json!([{ "number": 41, "id": 4100, "title": "Store" }, { "number": 42, "id": 4200, "title": "API" }]));
        publish_tickets(&st, &a, Some(40), vec![ticket("API", &["Store"]), ticket("Store", &[])]).await.unwrap();

        let out = crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Rounds it is." })).unwrap().unwrap();
        assert!(out.contains("end-of-planning card"), "{out}");
        let m = summary(&st, &a);
        assert_eq!(m.content, "Rounds it is.");
        assert_eq!(m.meta["grill"]["end_card"], json!({
            "repo": slug,
            "spec": { "number": 40, "title": "Planning in rounds", "url": "https://github.com/octo/card-team/issues/40" },
            "tickets": [
                { "number": 41, "title": "Store", "url": "https://github.com/octo/card-team/issues/41" },
                { "number": 42, "title": "API", "url": "https://github.com/octo/card-team/issues/42" },
            ],
            "next_message": "",
            "delivery": { "repo": slug, "spec": 40 },
            "missing_roles": ["dev"],
            "setup": {
                "agent_id": "loft",
                "message": format!("Set up the team for the card-team group: it has no Dev yet. {} just planned spec #40: https://github.com/octo/card-team/issues/40. Propose the agents it needs and create the ones I approve.", a.name),
            },
        }));
    }

    #[tokio::test]
    async fn the_team_setup_is_sent_once_and_stays_sent() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "card-setup", Some("git@github.com:octo/card-setup.git"));
        tickets_repo(&fake, "octo/card-setup", 9, &[9]);
        // Busy, so the setup message is queued instead of starting a real claude run.
        st.rt.lock().unwrap().running.insert(crate::setup::LOFT_ID.into(), std::sync::Arc::new(tokio::sync::Notify::new()));
        crate::setup::ensure_loft(&st);
        crate::grill::tests::round(&st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        publish(&st, &a, "Setup").await.unwrap();
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        let card = summary(&st, &a);
        assert_eq!(card.meta["grill"]["end_card"]["setup"]["sent_at"], Value::Null);

        let click = || send_team_setup(State(st.clone()), Path((a.id.clone(), card.id)));
        let first = click().await.unwrap().0;
        assert!(first["message"]["meta"]["grill"]["end_card"]["setup"]["sent_at"].is_string(), "{first}");
        let second = click().await.unwrap().0;
        assert_eq!(second["duplicate"], true);

        // Reloading the chat shows the card as sent, and loft got the message once.
        let sent_at = &summary(&st, &a).meta["grill"]["end_card"]["setup"]["sent_at"];
        assert_eq!(sent_at, &first["message"]["meta"]["grill"]["end_card"]["setup"]["sent_at"]);
        let setup = card.meta["grill"]["end_card"]["setup"]["message"].as_str().unwrap();
        let to_loft = st.db.list_messages(crate::setup::LOFT_ID, 50, None).unwrap().into_iter().filter(|m| m.role == "user" && m.content == setup).count();
        assert_eq!(to_loft, 1);

        // A card without a setup, or another agent's message, sends nothing.
        let plain = crate::tasks::tests::agent(&st, "card-setup-other");
        assert_eq!(send_team_setup(State(st.clone()), Path((plain.id.clone(), card.id))).await.err().unwrap().0, axum::http::StatusCode::NOT_FOUND);
        let note = crate::agents::add_msg(&st, &a.id, None, "assistant", "text", "hi", json!({})).unwrap();
        assert_eq!(send_team_setup(State(st.clone()), Path((a.id.clone(), note.id))).await.err().unwrap().0, axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_card_lists_tickets_published_before_a_failure() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/card-partial";
        let a = planner_on(&st, "card-partial", Some("git@github.com:octo/card-partial.git"));
        tickets_repo(&fake, slug, 30, &[30]);
        // First goes up; Second is created but its sub-issue link fails; Third is never reached.
        fake.in_turn("POST", &format!("/repos/{slug}/issues/30/sub_issues"), vec![(201, json!({})), (500, json!({ "message": "Server Error" }))]);
        fake.route("POST", &format!("/repos/{slug}/issues/32/dependencies/blocked_by"), 201, json!({}));
        let tickets = || vec![ticket("First", &[]), ticket("Second", &["First"]), ticket("Third", &["Second"])];
        let numbers = |card: &Value| card["tickets"].as_array().unwrap().iter().map(|t| t["number"].as_i64().unwrap()).collect::<Vec<_>>();

        crate::grill::tests::round(&st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        publish(&st, &a, "Partial").await.unwrap();
        fake.in_turn("POST", &format!("/repos/{slug}/issues"), vec![(201, json!({ "number": 31, "id": 3100 })), (201, json!({ "number": 32, "id": 3200 }))]);
        let err = publish_tickets(&st, &a, Some(30), tickets()).await.unwrap_err();
        assert!(err.starts_with("Couldn't make #32 a sub-issue of spec #30"), "{err}");

        // The agent ends the grill without retrying: the card still lists both tickets that exist.
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Stopped." })).unwrap().unwrap();
        let card = &summary(&st, &a).meta["grill"]["end_card"];
        assert_eq!(card["spec"]["title"], "Partial");
        assert_eq!(card["tickets"][1], json!({ "number": 32, "title": "Second", "url": "https://github.com/octo/card-partial/issues/32" }));
        assert_eq!(numbers(card), [31, 32]);

        // A retry in the next grill creates Third only, and its card lists each ticket once.
        crate::grill::tests::round(&st, &a, &["Finish?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        fake.route("GET", &format!("/repos/{slug}/issues/30/sub_issues"), 200, json!([{ "number": 31, "id": 3100, "title": "First" }]));
        fake.route("POST", &format!("/repos/{slug}/issues/30/sub_issues"), 201, json!({}));
        fake.route("GET", &format!("/repos/{slug}/issues/32"), 200, json!({ "number": 32, "id": 3200, "title": "Second" }));
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, json!({ "number": 33, "id": 3300 }));
        fake.route("POST", &format!("/repos/{slug}/issues/33/dependencies/blocked_by"), 201, json!({}));
        publish_tickets(&st, &a, Some(30), tickets()).await.unwrap();
        publish_tickets(&st, &a, Some(30), tickets()).await.unwrap();
        assert_eq!(posts(&fake, slug, "issues").iter().map(|i| i["title"].as_str().unwrap()).collect::<Vec<_>>(), ["Partial", "First", "Second", "Third"]);
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        assert_eq!(numbers(&summary(&st, &a).meta["grill"]["end_card"]), [31, 32, 33]);
    }

    #[tokio::test]
    async fn the_setup_message_names_every_missing_role() {
        let (fake, st) = (fake_github(), state());
        for (project, n, team, roles, text) in [
            ("card-empty", 11, &[][..], json!(["dev", "code_reviewer", "qa", "security"]), "no Dev, Code reviewer, QA or Security yet"),
            ("card-qa", 12, &["qa"][..], json!(["dev", "code_reviewer", "security"]), "no Dev, Code reviewer or Security yet"),
            ("card-two", 13, &["dev", "qa"][..], json!(["code_reviewer", "security"]), "no Code reviewer or Security yet"),
        ] {
            let a = planner_on(&st, project, Some(&format!("git@github.com:octo/{project}.git")));
            tickets_repo(&fake, &format!("octo/{project}"), n, &[n]);
            for role in team {
                let mut x = crate::tasks::tests::agent(&st, role);
                (x.group, x.role) = (project.into(), role.to_string());
                st.db.save_agent(&x).unwrap();
            }
            crate::grill::tests::round(&st, &a, &["Spec?"]).unwrap();
            crate::grill::tests::send(&st, &a).unwrap();
            publish(&st, &a, "Roles").await.unwrap();
            crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
            let card = &summary(&st, &a).meta["grill"]["end_card"];
            assert_eq!(card["missing_roles"], roles, "{project}");
            assert_eq!(
                card["setup"]["message"],
                format!("Set up the team for the {project} group: it has {text}. {} just planned spec #{n}: https://github.com/octo/{project}/issues/{n}. Propose the agents it needs and create the ones I approve.", a.name),
            );
        }
    }

    /// A grill on `name` that published spec #40 (PR #500 on `spec/40-rounds`) and these tickets as #41,
    /// #42… in publishing order, ended with its card. The planner and the card's message.
    async fn planned(st: &AppState, fake: &Fake, name: &str, tickets: Vec<Value>) -> (Agent, crate::db::Message) {
        let slug = format!("octo/{name}");
        let a = planner_on(st, name, Some(&format!("git@github.com:{slug}.git")));
        let numbers: Vec<i64> = (0..tickets.len() as i64).map(|i| 41 + i).collect();
        tickets_repo(fake, &slug, 40, &[&[40][..], &numbers].concat());
        // The mirror's catch-up after publishing finds the spec PR still a draft.
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        for n in &numbers {
            fake.route("POST", &format!("/repos/{slug}/issues/{n}/labels"), 200, json!([{ "name": "in-progress" }]));
        }
        crate::grill::tests::round(st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(st, &a).unwrap();
        publish(st, &a, "Rounds").await.unwrap();
        publish_tickets(st, &a, Some(40), tickets).await.unwrap();
        crate::grill::mcp_call(st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        let m = summary(st, &a);
        (a, m)
    }

    /// An agent in `project` with `role`, busy, so a wake waits in its queue instead of starting claude.
    fn busy(st: &AppState, name: &str, project: &str, role: &str) -> Agent {
        let mut x = crate::tasks::tests::agent(st, name);
        (x.project, x.role) = (project.into(), role.into());
        st.db.save_agent(&x).unwrap();
        st.rt.lock().unwrap().running.insert(x.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        x
    }

    fn woken(st: &AppState, a: &Agent) -> Vec<String> {
        st.rt.lock().unwrap().queued.get(&a.id).cloned().unwrap_or_default()
    }

    async fn press(st: &AppState, a: &Agent, m: &crate::db::Message) -> Result<Value, crate::api::ApiError> {
        press_with(st, a, m, json!({})).await
    }

    /// Start delivery with these query parameters (`init_git`).
    async fn press_with(st: &AppState, a: &Agent, m: &crate::db::Message, q: Value) -> Result<Value, crate::api::ApiError> {
        let q = axum::extract::Query(serde_json::from_value(q).unwrap());
        crate::delivery::start_from_card(State(st.clone()), Path((a.id.clone(), m.id)), q).await.map(|j| j.0)
    }

    #[tokio::test]
    async fn publishing_a_spec_and_its_tickets_records_the_delivery() {
        let (fake, st) = (fake_github(), state());
        let (a, _) = planned(&st, &fake, "dl-record", vec![ticket("Store", &[]), ticket("API", &["Store"]), ticket("UI", &["API", "Store"])]).await;
        let d = st.db.get_delivery("octo/dl-record", 40).unwrap().unwrap();
        assert_eq!(
            (d.project.as_str(), d.planner.as_str(), d.branch.as_str(), d.pr, d.state.as_str(), d.current),
            ("dl-record", a.id.as_str(), "spec/40-rounds", Some(500), "planned", None)
        );
        let ts: Vec<_> = d.tickets.iter().map(|t| (t.number, t.title.as_str(), t.blocked_by.clone(), t.state.as_str())).collect();
        assert_eq!(ts, [(41, "Store", vec![], "open"), (42, "API", vec![41], "open"), (43, "UI", vec![42, 41], "open")]);
        // A retry adds nothing.
        publish_tickets(&st, &a, Some(40), vec![ticket("Store", &[])]).await.unwrap();
        assert_eq!(st.db.get_delivery("octo/dl-record", 40).unwrap().unwrap().tickets.len(), 3);
    }

    #[tokio::test]
    async fn start_delivery_wakes_the_dev_on_the_first_unblocked_ticket_once() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-start";
        let (a, m) = planned(&st, &fake, "dl-start", vec![ticket("API", &["Store"]), ticket("Store", &[])]).await;
        assert_eq!(m.meta["grill"]["end_card"]["delivery"], json!({ "repo": slug, "spec": 40 }));
        let dev = busy(&st, "Builder", "dl-start", "dev");
        busy(&st, "Builder elsewhere", "other", "dev");
        let planner_queue = woken(&st, &a).len();

        let v = press(&st, &a, &m).await.unwrap();
        assert_eq!(v["duplicate"], false);
        // Store (#41) blocks API (#42), so it goes first: labelled, and the Dev woken with the Dev brief.
        assert_eq!(posts(&fake, slug, "issues/41/labels"), [json!({ "labels": ["in-progress"] })]);
        let w = woken(&st, &dev);
        assert_eq!(w.len(), 1, "{w:?}");
        for want in [
            "[Loft Delivery: spec #40] This is not a message from the user. You are the Dev on this Delivery: build ticket #41 \"Store\" (https://github.com/octo/dl-start/issues/41).",
            "Work on the spec branch `spec/40-rounds`: pull it, commit there and push. Open no PR: the spec PR https://github.com/octo/dl-start/pull/500 is the spec's only one.",
            "each message ending with `Refs #41`",
            "Your own instructions can add anything to this brief, like how to build and check the work in this project, but they can't change the Delivery's tools, its branch or how the work closes or merges.",
        ] {
            assert!(w[0].contains(want), "missing {want:?} in {}", w[0]);
        }
        assert_eq!(woken(&st, &a).len(), planner_queue, "the planner isn't woken when the team has a Dev");
        let started = &v["message"]["meta"]["grill"]["end_card"]["delivery"]["started"];
        assert_eq!((&started["ticket"], &started["agent_name"]), (&json!(41), &json!("Builder")));
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!((d.state.as_str(), d.current, d.tickets[0].state.as_str()), ("running", Some(41), "started"));

        // Pressed again (or on another device): nothing new starts, and the card still says who has it.
        let again = press(&st, &a, &m).await.unwrap();
        assert_eq!(again["duplicate"], true);
        assert_eq!(woken(&st, &dev).len(), 1);
        assert_eq!(posts(&fake, slug, "issues/41/labels").len(), 1);
        assert_eq!(again["message"]["meta"]["grill"]["end_card"]["delivery"]["started"], *started);
    }

    #[tokio::test]
    async fn without_a_dev_the_planner_builds_and_a_closed_blocker_frees_the_next_ticket() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-solo";
        let (a, m) = planned(&st, &fake, "dl-solo", vec![ticket("Store", &[]), ticket("API", &["Store"])]).await;
        // Its grill run ends first, so the wake below waits in its queue.
        for _ in 0..200 {
            if !crate::agents::is_running(&st, &a.id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        st.rt.lock().unwrap().running.insert(a.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        busy(&st, "Reviewer", "dl-solo", "code_reviewer");
        // #41 is already done: #42, which it blocked, is next.
        crate::delivery::update(&st, slug, 40, |d| d.tickets[0].state = "closed".into());

        press(&st, &a, &m).await.unwrap();
        let w = woken(&st, &a);
        assert!(w.last().is_some_and(|p| p.contains("You are the Dev on this Delivery: build ticket #42 \"API\"")), "{w:?}");
        assert_eq!(posts(&fake, slug, "issues/42/labels").len(), 1);
        assert!(posts(&fake, slug, "issues/41/labels").is_empty());
    }

    #[tokio::test]
    async fn with_a_lead_the_delivery_runs_tickets_on_the_devs_it_assigns_them_to() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-lead";
        let (a, m) = planned(&st, &fake, "dl-lead", vec![ticket("Store", &[]), ticket("API", &[]), ticket("UI", &["Store"])]).await;
        let lead = busy(&st, "Lofty", "dl-lead", "lead");
        let first = busy(&st, "First", "dl-lead", "dev");
        let second = busy(&st, "Second", "dl-lead", "dev");
        for n in [41, 42, 43] {
            fake.route("POST", &format!("/repos/{slug}/issues/{n}/comments"), 201, json!({}));
            fake.route("GET", &format!("/repos/{slug}/issues/{n}"), 200, gh_ticket(slug, n, "open", "## What to build\n\nX."));
            fake.route("PATCH", &format!("/repos/{slug}/issues/{n}"), 200, gh_ticket(slug, n, "closed", "## What to build\n\nX."));
        }
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));

        // Start delivery wakes the Lead, not a Dev, with the tickets and the Devs.
        let v = press(&st, &a, &m).await.unwrap();
        assert_eq!(v["message"]["meta"]["grill"]["end_card"]["delivery"]["started"]["agent_name"], "Lofty");
        let w = woken(&st, &lead);
        assert!(w.len() == 1 && w[0].contains("#41") && w[0].contains("blocked by #41") && w[0].contains("First: free") && w[0].contains("`mcp__loft__assign_ticket`"), "{w:?}");
        assert!(woken(&st, &first).is_empty() && woken(&st, &second).is_empty());
        assert!(posts(&fake, slug, "issues/41/labels").is_empty());
        assert_eq!(press(&st, &a, &m).await.unwrap()["duplicate"], true);
        assert_eq!(woken(&st, &lead).len(), 1, "a second press doesn't wake the Lead again");
        let shown = crate::delivery::delivery_json(&st, &delivery(&st, slug));
        assert_eq!(shown["waiting_for_lead"], json!([41, 42]));

        // Only the Lead assigns, only ready tickets, only to a Dev of the team.
        let assign = |n: i64, dev: &str| json!({ "ticket": n, "dev": dev });
        assert_eq!(tool(&st, &first, "assign_ticket", assign(41, "Second")).await.unwrap_err(), "Only the team's Lead hands out the tickets of a Delivery.");
        assert_eq!(tool(&st, &lead, "assign_ticket", assign(41, "Lofty")).await.unwrap_err(), "\"Lofty\" isn't a Dev on your team.");
        assert_eq!(tool(&st, &lead, "assign_ticket", assign(43, "First")).await.unwrap_err(), "#43 is blocked by #41: assign it once they close.");

        // Two ready tickets at once, on two Devs.
        tool(&st, &lead, "assign_ticket", assign(41, "first")).await.unwrap();
        tool(&st, &lead, "assign_ticket", assign(42, "Second")).await.unwrap();
        assert!(woken(&st, &first).last().is_some_and(|p| p.contains("build ticket #41 \"Store\"")));
        assert!(woken(&st, &second).last().is_some_and(|p| p.contains("build ticket #42 \"API\"")));
        assert_eq!((posts(&fake, slug, "issues/41/labels").len(), posts(&fake, slug, "issues/42/labels").len()), (1, 1));
        assert!(tool(&st, &lead, "assign_ticket", assign(41, "Second")).await.unwrap_err().contains("already with a Dev"));
        let shown = crate::delivery::delivery_json(&st, &delivery(&st, slug));
        assert_eq!((shown["tickets"][0]["agent_name"].as_str(), shown["tickets"][1]["agent_name"].as_str(), &shown["waiting_for_lead"]), (Some("First"), Some("Second"), &json!([])));

        // Each submits its own; closing #41 wakes the Lead with #43 while #42 is still being built.
        assert!(tool(&st, &second, "submit_ticket", json!({ "number": 41, "commits": ["abc1234"] })).await.unwrap_err().contains("is with First"));
        tool(&st, &first, "submit_ticket", submit_41()).await.unwrap();
        let w = woken(&st, &lead);
        assert!(w.len() == 2 && w[1].contains("Ready to hand out now: #43"), "{w:?}");
        assert_eq!(delivery(&st, slug).state, "running");
        tool(&st, &lead, "assign_ticket", assign(43, "First")).await.unwrap();
        tool(&st, &second, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "running", "#43 is still open");
        let out = tool(&st, &first, "submit_ticket", json!({ "number": 43, "commits": ["9999abcd"] })).await.unwrap();
        assert!(out.contains("spec PR #500 is now ready for review"), "{out}");
    }

    #[tokio::test]
    async fn the_lead_reassigns_a_started_ticket_but_not_one_under_review() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-reassign";
        let (a, m) = planned(&st, &fake, "dl-reassign", vec![ticket("Store", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-reassign", "lead");
        let first = busy(&st, "First", "dl-reassign", "dev");
        let second = busy(&st, "Second", "dl-reassign", "dev");
        busy(&st, "Tester", "dl-reassign", "qa");
        press(&st, &a, &m).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        let err = tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "Second" })).await.unwrap_err();
        assert!(err.contains("already with a Dev") && err.contains("reassign"), "{err}");

        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "Second", "reassign": true })).await.unwrap();
        assert!(woken(&st, &second).last().is_some_and(|p| p.contains("build ticket #41 \"Store\"")));
        let shown = crate::delivery::delivery_json(&st, &delivery(&st, slug));
        assert_eq!(shown["tickets"][0]["agent_name"].as_str(), Some("Second"));
        assert!(tool(&st, &first, "submit_ticket", submit_41()).await.unwrap_err().contains("is with Second"));
        tool(&st, &second, "submit_ticket", submit_41()).await.unwrap();

        let err = tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First", "reassign": true })).await.unwrap_err();
        assert!(err.contains("under review"), "{err}");
    }

    #[tokio::test]
    async fn reassigning_a_ticket_back_for_changes_gives_the_new_dev_the_reviewers_notes() {
        let (fake, st) = (fake_github(), state());
        let (a, m) = planned(&st, &fake, "dl-reassign-notes", vec![ticket("Store", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-reassign-notes", "lead");
        let first = busy(&st, "First", "dl-reassign-notes", "dev");
        let second = busy(&st, "Second", "dl-reassign-notes", "dev");
        let qa = busy(&st, "Tester", "dl-reassign-notes", "qa");
        press(&st, &a, &m).await.unwrap();
        for p in ["issues/500/comments", "issues/500/labels", "labels", "issues/41/comments"] {
            fake.route("POST", &format!("/repos/octo/dl-reassign-notes/{p}"), 201, json!({}));
        }
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        tool(&st, &first, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();

        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "Second", "reassign": true })).await.unwrap();
        let brief = woken(&st, &second).last().cloned().unwrap();
        assert!(brief.contains("build ticket #41 \"Store\"") && brief.contains("Round 1 of changes was asked:\n\nOff by one."), "{brief}");
    }

    #[tokio::test]
    async fn reassigning_gives_the_new_dev_that_tickets_notes_not_the_last_ones_of_the_delivery() {
        let (fake, st) = (fake_github(), state());
        let (a, m) = planned(&st, &fake, "dl-reassign-two", vec![ticket("Store", &[]), ticket("Show", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-reassign-two", "lead");
        let first = busy(&st, "First", "dl-reassign-two", "dev");
        let second = busy(&st, "Second", "dl-reassign-two", "dev");
        let third = busy(&st, "Third", "dl-reassign-two", "dev");
        let qa = busy(&st, "Tester", "dl-reassign-two", "qa");
        press(&st, &a, &m).await.unwrap();
        for p in ["issues/500/comments", "issues/500/labels", "labels", "issues/41/comments", "issues/42/comments"] {
            fake.route("POST", &format!("/repos/octo/dl-reassign-two/{p}"), 201, json!({}));
        }
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 42, "dev": "Second" })).await.unwrap();
        tool(&st, &first, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &second, "submit_ticket", json!({ "number": 42, "commits": ["abc1234def"] })).await.unwrap();
        tool(&st, &qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();
        tool(&st, &qa, "review_ticket", json!({ "number": 42, "verdict": "changes", "notes": "Wrong colour." })).await.unwrap();

        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "Third", "reassign": true })).await.unwrap();
        let brief = woken(&st, &third).last().cloned().unwrap();
        assert!(brief.contains("Off by one.") && !brief.contains("Wrong colour."), "{brief}");
    }

    #[tokio::test]
    async fn reassigning_from_a_stalled_delivery_resumes_it_and_the_old_devs_run_end_stalls_nothing() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-reassign-stall";
        let (a, m) = planned(&st, &fake, "dl-reassign-stall", vec![ticket("Store", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-reassign-stall", "lead");
        let first = busy(&st, "First", "dl-reassign-stall", "dev");
        let second = busy(&st, "Second", "dl-reassign-stall", "dev");
        press(&st, &a, &m).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();

        // First's run ends without submitting: the Delivery stalls. The Lead moves the ticket.
        run_starts(&st, &first);
        crate::delivery::run_ended(&st, &first.id, false).await;
        assert_eq!(delivery(&st, slug).state, "stalled");
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "Second", "reassign": true })).await.unwrap();
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.stall_reason.as_str()), ("running", ""));
        assert!(woken(&st, &second).last().is_some_and(|p| p.contains("build ticket #41")));

        // A run of First that was still going when the ticket moved ends: it has nothing to submit.
        let (st2, fake2) = (state(), fake_github());
        let (a2, m2) = planned(&st2, &fake2, "dl-reassign-old", vec![ticket("Store", &[])]).await;
        let lead2 = busy(&st2, "Lofty", "dl-reassign-old", "lead");
        let old = busy(&st2, "First", "dl-reassign-old", "dev");
        busy(&st2, "Second", "dl-reassign-old", "dev");
        press(&st2, &a2, &m2).await.unwrap();
        tool(&st2, &lead2, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        run_starts(&st2, &old);
        tool(&st2, &lead2, "assign_ticket", json!({ "ticket": 41, "dev": "Second", "reassign": true })).await.unwrap();
        crate::delivery::run_ended(&st2, &old.id, false).await;
        assert_eq!(delivery(&st2, "octo/dl-reassign-old").state, "running");
    }

    #[tokio::test]
    async fn a_stall_stays_while_a_ticket_that_caused_it_waits_whatever_happens_to_the_others() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-stall-per-ticket";
        let (a, m) = planned(&st, &fake, "dl-stall-per-ticket", vec![ticket("Store", &[]), ticket("API", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-stall-per-ticket", "lead");
        let first = busy(&st, "First", "dl-stall-per-ticket", "dev");
        busy(&st, "Second", "dl-stall-per-ticket", "dev");
        let third = busy(&st, "Third", "dl-stall-per-ticket", "dev");
        busy(&st, "Tester", "dl-stall-per-ticket", "qa");
        press(&st, &a, &m).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 42, "dev": "Second" })).await.unwrap();

        // First's run ends without submitting #41: the Delivery stalls on it.
        run_starts(&st, &first);
        crate::delivery::run_ended(&st, &first.id, false).await;
        assert_eq!((delivery(&st, slug).state.as_str(), delivery(&st, slug).stalled_on), ("stalled", vec![41]));

        // Reassigning and submitting #42 leave it stalled on #41.
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 42, "dev": "Third", "reassign": true })).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "stalled");
        tool(&st, &third, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.stalled_on.as_slice()), ("stalled", &[41][..]));

        // Resuming wakes First on #41 again.
        let before = woken(&st, &first).len();
        crate::delivery::resume(&st, slug, 40).await.unwrap();
        assert!(woken(&st, &first).len() > before);
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.stalled_on.len()), ("running", 0));

        // Moving #41 itself, the ticket that stalled it, clears the stall.
        run_starts(&st, &first);
        crate::delivery::run_ended(&st, &first.id, false).await;
        assert_eq!(delivery(&st, slug).state, "stalled");
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "Second", "reassign": true })).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "running");
    }

    #[tokio::test]
    async fn a_lead_whose_main_project_is_another_one_assigns_the_specs_tickets() {
        let (fake, st) = (fake_github(), state());
        let (a, m) = planned(&st, &fake, "dl-lead-multi", vec![ticket("Store", &[])]).await;
        let mut lead = busy(&st, "Lofty", "api", "lead");
        lead.projects = vec!["api".into(), "dl-lead-multi".into()];
        st.db.save_agent(&lead).unwrap();
        busy(&st, "First", "dl-lead-multi", "dev");
        press(&st, &a, &m).await.unwrap();
        assert_eq!(woken(&st, &lead).len(), 1, "the Lead is woken");
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        assert_eq!(delivery(&st, "octo/dl-lead-multi").current, Some(41));
    }

    #[tokio::test]
    async fn resuming_a_stalled_delivery_wakes_the_dev_the_lead_assigned() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-lead-resume";
        let (a, m) = planned(&st, &fake, "dl-lead-resume", vec![ticket("Store", &[]), ticket("API", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-lead-resume", "lead");
        let first = busy(&st, "First", "dl-lead-resume", "dev");
        let second = busy(&st, "Second", "dl-lead-resume", "dev");
        press(&st, &a, &m).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 42, "dev": "Second" })).await.unwrap();
        crate::delivery::stall_lost_runs(&st);
        assert_eq!(delivery(&st, slug).state, "stalled");
        let line = crate::delivery::resume(&st, slug, 40).await.unwrap();
        assert_eq!(line, "Resumed spec #40. First is building #41 again. Second is building #42 again.");
        assert_eq!(delivery(&st, slug).state, "running");
        // Both tickets in flight: each Dev is woken again on its own ticket.
        let (w1, w2) = (woken(&st, &first), woken(&st, &second));
        assert!(w1.len() == 2 && w1[1].contains("build ticket #41") && w1[1].contains("This is a resume"), "{w1:?}");
        assert!(w2.len() == 2 && w2[1].contains("build ticket #42") && w2[1].contains("This is a resume"), "{w2:?}");
    }

    #[tokio::test]
    async fn assign_ticket_refuses_a_delivery_with_no_project() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-no-project";
        let (a, m) = planned(&st, &fake, "dl-no-project", vec![ticket("Store", &[])]).await;
        let mut lead = busy(&st, "Lofty", "dl-no-project", "lead");
        busy(&st, "First", "dl-no-project", "dev");
        press(&st, &a, &m).await.unwrap();
        // A Delivery migrated from before Deliveries had projects, and a Lead with no main project.
        crate::delivery::update(&st, slug, 40, |d| d.project = String::new());
        lead.project = String::new();
        st.db.save_agent(&lead).unwrap();
        let err = tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap_err();
        assert!(err.contains("has no project"), "{err}");
        assert_eq!(delivery(&st, slug).current, None);
    }

    #[tokio::test]
    async fn a_lead_run_that_ends_without_assigning_stalls_the_delivery_and_resume_wakes_it_again() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-lead-lost";
        let (a, m) = planned(&st, &fake, "dl-lead-lost", vec![ticket("Store", &[]), ticket("API", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-lead-lost", "lead");
        busy(&st, "First", "dl-lead-lost", "dev");
        press(&st, &a, &m).await.unwrap();

        // The Lead's run ends without assign_ticket.
        run_starts(&st, &lead);
        crate::delivery::run_ended(&st, &lead.id, false).await;
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.resume_to.as_str(), d.stall_reason.as_str()), ("stalled", "running", "no_call"));
        crate::delivery::resume(&st, slug, 40).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "running");
        let w = woken(&st, &lead);
        assert!(w.last().is_some_and(|p| p.contains("Ready to hand out now: #41, #42")), "{w:?}");

        // A run that did assign doesn't stall it.
        tool(&st, &lead, "assign_ticket", json!({ "ticket": 41, "dev": "First" })).await.unwrap();
        crate::delivery::run_ended(&st, &lead.id, false).await;
        assert_eq!(delivery(&st, slug).state, "running");
    }

    #[tokio::test]
    async fn a_restart_stalls_a_lead_delivery_waiting_for_its_hand_out_and_resume_wakes_the_lead() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-lead-restart";
        let (a, m) = planned(&st, &fake, "dl-lead-restart", vec![ticket("Store", &[]), ticket("API", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-lead-restart", "lead");
        busy(&st, "First", "dl-lead-restart", "dev");
        press(&st, &a, &m).await.unwrap();
        // Nothing is in flight: the Lead's wake was lost with the restart.
        crate::delivery::stall_lost_runs(&st);
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.resume_to.as_str(), d.stall_reason.as_str()), ("stalled", "running", "restart"));
        crate::delivery::resume(&st, slug, 40).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "running");
        let w = woken(&st, &lead);
        assert!(w.last().is_some_and(|p| p.contains("Ready to hand out now: #41, #42")), "{w:?}");
    }

    #[tokio::test]
    async fn a_lead_on_a_team_with_no_dev_leaves_start_delivery_as_before() {
        let (fake, st) = (fake_github(), state());
        let (a, m) = planned(&st, &fake, "dl-nodev", vec![ticket("Store", &[])]).await;
        let lead = busy(&st, "Lofty", "dl-nodev", "lead");
        press(&st, &a, &m).await.unwrap();
        assert_eq!(st.db.get_delivery("octo/dl-nodev", 40).unwrap().unwrap().current, Some(41));
        assert!(woken(&st, &lead).is_empty());
    }

    #[tokio::test]
    async fn start_delivery_fails_cleanly_when_github_refuses_the_label_or_nothing_is_ready() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-fail";
        let (a, m) = planned(&st, &fake, "dl-fail", vec![ticket("Store", &[])]).await;
        let dev = busy(&st, "Builder", "dl-fail", "dev");
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 502, json!({ "message": "Server Error" }));
        let err = press(&st, &a, &m).await.unwrap_err();
        assert!(err.1.starts_with("The Delivery didn't start. Couldn't label #41 `in-progress`: GitHub 502"), "{}", err.1);
        assert!(woken(&st, &dev).is_empty());
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().state, "planned", "a later press can still start it");

        crate::delivery::update(&st, slug, 40, |d| d.tickets[0].blocked_by = vec![99]);
        let err = press(&st, &a, &m).await.unwrap_err();
        assert!(err.1.contains("has no ticket ready to start"), "{}", err.1);
    }

    /// Where a Delivery stands on ticket #n: its state, the current ticket, and the next one to start.
    fn standing(st: &AppState, slug: &str, n: i64) -> (String, Option<i64>, Option<i64>) {
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        let t = d.ticket(n).unwrap().state.clone();
        (t, d.current, crate::delivery::next_ticket(&d).map(|t| t.number))
    }

    #[tokio::test]
    async fn close_ticket_closes_the_ticket_on_its_delivery_and_frees_the_one_it_blocked() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-close";
        let (a, m) = planned(&st, &fake, "dl-close", vec![ticket("Store", &[]), ticket("API", &["Store"])]).await;
        busy(&st, "Builder", "dl-close", "dev");
        press(&st, &a, &m).await.unwrap();
        assert_eq!(standing(&st, slug, 41), ("started".into(), Some(41), None), "#42 waits for #41");

        // No "## Branch" section: the spec PR comes from the Delivery.
        let body = "## What to build\n\nStore.";
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "open", body));
        fake.route("POST", &format!("/repos/{slug}/issues/41/comments"), 201, json!({}));
        fake.route("PATCH", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "closed", body));
        let out = close(&st, &a, 41, &["abc1234"]).await.unwrap();
        assert!(out.contains("\"Done in `abc1234`, part of #500.\""), "{out}");
        assert_eq!(standing(&st, slug, 41), ("closed".into(), None, Some(42)));
    }

    #[tokio::test]
    async fn a_ticket_closed_by_hand_on_github_closes_on_its_delivery() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/dl-hand";
        planner_on(&st, "dl-hand", Some("git@github.com:octo/dl-hand.git"));
        delivery_40(&st, slug);
        crate::delivery::update(&st, slug, 40, |d| {
            (d.state, d.current) = ("running".into(), Some(41));
            d.tickets[0].state = "started".into();
            d.tickets[1].blocked_by = vec![41];
        });
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        let body = "## What to build\n\nStore.";
        fake.in_turn("GET", &format!("/repos/{slug}/issues"), vec![
            (200, json!([gh_ticket(slug, 41, "open", body), gh_ticket(slug, 42, "open", body)])),
            (200, json!([gh_ticket(slug, 41, "closed", body)])),
        ]);
        crate::mirror::sync_project(&st, "dl-hand", true).await.unwrap();
        assert_eq!(standing(&st, slug, 41), ("started".into(), Some(41), None));
        crate::mirror::sync_project(&st, "dl-hand", false).await.unwrap();
        assert_eq!(standing(&st, slug, 41), ("closed".into(), None, Some(42)));
        assert!(fake.seen("/graphql").is_empty(), "#42 is still open: the spec PR stays a draft");
    }

    #[tokio::test]
    async fn a_poll_waits_for_a_delivery_step_and_keeps_what_it_wrote() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/dl-race";
        planner_on(&st, "dl-race", Some("git@github.com:octo/dl-race.git"));
        delivery_40(&st, slug);
        crate::delivery::update(&st, slug, 40, |d| {
            (d.state, d.current) = ("running".into(), Some(41));
            d.tickets[0].state = "started".into();
            d.tickets[1].blocked_by = vec![41];
        });
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        let body = "## What to build\n\nStore.";
        fake.in_turn("GET", &format!("/repos/{slug}/issues"), vec![
            (200, json!([gh_ticket(slug, 41, "open", body), gh_ticket(slug, 42, "open", body)])),
            (200, json!([gh_ticket(slug, 41, "closed", body)])),
        ]);
        crate::mirror::sync_project(&st, "dl-race", true).await.unwrap();

        // A Delivery step holds the lock (finish closing #41 and starting #42) while the poll sees #41 closed.
        let held = st.publish_locks.of(slug).lock_owned().await;
        let poll = tokio::spawn({
            let st = st.clone();
            async move { crate::mirror::sync_project(&st, "dl-race", false).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!poll.is_finished(), "the poll waits for the step");
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!((d.current, d.tickets[0].state.as_str()), (Some(41), "started"), "the poll hasn't touched the record while the step holds the lock");
        crate::delivery::update(&st, slug, 40, |d| {
            d.tickets[0].state = "closed".into();
            d.tickets[1].state = "started".into();
            d.current = Some(42);
        });
        drop(held);
        poll.await.unwrap().unwrap();
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!((d.current, d.tickets[0].state.as_str(), d.tickets[1].state.as_str()), (Some(42), "closed", "started"), "the step's write survives the poll");
    }

    #[tokio::test]
    async fn a_ticket_its_delivery_doesnt_list_finds_it_by_its_branch_section() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/dl-fallback";
        let a = planner_on(&st, "dl-fallback", Some("git@github.com:octo/dl-fallback.git"));
        // As migrated from before Deliveries with no sub-issue links: the record has no tickets.
        crate::delivery::record_spec(&st, slug, 40, "", "", "");
        crate::delivery::update(&st, slug, 40, |d| d.pr = Some(500));
        let body = format!("## What to build\n\nStore.\n\n## Branch\n\nCommit on the spec branch `spec/40-rounds`. It ships in the spec PR https://github.com/{slug}/pull/500.");
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "open", &body));
        fake.route("POST", &format!("/repos/{slug}/issues/41/comments"), 201, json!({}));
        fake.route("PATCH", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "closed", &body));
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        subs(&st, &fake, slug, &["closed"]);
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));

        let out = close(&st, &a, 41, &["abc1234"]).await.unwrap();
        assert!(out.contains("part of #500") && out.contains("spec PR #500 is now ready for review"), "{out}");
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().state, "ready", "found by its spec PR");
    }

    async fn tool(st: &AppState, a: &Agent, name: &str, args: Value) -> Result<String, String> {
        crate::delivery::mcp_call(st, a, name, &args).await.unwrap()
    }

    /// A started Delivery on `name`: #41 then #42 (blocked by #41), #41 with the Dev, and the team's other
    /// `roles`, all busy. What reviewing and closing #41 needs on the fake is routed.
    async fn building(st: &AppState, fake: &Fake, name: &str, roles: &[&str]) -> (Agent, Vec<Agent>) {
        let slug = format!("octo/{name}");
        let (a, m) = planned(st, fake, name, vec![ticket("Store", &[]), ticket("API", &["Store"])]).await;
        let dev = busy(st, "Builder", name, "dev");
        let team = roles.iter().map(|r| busy(st, &format!("{r} agent"), name, r)).collect();
        press(st, &a, &m).await.unwrap();
        for p in ["issues/500/comments", "issues/500/labels", "labels", "issues/41/comments"] {
            fake.route("POST", &format!("/repos/{slug}/{p}"), 201, json!({}));
        }
        fake.route("DELETE", &format!("/repos/{slug}/issues/500/labels/changes-requested"), 200, json!([]));
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(&slug, 41, "open", "## What to build\n\nStore."));
        fake.route("PATCH", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(&slug, 41, "closed", "## What to build\n\nStore."));
        (dev, team)
    }

    fn submit_41() -> Value {
        json!({ "number": 41, "commits": ["abc1234def", "fed4321abc"] })
    }

    /// `a` set to Always with a Session, so any ticket boundary clears it.
    fn always_fresh(st: &AppState, a: &Agent) {
        st.db.update_agent(&a.id, |a| (a.fresh_context, a.session_id) = ("always".into(), Some("s-ticket".into()))).unwrap();
    }

    /// Whether the run that takes `a`'s queued wake starts by clearing its context. The agent is busy in these
    /// tests, so this runs that step by hand. It gets a Session back for the next check.
    fn cleared_at_next_run(st: &AppState, a: &Agent) -> bool {
        let n = || st.db.list_messages(&a.id, 400, None).unwrap().iter().filter(|m| m.kind == "clear").count();
        let before = n();
        crate::agents::fresh_boundary(st, &a.id);
        st.db.update_agent(&a.id, |a| a.session_id = Some("s-ticket".into())).unwrap();
        n() > before
    }

    #[tokio::test]
    async fn the_dev_keeps_its_context_through_review_rounds_of_a_ticket() {
        let (fake, st) = (fake_github(), state());
        let (dev, team) = building(&st, &fake, "fc-rounds", &["qa"]).await;
        fake.route("POST", "/repos/octo/fc-rounds/issues/41/labels", 200, json!([]));
        always_fresh(&st, &dev);
        // `building` already handed #41 to the Dev: a new ticket for it.
        assert!(cleared_at_next_run(&st, &dev), "a new ticket");
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();
        assert!(woken(&st, &dev).last().unwrap().contains("asked for changes on ticket #41"));
        assert!(!cleared_at_next_run(&st, &dev), "round 2 of #41 needs round 1");
    }

    #[tokio::test]
    async fn a_reviewer_is_cleared_for_a_new_ticket_but_not_to_review_the_same_one_again() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/fc-reviewer";
        let (dev, team) = building(&st, &fake, "fc-reviewer", &["qa"]).await;
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        let qa = &team[0];
        always_fresh(&st, qa);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(cleared_at_next_run(&st, qa), "reviewing #41 for the first time");
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(!cleared_at_next_run(&st, qa), "#41 again");
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "Fixed." })).await.unwrap();
        assert_eq!(standing(&st, slug, 41), ("closed".into(), Some(42), None));
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["1234567abc"] })).await.unwrap();
        assert!(cleared_at_next_run(&st, qa), "a new ticket, #42");
    }

    #[tokio::test]
    async fn a_lead_building_alone_is_cleared_before_its_first_ticket() {
        let (fake, st) = (fake_github(), state());
        let (a, m) = planned(&st, &fake, "fc-solo", vec![ticket("Store", &[])]).await;
        for _ in 0..200 {
            if !crate::agents::is_running(&st, &a.id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        st.rt.lock().unwrap().running.insert(a.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        // The Lead that ran the grill is the whole team: it builds the tickets.
        st.db.update_agent(&a.id, |a| a.role = "lead".into()).unwrap();
        always_fresh(&st, &a);
        let lead = st.db.get_agent(&a.id).unwrap().unwrap();
        assert!(crate::delivery::fresh_context_applies(&st, &lead));
        press(&st, &a, &m).await.unwrap();
        assert!(woken(&st, &a).last().is_some_and(|p| p.contains("build ticket #41")));
        assert!(cleared_at_next_run(&st, &a), "the grill's context goes before ticket #41");
    }

    #[tokio::test]
    async fn submit_wakes_qa_and_security_with_their_briefs() {
        let (fake, st) = (fake_github(), state());
        let (dev, team) = building(&st, &fake, "rv-submit", &["qa", "security", "code_reviewer"]).await;
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.starts_with("Submitted #41: qa agent (QA) and security agent (Security) are reviewing it."), "{out}");
        let (qa, sec, cr) = (woken(&st, &team[0]), woken(&st, &team[1]), woken(&st, &team[2]));
        assert!(cr.is_empty(), "Code review comes once, at the end of the spec: {cr:?}");
        assert_eq!((qa.len(), sec.len()), (1, 1));
        for (w, who) in [(&qa[0], "QA"), (&sec[0], "Security")] {
            for want in [
                format!("[Loft Delivery: spec #40] This is not a message from the user. You are {who} on this Delivery: review ticket #41 \"Store\""),
                "submitted as `abc1234`, `fed4321` on the spec branch `spec/40-rounds` (spec PR https://github.com/octo/rv-submit/pull/500)".into(),
                "call `mcp__loft__review_ticket` with 41, `verdict` \"pass\" or \"changes\"".into(),
            ] {
                assert!(w.contains(&want), "missing {want:?} in {w}");
            }
        }
        assert!(qa[0].contains("acceptance criteria") && sec[0].contains("security problems"));
        let d = st.db.get_delivery("octo/rv-submit", 40).unwrap().unwrap();
        assert_eq!((d.tickets[0].state.as_str(), d.tickets[0].commits.len()), ("review", 2));
        assert_eq!(serde_json::to_value(&d.tickets[0].verdicts).unwrap(), json!({ "qa": "pending", "security": "pending" }));
    }

    #[tokio::test]
    async fn a_branch_behind_its_base_is_refused_and_one_up_to_date_or_ahead_is_taken() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/rv-behind";
        let (dev, _team) = building(&st, &fake, "rv-behind", &["qa"]).await;
        let mut pr = st.db.list_pull_requests(None).unwrap().into_iter().find(|p| p.slug == slug && p.number == 500).expect("the spec PR is tracked");
        pr.base_ref = "main".into();
        st.db.save_pull_request(&pr).unwrap();
        fake.in_turn("GET", &format!("/repos/{slug}/compare/main...spec/40-rounds"), vec![
            (200, json!({ "status": "diverged", "ahead_by": 2, "behind_by": 3 })),
            (200, json!({ "status": "ahead", "ahead_by": 3, "behind_by": 0 })),
        ]);

        let err = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap_err();
        assert!(err.starts_with("`spec/40-rounds` is 3 commits behind origin/main") && err.contains("`git merge origin/main`"), "{err}");
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!(d.tickets[0].state, "started", "nothing taken in");
        assert_eq!(crate::delivery::delivery_json(&st, &d)["behind"], json!({ "count": 3, "base": "main" }), "the Delivery card shows it");

        // Merged the base: only ahead now, so it goes to review and the card no longer shows it.
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.starts_with("Submitted #41"), "{out}");
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!(crate::delivery::delivery_json(&st, &d)["behind"], Value::Null);
        assert!(fake.seen(&format!("/repos/{slug}/compare/")).iter().all(|s| s.method == "GET"), "a read on GitHub, no git in the Dev's clone");
    }

    #[tokio::test]
    async fn a_missing_role_is_skipped_and_a_team_without_reviewers_passes_on_submit() {
        let (fake, st) = (fake_github(), state());
        let (dev, team) = building(&st, &fake, "rv-qa-only", &["qa"]).await;
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.starts_with("Submitted #41: qa agent (QA) are reviewing it."), "{out}");
        assert_eq!(woken(&st, &team[0]).len(), 1);
        let d = st.db.get_delivery("octo/rv-qa-only", 40).unwrap().unwrap();
        assert_eq!(serde_json::to_value(&d.tickets[0].verdicts).unwrap(), json!({ "qa": "pending" }));

        let (fake, st) = (fake_github(), state());
        let (dev, _) = building(&st, &fake, "rv-alone", &[]).await;
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.contains("The team has no QA or Security, so it passed as submitted.") && out.contains("Next: #42 is with Builder."), "{out}");
        assert_eq!(standing(&st, "octo/rv-alone", 41), ("closed".into(), Some(42), None));
    }

    #[tokio::test]
    async fn verdicts_go_on_the_pr_and_two_passes_close_the_ticket_and_start_the_next() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/rv-pass";
        let (dev, team) = building(&st, &fake, "rv-pass", &["qa", "security"]).await;
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        let dev_wakes = woken(&st, &dev).len();

        let out = tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "Ran the suite; the table renders." })).await.unwrap();
        assert_eq!(out, "Posted your pass on #500. #41 waits for its other reviewers.");
        assert_eq!(posts(&fake, slug, "issues/500/comments"), [json!({ "body": "**QA** (qa agent) passed ticket #41 at `fed4321`.\n\nRan the suite; the table renders." })]);
        assert_eq!(posts(&fake, slug, "issues/500/labels"), [json!({ "labels": ["qa-passed"] })]);
        assert_eq!(posts(&fake, slug, "labels")[0]["name"], "qa-passed", "a missing label is created");
        assert!(posts(&fake, slug, "issues/41/comments").is_empty(), "not closed yet");
        assert_eq!(tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "again" })).await.unwrap_err(), "You already passed #41 in this round: nothing was posted.");

        let out = tool(&st, &team[1], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "No input reaches a query." })).await.unwrap();
        assert!(out.contains("Closed #41 as completed with the comment \"Done in `abc1234`, `fed4321`, part of #500.\"") && out.ends_with("Next: #42 is with Builder."), "{out}");
        assert_eq!(posts(&fake, slug, "issues/500/labels")[1], json!({ "labels": ["security:passed"] }));
        assert_eq!(standing(&st, slug, 41), ("closed".into(), Some(42), None));
        assert_eq!(posts(&fake, slug, "issues/42/labels"), [json!({ "labels": ["in-progress"] })]);
        let w = woken(&st, &dev);
        assert_eq!(w.len(), dev_wakes + 1);
        assert!(w.last().unwrap().contains("build ticket #42 \"API\""), "{w:?}");
    }

    #[tokio::test]
    async fn changes_go_back_to_the_dev_and_the_third_round_needs_the_user() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/rv-changes";
        let (dev, team) = building(&st, &fake, "rv-changes", &["qa", "security"]).await;
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        assert_eq!(tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "x" })).await.unwrap_err(), "#41 isn't waiting for a review: nothing was posted.");

        for round in 1..=3 {
            tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
            let before = woken(&st, &dev).len();
            let changes = json!({ "number": 41, "verdict": "changes", "notes": format!("Round {round}: the total is off by one.") });
            assert!(tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": " " })).await.unwrap_err().starts_with("notes are required"));
            let out = tool(&st, &team[0], "review_ticket", changes).await.unwrap();
            let posted = posts(&fake, slug, "issues/500/comments");
            assert_eq!(posted.last().unwrap()["body"], format!("**QA** (qa agent) asks for changes on ticket #41 at `fed4321`.\n\nRound {round}: the total is off by one."));
            assert_eq!(posts(&fake, slug, "issues/500/labels").last().unwrap(), &json!({ "labels": ["changes-requested"] }));
            // Security's review of this round is moot now.
            assert!(tool(&st, &team[1], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.is_err());
            if round < 3 {
                assert!(out.ends_with(&format!("The Dev is fixing #41 (round {round} of 3).")), "{out}");
                let w = woken(&st, &dev);
                assert_eq!(w.len(), before + 1);
                assert!(w.last().unwrap().contains(&format!("qa agent (QA) asked for changes on ticket #41 \"Store\" (round {round} of 3):\n\nRound {round}: the total is off by one.")), "{w:?}");
                assert!(w.last().unwrap().contains("call `mcp__loft__submit_ticket` again"));
            } else {
                assert!(out.contains("That's round 3 of changes on #41, so it waits for the user now."), "{out}");
                assert_eq!(woken(&st, &dev).len(), before, "the Dev isn't woken a third time");
            }
        }
        assert_eq!(posts(&fake, slug, "issues/41/labels"), [json!({ "labels": ["in-progress"] }), json!({ "labels": ["needs-human"] })]);
        assert_eq!(standing(&st, slug, 41).0, "needs_human");
        assert!(tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap_err().contains("waiting for the user"));
        // In Needs you, which is also what pushes it to the phone.
        let v = crate::needs_you::snapshot(&st).unwrap();
        let item = v["items"].as_array().unwrap().iter().find(|i| i["type"] == "task_blocked").unwrap_or_else(|| panic!("{v:#}"));
        assert!(item["reason"].as_str().unwrap().contains("3 rounds of changes on #41: it needs you."), "{item}");
    }

    #[tokio::test]
    async fn only_the_dev_submits_and_only_a_waited_for_reviewer_reviews() {
        let (fake, st) = (fake_github(), state());
        let (dev, team) = building(&st, &fake, "rv-guard", &["qa", "code_reviewer"]).await;
        assert_eq!(tool(&st, &team[0], "submit_ticket", submit_41()).await.unwrap_err(), "#41 is with Builder: only the agent building it submits it.");
        assert_eq!(tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["abc1234"] })).await.unwrap_err(), "#42 can't be submitted: it isn't the ticket being built.");
        assert!(tool(&st, &dev, "submit_ticket", json!({ "number": 99, "commits": ["abc1234"] })).await.unwrap_err().starts_with("#99 on octo/rv-guard isn't a ticket of a Delivery"));
        assert_eq!(tool(&st, &dev, "submit_ticket", json!({ "number": 41, "commits": ["main"] })).await.unwrap_err(), "\"main\" isn't a commit SHA: give 7 to 40 hex characters");
        assert_eq!(tool(&st, &dev, "submit_ticket", json!({ "number": 41, "commits": [] })).await.unwrap_err(), "commits is required: the SHAs of the ticket's commits");
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert_eq!(tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap_err(), "#41 can't be submitted: it's already submitted and waiting for its reviewers.");
        assert_eq!(tool(&st, &team[1], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "x" })).await.unwrap_err(), "#41 isn't a spec PR waiting for its Code Review: the Code reviewer reviews the whole spec PR once every ticket is closed.");
        assert_eq!(tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "maybe", "notes": "x" })).await.unwrap_err(), "verdict must be \"pass\" or \"changes\"");
        // The old tool is gone.
        let names: Vec<String> = crate::mcp::tools(&st).as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert!(names.contains(&"submit_ticket".into()) && names.contains(&"review_ticket".into()) && !names.contains(&"close_ticket".into()), "{names:?}");
        assert!(mcp_call(&st, &dev, "close_ticket", &json!({ "number": 41, "commits": ["abc1234"] })).await.is_none());
    }

    #[tokio::test]
    async fn a_close_that_fails_after_the_passes_is_retried_by_the_mirror() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rv-retry";
        let (dev, _) = building(&st, &fake, "rv-retry", &[]).await;
        fake.in_turn("PATCH", &format!("/repos/{slug}/issues/41"), vec![
            (502, json!({ "message": "Server Error" })),
            (200, gh_ticket(slug, 41, "closed", "## What to build\n\nStore.")),
        ]);
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.contains("#41 passed, but Commented on #41 but couldn't close it"), "{out}");
        assert_eq!(standing(&st, slug, 41), ("closing".into(), Some(41), None));
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
        crate::mirror::sync_project(&st, "rv-retry", false).await.unwrap();
        assert_eq!(standing(&st, slug, 41), ("closed".into(), Some(42), None));
        assert_eq!(posts(&fake, slug, "issues/41/comments").len(), 1, "the retry doesn't comment twice");
    }

    #[tokio::test]
    async fn a_next_start_that_failed_is_retried_by_the_mirror() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rv-next";
        let (dev, _) = building(&st, &fake, "rv-next", &[]).await;
        fake.route("POST", &format!("/repos/{slug}/issues/42/labels"), 502, json!({ "message": "Server Error" }));
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.contains("#42 is next, but Couldn't label #42 `in-progress`: GitHub 502"), "{out}");
        assert_eq!(standing(&st, slug, 41), ("closed".into(), None, Some(42)));
        let before = woken(&st, &dev).len();

        fake.route("POST", &format!("/repos/{slug}/issues/42/labels"), 200, json!([]));
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
        crate::mirror::sync_project(&st, "rv-next", false).await.unwrap();
        assert_eq!(standing(&st, slug, 42), ("started".into(), Some(42), None));
        let w = woken(&st, &dev);
        assert_eq!(w.len(), before + 1);
        assert!(w.last().unwrap().contains("build ticket #42 \"API\""), "{w:?}");
    }

    #[tokio::test]
    async fn a_close_whose_answer_was_lost_counts_as_closed_on_the_retry() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rv-lost";
        let (dev, _) = building(&st, &fake, "rv-lost", &[]).await;
        fake.route("PATCH", &format!("/repos/{slug}/issues/41"), 502, json!({ "message": "Server Error" }));
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert_eq!(standing(&st, slug, 41).0, "closing");

        // The close went through on GitHub after all.
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, gh_ticket(slug, 41, "closed", "## What to build\n\nStore."));
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
        crate::mirror::sync_project(&st, "rv-lost", false).await.unwrap();
        assert_eq!(standing(&st, slug, 41), ("closed".into(), Some(42), None));
        assert_eq!(fake.seen(&format!("/repos/{slug}/issues/41")).iter().filter(|s| s.method == "PATCH").count(), 1, "not closed twice");
        assert!(woken(&st, &dev).last().unwrap().contains("build ticket #42"));
    }

    #[tokio::test]
    async fn a_pass_after_changes_takes_changes_requested_off_the_pr() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/rv-relabel";
        let (dev, team) = building(&st, &fake, "rv-relabel", &["qa"]).await;
        let deletes = || fake.seen(&format!("/repos/{slug}/issues/500/labels/changes-requested")).iter().filter(|s| s.method == "DELETE").count();
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();
        assert_eq!(deletes(), 0, "still waiting for changes");
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "Fixed." })).await.unwrap();
        assert_eq!(deletes(), 1);
        assert_eq!(standing(&st, slug, 41).0, "closed");
    }

    /// `building` for both tickets: closing #42 is routed too.
    async fn building_both(st: &AppState, fake: &Fake, name: &str, roles: &[&str]) -> (Agent, Vec<Agent>) {
        let out = building(st, fake, name, roles).await;
        let slug = format!("octo/{name}");
        fake.route("POST", &format!("/repos/{slug}/issues/42/comments"), 201, json!({}));
        fake.route("GET", &format!("/repos/{slug}/issues/42"), 200, gh_ticket(&slug, 42, "open", "## What to build\n\nAPI."));
        fake.route("PATCH", &format!("/repos/{slug}/issues/42"), 200, gh_ticket(&slug, 42, "closed", "## What to build\n\nAPI."));
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));
        out
    }

    #[tokio::test]
    async fn each_wake_carries_only_its_roles_brief_and_what_its_instructions_can_change() {
        let st = state();
        let fake = own_fake(&st);
        let (dev, team) = building_both(&st, &fake, "brief-roles", &["qa", "security", "code_reviewer"]).await;
        let (qa, sec, cr) = (&team[0], &team[1], &team[2]);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();
        let roles = ["You are the Dev on this Delivery", "You are QA on this Delivery", "You are Security on this Delivery", "You are the Code reviewer on this Delivery"];
        let limit = "but they can't change the Delivery's tools, its branch or how the work closes or merges.";
        let only = |w: &str, mine: Option<&str>| {
            assert!(w.contains(limit), "no limits in {w}");
            for r in roles {
                assert_eq!(w.contains(r), Some(r) == mine, "{r:?} in {w}");
            }
        };
        let dw = woken(&st, &dev);
        only(&dw[0], Some(roles[0]));
        assert!(dw[0].contains("like how to build and check the work in this project"));
        only(&woken(&st, qa)[0], Some(roles[1]));
        assert!(woken(&st, qa)[0].contains("like how to test it in this project"));
        only(&woken(&st, sec)[0], Some(roles[2]));
        assert!(woken(&st, sec)[0].contains("like how to check its security in this project"));
        // A changes round: the Dev's fix wake carries no other role's brief either.
        only(dw.last().unwrap(), None);
        assert!(dw.last().unwrap().contains("asked for changes on ticket #41"));
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        tool(&st, sec, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": 42, "verdict": "pass", "notes": "ok" })).await.unwrap();
        tool(&st, sec, "review_ticket", json!({ "number": 42, "verdict": "pass", "notes": "ok" })).await.unwrap();
        let cw = woken(&st, cr);
        assert_eq!(cw.len(), 1);
        only(&cw[0], Some(roles[3]));
        assert!(cw[0].contains("like how to review it in this project"));
        tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": "A race." })).await.unwrap();
        only(woken(&st, &dev).last().unwrap(), None);
    }

    #[tokio::test]
    async fn the_last_close_wakes_the_code_reviewer_once_for_the_whole_spec_pr() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/cr-wake";
        let (dev, team) = building_both(&st, &fake, "cr-wake", &["code_reviewer"]).await;
        let cr = &team[0];
        // No QA or Security: each ticket passes on submit, and the Code reviewer isn't asked about either.
        let out = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(out.contains("Next: #42 is with Builder."), "{out}");
        assert!(woken(&st, cr).is_empty(), "no per-ticket Code Review");
        let out = tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        assert!(out.contains("It was the spec's last open ticket: code_reviewer agent (Code reviewer) is reviewing the whole spec PR #500 now."), "{out}");
        let w = woken(&st, cr);
        assert_eq!(w.len(), 1);
        for want in [
            "You are the Code reviewer on this Delivery: every ticket of spec #40 is closed (#41, #42), and you review the whole spec PR https://github.com/octo/cr-wake/pull/500 once, on the spec branch `spec/40-rounds`.",
            "call `mcp__loft__review_ticket` with 500",
            "`verdict` \"changes\" if you found anything, however small (every finding in `notes`, each with its file, line and fix: the Dev fixes them all on this PR)",
        ] {
            assert!(w[0].contains(want), "missing {want:?} in {}", w[0]);
        }
        assert!(!w[0].contains("tech_debt") && !w[0].contains("tech-debt"), "{}", w[0]);
        assert!(fake.seen("/graphql").is_empty(), "the spec PR stays a draft until the Code Review passes");
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().state, "final_review");
        // A catch-up poll doesn't wake it again or mark the PR ready.
        fake.route("GET", &format!("/repos/{slug}/issues"), 200, json!([]));
        crate::mirror::sync_project(&st, "cr-wake", true).await.unwrap();
        assert_eq!((woken(&st, cr).len(), fake.seen("/graphql").len()), (1, 0));
    }

    #[tokio::test]
    async fn blocking_findings_go_to_the_dev_and_a_pass_marks_the_pr_ready_and_tells_the_user() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/cr-flow";
        let (dev, team) = building_both(&st, &fake, "cr-flow", &["code_reviewer"]).await;
        let cr = &team[0];
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        let debt = json!([{ "title": "Split delivery.rs", "body": "It grew past 600 lines." }]);
        assert_eq!(tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "x" })).await.unwrap_err(), "#41 isn't a spec PR waiting for its Code Review: the Code reviewer reviews the whole spec PR once every ticket is closed.");

        // tech_debt is refused, with nothing posted or filed, and the review still waits.
        let issues = posts(&fake, slug, "issues").len();
        let err = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": "store.rs:12 drops the error.", "tech_debt": debt })).await.unwrap_err();
        assert!(err.starts_with("tech_debt is gone: put every finding in `notes`") && err.contains("verdict \"changes\""), "{err}");
        assert_eq!(posts(&fake, slug, "issues").len(), issues, "nothing is filed");
        assert!(!posts(&fake, slug, "issues/500/comments").iter().any(|c| c["body"].as_str().unwrap().contains("Code reviewer")), "nothing is posted");
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().state, "final_review");

        // Any finding is changes: back to the Dev, round 1 of 3, and the PR stays a draft.
        let before = woken(&st, &dev).len();
        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": "store.rs:12 drops the error. split.rs:1 is long." })).await.unwrap();
        assert!(out.ends_with("The Dev is fixing them (round 1 of 3); the spec PR stays a draft."), "{out}");
        assert_eq!(posts(&fake, slug, "issues/500/comments").last().unwrap()["body"], "**Code reviewer** (code_reviewer agent) asks for changes on the spec PR.\n\nstore.rs:12 drops the error. split.rs:1 is long.");
        assert_eq!(posts(&fake, slug, "issues/500/labels").last().unwrap(), &json!({ "labels": ["changes-requested"] }));
        assert_eq!(posts(&fake, slug, "issues").len(), issues, "no tech-debt issue is filed");
        let w = woken(&st, &dev);
        assert_eq!(w.len(), before + 1);
        assert!(w.last().unwrap().contains("asked for changes on the spec PR https://github.com/octo/cr-flow/pull/500 (round 1 of 3):\n\nstore.rs:12 drops the error. split.rs:1 is long."), "{w:?}");
        assert!(w.last().unwrap().contains("call `mcp__loft__submit_ticket` with 500"));
        assert!(fake.seen("/graphql").is_empty());

        // The fixes go back to the Code reviewer.
        assert!(tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "x" })).await.is_err(), "not before the fixes");
        let out = tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["feed1234"] })).await.unwrap();
        assert_eq!(out, "Submitted the fixes for spec PR #500: code_reviewer agent (Code reviewer) is looking again.");
        let again = woken(&st, cr);
        assert!(again.len() == 2 && again[1].contains("The Dev pushed fixes for your last notes") && again[1].contains("The fixes: `feed123`."), "{again:?}");

        // A pass: the PR is marked ready from the record alone, and watched so Needs you offers to merge it.
        let sub_reads = fake.seen(&format!("/repos/{slug}/issues/40/sub_issues")).len();
        fake.in_turn("GET", &format!("/repos/{slug}/pulls/500"), vec![
            (200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true })),
            (200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": false, "mergeable": true, "mergeable_state": "clean", "title": "Rounds", "head": { "ref": "spec/40-rounds", "sha": "feed1234" }, "base": { "ref": "main" }, "user": { "login": "octo-me" } })),
        ]);
        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "Fixed." })).await.unwrap();
        assert!(out.ends_with("Spec PR #500 is ready for review now, and the user is told it can be merged."), "{out}");
        assert_eq!(fake.seen("/graphql").len(), 1);
        assert_eq!(fake.seen(&format!("/repos/{slug}/issues/40/sub_issues")).len(), sub_reads, "the tickets aren't read again from GitHub");
        assert!(fake.seen(&format!("/repos/{slug}/issues/500/labels/changes-requested")).iter().any(|s| s.method == "DELETE"));
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().state, "ready");
        let v = crate::needs_you::snapshot(&st).unwrap();
        let item = v["items"].as_array().unwrap().iter().find(|i| i["type"] == "pr_ready").unwrap_or_else(|| panic!("{v:#}"));
        assert!(item["title"].as_str().unwrap().starts_with("octo/cr-flow#500"), "{item}");
    }

    #[tokio::test]
    async fn the_third_round_of_final_changes_waits_for_the_user() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/cr-stuck";
        let (dev, team) = building_both(&st, &fake, "cr-stuck", &["code_reviewer"]).await;
        let cr = &team[0];
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        fake.route("POST", &format!("/repos/{slug}/issues/500/labels"), 200, json!([]));
        // The spec's Loft issue, which the user is pointed at.
        let mut spec = gh_ticket(slug, 40, "open", "Spec.");
        spec["labels"] = json!([{ "name": "spec" }]);
        let (spec_task, _) = crate::mirror::bring_in(&st, "cr-stuck", slug, &spec).unwrap();

        for round in 1..=3 {
            let before = woken(&st, &dev).len();
            let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": format!("Round {round}: a race in retry.") })).await.unwrap();
            if round < 3 {
                assert_eq!(woken(&st, &dev).len(), before + 1);
                tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["feed1234"] })).await.unwrap();
            } else {
                assert!(out.contains("That's round 3 of changes on the spec PR, so it waits for the user now."), "{out}");
                assert_eq!(woken(&st, &dev).len(), before, "the Dev isn't woken a third time");
            }
        }
        assert_eq!(posts(&fake, slug, "issues/500/labels").last().unwrap(), &json!({ "labels": ["needs-human"] }));
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!((d.state.as_str(), d.final_rounds), ("needs_human", 3));
        assert!(fake.seen("/graphql").is_empty(), "still a draft");
        let t = st.db.get_task(&spec_task.id).unwrap().unwrap();
        assert_eq!(t.status, "blocked");
        assert!(t.error.starts_with("3 rounds of changes on spec PR #500: it needs you."), "{}", t.error);
        assert!(tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["feed1234"] })).await.is_err());
        assert!(tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "x" })).await.is_err());
    }

    #[tokio::test]
    async fn without_a_code_reviewer_the_last_close_marks_the_pr_ready() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/cr-none";
        let (dev, team) = building_both(&st, &fake, "cr-none", &["qa"]).await;
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &team[0], "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert!(fake.seen("/graphql").is_empty());
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        let out = tool(&st, &team[0], "review_ticket", json!({ "number": 42, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert!(out.contains("It was the spec's last open ticket: spec PR #500 is now ready for review"), "{out}");
        assert_eq!(fake.seen("/graphql").len(), 1);
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().state, "ready");
        assert!(st.db.list_pull_requests(None).unwrap().iter().any(|p| p.slug == slug && p.number == 500), "watched for Needs you");
        // Nobody files tech debt any more.
        assert!(tool(&st, &team[0], "review_ticket", json!({ "number": 42, "verdict": "pass", "notes": "ok", "tech_debt": [{ "title": "x", "body": "y" }] })).await.unwrap_err().starts_with("tech_debt is gone"));
    }

    /// A grill on `name` that published the standalone ticket #41 "Store" and ended with its card, with
    /// the team's `roles` (a Dev first) busy and what starting and reviewing it needs routed on the fake.
    async fn standalone(st: &AppState, fake: &Fake, name: &str, roles: &[&str]) -> (Agent, crate::db::Message, Vec<Agent>) {
        let slug = format!("octo/{name}");
        let a = planner_on(st, name, Some(&format!("git@github.com:{slug}.git")));
        repo(fake, &slug, &["spec", "ready-for-agent"], 41);
        fake.route("POST", &format!("/repos/{slug}/issues"), 201, json!({ "number": 41, "id": 4100, "title": "Store" }));
        crate::grill::tests::round(st, &a, &["One ticket?"]).unwrap();
        crate::grill::tests::send(st, &a).unwrap();
        publish_tickets(st, &a, None, vec![ticket("Store", &[])]).await.unwrap();
        crate::grill::mcp_call(st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        let team = roles.iter().map(|r| busy(st, &format!("{r} agent"), name, r)).collect();
        for p in ["issues/41/labels", "issues/500/comments", "issues/500/labels", "labels"] {
            fake.route("POST", &format!("/repos/{slug}/{p}"), 201, json!({}));
        }
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));
        let m = summary(st, &a);
        (a, m, team)
    }

    #[tokio::test]
    async fn start_delivery_on_a_standalone_ticket_makes_its_branch_and_draft_pr() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/solo-start";
        let (a, m, team) = standalone(&st, &fake, "solo-start", &["dev"]).await;
        let card = &m.meta["grill"]["end_card"];
        assert_eq!((&card["delivery"], &card["next_message"]), (&json!({ "repo": slug, "spec": 41, "standalone": true }), &json!("")));
        let d = st.db.get_delivery(slug, 41).unwrap().unwrap();
        assert_eq!((d.standalone, d.branch.as_str(), d.pr, d.tickets.len()), (true, "ticket/41-store", None, 1), "no branch or PR until it starts");
        assert!(posts(&fake, slug, "pulls").is_empty());

        press(&st, &a, &m).await.unwrap();
        assert_eq!(posts(&fake, slug, "git/commits"), [json!({ "message": "Start ticket #41", "tree": "main-tree", "parents": ["main-sha"] })]);
        assert_eq!(posts(&fake, slug, "git/refs"), [json!({ "ref": "refs/heads/ticket/41-store", "sha": "start-sha" })]);
        assert_eq!(posts(&fake, slug, "pulls"), [json!({ "title": "Store", "head": "ticket/41-store", "base": "main", "body": "Closes #41", "draft": true })]);
        assert_eq!(posts(&fake, slug, "issues/41/labels"), [json!({ "labels": ["in-progress"] })]);
        let w = woken(&st, &team[0]);
        assert_eq!(w.len(), 1);
        for want in [
            "[Loft Delivery: ticket #41] This is not a message from the user. You are the Dev on this Delivery: build ticket #41 \"Store\"",
            "- Read the ticket first, and build only what the ticket asks.",
            "Work on the ticket branch `ticket/41-store`: pull it, commit there and push. Open no PR: the PR https://github.com/octo/solo-start/pull/500 is the ticket's only one.",
        ] {
            assert!(w[0].contains(want), "missing {want:?} in {}", w[0]);
        }
        // Pressed again: nothing new.
        assert_eq!(press(&st, &a, &m).await.unwrap()["duplicate"], true);
        assert_eq!(posts(&fake, slug, "pulls").len(), 1);
    }

    #[tokio::test]
    async fn a_standalone_ticket_goes_through_the_same_reviews_and_stays_open_until_its_pr_merges() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/solo-flow";
        let (a, m, team) = standalone(&st, &fake, "solo-flow", &["dev", "qa", "security", "code_reviewer"]).await;
        let (dev, qa, sec, cr) = (&team[0], &team[1], &team[2], &team[3]);
        press(&st, &a, &m).await.unwrap();

        tool(&st, dev, "submit_ticket", submit_41()).await.unwrap();
        assert!(woken(&st, qa)[0].contains("on the ticket branch `ticket/41-store` (PR https://github.com/octo/solo-flow/pull/500)"));
        // A changes round first: the label it puts on the PR comes off with the later passes.
        let deletes = || fake.seen(&format!("/repos/{slug}/issues/500/labels/changes-requested")).iter().filter(|s| s.method == "DELETE").count();
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Off by one." })).await.unwrap();
        assert_eq!(posts(&fake, slug, "issues/500/labels").last().unwrap(), &json!({ "labels": ["changes-requested"] }));
        assert!(woken(&st, dev).last().unwrap().contains("Fix it on the ticket branch `ticket/41-store`"));
        tool(&st, dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert_eq!(deletes(), 0);
        let out = tool(&st, sec, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert!(out.contains("#41 passed its reviews. It stays open until its PR #500 merges.") && out.contains("It passed its reviews: code_reviewer agent (Code reviewer) is reviewing the whole PR #500 now."), "{out}");
        assert_eq!(deletes(), 1, "changes-requested comes off the PR when the ticket passes");
        // Loft doesn't close it: its PR does, when it merges.
        assert!(fake.seen(&format!("/repos/{slug}/issues/41")).iter().all(|s| s.method != "PATCH"));
        assert!(posts(&fake, slug, "issues/41/comments").is_empty());
        let w = woken(&st, cr);
        assert!(w.len() == 1 && w[0].contains("ticket #41 passed QA and Security, and you review the whole PR https://github.com/octo/solo-flow/pull/500 once, on the ticket branch `ticket/41-store`."), "{w:?}");

        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "Clean." })).await.unwrap();
        assert!(out.contains("PR #500 is ready for review now"), "{out}");
        assert_eq!(posts(&fake, slug, "issues/500/comments").last().unwrap()["body"], "**Code reviewer** (code_reviewer agent) passed the PR.\n\nClean.");
        assert_eq!(fake.seen("/graphql").len(), 1);
        assert_eq!(st.db.get_delivery(slug, 41).unwrap().unwrap().state, "ready");
        let done = done_items(&st);
        assert!(done.len() == 1 && done[0]["title"] == "Delivery done: Store" && done[0]["reason"] == "Every ticket is closed and its reviews passed: PR #500 is ready for review.", "{done:#?}");
        assert!(fake.seen(&format!("/repos/{slug}/issues/41")).iter().all(|s| s.method != "PATCH"), "still open until the merge");
    }

    #[tokio::test]
    async fn start_delivery_on_a_standalone_tickets_loft_issue() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/solo-issue";
        planner_on(&st, "solo-issue", Some("git@github.com:octo/solo-issue.git"));
        spec_pr_on(&fake, slug);
        let dev = busy(&st, "Builder", "solo-issue", "dev");
        // Published before Deliveries: a ticket with no record yet.
        let ticket = gh_ticket(slug, 41, "open", "## What to build\n\nStore.");
        fake.route("GET", &format!("/repos/{slug}/issues/41"), 200, ticket.clone());
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        let (t, _) = crate::mirror::bring_in(&st, "solo-issue", slug, &ticket).unwrap();
        let start = |id: String| crate::delivery::start_from_issue(State(st.clone()), Path(id));

        let v = start(t.id.clone()).await.unwrap().0;
        assert_eq!((&v["duplicate"], &v["ticket"], &v["agent_name"], &v["pr"]), (&json!(false), &json!(41), &json!("Builder"), &json!(500)));
        // The Delivery as the card shows it, like Pause and Resume answer: the iPhone app reads it from the reply.
        assert_eq!((&v["ok"], &v["state"], &v["started"], &v["can_pause"], &v["can_resume"]), (&json!(true), &json!("running"), &json!(true), &json!(true), &json!(false)));
        assert_eq!(posts(&fake, slug, "pulls")[0]["head"], "ticket/41-ticket-41");
        assert_eq!(posts(&fake, slug, "pulls")[0]["body"], "Closes #41");
        assert!(woken(&st, &dev)[0].contains("build ticket #41 \"Ticket 41\""));
        let again = start(t.id.clone()).await.unwrap().0;
        assert_eq!((&again["duplicate"], &again["state"], &again["started"]), (&json!(true), &json!("running"), &json!(true)), "a duplicate carries the Delivery too");
        assert_eq!(woken(&st, &dev).len(), 1);
        // After a reload the page asks Loft, which says who has it.
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(t.id.clone())).await.unwrap().0;
        assert_eq!(shown, json!({ "state": "running", "stall_reason": null, "state_line": { "tone": "running", "text": "Running." }, "can_pause": true, "can_resume": false, "needs_human": false, "started": true, "agent_id": dev.id, "agent_name": "Builder", "pr": 500, "behind": null }));

        // A spec's ticket starts from the spec's card; an issue that isn't a ticket has no Delivery.
        delivery_40(&st, slug);
        let (t42, _) = crate::mirror::bring_in(&st, "solo-issue", slug, &gh_ticket(slug, 42, "open", "x")).unwrap();
        assert!(start(t42.id).await.unwrap_err().1.starts_with("#42 is a ticket of spec #40: start the spec's Delivery"));
        let mut plain = gh_ticket(slug, 43, "open", "x");
        plain["labels"] = json!([]);
        fake.route("GET", &format!("/repos/{slug}/issues/43"), 200, plain.clone());
        let (t43, _) = crate::mirror::bring_in(&st, "solo-issue", slug, &plain).unwrap();
        assert_eq!(crate::delivery::issue_delivery(State(st.clone()), Path(t43.id.clone())).await.unwrap().0, Value::Null, "no Delivery");
        assert!(start(t43.id).await.unwrap_err().1.starts_with("#43 isn't an open ticket"));
        // A sub-issue of a spec with no Delivery isn't started on its own either.
        let (mut t44, _) = crate::mirror::bring_in(&st, "solo-issue", slug, &gh_ticket(slug, 44, "open", "x")).unwrap();
        t44.parent_id = Some(t.id.clone());
        st.db.save_task(&t44).unwrap();
        assert!(start(t44.id).await.unwrap_err().1.starts_with("#44 is part of a spec"));
        // Nor one whose spec Loft doesn't mirror: GitHub says it has a parent.
        let mut t45 = gh_ticket(slug, 45, "open", "x");
        t45["parent_issue_url"] = json!(format!("https://api.github.com/repos/{slug}/issues/99"));
        fake.route("GET", &format!("/repos/{slug}/issues/45"), 200, t45.clone());
        let (t45, _) = crate::mirror::bring_in(&st, "solo-issue", slug, &t45).unwrap();
        assert_eq!(t45.parent_id, None, "#99 isn't mirrored");
        assert!(start(t45.id).await.unwrap_err().1.starts_with("#45 is part of a spec on GitHub"));
    }

    /// Spec #40 on GitHub as `sub_issues` lists it and its Loft issue, with no Delivery yet.
    fn spec_40_on(st: &AppState, fake: &Fake, project: &str, slug: &str, subs: Value) -> crate::db::Task {
        let mut spec = gh_ticket(slug, 40, "open", "## Problem Statement\n\nIt hurts.");
        (spec["title"], spec["labels"]) = (json!("Rounds"), json!([{ "name": "spec" }]));
        fake.route("GET", &format!("/repos/{slug}/issues/40"), 200, spec.clone());
        fake.route("GET", &format!("/repos/{slug}/issues/40/sub_issues"), 200, subs);
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        fake.route("POST", &format!("/repos/{slug}/issues/45/labels"), 200, json!([]));
        crate::mirror::bring_in(st, project, slug, &spec).unwrap().0
    }

    #[tokio::test]
    async fn start_delivery_on_a_spec_published_earlier_builds_it_from_github() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/spec-page";
        planner_on(&st, "spec-page", Some("git@github.com:octo/spec-page.git"));
        spec_pr_on(&fake, slug);
        let dev = busy(&st, "Builder", "spec-page", "dev");
        let with = |n: i64, state: &str, labels: Value, blocked: i64| {
            let mut x = gh_ticket(slug, n, state, "x");
            x["labels"] = labels;
            x["issue_dependencies_summary"] = json!({ "blocked_by": blocked });
            x
        };
        let ready = json!([{ "name": "ready-for-agent" }]);
        let subs = json!([
            with(41, "closed", ready.clone(), 0),
            with(42, "open", json!([{ "name": "ready-for-agent" }, { "name": "in-progress" }]), 0),
            with(43, "open", ready.clone(), 1),
            with(45, "open", ready.clone(), 2),
        ]);
        let spec = spec_40_on(&st, &fake, "spec-page", slug, subs);
        fake.route("GET", &format!("/repos/{slug}/issues/43/dependencies/blocked_by"), 200, json!([{ "number": 42 }]));
        // #99 isn't the spec's: the Delivery can't tell when it closes, so it doesn't hold #45 back.
        fake.route("GET", &format!("/repos/{slug}/issues/45/dependencies/blocked_by"), 200, json!([{ "number": 41 }, { "number": 99 }]));
        let start = |id: String| crate::delivery::start_from_issue(State(st.clone()), Path(id));

        let v = start(spec.id.clone()).await.unwrap().0;
        assert_eq!((&v["duplicate"], &v["ticket"], &v["agent_name"], &v["pr"]), (&json!(false), &json!(45), &json!("Builder"), &json!(500)));
        assert_eq!(posts(&fake, slug, "git/refs")[0]["ref"], "refs/heads/spec/40-rounds");
        assert_eq!((&posts(&fake, slug, "pulls")[0]["head"], &posts(&fake, slug, "pulls")[0]["body"]), (&json!("spec/40-rounds"), &json!("Closes #40")));
        assert!(woken(&st, &dev)[0].contains("build ticket #45 \"Ticket 45\""));
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        let shape: Vec<(i64, &str, Vec<i64>)> = d.tickets.iter().map(|t| (t.number, t.state.as_str(), t.blocked_by.clone())).collect();
        assert_eq!(shape, vec![(41, "closed", vec![]), (42, "elsewhere", vec![]), (43, "open", vec![42]), (45, "started", vec![41])]);
        assert_eq!(v["tickets"][1], json!({ "number": 42, "title": "Ticket 42", "state": "elsewhere", "skipped": "is labelled in-progress", "agent_id": null, "agent_name": null }));
        assert_eq!(v["current"], json!({ "number": 45, "title": "Ticket 45" }));

        // Again, or on another device: who has it, and nothing started twice.
        assert_eq!(start(spec.id.clone()).await.unwrap().0["duplicate"], true);
        assert_eq!(woken(&st, &dev).len(), 1);
        assert_eq!(posts(&fake, slug, "pulls").len(), 1);
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&shown["started"], &shown["agent_name"], &shown["current"]["number"]), (&json!(true), &json!("Builder"), &json!(45)));
        // A ticket of it starts from the spec's page, not on its own.
        let (t43, _) = crate::mirror::bring_in(&st, "spec-page", slug, &with(43, "open", ready, 1)).unwrap();
        assert!(start(t43.id).await.unwrap_err().1.starts_with("#43 is a ticket of spec #40: start the spec's Delivery from the spec's page"));
    }

    #[tokio::test]
    async fn start_delivery_on_a_spec_reuses_its_spec_pr_and_skips_a_ticket_with_its_own_pr() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/spec-reuse";
        planner_on(&st, "spec-reuse", Some("git@github.com:octo/spec-reuse.git"));
        let dev = busy(&st, "Builder", "spec-reuse", "dev");
        let spec = spec_40_on(&st, &fake, "spec-reuse", slug, json!([gh_ticket(slug, 41, "open", "x"), gh_ticket(slug, 42, "open", "x")]));
        let own = json!({ "full_name": "Octo/Spec-Reuse" });
        fake.route("GET", &format!("/repos/{slug}/pulls"), 200, json!([
            { "number": 77, "body": "Fixes: #42", "head": { "ref": "someone/work", "repo": own } },
            { "number": 600, "body": "Closes #40", "head": { "ref": "spec/40-old-title", "repo": own } },
        ]));

        let v = crate::delivery::start_from_issue(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&v["ticket"], &v["pr"]), (&json!(41), &json!(600)));
        assert_eq!(v["tickets"][1]["skipped"], "has its own open PR #77");
        let d = st.db.get_delivery(slug, 40).unwrap().unwrap();
        assert_eq!(d.branch, "spec/40-old-title");
        assert!(posts(&fake, slug, "pulls").is_empty() && posts(&fake, slug, "git/refs").is_empty(), "the spec branch and PR are reused");
        assert!(woken(&st, &dev)[0].contains("Work on the spec branch `spec/40-old-title`"));
    }

    #[tokio::test]
    async fn start_delivery_on_a_spec_ignores_fork_prs() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/spec-fork";
        planner_on(&st, "spec-fork", Some("git@github.com:octo/spec-fork.git"));
        spec_pr_on(&fake, slug);
        let dev = busy(&st, "Builder", "spec-fork", "dev");
        let spec = spec_40_on(&st, &fake, "spec-fork", slug, json!([gh_ticket(slug, 41, "open", "x"), gh_ticket(slug, 42, "open", "x")]));
        // An outsider's fork PRs: one closes the spec from a `main` head, one closes ticket #41. And one of
        // the repo's own that closes the spec but isn't on a spec branch.
        let fork = json!({ "full_name": "mallory/spec-fork" });
        // The second read is the spec PR lookup by `head=octo:spec/40-rounds`, which GitHub filters.
        fake.in_turn("GET", &format!("/repos/{slug}/pulls"), vec![(200, json!([
            { "number": 66, "body": "Closes #40", "head": { "ref": "main", "repo": fork } },
            { "number": 67, "body": "Closes #41", "head": { "ref": "ticket/41-x", "repo": fork } },
            { "number": 68, "body": "Closes #40", "head": { "ref": "fix-things", "repo": { "full_name": slug } } },
        ])), (200, json!([]))]);

        let v = crate::delivery::start_from_issue(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&v["ticket"], &v["pr"]), (&json!(41), &json!(500)), "neither fork PR is reused or skips a ticket");
        assert_eq!(st.db.get_delivery(slug, 40).unwrap().unwrap().branch, "spec/40-rounds");
        assert_eq!(posts(&fake, slug, "pulls")[0]["head"], "spec/40-rounds");
        assert!(woken(&st, &dev)[0].contains("Work on the spec branch `spec/40-rounds`"));
    }

    #[test]
    fn a_closing_keyword_names_the_issue_it_closes() {
        use crate::delivery::closes_issue;
        assert!(closes_issue("Closes #40", 40));
        assert!(closes_issue("Some text.\n\nfixes: #40, and more", 40));
        assert!(closes_issue("RESOLVED #40", 40));
        assert!(!closes_issue("Closes #400", 40));
        assert!(!closes_issue("Refs #40", 40));
        assert!(!closes_issue("Part of #40", 40));
    }

    /// Runs git in `dir`, as a test author.
    fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=Test", "-c", "user.email=test@example.com"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A grill on `name`, a project whose folder is a git repo with no GitHub `origin` (one commit on
    /// `main`), that saved the spec "Offline mode" with the tickets "Store" then "API" (blocked by Store)
    /// as Loft issues and ended with its card. The team's `roles` (a Dev first) are busy.
    async fn local_plan(st: &AppState, name: &str, roles: &[&str]) -> (Agent, crate::db::Message, Vec<Agent>, std::path::PathBuf) {
        let a = planner_on(st, name, None);
        let dir = st.settings.boot().projects_dir.join(name);
        git_in(&dir, &["checkout", "-q", "-b", "main"]);
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        git_in(&dir, &["add", "."]);
        git_in(&dir, &["commit", "-q", "-m", "init"]);
        crate::grill::tests::round(st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(st, &a).unwrap();
        publish(st, &a, "Offline mode").await.unwrap();
        let spec = st.db.list_tasks().unwrap().into_iter().find(|t| t.project == name && t.labels == ["spec"]).unwrap();
        mcp_call(st, &a, "publish_tickets", &json!({ "spec": spec.id, "tickets": [ticket("API", &["Store"]), ticket("Store", &[])] })).await.unwrap().unwrap();
        crate::grill::mcp_call(st, &a, "grill_end", &json!({ "summary": "Offline it is." })).unwrap().unwrap();
        let team = roles.iter().map(|r| busy(st, &format!("{r} agent"), name, r)).collect();
        let m = summary(st, &a);
        (a, m, team, dir)
    }

    /// The Loft issue of the ticket titled `title` in `project`.
    fn loft_ticket(st: &AppState, project: &str, title: &str) -> crate::db::Task {
        st.db.list_tasks().unwrap().into_iter().find(|t| t.project == project && t.title == title).unwrap()
    }

    fn notes_on(st: &AppState, id: &str) -> Vec<String> {
        st.db.list_task_comments(id, 200, None).unwrap().into_iter().filter(|c| c.kind == "comment").map(|c| c.text).collect()
    }

    #[tokio::test]
    async fn start_delivery_on_a_plan_kept_in_loft_makes_its_branch_in_the_project_repo() {
        let st = state();
        let fake = own_fake(&st);
        let (a, m, team, dir) = local_plan(&st, "local-start", &["dev"]).await;
        let d = crate::delivery::local_of(&st, "local-start", &loft_ticket(&st, "local-start", "Offline mode").id).unwrap();
        assert_eq!((d.local, d.branch.as_str(), d.tickets.iter().map(|t| (t.number, t.title.as_str(), t.blocked_by.clone())).collect::<Vec<_>>()), (true, "spec/1-offline-mode", vec![(1, "Store", vec![]), (2, "API", vec![1])]));

        let v = press(&st, &a, &m).await.unwrap();
        assert_eq!(v["message"]["meta"]["grill"]["end_card"]["delivery"]["started"]["title"], "Store", "the card names the ticket");
        assert_eq!(git_in(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]), "main", "the checkout is left alone");
        assert_eq!(git_in(&dir, &["rev-parse", "spec/1-offline-mode"]), git_in(&dir, &["rev-parse", "main"]), "the spec branch starts at main");
        let store = loft_ticket(&st, "local-start", "Store");
        let w = woken(&st, &team[0]);
        assert_eq!(w.len(), 1);
        for want in [
            "[Loft Delivery: spec \"Offline mode\"] This is not a message from the user. You are the Dev on this Delivery: build ticket \"Store\" (Loft issue ".to_string(),
            format!("Work on the spec branch `spec/1-offline-mode` of the project's own repo `{}`, your clone's `origin`: fetch it, commit there and push it back with `git push origin spec/1-offline-mode`. There is no PR.", dir.canonicalize().unwrap().display()),
            format!("each message ending with `Refs {}`", store.id),
            format!("call `mcp__loft__submit_ticket` with \"{}\"", store.id),
        ] {
            assert!(w[0].contains(&want), "missing {want:?} in {}", w[0]);
        }
        assert!(notes_on(&st, &store.id).iter().any(|n| n.starts_with("Started by Loft's Delivery: with dev agent")), "{:?}", notes_on(&st, &store.id));
        assert_eq!(st.db.get_delivery(&d.repo, d.spec).unwrap().unwrap().base, "main");
        assert!(fake.seen("/").is_empty(), "no GitHub at all");
    }

    #[tokio::test]
    async fn a_plan_kept_in_loft_posts_verdicts_as_notes_closes_tickets_and_offers_merge() {
        let st = state();
        let fake = own_fake(&st);
        let project = "local-flow";
        let (a, m, team, _dir) = local_plan(&st, project, &["dev", "qa", "security", "code_reviewer"]).await;
        let (dev, qa, sec, cr) = (&team[0], &team[1], &team[2], &team[3]);
        press(&st, &a, &m).await.unwrap();
        st.rt.lock().unwrap().running.insert(a.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        let (store, api, spec) = (loft_ticket(&st, project, "Store"), loft_ticket(&st, project, "API"), loft_ticket(&st, project, "Offline mode"));
        let submit = |id: &str| json!({ "number": id, "commits": ["abc1234def"] });

        let out = tool(&st, dev, "submit_ticket", submit(&store.id)).await.unwrap();
        assert!(out.starts_with("Submitted \"Store\": qa agent (QA) and security agent (Security) are reviewing it."), "{out}");
        assert!(woken(&st, qa)[0].contains("Loft posts it on the ticket's Loft issue."));
        tool(&st, qa, "review_ticket", json!({ "number": store.id, "verdict": "changes", "notes": "Saving twice loses data." })).await.unwrap();
        assert!(notes_on(&st, &store.id).contains(&"**QA** (qa agent) asks for changes on ticket \"Store\" at `abc1234`.\n\nSaving twice loses data.".to_string()));
        assert!(woken(&st, dev).last().unwrap().contains(&format!("call `mcp__loft__submit_ticket` again with \"{}\"", store.id)));
        tool(&st, dev, "submit_ticket", submit(&store.id)).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": store.id, "verdict": "pass", "notes": "Fixed." })).await.unwrap();
        let out = tool(&st, sec, "review_ticket", json!({ "number": store.id, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert!(out.contains("\"Store\" is done.") && out.contains("Next: \"API\" is with dev agent."), "{out}");
        let s = st.db.get_task(&store.id).unwrap().unwrap();
        assert_eq!(s.status, "done");
        assert!(notes_on(&st, &store.id).iter().any(|n| n == "Done in `abc1234`, on the spec branch `spec/1-offline-mode`."));

        tool(&st, dev, "submit_ticket", submit(&api.id)).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": api.id, "verdict": "pass", "notes": "ok" })).await.unwrap();
        let out = tool(&st, sec, "review_ticket", json!({ "number": api.id, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert!(out.contains("code_reviewer agent (Code reviewer) is reviewing the whole spec branch `spec/1-offline-mode` now."), "{out}");
        let w = woken(&st, cr);
        assert!(w.len() == 1 && w[0].contains("against `main` (`git diff main...spec/1-offline-mode`)") && w[0].contains(&format!("`mcp__loft__review_ticket` with \"{}\"", spec.id)), "{w:?}");

        let debt = json!([{ "title": "Index the store", "body": "Lookups scan." }]);
        let err = tool(&st, cr, "review_ticket", json!({ "number": spec.id, "verdict": "pass", "notes": "Clean.", "tech_debt": debt })).await.unwrap_err();
        assert!(err.starts_with("tech_debt is gone"), "{err}");
        assert!(!st.db.list_tasks().unwrap().iter().any(|t| t.labels == ["tech-debt"]), "no tech-debt issue is filed");
        let out = tool(&st, cr, "review_ticket", json!({ "number": spec.id, "verdict": "pass", "notes": "Clean." })).await.unwrap();
        assert!(out.ends_with("The spec branch `spec/1-offline-mode` is ready to merge into `main`: the user is offered Merge."), "{out}");
        assert!(notes_on(&st, &spec.id).iter().any(|n| n.starts_with("**Code reviewer** (code_reviewer agent) passed the spec branch.")));
        let v = crate::needs_you::snapshot(&st).unwrap();
        let item = v["items"].as_array().unwrap().iter().find(|i| i["type"] == "delivery_merge").unwrap_or_else(|| panic!("{v:#}"));
        assert_eq!(item["title"], "Merge \"Offline mode\"");
        // Done: its own item with Merge, opening the spec's Loft issue, and the planner told once.
        let done = done_items(&st);
        assert_eq!(done.len(), 1, "{done:#?}");
        assert_eq!((&done[0]["title"], &done[0]["reason"]), (&json!("Delivery done: Offline mode"), &json!("Every ticket is done and its reviews passed: merge `spec/1-offline-mode` into `main`.")));
        let acts = done[0]["actions"].as_array().unwrap();
        assert_eq!((&acts[0]["target"], &acts[1]["kind"], &acts[1]["path"]), (&json!({ "screen": "task", "task_id": spec.id }), &json!("merge"), &json!("/api/deliveries/merge")));
        let w: Vec<String> = woken(&st, &a).into_iter().filter(|p| p.contains("is done.")).collect();
        assert_eq!(w.len(), 1, "{w:?}");
        for want in ["The Delivery of spec \"Offline mode\" is done.", "- Its spec branch `spec/1-offline-mode` is ready to merge into `main`: the user is offered Merge.", "\"Store\"", "\"API\""] {
            assert!(w[0].contains(want), "missing {want:?} in {}", w[0]);
        }
        assert!(!w[0].contains("Tech debt"), "{}", w[0]);
        assert!(fake.seen("/").is_empty(), "no GitHub at all");
    }

    /// The Dev's clone of `dir` in its workspace, with `file` committed on the spec branch and pushed back,
    /// as a Dev builds a plan kept in Loft. The clone is left on the spec branch.
    fn dev_builds(st: &AppState, d: &crate::db::Delivery, dir: &std::path::Path, file: &str, text: &str) -> std::path::PathBuf {
        let clone = crate::agents::workspace(st, &d.dev).join("work").join(dir.file_name().unwrap());
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        if !clone.exists() {
            git_in(clone.parent().unwrap(), &["clone", "-q", dir.to_str().unwrap(), clone.to_str().unwrap()]);
        }
        git_in(&clone, &["checkout", "-q", &d.branch]);
        std::fs::write(clone.join(file), text).unwrap();
        git_in(&clone, &["add", "."]);
        git_in(&clone, &["commit", "-q", "-m", file]);
        git_in(&clone, &["push", "-q", "origin", &d.branch]);
        clone
    }

    /// Everything in the user's copy Merge could change: its branches, what's checked out and its files.
    fn user_copy(dir: &std::path::Path) -> (String, String, String) {
        (git_in(dir, &["for-each-ref", "refs/heads"]), git_in(dir, &["rev-parse", "--abbrev-ref", "HEAD"]), git_in(dir, &["status", "--porcelain", "--untracked-files=all"]))
    }

    #[tokio::test]
    async fn merge_merges_in_the_devs_clone_and_never_touches_the_users_copy() {
        let st = state();
        let (a, m, _team, dir) = local_plan(&st, "local-merge", &["dev"]).await;
        press(&st, &a, &m).await.unwrap();
        let d = crate::delivery::local_of(&st, "local-merge", &loft_ticket(&st, "local-merge", "Offline mode").id).unwrap();
        let merge = || crate::delivery::merge(&st, &d.repo, d.spec);
        assert!(merge().await.unwrap_err().ends_with("isn't ready to merge yet: its reviews haven't all passed."), "only after the last review");
        crate::delivery::update(&st, &d.repo, d.spec, |d| d.state = "ready".into());
        assert!(merge().await.unwrap_err().starts_with("Nothing was merged: the Dev has no clone of "), "no clone, no merge");

        let clone = dev_builds(&st, &d, &dir, "store.rs", "fn store() {}\n");
        // The user is on main with uncommitted work of their own.
        std::fs::write(dir.join("README.md"), "edited\n").unwrap();
        std::fs::write(dir.join("scratch.txt"), "mine\n").unwrap();
        let before = user_copy(&dir);

        // The clone's own config can't make Loft run anything: no fsmonitor command, no hooks.
        let marker = dir.parent().unwrap().join("ran-something");
        git_in(&clone, &["config", "core.fsmonitor", &format!("touch {}", marker.display())]);
        let hook = clone.join(".git/hooks/post-merge");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        git_in(&clone, &["config", "user.name", "Test"]);
        git_in(&clone, &["config", "user.email", "test@example.com"]);

        let shown = clone.display().to_string();
        assert_eq!(merge().await.unwrap(), format!("Merged `spec/1-offline-mode` into `main` in the Dev's clone `{shown}`."));
        assert_eq!(git_in(&clone, &["-c", "core.fsmonitor=false", "log", "-1", "--format=%s", "main"]), "Merge spec \"Offline mode\"");
        assert_eq!(git_in(&clone, &["-c", "core.fsmonitor=false", "show", "main:store.rs"]), "fn store() {}");
        assert!(!marker.exists(), "neither the fsmonitor command nor a hook ran");
        assert_eq!(user_copy(&dir), before, "the user's copy is exactly as it was");
        assert!(!dir.join("store.rs").exists());
        assert_eq!(st.db.get_delivery(&d.repo, d.spec).unwrap().unwrap().state, "merged");
        assert_eq!(st.db.get_task(&d.spec_id).unwrap().unwrap().status, "done");
        assert_eq!(merge().await.unwrap(), "`spec/1-offline-mode` is already merged into `main`.");
    }

    #[tokio::test]
    async fn merge_brings_the_devs_checkout_of_the_base_along() {
        let st = state();
        let (a, m, _team, dir) = local_plan(&st, "local-merge-on-main", &["dev"]).await;
        press(&st, &a, &m).await.unwrap();
        let d = crate::delivery::local_of(&st, "local-merge-on-main", &loft_ticket(&st, "local-merge-on-main", "Offline mode").id).unwrap();
        let clone = dev_builds(&st, &d, &dir, "store.rs", "fn store() {}\n");
        git_in(&clone, &["checkout", "-q", "main"]);
        crate::delivery::update(&st, &d.repo, d.spec, |d| d.state = "ready".into());
        let before = user_copy(&dir);

        crate::delivery::merge(&st, &d.repo, d.spec).await.unwrap();
        assert_eq!(std::fs::read_to_string(clone.join("store.rs")).unwrap(), "fn store() {}\n", "the Dev's files follow its main");
        assert_eq!(git_in(&clone, &["status", "--porcelain"]), "");
        assert_eq!(user_copy(&dir), before);
    }

    #[tokio::test]
    async fn a_conflict_merges_nothing() {
        let st = state();
        let (a, m, _team, dir) = local_plan(&st, "local-conflict", &["dev"]).await;
        press(&st, &a, &m).await.unwrap();
        let d = crate::delivery::local_of(&st, "local-conflict", &loft_ticket(&st, "local-conflict", "Offline mode").id).unwrap();
        // The spec branch and the clone's main both change README.md.
        let clone = dev_builds(&st, &d, &dir, "README.md", "from the spec\n");
        git_in(&clone, &["checkout", "-q", "main"]);
        std::fs::write(clone.join("README.md"), "from main\n").unwrap();
        git_in(&clone, &["commit", "-qam", "Main edit"]);
        crate::delivery::update(&st, &d.repo, d.spec, |d| d.state = "ready".into());
        let (main, before) = (git_in(&clone, &["rev-parse", "main"]), user_copy(&dir));

        let err = crate::delivery::merge(&st, &d.repo, d.spec).await.unwrap_err();
        assert_eq!(err, "`spec/1-offline-mode` doesn't merge cleanly into `main`, so nothing was merged.");
        assert_eq!(git_in(&clone, &["rev-parse", "main"]), main, "the clone's main didn't move");
        assert_eq!(git_in(&clone, &["status", "--porcelain"]), "");
        assert_eq!(user_copy(&dir), before);
        assert_eq!(st.db.get_delivery(&d.repo, d.spec).unwrap().unwrap().state, "ready", "Merge can be pressed again");
    }

    #[tokio::test]
    async fn git_never_runs_in_a_project_that_resolves_into_an_agent_workspace() {
        let st = state();
        // An agent's workspace holds a repo; a project folder links to it.
        let inside = st.cfg.storage_dir.join("some-agent").join("x");
        std::fs::create_dir_all(&inside).unwrap();
        git_in(&inside, &["init", "-q", "-b", "main"]);
        git_in(&inside, &["commit", "-q", "--allow-empty", "-m", "init"]);
        std::os::unix::fs::symlink(&inside, st.settings.boot().projects_dir.join("sneaky")).unwrap();
        crate::delivery::record_local_spec(&st, "sneaky", "spec-id", "Offline mode", "");
        crate::delivery::record_local_ticket(&st, "sneaky", "spec-id", "t1", "Store", &[]);
        busy(&st, "Builder", "sneaky", "dev");
        let d = crate::delivery::local_of(&st, "sneaky", "spec-id").unwrap();

        let Err(err) = crate::delivery::start(&st, &d.repo, d.spec, "").await else { panic!("started in an agent's workspace") };
        assert_eq!(err, "The Delivery didn't start: the project folder of sneaky isn't a git repo of its own in the projects folder, so Loft won't run git there.");
        assert!(git_in(&inside, &["branch", "--list", "spec/*"]).is_empty(), "no branch was made there");
        crate::delivery::update(&st, &d.repo, d.spec, |d| (d.state, d.base) = ("ready".into(), "main".into()));
        let err = crate::delivery::merge(&st, &d.repo, d.spec).await.unwrap_err();
        assert!(err.starts_with("Nothing was merged: the project folder of sneaky isn't a git repo of its own"), "{err}");
    }

    /// `local_plan`, in a project folder with no git at all (it holds one file).
    async fn plan_without_git(st: &AppState, name: &str) -> (Agent, crate::db::Message, Agent, std::path::PathBuf) {
        let mut a = planner(st, name);
        a.project = name.into();
        st.db.save_agent(&a).unwrap();
        let dir = st.settings.boot().projects_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.md"), "plans\n").unwrap();
        crate::grill::tests::round(st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(st, &a).unwrap();
        publish(st, &a, "Offline mode").await.unwrap();
        let spec = st.db.list_tasks().unwrap().into_iter().find(|t| t.project == name && t.labels == ["spec"]).unwrap();
        mcp_call(st, &a, "publish_tickets", &json!({ "spec": spec.id, "tickets": [ticket("Store", &[])] })).await.unwrap().unwrap();
        crate::grill::mcp_call(st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        let dev = busy(st, "Builder", name, "dev");
        let m = summary(st, &a);
        (a, m, dev, dir)
    }

    #[tokio::test]
    async fn start_delivery_in_a_folder_with_no_git_asks_first_and_changes_nothing_until_yes() {
        let st = state();
        let (a, m, dev, dir) = plan_without_git(&st, "no-git").await;
        let v = press(&st, &a, &m).await.unwrap();
        assert_eq!((&v["ok"], &v["needs_git"], &v["dir"]), (&json!(false), &json!(true), &json!(dir.canonicalize().unwrap().display().to_string())));
        // Asked, not done: no repo, no Delivery started, nobody woken.
        assert!(!dir.join(".git").exists() && !dir.join(".gitignore").exists());
        let d = crate::delivery::local_of(&st, "no-git", &loft_ticket(&st, "no-git", "Offline mode").id).unwrap();
        assert_eq!((d.state.as_str(), d.base.as_str()), ("planned", ""));
        assert!(woken(&st, &dev).is_empty());

        // Yes: git is set up with a first commit that leaves secret files out, then the Delivery starts.
        std::fs::write(dir.join(".env"), "KEY=1\n").unwrap();
        let v = press_with(&st, &a, &m, json!({ "init_git": true })).await.unwrap();
        assert_eq!(v["ok"], true);
        assert!(dir.join(".git").is_dir());
        assert!(std::fs::read_to_string(dir.join(".gitignore")).unwrap().contains(".DS_Store"));
        assert_eq!(git_in(&dir, &["log", "--format=%s"]), "First commit");
        assert_eq!(git_in(&dir, &["show", "--name-only", "--format=", "HEAD"]), ".gitignore\nnotes.md");
        assert_eq!(git_in(&dir, &["status", "--porcelain"]), "", "the ignored .env isn't left over as a change");
        assert!(dir.join(".env").exists());
        let d = st.db.get_delivery(&d.repo, d.spec).unwrap().unwrap();
        assert_eq!(d.state, "running");
        assert!(git_in(&dir, &["branch", "--list", "spec/1-offline-mode"]).contains("spec/1-offline-mode"));
        assert!(woken(&st, &dev)[0].contains("build ticket \"Store\""));
        // Once it has git, Start delivery doesn't ask again.
        assert_eq!(press(&st, &a, &m).await.unwrap()["duplicate"], true);
    }

    #[tokio::test]
    async fn a_project_whose_git_isnt_a_folder_of_its_own_is_refused_and_not_offered_git() {
        let st = state();
        let (a, m, dev, dir) = plan_without_git(&st, "gitdir-file").await;
        // Its .git is a `gitdir:` file pointing at a repo in an agent's workspace.
        let agent_repo = st.cfg.storage_dir.join("some-agent").join("repo");
        std::fs::create_dir_all(&agent_repo).unwrap();
        git_in(&agent_repo, &["init", "-q", "-b", "main"]);
        git_in(&agent_repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        std::fs::write(dir.join(".git"), format!("gitdir: {}\n", agent_repo.join(".git").display())).unwrap();

        let err = press(&st, &a, &m).await.unwrap_err();
        assert_eq!(err.1, "The Delivery didn't start: the project folder of gitdir-file isn't a git repo of its own in the projects folder, so Loft won't run git there.");
        let err = press_with(&st, &a, &m, json!({ "init_git": true })).await.unwrap_err();
        assert!(err.1.contains("isn't a git repo of its own"), "{}", err.1);
        assert!(git_in(&agent_repo, &["branch", "--list", "spec/*"]).is_empty(), "nothing ran in the agent's repo");
        assert!(woken(&st, &dev).is_empty());
        let d = crate::delivery::local_of(&st, "gitdir-file", &loft_ticket(&st, "gitdir-file", "Offline mode").id).unwrap();
        crate::delivery::update(&st, &d.repo, d.spec, |d| (d.state, d.base) = ("ready".into(), "main".into()));
        assert!(crate::delivery::merge(&st, &d.repo, d.spec).await.unwrap_err().starts_with("Nothing was merged: the project folder of gitdir-file isn't a git repo of its own"));
    }

    #[test]
    fn spec_prs_recorded_before_deliveries_become_delivery_records() {
        let dir = crate::testdir::for_this_test("test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loft.db");
        drop(crate::db::Db::open(&path).unwrap());
        let conn = rusqlite::Connection::open(&path).unwrap();
        for (slug, kind, key, n) in [
            ("octo/a", "spec", "rounds", 40),
            ("octo/a", "spec_pr", "40", 500),
            ("octo/a", "sub_issue", "40/41", 41),
            ("octo/a", "sub_issue", "40/42", 42),
            ("octo/a", "blocked_by", "42/41", 41),
            ("octo/a", "spec_pr_retry", "500", 500),
            ("octo/a", "spec_pr", "60", 600),
            ("octo/a", "spec_pr_settled", "600", 600),
            ("octo/b", "spec_pr", "40", 700),
            ("octo/b", "sub_issue", "40/43", 43),
            ("octo/a", "close_comment", "41 Done in `abc1234`.", 41),
        ] {
            conn.execute("INSERT INTO published_issues (slug, kind, key, number, created_at) VALUES (?1, ?2, ?3, ?4, '2026-10-01T00:00:00Z')", rusqlite::params![slug, kind, key, n]).unwrap();
        }
        drop(conn);

        let db = crate::db::Db::open(&path).unwrap();
        let a40 = db.get_delivery("octo/a", 40).unwrap().unwrap();
        assert_eq!((a40.pr, a40.state.as_str(), a40.ready_retry, a40.branch.as_str(), a40.migrated), (Some(500), "planned", true, "", true));
        let ts: Vec<_> = a40.tickets.iter().map(|t| (t.number, t.blocked_by.clone(), t.state.as_str())).collect();
        assert_eq!(ts, [(41, vec![], "open"), (42, vec![41], "open")]);
        let a60 = db.get_delivery("octo/a", 60).unwrap().unwrap();
        assert_eq!((a60.pr, a60.state.as_str(), a60.ready_retry, a60.tickets.len()), (Some(600), "settled", false, 0));
        let b40 = db.get_delivery("octo/b", 40).unwrap().unwrap();
        assert_eq!((b40.pr, b40.tickets.iter().map(|t| t.number).collect::<Vec<_>>()), (Some(700), vec![43]));
        // The old kinds are gone; the rest stays.
        for kind in ["spec_pr", "spec_pr_retry", "spec_pr_settled"] {
            assert_eq!(db.published_issue("octo/a", kind, "40").unwrap(), None);
            assert_eq!(db.published_issue("octo/a", kind, "500").unwrap(), None);
        }
        assert_eq!(db.published_issue("octo/a", "spec", "rounds").unwrap(), Some(40));
        assert_eq!(db.published_issue("octo/a", "sub_issue", "40/41").unwrap(), Some(41));
        // Once: a Delivery changed afterwards isn't rebuilt on the next start.
        let mut d = a40.clone();
        d.state = "running".into();
        db.save_delivery(&d).unwrap();
        drop(db);
        assert_eq!(crate::db::Db::open(&path).unwrap().get_delivery("octo/a", 40).unwrap().unwrap().state, "running");
    }

    #[tokio::test]
    async fn a_full_team_gets_no_setup_button_and_an_unpublished_grill_no_card() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "card-full", Some("git@github.com:octo/card-full.git"));
        tickets_repo(&fake, "octo/card-full", 7, &[7]);
        for (name, role) in [("Builder", "dev"), ("Checker", "code_reviewer"), ("Tester", "qa"), ("Guard", "security")] {
            let mut x = crate::tasks::tests::agent(&st, name);
            x.group = " card-full ".into();
            x.role = role.into();
            st.db.save_agent(&x).unwrap();
        }
        crate::grill::tests::round(&st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        publish(&st, &a, "Rounds").await.unwrap();
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        let card = &summary(&st, &a).meta["grill"]["end_card"];
        assert_eq!((&card["missing_roles"], &card["setup"], &card["tickets"]), (&json!([]), &Value::Null, &json!([])));
        assert_eq!((&card["next_message"], &card["delivery"]), (&json!(""), &json!({ "repo": "octo/card-full", "spec": 7 })));

        // A grill that stopped before publishing ends with the summary only.
        crate::grill::tests::round(&st, &a, &["Another?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Nothing published." })).unwrap().unwrap();
        assert_eq!(summary(&st, &a).meta["grill"]["end_card"], Value::Null);
    }

    #[tokio::test]
    async fn the_team_check_goes_by_stored_roles_not_names() {
        let st = state();
        let add = |name: &str, title: &str, role: &str, project: &str| {
            let mut x = crate::tasks::tests::agent(&st, name);
            (x.title, x.role, x.group) = (title.into(), role.into(), project.into());
            st.db.save_agent(&x).unwrap();
            x
        };
        let all = ["dev", "code_reviewer", "qa", "security"];
        assert_eq!(missing_roles(&st, "team-a"), all);
        add("Lead", "Team lead", "lead", "team-a");
        add("Dev", "Feature developer", "custom", "team-a");
        add("Builder", "", "dev", "team-b");
        assert_eq!(missing_roles(&st, "team-a"), all, "names, titles, the Lead and other projects don't count");
        // A QA engineer is QA: it doesn't make the team's Dev or its other reviewers.
        add("QA", "QA engineer", "qa", "team-a");
        assert_eq!(missing_roles(&st, "team-a"), ["dev", "code_reviewer", "security"]);
        add("Reviewer", "", "code_reviewer", "team-a");
        add("Guard", "", "security", "team-a");
        // A renamed Dev still counts.
        let mut dev = add("Dev", "Feature developer", "dev", "team-a");
        (dev.name, dev.title) = ("Ana".into(), "Frontend".into());
        st.db.save_agent(&dev).unwrap();
        assert!(missing_roles(&st, "team-a").is_empty(), "a full team");
        assert_eq!(missing_roles(&st, " "), all, "no project, no team");
        // A group is one team across its projects; its agents leave their projects' teams.
        let mut b = add("Web", "", "qa", "team-b");
        b.group = "Acme".into();
        st.db.save_agent(&b).unwrap();
        for (name, role) in [("Api", "dev"), ("Rev", "code_reviewer"), ("Sec", "security")] {
            let mut c = add(name, "", role, "team-c");
            c.group = "Acme".into();
            st.db.save_agent(&c).unwrap();
        }
        assert!(missing_roles(&st, "Acme").is_empty());
        assert_eq!(missing_roles(&st, "team-b"), ["code_reviewer", "qa", "security"], "Web is in Acme now: team-b keeps only Builder, a Dev");
    }

    #[test]
    fn roles_come_from_names_and_titles_only_for_agents_without_one() {
        assert_eq!(infer_role("QA", "QA engineer"), "qa");
        assert_eq!(infer_role("QA", "Test keeper"), "qa");
        assert_eq!(infer_role("Security", "Security auditor"), "security");
        assert_eq!(infer_role("Loft Reviewer", ""), "code_reviewer");
        assert_eq!(infer_role("Ana", "Desenvolvedora"), "dev");
        assert_eq!(infer_role("Dev", "Feature developer"), "dev");
        assert_eq!(infer_role("Devious", "Docs writer"), "custom");
        assert_eq!(infer_role("Lead", "Team lead"), "custom");
        assert_eq!(reviewer_role("Checker", "Test runner"), "qa");
        assert_eq!(reviewer_role("Shield", "Security auditor"), "security");
        assert_eq!(reviewer_role("Loft Reviewer", "Code reviewer"), "code_reviewer");
        assert_eq!(reviewer_role("Ana", ""), "code_reviewer");
    }

    #[test]
    fn agents_get_the_six_roles_once_and_keep_their_schedules() {
        let dir = crate::testdir::for_this_test("test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loft.db");
        let db = crate::db::Db::open(&path).unwrap();
        let st = AppState { db, ..state() };
        // As stored before the six roles: (name, title, `lead`, `role`, None for no `role` key at all).
        let old: &[(&str, &str, bool, Option<&str>)] = &[
            ("Captain", "Team lead", true, Some("")),
            ("Builder Lead", "Feature developer", true, Some("dev")),
            ("Loft Reviewer", "Code reviewer", false, Some("reviewer")),
            ("QA", "Test keeper", false, Some("reviewer")),
            ("Tester", "", false, Some("reviewer")),
            ("Shield", "Security auditor", false, Some("reviewer")),
            ("Loft Dev", "Feature developer", false, Some("dev")),
            ("Docs", "Docs keeper", false, Some("")),
            ("Ancient", "Security auditor", false, None),
            ("Ancient Lead", "", true, None),
        ];
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut ids = vec![];
        for (i, (name, title, lead, role)) in old.iter().enumerate() {
            let mut a = crate::tasks::tests::agent(&st, name);
            (a.title, a.project, a.interval_minutes) = (title.to_string(), "team".into(), 60 * (i as i64 + 1));
            a.next_run_at = Some(format!("2026-11-0{}T10:00:00Z", i % 9 + 1));
            st.db.save_agent(&a).unwrap();
            let mut raw = serde_json::to_value(&a).unwrap();
            let o = raw.as_object_mut().unwrap();
            o.insert("lead".into(), json!(lead));
            match role {
                Some(r) => o.insert("role".into(), json!(r)),
                None => o.remove("role"),
            };
            conn.execute("UPDATE agents SET data = ?1 WHERE id = ?2", rusqlite::params![raw.to_string(), a.id]).unwrap();
            ids.push(a.id);
        }
        // Already on the six roles: left alone, even a role its name doesn't suggest.
        let mut set = crate::tasks::tests::agent(&st, "Dev");
        set.role = "security".into();
        st.db.save_agent(&set).unwrap();
        drop(st);

        let db = crate::db::Db::open(&path).unwrap();
        let roles: Vec<String> = ids.iter().map(|id| db.get_agent(id).unwrap().unwrap().role).collect();
        assert_eq!(roles, ["lead", "lead", "code_reviewer", "qa", "qa", "security", "dev", "custom", "security", "lead"]);
        for (i, id) in ids.iter().enumerate() {
            let a = db.get_agent(id).unwrap().unwrap();
            assert_eq!((a.interval_minutes, a.next_run_at.as_deref()), (60 * (i as i64 + 1), Some(format!("2026-11-0{}T10:00:00Z", i % 9 + 1)).as_deref()), "{} keeps its schedule", a.name);
        }
        assert_eq!(db.get_agent(&set.id).unwrap().unwrap().role, "security");
        // The `lead` flag is gone from what is stored, so the next start changes nothing.
        let raw: String = rusqlite::Connection::open(&path).unwrap().query_row("SELECT data FROM agents WHERE id = ?1", [&ids[0]], |r| r.get(0)).unwrap();
        assert!(serde_json::from_str::<Value>(&raw).unwrap().get("lead").is_none(), "{raw}");
        let mut a = db.get_agent(&ids[0]).unwrap().unwrap();
        a.role = "dev".into();
        db.save_agent(&a).unwrap();
        drop(db);
        assert_eq!(crate::db::Db::open(&path).unwrap().get_agent(&ids[0]).unwrap().unwrap().role, "dev", "migrated once: the user's later choice stays");
    }

    // ---- stalled Deliveries (#335) ----

    /// The run that takes `a`'s queued work starts: it takes the Delivery steps queued with it. Agents are
    /// busy in these tests, so this does by hand what the run loop does.
    fn run_starts(st: &AppState, a: &Agent) {
        let mut rt = st.rt.lock().unwrap();
        rt.queued.remove(&a.id);
        crate::agents::take_delivery_queue(&mut rt, &a.id);
    }

    fn delivery(st: &AppState, slug: &str) -> crate::db::Delivery {
        st.db.get_delivery(slug, 40).unwrap().unwrap()
    }

    fn stalled_items(st: &AppState) -> Vec<Value> {
        let v = crate::needs_you::snapshot(st).unwrap();
        v["items"].as_array().unwrap().iter().filter(|i| i["type"] == "delivery_stalled").cloned().collect()
    }

    /// The spec's Loft issue, as the mirror brings it in. The fake answers for it too: publishing the tickets
    /// started a full read in the background, which asks about a linked issue it didn't list and, on a 404,
    /// deletes it.
    fn spec_issue(st: &AppState, fake: &Fake, project: &str, slug: &str) -> crate::db::Task {
        let mut spec = gh_ticket(slug, 40, "open", "Spec.");
        (spec["title"], spec["labels"]) = (json!("Rounds"), json!([{ "name": "spec" }]));
        fake.route("GET", &format!("/repos/{slug}/issues/40"), 200, spec.clone());
        crate::mirror::bring_in(st, project, slug, &spec).unwrap().0
    }

    #[tokio::test]
    async fn a_delivery_run_that_ends_without_its_call_stalls_the_delivery() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-end";
        let (dev, _) = building(&st, &fake, "st-end", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "st-end", slug);

        // The Dev's run ends without submit_ticket.
        run_starts(&st, &dev);
        let mut events = st.events.subscribe();
        crate::delivery::run_ended(&st, &dev.id, false).await;
        let told = std::iter::from_fn(|| events.try_recv().ok()).any(|e| matches!(e, crate::state::Event::Delivery { ref repo, spec: 40 } if repo == slug));
        assert!(told, "the clients showing the Delivery are told it changed");
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.resume_to.as_str(), d.stall_reason.as_str()), ("stalled", "running", "no_call"));
        assert_eq!(d.tickets[0].state, "started", "the ticket keeps its own state");

        // One Needs you item, opening the spec's issue, pushed as delivery_stalled with names and titles only.
        let items = stalled_items(&st);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!((&items[0]["title"], &items[0]["urgency"]), (&json!("Delivery stalled: Rounds"), &json!(2)));
        assert_eq!(items[0]["reason"], "The run Loft woke for its step ended without its call. Stopped at: building #41 on Builder.");
        assert_eq!(items[0]["actions"][1]["target"], json!({ "screen": "task", "task_id": spec.id }));
        let all = crate::needs_you::collect(&st).unwrap();
        let p = crate::needs_you::to_push(all.iter().find(|i| i.kind == "delivery_stalled").unwrap());
        assert_eq!((p.kind, p.title.as_str(), p.body.as_str()), ("delivery_stalled", "Delivery stalled", "Rounds"));
        assert_eq!(p.target, json!({ "screen": "task", "task_id": spec.id }));
        assert!(crate::apns::DEFAULT_KINDS.contains(&"delivery_stalled") && crate::apns::KINDS.contains(&"delivery_stalled"), "on by default");

        // The issue page shows the state and why.
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&shown["state"], &shown["stall_reason"], &shown["needs_human"]), (&json!("stalled"), &json!("no_call"), &json!(false)));

        // The mirror's catch-up starts nothing while it's stalled.
        crate::delivery::retry(&st, slug).await;
        assert_eq!(delivery(&st, slug).state, "stalled");

        // The call comes after all (the user nudged the Dev): the Delivery carries on, and the item goes.
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.stall_reason.as_str(), d.tickets[0].state.as_str()), ("running", "", "review"));
        assert!(stalled_items(&st).is_empty());
    }

    #[tokio::test]
    async fn a_run_that_makes_its_call_doesnt_stall_and_a_failed_one_says_so() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-call";
        let (dev, team) = building(&st, &fake, "st-call", &["qa", "security"]).await;
        let (qa, sec) = (&team[0], &team[1]);

        run_starts(&st, &dev);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).state, "running", "the Dev's call came");

        run_starts(&st, qa);
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        crate::delivery::run_ended(&st, &qa.id, false).await;
        assert_eq!(delivery(&st, slug).state, "running", "QA's verdict came; only Security's is pending");

        // Security's run fails before its verdict.
        run_starts(&st, sec);
        crate::delivery::run_ended(&st, &sec.id, true).await;
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.stall_reason.as_str()), ("stalled", "run_failed"));
        assert_eq!(stalled_items(&st)[0]["reason"], "The run Loft woke for its step failed. Stopped at: the review of #41.");
        // A stalled Delivery isn't stalled again by another run's end.
        let at = d.stalled_at.clone();
        run_starts(&st, qa);
        crate::delivery::run_ended(&st, &qa.id, false).await;
        assert_eq!(delivery(&st, slug).stalled_at, at);
    }

    #[tokio::test]
    async fn a_delivery_wake_still_queued_keeps_the_delivery_from_stalling() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-queued";
        let (dev, _) = building(&st, &fake, "st-queued", &["qa"]).await;
        run_starts(&st, &dev);
        // Another Delivery wake for the Dev arrives while its run is going.
        let again = crate::delivery::Wake { repo: slug.into(), spec: 40, step: crate::delivery::Step::Build { ticket: 41, round: 0 } };
        crate::agents::wake_on_ticket(&st, &dev.id, "octo/st-queued#41", true, "again".into(), again);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).state, "running", "the run that takes the queued wake carries on");
        // That run ends without the call too.
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).state, "stalled");
    }

    /// QA and Security review together: a stall from QA's run stays until QA's verdict comes, whatever
    /// Security says meanwhile. Changes from either end the review step for both.
    #[tokio::test]
    async fn one_reviewers_pass_doesnt_clear_a_stall_from_the_other_reviewer() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-two";
        let (dev, team) = building(&st, &fake, "st-two", &["qa", "security"]).await;
        let (qa, sec) = (&team[0], &team[1]);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        run_starts(&st, qa);
        crate::delivery::run_ended(&st, &qa.id, false).await;
        assert_eq!(delivery(&st, slug).state, "stalled");

        tool(&st, sec, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.tickets[0].verdicts.get("qa").map(String::as_str)), ("stalled", Some("pending")), "QA's verdict is still missing");
        assert_eq!(stalled_items(&st).len(), 1, "the user is still told");

        // QA's late pass ends the step: the Delivery carries on and closes #41.
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.tickets[0].state.as_str()), ("running", "closed"));
        assert!(stalled_items(&st).is_empty());
    }

    #[tokio::test]
    async fn changes_from_the_other_reviewer_end_the_stalled_review() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-changes";
        let (dev, team) = building(&st, &fake, "st-changes", &["qa", "security"]).await;
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        run_starts(&st, &team[0]);
        crate::delivery::run_ended(&st, &team[0].id, false).await;
        tool(&st, &team[1], "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "It leaks the token." })).await.unwrap();
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.tickets[0].state.as_str()), ("running", "started"), "back with the Dev, who is woken");
    }

    /// An agent on two Deliveries: its run for spec #40 ends without its call while a wake for another
    /// Delivery is queued. The run that takes that wake checks #40's step too.
    #[tokio::test]
    async fn a_queued_wake_for_another_delivery_keeps_the_ended_step_for_the_next_run() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-other";
        let (dev, _) = building(&st, &fake, "st-other", &["qa"]).await;
        run_starts(&st, &dev);
        let other = crate::delivery::Wake { repo: "octo/other".into(), spec: 99, step: crate::delivery::Step::Build { ticket: 100, round: 0 } };
        crate::agents::wake_on_ticket(&st, &dev.id, "octo/other#100", true, "other work".into(), other);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).state, "running", "the next run may still make the call");
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).state, "stalled", "it didn't");
    }

    #[tokio::test]
    async fn a_restart_stalls_the_deliveries_with_a_step_in_flight() {
        let st = state();
        let slug = "octo/st-restart";
        planner_on(&st, "st-restart", Some("git@github.com:octo/st-restart.git"));
        let mut d: crate::db::Delivery = serde_json::from_value(json!({ "repo": slug, "spec": 40, "project": "st-restart", "state": "running", "created_at": "" })).unwrap();
        let t = |n: i64, state: &str| crate::db::DeliveryTicket { number: n, title: format!("T{n}"), blocked_by: vec![], state: state.into(), rounds: 0, commits: vec![], verdicts: Default::default(), id: String::new(), skipped: String::new(), agent: String::new(), notes: String::new() };
        let mut save = |spec: i64, state: &str, tickets: Vec<crate::db::DeliveryTicket>| {
            (d.spec, d.state, d.tickets) = (spec, state.into(), tickets);
            st.db.save_delivery(&d).unwrap();
        };
        save(40, "running", vec![t(41, "closed"), t(42, "started")]);
        save(50, "running", vec![t(51, "review")]);
        save(60, "final_review", vec![t(61, "closed")]);
        save(70, "final_fixes", vec![t(71, "closed")]);
        save(80, "running", vec![t(81, "needs_human"), t(82, "open")]);
        save(90, "needs_human", vec![t(91, "closed")]);
        save(100, "planned", vec![t(101, "open")]);
        save(110, "running", vec![t(111, "closed"), t(112, "open")]);
        // A final review that passed waits on the ready retry, not on a run.
        save(120, "final_review", vec![t(121, "closed")]);
        crate::delivery::update(&st, slug, 120, |d| d.ready_retry = true);

        crate::delivery::stall_lost_runs(&st);
        let state_of = |spec: i64| {
            let d = st.db.get_delivery(slug, spec).unwrap().unwrap();
            (d.state, d.resume_to, d.stall_reason)
        };
        for (spec, back) in [(40, "running"), (50, "running"), (60, "final_review"), (70, "final_fixes")] {
            assert_eq!(state_of(spec), ("stalled".into(), back.into(), "restart".into()), "spec #{spec}");
        }
        for (spec, still) in [(80, "running"), (90, "needs_human"), (100, "planned"), (110, "running"), (120, "final_review")] {
            assert_eq!(state_of(spec).0, still, "spec #{spec} had no step in flight");
        }
        let items = stalled_items(&st);
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|i| i["reason"].as_str().unwrap().starts_with("Its run was lost when Loft restarted")), "{items:?}");
        let fixes = items.iter().find(|i| i["title"] == "Delivery stalled: spec #70").unwrap();
        assert_eq!(fixes["reason"], "Its run was lost when Loft restarted. Stopped at: the fixes from the final Code Review.");
        assert_eq!(fixes["actions"][1]["target"], json!({ "url": "https://github.com/octo/st-restart/issues/70" }), "not mirrored: its GitHub issue");
    }

    #[tokio::test]
    async fn the_final_review_stalls_and_its_late_verdict_still_counts() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/st-final";
        let (dev, team) = building_both(&st, &fake, "st-final", &["code_reviewer"]).await;
        let cr = &team[0];
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        run_starts(&st, cr);
        crate::delivery::run_ended(&st, &cr.id, false).await;
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.resume_to.as_str()), ("stalled", "final_review"));
        assert_eq!(stalled_items(&st)[0]["reason"], "The run Loft woke for its step ended without its call. Stopped at: the final Code Review.");

        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": "store.rs:12 drops the error." })).await.unwrap();
        assert!(out.contains("The Dev is fixing them (round 1 of 3)"), "{out}");
        assert_eq!(delivery(&st, slug).state, "final_fixes");
        // The Dev's fixes run ends without its submit.
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).resume_to, "final_fixes");
        tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["feed1234"] })).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "final_review");
    }

    #[tokio::test]
    async fn a_plan_kept_in_loft_stalls_the_same_way() {
        let st = state();
        let project = "st-local";
        let (a, m, team, _dir) = local_plan(&st, project, &["dev"]).await;
        press(&st, &a, &m).await.unwrap();
        let spec = loft_ticket(&st, project, "Offline mode");
        let d = crate::delivery::local_of(&st, project, &spec.id).unwrap();
        run_starts(&st, &team[0]);
        crate::delivery::run_ended(&st, &team[0].id, false).await;
        let d = st.db.get_delivery(&d.repo, d.spec).unwrap().unwrap();
        assert_eq!(d.state, "stalled");
        let items = stalled_items(&st);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "Delivery stalled: Offline mode");
        assert_eq!(items[0]["reason"], "The run Loft woke for its step ended without its call. Stopped at: building \"Store\" on dev agent.");
        assert_eq!(items[0]["actions"][1]["target"], json!({ "screen": "task", "task_id": spec.id }));
    }

    // ---- Resume a stalled Delivery (#336) ----

    /// Resume on the issue's Delivery card.
    async fn resume_card(st: &AppState, issue: &crate::db::Task) -> Result<Value, crate::api::ApiError> {
        crate::delivery::resume_from_issue(State(st.clone()), Path(issue.id.clone())).await.map(|j| j.0)
    }

    /// The "Delivery stalled" item's Resume, sent as Needs you gives it.
    async fn resume_from_needs_you(st: &AppState) -> Result<Value, crate::api::ApiError> {
        let items = stalled_items(st);
        assert_eq!(items.len(), 1, "{items:?}");
        let a = items[0]["actions"].as_array().unwrap().iter().find(|a| a["kind"] == "resume").unwrap().clone();
        assert_eq!((&a["label"], &a["method"], &a["path"]), (&json!("Resume"), &json!("POST"), &json!("/api/deliveries/resume")));
        crate::delivery::resume_route(State(st.clone()), axum::Json(serde_json::from_value(a["body"].clone()).unwrap())).await.map(|j| j.0)
    }

    fn commit(sha: &str, message: &str) -> Value {
        json!({ "sha": sha, "commit": { "message": message } })
    }

    #[tokio::test]
    async fn resume_wakes_the_dev_on_its_started_ticket_with_the_commits_already_pushed() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rs-build";
        let (dev, _) = building(&st, &fake, "rs-build", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "rs-build", slug);
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, true).await;
        assert_eq!(delivery(&st, slug).state, "stalled");
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!(shown["can_resume"], true, "the card offers Resume");

        // Newest first, as GitHub lists them: one for #41, one for #410 and one for #42.
        fake.route("GET", &format!("/repos/{slug}/commits"), 200, json!([
            commit("bbbb2222cccc", "Wire the API\n\nRefs #42"),
            commit("cccc3333dddd", "Unrelated\n\nRefs #410"),
            commit("aaaa1111bbbb", "Store table\n\nRefs #41"),
        ]));
        // Since then the Dev worked on another ticket, and its setting clears context at each new one.
        st.db.update_agent(&dev.id, |a| (a.fresh_context, a.delivery_ticket) = ("always".into(), Some("octo/other#9".into()))).unwrap();
        st.rt.lock().unwrap().fresh_before.remove(&dev.id); // taken by the run that took the first wake

        let v = resume_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Builder is building #41 again.");
        assert_eq!((&v["state"], &v["can_resume"], &v["stall_reason"]), (&json!("running"), &json!(false), &Value::Null));
        let commits = fake.seen(&format!("/repos/{slug}/commits"));
        assert_eq!(commits[0].path, format!("/repos/{slug}/commits?sha=spec/40-rounds&per_page=100"));
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.resume_to.as_str(), d.stall_reason.as_str(), d.tickets[0].state.as_str()), ("running", "", "", "started"));
        assert!(stalled_items(&st).is_empty(), "the stalled item goes");

        let w = woken(&st, &dev);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("You are the Dev on this Delivery: build ticket #41"), "the role brief: {}", w[0]);
        assert!(w[0].contains("This is a resume: the Delivery stalled because the run Loft woke for its step failed"), "{}", w[0]);
        assert!(w[0].contains("with `Refs #41`: `aaaa111` Store table."), "{}", w[0]);
        assert!(!w[0].contains("#410") && !w[0].contains("Wire the API"), "only this ticket's commits: {}", w[0]);
        assert!(!st.rt.lock().unwrap().fresh_before.contains(&dev.id), "a Resume on the same ticket isn't a ticket boundary");

        // The wake carries its step: a run that ends without the call stalls it again.
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).stall_reason, "no_call");
    }

    #[tokio::test]
    async fn resume_on_a_review_wakes_only_the_reviewers_still_pending() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rs-review";
        let (dev, team) = building(&st, &fake, "rs-review", &["qa", "security"]).await;
        let (qa, sec) = (&team[0], &team[1]);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        for a in [&dev, qa, sec] {
            run_starts(&st, a);
        }
        crate::delivery::run_ended(&st, &sec.id, false).await;
        assert_eq!(delivery(&st, slug).state, "stalled");

        let v = resume_from_needs_you(&st).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. security agent (Security) is reviewing #41 again.");
        assert!(woken(&st, qa).is_empty() && woken(&st, &dev).is_empty(), "QA passed already; the Dev waits");
        let w = woken(&st, sec);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("You are Security on this Delivery: review ticket #41") && w[0].contains("`abc1234`"), "{}", w[0]);
        assert!(w[0].contains("This is a resume") && w[0].contains("Verdicts already in: QA passed it."), "{}", w[0]);
        assert!(stalled_items(&st).is_empty());

        // Security's verdict closes it, as usual.
        tool(&st, sec, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        assert_eq!(delivery(&st, slug).tickets[0].state, "closed");
    }

    #[tokio::test]
    async fn resume_on_a_stalled_final_review_wakes_the_code_reviewer_then_the_fixes_the_dev() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rs-final";
        let (dev, team) = building_both(&st, &fake, "rs-final", &["code_reviewer"]).await;
        let cr = &team[0];
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        run_starts(&st, cr);
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &cr.id, false).await;
        assert_eq!(delivery(&st, slug).resume_to, "final_review");

        let v = resume_from_needs_you(&st).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. code_reviewer agent (Code reviewer) is doing the final review again.");
        assert_eq!(delivery(&st, slug).state, "final_review");
        let w = woken(&st, cr);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("You are the Code reviewer on this Delivery") && w[0].contains("This is a resume"), "{}", w[0]);
        assert!(woken(&st, &dev).is_empty());

        // Changes, then the Dev's fixes run stalls: Resume wakes the Dev with where the notes are.
        tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": "store.rs:12 drops the error." })).await.unwrap();
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        assert_eq!(delivery(&st, slug).resume_to, "final_fixes");
        fake.route("GET", &format!("/repos/{slug}/commits"), 200, json!([commit("feed1234aaaa", "Keep the error\n\nRefs #40"), commit("abc1234def", "Store\n\nRefs #41")]));
        let v = resume_from_needs_you(&st).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Builder is fixing what the final Code Review found again.");
        let w = woken(&st, &dev);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("fix what the Code reviewer asked for on the spec PR https://github.com/octo/rs-final/pull/500 (round 1 of 3). Its notes are its last comment there."), "{}", w[0]);
        assert!(w[0].contains("`feed123` Keep the error") && !w[0].contains("`abc1234` Store"), "{}", w[0]);
        tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["feed1234"] })).await.unwrap();
        assert_eq!(delivery(&st, slug).state, "final_review");
    }

    #[tokio::test]
    async fn resume_marks_a_ticket_closed_by_hand_closed_and_starts_the_next() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rs-hand";
        let (dev, _) = building(&st, &fake, "rs-hand", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "rs-hand", slug);
        // The full read publishing started may run only now: the spec's Loft issue stays.
        let _ = crate::mirror::sync_project(&st, "rs-hand", true).await;
        assert!(st.db.get_task(&spec.id).unwrap().is_some());
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        // The user closed #41 on GitHub meanwhile.
        subs_on_github(&fake, slug, &["closed", "open"]);

        let v = resume_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Closed by hand meanwhile, so marked closed: #41. Next: #42 is with Builder.");
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.current, d.tickets[0].state.as_str(), d.tickets[1].state.as_str()), ("running", Some(42), "closed", "started"));
        let w = woken(&st, &dev);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("build ticket #42") && !w[0].contains("This is a resume"), "a new ticket gets the plain Dev brief: {}", w[0]);

        // Every ticket closed by hand: on to the final review (here, with no Code reviewer, the spec PR is ready).
        run_starts(&st, &dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        subs_on_github(&fake, slug, &["closed", "closed"]);
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true }));
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));
        let v = resume_card(&st, &spec).await.unwrap();
        assert!(v["message"].as_str().unwrap().contains("marked closed: #42. It was the spec's last open ticket: spec PR #500 is now ready for review"), "{v}");
        assert_eq!(delivery(&st, slug).state, "ready");
    }

    #[tokio::test]
    async fn resume_is_refused_on_a_delivery_that_isnt_stalled() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rs-refuse";
        let (dev, _) = building(&st, &fake, "rs-refuse", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "rs-refuse", slug);
        let no = |e: crate::api::ApiError| e.1;
        let err = resume_card(&st, &spec).await.err().map(no).unwrap();
        assert_eq!(err, "spec #40 isn't stalled or paused: an agent is working on its step, so there's nothing to resume.");
        assert!(woken(&st, &dev).len() <= 1, "nothing more was woken");
        let before = woken(&st, &dev).len();
        for (state, says) in [("planned", "hasn't started"), ("ready", "is done")] {
            crate::delivery::update(&st, slug, 40, |d| d.state = state.into());
            let err = crate::delivery::resume(&st, slug, 40).await.unwrap_err();
            assert!(err.contains(says), "{state}: {err}");
        }
        assert_eq!(woken(&st, &dev).len(), before);
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!(shown["can_resume"], false);
    }

    #[tokio::test]
    async fn resume_on_a_plan_kept_in_loft_reads_its_loft_issues_and_the_local_branch() {
        let st = state();
        let project = "rs-local";
        let (a, m, team, dir) = local_plan(&st, project, &["dev"]).await;
        press(&st, &a, &m).await.unwrap();
        let (dev, store) = (&team[0], loft_ticket(&st, project, "Store"));
        let d = crate::delivery::local_of(&st, project, &loft_ticket(&st, project, "Offline mode").id).unwrap();
        run_starts(&st, dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        // The Dev pushed one commit for Store to the spec branch before its run ended.
        git_in(&dir, &["checkout", "-q", "spec/1-offline-mode"]);
        std::fs::write(dir.join("store.rs"), "// store\n").unwrap();
        git_in(&dir, &["add", "."]);
        git_in(&dir, &["commit", "-q", "-m", &format!("Store table\n\nRefs {}", store.id)]);
        git_in(&dir, &["checkout", "-q", "main"]);

        let out = resume_from_needs_you(&st).await.unwrap();
        assert_eq!(out["message"], "Resumed spec \"Offline mode\". dev agent is building \"Store\" again.");
        let w = woken(&st, dev);
        assert!(w[0].contains("This is a resume") && w[0].contains("Store table"), "{}", w[0]);

        // Stalls again; the user marked Store done meanwhile: API starts.
        run_starts(&st, dev);
        crate::delivery::run_ended(&st, &dev.id, false).await;
        let mut x = st.db.get_task(&store.id).unwrap().unwrap();
        crate::tasks::set_status(&st, &mut x, "done", "user", None);
        let out = crate::delivery::resume(&st, &d.repo, d.spec).await.unwrap();
        assert!(out.contains("marked closed: \"Store\"") && out.contains("Next: \"API\" is with dev agent."), "{out}");
        let d = st.db.get_delivery(&d.repo, d.spec).unwrap().unwrap();
        assert_eq!((d.state.as_str(), d.tickets[0].state.as_str(), d.tickets[1].state.as_str()), ("running", "closed", "started"));
    }

    /// A stall caught after the step's work was done but before Loft moved on: every verdict in, or the close
    /// pending. Resume only closes the ticket and starts the next, waking no reviewer.
    #[tokio::test]
    async fn resume_closes_a_ticket_whose_verdicts_are_all_in_or_whose_close_was_pending() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/rs-close";
        let (dev, team) = building_both(&st, &fake, "rs-close", &["qa"]).await;
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        run_starts(&st, &team[0]);
        run_starts(&st, &dev);
        let stall = |ticket: i64, state: &str, verdict: &str| {
            crate::delivery::update(&st, slug, 40, |d| {
                let t = d.ticket_mut(ticket).unwrap();
                t.state = state.into();
                t.verdicts.insert("qa".into(), verdict.into());
                (d.current, d.state, d.resume_to, d.stall_reason) = (Some(ticket), "stalled".into(), "running".into(), "restart".into());
            });
        };

        stall(41, "review", "pass");
        let out = crate::delivery::resume(&st, slug, 40).await.unwrap();
        assert!(out.contains("Closed #41") && out.contains("Next: #42 is with Builder."), "{out}");
        assert!(woken(&st, &team[0]).is_empty(), "no reviewer is woken");
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.tickets[0].state.as_str(), d.tickets[1].state.as_str()), ("running", "closed", "started"));

        // #42's close failed after its pass, then the Delivery stalled.
        run_starts(&st, &dev);
        stall(42, "closing", "pass");
        crate::delivery::update(&st, slug, 40, |d| d.ticket_mut(42).unwrap().commits = vec!["0123abcd".into()]);
        let out = crate::delivery::resume(&st, slug, 40).await.unwrap();
        assert!(out.contains("Closed #42"), "{out}");
        assert_eq!(delivery(&st, slug).tickets[1].state, "closed");
        assert!(stalled_items(&st).is_empty());
    }

    // ---- Resume a ticket or final review waiting for the user (#337) ----

    /// Three changes rounds from QA on #41, numbered from `first`: the reply to the last.
    async fn three_changes(st: &AppState, dev: &Agent, qa: &Agent, first: u32) -> String {
        let mut out = String::new();
        for round in first..first + 3 {
            tool(st, dev, "submit_ticket", submit_41()).await.unwrap();
            out = tool(st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": format!("Change {round}: the total is off by one.") })).await.unwrap();
        }
        out
    }

    /// The Needs you item of the blocked Loft issue `id`.
    fn blocked_item(st: &AppState, id: &str) -> Option<Value> {
        let v = crate::needs_you::snapshot(st).unwrap();
        v["items"].as_array().unwrap().iter().find(|i| i["type"] == "task_blocked" && i["task_id"] == id).cloned()
    }

    /// The Resume on that item, sent as Needs you gives it.
    async fn resume_blocked(st: &AppState, id: &str) -> Result<Value, crate::api::ApiError> {
        let item = blocked_item(st, id).unwrap_or_else(|| panic!("no blocked item for {id}"));
        let a = item["actions"][0].clone();
        assert_eq!((&a["kind"], &a["label"], &a["method"], &a["path"]), (&json!("resume"), &json!("Resume"), &json!("POST"), &json!("/api/deliveries/resume")), "{item}");
        crate::delivery::resume_route(State(st.clone()), axum::Json(serde_json::from_value(a["body"].clone()).unwrap())).await.map(|j| j.0)
    }

    fn github_task(st: &AppState, slug: &str, n: i64) -> crate::db::Task {
        st.db.list_tasks().unwrap().into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.repo == slug && g.number == n)).unwrap()
    }

    #[tokio::test]
    async fn resuming_a_needs_human_ticket_gives_the_dev_that_tickets_notes_not_the_deliverys_last() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/nh-own-notes";
        let (dev, team) = building(&st, &fake, "nh-own-notes", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "nh-own-notes", slug);
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        fake.route("DELETE", &format!("/repos/{slug}/issues/41/labels/needs-human"), 200, json!([]));
        fake.route("GET", &format!("/repos/{slug}/commits"), 200, json!([commit("fed4321abcde", "Fix the total\n\nRefs #41")]));
        three_changes(&st, &dev, &team[0], 1).await;
        // Another ticket's changes verdict came after: the Delivery's last notes are now its.
        crate::delivery::update(&st, slug, 40, |d| d.last_notes = "Other ticket: wrong colour.".into()).unwrap();
        let ticket = github_task(&st, slug, 41);
        resume_blocked(&st, &ticket.id).await.unwrap();
        let brief = woken(&st, &dev).last().cloned().unwrap();
        assert!(brief.contains("Change 3: the total is off by one.") && !brief.contains("wrong colour"), "{brief}");
        let _ = spec;
    }

    #[tokio::test]
    async fn resume_on_a_needs_human_ticket_resets_its_rounds_and_wakes_the_dev_with_the_last_notes() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/nh-ticket";
        let (dev, team) = building(&st, &fake, "nh-ticket", &["qa"]).await;
        let qa = &team[0];
        let spec = spec_issue(&st, &fake, "nh-ticket", slug);
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        fake.route("DELETE", &format!("/repos/{slug}/issues/41/labels/needs-human"), 200, json!([]));
        fake.route("GET", &format!("/repos/{slug}/commits"), 200, json!([commit("fed4321abcde", "Fix the total\n\nRefs #41")]));
        let out = three_changes(&st, &dev, qa, 1).await;
        assert!(out.contains("That's round 3 of changes on #41, so it waits for the user now."), "{out}");
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&shown["can_resume"], &shown["needs_human"]), (&json!(true), &json!(true)), "the card offers Resume");
        let ticket = github_task(&st, slug, 41);
        assert_eq!(ticket.status, "blocked");
        let before = woken(&st, &dev).len();

        let v = resume_blocked(&st, &ticket.id).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Builder is fixing #41 again, with a fresh 3 rounds of changes.");
        assert!(fake.seen(&format!("/repos/{slug}/issues/41/labels/needs-human")).iter().any(|s| s.method == "DELETE"), "the label comes off");
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.current, d.tickets[0].state.as_str(), d.tickets[0].rounds), ("running", Some(41), "started", 0));
        let ticket = st.db.get_task(&ticket.id).unwrap().unwrap();
        assert_eq!((ticket.status.as_str(), ticket.error.as_str()), ("in_progress", ""), "the Loft issue is unblocked");
        assert!(blocked_item(&st, &ticket.id).is_none());
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&shown["can_resume"], &shown["needs_human"]), (&json!(false), &json!(false)));

        let w = woken(&st, &dev);
        assert_eq!(w.len(), before + 1, "{w:?}");
        let brief = w.last().unwrap();
        assert!(brief.contains("You are the Dev on this Delivery: build ticket #41"), "the role brief: {brief}");
        assert!(brief.contains("This is a resume: #41 waited for the user after 3 rounds of changes; they looked at it, so it gets a fresh 3 rounds, and the user pressed Resume."), "{brief}");
        assert!(brief.contains("The last notes asking for changes, also on the spec PR https://github.com/octo/nh-ticket/pull/500:\n\nChange 3: the total is off by one."), "{brief}");
        assert!(!brief.contains("Change 2:"), "only the last notes: {brief}");

        // A fresh set of rounds: the first two go back to the Dev, the third to the user again.
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        let out = tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Change 4." })).await.unwrap();
        assert!(out.ends_with("The Dev is fixing #41 (round 1 of 3)."), "{out}");
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Change 5." })).await.unwrap();
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        let out = tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "changes", "notes": "Change 6." })).await.unwrap();
        assert!(out.contains("That's round 3 of changes on #41, so it waits for the user now."), "{out}");
        assert_eq!(posts(&fake, slug, "issues/41/labels").iter().filter(|l| **l == json!({ "labels": ["needs-human"] })).count(), 2);
        assert_eq!(st.db.get_task(&ticket.id).unwrap().unwrap().status, "blocked");
        assert_eq!(delivery(&st, slug).last_notes, "Change 6.");
    }

    #[tokio::test]
    async fn resume_on_a_needs_human_final_review_wakes_the_dev_to_fix_then_the_code_reviewer() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/nh-final";
        let (dev, team) = building_both(&st, &fake, "nh-final", &["code_reviewer"]).await;
        let cr = &team[0];
        let spec = spec_issue(&st, &fake, "nh-final", slug);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        fake.route("POST", &format!("/repos/{slug}/issues/500/labels"), 200, json!([]));
        fake.route("DELETE", &format!("/repos/{slug}/issues/500/labels/needs-human"), 200, json!([]));
        for round in 1..=3 {
            tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": format!("Round {round}: a race in retry.") })).await.unwrap();
            if round < 3 {
                tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["feed1234"] })).await.unwrap();
            }
        }
        assert_eq!(delivery(&st, slug).state, "needs_human");
        assert_eq!(st.db.get_task(&spec.id).unwrap().unwrap().status, "blocked");
        assert!(blocked_item(&st, &spec.id).unwrap()["actions"][0]["kind"] == "resume", "Needs you offers Resume");
        let (dev_before, cr_before) = (woken(&st, &dev).len(), woken(&st, cr).len());

        let v = resume_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Builder is fixing what the final Code Review found, with a fresh 3 rounds of changes; then the Code reviewer looks again.");
        assert_eq!((&v["state"], &v["can_resume"]), (&json!("final_fixes"), &json!(false)));
        assert!(fake.seen(&format!("/repos/{slug}/issues/500/labels/needs-human")).iter().any(|s| s.method == "DELETE"), "the label comes off the spec PR");
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.final_rounds), ("final_fixes", 0));
        assert_eq!(st.db.get_task(&spec.id).unwrap().unwrap().status, "in_progress");
        let w = woken(&st, &dev);
        assert_eq!(w.len(), dev_before + 1);
        let brief = w.last().unwrap();
        assert!(brief.contains("fix what the Code reviewer asked for on the spec PR https://github.com/octo/nh-final/pull/500. Its notes are its last comment there."), "no round 0: {brief}");
        assert!(brief.contains("This is a resume: spec PR #500 waited for the user after 3 rounds of changes") && brief.ends_with("Round 3: a race in retry."), "{brief}");
        assert_eq!(woken(&st, cr).len(), cr_before, "the Code reviewer waits for the fixes");

        // The fixes go to the Code reviewer, and its next changes are round 1 of a fresh 3.
        let out = tool(&st, &dev, "submit_ticket", json!({ "number": 500, "commits": ["beef5678"] })).await.unwrap();
        assert_eq!(out, "Submitted the fixes for spec PR #500: code_reviewer agent (Code reviewer) is looking again.");
        assert_eq!(woken(&st, cr).len(), cr_before + 1);
        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "changes", "notes": "Still racy." })).await.unwrap();
        assert!(out.contains("The Dev is fixing them (round 1 of 3)"), "{out}");
    }

    #[tokio::test]
    async fn resume_on_a_needs_human_ticket_of_a_plan_kept_in_loft_unblocks_its_loft_issue() {
        let st = state();
        let fake = own_fake(&st);
        let project = "nh-local";
        let (a, m, team, _dir) = local_plan(&st, project, &["dev", "qa"]).await;
        let (dev, qa) = (&team[0], &team[1]);
        press(&st, &a, &m).await.unwrap();
        let store = loft_ticket(&st, project, "Store");
        for round in 1..=3 {
            tool(&st, dev, "submit_ticket", json!({ "number": store.id, "commits": ["abc1234def"] })).await.unwrap();
            tool(&st, qa, "review_ticket", json!({ "number": store.id, "verdict": "changes", "notes": format!("Round {round}: saving twice loses data.") })).await.unwrap();
        }
        assert_eq!(st.db.get_task(&store.id).unwrap().unwrap().status, "blocked");
        let before = woken(&st, dev).len();

        let v = resume_blocked(&st, &store.id).await.unwrap();
        assert_eq!(v["message"], "Resumed spec \"Offline mode\". dev agent is fixing \"Store\" again, with a fresh 3 rounds of changes.");
        assert_eq!(st.db.get_task(&store.id).unwrap().unwrap().status, "in_progress");
        let d = crate::delivery::local_of(&st, project, &loft_ticket(&st, project, "Offline mode").id).unwrap();
        assert_eq!((d.tickets[0].state.as_str(), d.tickets[0].rounds), ("started", 0));
        let w = woken(&st, dev);
        assert_eq!(w.len(), before + 1);
        let brief = w.last().unwrap();
        assert!(brief.contains(&format!("The last notes asking for changes, also on its Loft issue {}:\n\nRound 3: saving twice loses data.", store.id)), "{brief}");
        assert!(fake.seen("/").is_empty(), "nothing is labelled: no GitHub at all");
    }

    #[tokio::test]
    async fn resume_on_a_needs_human_ticket_closed_by_hand_starts_the_next() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/nh-hand";
        let (dev, team) = building(&st, &fake, "nh-hand", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "nh-hand", slug);
        fake.route("POST", &format!("/repos/{slug}/issues/41/labels"), 200, json!([]));
        three_changes(&st, &dev, &team[0], 1).await;
        // The user looked, and closed #41 on GitHub before the mirror noticed.
        subs_on_github(&fake, slug, &["closed", "open"]);
        let v = resume_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Closed by hand meanwhile, so marked closed: #41. Next: #42 is with Builder.");
        let d = delivery(&st, slug);
        assert_eq!((d.current, d.tickets[0].state.as_str(), d.tickets[1].state.as_str()), (Some(42), "closed", "started"));
        assert!(fake.seen(&format!("/repos/{slug}/issues/41/labels/needs-human")).is_empty(), "nothing to unlabel on a closed ticket");
    }

    // ---- paused Deliveries (#338) ----

    async fn pause_card(st: &AppState, issue: &crate::db::Task) -> Result<Value, crate::api::ApiError> {
        crate::delivery::pause_from_issue(State(st.clone()), Path(issue.id.clone())).await.map(|j| j.0)
    }

    /// Whether `a`'s run was told to stop.
    async fn stopped(kill: &std::sync::Arc<tokio::sync::Notify>) -> bool {
        tokio::time::timeout(std::time::Duration::from_millis(100), kill.notified()).await.is_ok()
    }

    #[tokio::test]
    async fn pause_stops_the_dev_run_drops_queued_delivery_wakes_and_never_stalls() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ps-run";
        let (dev, _) = building(&st, &fake, "ps-run", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "ps-run", slug);
        let shown = crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0;
        assert_eq!((&shown["can_pause"], &shown["can_resume"]), (&json!(true), &json!(false)), "a running Delivery shows Pause");

        // The Dev's run is building #41. Queued behind it: another wake for this Delivery, the user's
        // message, and a wake for another Delivery.
        run_starts(&st, &dev);
        let kill = st.rt.lock().unwrap().running[&dev.id].clone();
        let again = crate::delivery::Wake { repo: slug.into(), spec: 40, step: crate::delivery::Step::Build { ticket: 41, round: 0 } };
        crate::agents::wake_on_ticket(&st, &dev.id, "octo/ps-run#41", true, "again".into(), again);
        crate::agents::wake_for_user(&st, &dev.id, "from the user".into());
        let other = crate::delivery::Wake { repo: "octo/other".into(), spec: 99, step: crate::delivery::Step::Build { ticket: 100, round: 0 } };
        crate::agents::wake_on_ticket(&st, &dev.id, "octo/other#100", true, "other".into(), other.clone());

        let v = pause_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Paused spec #40. Stopped the run of Builder. Dropped the Delivery wake queued for Builder. Loft sends no more wakes until you resume it.");
        assert_eq!((&v["state"], &v["can_pause"], &v["can_resume"]), (&json!("paused"), &json!(false), &json!(true)), "the card shows Resume instead");
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.resume_to.as_str(), d.tickets[0].state.as_str()), ("paused", "running", "started"));
        assert!(stopped(&kill).await, "the run in flight was stopped, as the agent's Stop does");
        assert_eq!(woken(&st, &dev), ["from the user", "other"], "only this Delivery's queued wake is dropped");
        let queued: Vec<crate::delivery::Wake> = st.rt.lock().unwrap().delivery_queued[&dev.id].iter().map(|(w, _)| w.clone()).collect();
        assert_eq!(queued, [other]);

        // The stopped run ends (as failed, even): the Delivery isn't stalled, nor by a restart.
        crate::delivery::run_ended(&st, &dev.id, true).await;
        crate::delivery::stall_lost_runs(&st);
        assert_eq!(delivery(&st, slug).state, "paused");
        // No Needs you item, so no push.
        let v = crate::needs_you::snapshot(&st).unwrap();
        let items = v["items"].as_array().unwrap();
        assert!(!items.iter().any(|i| i["type"].as_str().unwrap().starts_with("delivery") || i["title"].as_str().unwrap().contains("Rounds")), "{items:?}");
        // A late call takes nothing in, and wakes nobody.
        let qa_before = st.db.list_agents().unwrap().into_iter().find(|a| a.role == "qa").map(|qa| woken(&st, &qa).len()).unwrap();
        let err = tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap_err();
        assert!(err.contains("The Delivery of spec #40 is paused"), "{err}");
        assert_eq!(delivery(&st, slug).tickets[0].state, "started");
        assert_eq!(st.db.list_agents().unwrap().into_iter().find(|a| a.role == "qa").map(|qa| woken(&st, &qa).len()).unwrap(), qa_before);
        // Pausing again is refused, saying why.
        let err = pause_card(&st, &spec).await.err().unwrap().1;
        assert_eq!(err, "spec #40 is already paused: Resume carries it on.");

        // Resume wakes the Dev on #41 with the resume brief.
        subs_on_github(&fake, slug, &["open", "open"]);
        fake.route("GET", &format!("/repos/{slug}/commits"), 200, json!([]));
        st.rt.lock().unwrap().fresh_before.remove(&dev.id);
        let v = resume_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. Builder is building #41 again.");
        assert_eq!((&v["state"], &v["can_pause"], &v["can_resume"]), (&json!("running"), &json!(true), &json!(false)));
        let w = woken(&st, &dev);
        let brief = w.last().unwrap();
        assert!(brief.contains("build ticket #41") && brief.contains("This is a resume: the user paused the Delivery, and Loft stopped the work on its step"), "{brief}");
        assert!(!st.rt.lock().unwrap().fresh_before.contains(&dev.id), "a Resume isn't a ticket boundary");
    }

    #[tokio::test]
    async fn pause_drops_a_queued_review_and_resume_wakes_only_the_pending_reviewer() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ps-review";
        let (dev, team) = building(&st, &fake, "ps-review", &["qa", "security"]).await;
        let (qa, sec) = (&team[0], &team[1]);
        let spec = spec_issue(&st, &fake, "ps-review", slug);
        run_starts(&st, &dev);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        crate::delivery::run_ended(&st, &dev.id, false).await;
        run_starts(&st, qa);
        tool(&st, qa, "review_ticket", json!({ "number": 41, "verdict": "pass", "notes": "ok" })).await.unwrap();
        crate::delivery::run_ended(&st, &qa.id, false).await;
        let sec_kill = st.rt.lock().unwrap().running[&sec.id].clone();
        assert_eq!(woken(&st, sec).len(), 1, "Security's review is queued behind its run");

        let v = pause_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Paused spec #40. Dropped the Delivery wake queued for security agent. Loft sends no more wakes until you resume it.");
        assert!(woken(&st, sec).is_empty());
        assert!(!stopped(&sec_kill).await, "Security's own run wasn't for this Delivery");

        subs_on_github(&fake, slug, &["open", "open"]);
        let v = resume_card(&st, &spec).await.unwrap();
        assert_eq!(v["message"], "Resumed spec #40. security agent (Security) is reviewing #41 again.");
        let w = woken(&st, sec);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("This is a resume: the user paused the Delivery") && w[0].contains("Verdicts already in: QA passed it."), "{}", w[0]);
        assert!(woken(&st, qa).is_empty() && woken(&st, &dev).is_empty(), "only the pending reviewer is woken");
    }

    #[tokio::test]
    async fn while_paused_the_catch_up_closes_and_starts_nothing() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ps-catch";
        let (dev, _) = building(&st, &fake, "ps-catch", &[]).await;
        crate::delivery::pause(&st, slug, 40).await.unwrap();
        // #41 passed, but its close failed: the catch-up would close it and start #42.
        crate::delivery::update(&st, slug, 40, |d| d.tickets[0].state = "closing".into());
        let before = woken(&st, &dev).len();
        crate::delivery::retry(&st, slug).await;
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.current, d.tickets[0].state.as_str(), d.tickets[1].state.as_str()), ("paused", Some(41), "closing", "open"));
        assert!(fake.seen(&format!("/repos/{slug}/issues/41")).iter().all(|m| m.method != "PATCH"), "nothing was closed");
        assert_eq!(woken(&st, &dev).len(), before);
        // Nor does its last ticket closed by hand move it on to its final review or a ready spec PR.
        let pr_calls = || fake.seen("/graphql").len() + fake.seen(&format!("/repos/{slug}/pulls/500")).len();
        let pr_before = pr_calls();
        crate::delivery::update(&st, slug, 40, |d| (d.tickets[0].state, d.tickets[1].state) = ("started".into(), "closed".into()));
        let _held = st.publish_locks.of(slug).lock_owned().await;
        assert_eq!(ready_when_last(&st, slug, 41, "").await.unwrap(), None);
        drop(_held);
        crate::delivery::retry(&st, slug).await;
        let d = delivery(&st, slug);
        assert_eq!((d.state.as_str(), d.current, d.tickets[0].state.as_str()), ("paused", None, "closed"));
        assert_eq!(pr_calls(), pr_before, "the spec PR wasn't touched");
        assert_eq!(woken(&st, &dev).len(), before);
    }

    #[tokio::test]
    async fn pause_applies_only_to_a_running_delivery_and_says_why_otherwise() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ps-refuse";
        building(&st, &fake, "ps-refuse", &["qa"]).await;
        let spec = spec_issue(&st, &fake, "ps-refuse", slug);
        let shown = || async { crate::delivery::issue_delivery(State(st.clone()), Path(spec.id.clone())).await.unwrap().0 };
        for (state, ticket, says) in [
            ("planned", "open", "spec #40 hasn't started, so there's nothing to pause."),
            ("stalled", "started", "spec #40 is stalled: no agent is working on it, so there's nothing to pause. Resume carries it on."),
            ("running", "needs_human", "spec #40 waits for you after too many changes rounds, so no agent is working on it. Resume gives it a fresh set of rounds."),
            ("needs_human", "closed", "spec #40 waits for you after too many changes rounds, so no agent is working on it. Resume gives it a fresh set of rounds."),
            ("ready", "closed", "spec #40 is done: every review passed, so there's nothing to pause."),
        ] {
            crate::delivery::update(&st, slug, 40, |d| (d.state, d.resume_to, d.tickets[0].state) = (state.into(), "running".into(), ticket.into()));
            assert_eq!(shown().await["can_pause"], false, "{state}");
            assert_eq!(pause_card(&st, &spec).await.err().unwrap().1, says, "{state}");
            assert_eq!(delivery(&st, slug).state, state, "unchanged");
        }
        // The final review and its fixes pause, and go back to where they were.
        for state in ["final_review", "final_fixes"] {
            crate::delivery::update(&st, slug, 40, |d| (d.state, d.resume_to) = (state.into(), String::new()));
            assert_eq!(shown().await["can_pause"], true, "{state}");
            pause_card(&st, &spec).await.unwrap();
            let d = delivery(&st, slug);
            assert_eq!((d.state.as_str(), d.resume_to.as_str()), ("paused", state));
        }
    }

    // ---- Delivery done (#339) ----

    fn done_items(st: &AppState) -> Vec<Value> {
        let v = crate::needs_you::snapshot(st).unwrap();
        v["items"].as_array().unwrap().iter().filter(|i| i["type"] == "delivery_done").cloned().collect()
    }

    /// The agent that planned spec #40 on `slug`, busy so its wakes queue where `woken` sees them.
    fn planner_of(st: &AppState, slug: &str) -> Agent {
        let a = st.db.get_agent(&delivery(st, slug).planner).unwrap().unwrap();
        st.rt.lock().unwrap().running.insert(a.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        a
    }

    /// Every prompt queued for any agent that is a done wake.
    fn done_wakes(st: &AppState) -> Vec<String> {
        st.rt.lock().unwrap().queued.values().flatten().filter(|p| p.contains("This is not a message from the user. The Delivery of")).cloned().collect()
    }

    #[tokio::test]
    async fn the_final_pass_marks_the_delivery_done_once_even_with_conflicts_and_wakes_the_planner() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/done-flow";
        let (dev, team) = building_both(&st, &fake, "done-flow", &["code_reviewer"]).await;
        let cr = &team[0];
        let planner = planner_of(&st, slug);
        let spec = spec_issue(&st, &fake, "done-flow", slug);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        let draft = json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true });
        // Ready, but with conflicts and checks still running: GitHub won't call it mergeable.
        let conflicted = json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": false, "mergeable": false, "mergeable_state": "dirty", "title": "Rounds", "html_url": format!("https://github.com/{slug}/pull/500"), "head": { "ref": "spec/40-rounds", "sha": "feed1234" }, "base": { "ref": "main" }, "user": { "login": "octo-me" } });
        fake.in_turn("GET", &format!("/repos/{slug}/pulls/500"), vec![(200, draft.clone()), (200, draft), (200, conflicted)]);
        fake.in_turn("POST", "/graphql", vec![(200, json!({ "errors": [{ "message": "Something went wrong" }] })), (200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }))]);

        // The pass whose ready call fails: not done yet.
        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "Clean." })).await.unwrap();
        assert!(out.ends_with("Loft tries again on its next poll."), "{out}");
        assert!(done_items(&st).is_empty() && woken(&st, &planner).is_empty());
        assert_eq!(delivery(&st, slug).done_at, "");

        // The mirror's retry marks it ready: done, once.
        retry_ready(&st, slug, false).await;
        assert_eq!(fake.seen("/graphql").len(), 2);
        let d = delivery(&st, slug);
        assert!(d.state == "ready" && !d.done_at.is_empty(), "{d:?}");
        let items = done_items(&st);
        assert_eq!(items.len(), 1, "{items:#?}");
        let item = &items[0];
        assert_eq!((&item["title"], &item["reason"], &item["urgency"]), (&json!("Delivery done: Rounds"), &json!("Every ticket is closed and its reviews passed: spec PR #500 is ready for review."), &json!(1)));
        let acts = item["actions"].as_array().unwrap();
        assert_eq!(acts.iter().map(|a| a["label"].clone()).collect::<Vec<_>>(), [json!("Open issue"), json!("Open PR"), json!("Dismiss")]);
        assert_eq!(acts[1]["target"], json!({ "url": format!("https://github.com/{slug}/pull/500") }));
        let v = crate::needs_you::snapshot(&st).unwrap();
        assert!(!v["items"].as_array().unwrap().iter().any(|i| i["type"] == "pr_ready"), "pr_ready waits for GitHub, the done item doesn't");
        // Pushed under delivery_done with names and titles only, opening the spec's issue; on by default.
        let all = crate::needs_you::collect(&st).unwrap();
        let p = crate::needs_you::to_push(all.iter().find(|i| i.kind == "delivery_done").unwrap());
        assert_eq!((p.kind, p.title.as_str(), p.body.as_str()), ("delivery_done", "Delivery done", "Rounds"));
        assert_eq!(p.target, json!({ "screen": "task", "task_id": spec.id }));
        assert!(crate::apns::DEFAULT_KINDS.contains(&"delivery_done") && crate::apns::KINDS.contains(&"delivery_done"));

        // The planner, woken once with the summary, in a run whose reply is an Unread reply.
        let w = woken(&st, &planner);
        assert_eq!(w.len(), 1, "{w:?}");
        for want in [
            "[Loft Delivery: spec #40] This is not a message from the user. The Delivery of spec #40 \"Rounds\" is done.\n",
            "- Loft marked its spec PR https://github.com/octo/done-flow/pull/500 ready for review: the user merges it.",
            "- Tickets closed: #41 \"Store\", #42 \"API\".",
        ] {
            assert!(w[0].contains(want), "missing {want:?} in {}", w[0]);
        }
        assert!(!w[0].contains("Tech debt"), "{}", w[0]);
        assert!(!w[0].contains("team's Lead"));
        assert!(st.rt.lock().unwrap().queued_for_user.contains(&planner.id), "its reply is an Unread reply");

        // Retries change nothing: the mirror, a late pass, the done call itself.
        retry_ready(&st, slug, true).await;
        assert!(tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "Again." })).await.is_err());
        crate::delivery::done(&st, slug, 40);
        assert_eq!((woken(&st, &planner).len(), done_items(&st).len(), done_wakes(&st).len()), (1, 1, 1));

        // It goes when the PR merges.
        let mut pr = st.db.list_pull_requests(None).unwrap().into_iter().find(|p| p.slug == slug && p.number == 500).expect("tracked");
        pr.state = "merged".into();
        st.db.save_pull_request(&pr).unwrap();
        assert!(done_items(&st).is_empty());
    }

    #[tokio::test]
    async fn a_final_pass_whose_ready_call_failed_waits_on_the_retry_not_stalled() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/done-retry";
        let (dev, team) = building_both(&st, &fake, "done-retry", &["code_reviewer"]).await;
        let cr = &team[0];
        spec_issue(&st, &fake, "done-retry", slug);
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        let draft = json!({ "number": 500, "node_id": "PR_500", "state": "open", "draft": true });
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, draft);
        fake.in_turn("POST", "/graphql", vec![(200, json!({ "errors": [{ "message": "Something went wrong" }] })), (200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }))]);

        // The Code reviewer's run takes the final review wake, passes, and the ready call fails.
        run_starts(&st, cr);
        let out = tool(&st, cr, "review_ticket", json!({ "number": 500, "verdict": "pass", "notes": "Clean." })).await.unwrap();
        assert!(out.ends_with("Loft tries again on its next poll."), "{out}");
        let d = delivery(&st, slug);
        assert!(d.state == "final_review" && d.ready_retry, "{d:?}");

        // Its run ends: the step was done, only Loft's retry is left. No stall.
        crate::delivery::run_ended(&st, &cr.id, false).await;
        let d = delivery(&st, slug);
        assert!(d.state == "final_review" && d.stall_reason.is_empty(), "{d:?}");
        assert!(stalled_items(&st).is_empty());

        // Nor does a restart stall it.
        crate::delivery::stall_lost_runs(&st);
        assert_eq!(delivery(&st, slug).state, "final_review");

        // The retry marks it ready, and it's done once.
        retry_ready(&st, slug, false).await;
        let d = delivery(&st, slug);
        assert!(d.state == "ready" && !d.ready_retry && !d.done_at.is_empty(), "{d:?}");
        assert_eq!(done_items(&st).len(), 1);
        assert!(stalled_items(&st).is_empty());
    }

    #[tokio::test]
    async fn the_done_item_can_be_dismissed() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/done-dismiss";
        let (dev, _) = building_both(&st, &fake, "done-dismiss", &[]).await;
        tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
        tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
        let id = done_items(&st)[0]["id"].as_str().unwrap().to_string();
        assert_eq!(id, format!("done:{slug}:40"));
        let _ = crate::needs_you::dismiss_item(State(st.clone()), Path(id)).await.unwrap();
        assert!(done_items(&st).is_empty());
    }

    #[tokio::test]
    async fn with_the_planner_gone_the_teams_lead_is_woken_and_with_neither_nobody() {
        for (name, lead) in [("done-lead", true), ("done-nobody", false)] {
            let st = state();
            let fake = own_fake(&st);
            let slug = format!("octo/{name}");
            // No reviewers: each ticket passes on submit, and the last one marks the spec PR ready.
            let (dev, _) = building_both(&st, &fake, name, &[]).await;
            st.db.delete_agent(&delivery(&st, &slug).planner).unwrap();
            tool(&st, &dev, "submit_ticket", submit_41()).await.unwrap();
            let lead = lead.then(|| busy(&st, "Lead", name, "lead"));
            let out = tool(&st, &dev, "submit_ticket", json!({ "number": 42, "commits": ["0123abcd"] })).await.unwrap();
            assert!(out.contains("spec PR #500 is now ready for review"), "{out}");
            assert_eq!(done_items(&st).len(), 1, "{name}: the user is told either way");
            let wakes = done_wakes(&st);
            match lead {
                Some(l) => {
                    let w = woken(&st, &l);
                    assert!(wakes.len() == 1 && w.len() == 1 && w[0].contains("is done. You get this as the team's Lead: the agent that planned it is gone.") && !w[0].contains("Tech debt"), "{w:?}");
                    assert!(st.rt.lock().unwrap().queued_for_user.contains(&l.id));
                }
                None => assert!(wakes.is_empty(), "{wakes:?}"),
            }
        }
    }
}
