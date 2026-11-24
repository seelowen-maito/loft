//! Project settings: the settings one project keeps in Loft, each starting at Loft's default. The user changes them
//! on the project's own page (`/api/project-settings`), or the project's Lead with `set_project_setting` once the
//! user said yes; each change records who made it and when.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiError, ApiResult, bad};
use crate::db::{Agent, ProjectSetting};
use crate::state::AppState;

/// One setting: what it's called, what it does, the values it takes (each with its label and what it does) and
/// Loft's default.
struct Def {
    key: &'static str,
    label: &'static str,
    help: &'static str,
    choices: &'static [(&'static str, &'static str, &'static str)],
    default: &'static str,
}

pub const CI_MODE: &str = "ci_mode";
/// CI mode: each ticket closes only once its CI is green (the default).
pub const PER_TICKET: &str = "per_ticket";
/// CI mode: tickets close on their reviewers, and the CI is checked once the spec PR is ready.
pub const AT_THE_END: &str = "at_the_end";

const DEFS: &[Def] = &[Def {
    key: CI_MODE,
    label: "CI mode",
    help: "When a Delivery waits for CI. It only changes what Loft waits for, never which workflows GitHub runs.",
    choices: &[
        (PER_TICKET, "Per ticket", "Each ticket closes only once its CI is green."),
        (AT_THE_END, "At the end", "Tickets close on their reviewers; the CI is checked once the spec PR is ready."),
    ],
    default: PER_TICKET,
}];

fn def(key: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.key == key)
}

/// `project`'s value of setting `key`: the one saved, or Loft's default (also for a saved value the setting no
/// longer takes).
fn value_of(state: &AppState, project: &str, key: &str) -> &'static str {
    let d = def(key).expect("a known setting");
    let saved = state.db.project_settings(project).ok().and_then(|mut s| s.remove(key));
    saved.and_then(|s| d.choices.iter().find(|c| s.value == c.0).map(|c| c.0)).unwrap_or(d.default)
}

/// Whether `project`'s CI mode is `at the end`: its Delivery tickets close on their reviewers, without waiting for CI.
pub fn ci_at_the_end(state: &AppState, project: &str) -> bool {
    !project.trim().is_empty() && value_of(state, project, CI_MODE) == AT_THE_END
}

/// Who changed a setting, as the page shows it: the agent (by its name now, or the one it had then once it's
/// gone), or none for the user.
fn changed_json(state: &AppState, s: &ProjectSetting) -> Value {
    let agent = (!s.changed_by.is_empty()).then(|| {
        let name = state.db.get_agent(&s.changed_by).ok().flatten().map_or_else(|| s.changed_by_name.clone(), |a| a.name);
        json!({ "id": s.changed_by, "name": name })
    });
    json!({ "at": s.changed_at, "agent": agent })
}

/// `project`'s settings, each with its choices, Loft's default, its value and, once changed, who changed it and when.
pub fn view(state: &AppState, project: &str) -> Value {
    let saved = state.db.project_settings(project).unwrap_or_default();
    let settings: Vec<Value> = DEFS
        .iter()
        .map(|d| {
            let choices: Vec<Value> = d.choices.iter().map(|(v, l, h)| json!({ "value": v, "label": l, "help": h })).collect();
            json!({
                "key": d.key,
                "label": d.label,
                "help": d.help,
                "choices": choices,
                "default": d.default,
                "value": value_of(state, project, d.key),
                "changed": saved.get(d.key).map(|s| changed_json(state, s)),
            })
        })
        .collect();
    json!({ "project": project.trim(), "settings": settings })
}

/// Sets `project`'s setting `key` to `value`, recording who: agent `by`, or the user with None.
pub fn set(state: &AppState, project: &str, key: &str, value: &Value, by: Option<&Agent>) -> Result<(), String> {
    let d = def(key).ok_or_else(|| format!("There's no project setting `{key}`. The settings are: {}.", DEFS.iter().map(|d| format!("`{}`", d.key)).collect::<Vec<_>>().join(", ")))?;
    let v = value.as_str().map(str::trim).filter(|v| d.choices.iter().any(|c| c.0 == *v));
    let v = v.ok_or_else(|| format!("{} takes {}.", d.label, d.choices.iter().map(|c| format!("`{}`", c.0)).collect::<Vec<_>>().join(" or ")))?;
    let s = ProjectSetting {
        value: json!(v),
        changed_by: by.map(|a| a.id.clone()).unwrap_or_default(),
        changed_by_name: by.map(|a| a.name.clone()).unwrap_or_default(),
        changed_at: crate::db::now(),
    };
    state.db.set_project_setting(project, key, &s).map_err(|e| e.to_string())
}

/// `project` as the page and the API name it, once it's a folder in the projects folder.
fn known_project(state: &AppState, project: &str) -> Result<String, ApiError> {
    let name = crate::repos::project_name(project).map_err(bad)?;
    if name.is_empty() {
        return Err(bad("project is required"));
    }
    match crate::repos::project_problem(state, &name) {
        Some(why) => Err(ApiError(axum::http::StatusCode::NOT_FOUND, format!("{why}."))),
        None => Ok(name),
    }
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    project: String,
}

/// `GET /api/project-settings?project=<name>`: the project's settings.
pub async fn get(State(state): State<AppState>, Query(q): Query<ProjectQuery>) -> ApiResult<Value> {
    let project = known_project(&state, &q.project)?;
    Ok(Json(view(&state, &project)))
}

#[derive(Deserialize)]
pub struct Change {
    project: String,
    key: String,
    value: Value,
}

/// `PUT /api/project-settings`: the user changes one setting of a project. Its settings after the change.
pub async fn put(State(state): State<AppState>, Json(c): Json<Change>) -> ApiResult<Value> {
    let project = known_project(&state, &c.project)?;
    set(&state, &project, &c.key, &c.value, None).map_err(bad)?;
    Ok(Json(view(&state, &project)))
}

// ---- the agents' tools ---------------------------------------------------

pub fn mcp_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "project_settings",
            "description": "Read the Project settings of one of your projects: each setting's value, its choices, Loft's default, and who changed it and when.",
            "inputSchema": { "type": "object", "properties": {
                "project": { "type": "string", "description": "The project; your main project when omitted." }
            } }
        }),
        json!({
            "name": "set_project_setting",
            "description": "As a project's Lead: change one of its Project settings, only once the user said yes to that change. Loft records you as who changed it, and when, on the project's settings page. Settings: `ci_mode` is `per_ticket` (each ticket closes only once its CI is green, the default) or `at_the_end` (tickets close on their reviewers, and the CI is checked once the spec PR is ready); it never changes which workflows GitHub runs.",
            "inputSchema": { "type": "object", "properties": {
                "project": { "type": "string", "description": "The project; your main project when omitted." },
                "key": { "type": "string", "description": "The setting, like `ci_mode`." },
                "value": { "type": "string", "description": "Its new value, like `at_the_end`." }
            }, "required": ["key", "value"] }
        }),
    ]
}

/// The project a tool call names, or the caller's main project; one of the caller's projects either way.
fn project_arg(me: &Agent, args: &Value) -> Result<String, String> {
    let project = args["project"].as_str().map(str::trim).filter(|p| !p.is_empty()).unwrap_or(me.project.trim()).to_string();
    if project.is_empty() {
        return Err("project is required: you have no main project.".into());
    }
    if me.project.trim() != project && !me.has_project(&project) {
        return Err(format!("{project} isn't one of your projects."));
    }
    Ok(project)
}

/// The settings as tool text: one line each, with its value, Loft's default, its choices and who last changed it.
fn tool_text(state: &AppState, project: &str) -> String {
    let saved = state.db.project_settings(project).unwrap_or_default();
    let lines: Vec<String> = DEFS
        .iter()
        .map(|d| {
            let choices = d.choices.iter().map(|(v, _, h)| format!("`{v}` ({})", h.trim_end_matches('.'))).collect::<Vec<_>>().join(", ");
            let changed = saved.get(d.key).map_or(" Never changed.".to_string(), |s| {
                let who = if s.changed_by.is_empty() { "the user".to_string() } else { state.db.get_agent(&s.changed_by).ok().flatten().map_or_else(|| s.changed_by_name.clone(), |a| a.name) };
                format!(" Changed by {who} at {}.", s.changed_at)
            });
            format!("- {} (`{}`): `{}`. Loft's default: `{}`. Choices: {choices}.{changed}", d.label, d.key, value_of(state, project, d.key), d.default)
        })
        .collect();
    format!("Project settings of {project}:\n{}", lines.join("\n"))
}

/// Handle the Project settings tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    match name {
        "project_settings" => Some(project_arg(me, args).map(|p| tool_text(state, &p))),
        "set_project_setting" => Some(set_tool(state, me, args)),
        _ => None,
    }
}

/// `set_project_setting`: the project's Lead changes one setting.
fn set_tool(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    if me.role != "lead" {
        return Err("Only the project's Lead changes its Project settings, once the user said yes.".into());
    }
    let project = project_arg(me, args)?;
    let key = args["key"].as_str().map(str::trim).filter(|k| !k.is_empty()).ok_or("key is required, like `ci_mode`")?;
    set(state, &project, key, &args["value"], Some(me))?;
    Ok(format!("Saved, as changed by you now.\n{}", tool_text(state, &project)))
}
