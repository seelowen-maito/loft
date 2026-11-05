//! Groups as stored settings: a group's project list, its main project (where its specs live) and an
//! optional folder whose git repos Loft suggests as projects. Groups are first made from the group labels
//! and projects agents already have, so nothing changes until the user edits one.

use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

use crate::api::{ApiResult, bad};
use crate::db::Group;
use crate::state::AppState;

/// The most projects one group can hold.
const MAX_PROJECTS: usize = 50;

/// Makes a group for every group label in use that has none yet, from its agents as they are: their
/// projects in roster order, and as main project the Lead's main project, else the first agent's. A
/// group already there is never changed.
pub fn seed(state: &AppState) {
    let agents = state.db.list_agents().unwrap_or_default();
    let mut made: Vec<Group> = vec![];
    for a in &agents {
        let name = a.group.trim();
        if name.is_empty() {
            continue;
        }
        let i = match made.iter().position(|g| g.name == name) {
            Some(i) => i,
            None => {
                made.push(Group { name: name.into(), ..Default::default() });
                made.len() - 1
            }
        };
        let g = &mut made[i];
        for p in a.projects.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            if !g.projects.iter().any(|x| x == p) {
                g.projects.push(p.into());
            }
        }
        let main = a.project.trim();
        if !main.is_empty() && (g.main_project.is_empty() || a.role == "lead") {
            g.main_project = main.into();
        }
    }
    for g in made {
        if let Err(e) = state.db.add_group(&g) {
            tracing::warn!("group {}: couldn't save it: {e}", g.name);
        }
    }
}

/// The groups, each with the git repos directly inside its folder that aren't among its projects yet.
pub fn list(state: &AppState) -> Vec<Value> {
    seed(state);
    state.db.list_groups().unwrap_or_default().iter().map(|g| json(state, g)).collect()
}

fn json(state: &AppState, g: &Group) -> Value {
    json!({ "name": g.name, "folder": g.folder, "projects": g.projects, "main_project": g.main_project, "suggestions": suggestions(state, g) })
}

/// The git repos directly inside the group's folder, as projects (`folder/repo`), that the group doesn't
/// have yet, by name. A repo that appears there later shows up here: Loft never adds one by itself.
pub fn suggestions(state: &AppState, g: &Group) -> Vec<String> {
    let Some(dir) = (!g.folder.is_empty()).then(|| crate::repos::project_dir(state, &g.folder)).flatten() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join(".git").exists())
        .filter_map(|e| e.file_name().to_str().map(|n| format!("{}/{n}", g.folder)))
        .filter(|p| !g.projects.contains(p))
        .collect();
    out.sort();
    out
}

/// The group's main project, when it has one: where its specs go when no project is named.
pub fn main_project(state: &AppState, group: &str) -> Option<String> {
    let group = group.trim();
    if group.is_empty() {
        return None;
    }
    seed(state);
    state.db.get_group(group).ok().flatten().map(|g| g.main_project).filter(|m| !m.is_empty())
}

/// `GET /api/groups`.
pub async fn list_route(State(state): State<AppState>) -> ApiResult<Vec<Value>> {
    Ok(Json(list(&state)))
}

/// `PUT /api/groups/{name}`: `{ folder?, projects?, main_project? }`, creating the group if it's new. The
/// main project must be one of its projects; without one it is the first.
pub async fn put_route(State(state): State<AppState>, Path(name): Path<String>, Json(p): Json<Value>) -> ApiResult<Value> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(bad("group must be 1 to 100 characters, with no control characters"));
    }
    seed(&state);
    let mut g = state.db.get_group(&name)?.unwrap_or(Group { name: name.clone(), ..Default::default() });
    if let Some(v) = p.get("folder") {
        let f = crate::repos::project_name(v.as_str().ok_or_else(|| bad("folder must be a string"))?).map_err(|_| bad("folder must be a folder inside the projects folder (no `..`, no absolute path)"))?;
        if !f.is_empty() && crate::repos::project_dir(&state, &f).is_none_or(|d| !d.is_dir()) {
            return Err(bad(format!("There's no folder {f} in the projects folder.")));
        }
        g.folder = f;
    }
    if let Some(v) = p.get("projects") {
        let raw: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| bad("projects must be a list of strings"))?;
        let mut ps: Vec<String> = vec![];
        for r in raw {
            let n = crate::repos::project_name(&r).map_err(bad)?;
            if !n.is_empty() && !ps.contains(&n) {
                ps.push(n);
            }
        }
        if ps.len() > MAX_PROJECTS {
            return Err(bad(format!("a group can have at most {MAX_PROJECTS} projects")));
        }
        g.projects = ps;
    }
    if let Some(v) = p.get("main_project") {
        g.main_project = v.as_str().ok_or_else(|| bad("main_project must be a string"))?.trim().into();
    }
    if !g.main_project.is_empty() && !g.projects.contains(&g.main_project) {
        return Err(bad(format!("The main project must be one of the group's projects: {}.", g.projects.join(", "))));
    }
    if g.main_project.is_empty() {
        g.main_project = g.projects.first().cloned().unwrap_or_default();
    }
    state.db.save_group(&g)?;
    Ok(Json(json(&state, &g)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::tests::state;

    fn agent(st: &AppState, name: &str, group: &str, projects: &[&str], role: &str) {
        let mut a = crate::tasks::tests::agent(st, name);
        (a.group, a.role) = (group.into(), role.into());
        a.projects = projects.iter().map(|p| p.to_string()).collect();
        a.project = a.projects.first().cloned().unwrap_or_default();
        st.db.save_agent(&a).unwrap();
    }

    async fn put(st: &AppState, name: &str, body: Value) -> Result<Value, String> {
        put_route(State(st.clone()), Path(name.into()), Json(body)).await.map(|Json(v)| v).map_err(|e| e.1)
    }

    #[tokio::test]
    async fn groups_are_made_from_todays_labels_and_projects_and_kept_once_edited() {
        let st = state();
        agent(&st, "Web Dev", "Acme", &["web", "shared"], "dev");
        agent(&st, "Acme Lead", "Acme", &["api", "web"], "lead");
        agent(&st, "Solo", "", &["notes"], "dev");
        agent(&st, "Ops", "Ops", &[], "custom");

        let gs = list(&st);
        assert_eq!(gs.len(), 2, "one per label, none for an agent without one: {gs:?}");
        assert_eq!(gs[0], json!({ "name": "Acme", "folder": "", "projects": ["web", "shared", "api"], "main_project": "api", "suggestions": [] }), "the Lead's main project");
        assert_eq!((gs[1]["name"].clone(), gs[1]["projects"].clone(), gs[1]["main_project"].clone()), (json!("Ops"), json!([]), json!("")));

        // Edited, it stays as the user set it, even when agents change.
        put(&st, "Acme", json!({ "projects": ["api", "web"], "main_project": "web" })).await.unwrap();
        agent(&st, "Late", "Acme", &["mobile"], "dev");
        assert_eq!(list(&st)[0]["projects"], json!(["api", "web"]));
        assert_eq!(main_project(&st, "Acme").as_deref(), Some("web"));
        assert_eq!(main_project(&st, "Nobody"), None);
    }

    #[tokio::test]
    async fn a_groups_folder_suggests_its_repos_and_new_ones_only_as_suggestions() {
        let st = state();
        let root = st.settings.boot().projects_dir.join("dine");
        assert!(root.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        for repo in ["api", "pos"] {
            std::fs::create_dir_all(root.join(repo).join(".git")).unwrap();
        }
        std::fs::create_dir_all(root.join("notes")).unwrap();

        let g = put(&st, "Dineout", json!({ "folder": "dine", "projects": ["dine/api"] })).await.unwrap();
        assert_eq!((g["projects"].clone(), g["main_project"].clone(), g["suggestions"].clone()), (json!(["dine/api"]), json!("dine/api"), json!(["dine/pos"])), "a folder that isn't a repo is no suggestion");

        // A repo that appears later is suggested, and never added.
        std::fs::create_dir_all(root.join("b2b").join(".git")).unwrap();
        let g = &list(&st)[0];
        assert_eq!((g["projects"].clone(), g["suggestions"].clone()), (json!(["dine/api"]), json!(["dine/b2b", "dine/pos"])));
    }

    #[tokio::test]
    async fn group_settings_are_validated() {
        let st = state();
        put(&st, "Acme", json!({ "projects": ["api", " web ", "api", ""] })).await.unwrap();
        let before = list(&st);
        assert_eq!(before[0]["projects"], json!(["api", "web"]));
        for (body, err) in [
            (json!({ "main_project": "mobile" }), "The main project must be one of the group's projects: api, web."),
            (json!({ "folder": "missing" }), "There's no folder missing in the projects folder."),
            (json!({ "folder": "../up" }), "folder must be a folder inside the projects folder (no `..`, no absolute path)"),
            (json!({ "projects": ["/abs"] }), "project must be a name, or a folder inside the projects folder (no `..`, no absolute path)"),
        ] {
            assert_eq!(put(&st, "Acme", body).await.unwrap_err(), err);
        }
        assert_eq!(put(&st, "Dine\nout", json!({})).await.unwrap_err(), "group must be 1 to 100 characters, with no control characters");
        assert_eq!(list(&st), before, "nothing changed");
    }
}
