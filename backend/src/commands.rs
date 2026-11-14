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
const BUILT_IN: &[(&str, &str)] = &[
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
    ("deep-research", "Research a question across many sources and report what was found."),
    ("design", "Design a screen or a visual from a description."),
    ("slides", "Make a slide deck."),
    ("design-sync", "Sync a design with its source."),
    ("design-consent", "Allow Claude to use your design files."),
    ("design-revoke", "Take back that design access."),
    ("dataviz", "Make a chart or a dashboard that reads well."),
    ("artifact-diagramming", "Draw a diagram for an artifact."),
    ("artifact-capabilities", "Add live data or saved state to an artifact page."),
    ("whiteboard", "Sketch an idea on a whiteboard."),
    ("update-config", "Change Claude Code's settings, permissions or hooks."),
    ("verify", "Check that a change really works."),
    ("batch", "Run one change across many files in parallel."),
    ("fewer-permission-prompts", "Allow the safe commands you keep approving, so it asks less."),
    ("schedule", "Run a prompt later or on a schedule."),
    ("claude-api", "Help with building on the Claude API."),
    ("workflow-authoring", "Write a multi-agent workflow script."),
    ("run", "Start the project's app to see a change working."),
    ("run-skill-generator", "Write a skill that knows how to launch this project."),
    ("plugin-authoring", "Make or change a Claude Code plugin."),
    ("advisor", "Ask a stronger model for a second opinion."),
    ("agents", "Manage subagents."),
    ("auto-mode-setup", "Set up auto permission mode."),
    ("autocompact", "Set when the context compacts by itself."),
    ("color", "Pick the prompt bar color."),
    ("output-style", "Pick how replies are written."),
    ("fast", "Toggle fast mode."),
    ("focus", "Toggle the focus view."),
    ("heapdump", "Write a memory dump for debugging."),
    ("import", "Import settings or sessions from another tool."),
    ("workflow-launch-exec", "Start a multi-agent workflow."),
    ("reload-plugins", "Reload the plugins."),
    ("reload-skills", "Reload the skills."),
    ("rename", "Rename this session."),
    ("usage-credits", "Show the usage credits left."),
    ("extra-usage", "Turn on extra usage past the plan limit."),
    ("insights", "Show patterns in how you use Claude Code."),
    ("recap", "Summarize what happened in this session."),
    ("skill-doctor", "Check the skills for problems."),
    ("goal", "Set a goal the agent keeps working toward."),
    ("list-agents", "List the running agents."),
    ("team-onboarding", "Write an onboarding guide for the team."),
];

/// The `description:` of the user's own command or skill of that name in the Claude folder, if it has one.
fn user_description(claude_dir: &Path, name: &str) -> Option<String> {
    let name = name.strip_prefix("user:").unwrap_or(name);
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
            .unwrap_or_else(|| "A Claude Code command or skill.".to_string());
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

/// Removes the links an earlier version put in a workspace (`.claude/commands`, `.claude/skills`) to the user's folders.
pub fn unlink_user_commands(workspace: &Path, claude_dir: &Path) {
    for kind in ["commands", "skills"] {
        let dst = workspace.join(".claude").join(kind);
        if std::fs::read_link(&dst).is_ok_and(|to| to == claude_dir.join(kind)) {
            let _ = std::fs::remove_file(&dst);
        }
    }
}

/// The plugin name the user's own commands and skills load under: `/user:ship`.
pub const USER_PLUGIN: &str = "user";

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        // `metadata` follows links, so a linked skill folder is copied as what it points at.
        if std::fs::metadata(e.path())?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Copies the user's own commands and skills (the Claude folder's `commands` and `skills`) into a plugin folder
/// that Loft owns, outside every workspace, and returns it for `--plugin-dir`. A copy, so an agent can't write
/// through to the user's folders; with only these two folders in it, the user's settings, hooks and plugins stay
/// out. Runs read no settings file from the workspace (`--setting-sources ""`), which an agent could write.
pub fn sync_user_plugin(storage: &Path, claude_dir: &Path) -> Option<std::path::PathBuf> {
    let (dir, tmp) = (storage.join("user-plugin"), storage.join(format!("user-plugin.{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&tmp);
    let built = (|| -> std::io::Result<bool> {
        let mut any = false;
        for kind in ["commands", "skills"] {
            if claude_dir.join(kind).is_dir() {
                copy_dir(&claude_dir.join(kind), &tmp.join(kind))?;
                any = true;
            }
        }
        if any {
            std::fs::create_dir_all(tmp.join(".claude-plugin"))?;
            std::fs::write(tmp.join(".claude-plugin/plugin.json"), json!({ "name": USER_PLUGIN, "version": "1.0.0" }).to_string())?;
        }
        Ok(any)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    match built {
        Ok(true) if std::fs::rename(&tmp, &dir).is_ok() => Some(dir),
        _ => {
            let _ = std::fs::remove_dir_all(&tmp);
            None
        }
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
        assert!(list.iter().all(|c| !c.description.is_empty()), "no row is left without a description");
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
    fn the_users_commands_and_skills_are_copied_into_a_plugin_and_nothing_else() {
        let (home, st) = (dir("plug-home"), dir("plug-st"));
        std::fs::create_dir_all(home.join("commands")).unwrap();
        std::fs::create_dir_all(home.join("skills/b2b")).unwrap();
        std::fs::write(home.join("commands/ship.md"), "ship").unwrap();
        std::fs::write(home.join("skills/b2b/SKILL.md"), "b2b").unwrap();
        std::fs::write(home.join("settings.json"), "{\"hooks\":{}}").unwrap();
        std::fs::create_dir_all(home.join("plugins")).unwrap();
        let p = sync_user_plugin(&st, &home).unwrap();
        sync_user_plugin(&st, &home).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("commands/ship.md")).unwrap(), "ship");
        assert_eq!(std::fs::read_to_string(p.join("skills/b2b/SKILL.md")).unwrap(), "b2b");
        assert!(std::fs::symlink_metadata(p.join("commands")).unwrap().is_dir(), "a copy, not a link into the user's folder");
        let mut seen: Vec<_> = std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        seen.sort();
        assert_eq!(seen, [".claude-plugin", "commands", "skills"]);
        // A write through the plugin leaves the user's folder alone.
        std::fs::write(p.join("commands/ship.md"), "changed").unwrap();
        assert_eq!(std::fs::read_to_string(home.join("commands/ship.md")).unwrap(), "ship");
        // Nothing to share: no plugin.
        assert!(sync_user_plugin(&st, &dir("plug-empty")).is_none());
        assert!(!st.join("user-plugin").exists());
        // An old link into the user's folder goes; an agent's own folder stays.
        let ws = dir("plug-ws");
        std::fs::create_dir_all(ws.join(".claude")).unwrap();
        std::os::unix::fs::symlink(home.join("commands"), ws.join(".claude/commands")).unwrap();
        std::fs::create_dir_all(ws.join(".claude/skills")).unwrap();
        unlink_user_commands(&ws, &home);
        assert!(!ws.join(".claude/commands").exists() && ws.join(".claude/skills").is_dir());
    }
}
