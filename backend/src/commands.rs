//! The commands an agent's Claude Code offers: what the CLI reports when a run starts, merged with Loft's own,
//! and each tagged with how it runs.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// What the CLI's `system`/`init` event lists, kept per agent and replaced by every run's.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Reported {
    /// Commands and skills, without the `/`.
    pub commands: Vec<String>,
    /// The ones the CLI runs only in its terminal app.
    pub terminal: Vec<String>,
}

pub fn parse_init(ev: &Value) -> Reported {
    let names = |key: &str| -> Vec<String> { ev[key].as_array().into_iter().flatten().filter_map(|v| v.as_str()).map(String::from).collect() };
    let mut commands = names("slash_commands");
    for s in names("skills") {
        if !commands.contains(&s) {
            commands.push(s);
        }
    }
    Reported { commands, terminal: names("terminal_slash_commands") }
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Command {
    /// With its `/`.
    pub name: String,
    pub description: String,
    /// `passthrough` (sent to the CLI as the prompt), `loft` (Loft runs it itself) or `terminal` (can't run here).
    pub tag: &'static str,
    /// Why a terminal command can't run.
    pub reason: Option<String>,
}

/// Loft's own: they run in Loft, not in the CLI, even where the CLI has one of the same name.
const LOFT: [(&str, &str); 6] = [
    ("clear", "Clear the conversation context and start fresh. Memory is kept."),
    ("compact", "Summarize the conversation so later runs carry less context. Add instructions after it to steer the summary."),
    ("resume", "Go back to one of this agent's past sessions."),
    ("code", "Open this agent's workspace folder in VS Code."),
    ("loop", "Repeat a prompt every interval until stopped: /loop 10m check the deploy. /loop stop ends it."),
    ("grill", "Plan an idea: the agent grills you on it in rounds of questions, each with a recommendation. Add the topic after it."),
];

/// What the CLI's own commands do, for the ones the CLI doesn't describe in its init event.
const BUILT_IN: [(&str, &str); 12] = [
    ("init", "Create a CLAUDE.md for this project."),
    ("mcp", "Show the MCP servers the agent has."),
    ("config", "Show Claude Code's settings."),
    ("context", "Show what fills the context window."),
    ("doctor", "Check the Claude Code installation."),
    ("security-review", "Review the pending changes for security problems."),
    ("code-review", "Review the current diff or a pull request."),
    ("simplify", "Clean up the changed code."),
    ("debug", "Look into a problem in the project."),
    ("model", "Switch the model."),
    ("effort", "Set how hard the model thinks."),
    ("usage", "Show plan usage."),
];

/// The `description:` of the user's own command or skill of that name in the Claude folder, if it has one.
fn user_description(claude_dir: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(claude_dir.join("commands").join(format!("{name}.md")))
        .or_else(|_| std::fs::read_to_string(claude_dir.join("skills").join(name).join("SKILL.md")))
        .ok()?;
    let front = text.strip_prefix("---")?.split("\n---").next()?;
    let line = front.lines().find_map(|l| l.strip_prefix("description:"))?;
    let d = line.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    (!d.is_empty()).then(|| d.to_string())
}

/// Loft's commands first, then the CLI's by name.
pub fn merged(reported: &Reported, claude_dir: &Path) -> Vec<Command> {
    let mut out: Vec<Command> = LOFT
        .iter()
        .map(|(n, d)| Command { name: format!("/{n}"), description: d.to_string(), tag: "loft", reason: None })
        .collect();
    let mut cli: Vec<&String> = reported.commands.iter().chain(&reported.terminal).filter(|n| !LOFT.iter().any(|(l, _)| l == n) && !n.starts_with("__")).collect();
    cli.sort();
    cli.dedup();
    for n in cli {
        let description = user_description(claude_dir, n)
            .or_else(|| BUILT_IN.iter().find(|(b, _)| b == n).map(|(_, d)| d.to_string()))
            .unwrap_or_default();
        let terminal = reported.terminal.contains(n);
        out.push(Command {
            name: format!("/{n}"),
            description,
            tag: if terminal { "terminal" } else { "passthrough" },
            reason: terminal.then(|| "Claude Code runs it only in its own terminal app, not in the headless mode Loft uses.".to_string()),
        });
    }
    out
}

fn file(storage: &Path, agent_id: &str) -> std::path::PathBuf {
    storage.join(crate::agents::AGENT_FILES).join(format!("{agent_id}.commands.json"))
}

pub fn save(storage: &Path, agent_id: &str, reported: &Reported) -> Result<()> {
    crate::agents::write_private(&file(storage, agent_id), &serde_json::to_string(reported)?)
}

/// What the agent's last run reported; nothing before its first run.
pub fn load(storage: &Path, agent_id: &str) -> Reported {
    std::fs::read_to_string(file(storage, agent_id)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Makes the user's own commands and skills (the Claude folder's `commands` and `skills`) visible to the agent's
/// runs as its project's, which `--setting-sources project` loads. Their settings, hooks and plugins stay out:
/// only the workspace's own `.claude/settings.json` is read, and the user's isn't there.
pub fn link_user_commands(workspace: &Path, claude_dir: &Path) {
    for kind in ["commands", "skills"] {
        let (src, dst) = (claude_dir.join(kind), workspace.join(".claude").join(kind));
        if !src.is_dir() {
            continue;
        }
        match std::fs::read_link(&dst) {
            Ok(to) if to == src => continue,
            Ok(_) => {
                let _ = std::fs::remove_file(&dst);
            }
            // Something of the agent's own, or nothing yet.
            Err(_) if dst.exists() => continue,
            Err(_) => {}
        }
        let _ = std::fs::create_dir_all(workspace.join(".claude"));
        let _ = std::os::unix::fs::symlink(&src, &dst);
    }
}

/// `GET /api/agents/{id}/commands`
pub async fn list(axum::extract::State(state): axum::extract::State<crate::state::AppState>, axum::extract::Path(id): axum::extract::Path<String>) -> crate::api::ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(crate::api::not_found)?;
    let reported = load(&state.cfg.storage_dir, &id);
    Ok(axum::Json(json!(merged(&reported, &crate::sessions::claude_dir()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("loft-commands-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn init_gives_commands_and_skills_once_and_the_terminal_only_ones() {
        let ev = json!({ "slash_commands": ["compact", "review", "doctor"], "skills": ["review", "dataviz"], "terminal_slash_commands": ["doctor"] });
        let r = parse_init(&ev);
        assert_eq!(r.commands, ["compact", "review", "doctor", "dataviz"]);
        assert_eq!(r.terminal, ["doctor"]);
        assert_eq!(parse_init(&json!({ "type": "system" })), Reported::default(), "an init without them reports nothing");
    }

    #[test]
    fn loft_commands_come_first_and_win_over_the_clis_and_the_rest_are_tagged() {
        let home = dir("merge");
        std::fs::create_dir_all(home.join("commands")).unwrap();
        std::fs::write(home.join("commands/ship.md"), "---\ndescription: \"Ship the branch\"\n---\nDo it").unwrap();
        let r = Reported { commands: vec!["review".into(), "clear".into(), "ship".into(), "doctor".into(), "__internal".into()], terminal: vec!["doctor".into(), "focus".into()] };
        let list = merged(&r, &home);
        let tags: Vec<_> = list.iter().map(|c| (c.name.as_str(), c.tag)).collect();
        assert_eq!(
            tags,
            [("/clear", "loft"), ("/compact", "loft"), ("/resume", "loft"), ("/code", "loft"), ("/loop", "loft"), ("/grill", "loft"), ("/doctor", "terminal"), ("/focus", "terminal"), ("/review", "passthrough"), ("/ship", "passthrough")],
            "the CLI's internal __ commands are left out"
        );
        let doctor = list.iter().find(|c| c.name == "/doctor").unwrap();
        assert!(doctor.reason.as_deref().unwrap().contains("terminal"));
        assert_eq!(doctor.description, "Check the Claude Code installation.");
        assert_eq!(list.iter().find(|c| c.name == "/ship").unwrap().description, "Ship the branch", "a user's command is described by its file");
        assert!(list.iter().find(|c| c.name == "/review").unwrap().reason.is_none());
    }

    #[test]
    fn a_skill_is_described_by_its_skill_file() {
        let home = dir("skill");
        std::fs::create_dir_all(home.join("skills/b2b")).unwrap();
        std::fs::write(home.join("skills/b2b/SKILL.md"), "---\nname: b2b\ndescription: Test the B2B app\n---\nbody").unwrap();
        assert_eq!(user_description(&home, "b2b").as_deref(), Some("Test the B2B app"));
        assert_eq!(user_description(&home, "none"), None);
    }

    #[test]
    fn reported_commands_are_kept_per_agent() {
        let s = dir("save");
        assert_eq!(load(&s, "a1"), Reported::default());
        let r = Reported { commands: vec!["review".into()], terminal: vec![] };
        save(&s, "a1", &r).unwrap();
        assert_eq!(load(&s, "a1"), r);
        assert_eq!(load(&s, "a2"), Reported::default());
    }

    #[test]
    fn the_users_commands_and_skills_are_linked_into_the_workspace_and_nothing_else() {
        let (home, ws) = (dir("link-home"), dir("link-ws"));
        std::fs::create_dir_all(home.join("commands")).unwrap();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        std::fs::write(home.join("settings.json"), "{\"hooks\":{}}").unwrap();
        link_user_commands(&ws, &home);
        link_user_commands(&ws, &home);
        assert_eq!(std::fs::read_link(ws.join(".claude/commands")).unwrap(), home.join("commands"));
        assert_eq!(std::fs::read_link(ws.join(".claude/skills")).unwrap(), home.join("skills"));
        assert!(!ws.join(".claude/settings.json").exists(), "the user's settings and hooks are not linked");
        let mut seen: Vec<_> = std::fs::read_dir(ws.join(".claude")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        seen.sort();
        assert_eq!(seen, ["commands", "skills"]);
    }

    #[test]
    fn an_agents_own_commands_folder_is_left_alone() {
        let (home, ws) = (dir("own-home"), dir("own-ws"));
        std::fs::create_dir_all(home.join("commands")).unwrap();
        std::fs::create_dir_all(ws.join(".claude/commands")).unwrap();
        link_user_commands(&ws, &home);
        assert!(std::fs::read_link(ws.join(".claude/commands")).is_err());
    }
}
