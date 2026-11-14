//! `/model`, `/effort`, `/cost` and `/status`: Loft's own versions of the CLI's terminal-only commands.
//! Each answers with a notice in the agent's chat.
use axum::extract::{Path, State};
use axum::Json;
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agents::{add_msg, emit_agent};
use crate::api::{bad, not_found, ApiResult};
use crate::state::AppState;

const MODELS: [&str; 4] = ["sonnet", "opus", "fable", "haiku"];
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    arg: String,
}

fn setting(state: &AppState, id: &str, what: &str, arg: &str, choices: &[&str], current: String, set: fn(&mut crate::db::Agent, String)) -> Result<String, crate::api::ApiError> {
    let arg = arg.trim().to_lowercase();
    if arg.is_empty() {
        let now = if current.is_empty() { "default".to_string() } else { current };
        let extra = if what == "Effort" { ", default" } else { "" };
        return Ok(format!("{what}: {now}. Choices: {}{extra}.", choices.join(", ")));
    }
    // Effort has no "default" in the list: it clears the setting.
    let value = if what == "Effort" && arg == "default" { String::new() } else { arg };
    if !value.is_empty() && !choices.contains(&value.as_str()) {
        let extra = if what == "Effort" { ", default" } else { "" };
        return Err(bad(format!("{what} is one of {}{extra}.", choices.join(", "))));
    }
    let a = state.db.update_agent(id, |a| set(a, value.clone()))?.ok_or_else(not_found)?;
    emit_agent(state, &a);
    Ok(format!("{what} set to {}. The next run uses it.", if value.is_empty() { "default" } else { &value }))
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

async fn usage_text(state: &AppState, id: &str, with_plan: bool) -> Result<String, crate::api::ApiError> {
    let a = state.db.get_agent(id)?.ok_or_else(not_found)?;
    let usage = crate::usage::get(state).await;
    let (since, kind) = match crate::usage::usage_week(&usage) {
        Some((start, _)) => (start, "this usage week"),
        None => (Utc::now() - Duration::days(7), "the last 7 days"),
    };
    let (runs, cost) = state.db.spend_between(&since.to_rfc3339(), None)?;
    let tokens = a.context_tokens.map_or("not reported yet".to_string(), |t| if t >= 1000 { format!("{}k tokens in context", t / 1000) } else { format!("{t} tokens in context") });
    let usd = |v: f64| if v > 0.0 && v < 0.01 { "<$0.01".to_string() } else { format!("${v:.2}") };
    let mut out = format!(
        "{}: {tokens}; {} in total, {} this session.\nAll agents, {kind}: {runs} run{}, {}.",
        a.name, usd(a.total_cost_usd), usd(a.session_cost_usd), if runs == 1 { "" } else { "s" }, usd(cost)
    );
    if with_plan {
        match usage["windows"].as_array().filter(|w| !w.is_empty()) {
            Some(ws) => {
                for w in ws {
                    out.push_str(&format!("\nPlan {}: {:.0}% used.", cap(w["label"].as_str().unwrap_or("limit")), w["percent"].as_f64().unwrap_or(0.0)));
                }
            }
            None => out.push_str(&format!("\nPlan limits: {}", usage["error"].as_str().unwrap_or("not available."))),
        }
    }
    Ok(out)
}

pub async fn run(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<Body>) -> ApiResult<Value> {
    let a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let text = match b.name.as_str() {
        "model" => setting(&state, &id, "Model", &b.arg, &MODELS, a.model, |a, v| a.model = v)?,
        "effort" => setting(&state, &id, "Effort", &b.arg, &EFFORTS, a.effort, |a, v| a.effort = v)?,
        "login" => crate::login::text().await,
        "cost" => usage_text(&state, &id, false).await?,
        "status" => usage_text(&state, &id, true).await?,
        _ => return Err(bad("Unknown command.")),
    };
    add_msg(&state, &id, None, "system", "notice", &text, json!({ "command": b.name }));
    Ok(Json(json!({ "text": text })))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(st: &AppState, id: &str, name: &str, arg: &str) -> Result<String, String> {
        run(State(st.clone()), Path(id.into()), Json(Body { name: name.into(), arg: arg.into() }))
            .await
            .map(|j| j.0["text"].as_str().unwrap().to_string())
            .map_err(|e| e.1)
    }

    #[tokio::test]
    async fn model_and_effort_show_set_and_refuse() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        assert!(call(&st, &a.id, "model", "").await.unwrap().contains("Choices: sonnet, opus, fable, haiku"));
        assert!(call(&st, &a.id, "model", "Opus").await.unwrap().contains("set to opus"));
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().model, "opus");
        assert!(call(&st, &a.id, "model", "gpt").await.is_err());
        assert!(call(&st, &a.id, "effort", "").await.unwrap().starts_with("Effort: default.") && call(&st, &a.id, "effort", "").await.unwrap().ends_with("max, default."));
        call(&st, &a.id, "effort", "high").await.unwrap();
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().effort, "high");
        assert!(call(&st, &a.id, "effort", "huge").await.is_err());
        call(&st, &a.id, "effort", "default").await.unwrap();
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().effort, "");
        let notices = st.db.list_messages(&a.id, 50, None).unwrap().iter().filter(|m| m.kind == "notice").count();
        assert_eq!(notices, 6, "each answer is a notice in the chat");
    }

    #[tokio::test]
    async fn cost_and_status_show_usage_and_status_adds_the_plan() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        st.db.update_agent(&a.id, |x| {
            x.context_tokens = Some(41145);
            x.total_cost_usd = 1.5;
        })
        .unwrap();
        crate::usage::fake(&st, json!({ "windows": [{ "key": "five_hour", "label": "session", "percent": 42.0, "resets_at": null }], "error": null })).await;
        let cost = call(&st, &a.id, "cost", "").await.unwrap();
        assert!(cost.contains("0 runs,") && cost.contains("41k tokens in context") && cost.contains("$1.50 in total") && !cost.contains("Plan"), "{cost}");
        let status = call(&st, &a.id, "status", "").await.unwrap();
        st.db.update_agent(&a.id, |x| x.total_cost_usd = 0.003).unwrap();
        assert!(call(&st, &a.id, "cost", "").await.unwrap().contains("<$0.01 in total"));
        assert!(status.contains("Plan Session: 42% used."), "{status}");
        assert!(call(&st, &a.id, "nope", "").await.is_err());
    }
}
