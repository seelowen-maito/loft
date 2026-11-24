//! Saved: chat messages the user keeps for good, with a title and a tag (spec #675). An item holds a copy of the
//! message, so it outlives a cleared chat and a deleted agent; its page then says the source is gone.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiResult, bad, not_found};
use crate::db::{SavedItem, title_from};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct NewSaved {
    agent_id: String,
    message_id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    tag: String,
}

/// Saves a chat message: yours or the agent's text. An empty title becomes the message's first line.
pub async fn create(State(state): State<AppState>, Json(b): Json<NewSaved>) -> ApiResult<SavedItem> {
    let agent = state.db.get_agent(&b.agent_id)?.ok_or_else(not_found)?;
    let m = state.db.get_message(&agent.id, b.message_id)?.ok_or_else(|| bad("That message isn't in this chat."))?;
    if m.kind != "text" || !matches!(m.role.as_str(), "user" | "assistant") || m.content.trim().is_empty() {
        return Err(bad("Only a chat message with text can be saved."));
    }
    let title = match b.title.trim() {
        "" => title_from(&m.content),
        t => t.to_string(),
    };
    Ok(Json(state.db.add_saved(&title, b.tag.trim(), &m.content, &agent.id, &agent.name, m.id)?))
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    tag: String,
}

/// Every item, newest first, matching the search and the tag; `tags` is every tag in use, for the filter.
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<Value> {
    Ok(Json(json!({
        "items": state.db.list_saved(q.q.trim(), q.tag.trim())?,
        "tags": state.db.saved_tags()?,
    })))
}

/// One item, whether the message it came from is still in its agent's chat (`source_exists`), and the pull request
/// of its last export to docs (`pr`, or null).
pub async fn get(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    let item = state.db.get_saved(id)?.ok_or_else(not_found)?;
    let source_exists = state.db.get_message(&item.agent_id, item.message_id)?.is_some();
    let pr = match &item.export_pr {
        Some(pr) => state.db.get_pull_request(pr)?,
        None => None,
    };
    let mut v = json!(item);
    v["source_exists"] = json!(source_exists);
    v["pr"] = json!(pr);
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct ExportTo {
    project: String,
}

/// `docs/<slug>.md` for an item: its title in lowercase words joined by `-`, or `saved-<id>` when it has none.
fn doc_path(i: &SavedItem) -> String {
    let mut slug = String::new();
    for w in i.title.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        if !slug.is_empty() && slug.len() + w.len() >= 60 {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w.to_lowercase());
    }
    if slug.is_empty() { format!("docs/saved-{}.md", i.id) } else { format!("docs/{slug}.md") }
}

/// Exports an item to a project's docs: a new branch off the default one with `docs/<slug>.md` (made, or updated when
/// it's already there) holding the content and a line back to the item, and a pull request from it. Each export
/// opens a new PR, which becomes the item's. Answers the PR, as the PR cards have it.
pub async fn export(State(state): State<AppState>, Path(id): Path<i64>, Json(b): Json<ExportTo>) -> ApiResult<Value> {
    let item = state.db.get_saved(id)?.ok_or_else(not_found)?;
    let slug = crate::repos::github_repo(&state, &b.project).await.map_err(|e| bad(format!("{e}, so Loft can't open a pull request there.")))?;
    let failed = |e: anyhow::Error| bad(format!("Couldn't export to {slug}: {e}"));
    let repo = |path: String| format!("/repos/{slug}{path}");
    let base = crate::github::get(&state, &repo(String::new())).await.map_err(failed)?["default_branch"].as_str().unwrap_or("main").to_string();
    let head = crate::github::get(&state, &repo(format!("/git/ref/heads/{base}"))).await.map_err(failed)?;
    let branch = format!("docs/saved-{id}-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"));
    let post = reqwest::Method::POST;
    let r#ref = json!({ "ref": format!("refs/heads/{branch}"), "sha": head["object"]["sha"] });
    crate::github::request(&state, post.clone(), &repo("/git/refs".into()), Some(r#ref)).await.map_err(failed)?;

    let path = doc_path(&item);
    // Already in the docs: the PR updates it.
    let existing = match crate::github::get(&state, &repo(format!("/contents/{path}?ref={branch}"))).await {
        Ok(f) => f["sha"].as_str().map(String::from),
        Err(e) if crate::github::is_gone(&e) => None,
        Err(e) => return Err(failed(e)),
    };
    let port = state.cfg.port;
    let link = match tokio::task::spawn_blocking(move || crate::tailnet::web_url(port)).await.ok().flatten() {
        Some(web) => format!("[{}]({web}/saved/{id})", item.title),
        None => format!("{} (saved item {id})", item.title),
    };
    let doc = format!("{}\n\n---\n\n_From Loft's Saved: {link}._\n", item.content.trim_end());
    use base64::Engine;
    let mut put = json!({
        "message": format!("Docs: {} from Loft's Saved", item.title),
        "content": base64::engine::general_purpose::STANDARD.encode(doc),
        "branch": branch,
    });
    if let Some(sha) = &existing {
        put["sha"] = json!(sha);
    }
    crate::github::request(&state, reqwest::Method::PUT, &repo(format!("/contents/{path}")), Some(put)).await.map_err(failed)?;
    let verb = if existing.is_some() { "Updates" } else { "Adds" };
    let made = crate::github::request(&state, post, &repo("/pulls".into()), Some(json!({
        "title": format!("Docs: {}", item.title),
        "head": branch,
        "base": base,
        "body": format!("{verb} `{path}` with \"{}\", kept in Loft's Saved.", item.title),
    })))
    .await
    .map_err(failed)?;
    let number = made["number"].as_i64().ok_or_else(|| bad(format!("GitHub opened the pull request on {slug} but didn't return its number.")))?;
    let pr = crate::prs::register(&state, &item.agent_id, &slug, number, None).await.map_err(failed)?;
    state.db.set_saved_export(id, &pr.id)?;
    Ok(Json(json!(pr)))
}

#[derive(Deserialize)]
pub struct SavedEdit {
    title: String,
    #[serde(default)]
    tag: String,
    content: String,
}

/// Edits an item's title, tag and content; the item then says when it was edited (`updated_at`).
pub async fn update(State(state): State<AppState>, Path(id): Path<i64>, Json(b): Json<SavedEdit>) -> ApiResult<SavedItem> {
    let title = b.title.trim();
    if title.is_empty() {
        return Err(bad("A saved item needs a title."));
    }
    if b.content.trim().is_empty() {
        return Err(bad("A saved item needs some text."));
    }
    Ok(Json(state.db.update_saved(id, title, b.tag.trim(), &b.content)?.ok_or_else(not_found)?))
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    if !state.db.delete_saved(id)? {
        return Err(not_found());
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::{agent, state};

    async fn save(st: &AppState, agent_id: &str, message_id: i64, title: &str, tag: &str) -> Result<SavedItem, crate::api::ApiError> {
        let b = NewSaved { agent_id: agent_id.into(), message_id, title: title.into(), tag: tag.into() };
        create(State(st.clone()), Json(b)).await.map(|Json(i)| i)
    }

    async fn listed(st: &AppState, q: &str, tag: &str) -> Value {
        let Json(v) = list(State(st.clone()), Query(ListQuery { q: q.into(), tag: tag.into() })).await.unwrap();
        v
    }

    fn titles(v: &Value) -> Vec<&str> {
        v["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect()
    }

    #[tokio::test]
    async fn saves_the_agents_message_and_yours_with_title_tag_and_content() {
        let st = state();
        let a = agent(&st, "Dev");
        let reply = st.db.add_message(&a.id, None, "assistant", "text", "# Deploy\n\nRun `./start.sh`.", json!({})).unwrap();
        let mine = st.db.add_message(&a.id, None, "user", "text", "Remember: deploys go out on Fridays", json!({})).unwrap();

        let i = save(&st, &a.id, reply.id, " How to deploy ", "Tutorial").await.unwrap();
        assert_eq!((i.title.as_str(), i.tag.as_str(), i.content.as_str()), ("How to deploy", "Tutorial", "# Deploy\n\nRun `./start.sh`."));
        assert_eq!((i.agent_id.as_str(), i.agent_name.as_str(), i.message_id), (a.id.as_str(), "Dev", reply.id));

        // No title: the message's first line.
        let i = save(&st, &a.id, mine.id, "", "Decision").await.unwrap();
        assert_eq!(i.title, "Remember: deploys go out on Fridays");
        assert_eq!(i.content, mine.content);
    }

    #[tokio::test]
    async fn only_chat_text_from_that_agent_can_be_saved() {
        let st = state();
        let a = agent(&st, "Dev");
        let b = agent(&st, "Other");
        let tool = st.db.add_message(&a.id, None, "assistant", "tool_use", "Bash", json!({})).unwrap();
        let text = st.db.add_message(&a.id, None, "assistant", "text", "hi", json!({})).unwrap();
        assert!(save(&st, &a.id, tool.id, "t", "").await.is_err());
        // Another agent's chat doesn't hold it.
        assert!(save(&st, &b.id, text.id, "t", "").await.is_err());
        assert_eq!(save(&st, "no-such-agent", text.id, "t", "").await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn lists_newest_first_with_search_and_a_tag_filter() {
        let st = state();
        let a = agent(&st, "Dev");
        for (title, content, tag) in [("Deploy", "Run start.sh", "Tutorial"), ("Ports", "Each agent has ten", "Good to know"), ("Sqlite", "We keep SQLite", "Decision")] {
            let m = st.db.add_message(&a.id, None, "assistant", "text", content, json!({})).unwrap();
            save(&st, &a.id, m.id, title, tag).await.unwrap();
        }
        let all = listed(&st, "", "").await;
        assert_eq!(titles(&all), ["Sqlite", "Ports", "Deploy"]);
        assert_eq!(all["tags"], json!(["Decision", "Good to know", "Tutorial"]));
        // Search reads the title and the content, ignoring case.
        assert_eq!(titles(&listed(&st, "deploy", "").await), ["Deploy"]);
        assert_eq!(titles(&listed(&st, "SQLITE", "").await), ["Sqlite"]);
        assert_eq!(titles(&listed(&st, "ten", "").await), ["Ports"]);
        // A % is plain text, not a wildcard.
        assert!(titles(&listed(&st, "%", "").await).is_empty());
        // The tag filter, alone and with a search; the tag list stays whole.
        let tagged = listed(&st, "", "good to know").await;
        assert_eq!(titles(&tagged), ["Ports"]);
        assert_eq!(tagged["tags"].as_array().unwrap().len(), 3);
        assert!(titles(&listed(&st, "deploy", "Decision").await).is_empty());
    }

    #[tokio::test]
    async fn an_item_outlives_a_cleared_chat_and_a_deleted_agent() {
        let st = state();
        let a = agent(&st, "Dev");
        let m = st.db.add_message(&a.id, None, "assistant", "text", "Keep this", json!({})).unwrap();
        let i = save(&st, &a.id, m.id, "Kept", "Good to know").await.unwrap();
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert_eq!(v["source_exists"], json!(true));

        st.db.clear_messages(&a.id).unwrap();
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert_eq!((v["title"].as_str(), v["content"].as_str(), v["source_exists"].as_bool()), (Some("Kept"), Some("Keep this"), Some(false)));

        st.db.delete_agent(&a.id).unwrap();
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert_eq!((v["agent_name"].as_str(), v["source_exists"].as_bool()), (Some("Dev"), Some(false)));
        assert_eq!(titles(&listed(&st, "", "").await), ["Kept"]);
    }

    fn edit(title: &str, tag: &str, content: &str) -> Json<SavedEdit> {
        Json(SavedEdit { title: title.into(), tag: tag.into(), content: content.into() })
    }

    #[tokio::test]
    async fn edits_the_title_tag_and_content_and_stamps_the_edit() {
        let st = state();
        let a = agent(&st, "Dev");
        let m = st.db.add_message(&a.id, None, "assistant", "text", "Run start.sh", json!({})).unwrap();
        let i = save(&st, &a.id, m.id, "Deploy", "Tutorial").await.unwrap();
        assert!(i.updated_at.is_none());

        let Json(e) = update(State(st.clone()), Path(i.id), edit(" How to deploy ", " Decision ", "# Deploy\n\nRun `./start.sh`.")).await.unwrap();
        assert_eq!((e.title.as_str(), e.tag.as_str(), e.content.as_str()), ("How to deploy", "Decision", "# Deploy\n\nRun `./start.sh`."));
        assert!(e.updated_at.is_some());
        // The source and creation time stay.
        assert_eq!((e.agent_id.as_str(), e.message_id, e.created_at.as_str()), (a.id.as_str(), m.id, i.created_at.as_str()));
        // The list and the page read the edit.
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert_eq!((v["title"].as_str(), v["tag"].as_str()), (Some("How to deploy"), Some("Decision")));
        assert_eq!(titles(&listed(&st, "start.sh", "Decision").await), ["How to deploy"]);
        assert_eq!(listed(&st, "", "").await["tags"], json!(["Decision"]));
        // The message in the chat is untouched.
        assert_eq!(st.db.get_message(&a.id, m.id).unwrap().unwrap().content, "Run start.sh");
    }

    #[tokio::test]
    async fn an_edit_needs_a_title_and_text_and_an_item() {
        let st = state();
        let a = agent(&st, "Dev");
        let m = st.db.add_message(&a.id, None, "assistant", "text", "Keep", json!({})).unwrap();
        let i = save(&st, &a.id, m.id, "Kept", "").await.unwrap();
        assert_eq!(update(State(st.clone()), Path(i.id), edit("  ", "", "x")).await.unwrap_err().0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(update(State(st.clone()), Path(i.id), edit("t", "", " \n")).await.unwrap_err().0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(update(State(st.clone()), Path(99), edit("t", "", "x")).await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
        // A refused edit changes nothing.
        let kept = st.db.get_saved(i.id).unwrap().unwrap();
        assert_eq!((kept.title.as_str(), kept.content.as_str(), kept.updated_at), ("Kept", "Keep", None));
    }

    #[tokio::test]
    async fn deletes_an_item_and_only_that_one() {
        let st = state();
        let a = agent(&st, "Dev");
        let m = st.db.add_message(&a.id, None, "assistant", "text", "Keep", json!({})).unwrap();
        let gone = save(&st, &a.id, m.id, "Gone", "").await.unwrap();
        save(&st, &a.id, m.id, "Stays", "").await.unwrap();
        let Json(v) = delete(State(st.clone()), Path(gone.id)).await.unwrap();
        assert_eq!(v, json!({ "ok": true }));
        assert_eq!(titles(&listed(&st, "", "").await), ["Stays"]);
        assert_eq!(get(State(st.clone()), Path(gone.id)).await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(delete(State(st.clone()), Path(gone.id)).await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
        // The message it came from stays in the chat.
        assert!(st.db.get_message(&a.id, m.id).unwrap().is_some());
    }

    /// A project folder that's a git repo with this `origin`, or none.
    fn project(st: &AppState, name: &str, origin: Option<&str>) {
        let dir = st.settings.boot().projects_dir.join(name);
        assert!(dir.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        if let Some(o) = origin {
            git(&["remote", "add", "origin", o]);
        }
    }

    fn decoded(put: &Value) -> String {
        use base64::Engine;
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(put["content"].as_str().unwrap()).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn exports_to_docs_as_a_pull_request_and_again_updates_the_same_file() {
        let st = crate::planning::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "site", Some("git@github.com:octo/site.git"));
        let a = agent(&st, "Dev");
        let m = st.db.add_message(&a.id, None, "assistant", "text", "Run `./start.sh`.\n", json!({})).unwrap();
        let i = save(&st, &a.id, m.id, "How to deploy!", "Tutorial").await.unwrap();

        let r = "/repos/octo/site";
        fake.route("GET", r, 200, json!({ "default_branch": "trunk" }));
        fake.route("GET", &format!("{r}/git/ref/heads/trunk"), 200, json!({ "object": { "sha": "trunk-sha" } }));
        fake.route("POST", &format!("{r}/git/refs"), 201, json!({}));
        // Not in the docs yet, then there (the first PR merged).
        fake.in_turn("GET", &format!("{r}/contents/docs/how-to-deploy.md"), vec![(404, json!({ "message": "Not Found" })), (200, json!({ "sha": "file-sha" }))]);
        fake.route("PUT", &format!("{r}/contents/docs/how-to-deploy.md"), 201, json!({}));
        fake.in_turn("POST", &format!("{r}/pulls"), vec![(201, json!({ "number": 7 })), (201, json!({ "number": 8 }))]);
        fake.route("GET", &format!("{r}/pulls/7"), 200, json!({ "number": 7, "title": "Docs: How to deploy!", "state": "open" }));
        fake.route("GET", &format!("{r}/pulls/8"), 200, json!({ "number": 8, "title": "Docs: How to deploy!", "state": "open" }));

        let Json(pr) = export(State(st.clone()), Path(i.id), Json(ExportTo { project: "site".into() })).await.unwrap();
        assert_eq!((pr["slug"].as_str(), pr["number"].as_i64(), pr["url"].as_str()), (Some("octo/site"), Some(7), Some("https://github.com/octo/site/pull/7")));

        // A branch off the default one, the file on it, and the PR from it.
        let refs: Vec<Value> = fake.seen(&format!("{r}/git/refs")).into_iter().map(|s| s.body).collect();
        let branch = refs[0]["ref"].as_str().unwrap().strip_prefix("refs/heads/").unwrap().to_string();
        assert!(branch.starts_with(&format!("docs/saved-{}-", i.id)), "{branch}");
        assert_eq!(refs[0]["sha"], "trunk-sha");
        let puts: Vec<Value> = fake.seen(&format!("{r}/contents/")).into_iter().filter(|s| s.method == "PUT").map(|s| s.body).collect();
        assert_eq!((puts[0]["branch"].as_str(), puts[0].get("sha")), (Some(branch.as_str()), None));
        let doc = decoded(&puts[0]);
        assert!(doc.starts_with("Run `./start.sh`.\n\n---\n\n_From Loft's Saved: "), "{doc}");
        assert!(doc.contains(&format!("saved item {}", i.id)) || doc.contains(&format!("/saved/{})", i.id)), "a line back to the item: {doc}");
        let pulls: Vec<Value> = fake.seen(&format!("{r}/pulls")).into_iter().filter(|s| s.method == "POST").map(|s| s.body).collect();
        assert_eq!(pulls[0], json!({ "title": "Docs: How to deploy!", "head": branch, "base": "trunk", "body": "Adds `docs/how-to-deploy.md` with \"How to deploy!\", kept in Loft's Saved." }));

        // The item's page has the PR, for its card.
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert_eq!((v["pr"]["number"].as_i64(), v["pr"]["state"].as_str()), (Some(7), Some("open")));

        // Again: the same file, updated, in a new PR from a new branch.
        let Json(pr) = export(State(st.clone()), Path(i.id), Json(ExportTo { project: "site".into() })).await.unwrap();
        assert_eq!(pr["number"], 8);
        let puts: Vec<Value> = fake.seen(&format!("{r}/contents/")).into_iter().filter(|s| s.method == "PUT").map(|s| s.body).collect();
        assert_eq!(puts.len(), 2);
        assert_eq!(puts[1]["sha"], "file-sha");
        let pulls: Vec<Value> = fake.seen(&format!("{r}/pulls")).into_iter().filter(|s| s.method == "POST").map(|s| s.body).collect();
        assert!(pulls[1]["body"].as_str().unwrap().starts_with("Updates `docs/how-to-deploy.md`"));
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert_eq!(v["pr"]["number"], 8);
    }

    #[tokio::test]
    async fn a_project_without_a_github_repo_says_so_and_sends_nothing() {
        let st = crate::planning::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "local", None);
        project(&st, "elsewhere", Some("https://gitlab.com/octo/elsewhere.git"));
        let a = agent(&st, "Dev");
        let m = st.db.add_message(&a.id, None, "assistant", "text", "Keep", json!({})).unwrap();
        let i = save(&st, &a.id, m.id, "Kept", "").await.unwrap();
        for (p, says) in [("local", "has no `origin` remote"), ("elsewhere", "isn't on GitHub"), ("nope", "isn't a git repo")] {
            let e = export(State(st.clone()), Path(i.id), Json(ExportTo { project: p.into() })).await.unwrap_err();
            assert_eq!(e.0, axum::http::StatusCode::BAD_REQUEST);
            assert!(e.1.contains(says) && e.1.ends_with("so Loft can't open a pull request there."), "{}", e.1);
        }
        assert!(fake.seen("/").is_empty());
        let Json(v) = get(State(st.clone()), Path(i.id)).await.unwrap();
        assert!(v["pr"].is_null());
    }

    #[test]
    fn the_doc_is_named_after_the_title() {
        let mut i = SavedItem { id: 4, title: "How to: deploy the *backend* (v2)".into(), tag: String::new(), content: String::new(), agent_id: String::new(), agent_name: String::new(), message_id: 0, created_at: String::new(), updated_at: None, export_pr: None };
        assert_eq!(doc_path(&i), "docs/how-to-deploy-the-backend-v2.md");
        i.title = "Ünïcode ✨".into();
        assert_eq!(doc_path(&i), "docs/n-code.md");
        i.title = "✨".into();
        assert_eq!(doc_path(&i), "docs/saved-4.md");
    }

    #[tokio::test]
    async fn a_missing_item_is_not_found() {
        let st = state();
        assert_eq!(get(State(st), Path(42)).await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
    }
}
