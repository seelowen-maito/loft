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
/// Loft's default. A branch setting takes one of the repo's branches instead of fixed choices.
struct Def {
    key: &'static str,
    label: &'static str,
    help: &'static str,
    choices: &'static [(&'static str, &'static str, &'static str)],
    default: &'static str,
    branch: bool,
}

pub const CI_MODE: &str = "ci_mode";
/// CI mode: each ticket closes only once its CI is green (the default).
pub const PER_TICKET: &str = "per_ticket";
/// CI mode: tickets close on their reviewers, and the CI is checked once the spec PR is ready.
pub const AT_THE_END: &str = "at_the_end";
/// The branch a project's next spec PRs and standalone tickets start from and merge into.
pub const BASE_BRANCH: &str = "base_branch";

const DEFS: &[Def] = &[
    Def {
        key: CI_MODE,
        label: "CI mode",
        help: "When a Delivery waits for CI. It only changes what Loft waits for, never which workflows GitHub runs.",
        choices: &[
            (PER_TICKET, "Per ticket", "Each ticket closes only once its CI is green."),
            (AT_THE_END, "At the end", "Tickets close on their reviewers; the CI is checked once the spec PR is ready."),
        ],
        default: PER_TICKET,
        branch: false,
    },
    Def {
        key: BASE_BRANCH,
        label: "Base branch",
        help: "The branch the next spec PRs and standalone tickets start from and merge into. Deliveries already under way keep the base they have.",
        choices: &[],
        // Empty: the repo's default branch on GitHub.
        default: "",
        branch: true,
    },
];

fn def(key: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.key == key)
}

/// `project`'s value of setting `key`: the one saved, or Loft's default (also for a saved value the setting no
/// longer takes).
fn value_of(state: &AppState, project: &str, key: &str) -> String {
    let d = def(key).expect("a known setting");
    let saved = state.db.project_settings(project).ok().and_then(|mut s| s.remove(key));
    let saved = saved.and_then(|s| s.value.as_str().map(str::to_string));
    let taken = |v: &String| if d.branch { v.is_empty() || crate::planning::is_branch_name(v) } else { d.choices.iter().any(|c| c.0 == v) };
    saved.filter(taken).unwrap_or_else(|| d.default.to_string())
}

/// The branch `project`'s next spec PRs and standalone tickets start from and merge into, when it isn't the repo's
/// default branch on GitHub.
pub fn base_branch(state: &AppState, project: &str) -> Option<String> {
    Some(value_of(state, project, BASE_BRANCH)).filter(|b| !project.trim().is_empty() && !b.is_empty())
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
                "kind": if d.branch { "branch" } else { "choice" },
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
    let v = if d.branch {
        let v = value.as_str().map(str::trim).filter(|v| v.is_empty() || crate::planning::is_branch_name(v));
        v.ok_or_else(|| format!("{} takes the name of one of the repo's branches, or an empty one for its default branch on GitHub.", d.label))?
    } else {
        let v = value.as_str().map(str::trim).filter(|v| d.choices.iter().any(|c| c.0 == *v));
        v.ok_or_else(|| format!("{} takes {}.", d.label, d.choices.iter().map(|c| format!("`{}`", c.0)).collect::<Vec<_>>().join(" or ")))?
    };
    let s = ProjectSetting {
        value: json!(v),
        changed_by: by.map(|a| a.id.clone()).unwrap_or_default(),
        changed_by_name: by.map(|a| a.name.clone()).unwrap_or_default(),
        changed_at: crate::db::now(),
    };
    state.db.set_project_setting(project, key, &s).map_err(|e| e.to_string())
}

/// Refuses a branch setting's value that isn't one of the project's branches on GitHub. Anything else, an empty
/// value (the default branch) and a project with no GitHub repo pass: those are checked when saved, or never used.
async fn on_github(state: &AppState, project: &str, key: &str, value: &Value) -> Result<(), String> {
    let branch = value.as_str().map(str::trim).unwrap_or_default();
    if !def(key).is_some_and(|d| d.branch) || branch.is_empty() || !crate::planning::is_branch_name(branch) {
        return Ok(());
    }
    let Ok(slug) = crate::repos::github_repo(state, project).await else { return Ok(()) };
    match crate::github::get(state, &format!("/repos/{slug}/git/ref/heads/{branch}")).await {
        Ok(_) => Ok(()),
        Err(e) if crate::github::is_gone(&e) => Err(format!("`{branch}` isn't a branch of {slug} on GitHub.")),
        Err(e) => Err(format!("Couldn't check that `{branch}` is a branch of {slug}: {e}")),
    }
}

/// `project`'s branches on GitHub, by name, and its default branch: what a branch setting can take.
async fn branches_of(state: &AppState, project: &str) -> Result<Value, String> {
    let slug = crate::repos::github_repo(state, project).await?;
    let repo = crate::github::get(state, &format!("/repos/{slug}")).await.map_err(|e| format!("Couldn't read {slug} on GitHub: {e}"))?;
    let mut names: Vec<String> = vec![];
    // 100 a page, up to 1,000: far more than a repo's base is picked from.
    for page in 1..=10 {
        let got = crate::github::get(state, &format!("/repos/{slug}/branches?per_page=100&page={page}")).await.map_err(|e| format!("Couldn't read the branches of {slug}: {e}"))?;
        let got = got.as_array().cloned().unwrap_or_default();
        names.extend(got.iter().filter_map(|b| b["name"].as_str()).map(str::to_string));
        if got.len() < 100 {
            break;
        }
    }
    names.sort();
    Ok(json!({ "repo": slug, "default_branch": repo["default_branch"], "branches": names }))
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

/// `GET /api/project-settings/branches?project=<name>`: the project's branches on GitHub, for its base branch.
pub async fn branches(State(state): State<AppState>, Query(q): Query<ProjectQuery>) -> ApiResult<Value> {
    let project = known_project(&state, &q.project)?;
    Ok(Json(branches_of(&state, &project).await.map_err(bad)?))
}

/// `PUT /api/project-settings`: the user changes one setting of a project. Its settings after the change.
pub async fn put(State(state): State<AppState>, Json(c): Json<Change>) -> ApiResult<Value> {
    let project = known_project(&state, &c.project)?;
    on_github(&state, &project, &c.key, &c.value).await.map_err(bad)?;
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
            "description": "As a project's Lead: change one of its Project settings, only once the user said yes to that change. Loft records you as who changed it, and when, on the project's settings page. Settings: `ci_mode` is `per_ticket` (each ticket closes only once its CI is green, the default) or `at_the_end` (tickets close on their reviewers, and the CI is checked once the spec PR is ready); it never changes which workflows GitHub runs. `base_branch` is one of the repo's branches, the one the next spec PRs and standalone tickets start from and merge into, or empty for the repo's default branch on GitHub (the default); Deliveries already under way keep theirs.",
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
            let choices = if d.branch {
                "one of the repo's branches, or empty for its default branch on GitHub".to_string()
            } else {
                d.choices.iter().map(|(v, _, h)| format!("`{v}` ({})", h.trim_end_matches('.'))).collect::<Vec<_>>().join(", ")
            };
            let changed = saved.get(d.key).map_or(" Never changed.".to_string(), |s| {
                let who = if s.changed_by.is_empty() { "the user".to_string() } else { state.db.get_agent(&s.changed_by).ok().flatten().map_or_else(|| s.changed_by_name.clone(), |a| a.name) };
                format!(" Changed by {who} at {}.", s.changed_at)
            });
            let shown = |v: &str| if d.branch && v.is_empty() { "`` (the repo's default branch on GitHub)".to_string() } else { format!("`{v}`") };
            format!("- {} (`{}`): {}. Loft's default: {}. Choices: {choices}.{changed}", d.label, d.key, shown(&value_of(state, project, d.key)), shown(d.default))
        })
        .collect();
    format!("Project settings of {project}:\n{}", lines.join("\n"))
}

/// Handle the Project settings tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    match name {
        "project_settings" => Some(project_arg(me, args).map(|p| tool_text(state, &p))),
        "set_project_setting" => Some(set_tool(state, me, args).await),
        _ => None,
    }
}

/// `set_project_setting`: the project's Lead changes one setting.
async fn set_tool(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    if me.role != "lead" {
        return Err("Only the project's Lead changes its Project settings, once the user said yes.".into());
    }
    let project = project_arg(me, args)?;
    let key = args["key"].as_str().map(str::trim).filter(|k| !k.is_empty()).ok_or("key is required, like `ci_mode`")?;
    on_github(state, &project, key, &args["value"]).await?;
    set(state, &project, key, &args["value"], Some(me))?;
    Ok(format!("Saved, as changed by you now.\n{}", tool_text(state, &project)))
}
