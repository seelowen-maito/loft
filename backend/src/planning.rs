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

pub fn mcp_tools() -> Vec<Value> {
    vec![json!({
        "name": "publish_spec",
        "description": "Publish the spec as a GitHub issue on your project's repo, labelled `spec` and assigned to the user. Loft creates it with the user's token and creates missing labels. Calling it again with the same title returns the spec already published instead of creating another, so a retry is safe.",
        "inputSchema": { "type": "object", "properties": {
            "title": { "type": "string", "description": "The spec's title: what gets built, in a few words." },
            "body": { "type": "string", "description": "The spec in Markdown, following the spec template." }
        }, "required": ["title", "body"] }
    })]
}

/// Handle the planning tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    if name != "publish_spec" {
        return None;
    }
    if !crate::grill::enabled(me) {
        return Some(Err("Planning isn't on for you.".into()));
    }
    Some(publish_spec(state, me, args["title"].as_str().unwrap_or(""), args["body"].as_str().unwrap_or("")).await)
}

/// Same spec, same key: case and spacing don't make a new one.
fn key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// `owner/repo` from the `origin` of the agent's project folder.
async fn repo_for(state: &AppState, a: &Agent) -> Result<String, String> {
    let project = a.project.trim();
    if project.is_empty() {
        return Err("You have no project, so there is no repo to publish to. Ask the user to set your project in the Config tab.".into());
    }
    let dir = state.settings.boot().projects_dir.join(project);
    if !dir.join(".git").exists() {
        return Err(format!("The project folder {} isn't a git repo, so there is no GitHub repo to publish to.", dir.display()));
    }
    let remote = crate::repos::git(&dir, &["remote", "get-url", "origin"])
        .await
        .map_err(|_| format!("The project {project} has no `origin` remote, so there is no GitHub repo to publish to."))?;
    crate::repos::github_slug(&remote).ok_or_else(|| format!("The project {project}'s `origin` ({}) isn't on GitHub.", remote.trim()))
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

async fn publish_spec(state: &AppState, me: &Agent, title: &str, body: &str) -> Result<String, String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || body.trim().is_empty() {
        return Err("title and body are required".into());
    }
    if title.chars().count() > MAX_TITLE {
        return Err(format!("the title is at most {MAX_TITLE} characters"));
    }
    let slug = repo_for(state, me).await?;
    if crate::github::token(state).is_none() {
        return Err("GitHub isn't connected: the user needs to add a GitHub token in Loft's Settings or run `gh auth login`.".into());
    }
    let k = key(&title);
    let already = |n: i64| format!("The spec \"{title}\" is already published as #{n}: https://github.com/{slug}/issues/{n}. Nothing new was created and its text wasn't changed.");
    if let Ok(Some(n)) = state.db.published_issue(&slug, "spec", &k) {
        return Ok(already(n));
    }
    let me_gh = crate::github::get(state, "/user").await.map_err(|e| format!("Couldn't read the GitHub user of Loft's token: {e}"))?;
    let login = me_gh["login"].as_str().ok_or("GitHub didn't say who the token belongs to.")?.to_string();
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

    fn posts(fake: &Fake, slug: &str, what: &str) -> Vec<Value> {
        fake.seen(&format!("/repos/{slug}/{what}")).into_iter().filter(|s| s.method == "POST").map(|s| s.body).collect()
    }

    #[tokio::test]
    async fn publishes_the_spec_labelled_and_assigned_creating_missing_labels() {
        let (fake, st) = (fake_github(), state());
        let a = planner_on(&st, "spec-new", Some("git@github.com:octo/spec-new.git"));
        repo(&fake, "octo/spec-new", &["bug", "Ready-For-Agent"], 40);

        let out = publish(&st, &a, "  Planning   in rounds ").await.unwrap();
        assert_eq!(out, "Published the spec as #40, assigned to octo-me: https://github.com/octo/spec-new/issues/40");

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
}
