//! The per-project option "the Lead can start and resume Deliveries" (ADR 0006): off by default, with three
//! limits, each under a fixed ceiling. Only the user changes it, through `PUT /api/projects/lead-delivery`
//! (a user endpoint, ADR 0004); no MCP tool does. Every change is logged: who, when, and the values.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{ApiResult, bad};
use crate::state::AppState;

/// The ceilings: a limit above one is refused, and so is one below 1.
pub const MAX_PER_HOUR: u32 = 6;
pub const MAX_PER_DAY: u32 = 24;
pub const MAX_RUNNING: u32 = 10;

/// How many log lines the endpoint returns.
const LOG_LIMIT: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub enabled: bool,
    /// Resumes the Lead may do on one Delivery in an hour.
    pub resumes_per_hour: u32,
    /// Resumes the Lead may do on one Delivery in a day.
    pub resumes_per_day: u32,
    /// Deliveries running at once in the project.
    pub running_deliveries: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { enabled: false, resumes_per_hour: 2, resumes_per_day: 6, running_deliveries: 2 }
    }
}

/// A project's options; the defaults while it was never changed.
pub fn options(state: &AppState, project: &str) -> Options {
    state.db.lead_delivery_options(project).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Apply a partial change from the user, refusing a value out of range or of the wrong type.
fn apply(mut o: Options, body: &serde_json::Map<String, Value>) -> Result<Options, String> {
    let int = |k: &str, max: u32| -> Result<Option<u32>, String> {
        match body.get(k) {
            None => Ok(None),
            Some(v) => match v.as_u64() {
                Some(n) if (1..=max as u64).contains(&n) => Ok(Some(n as u32)),
                _ => Err(format!("{k} must be a whole number from 1 to {max}")),
            },
        }
    };
    if let Some(v) = body.get("enabled") {
        o.enabled = v.as_bool().ok_or("enabled must be true or false")?;
    }
    if let Some(n) = int("resumes_per_hour", MAX_PER_HOUR)? {
        o.resumes_per_hour = n;
    }
    if let Some(n) = int("resumes_per_day", MAX_PER_DAY)? {
        o.resumes_per_day = n;
    }
    if let Some(n) = int("running_deliveries", MAX_RUNNING)? {
        o.running_deliveries = n;
    }
    Ok(o)
}

fn view(state: &AppState, project: &str) -> Value {
    let log: Vec<Value> = state
        .db
        .lead_delivery_log(project, LOG_LIMIT)
        .unwrap_or_default()
        .into_iter()
        .map(|(at, by, before, after)| {
            let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Null);
            json!({ "at": at, "by": by, "before": parse(&before), "after": parse(&after) })
        })
        .collect();
    json!({
        "project": project,
        "options": options(state, project),
        "ceilings": { "resumes_per_hour": MAX_PER_HOUR, "resumes_per_day": MAX_PER_DAY, "running_deliveries": MAX_RUNNING },
        "log": log,
    })
}

fn project_of(raw: &str) -> Result<String, crate::api::ApiError> {
    match crate::repos::project_name(raw) {
        Ok(p) if !p.is_empty() => Ok(p),
        Ok(_) => Err(bad("project is required")),
        Err(e) => Err(bad(e)),
    }
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    project: String,
}

/// `GET /api/projects/lead-delivery?project=`: the options, their ceilings and the log of changes.
pub async fn get_route(State(state): State<AppState>, Query(q): Query<ProjectQuery>) -> ApiResult<Value> {
    Ok(Json(view(&state, &project_of(&q.project)?)))
}

/// `PUT /api/projects/lead-delivery` with `{ project, enabled?, resumes_per_hour?, resumes_per_day?, running_deliveries? }`.
pub async fn put_route(State(state): State<AppState>, Json(body): Json<Value>) -> ApiResult<Value> {
    let Value::Object(body) = body else { return Err(bad("body must be an object")) };
    let project = project_of(body.get("project").and_then(Value::as_str).unwrap_or(""))?;
    let before = options(&state, &project);
    let after = apply(before.clone(), &body).map_err(bad)?;
    if after != before {
        let (b, a) = (serde_json::to_string(&before).unwrap(), serde_json::to_string(&after).unwrap());
        state.db.save_lead_delivery_options(&project, "user", &b, &a)?;
    }
    Ok(Json(view(&state, &project)))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn put(st: &AppState, body: Value) -> Result<Value, crate::api::ApiError> {
        put_route(State(st.clone()), Json(body)).await.map(|Json(v)| v)
    }

    #[tokio::test]
    async fn a_project_starts_off_with_the_default_limits() {
        let st = crate::tasks::tests::state();
        let Json(v) = get_route(State(st.clone()), Query(ProjectQuery { project: "never-touched".into() })).await.ok().unwrap();
        assert_eq!(v["options"], json!({ "enabled": false, "resumes_per_hour": 2, "resumes_per_day": 6, "running_deliveries": 2 }));
        assert_eq!(v["log"], json!([]));
        assert!(!options(&st, "never-touched").enabled);
    }

    #[tokio::test]
    async fn the_user_turns_it_on_and_adjusts_limits_per_project() {
        let st = crate::tasks::tests::state();
        let v = put(&st, json!({ "project": "loft", "enabled": true, "resumes_per_hour": 3 })).await.ok().unwrap();
        assert_eq!(v["options"], json!({ "enabled": true, "resumes_per_hour": 3, "resumes_per_day": 6, "running_deliveries": 2 }));
        // A later partial change keeps the rest.
        let v = put(&st, json!({ "project": "loft", "running_deliveries": 4 })).await.ok().unwrap();
        assert_eq!(options(&st, "loft"), Options { enabled: true, resumes_per_hour: 3, resumes_per_day: 6, running_deliveries: 4 });
        assert_eq!(v["options"]["running_deliveries"], 4);
        assert!(!options(&st, "other").enabled, "another project is untouched");
        put(&st, json!({ "project": "loft", "enabled": false })).await.ok().unwrap();
        assert!(!options(&st, "loft").enabled);
    }

    #[tokio::test]
    async fn a_value_above_its_ceiling_or_below_one_is_refused_and_changes_nothing() {
        let st = crate::tasks::tests::state();
        let cases = [
            ("resumes_per_hour", json!(MAX_PER_HOUR + 1)),
            ("resumes_per_day", json!(MAX_PER_DAY + 1)),
            ("running_deliveries", json!(MAX_RUNNING + 1)),
            ("resumes_per_hour", json!(0)),
            ("resumes_per_day", json!(-1)),
            ("running_deliveries", json!("3")),
            ("running_deliveries", json!(1.5)),
            ("enabled", json!("yes")),
        ];
        for (k, val) in cases {
            let err = put(&st, json!({ "project": "loft", "enabled": true, k: val })).await.unwrap_err();
            assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST, "{k}");
        }
        assert_eq!(options(&st, "loft"), Options::default(), "a refused change applies nothing, not even the valid part");
        let v = get_route(State(st.clone()), Query(ProjectQuery { project: "loft".into() })).await.ok().unwrap().0;
        assert_eq!(v["log"], json!([]));
        // The ceilings themselves, and 1, are fine.
        put(&st, json!({ "project": "loft", "resumes_per_hour": MAX_PER_HOUR, "resumes_per_day": MAX_PER_DAY, "running_deliveries": 1 })).await.ok().unwrap();
    }

    #[tokio::test]
    async fn the_running_deliveries_ceiling_is_ten() {
        let st = crate::tasks::tests::state();
        assert_eq!(MAX_RUNNING, 10);
        for n in 1..=10 {
            let v = put(&st, json!({ "project": "loft", "running_deliveries": n })).await.ok().unwrap();
            assert_eq!(v["options"]["running_deliveries"], n);
            assert_eq!(v["ceilings"]["running_deliveries"], 10);
        }
        let err = put(&st, json!({ "project": "loft", "running_deliveries": 11 })).await.unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(options(&st, "loft").running_deliveries, 10, "the refused 11 changed nothing");
        assert_eq!(Options::default().running_deliveries, 2, "the default stays 2");
    }

    #[tokio::test]
    async fn every_change_is_logged_with_who_when_and_the_values() {
        let st = crate::tasks::tests::state();
        put(&st, json!({ "project": "loft", "enabled": true })).await.ok().unwrap();
        put(&st, json!({ "project": "loft", "enabled": true })).await.ok().unwrap(); // no change: no line
        let v = put(&st, json!({ "project": "loft", "resumes_per_day": 10 })).await.ok().unwrap();
        let log = v["log"].as_array().unwrap();
        assert_eq!(log.len(), 2, "{log:?}");
        assert_eq!(log[0]["by"], "user");
        assert_eq!(log[0]["before"]["resumes_per_day"], 6);
        assert_eq!(log[0]["after"]["resumes_per_day"], 10);
        assert_eq!(log[1]["before"]["enabled"], false);
        assert_eq!(log[1]["after"]["enabled"], true);
        assert!(chrono::DateTime::parse_from_rfc3339(log[0]["at"].as_str().unwrap()).is_ok());
        // Other projects don't see it.
        let Json(o) = get_route(State(st.clone()), Query(ProjectQuery { project: "other".into() })).await.ok().unwrap();
        assert_eq!(o["log"], json!([]));
    }

    #[tokio::test]
    async fn a_project_name_must_stay_inside_the_projects_folder() {
        let st = crate::tasks::tests::state();
        for p in ["", "../x", "/abs"] {
            assert!(put(&st, json!({ "project": p, "enabled": true })).await.is_err(), "{p:?}");
        }
        assert!(put(&st, json!({ "project": "work/isa", "enabled": true })).await.is_ok());
    }

    #[test]
    fn no_mcp_tool_changes_the_option() {
        let tools = crate::mcp::tools(&crate::tasks::tests::state());
        for t in tools.as_array().unwrap() {
            let text = t.to_string().to_lowercase();
            assert!(!text.contains("lead_delivery") && !text.contains("lead can start"), "{}", t["name"]);
        }
    }
}
