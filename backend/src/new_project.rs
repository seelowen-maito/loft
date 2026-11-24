//! A new Project, started from Repos: Loft makes its empty folder under the projects folder and starts a
//! grill on the chosen Lead for it with the user's idea. The grill is scoped to the new project
//! (`grill::scope`); the Lead's main project stays. Loft writes nothing into the folder: `git init` and the
//! GitHub repo come with the greenfield flow in the grill's first round (`greenfield`). User-only (ADR 0004): agents get no tool for it.

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiError, ApiResult};
use crate::db::Agent;
use crate::grill::NewProject;
use crate::state::AppState;

const MAX_NAME: usize = 100;
const MAX_IDEA: usize = 500;
pub const NO_LEAD: &str = "No Lead yet. A Lead plans new projects with you. Create one, then come back.";

/// The name as typed, fixed the way the form fixes it as you type: lower case, and anything GitHub's repo
/// names don't allow (spaces included) becomes one `-`, trimmed at both ends. `On Call` → `on-call`.
pub fn fix_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out.trim_matches('-').to_string()
}

/// Why a fixed name can't be a project folder, if it can't: what the fix can't repair.
pub fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("Give it a name: letters, numbers, dots, dashes or underscores.")
    } else if name.starts_with('.') {
        Some("A name can't start with a dot.")
    } else if name.contains("..") {
        Some("A name can't contain \"..\".")
    } else if name.chars().count() > MAX_NAME {
        Some("Keep the name under 100 characters.")
    } else {
        None
    }
}

/// The projects folder, as the user knows it: `~/projects` under their home.
fn shown_root(root: &Path) -> String {
    match dirs::home_dir().and_then(|h| root.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => root.display().to_string(),
    }
}

fn shown(state: &AppState, name: &str) -> String {
    format!("{}/{name}", shown_root(&state.settings.boot().projects_dir).trim_end_matches('/'))
}

fn taken_message(state: &AppState, name: &str) -> String {
    format!("{} already exists. Pick another name, or open it from Repos.", shown(state, name))
}

/// Anything at all, even a dangling link or a file, takes the name.
fn exists(state: &AppState, name: &str) -> bool {
    state.settings.boot().projects_dir.join(name).symlink_metadata().is_ok()
}

/// The problem with `name` (already fixed) for a new project: invalid or taken.
fn problem(state: &AppState, name: &str) -> Option<(StatusCode, &'static str, String)> {
    if let Some(p) = name_problem(name) {
        return Some((StatusCode::BAD_REQUEST, "invalid", p.into()));
    }
    exists(state, name).then(|| (StatusCode::CONFLICT, "taken", taken_message(state, name)))
}

#[derive(Deserialize)]
pub struct CheckQuery {
    #[serde(default)]
    name: String,
}

/// `GET /api/projects/check?name=`: the fixed name, where it would go, and why it can't, as the form types.
pub async fn check(State(state): State<AppState>, Query(q): Query<CheckQuery>) -> Json<Value> {
    let name = fix_name(&q.name);
    let problem = problem(&state, &name).map(|(_, kind, message)| json!({ "kind": kind, "message": message }));
    Json(json!({ "name": name, "path": shown(&state, &name), "problem": problem }))
}

#[derive(Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    idea: String,
    /// The Lead who plans it; may be left out when there's only one.
    #[serde(default)]
    lead_id: Option<String>,
}

/// The Lead to give it to: the one named, or the only one. Leads come in team order.
fn pick_lead(state: &AppState, id: Option<&str>) -> Result<Agent, ApiError> {
    let leads: Vec<Agent> = state.db.list_agents()?.into_iter().filter(|a| a.role == "lead").collect();
    if leads.is_empty() {
        return Err(ApiError(StatusCode::CONFLICT, NO_LEAD.into()));
    }
    match id.map(str::trim).filter(|i| !i.is_empty()) {
        Some(id) => leads.into_iter().find(|a| a.id == id).ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "That agent isn't a Lead any more. Pick another one.".into())),
        None if leads.len() == 1 => Ok(leads.into_iter().next().unwrap()),
        None => Err(ApiError(StatusCode::BAD_REQUEST, "Pick the Lead who plans it.".into())),
    }
}

/// Makes the empty folder for `name` (fixed and checked), refusing one outside the projects folder.
fn make_dir(state: &AppState, name: &str) -> Result<PathBuf, ApiError> {
    let root = state.settings.boot().projects_dir.clone();
    let root = root.canonicalize().map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("the projects folder {} can't be read ({e})", root.display())))?;
    let dir = root.join(name);
    let workspaces = state.cfg.storage_dir.canonicalize().unwrap_or_else(|_| state.cfg.storage_dir.clone());
    if dir.parent() != Some(root.as_path()) || dir.starts_with(&workspaces) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "That name points outside the projects folder.".into()));
    }
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(ApiError(StatusCode::CONFLICT, taken_message(state, name))),
        Err(e) => return Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("the folder couldn't be made ({e})"))),
    }
    // What Loft resolves for the project from now on must be this very folder.
    if crate::repos::project_dir(state, name).as_deref() != Some(dir.as_path()) {
        let _ = std::fs::remove_dir(&dir);
        return Err(ApiError(StatusCode::BAD_REQUEST, "That name points outside the projects folder.".into()));
    }
    Ok(dir)
}

/// What `POST /api/projects` does, short of waking the Lead: the response and the wake prompt.
pub(crate) fn create_project(state: &AppState, b: &CreateBody) -> Result<(Value, Agent, Option<String>), ApiError> {
    let name = fix_name(&b.name);
    let idea = b.idea.trim().chars().take(MAX_IDEA).collect::<String>();
    // Everything is checked before anything is made.
    if let Some((code, _, message)) = problem(state, &name) {
        return Err(ApiError(code, message));
    }
    if idea.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Say what it is, in a sentence.".into()));
    }
    let lead = pick_lead(state, b.lead_id.as_deref())?;
    let dir = make_dir(state, &name)?;

    // The Lead's main project stays: the grill is scoped to the new one (`grill::scope`). The Lead gets
    // it as one more project, last, so it can work there after the grill; a Lead with no projects at all
    // isn't limited to any, so it needs none.
    let before = lead.clone();
    let mut lead = lead;
    if lead.projects.is_empty() && !lead.project.trim().is_empty() {
        lead.projects.push(lead.project.trim().to_string());
    }
    if !lead.projects.is_empty() && !lead.has_project(&name) {
        lead.projects.push(name.clone());
    }
    let undo = |state: &AppState| {
        let _ = state.db.save_agent(&before);
        crate::agents::emit_agent(state, &before);
        let _ = std::fs::remove_dir(&dir);
    };
    if let Err(e) = state.db.save_agent(&lead) {
        undo(state);
        return Err(e.into());
    }
    crate::agents::emit_agent(state, &lead);

    let project = NewProject { name: name.clone(), path: shown(state, &name) };
    let text = format!("New project {name}: {idea}");
    let wake = match crate::grill::start_for_project(state, &lead, &text, project.clone()) {
        Ok(w) => w,
        Err(e) => {
            undo(state);
            return Err(e);
        }
    };
    let session = crate::grill::active(state, &lead.id).map(|s| s.id);
    Ok((json!({ "ok": true, "project": project, "agent_id": lead.id, "session_id": session }), lead, wake))
}

/// `POST /api/projects`: creates the folder and starts the Lead's grill on it.
pub async fn create(State(state): State<AppState>, Json(b): Json<CreateBody>) -> ApiResult<Value> {
    let (out, lead, wake) = create_project(&state, &b)?;
    if let Some(p) = wake {
        crate::agents::wake(&state, &lead.id, p, "grill");
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::{agent, state};

    fn lead(st: &AppState, name: &str, team: &str, projects: &[&str]) -> Agent {
        let mut a = agent(st, name);
        a.role = "lead".into();
        a.group = team.into();
        a.projects = projects.iter().map(|p| p.to_string()).collect();
        a.project = a.projects.first().cloned().unwrap_or_default();
        st.db.save_agent(&a).unwrap();
        a
    }

    fn body(name: &str, idea: &str, lead: Option<&str>) -> CreateBody {
        CreateBody { name: name.into(), idea: idea.into(), lead_id: lead.map(String::from) }
    }

    fn root(st: &AppState) -> PathBuf {
        st.settings.boot().projects_dir.clone()
    }

    /// Nothing new in the projects folder.
    fn listing(st: &AppState) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(root(st)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn names_are_fixed_as_typed() {
        assert_eq!(fix_name("On Call"), "on-call");
        assert_eq!(fix_name("  My  App!! "), "my-app");
        assert_eq!(fix_name("api_v2.web"), "api_v2.web");
        assert_eq!(fix_name("a/b"), "a-b");
        assert_eq!(fix_name("../etc"), "..-etc");
        assert_eq!(fix_name("-x-"), "x");
    }

    #[test]
    fn only_what_the_fix_cant_repair_is_invalid() {
        assert!(name_problem("on-call").is_none());
        for bad in ["", ".", "..", ".hidden", "a..b", &"x".repeat(101)] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn it_creates_the_folder_and_starts_the_grill_without_moving_the_main_project() {
        let st = state();
        let l = lead(&st, "Lofty", "Acme", &["api"]);
        let (out, got, wake) = create_project(&st, &body("On Call", "A pager rota for the team", Some(&l.id))).unwrap();
        let dir = root(&st).join("on-call");
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing is written into the folder");
        assert_eq!(out["project"]["name"], "on-call");
        assert!(out["project"]["path"].as_str().unwrap().ends_with("/on-call"));
        assert_eq!(out["agent_id"], l.id);

        // The Lead's main project is unchanged; the new one is added last.
        let saved = st.db.get_agent(&l.id).unwrap().unwrap();
        assert_eq!(saved.project, "api");
        assert_eq!(saved.projects, vec!["api".to_string(), "on-call".into()]);
        assert_eq!(got.project, "api");

        let s = crate::grill::active(&st, &l.id).unwrap();
        assert_eq!(out["session_id"], s.id);
        assert_eq!(s.status, "preparing");
        assert!(s.topic.contains("on-call") && s.topic.contains("pager rota"), "{}", s.topic);
        assert_eq!(s.project.as_ref().unwrap().name, "on-call");
        assert_eq!(crate::grill::view(&s)["project"]["name"], "on-call");
        // No repo yet, so the first round offers to create it.
        let w = wake.unwrap();
        assert!(w.contains("A pager rota") && w.contains("no GitHub repo yet"), "{w}");
        assert!(w.contains("new project `on-call`") && !w.contains("several projects"), "{w}");

        // Greenfield and publishing see the new project while the grill runs, the main project after.
        let scoped = crate::grill::scoped(&st, &saved);
        assert_eq!((scoped.project.as_str(), scoped.projects.as_slice()), ("on-call", &["on-call".to_string()][..]));
        assert!(crate::greenfield::needs_repo(&st, &scoped));
        assert!(!crate::greenfield::needs_repo(&st, &saved), "the main project isn't touched");
        assert_eq!(crate::planning::for_project(&scoped, &json!({})).unwrap().project, "on-call");
        crate::grill::mcp_call(&st, &saved, "grill_end", &json!({ "summary": "Planned." })).unwrap().unwrap();
        assert_eq!(crate::grill::scoped(&st, &saved).project, "api");
    }

    #[test]
    fn a_grill_started_normally_uses_the_main_project() {
        let st = state();
        let l = lead(&st, "Lofty", "Acme", &["api", "web"]);
        std::fs::create_dir(root(&st).join("api")).unwrap();
        crate::grill::on_user_message(&st, &l, "/grill the login flow", Some(&json!({ "role": "start_request" })), json!({})).unwrap();
        let s = crate::grill::active(&st, &l.id).unwrap();
        assert!(s.project.is_none());
        let scoped = crate::grill::scoped(&st, &l);
        assert_eq!((scoped.project.as_str(), scoped.projects.len()), ("api", 2));
    }

    #[test]
    fn a_lead_without_projects_keeps_none() {
        let st = state();
        let l = lead(&st, "Lofty", "Acme", &[]);
        create_project(&st, &body("fresh", "idea", None)).unwrap();
        let saved = st.db.get_agent(&l.id).unwrap().unwrap();
        assert_eq!((saved.project.as_str(), saved.projects.len()), ("", 0));
        assert_eq!(crate::grill::scoped(&st, &saved).project, "fresh");
    }

    #[test]
    fn with_one_lead_it_may_be_left_out() {
        let st = state();
        let l = lead(&st, "Lofty", "Acme", &[]);
        let (out, ..) = create_project(&st, &body("solo", "Just one", None)).unwrap();
        assert_eq!(out["agent_id"], l.id);
    }

    #[test]
    fn it_refuses_and_creates_nothing() {
        let st = state();
        let l = lead(&st, "Lofty", "Acme", &["api"]);
        std::fs::create_dir(root(&st).join("taken")).unwrap();
        let before = listing(&st);
        let code = |b: CreateBody| create_project(&st, &b).unwrap_err();

        for name in ["..", ".hidden", "a..b", "", "!!!"] {
            assert_eq!(code(body(name, "idea", Some(&l.id))).0, StatusCode::BAD_REQUEST, "{name}");
        }
        let e = code(body("Taken", "idea", Some(&l.id)));
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(e.1.ends_with("/taken already exists. Pick another name, or open it from Repos."), "{}", e.1);
        assert_eq!(code(body("fine", "   ", Some(&l.id))).0, StatusCode::BAD_REQUEST, "no idea");
        assert_eq!(code(body("fine", "idea", Some("nope"))).0, StatusCode::BAD_REQUEST, "not a Lead");
        let dev = agent(&st, "Dev");
        assert_eq!(code(body("fine", "idea", Some(&dev.id))).0, StatusCode::BAD_REQUEST, "a Dev isn't a Lead");

        assert_eq!(listing(&st), before);
        assert_eq!(st.db.get_agent(&l.id).unwrap().unwrap().projects, vec!["api".to_string()]);
        assert!(crate::grill::active(&st, &l.id).is_none());
    }

    #[test]
    fn a_link_out_of_the_projects_folder_is_taken_not_followed() {
        let st = state();
        lead(&st, "Lofty", "Acme", &[]);
        let outside = st.cfg.storage_dir.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root(&st).join("escape")).unwrap();
        assert_eq!(create_project(&st, &body("escape", "idea", None)).unwrap_err().0, StatusCode::CONFLICT);
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[test]
    fn with_no_lead_it_says_so() {
        let st = state();
        agent(&st, "Custom");
        let e = create_project(&st, &body("fine", "idea", None)).unwrap_err();
        assert_eq!((e.0, e.1.as_str()), (StatusCode::CONFLICT, NO_LEAD));
        assert!(!root(&st).join("fine").exists());
    }

    #[test]
    fn several_leads_need_one_named() {
        let st = state();
        lead(&st, "A", "Acme", &[]);
        let b = lead(&st, "B", "Beta", &[]);
        assert_eq!(create_project(&st, &body("x", "idea", None)).unwrap_err().0, StatusCode::BAD_REQUEST);
        let (out, ..) = create_project(&st, &body("x", "idea", Some(&b.id))).unwrap();
        assert_eq!(out["agent_id"], b.id);
    }

    #[tokio::test]
    async fn check_reports_the_fixed_name_and_its_problem() {
        let st = state();
        std::fs::create_dir(root(&st).join("loft")).unwrap();
        let get = |n: &str| {
            let st = st.clone();
            let n = n.to_string();
            async move { check(State(st), Query(CheckQuery { name: n })).await.0 }
        };
        let v = get("On Call").await;
        assert_eq!((v["name"].as_str(), v["problem"].is_null()), (Some("on-call"), true));
        assert_eq!(get("Loft").await["problem"]["kind"], "taken");
        assert_eq!(get("..").await["problem"]["kind"], "invalid");
    }
}
