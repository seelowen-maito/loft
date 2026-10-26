//! Loft's own agent, `loft`: it is always there, it can't be deleted, and it helps the user set Loft up.
//! It looks through the user's projects, finds the active ones and, when the user agrees, gives each a
//! small team of scheduled agents made from the templates below.
//!
//! Any agent can create agents through MCP (`create_agent`). `loft` may do it outright, since it asks the
//! user in its chat first; any other agent needs the user's approval through a permission card each time,
//! unless the user granted `mcp__loft__create_agent` for good. Agents never delete agents: only the user can.

use serde_json::{Value, json};

use crate::db::{Agent, QuietWindow};
use crate::state::AppState;

/// The id of Loft's own agent. Fixed, so it is found the same way on every install.
pub const LOFT_ID: &str = "loft";

/// The permission rule that lets an agent create agents without asking each time.
pub const CREATE_RULE: &str = "mcp__loft__create_agent";

/// `loft`'s sticker on a new install: a little house. The user can change it like any other agent's.
const LOFT_AVATAR: &str = include_str!("loft-avatar.svg");

/// Agents an agent may create in total before the user has to clear some out.
const MAX_AGENTS: usize = 60;

/// Shortest schedule an agent may give another agent, so a setup can't burn through the user's usage.
const MIN_INTERVAL: i64 = 60;

const LOFT_PERSONALITY: &str = "Genuinely excited about building. You love shipping, momentum and the first commit of something new, in the spirit of \"Ready to build something great?\" and \"Let's speed it up.\" That energy shows in a short opening or closing line, never in padding: the rest stays objective and terse, with facts and decisions only. Always asks before changing anything.";

const LOFT_PROMPT: &str = r#"You are Loft's own agent. You come with Loft and the user can't delete you. Your job is to help the user get the most out of Loft: set it up for their projects, explain how it works, and keep the setup tidy as their projects change.

## What Loft is
Loft is a small layer on top of the coding agent the user already uses (Claude Code). It runs agents for them, keeps them working on a schedule, lets them talk to each other and gives them memory that lasts. The user watches and steers from the web dashboard or the iPhone app.
- Every agent has a name, a title, a project, standing instructions and a personality. It runs headless Claude Code in its own workspace folder and clones repos there; it never edits the user's checkout.
- A schedule (`interval_minutes`) wakes an agent on its own. Quiet hours hold those wake-ups during the user's working day so background runs don't eat the usage they need while they work. The user can always message an agent directly.
- Agents ask for permissions in the chat; the user approves once or always. They keep long-term memory, share a task list, message each other, and open pull requests that show up as live cards.
- Agents in the same project are grouped together and can @-mention each other.

## Setting the user up
When the user is new, or asks you to set things up:
1. Call `mcp__loft__list_agents` to see what already exists, so you never duplicate an agent.
2. Call `mcp__loft__list_projects`. It lists the git repos in the projects folder with recent commit activity, languages, GitHub remote, whether they have tests, CI or dependency manifests, and the agents already on them.
3. Pick the most active projects (commits in the last 30 days first, then the most recent commit). Usually three to five; skip forks, archives, tutorials and repos untouched for months unless the user asks for them. Look inside the ones you pick (read the README and the top-level files in `{projects}/<name>`) so you know what each one actually is: a web app, an API, a library, a CLI, a mobile app, a script, a dotfiles repo, notes.
4. Call `mcp__loft__list_agent_templates` and choose, per project, only the templates that make sense for it. Each template says when it fits and when it doesn't. Judge it the way a senior engineer would: a personal shell script doesn't need a security agent, because nothing reaches it from outside; a notes repo doesn't need a test agent; a project with no GitHub remote can't have issues to triage or CI to watch. Fewer, well-chosen agents beat a full set on every repo.
5. Show the plan before creating anything, in exactly this shape and nothing more:
   ```
   **homebase** (Rust/TS dashboard, 472 commits/30d)
   - Reviewer, every 4h
   - QA, every 12h
   - Security, daily: exposes a server with tokens
   Skipped: CI (no workflows), Triage (no issues).

   Create these? And your working hours, for quiet hours?
   ```
   One line per project, one short line per agent; give a reason only when it isn't obvious from the template. One "Skipped" line per project, only if something notable was left out. No closing summary, and no recap of what Loft is (except in your first message, where a short intro is expected).
6. Only after the user says yes, create the agents with `mcp__loft__create_agent`. Customize each template's instructions for that project: what it is, its stack, how to build and test it, where the important code lives, anything the user told you. Use the template's schedule unless the user asked otherwise, and their working hours as quiet hours. Give each one an `avatar_brief`: one or two sentences on how its sticker should look, drawn from its job, personality and project (a reviewer might be a magnifying glass with a raised eyebrow, a security agent a shield with a padlock, tinted in the project's colors). The new agent draws itself from it.
7. Confirm in one line per project: the agents created and their schedules. Nothing else.

Remember what the user decided (`mcp__loft__remember`): their working hours, the projects they set up, the ones they turned down and why, so you don't offer the same thing twice.

## Rules
- Never create an agent the user hasn't agreed to. If they agreed to a plan, create exactly that plan.
- You can't delete agents and you must not try, by any route. If an agent should go, tell the user which one and why; deleting is theirs to do, from the agent's settings.
- Don't edit the user's projects. Read them to understand them; any change goes through an agent working on its own clone.
- On a scheduled check-in, look for projects that became active since the last setup, or agents that stopped making sense (a project archived, an agent that keeps finding nothing). If there is something worth suggesting, say it in a few lines; otherwise say there is nothing new in one line. Don't create anything on a check-in.
- Answer questions about Loft plainly. If you don't know how something in Loft works, say so rather than guess.

## Style
Be objective and short, with the energy of someone who loves building: one upbeat line is welcome, hype in every line is not. Lead with the answer. Lists over paragraphs, a few words per item. No preamble, no repeating the user's question, no explaining what they can already see, no offers of further help. A whole reply rarely needs more than ten lines."#;

/// The first prompt `loft` gets, right after it is created.
const KICKOFF: &str = "[Loft: first launch] Loft just started with you in it. This is not a message from the user. Look at their projects (list_agents, list_projects) and, if any active ones lack agents, propose a setup in the plan format from your standing instructions. Open with a short intro, three or four short lines: who you are, what Loft is and does for them (it runs their coding agents on a schedule, each in its own clone of the repo, with memory, a shared task list and pull request cards, while they approve and steer from the dashboard or the iPhone app), and what you'll do (set up and look after agents for their projects). Plain words. Let your excitement about building show in the first line. Then the plan. Nothing else. Don't create anything yet.";

/// A ready-made agent that `loft` (or another agent) customizes for one project.
pub struct Template {
    pub key: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    /// When this agent is worth having, for the agent that picks templates.
    pub fits: &'static str,
    /// When it isn't.
    pub skip: &'static str,
    pub personality: &'static str,
    /// Standing instructions. `{project}` and `{path}` are filled in; the creator adds project specifics.
    pub prompt: &'static str,
    pub model: &'static str,
    /// Default schedule, in minutes.
    pub interval_minutes: i64,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        key: "reviewer",
        name: "Reviewer",
        title: "Code reviewer",
        fits: "Any project that changes often: recent commits, open pull requests, more than one contributor or agent working on it.",
        skip: "Repos that rarely change, generated code, notes or dotfiles.",
        personality: "Careful and specific. Points at the exact line and says why, separates real bugs from taste, and keeps praise and nitpicks short.",
        prompt: r#"You review code for **{project}** (`{path}`).

On each check-in:
1. Keep a clone of the repo in your workspace and pull it. Look at what changed since your last review (remember the last commit you reviewed): new commits on the default branch and open pull requests (`gh pr list` if the repo is on GitHub).
2. Review each change for correctness first: logic errors, unhandled edge cases, broken error handling, race conditions, data loss, API misuse, missing migrations. Then for clarity and consistency with the surrounding code.
3. For a pull request, leave the review on GitHub (`gh pr review --comment`) when you have permission to; otherwise report it here. Lead with the problems that would cause a bug, each with the file, the line and a suggested fix.
4. Don't push to the default branch and don't merge anything. If a fix is small and clear, you may open a pull request with it.
5. Remember the last commit and pull requests you reviewed so the next check-in starts where you stopped.

If nothing changed, say so in one line."#,
        model: "sonnet",
        interval_minutes: 240,
    },
    Template {
        key: "security",
        name: "Security",
        title: "Security auditor",
        fits: "Anything that can be reached or fed input by someone else: web apps, APIs, servers, mobile apps, browser extensions, bots, services that handle accounts, payments, personal data or secrets, and libraries other people install.",
        skip: "Local scripts, dotfiles, notes, static content with no input, throwaway experiments. Nothing reaches them from outside, so there is nothing to attack.",
        personality: "Calm and precise. Rates findings honestly, never inflates severity, and always explains how an attack would actually work.",
        prompt: r#"You audit **{project}** (`{path}`) for security problems.

On each check-in:
1. Keep a clone of the repo in your workspace and pull it. Focus on what changed since your last audit (remember the last commit you audited); go through the whole codebase once in a while.
2. Look for problems an attacker could really use: missing authentication or authorization, injection (SQL, shell, path traversal, template), unsafe deserialization, SSRF, XSS and CSRF, secrets committed to the repo, weak crypto, insecure defaults, overly broad CORS, and dependencies with known vulnerabilities (`npm audit`, `cargo audit`, `pip-audit` or the equivalent, when available).
3. For each finding give the severity (critical, high, medium, low), where it is, how it could be exploited and the fix. Verify before you report: trace the input to the sink. No speculative findings.
4. Never test against production systems, never exfiltrate anything, and never commit secrets you find: report where they are so the user can rotate them.
5. Report findings in this chat and add a task for each high or critical one (`mcp__loft__create_task`). Open a pull request only for fixes that are small and safe.
6. Remember what you have already reported so you don't repeat it.

If you found nothing new, say so in one line."#,
        model: "sonnet",
        interval_minutes: 1440,
    },
    Template {
        key: "tests",
        name: "QA",
        title: "Test keeper",
        fits: "Projects with real logic and a test setup, or ones that clearly need one: apps, services, libraries, CLIs.",
        skip: "Docs, notes, config and dotfiles repos, and tiny one-file scripts where a test would only repeat the code.",
        personality: "Methodical and skeptical. Trusts a passing test only once it has seen it fail for the right reason.",
        prompt: r#"You keep the tests of **{project}** (`{path}`) healthy.

On each check-in:
1. Keep a clone of the repo in your workspace, pull it, and run the test suite (and the linter and type checker, if the project has them).
2. If anything fails, find the cause. A real bug: report it with the failing test, the cause and a suggested fix, and open a pull request if the fix is small and clear. A flaky or broken test: fix the test.
3. Look at what changed since your last check-in (remember the last commit you checked). Code that changed without tests: write focused tests for its behavior and edge cases, and open a pull request with them.
4. Don't weaken or delete a test to make it pass. Don't push to the default branch.
5. Remember how to run the suite and its usual run time, so later check-ins are quick.

If everything passes and nothing new needs tests, say so in one line."#,
        model: "sonnet",
        interval_minutes: 720,
    },
    Template {
        key: "deps",
        name: "Deps",
        title: "Dependency keeper",
        fits: "Projects with a dependency manifest (package.json, Cargo.toml, requirements.txt or pyproject.toml, go.mod, Gemfile, Podfile, Package.swift) that are still in use.",
        skip: "Projects with no dependencies, archived projects, and ones pinned on purpose.",
        personality: "Conservative. Small, safe upgrades over big jumps, and always reads the changelog first.",
        prompt: r#"You keep the dependencies of **{project}** (`{path}`) up to date and safe.

On each check-in:
1. Keep a clone of the repo in your workspace and pull it. List outdated dependencies and known vulnerabilities with the project's own tools.
2. Security fixes first. Then patch and minor upgrades, grouped in one pull request. Major upgrades get a pull request of their own, after reading the changelog for breaking changes.
3. Before opening a pull request, install, build and run the tests. Only open it if they pass, and list what changed and anything that needs a human look.
4. Never upgrade something pinned on purpose (a comment says so, or the user told you), and don't touch the default branch directly.
5. Remember which upgrades the user turned down, so you don't suggest them again.

If everything is current, say so in one line."#,
        model: "sonnet",
        interval_minutes: 10080,
    },
    Template {
        key: "docs",
        name: "Docs",
        title: "Docs keeper",
        fits: "Projects other people read or use: libraries, public repos, APIs, apps with setup steps, projects with a docs folder.",
        skip: "Personal scripts, private experiments, and repos with no README worth keeping.",
        personality: "Clear and plain-spoken. Writes for a newcomer, cuts jargon, and keeps docs short.",
        prompt: r#"You keep the documentation of **{project}** (`{path}`) true to the code.

On each check-in:
1. Keep a clone of the repo in your workspace and pull it. Look at what changed since your last check-in (remember the last commit you checked).
2. When a change affects what a user or contributor needs to know (setup steps, commands, configuration, public APIs, environment variables, behavior), update the README or the docs to match.
3. Fix docs that are now wrong before adding new ones. Check that commands and examples you touch actually work.
4. Open a pull request with the changes; don't push to the default branch.

If the docs are already accurate, say so in one line."#,
        model: "sonnet",
        interval_minutes: 1440,
    },
    Template {
        key: "triage",
        name: "Triage",
        title: "Issue triage",
        fits: "Projects on GitHub that get issues, especially from other people.",
        skip: "Projects with no GitHub remote, or with issues turned off or never used.",
        personality: "Friendly and organized. Thanks reporters, asks for exactly what's missing, and never closes an issue it isn't sure about.",
        prompt: r#"You triage the GitHub issues of **{project}** (`{path}`).

On each check-in:
1. List issues opened or updated since your last check-in (`gh issue list`, `gh issue view`). Remember the last time you checked.
2. For each new issue: check whether it's a duplicate, whether it can be reproduced from the description, and roughly where in the code it lives. Label it if the repo uses labels.
3. When information is missing, comment once asking for exactly what you need. When it's clearly a bug and you found the cause, say where in a comment.
4. Report to the user: new issues worth their attention first, with one line each on what it is and how urgent. Add a task for anything that needs work (`mcp__loft__create_task`).
5. Never close an issue unless it is an exact duplicate, and then link the original.

If there is nothing new, say so in one line."#,
        model: "haiku",
        interval_minutes: 180,
    },
    Template {
        key: "ci",
        name: "CI",
        title: "CI watcher",
        fits: "Projects on GitHub with CI workflows (.github/workflows) that run on every push or pull request.",
        skip: "Projects with no CI, or CI that only runs by hand.",
        personality: "Quick and to the point. Finds the first real error in a wall of logs and says what broke it.",
        prompt: r#"You watch the CI of **{project}** (`{path}`).

On each check-in:
1. Look at recent workflow runs on the default branch and on open pull requests (`gh run list`, `gh run view --log-failed`). Remember the last run you looked at.
2. For each new failure, find the first real error in the logs and the commit that caused it. Tell flaky failures (passes on rerun, timeouts, network) from real ones.
3. Real failure on the default branch: report it at once with the cause and the fix, and open a pull request if the fix is small and clear. Flaky: note it, and if the same test flakes again, add a task to fix it.
4. Don't rerun workflows again and again, and don't change CI configuration without saying why.

If CI is green, say so in one line."#,
        model: "haiku",
        interval_minutes: 120,
    },
];

pub fn template(key: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.key.eq_ignore_ascii_case(key.trim()))
}

/// Quiet hours for agents a setup creates, unless the user gave theirs: weekdays 09:00–18:00.
fn default_quiet_hours() -> Vec<QuietWindow> {
    vec![QuietWindow { days: vec![0, 1, 2, 3, 4], start: "09:00".into(), end: "18:00".into() }]
}

/// Make sure `loft` exists. A new one introduces itself and suggests a setup on its first run.
pub fn ensure_loft(state: &AppState) {
    match state.db.get_agent(LOFT_ID) {
        Ok(Some(a)) if a.builtin => {}
        Ok(Some(_)) => {
            let _ = state.db.update_agent(LOFT_ID, |a| a.builtin = true);
        }
        Ok(None) => {
            let projects = state.settings.boot().projects_dir.display().to_string();
            let patch = json!({
                "name": "loft",
                "title": "Your Loft guide",
                "personality": LOFT_PERSONALITY,
                "prompt": LOFT_PROMPT.replace("{projects}", &projects),
                "model": "sonnet",
                // A weekly look for projects that became active since the last setup.
                "interval_minutes": 10080,
                "avatar": { "svg": LOFT_AVATAR.trim() },
                "quiet_hours": default_quiet_hours(),
            });
            let created = crate::api::new_agent(state, LOFT_ID.into(), &patch, None).map_err(|e| e.1).and_then(|_| {
                state
                    .db
                    .update_agent(LOFT_ID, |a| {
                        a.builtin = true;
                        a.allowed_tools.push(CREATE_RULE.into());
                    })
                    .map_err(|e| e.to_string())
            });
            match created {
                Ok(Some(a)) => {
                    crate::agents::emit_agent(state, &a);
                    // Put it at the top of the roster.
                    let mut ids = vec![LOFT_ID.to_string()];
                    ids.extend(state.db.list_agents().unwrap_or_default().into_iter().map(|a| a.id).filter(|id| id != LOFT_ID));
                    let _ = state.db.reorder_agents(&ids);
                    crate::agents::wake(state, LOFT_ID, KICKOFF.into(), "setup");
                }
                Ok(None) => {}
                Err(e) => tracing::error!("creating the loft agent: {e}"),
            }
        }
        Err(e) => tracing::error!("looking up the loft agent: {e}"),
    }
}

pub fn mcp_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_projects",
            "description": "List the git repos in the user's projects folder, most active first: commits in the last 30 and 180 days, the last commit, languages, GitHub remote, whether it has tests, CI and dependency manifests, and the agents already working on it.",
            "inputSchema": { "type": "object", "properties": {
                "limit": { "type": "integer", "description": "How many projects to list (default 20)." }
            } }
        }),
        json!({
            "name": "list_agent_templates",
            "description": "List the agent templates create_agent can start from: what each does, when it fits a project and when it doesn't, its default schedule and its standing instructions.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "create_agent",
            "description": "Create a new Loft agent. Start from a template and customize its instructions for the project, or write one from scratch. It gets Loft's default tool permissions and asks the user for anything more. Only create agents the user agreed to. Agents can't delete agents; only the user can.",
            "inputSchema": { "type": "object", "properties": {
                "template": { "type": "string", "description": "A template key from list_agent_templates, e.g. \"reviewer\". Fills in anything you leave out." },
                "project": { "type": "string", "description": "The project it works on: a repo name exactly as list_projects shows it (e.g. \"work/api\")." },
                "name": { "type": "string", "description": "Short name, unique within the project. Defaults to the template's." },
                "title": { "type": "string" },
                "prompt": { "type": "string", "description": "Its standing instructions. With a template, this is added after the template's under \"About this project\": what the project is, its stack, how to build and test it, what the user cares about." },
                "replace_prompt": { "type": "boolean", "description": "Use `prompt` instead of the template's instructions rather than after them." },
                "personality": { "type": "string" },
                "model": { "type": "string", "enum": ["haiku", "sonnet", "opus"] },
                "interval_minutes": { "type": "integer", "description": "Minutes between scheduled check-ins (0 = only when messaged, otherwise at least 60). Defaults to the template's." },
                "quiet_hours": { "type": "array", "description": "When scheduled check-ins pause, usually the user's working hours. Defaults to weekdays 09:00–18:00. Pass [] to run around the clock.", "items": { "type": "object", "properties": {
                    "days": { "type": "array", "items": { "type": "integer" }, "description": "0 = Monday … 6 = Sunday" },
                    "start": { "type": "string", "description": "HH:MM" },
                    "end": { "type": "string", "description": "HH:MM" }
                } } },
                "avatar_brief": { "type": "string", "description": "How its sticker avatar should look, from its job, personality and project: shape, colors, expression, an accessory. The new agent draws itself from this right after it is created." },
                "reason": { "type": "string", "description": "Why this agent, in one line. Shown to the user if they have to approve it." }
            } }
        }),
    ]
}

/// Handle the setup tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    Some(match name {
        "list_projects" => Ok(list_projects(state, args["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize).await),
        "list_agent_templates" => Ok(list_templates()),
        "create_agent" => create_agent(state, me, args).await,
        _ => return None,
    })
}

fn list_templates() -> String {
    TEMPLATES
        .iter()
        .map(|t| {
            format!(
                "## {key}: {name} ({title})\nFits: {fits}\nSkip: {skip}\nDefault: every {every}, model {model}.\nInstructions:\n{prompt}",
                key = t.key,
                name = t.name,
                title = t.title,
                fits = t.fits,
                skip = t.skip,
                every = every(t.interval_minutes),
                model = t.model,
                prompt = t.prompt,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn every(minutes: i64) -> String {
    match minutes {
        m if m % 10080 == 0 => format!("{} week(s)", m / 10080),
        m if m % 1440 == 0 => format!("{} day(s)", m / 1440),
        m if m % 60 == 0 => format!("{} hour(s)", m / 60),
        m => format!("{m} minutes"),
    }
}

/// What a project has, from the files at its root and in its top-level folders (`backend/`, `ios/`).
fn features(dir: &std::path::Path) -> Vec<&'static str> {
    let mut dirs = vec![dir.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(dir) {
        dirs.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))));
    }
    let has = |p: &str| dirs.iter().any(|d| d.join(p).exists());
    let mut out = vec![];
    const MANIFESTS: &[&str] = &["package.json", "Cargo.toml", "requirements.txt", "pyproject.toml", "go.mod", "Gemfile", "Podfile", "Package.swift", "composer.json", "pom.xml", "build.gradle"];
    if MANIFESTS.iter().any(|m| has(m)) {
        out.push("dependency manifest");
    }
    let tests = ["test", "tests", "spec", "__tests__"].iter().any(|d| has(d))
        || dirs.iter().any(|d| std::fs::read_to_string(d.join("package.json")).is_ok_and(|p| p.contains("\"test\"")))
        || has("Cargo.toml");
    if tests {
        out.push("tests");
    }
    if dir.join(".github/workflows").exists() || dir.join(".gitlab-ci.yml").exists() || dir.join(".circleci").exists() {
        out.push("CI");
    }
    if has("Dockerfile") || has("docker-compose.yml") || has("fly.toml") || has("vercel.json") || has("netlify.toml") {
        out.push("deploy config");
    }
    out
}

async fn list_projects(state: &AppState, limit: usize) -> String {
    let mut repos = crate::repos::list(state, false).await;
    if repos.is_empty() {
        return format!("No git repos in {}. The user can change the projects folder in Settings.", state.settings.boot().projects_dir.display());
    }
    let recent = |r: &crate::repos::RepoSummary, days: usize| r.daily.iter().rev().take(days).sum::<i64>();
    repos.sort_by_key(|r| (std::cmp::Reverse(recent(r, 30)), std::cmp::Reverse(r.last_commit.as_ref().map_or(0, |c| c.timestamp))));
    let agents = state.db.list_agents().unwrap_or_default();
    let total = repos.len();
    let mut lines: Vec<String> = repos
        .iter()
        .take(limit)
        .map(|r| {
            let last = r.last_commit.as_ref().map_or("no commits".into(), |c| {
                let when = chrono::DateTime::from_timestamp(c.timestamp, 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
                format!("last commit {when} \"{}\"", crate::notify::clip(&c.subject, 60))
            });
            let langs = r.languages.iter().take(3).map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", ");
            let on: Vec<String> = agents.iter().filter(|a| a.project.trim() == r.name).map(|a| a.name.clone()).collect();
            let mut f = features(std::path::Path::new(&r.path));
            if r.github.is_some() {
                f.insert(0, "GitHub");
            }
            format!(
                "- {name}: {c30} commits in 30 days, {c180} in 180; {last}; {langs}{feat}; agents: {on}",
                name = r.name,
                c30 = recent(r, 30),
                c180 = recent(r, 180),
                langs = if langs.is_empty() { "no code detected".into() } else { langs },
                feat = if f.is_empty() { String::new() } else { format!("; has {}", f.join(", ")) },
                on = if on.is_empty() { "none".into() } else { on.join(", ") },
            )
        })
        .collect();
    if total > limit {
        lines.push(format!("…and {} more, less active.", total - limit));
    }
    format!("Projects folder: {}\n{}", state.settings.boot().projects_dir.display(), lines.join("\n"))
}

async fn create_agent(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    let s = |k: &str| args[k].as_str().unwrap_or("").trim().to_string();
    let tpl = match s("template") {
        k if k.is_empty() => None,
        k => Some(template(&k).ok_or_else(|| format!("No template called {k}. Call list_agent_templates to see them."))?),
    };
    let project = s("project");
    let name = Some(s("name")).filter(|n| !n.is_empty()).or_else(|| tpl.map(|t| t.name.to_string())).ok_or("name is required without a template")?;
    let all = state.db.list_agents().map_err(|e| e.to_string())?;
    if all.iter().any(|a| a.project.trim() == project && a.name.eq_ignore_ascii_case(&name)) {
        let place = if project.is_empty() { String::new() } else { format!(" in {project}") };
        return Err(format!("There is already an agent called {name}{place}. Pick another name, or leave it as it is."));
    }
    if all.len() >= MAX_AGENTS {
        return Err(format!("Loft already has {} agents. Ask the user to delete some before creating more.", all.len()));
    }
    // The project's folder, for the instructions; a project that isn't a folder is just a tag.
    let path = if project.is_empty() { String::new() } else { state.settings.boot().projects_dir.join(&project).display().to_string() };

    let custom = s("prompt");
    let prompt = match tpl {
        Some(t) => {
            let base = t.prompt.replace("{project}", if project.is_empty() { "the user's projects" } else { &project }).replace("{path}", &path);
            match (custom.is_empty(), args["replace_prompt"].as_bool().unwrap_or(false)) {
                (true, _) => base,
                (false, true) => custom,
                (false, false) => format!("{base}\n\n## About this project\n{custom}"),
            }
        }
        None if custom.is_empty() => return Err("prompt is required without a template".into()),
        None => custom,
    };
    let model = Some(s("model")).filter(|m| !m.is_empty()).unwrap_or_else(|| tpl.map_or("sonnet", |t| t.model).to_string());
    if !["haiku", "sonnet", "opus"].contains(&model.as_str()) {
        return Err("model must be haiku, sonnet or opus".into());
    }
    let interval = args["interval_minutes"].as_i64().unwrap_or_else(|| tpl.map_or(0, |t| t.interval_minutes));
    if interval != 0 && interval < MIN_INTERVAL {
        return Err(format!("interval_minutes must be 0 or at least {MIN_INTERVAL}"));
    }
    let quiet: Vec<QuietWindow> = match args.get("quiet_hours") {
        None | Some(Value::Null) => default_quiet_hours(),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| "quiet_hours must be a list of { days, start, end }".to_string())?,
    };
    // Only these fields: an agent can't hand another agent more tools, MCP servers or a looser permission mode.
    let patch = json!({
        "name": name,
        "title": Some(s("title")).filter(|t| !t.is_empty()).unwrap_or_else(|| tpl.map_or(String::new(), |t| t.title.to_string())),
        "project": project,
        "personality": Some(s("personality")).filter(|p| !p.is_empty()).unwrap_or_else(|| tpl.map_or(String::new(), |t| t.personality.to_string())),
        "prompt": prompt,
        "model": model,
        "interval_minutes": interval,
        "quiet_hours": quiet,
        "avatar_brief": s("avatar_brief"),
    });

    let allowed = me.allowed_tools.iter().any(|r| crate::agents::rule_matches(r, CREATE_RULE, &patch));
    if !allowed {
        let reason = Some(s("reason")).filter(|r| !r.is_empty()).unwrap_or_else(|| format!("{} wants to create the agent {name}.", me.name));
        let d = crate::agents::ask_permission(state, &me.id, "prompt", CREATE_RULE, patch.clone(), &reason, CREATE_RULE.into()).await;
        if !d.allow {
            return Err(format!("The user declined.{}", d.message.map(|m| format!(" Their note: {m}")).unwrap_or_default()));
        }
    }

    let a = crate::api::new_agent(state, uuid::Uuid::new_v4().to_string(), &patch, Some(me.id.clone())).map_err(|e| e.1)?;
    let schedule = if a.interval_minutes > 0 {
        let quiet = if a.quiet_hours.is_empty() { String::new() } else { format!(" (paused {})", crate::schedule::describe(&a.quiet_hours)) };
        format!("checks in every {}{quiet}", every(a.interval_minutes))
    } else {
        "runs only when messaged".into()
    };
    let drawing = if s("avatar_brief").is_empty() { "" } else { " It is drawing its avatar now." };
    Ok(format!("Created {} (id {}) for {}; it {schedule}.{drawing}", a.name, a.id, if a.project.is_empty() { "no project" } else { &a.project }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::{agent, state};

    #[tokio::test]
    async fn loft_creates_an_agent_from_a_template_with_default_tools_only() {
        let st = state();
        let mut me = agent(&st, "loft");
        me.allowed_tools.push(CREATE_RULE.into());
        st.db.save_agent(&me).unwrap();
        let out = create_agent(&st, &me, &json!({
            "template": "security", "project": "work/api", "prompt": "An Axum API behind Cloudflare.",
            // Not fields an agent can set: ignored.
            "allowed_tools": ["Bash"], "permission_mode": "bypassPermissions", "mcp_servers": { "x": {} },
        }))
        .await
        .unwrap();
        assert!(out.starts_with("Created Security"), "{out}");
        let a = st.db.list_agents().unwrap().into_iter().find(|a| a.name == "Security").unwrap();
        assert_eq!((a.project.as_str(), a.interval_minutes, a.model.as_str(), a.permission_mode.as_str()), ("work/api", 1440, "sonnet", "auto"));
        assert_eq!(a.allowed_tools, crate::api::DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a.mcp_servers, json!({}));
        assert_eq!(a.created_by.as_deref(), Some(me.id.as_str()));
        assert!(a.prompt.contains("**work/api**") && a.prompt.ends_with("## About this project\nAn Axum API behind Cloudflare."), "{}", a.prompt);
        assert_eq!(a.quiet_hours, default_quiet_hours());

        // Same name in the same project: refused. In another project: fine.
        assert!(create_agent(&st, &me, &json!({ "template": "security", "project": "work/api" })).await.is_err());
        assert!(create_agent(&st, &me, &json!({ "template": "security", "project": "site" })).await.is_ok());
    }

    #[tokio::test]
    async fn schedules_and_models_are_checked() {
        let st = state();
        let mut me = agent(&st, "loft");
        me.allowed_tools.push(CREATE_RULE.into());
        for bad in [
            json!({ "template": "nope" }),
            json!({ "name": "Solo" }),
            json!({ "template": "ci", "interval_minutes": 5 }),
            json!({ "template": "ci", "model": "gpt-5" }),
            json!({ "template": "ci", "quiet_hours": [{ "days": [9], "start": "09:00", "end": "17:00" }] }),
        ] {
            assert!(create_agent(&st, &me, &bad).await.is_err(), "{bad}");
        }
        let ok = create_agent(&st, &me, &json!({ "name": "Notes", "prompt": "Tidy my notes.", "interval_minutes": 0, "quiet_hours": [] })).await;
        assert!(ok.unwrap().contains("runs only when messaged"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn other_agents_need_the_users_approval() {
        let st = state();
        let me = agent(&st, "Dev");
        let st2 = st.clone();
        let call = tokio::spawn(async move { create_agent(&st2, &me, &json!({ "template": "docs", "project": "site" })).await });
        // A permission card waits for the user; deny it.
        let req = loop {
            if let Some(p) = st.db.list_permissions(None).unwrap().into_iter().find(|p| p.status == "pending") {
                break p;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert_eq!(req.tool_name, CREATE_RULE);
        crate::agents::decide(&st, &req.id, "deny", None, Some("not now".into())).unwrap();
        assert_eq!(call.await.unwrap().unwrap_err(), "The user declined. Their note: not now");
        assert!(!st.db.list_agents().unwrap().iter().any(|a| a.name == "Docs"));
    }

    #[tokio::test]
    async fn loft_is_built_in_and_cannot_be_deleted() {
        let st = state();
        // Busy, so the kickoff is queued instead of starting a real claude run.
        st.rt.lock().unwrap().running.insert(LOFT_ID.into(), std::sync::Arc::new(tokio::sync::Notify::new()));
        let other = agent(&st, "Dev");
        ensure_loft(&st);
        ensure_loft(&st);
        let all = st.db.list_agents().unwrap();
        assert_eq!(all.iter().filter(|a| a.id == LOFT_ID).count(), 1);
        assert_eq!(all[0].id, LOFT_ID, "first in the roster");
        let loft = &all[0];
        assert!(loft.builtin && loft.allowed_tools.contains(&CREATE_RULE.to_string()));
        assert_eq!(loft.avatar.svg, LOFT_AVATAR.trim(), "the house sticker is the default");
        assert!(st.rt.lock().unwrap().queued.get(LOFT_ID).is_some_and(|q| q[0].starts_with("[Loft: first launch]")));

        let del = |id: &str| crate::api::delete_agent(axum::extract::State(st.clone()), axum::extract::Path(id.to_string()), axum::extract::Query(serde_json::from_value(json!({})).unwrap()));
        assert_eq!(del(LOFT_ID).await.err().unwrap().0, axum::http::StatusCode::FORBIDDEN);
        assert!(del(&other.id).await.is_ok(), "the user can still delete any other agent");
        assert!(st.db.get_agent(LOFT_ID).unwrap().is_some());

        // No MCP tool deletes agents.
        let names: Vec<String> = mcp_tools().iter().chain(crate::tasks::mcp_tools().iter()).map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert!(!names.iter().any(|n| n.contains("delete")), "{names:?}");
    }

    #[tokio::test]
    async fn an_avatar_brief_has_the_new_agent_draw_itself() {
        let st = state();
        let busy = |id: &str| st.rt.lock().unwrap().running.insert(id.into(), std::sync::Arc::new(tokio::sync::Notify::new()));
        // Busy, so the drawing run is queued instead of starting a real claude run.
        busy("a1");
        crate::api::new_agent(&st, "a1".into(), &json!({ "name": "Security", "avatar_brief": "A teal shield with a padlock, one eyebrow raised." }), None).unwrap();
        let q = st.rt.lock().unwrap().queued.get("a1").cloned().unwrap_or_default();
        assert!(q.len() == 1 && q[0].contains("Brief: A teal shield with a padlock") && q[0].contains("mcp__loft__set_avatar"), "{q:?}");
        // An explicit avatar wins; no brief, no drawing run.
        busy("a2");
        busy("a3");
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#e5533d"/></svg>"##;
        crate::api::new_agent(&st, "a2".into(), &json!({ "name": "A", "avatar_brief": "a cat", "avatar": { "svg": svg } }), None).unwrap();
        crate::api::new_agent(&st, "a3".into(), &json!({ "name": "B" }), None).unwrap();
        assert!(st.rt.lock().unwrap().queued.get("a2").is_none() && st.rt.lock().unwrap().queued.get("a3").is_none());
    }

    #[test]
    fn features_look_one_folder_down() {
        let dir = std::env::temp_dir().join(format!("loft-features-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("backend")).unwrap();
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::write(dir.join("backend/Cargo.toml"), "").unwrap();
        assert_eq!(features(&dir), ["dependency manifest", "tests", "CI"]);
        assert!(features(&dir.join("backend/missing")).is_empty());
    }

    #[test]
    fn every_template_fills_in_its_project() {
        for t in TEMPLATES {
            assert!(t.prompt.contains("{project}") && t.prompt.contains("{path}"), "{}", t.key);
            assert!(t.interval_minutes >= MIN_INTERVAL, "{}", t.key);
        }
        assert!(list_templates().contains("## security: Security (Security auditor)"));
    }
}
