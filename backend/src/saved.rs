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

/// One item, and whether the message it came from is still in its agent's chat (`source_exists`).
pub async fn get(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    let item = state.db.get_saved(id)?.ok_or_else(not_found)?;
    let source_exists = state.db.get_message(&item.agent_id, item.message_id)?.is_some();
    let mut v = json!(item);
    v["source_exists"] = json!(source_exists);
    Ok(Json(v))
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

    #[tokio::test]
    async fn a_missing_item_is_not_found() {
        let st = state();
        assert_eq!(get(State(st), Path(42)).await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
    }
}
