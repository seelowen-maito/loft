//! The Issues tab: each project's GitHub issues, read-only and fetched live through the GitHub client.
//! A project is one the agents belong to; its repo comes from the project folder's `origin`.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, bad, not_found};
use crate::state::AppState;

/// The fields the tab shows of an issue; `body` only on the single-issue view.
fn shape(i: &Value, body: bool) -> Value {
    let mut v = json!({
        "number": i["number"],
        "title": i["title"],
        "state": i["state"],
        "state_reason": i["state_reason"],
        "url": i["html_url"],
        "author": i["user"]["login"],
        "labels": i["labels"].as_array().into_iter().flatten().map(|l| json!({ "name": l["name"], "color": l["color"], "description": l["description"] })).collect::<Vec<_>>(),
        "assignees": i["assignees"].as_array().into_iter().flatten().map(|a| json!({ "login": a["login"], "avatar_url": a["avatar_url"] })).collect::<Vec<_>>(),
        "comments": i["comments"].as_i64().unwrap_or(0),
        "created_at": i["created_at"],
        "updated_at": i["updated_at"],
        "sub_issues": { "total": i["sub_issues_summary"]["total"].as_i64().unwrap_or(0), "completed": i["sub_issues_summary"]["completed"].as_i64().unwrap_or(0) },
        // Open ones only, as GitHub marks an issue blocked: a closed blocker no longer holds it up.
        "blocked_by": i["issue_dependencies_summary"]["blocked_by"].as_i64().unwrap_or(0),
        "blocking": i["issue_dependencies_summary"]["blocking"].as_i64().unwrap_or(0),
    });
    if body {
        v["body"] = json!(i["body"].as_str().unwrap_or(""));
    }
    v
}

/// A linked issue: enough to name it and open it.
fn link(i: &Value) -> Value {
    json!({ "number": i["number"], "title": i["title"], "state": i["state"], "url": i["html_url"] })
}

/// The projects the agents belong to, A–Z (case-insensitive), each once.
fn projects(state: &AppState) -> anyhow::Result<Vec<String>> {
    let mut ps: Vec<String> = state.db.list_agents()?.iter().map(|a| a.project.trim().to_string()).filter(|p| !p.is_empty()).collect();
    ps.sort();
    ps.dedup();
    ps.sort_by_key(|p| p.to_lowercase());
    Ok(ps)
}

/// Pages of 100 read per repo at most: 1,000 issues.
const MAX_PAGES: usize = 10;

/// Open and closed issues, newest first. GraphQL's `issues` never holds pull requests, so a PR-heavy repo
/// costs no pages; a connection whose `want` is false is skipped.
const QUERY: &str = r#"
query($owner: String!, $name: String!, $wantOpen: Boolean!, $openAfter: String, $wantClosed: Boolean!, $closedAfter: String) {
  repository(owner: $owner, name: $name) {
    open: issues(states: OPEN, first: 100, after: $openAfter, orderBy: {field: CREATED_AT, direction: DESC}) @include(if: $wantOpen) { ...page }
    closed: issues(states: CLOSED, first: 100, after: $closedAfter, orderBy: {field: CREATED_AT, direction: DESC}) @include(if: $wantClosed) { ...page }
  }
}
fragment page on IssueConnection {
  pageInfo { hasNextPage endCursor }
  nodes {
    number title state stateReason url author { login } createdAt updatedAt
    labels(first: 20) { nodes { name color description } }
    assignees(first: 10) { nodes { login avatarUrl } }
    comments { totalCount }
    subIssuesSummary { total completed }
    issueDependenciesSummary { blockedBy blocking }
  }
}"#;

/// A GraphQL issue in the REST fields `shape` reads.
fn rest(n: &Value) -> Value {
    let lower = |v: &Value| v.as_str().map_or(Value::Null, |s| json!(s.to_lowercase()));
    json!({
        "number": n["number"], "title": n["title"], "state": lower(&n["state"]), "state_reason": lower(&n["stateReason"]),
        "html_url": n["url"], "user": { "login": n["author"]["login"] },
        "labels": n["labels"]["nodes"],
        "assignees": n["assignees"]["nodes"].as_array().into_iter().flatten().map(|a| json!({ "login": a["login"], "avatar_url": a["avatarUrl"] })).collect::<Vec<_>>(),
        "comments": n["comments"]["totalCount"], "created_at": n["createdAt"], "updated_at": n["updatedAt"],
        "sub_issues_summary": n["subIssuesSummary"],
        "issue_dependencies_summary": { "blocked_by": n["issueDependenciesSummary"]["blockedBy"], "blocking": n["issueDependenciesSummary"]["blocking"] },
    })
}

/// Where one connection stands: pages read and the cursor of the next, if any.
struct Conn {
    pages: usize,
    next: Option<String>,
}

impl Conn {
    fn more(&self) -> bool {
        self.pages == 0 || self.next.is_some()
    }
}

/// Open issues in full (up to `MAX_PAGES`), closed ones with the pages left, so an old open issue never
/// drops off. The first request asks for both, so most repos take one; also whether any were left.
async fn fetch_issues(state: &AppState, repo: &str) -> anyhow::Result<(Vec<Value>, bool)> {
    let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
    let (mut open, mut closed) = (Conn { pages: 0, next: None }, Conn { pages: 0, next: None });
    let (mut opens, mut closeds) = (vec![], vec![]);
    loop {
        let want_open = open.more() && open.pages < MAX_PAGES;
        let want_closed = closed.more() && (closed.pages == 0 || !open.more()) && open.pages + closed.pages < MAX_PAGES;
        if !want_open && !want_closed {
            break;
        }
        let vars = json!({
            "owner": owner, "name": name,
            "wantOpen": want_open, "openAfter": open.next, "wantClosed": want_closed, "closedAfter": closed.next,
        });
        let resp = crate::github::request(state, reqwest::Method::POST, "/graphql", Some(json!({ "query": QUERY, "variables": vars }))).await?;
        if let Some(e) = resp["errors"][0]["message"].as_str() {
            anyhow::bail!("GitHub: {e}");
        }
        for (want, key, conn, out) in [(want_open, "open", &mut open, &mut opens), (want_closed, "closed", &mut closed, &mut closeds)] {
            if !want {
                continue;
            }
            let c = &resp["data"]["repository"][key];
            out.extend(c["nodes"].as_array().into_iter().flatten().map(|n| shape(&rest(n), false)));
            conn.pages += 1;
            conn.next = c["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false).then(|| c["pageInfo"]["endCursor"].as_str().map(String::from)).flatten();
        }
    }
    opens.extend(closeds);
    Ok((opens, open.next.is_some() || closed.next.is_some()))
}

async fn project_issues(state: &AppState, project: String) -> Value {
    let repo = match crate::repos::github_repo(state, &project).await {
        Ok(r) => r,
        Err(e) => return json!({ "project": project, "repo": null, "problem": format!("{e}."), "issues": [] }),
    };
    match fetch_issues(state, &repo).await {
        Ok((issues, truncated)) => json!({ "project": project, "repo": repo, "issues": issues, "truncated": truncated }),
        Err(e) => json!({ "project": project, "repo": repo, "error": e.to_string(), "issues": [] }),
    }
}

#[derive(Deserialize)]
pub struct Filter {
    project: Option<String>,
}

/// Every project's issues (or just `project`'s, as a publish refetches), open and closed, newest
/// first as GitHub lists them. Without a token nothing is fetched and `connected` is false.
pub async fn list(State(state): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let connected = crate::github::token(&state).is_some();
    let mut ps = projects(&state)?;
    if let Some(p) = f.project {
        ps.retain(|q| *q == p.trim());
        if ps.is_empty() {
            return Err(not_found());
        }
    }
    let out: Vec<Value> = if connected {
        futures::future::join_all(ps.into_iter().map(|p| project_issues(&state, p))).await
    } else {
        let mut out = vec![];
        for p in ps {
            let (repo, problem) = match crate::repos::github_repo(&state, &p).await {
                Ok(r) => (json!(r), Value::Null),
                Err(e) => (Value::Null, json!(format!("{e}."))),
            };
            out.push(json!({ "project": p, "repo": repo, "problem": problem, "issues": [] }));
        }
        out
    };
    Ok(Json(json!({ "connected": connected, "projects": out })))
}

#[derive(Deserialize)]
pub struct One {
    project: String,
}

/// One issue with its body, the sub-issues of a spec and a ticket's blocked-by and blocking issues.
pub async fn get(State(state): State<AppState>, Path(number): Path<i64>, Query(q): Query<One>) -> ApiResult<Value> {
    if !projects(&state)?.contains(&q.project.trim().to_string()) {
        return Err(not_found());
    }
    if crate::github::token(&state).is_none() {
        return Err(bad("GitHub isn't connected"));
    }
    let repo = crate::repos::github_repo(&state, &q.project).await.map_err(bad)?;
    let base = format!("/repos/{repo}/issues/{number}");
    let issue = crate::github::get(&state, &base).await.map_err(|e| bad(e.to_string()))?;
    if issue.get("pull_request").is_some() {
        return Err(not_found());
    }
    let paths = ["sub_issues", "dependencies/blocked_by", "dependencies/blocking"].map(|p| format!("{base}/{p}?per_page=100"));
    let (subs, blocked_by, blocking) = futures::join!(crate::github::get(&state, &paths[0]), crate::github::get(&state, &paths[1]), crate::github::get(&state, &paths[2]));
    // A link list GitHub won't give (an older GitHub Enterprise, say) shows as empty, not as a failed issue.
    let links = |r: anyhow::Result<Value>| r.ok().and_then(|v| v.as_array().map(|a| a.iter().map(link).collect::<Vec<_>>())).unwrap_or_default();
    let mut v = shape(&issue, true);
    v["project"] = json!(q.project.trim());
    v["repo"] = json!(repo);
    v["sub_issue_list"] = json!(links(subs));
    v["blocked_by_list"] = json!(links(blocked_by));
    v["blocking_list"] = json!(links(blocking));
    Ok(Json(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::tests::{fake_github, own_fake, state_with_token};

    /// A test state with a GitHub token whose projects folder is its own temp dir, read at boot like Loft does.
    fn state() -> AppState {
        let st = state_with_token();
        let dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        let projects = dir.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(dir.join("settings.json"), json!({ "projects_dir": projects, "github_token": "test-token" }).to_string()).unwrap();
        AppState { settings: std::sync::Arc::new(crate::settings::Store::load(&dir).unwrap()), ..st }
    }

    /// An agent on `project`, whose folder is a git repo with this `origin` (none: not a repo at all).
    fn project(st: &AppState, project: &str, origin: Option<&str>) {
        let mut a = crate::tasks::tests::agent(st, &format!("{project}-dev"));
        a.project = project.into();
        st.db.save_agent(&a).unwrap();
        let Some(o) = origin else { return };
        let dir = st.settings.boot().projects_dir.join(project);
        assert!(dir.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        if !o.is_empty() {
            git(&["remote", "add", "origin", o]);
        }
    }

    fn issue(n: i64, title: &str) -> Value {
        json!({
            "number": n, "id": n * 100, "title": title, "state": "open", "state_reason": null,
            "html_url": format!("https://github.com/octo/issues-tab/issues/{n}"), "user": { "login": "octo-me" },
            "labels": [{ "name": "spec", "color": "5319e7", "description": "What to build", "id": 1 }],
            "assignees": [{ "login": "octo-me", "avatar_url": "https://avatars.example/octo-me", "id": 2 }],
            "comments": 3, "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-02T00:00:00Z",
            "body": "## What to build\n\nRounds.",
            "sub_issues_summary": { "total": 2, "completed": 1, "percent_completed": 50 },
            "issue_dependencies_summary": { "blocked_by": 1, "total_blocked_by": 3, "blocking": 2, "total_blocking": 2 },
        })
    }

    /// `issue` as GraphQL lists it.
    fn node(n: i64, title: &str) -> Value {
        json!({
            "number": n, "title": title, "state": "OPEN", "stateReason": null,
            "url": format!("https://github.com/octo/issues-tab/issues/{n}"), "author": { "login": "octo-me" },
            "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-02T00:00:00Z",
            "labels": { "nodes": [{ "name": "spec", "color": "5319e7", "description": "What to build" }] },
            "assignees": { "nodes": [{ "login": "octo-me", "avatarUrl": "https://avatars.example/octo-me" }] },
            "comments": { "totalCount": 3 },
            "subIssuesSummary": { "total": 2, "completed": 1 },
            "issueDependenciesSummary": { "blockedBy": 1, "blocking": 2 },
        })
    }

    /// One connection's page: these issues, then the cursor of the next page if there is one.
    fn conn(nodes: Vec<Value>, next: Option<&str>) -> Value {
        json!({ "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next }, "nodes": nodes })
    }

    /// A GraphQL reply with the `open` and `closed` connections asked for.
    fn reply(open: Option<Value>, closed: Option<Value>) -> (u16, Value) {
        let mut r = json!({});
        if let Some(o) = open {
            r["open"] = o;
        }
        if let Some(c) = closed {
            r["closed"] = c;
        }
        (200, json!({ "data": { "repository": r } }))
    }

    /// The variables of each issues query the fake got, oldest first.
    fn queries(fake: &crate::github::tests::Fake) -> Vec<Value> {
        fake.seen("/graphql").into_iter().map(|s| s.body["variables"].clone()).collect()
    }

    #[tokio::test]
    async fn lists_issues_per_project_and_explains_missing_remotes() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "issues-tab", Some("git@github.com:octo/issues-tab.git"));
        project(&st, "Issues-local", Some(""));
        project(&st, "issues-gitlab", Some("https://gitlab.com/octo/x.git"));
        project(&st, "issues-folderless", None);
        let mut done = node(3, "Shipped");
        (done["state"], done["stateReason"]) = (json!("CLOSED"), json!("NOT_PLANNED"));
        fake.in_turn("POST", "/graphql", vec![reply(Some(conn(vec![node(4, "Rounds")], None)), Some(conn(vec![done], None)))]);

        let Json(v) = list(State(st.clone()), Query(Filter { project: None })).await.unwrap();
        assert_eq!(v["connected"], true);
        let ps = v["projects"].as_array().unwrap();
        assert_eq!(ps.iter().map(|p| p["project"].as_str().unwrap()).collect::<Vec<_>>(), ["issues-folderless", "issues-gitlab", "Issues-local", "issues-tab"]);
        assert!(ps[0]["problem"].as_str().unwrap().ends_with("isn't a git repo."));
        assert_eq!(ps[1]["problem"], "The project issues-gitlab's `origin` (https://gitlab.com/octo/x.git) isn't on GitHub.");
        assert_eq!(ps[2]["problem"], "The project Issues-local has no `origin` remote.");
        assert!(ps[..3].iter().all(|p| p["repo"].is_null() && p["issues"] == json!([])));

        assert_eq!(ps[3]["repo"], "octo/issues-tab");
        assert_eq!(ps[3]["issues"][0], json!({
            "number": 4, "title": "Rounds", "state": "open", "state_reason": null,
            "url": "https://github.com/octo/issues-tab/issues/4", "author": "octo-me",
            "labels": [{ "name": "spec", "color": "5319e7", "description": "What to build" }],
            "assignees": [{ "login": "octo-me", "avatar_url": "https://avatars.example/octo-me" }],
            "comments": 3, "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-02T00:00:00Z",
            "sub_issues": { "total": 2, "completed": 1 }, "blocked_by": 1, "blocking": 2,
        }), "the same fields as REST gave, and no body");
        assert_eq!((&ps[3]["issues"][1]["state"], &ps[3]["issues"][1]["state_reason"]), (&json!("closed"), &json!("not_planned")));
        assert_eq!(ps[3]["truncated"], false);
        let q = queries(&fake);
        assert_eq!(q.len(), 1, "open and closed ones in one request");
        assert_eq!(q[0], json!({ "owner": "octo", "name": "issues-tab", "wantOpen": true, "openAfter": null, "wantClosed": true, "closedAfter": null }));
    }

    #[tokio::test]
    async fn reads_every_page_open_ones_first() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "issues-many", Some("https://github.com/octo/issues-many"));
        let mut closed = node(500, "Done");
        closed["state"] = json!("CLOSED");
        let mut older = closed.clone();
        older["number"] = json!(400);
        fake.in_turn("POST", "/graphql", vec![
            reply(Some(conn((1..=100).map(|n| node(n, "Old")).collect(), Some("o1"))), Some(conn(vec![closed], Some("c1")))),
            reply(Some(conn(vec![node(200, "Oldest open")], None)), None),
            reply(None, Some(conn(vec![older], None))),
        ]);

        let Json(v) = list(State(st), Query(Filter { project: None })).await.unwrap();
        let p = &v["projects"][0];
        let numbers: Vec<i64> = p["issues"].as_array().unwrap().iter().map(|i| i["number"].as_i64().unwrap()).collect();
        assert_eq!(numbers.len(), 103);
        assert_eq!(numbers[99..], [100, 200, 500, 400], "every open one, then the closed ones");
        assert_eq!(p["truncated"], false);
        let q = queries(&fake);
        let asked: Vec<_> = q.iter().map(|q| (q["wantOpen"].clone(), q["openAfter"].clone(), q["wantClosed"].clone(), q["closedAfter"].clone())).collect();
        assert_eq!(asked, [
            (json!(true), json!(null), json!(true), json!(null)),
            (json!(true), json!("o1"), json!(false), json!("c1")),
            (json!(false), json!(null), json!(true), json!("c1")),
        ], "closed pages go on once open ones are done");
    }

    #[tokio::test]
    async fn stops_after_max_pages() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "issues-huge", Some("https://github.com/octo/issues-huge"));
        fake.route("POST", "/graphql", 200, reply(Some(conn(vec![node(1, "Open")], Some("o"))), Some(conn(vec![node(2, "Closed")], Some("c")))).1);
        let Json(v) = list(State(st), Query(Filter { project: None })).await.unwrap();
        assert_eq!(v["projects"][0]["truncated"], true);
        let q = queries(&fake);
        assert_eq!(q.len(), MAX_PAGES, "no more closed pages while open ones fill the budget");
        assert!(q[1..].iter().all(|q| q["wantOpen"] == true && q["wantClosed"] == false));
    }

    #[tokio::test]
    async fn one_project_fetches_only_that_repo() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "issues-this", Some("https://github.com/octo/issues-this"));
        project(&st, "issues-that", Some("https://github.com/octo/issues-that"));
        fake.route("POST", "/graphql", 200, reply(Some(conn(vec![node(1, "This")], None)), Some(conn(vec![], None))).1);

        let Json(v) = list(State(st.clone()), Query(Filter { project: Some(" issues-this ".into()) })).await.unwrap();
        assert_eq!(v["projects"].as_array().unwrap().iter().map(|p| p["project"].as_str().unwrap()).collect::<Vec<_>>(), ["issues-this"]);
        assert_eq!(queries(&fake).iter().map(|q| q["name"].as_str().unwrap()).collect::<Vec<_>>(), ["issues-this"]);
        assert_eq!(list(State(st), Query(Filter { project: Some("../elsewhere".into()) })).await.unwrap_err().1, "not found");
    }

    #[tokio::test]
    async fn a_repo_github_turns_down_shows_its_error() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "issues-gone", Some("https://github.com/octo/issues-gone"));
        let message = "Could not resolve to a Repository with the name 'octo/issues-gone'.";
        fake.route("POST", "/graphql", 200, json!({ "data": { "repository": null }, "errors": [{ "type": "NOT_FOUND", "message": message }] }));
        let Json(v) = list(State(st), Query(Filter { project: None })).await.unwrap();
        assert_eq!(v["projects"][0], json!({ "project": "issues-gone", "repo": "octo/issues-gone", "error": format!("GitHub: {message}"), "issues": [] }));
    }

    #[tokio::test]
    async fn without_a_token_nothing_is_fetched() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "issues-offline", Some("https://github.com/octo/issues-offline"));
        let mut s = st.settings.get();
        s.github_token = String::new();
        st.settings.save(s).unwrap();

        let Json(v) = list(State(st.clone()), Query(Filter { project: None })).await.unwrap();
        assert_eq!(v, json!({ "connected": false, "projects": [{ "project": "issues-offline", "repo": "octo/issues-offline", "problem": null, "issues": [] }] }));
        assert!(fake.seen("/").is_empty());
        let err = get(State(st), Path(1), Query(One { project: "issues-offline".into() })).await.unwrap_err();
        assert_eq!(err.1, "GitHub isn't connected");
    }

    #[tokio::test]
    async fn one_issue_has_its_body_sub_issues_and_dependencies() {
        let (fake, st) = (fake_github(), state());
        project(&st, "issues-one", Some("git@github.com:octo/issues-one.git"));
        let base = "/repos/octo/issues-one/issues/4";
        fake.route("GET", base, 200, issue(4, "Rounds"));
        fake.route("GET", &format!("{base}/sub_issues"), 200, json!([issue(5, "Store"), issue(6, "UI")]));
        fake.route("GET", &format!("{base}/dependencies/blocked_by"), 200, json!([issue(2, "Docs")]));
        // No blocking route: a list GitHub won't give shows as empty.

        let Json(v) = get(State(st.clone()), Path(4), Query(One { project: " issues-one ".into() })).await.unwrap();
        assert_eq!((&v["project"], &v["repo"], &v["number"]), (&json!("issues-one"), &json!("octo/issues-one"), &json!(4)));
        assert_eq!(v["body"], "## What to build\n\nRounds.");
        assert_eq!(v["labels"][0]["color"], "5319e7");
        assert_eq!(v["sub_issue_list"], json!([
            { "number": 5, "title": "Store", "state": "open", "url": "https://github.com/octo/issues-tab/issues/5" },
            { "number": 6, "title": "UI", "state": "open", "url": "https://github.com/octo/issues-tab/issues/6" },
        ]));
        assert_eq!(v["blocked_by_list"], json!([{ "number": 2, "title": "Docs", "state": "open", "url": "https://github.com/octo/issues-tab/issues/2" }]));
        assert_eq!(v["blocking_list"], json!([]));
        let paths: Vec<String> = fake.seen(base).into_iter().map(|s| s.path).collect();
        assert!(paths.contains(&format!("{base}/dependencies/blocking?per_page=100")), "{paths:?}");
        assert!(fake.seen("/repos/octo/issues-one/").iter().all(|s| s.method == "GET"), "the tab never writes");

        // Only the agents' projects, and only issues.
        let err = get(State(st.clone()), Path(4), Query(One { project: "../elsewhere".into() })).await.unwrap_err();
        assert_eq!(err.1, "not found");
        fake.route("GET", "/repos/octo/issues-one/issues/7", 200, json!({ "number": 7, "title": "A PR", "pull_request": {} }));
        assert_eq!(get(State(st), Path(7), Query(One { project: "issues-one".into() })).await.unwrap_err().1, "not found");
    }
}
