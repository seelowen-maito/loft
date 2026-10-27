//! Publishing a plan as GitHub issues: Loft itself creates them with the user's token, on the repo
//! of the agent's project, and records what it published so a retry never creates a second one.

use serde_json::{Value, json};

use crate::db::Agent;
use crate::state::AppState;

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
        "description": "Publish the spec as a GitHub issue on your project's repo, labelled `spec` and assigned to the user. Loft creates it with the user's token and creates missing labels. Calling it again with the same title returns the spec already published instead of creating another, so a retry is safe.",
        "inputSchema": { "type": "object", "properties": {
            "title": { "type": "string", "description": "The spec's title: what gets built, in a few words." },
            "body": { "type": "string", "description": "The spec in Markdown, following the spec template." }
        }, "required": ["title", "body"] }
    }), json!({
        "name": "publish_tickets",
        "description": "Publish tickets as GitHub issues on your project's repo, only after the user approved the breakdown. Loft creates them blockers first as sub-issues of the spec, labelled `ready-for-agent` and assigned to the user, and links each blocker as a GitHub issue dependency. Without `spec` it publishes one standalone ticket. Calling it again with the same tickets skips what is already published, so a retry after a failure is safe.",
        "inputSchema": { "type": "object", "properties": {
            "spec": { "type": "integer", "description": "The spec's issue number from publish_spec. Leave it out only for a single standalone ticket." },
            "tickets": { "type": "array", "description": "The tickets, in any order.", "items": { "type": "object", "properties": {
                "title": { "type": "string", "description": "What the slice delivers, in a few words." },
                "body": { "type": "string", "description": "The ticket in Markdown, following the ticket template." },
                "blocked_by": { "type": "array", "items": { "type": "string" }, "description": "Titles of the tickets in this list that must land before this one." }
            }, "required": ["title", "body"] } }
        }, "required": ["tickets"] }
    })]
}

/// Handle the planning tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    if name != "publish_spec" && name != "publish_tickets" {
        return None;
    }
    if !crate::grill::enabled(me) {
        return Some(Err("Planning isn't on for you.".into()));
    }
    let out = if name == "publish_tickets" {
        publish_tickets(state, me, args).await
    } else {
        publish_spec(state, me, args["title"].as_str().unwrap_or(""), args["body"].as_str().unwrap_or("")).await
    };
    // Even a failure can leave issues published, so the Issues tab refetches either way.
    state.emit(crate::state::Event::Issues { project: me.project.trim().to_string() });
    Some(out)
}

/// Same spec, same key: case and spacing don't make a new one.
fn key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// `owner/repo` from the `origin` of the agent's project folder.
async fn repo_for(state: &AppState, a: &Agent) -> Result<String, String> {
    if a.project.trim().is_empty() {
        return Err("You have no project, so there is no repo to publish to. Ask the user to set your project in the Config tab.".into());
    }
    crate::repos::github_repo(state, &a.project).await.map_err(|e| format!("{e}, so there is no GitHub repo to publish to."))
}

/// Creates the labels the repo doesn't have yet; existing ones are left as they are.
async fn ensure_labels(state: &AppState, slug: &str) -> Result<(), String> {
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
    Ok(())
}

fn connected(state: &AppState) -> Result<(), String> {
    match crate::github::token(state) {
        Some(_) => Ok(()),
        None => Err("GitHub isn't connected: the user needs to add a GitHub token in Loft's Settings or run `gh auth login`.".into()),
    }
}

/// The login of the token's user: everything Loft publishes is assigned to them.
async fn login(state: &AppState) -> Result<String, String> {
    let me_gh = crate::github::get(state, "/user").await.map_err(|e| format!("Couldn't read the GitHub user of Loft's token: {e}"))?;
    Ok(me_gh["login"].as_str().ok_or("GitHub didn't say who the token belongs to.")?.to_string())
}

async fn publish_spec(state: &AppState, me: &Agent, title: &str, body: &str) -> Result<String, String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || body.trim().is_empty() {
        return Err("title and body are required".into());
    }
    if title.chars().count() > MAX_TITLE {
        return Err(format!("the title is at most {MAX_TITLE} characters"));
    }
    let slug = repo_for(state, me).await?;
    connected(state)?;
    let k = key(&title);
    let already = |n: i64| format!("The spec \"{title}\" is already published as #{n}: https://github.com/{slug}/issues/{n}. Nothing new was created and its text wasn't changed.");
    if let Ok(Some(n)) = state.db.published_issue(&slug, "spec", &k) {
        return Ok(already(n));
    }
    let login = login(state).await?;
    ensure_labels(state, &slug).await?;
    // Created before but never recorded (the reply was lost): adopt it rather than publish it twice.
    let existing = crate::github::get(state, &format!("/repos/{slug}/issues?labels=spec&state=all&per_page=100"))
        .await
        .map_err(|e| format!("Couldn't check {slug} for an existing spec: {e}"))?;
    let found = existing.as_array().into_iter().flatten().find(|i| i.get("pull_request").is_none() && i["title"].as_str().is_some_and(|t| key(t) == k));
    if let Some(n) = found.and_then(|i| i["number"].as_i64()) {
        let _ = state.db.record_published_issue(&slug, "spec", &k, n);
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
    state.db.record_published_issue(&slug, "spec", &k, n).map_err(|e| e.to_string())?;
    Ok(format!("Published the spec as #{n}, assigned to {login}: https://github.com/{slug}/issues/{n}"))
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

async fn publish_tickets(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    let ts = tickets_in(args)?;
    let spec = args["spec"].as_i64();
    if spec.is_none() && ts.len() > 1 {
        return Err("Several tickets belong to a spec: publish it with `publish_spec` first and pass its number as `spec`. Only a single ticket goes without one.".into());
    }
    let order = blockers_first(&ts)?;
    let slug = repo_for(state, me).await?;
    connected(state)?;
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
    let scope = spec.map_or("standalone".to_string(), |s| format!("#{s}"));
    let retry = "What was published so far stays: call publish_tickets again with the same tickets to finish without duplicates.";
    let mut made: Vec<Option<(i64, Option<i64>)>> = vec![None; ts.len()];
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
                Some(json!({ "title": t.title, "body": t.body, "labels": ["ready-for-agent"], "assignees": [login] })),
            )
            .await
            .map_err(|e| format!("GitHub rejected the ticket \"{}\" on {slug}: {e}. {retry}", t.title))?;
            let (Some(n), Some(id)) = (v["number"].as_i64(), v["id"].as_i64()) else {
                return Err(format!("GitHub created the ticket \"{}\" but didn't return its number. {retry}", t.title));
            };
            state.db.record_published_issue(&slug, "ticket", &k, n).map_err(|e| e.to_string())?;
            fresh += 1;
            (n, Some(id), true)
        };
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
    let head = match (spec, ts.len()) {
        (Some(s), c) => format!("Published {c} ticket{} as sub-issues of spec #{s}, blockers first", if c == 1 { "" } else { "s" }),
        (None, _) => "Published a standalone ticket".to_string(),
    };
    let none_new = if fresh == 0 { "\nNothing new was created: every ticket was already published." } else { "" };
    Ok(format!("{head}, labelled `ready-for-agent` and assigned to {login}:\n{}{none_new}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::tests::{Fake, fake_github, state_with_token};
    use crate::grill::tests::planner;

    /// A test state with a GitHub token whose projects folder is its own temp dir, read at boot like Loft does.
    fn state() -> AppState {
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
        assert_eq!(out, "Published the spec as #40, assigned to octo-me: https://github.com/octo/spec-new/issues/40");
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
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-partial", Some("git@github.com:octo/spec-partial.git"));
        repo(&fake, "octo/spec-partial", &[], 3);

        // The labels go up, then GitHub turns the spec down.
        fake.route("POST", "/repos/octo/spec-partial/issues", 422, json!({ "message": "Validation Failed", "errors": [{ "message": "could not add assignee" }] }));
        let err = publish(&st, &a, "Specs").await.unwrap_err();
        assert_eq!(err, "GitHub rejected the spec on octo/spec-partial: GitHub 422: Validation Failed (could not add assignee)");
        assert_eq!(posts(&fake, "octo/spec-partial", "labels").len(), 2);

        // The retry finds the labels in place and creates the spec once.
        fake.route("GET", "/repos/octo/spec-partial/labels", 200, json!([{ "name": "spec" }, { "name": "ready-for-agent" }]));
        fake.route("POST", "/repos/octo/spec-partial/issues", 201, json!({ "number": 3 }));
        assert!(publish(&st, &a, "Specs").await.unwrap().starts_with("Published the spec as #3"));
        assert_eq!(posts(&fake, "octo/spec-partial", "labels").len(), 2);
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
        assert_eq!(fake.seen("/repos/octo/spec-lost/issues?").len(), 1, "the second retry trusts Loft's record");
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

    #[test]
    fn the_planning_prompt_offers_the_spec_and_asks_the_seams() {
        let st = state();
        let p = crate::grill::prompt_section(&planner(&st, "spec-prompt"));
        for want in ["Turn this into a spec and tickets?", "Test seams", "round question", "## Problem Statement", "## Testing Decisions", "## Out of Scope", "mcp__loft__publish_spec"] {
            assert!(p.contains(want), "missing {want}");
        }
        assert!(crate::grill::prompt_section(&crate::tasks::tests::agent(&st, "spec-none")).is_empty());
    }

    #[tokio::test]
    async fn explains_what_is_missing() {
        let (_fake, st) = (fake_github(), state());
        let mut a = planner(&st, "spec-nowhere");
        assert!(publish(&st, &a, "X").await.unwrap_err().starts_with("You have no project"));
        a.project = "spec-nowhere".into();
        assert!(publish(&st, &a, "X").await.unwrap_err().contains("isn't a git repo"));

        let a = planner_on(&st, "spec-no-origin", None);
        assert!(publish(&st, &a, "X").await.unwrap_err().contains("has no `origin` remote"));
        let a = planner_on(&st, "spec-gitlab", Some("https://gitlab.com/octo/x.git"));
        assert!(publish(&st, &a, "X").await.unwrap_err().contains("isn't on GitHub"));

        let a = planner_on(&st, "spec-no-token", Some("https://github.com/octo/spec-no-token"));
        let mut s = st.settings.get();
        s.github_token = String::new();
        st.settings.save(s).unwrap();
        assert!(publish(&st, &a, "X").await.unwrap_err().starts_with("GitHub isn't connected"));

        let mut plain = crate::tasks::tests::agent(&st, "spec-plain");
        plain.project = "spec-gitlab".into();
        assert_eq!(publish(&st, &plain, "X").await.unwrap_err(), "Planning isn't on for you.");
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
        assert_eq!(fake.seen(&format!("/repos/{slug}/issues/21")).len(), 1);
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

        let mut plain = crate::tasks::tests::agent(&st, "tickets-plain");
        plain.project = "tickets-bad".into();
        assert_eq!(publish_tickets(&st, &plain, None, vec![ticket("A", &[])]).await.unwrap_err(), "Planning isn't on for you.");
    }

    #[test]
    fn the_planning_prompt_slices_tickets_and_covers_direct_requests() {
        let st = state();
        let p = crate::grill::prompt_section(&planner(&st, "tickets-prompt"));
        for want in [
            "Publish these N tickets?", "Vertical slices", "tracer bullet", "expand–contract", "## Acceptance criteria", "mcp__loft__publish_tickets",
            "# Planning: direct requests", "Never start a grill unasked", "at most one short question", "/grill", "single standalone ticket",
        ] {
            assert!(p.contains(want), "missing {want}");
        }
    }
}
