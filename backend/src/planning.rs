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

/// The roles a team needs to implement a plan, as stored in `Agent::role`.
pub const ROLES: &[&str] = &["dev", "reviewer"];

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
        "description": "Publish the spec as a GitHub issue on your project's repo, labelled `spec` and assigned to the user. Loft creates it with the user's token and creates missing labels. When the project has no GitHub repo (no git, no GitHub `origin`, or GitHub not connected), it is saved as a Loft issue labelled `spec` instead, and nothing goes to GitHub. On GitHub it also creates the spec branch `spec/<N>-<slug>`, from `docs_branch` when given or else with an empty \"Start spec #N\" commit on the default branch, and opens a draft spec PR from it to the default branch, with `Closes #<spec>`; if that fails the spec stays published and the result says why. Calling it again with the same title returns the spec already published instead of creating another and only makes the missing branch or PR, so a retry is safe.",
        "inputSchema": { "type": "object", "properties": {
            "title": { "type": "string", "description": "The spec's title: what gets built, in a few words." },
            "body": { "type": "string", "description": "The spec in Markdown, following the spec template." },
            "docs_branch": { "type": "string", "description": "In a grill that committed glossary or ADR changes: the docs branch, pushed to GitHub first. The spec branch starts from it, so the spec PR carries the docs and no Docs PR is opened. Leave it out when there are no docs commits." }
        }, "required": ["title", "body"] }
    }), json!({
        "name": "publish_tickets",
        "description": "Publish tickets as GitHub issues on your project's repo, only after the user approved the breakdown. Loft creates them blockers first as sub-issues of the spec, labelled `ready-for-agent` and assigned to the user, and links each blocker as a GitHub issue dependency. Under a spec with a spec PR, Loft adds a \"## Branch\" section to each ticket naming the spec branch and the spec PR. When the project has no GitHub repo they become Loft issues under the spec instead, with the blockers as dependencies, and nothing goes to GitHub. Without `spec` it publishes one standalone ticket. Calling it again with the same tickets skips what is already published, so a retry after a failure is safe.",
        "inputSchema": { "type": "object", "properties": {
            "spec": { "type": ["integer", "string"], "description": "The spec from publish_spec: its GitHub issue number, or its Loft issue id when the plan is kept in Loft. Leave it out only for a single standalone ticket." },
            "tickets": { "type": "array", "description": "The tickets, in any order.", "items": { "type": "object", "properties": {
                "title": { "type": "string", "description": "What the slice delivers, in a few words." },
                "body": { "type": "string", "description": "The ticket in Markdown, following the ticket template." },
                "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "Titles of the tickets in this list that must land before this one." }
            }, "required": ["title", "body"] } }
        }, "required": ["tickets"] }
    }), json!({
        "name": "close_ticket",
        "description": "Close a ticket on your project's GitHub repo once its work has passed: Loft comments \"Done in `<sha…>`, part of #<spec PR>\" on it, closes it as completed so the tickets it blocks can start, and marks its Loft issue done. Only for open tickets labelled `ready-for-agent`.",
        "inputSchema": { "type": "object", "properties": {
            "number": { "type": "integer", "description": "The ticket's GitHub issue number." },
            "commits": { "type": "array", "items": { "type": "string" }, "description": "The SHAs of the ticket's commits, oldest first." }
        }, "required": ["number", "commits"] }
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
    if name == "close_ticket" {
        return Some(close_ticket(state, me, args).await);
    }
    if name != "publish_spec" && name != "publish_tickets" {
        return None;
    }
    let out = if name == "publish_tickets" {
        publish_tickets(state, me, args).await
    } else {
        let docs = args["docs_branch"].as_str().map(str::trim).filter(|d| !d.is_empty());
        publish_spec(state, me, args["title"].as_str().unwrap_or(""), args["body"].as_str().unwrap_or(""), docs).await.map(|(msg, slug, spec)| {
            crate::grill::record_published(state, &me.id, &slug, Some(spec), vec![]);
            msg
        })
    };
    // Even a failure can leave issues published, so clients hear about it either way.
    state.emit(crate::state::Event::Issues { project: me.project.trim().to_string() });
    // What was published shows up as Loft issues without waiting for the next poll.
    crate::mirror::spawn_sync(state, &me.project);
    Some(out)
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

/// The team check: the roles no agent in `project` fills, going by their stored roles.
pub fn missing_roles(state: &AppState, project: &str) -> Vec<&'static str> {
    let project = project.trim();
    let team: Vec<String> = state.db.list_agents().unwrap_or_default().into_iter().filter(|a| !project.is_empty() && a.project.trim() == project).map(|a| a.role).collect();
    ROLES.iter().copied().filter(|r| !team.iter().any(|t| t == r)).collect()
}

/// A role for an agent saved before roles existed, from the words in its name and title. Reviewing
/// words win, so a "QA engineer" reviews.
pub fn infer_role(name: &str, title: &str) -> &'static str {
    let text = format!("{name} {title}").to_lowercase();
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    if has(&["reviewer", "review", "revisor", "revisora", "qa", "tester", "auditor", "security"]) {
        "reviewer"
    } else if has(&["dev", "developer", "engineer", "programmer", "coder", "desenvolvedor", "desenvolvedora"]) {
        "dev"
    } else {
        ""
    }
}

/// One lock per repo, held for a whole publish: the check for what is already published and the
/// create are several awaits apart, so two overlapping calls (two planners on one project, parallel
/// tool calls, a retry while a slow run is still going) would otherwise both create the same issue.
#[derive(Default)]
pub struct Locks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl Locks {
    fn of(&self, slug: &str) -> Arc<tokio::sync::Mutex<()>> {
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
fn key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Where a plan goes: `owner/repo` from the `origin` of the agent's project folder, with GitHub
/// connected. Err says why it stays in Loft instead: no project, no git, no GitHub remote, no token.
async fn github_home(state: &AppState, a: &Agent) -> Result<String, String> {
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
        Ok((msg, String::new(), IssueLink { number: 0, title: t.title.clone(), id: t.id.clone() }))
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
        let siblings: Vec<Task> = in_project(state, me)
            .into_iter()
            .filter(|t| t.labels.iter().any(|l| l == "ready-for-agent"))
            .filter(|t| match &spec {
                Some(s) => t.parent_id.as_deref() == Some(&s.id),
                None => t.parent_id.is_none() && t.state() == "open",
            })
            .collect();
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
                        "title": t.title, "description": t.body, "labels": ["ready-for-agent"], "project": project, "status": "ready",
                        "parent_id": spec.as_ref().map(|s| s.id.clone()), "depends_on": blockers,
                    }))?;
                    fresh += 1;
                    (made, true)
                }
            };
            let spec_link = spec.as_ref().map(|s| IssueLink { number: 0, title: s.title.clone(), id: s.id.clone() });
            crate::grill::record_published(state, &me.id, "", spec_link, vec![IssueLink { number: pos as i64 + 1, title: t.title.clone(), id: made.id.clone() }]);
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
async fn login(state: &AppState) -> Result<String, String> {
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
    let pr = match spec_pr(state, &slug, n, &title, docs).await {
        Ok(line) => line,
        Err(e) => {
            let same = if docs.is_some() { " and `docs_branch`" } else { "" };
            format!("The spec is published, but {e}. Call publish_spec again with the same title{same} to retry only the spec branch and PR.")
        }
    };
    Ok((format!("{msg}\n{pr}"), slug, IssueLink { number: n, title, id: String::new() }))
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
fn spec_branch(n: i64, title: &str) -> String {
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
    if slug.is_empty() { format!("spec/{n}") } else { format!("spec/{n}-{slug}") }
}

/// A branch name as git takes it: letters, digits and `-_./`, no `..`, not starting or ending with `/`.
fn is_branch_name(b: &str) -> bool {
    !b.is_empty() && b.len() <= 200 && b.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) && !b.contains("..") && !b.starts_with('/') && !b.ends_with('/') && !b.ends_with(".lock")
}

/// Makes sure spec #n has its spec branch and a draft spec PR to the default branch, all through the
/// GitHub API. The branch starts at the head of the grill's docs branch when there is one, so the
/// docs commits ship in the spec PR; otherwise at an empty commit on the default branch. Whatever is already there (recorded, or found on GitHub) is kept, so a retry only makes
/// what's missing. The PR is recorded under the spec's number, for its tickets to find. Ok is the
/// reply's line naming both; Err says what failed.
async fn spec_pr(state: &AppState, slug: &str, n: i64, title: &str, docs: Option<&str>) -> Result<String, String> {
    let branch = spec_branch(n, title);
    let line = |pr: i64| format!("Spec branch: `{branch}`. Spec PR (draft, to the default branch): https://github.com/{slug}/pull/{pr}");
    if let Ok(Some(pr)) = state.db.published_issue(slug, "spec_pr", &n.to_string()) {
        return Ok(line(pr));
    }
    let gh = |path: String| async move { crate::github::get(state, &format!("/repos/{slug}{path}")).await };
    let post = |path: &'static str, body: Value| async move { crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}{path}"), Some(body)).await };
    let repo = gh(String::new()).await.map_err(|e| format!("couldn't read {slug} for its default branch: {e}"))?;
    let base = repo["default_branch"].as_str().filter(|b| !b.is_empty()).ok_or_else(|| format!("GitHub didn't give the default branch of {slug}"))?.to_string();
    match gh(format!("/git/ref/heads/{branch}")).await {
        Ok(_) => {}
        Err(e) if crate::github::is_gone(&e) => {
            let failed = |e: anyhow::Error| format!("the spec branch `{branch}` couldn't be created: {e}");
            let start = match docs {
                Some(d) => {
                    let head = gh(format!("/git/ref/heads/{d}")).await.map_err(|e| {
                        if crate::github::is_gone(&e) { format!("the docs branch `{d}` isn't on GitHub, so the spec branch `{branch}` wasn't created: push it first") } else { failed(e) }
                    })?;
                    head["object"]["sha"].clone()
                }
                None => {
                    let head = gh(format!("/git/ref/heads/{base}")).await.map_err(failed)?;
                    let sha = head["object"]["sha"].as_str().unwrap_or_default().to_string();
                    let commit = gh(format!("/git/commits/{sha}")).await.map_err(failed)?;
                    // An empty commit: the tree of the default branch, so the branch has something to open a PR for.
                    let made = post("/git/commits", json!({ "message": format!("Start spec #{n}"), "tree": commit["tree"]["sha"], "parents": [sha] })).await.map_err(failed)?;
                    made["sha"].clone()
                }
            };
            post("/git/refs", json!({ "ref": format!("refs/heads/{branch}"), "sha": start })).await.map_err(failed)?;
        }
        Err(e) => return Err(format!("couldn't check {slug} for the spec branch `{branch}`: {e}")),
    }
    // Opened before but never recorded: adopt it rather than open a second one.
    let owner = slug.split('/').next().unwrap_or_default();
    let open = gh(format!("/pulls?head={owner}:{branch}&state=all")).await.map_err(|e| format!("couldn't check {slug} for the spec PR: {e}"))?;
    let pr = match open.as_array().and_then(|ps| ps.first()).and_then(|p| p["number"].as_i64()) {
        Some(pr) => pr,
        None => {
            let made = post("/pulls", json!({ "title": title, "head": branch, "base": base, "body": format!("Closes #{n}"), "draft": true }))
                .await
                .map_err(|e| format!("the draft spec PR from `{branch}` couldn't be opened: {e}"))?;
            made["number"].as_i64().ok_or("GitHub opened the spec PR but didn't return its number")?
        }
    };
    let _ = state.db.record_published_issue(slug, "spec_pr", &n.to_string(), pr);
    Ok(line(pr))
}

/// The "## Branch" section of a ticket under spec #spec, when the spec has a spec PR: where to commit
/// and where it ships. None for a spec without one (published before spec PRs, or its PR failed).
async fn branch_section(state: &AppState, slug: &str, spec: i64) -> Result<Option<String>, String> {
    let Ok(Some(pr)) = state.db.published_issue(slug, "spec_pr", &spec.to_string()) else {
        return Ok(None);
    };
    let p = crate::github::get(state, &format!("/repos/{slug}/pulls/{pr}")).await.map_err(|e| format!("Couldn't read spec PR #{pr} on {slug} for its branch: {e}"))?;
    let branch = p["head"]["ref"].as_str().ok_or_else(|| format!("GitHub didn't give the branch of spec PR #{pr} on {slug}"))?;
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

/// `close_ticket`: the passed ticket's closing comment, its close as completed, and its Loft issue done.
async fn close_ticket(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    let n = args["number"].as_i64().filter(|n| *n > 0).ok_or("number is required: the ticket's GitHub issue number")?;
    let commits: Vec<String> = args["commits"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).map(|c| c.trim().to_lowercase()).collect();
    if commits.is_empty() {
        return Err("commits is required: the SHAs of the ticket's commits".into());
    }
    if let Some(bad) = commits.iter().find(|c| !(7..=40).contains(&c.len()) || !c.chars().all(|ch| ch.is_ascii_hexdigit())) {
        return Err(format!("\"{bad}\" isn't a commit SHA: give 7 to 40 hex characters"));
    }
    let slug = github_home(state, me).await.map_err(|why| format!("close_ticket closes tickets on GitHub, and {why}: mark the ticket's Loft issue done instead."))?;
    let _held = state.publish_locks.of(&slug).lock_owned().await;
    let i = crate::github::get(state, &format!("/repos/{slug}/issues/{n}")).await.map_err(|e| format!("Couldn't read #{n} on {slug}: {e}"))?;
    let is_ticket = i.get("pull_request").is_none() && i["labels"].as_array().into_iter().flatten().any(|l| l["name"] == "ready-for-agent");
    if !is_ticket {
        return Err(format!("#{n} on {slug} isn't a ticket (an issue labelled `ready-for-agent`), so it wasn't closed."));
    }
    if i["state"] != "open" {
        return Err(format!("#{n} on {slug} is already closed: nothing was done."));
    }
    let pr = spec_pr_in(i["body"].as_str().unwrap_or(""), &slug);
    let shas: Vec<String> = commits.iter().map(|c| format!("`{}`", &c[..7])).collect();
    let part = pr.map(|p| format!(", part of #{p}")).unwrap_or_default();
    let comment = format!("Done in {}{part}.", shas.join(", "));
    // Commented before but the close failed: the retry doesn't comment twice.
    if !matches!(state.db.published_issue(&slug, "close_comment", &n.to_string()), Ok(Some(_))) {
        crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{n}/comments"), Some(json!({ "body": comment })))
            .await
            .map_err(|e| format!("Couldn't comment on #{n}, so it wasn't closed: {e}"))?;
        let _ = state.db.record_published_issue(&slug, "close_comment", &n.to_string(), n);
    }
    let closed = crate::github::request(state, reqwest::Method::PATCH, &format!("/repos/{slug}/issues/{n}"), Some(json!({ "state": "closed", "state_reason": "completed" })))
        .await
        .map_err(|e| format!("Commented on #{n} but couldn't close it: {e}. Call close_ticket again to retry the close."))?;
    // The mirror leaves an issue someone is working on open when GitHub closes it; this close is the work's end.
    if let Some((mut t, _)) = crate::mirror::bring_in(state, me.project.trim(), &slug, &closed)
        && t.status != "done"
    {
        t.status = "done".into();
        t.completed_at = Some(crate::db::now());
        t.updated_at = crate::db::now();
        if state.db.save_task(&t).is_ok() {
            crate::tasks::emit(state, &t);
            crate::tasks::log_status(state, &t, &me.id, &format!("Closed by {} with close_ticket", me.name));
        }
    }
    state.emit(crate::state::Event::Issues { project: me.project.trim().to_string() });
    let ready = match ready_when_last(state, &slug, i["body"].as_str().unwrap_or("")).await {
        Ok(Some(pr)) => format!("\nIt was the spec's last open ticket: spec PR #{pr} is now ready for review: https://github.com/{slug}/pull/{pr}"),
        Ok(None) => String::new(),
        Err(e) => format!("\nThe ticket is closed, but {e}."),
    };
    Ok(format!("Closed #{n} as completed with the comment \"{comment}\": https://github.com/{slug}/issues/{n}{ready}"))
}

/// After a ticket closed: when its spec PR (from the ticket's "## Branch" section) is still an open
/// draft and every ticket of its spec (the `Closes #N` in the PR) is closed, marks the PR ready for
/// review. Some(PR) when it did; None when there's nothing to do.
pub(crate) async fn ready_when_last(state: &AppState, slug: &str, ticket_body: &str) -> Result<Option<i64>, String> {
    let Some(pr) = spec_pr_in(ticket_body, slug) else { return Ok(None) };
    let p = crate::github::get(state, &format!("/repos/{slug}/pulls/{pr}")).await.map_err(|e| format!("couldn't read spec PR #{pr}: {e}"))?;
    if p["state"] != "open" || p["draft"] != true {
        return Ok(None);
    }
    let body = p["body"].as_str().unwrap_or("").to_lowercase();
    let Some(spec) = body.split("closes #").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()?.parse::<i64>().ok()) else {
        return Ok(None);
    };
    let subs = crate::github::get(state, &format!("/repos/{slug}/issues/{spec}/sub_issues?per_page=100")).await.map_err(|e| format!("couldn't read the tickets of spec #{spec}: {e}"))?;
    let subs = subs.as_array().cloned().unwrap_or_default();
    if subs.is_empty() || subs.iter().any(|s| s["state"] != "closed") {
        return Ok(None);
    }
    let query = "mutation($id: ID!) { markPullRequestReadyForReview(input: { pullRequestId: $id }) { pullRequest { isDraft } } }";
    let v = crate::github::request(state, reqwest::Method::POST, "/graphql", Some(json!({ "query": query, "variables": { "id": p["node_id"] } })))
        .await
        .map_err(|e| format!("couldn't mark spec PR #{pr} ready for review: {e}"))?;
    if let Some(e) = v["errors"].as_array().and_then(|es| es.first()) {
        return Err(format!("couldn't mark spec PR #{pr} ready for review: {}", e["message"].as_str().unwrap_or("GitHub refused")));
    }
    Ok(Some(pr))
}

struct Ticket {
    title: String,
    body: String,
    /// Positions in the list of the tickets that block this one.
    blocked_by: Vec<usize>,
}

fn tickets_in(args: &Value) -> Result<Vec<Ticket>, String> {
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
        let mut blocked_by = vec![];
        for b in t["blocked_by"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            match keys.iter().position(|k| *k == key(b)) {
                Some(j) if j == i => return Err(format!("\"{title}\" can't block itself")),
                Some(j) if !blocked_by.contains(&j) => blocked_by.push(j),
                Some(_) => {}
                None => return Err(format!("\"{title}\" is blocked by \"{b}\", which isn't one of these tickets")),
            }
        }
        out.push(Ticket { title, body, blocked_by });
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

/// An issue on GitHub as publishing needs it: number, database id (what links take), title key.
fn issue_ref(i: &Value) -> Option<(i64, i64, String)> {
    Some((i["number"].as_i64()?, i["id"].as_i64()?, key(i["title"].as_str()?)))
}

/// The database id of issue #n, which sub-issue and dependency links take instead of its number.
async fn issue_id(state: &AppState, slug: &str, n: i64, known: Option<i64>) -> Result<i64, String> {
    if let Some(id) = known {
        return Ok(id);
    }
    let v = crate::github::get(state, &format!("/repos/{slug}/issues/{n}")).await.map_err(|e| format!("Couldn't read #{n} on {slug}: {e}."))?;
    v["id"].as_i64().ok_or_else(|| format!("GitHub didn't return the id of #{n} on {slug}."))
}

/// Publishes the tickets, noting each one on the running grill as soon as it's on GitHub, so the
/// end-of-planning card lists it even if a later step fails and the grill ends without a retry.
async fn publish_tickets(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    let ts = tickets_in(args)?;
    let given = !args["spec"].is_null();
    if !given && ts.len() > 1 {
        return Err("Several tickets belong to a spec: publish it with `publish_spec` first and pass it as `spec`. Only a single ticket goes without one.".into());
    }
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
    let _held = state.publish_locks.of(&slug).lock_owned().await;
    let login = login(state).await?;
    ensure_labels(state, &slug).await?;
    // What GitHub already has: the spec's sub-issues, or the open tickets for a standalone one.
    // A ticket created before but never recorded (the reply was lost) is adopted, not published twice.
    let listed = match spec {
        Some(s) => crate::github::get(state, &format!("/repos/{slug}/issues/{s}/sub_issues?per_page=100")).await.map_err(|e| format!("Couldn't read spec #{s} on {slug}: {e}"))?,
        None => crate::github::get(state, &format!("/repos/{slug}/issues?labels=ready-for-agent&state=open&per_page=100"))
            .await
            .map_err(|e| format!("Couldn't check {slug} for an existing ticket: {e}"))?,
    };
    let listed: Vec<(i64, i64, String)> = listed.as_array().into_iter().flatten().filter(|i| i.get("pull_request").is_none()).filter_map(issue_ref).collect();
    let branch = match spec {
        Some(s) => branch_section(state, &slug, s).await?,
        None => None,
    };
    let scope = spec.map_or("standalone".to_string(), |s| format!("#{s}"));
    let retry = "What was published so far stays: call publish_tickets again with the same tickets to finish without duplicates.";
    let mut made: Vec<Option<(i64, Option<i64>)>> = vec![None; ts.len()];
    // GitHub's answer for each ticket created now, to bring it in without reading it again.
    let mut created: HashMap<i64, Value> = HashMap::new();
    let mut lines = vec![];
    let mut fresh = 0;
    for &i in &order {
        let t = &ts[i];
        let k = format!("{scope}/{}", key(&t.title));
        // A standalone ticket counts as published only while it's still open: once closed, the same
        // request is new work. The database id is only read from GitHub when a link still needs it.
        let recorded = state.db.published_issue(&slug, "ticket", &k).ok().flatten().filter(|n| spec.is_some() || listed.iter().any(|l| l.0 == *n));
        let (n, mut id, new) = if let Some(n) = recorded {
            (n, listed.iter().find(|l| l.0 == n).map(|l| l.1), false)
        } else if let Some(l) = listed.iter().find(|l| l.2 == key(&t.title)) {
            state.db.record_published_issue(&slug, "ticket", &k, l.0).map_err(|e| e.to_string())?;
            (l.0, Some(l.1), false)
        } else {
            let v = crate::github::request(
                state,
                reqwest::Method::POST,
                &format!("/repos/{slug}/issues"),
                Some(json!({ "title": t.title, "body": branch.as_deref().map_or(t.body.clone(), |b| with_branch(&t.body, b)), "labels": ["ready-for-agent"], "assignees": [login] })),
            )
            .await
            .map_err(|e| format!("GitHub rejected the ticket \"{}\" on {slug}: {e}. {retry}", t.title))?;
            let (Some(n), Some(id)) = (v["number"].as_i64(), v["id"].as_i64()) else {
                return Err(format!("GitHub created the ticket \"{}\" but didn't return its number. {retry}", t.title));
            };
            state.db.record_published_issue(&slug, "ticket", &k, n).map_err(|e| e.to_string())?;
            fresh += 1;
            created.insert(n, v);
            (n, Some(id), true)
        };
        let spec_link = spec.map(|number| IssueLink { number, title: String::new(), id: String::new() });
        crate::grill::record_published(state, &me.id, &slug, spec_link, vec![IssueLink { number: n, title: t.title.clone(), id: String::new() }]);
        if let Some(s) = spec {
            let lk = format!("{s}/{n}");
            if !matches!(state.db.published_issue(&slug, "sub_issue", &lk), Ok(Some(_))) {
                if !listed.iter().any(|l| l.0 == n) {
                    let id = *id.insert(issue_id(state, &slug, n, id).await.map_err(|e| format!("{e} {retry}"))?);
                    crate::github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/issues/{s}/sub_issues"), Some(json!({ "sub_issue_id": id })))
                        .await
                        .map_err(|e| format!("Couldn't make #{n} a sub-issue of spec #{s}: {e}. {retry}"))?;
                }
                state.db.record_published_issue(&slug, "sub_issue", &lk, n).map_err(|e| e.to_string())?;
            }
        }
        let mut blockers = vec![];
        for &b in &t.blocked_by {
            // Blockers go first, so every one of them is on GitHub by now.
            let (bn, bid) = made[b].expect("blockers are published first");
            blockers.push(format!("#{bn}"));
            let dk = format!("{n}/{bn}");
            if matches!(state.db.published_issue(&slug, "blocked_by", &dk), Ok(Some(_))) {
                continue;
            }
            let bid = issue_id(state, &slug, bn, bid).await.map_err(|e| format!("{e} {retry}"))?;
            made[b] = Some((bn, Some(bid)));
            let path = format!("/repos/{slug}/issues/{n}/dependencies/blocked_by");
            if let Err(e) = crate::github::request(state, reqwest::Method::POST, &path, Some(json!({ "issue_id": bid }))).await {
                // Linked before but never recorded: GitHub turns the second link down, which is fine.
                let have = crate::github::get(state, &format!("{path}?per_page=100")).await.unwrap_or(Value::Null);
                if !have.as_array().into_iter().flatten().any(|d| d["number"].as_i64() == Some(bn)) {
                    return Err(format!("Couldn't mark #{n} as blocked by #{bn}: {e}. {retry}"));
                }
            }
            state.db.record_published_issue(&slug, "blocked_by", &dk, bn).map_err(|e| e.to_string())?;
        }
        made[i] = Some((n, id));
        let by = if blockers.is_empty() { String::new() } else { format!(", blocked by {}", blockers.join(", ")) };
        let was = if new { "" } else { ", already published" };
        lines.push(format!("- #{n} {}{by}{was}: https://github.com/{slug}/issues/{n}", t.title));
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

/// The published tickets as Loft issues right away, as in a project without a repo: linked to their
/// GitHub issues, ready and unassigned, under the spec's Loft issue, depending on their blockers'. A
/// ticket that already has its Loft issue (a retry) is left as it is.
#[allow(clippy::too_many_arguments)]
async fn loft_issues_for(state: &AppState, me: &Agent, slug: &str, spec: Option<i64>, ts: &[Ticket], order: &[usize], made: &[Option<(i64, Option<i64>)>], created: &HashMap<i64, Value>) {
    let project = me.project.trim();
    let bring = |n: i64| async move {
        let i = match created.get(&n) {
            Some(i) => i.clone(),
            None => crate::github::get(state, &format!("/repos/{slug}/issues/{n}")).await.ok()?,
        };
        crate::mirror::bring_in(state, project, slug, &i)
    };
    let parent = match spec {
        Some(s) => bring(s).await.map(|(t, _)| t.id),
        None => None,
    };
    let mut ids: HashMap<usize, String> = HashMap::new();
    for &i in order {
        let Some((n, _)) = made[i] else { continue };
        let Some((mut t, new)) = bring(n).await else { continue };
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
    fn planner_on(st: &AppState, name: &str, origin: Option<&str>) -> Agent {
        let mut a = planner(st, name);
        a.project = name.into();
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
        let ev = serde_json::to_value(events.try_recv().unwrap()).unwrap();
        assert_eq!(ev, json!({ "type": "issues", "project": "spec-new" }));

        let labels = posts(&fake, "octo/spec-new", "labels");
        assert_eq!(labels.len(), 1, "only the missing label is created: {labels:?}");
        assert_eq!(labels[0]["name"], "spec");
        assert_eq!(posts(&fake, "octo/spec-new", "issues"), [json!({
            "title": "Planning in rounds", "body": "## Problem Statement\n\nIt hurts.", "labels": ["spec"], "assignees": ["octo-me"],
        })]);
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

    async fn close(st: &AppState, a: &Agent, n: i64, commits: &[&str]) -> Result<String, String> {
        mcp_call(st, a, "close_ticket", &json!({ "number": n, "commits": commits })).await.unwrap()
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
    fn spec_with_tickets(fake: &Fake, slug: &str) -> String {
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

    fn subs(fake: &Fake, slug: &str, states: &[&str]) {
        let list: Vec<Value> = states.iter().enumerate().map(|(i, s)| json!({ "number": 41 + i as i64, "state": s })).collect();
        fake.route("GET", &format!("/repos/{slug}/issues/40/sub_issues"), 200, json!(list));
    }

    #[tokio::test]
    async fn closing_the_last_ticket_marks_the_spec_pr_ready_and_not_before() {
        let st = state();
        let fake = own_fake(&st);
        let slug = "octo/ready-last";
        let a = planner_on(&st, "ready-last", Some("git@github.com:octo/ready-last.git"));
        spec_with_tickets(&fake, slug);

        subs(&fake, slug, &["closed", "open"]);
        let out = close(&st, &a, 41, &["abc1234"]).await.unwrap();
        assert!(!out.contains("ready for review"), "{out}");
        assert!(fake.seen("/graphql").is_empty(), "#42 is still open");

        subs(&fake, slug, &["closed", "closed"]);
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
        spec_with_tickets(&fake, slug);
        subs(&fake, slug, &["closed", "closed"]);
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
        spec_with_tickets(&fake, slug);
        subs(&fake, slug, &["closed", "closed"]);

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
        let body = spec_with_tickets(&fake, slug);
        subs(&fake, slug, &["closed", "closed"]);
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
        crate::mirror::sync_project(&st, "ready-hand", false).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 1, "the last ticket closed: the PR is ready");
        assert_eq!(fake.seen("/graphql")[0].body["variables"], json!({ "id": "PR_500" }));
        // Seeing it closed again isn't a new close.
        crate::mirror::sync_project(&st, "ready-hand", false).await.unwrap();
        assert_eq!(fake.seen("/graphql").len(), 1);
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
        assert_eq!(close(&st, &a, 1, &[]).await.unwrap_err(), "commits is required: the SHAs of the ticket's commits");
        assert_eq!(close(&st, &a, 1, &["main"]).await.unwrap_err(), "\"main\" isn't a commit SHA: give 7 to 40 hex characters");
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
        assert_eq!(err, "Commented on #5 but couldn't close it: GitHub 502: Server Error. Call close_ticket again to retry the close.");
        assert!(close(&st, &a, 5, &["abc1234"]).await.unwrap().starts_with("Closed #5 as completed with the comment \"Done in `abc1234`.\""));
        assert_eq!(posts(&fake, slug, "issues/5/comments").len(), 1);
    }

    #[tokio::test]
    async fn close_ticket_without_a_github_repo_points_at_the_loft_issue() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "close-loft", None);
        let err = close(&st, &a, 1, &["abc1234"]).await.unwrap_err();
        assert!(err.starts_with("close_ticket closes tickets on GitHub, and ") && err.ends_with(": mark the ticket's Loft issue done instead."), "{err}");
        assert!(fake.seen("/repos/").iter().all(|s| !s.path.contains("close-loft")));
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
        assert_eq!(card["spec"], json!({ "number": 0, "title": "Offline mode", "url": format!("/tasks/{}", spec.id) }));
        assert_eq!(card["tickets"].as_array().unwrap().iter().map(|t| t["url"].as_str().unwrap().to_string()).collect::<Vec<_>>()[..3], [format!("/tasks/{}", store.id), format!("/tasks/{}", ui.id), format!("/tasks/{}", sync.id)]);
        assert_eq!(card["next_message"], "Implement the plan: hand its ready Loft issues to the team");
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
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 200, json!({ "number": 500, "head": { "ref": "spec/40-rounds" } }));

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
    async fn tickets_wait_for_the_spec_pr_branch_github_wont_give() {
        let (fake, st) = (fake_github(), state());
        let slug = "octo/tickets-no-pr";
        let a = planner_on(&st, "tickets-no-pr", Some("git@github.com:octo/tickets-no-pr.git"));
        repo(&fake, slug, &["spec", "ready-for-agent"], 40);
        publish(&st, &a, "Rounds").await.unwrap();
        tickets_repo(&fake, slug, 40, &[41]);
        fake.route("GET", &format!("/repos/{slug}/pulls/500"), 502, json!({ "message": "Server Error" }));

        let err = publish_tickets(&st, &a, Some(40), vec![ticket("Store", &[])]).await.unwrap_err();
        assert_eq!(err, "Couldn't read spec PR #500 on octo/tickets-no-pr for its branch: GitHub 502: Server Error");
        assert_eq!(posts(&fake, slug, "issues").len(), 1, "no ticket goes up without its branch");
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

    #[tokio::test]
    async fn the_planning_guide_gives_the_spec_and_ticket_templates() {
        let st = state();
        let a = crate::tasks::tests::agent(&st, "guide");
        let g = mcp_call(&st, &a, "planning_guide", &json!({})).await.unwrap().unwrap();
        assert!(g.contains("## Problem Statement") && g.contains("## Acceptance criteria") && !g.contains("# Planning: grilling"), "{g}");
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
        // The team has a reviewer but no developer.
        let mut rev = crate::tasks::tests::agent(&st, "Checker");
        (rev.project, rev.role) = ("card-team".into(), "reviewer".into());
        st.db.save_agent(&rev).unwrap();

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
            "next_message": "Implement spec #40: hand the ready tickets to the team",
            "missing_roles": ["dev"],
            "setup": {
                "agent_id": "loft",
                "message": format!("Set up the team for the card-team project: it has no Dev yet. {} just planned spec #40: https://github.com/octo/card-team/issues/40. Propose the agents it needs and create the ones I approve.", a.name),
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
    async fn a_full_team_gets_no_setup_button_and_an_unpublished_grill_no_card() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "card-full", Some("git@github.com:octo/card-full.git"));
        tickets_repo(&fake, "octo/card-full", 7, &[7]);
        for (name, role) in [("Builder", "dev"), ("Checker", "reviewer")] {
            let mut x = crate::tasks::tests::agent(&st, name);
            x.project = " card-full ".into();
            x.role = role.into();
            st.db.save_agent(&x).unwrap();
        }
        crate::grill::tests::round(&st, &a, &["Spec?"]).unwrap();
        crate::grill::tests::send(&st, &a).unwrap();
        publish(&st, &a, "Rounds").await.unwrap();
        crate::grill::mcp_call(&st, &a, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        let card = &summary(&st, &a).meta["grill"]["end_card"];
        assert_eq!((&card["missing_roles"], &card["setup"], &card["tickets"]), (&json!([]), &Value::Null, &json!([])));
        assert_eq!(card["next_message"], "Implement spec #7: hand the ready tickets to the team");

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
            (x.title, x.role, x.project) = (title.into(), role.into(), project.into());
            st.db.save_agent(&x).unwrap();
            x
        };
        assert_eq!(missing_roles(&st, "team-a"), ["dev", "reviewer"]);
        add("Lead", "Team lead", "", "team-a");
        add("Dev", "Feature developer", "", "team-a");
        add("Builder", "", "dev", "team-b");
        assert_eq!(missing_roles(&st, "team-a"), ["dev", "reviewer"], "names, titles and other projects don't count");
        // A QA engineer reviews: it doesn't make the team's Dev.
        add("QA", "QA engineer", "reviewer", "team-a");
        assert_eq!(missing_roles(&st, "team-a"), ["dev"]);
        // A renamed Dev still counts.
        let mut dev = add("Dev", "Feature developer", "dev", "team-a");
        (dev.name, dev.title) = ("Ana".into(), "Frontend".into());
        st.db.save_agent(&dev).unwrap();
        assert!(missing_roles(&st, "team-a").is_empty(), "a full team");
        assert_eq!(missing_roles(&st, " "), ["dev", "reviewer"], "no project, no team");
    }

    #[test]
    fn agents_saved_before_roles_get_one_inferred_once() {
        assert_eq!(infer_role("QA", "QA engineer"), "reviewer");
        assert_eq!(infer_role("Security", "Security auditor"), "reviewer");
        assert_eq!(infer_role("Loft Reviewer", ""), "reviewer");
        assert_eq!(infer_role("Ana", "Desenvolvedora"), "dev");
        assert_eq!(infer_role("Dev", "Feature developer"), "dev");
        assert_eq!(infer_role("Devious", "Docs writer"), "");
        assert_eq!(infer_role("Lead", "Team lead"), "");

        let dir = std::env::temp_dir().join(format!("loft-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loft.db");
        let db = crate::db::Db::open(&path).unwrap();
        let st = AppState { db, ..state() };
        let mut old = crate::tasks::tests::agent(&st, "QA");
        old.title = "QA engineer".into();
        st.db.save_agent(&old).unwrap();
        let mut set = crate::tasks::tests::agent(&st, "Dev");
        set.role = "reviewer".into();
        st.db.save_agent(&set).unwrap();
        // Strip `role` from the first, as a database from before roles has it.
        let mut raw = serde_json::to_value(&old).unwrap();
        raw.as_object_mut().unwrap().remove("role");
        rusqlite::Connection::open(&path).unwrap().execute("UPDATE agents SET data = ?1 WHERE id = ?2", rusqlite::params![raw.to_string(), old.id]).unwrap();
        drop(st);

        let db = crate::db::Db::open(&path).unwrap();
        assert_eq!(db.get_agent(&old.id).unwrap().unwrap().role, "reviewer");
        assert_eq!(db.get_agent(&set.id).unwrap().unwrap().role, "reviewer", "a stored role is kept, even one its name doesn't suggest");
    }
}
