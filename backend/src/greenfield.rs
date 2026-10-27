//! A project with no GitHub repo yet. Its first round offers to create one, and only after the user's yes
//! does Loft itself (never the agent) publish the folder: a `.gitignore` for obvious junk, `git init`, the
//! first commit, the GitHub repo under the chosen owner, private by default, and the push. This is the one
//! place Loft writes into a project folder.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::db::Agent;
use crate::grill::Question;
use crate::state::AppState;

/// Obvious junk the first commit leaves out.
const IGNORE: &[&str] = &[".DS_Store", ".remember/"];
/// Files the preview lists by name; the rest are counted.
const MAX_LISTED: usize = 100;
const MAX_QUESTION: usize = 500;
const MAX_OWNERS: usize = 5;

/// What publishing the spec says when the project has no GitHub repo: the user is asked again first.
pub const ASK_AGAIN: &str = "Ask the user again first: put \"Create the GitHub repo and publish this folder?\" to them as a round question (`mcp__loft__preview_new_repo` gives it), and after their yes call `mcp__loft__create_repo`, then publish.";

pub fn mcp_tools() -> Vec<Value> {
    vec![json!({
        "name": "preview_new_repo",
        "description": "For a project with no git repo or no GitHub remote yet: what publishing its folder as a new GitHub repo would hold (the files and total size of the first commit, after the `.gitignore` Loft adds for `.DS_Store` and `.remember/`), the owners the user can create it under, and the round question to ask. Writes nothing.",
        "inputSchema": { "type": "object", "properties": {} }
    }), json!({
        "name": "create_repo",
        "description": "Only after the user said yes to the round question from preview_new_repo (it names `<owner>/<name>`), in a round they sent. A picked option or the accepted recommendation fixes the owner it names and a private repo; only an answer the user wrote can ask for a public one. Loft writes the `.gitignore`, runs `git init`, commits everything else in the project folder, creates the GitHub repo and pushes. Calling it again after a failure reuses the repo already created.",
        "inputSchema": { "type": "object", "properties": {
            "question_id": { "type": "string", "description": "The id of the round question the user said yes to, from the round's answers." },
            "owner": { "type": "string", "description": "The account or organization the user chose, one of the owners from preview_new_repo. Default: the user's account." },
            "name": { "type": "string", "description": "The repo's name. Default: the project's folder name." },
            "private": { "type": "boolean", "description": "Default true. False only when the user asked for a public repo." }
        }, "required": ["question_id"] }
    })]
}

/// Handle the new-repo tools. None when `name` isn't one of them.
pub async fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    if name != "preview_new_repo" && name != "create_repo" {
        return None;
    }
    if !crate::grill::enabled(me) {
        return Some(Err("Planning isn't on for you.".into()));
    }
    Some(if name == "preview_new_repo" { preview(state, me).await } else { create(state, me, args).await })
}

/// True when the agent's project folder exists but has no git repo or no `origin` yet, so a grill there
/// starts by offering to create the repo.
pub fn needs_repo(state: &AppState, a: &Agent) -> bool {
    if a.project.trim().is_empty() {
        return false;
    }
    let dir = project_dir(state, a);
    dir.is_dir() && !std::process::Command::new("git").arg("-C").arg(&dir).args(["remote", "get-url", "origin"]).output().is_ok_and(|o| o.status.success())
}

fn project_dir(state: &AppState, a: &Agent) -> PathBuf {
    state.settings.boot().projects_dir.join(a.project.trim())
}

/// The project folder, when it is one that still needs a GitHub repo, and whether it is a git repo already.
async fn greenfield_dir(state: &AppState, a: &Agent) -> Result<(PathBuf, bool), String> {
    if a.project.trim().is_empty() {
        return Err("You have no project, so there is no folder to publish. Ask the user to set your project in the Config tab.".into());
    }
    let dir = project_dir(state, a);
    if !dir.is_dir() {
        return Err(format!("The project folder {} doesn't exist.", dir.display()));
    }
    if !dir.join(".git").exists() {
        return Ok((dir, false));
    }
    match crate::repos::git(&dir, &["remote", "get-url", "origin"]).await {
        Err(_) => Ok((dir, true)),
        Ok(url) => match crate::repos::github_slug(&url) {
            Some(slug) => Err(format!("The project is already on GitHub as {slug}: there is nothing to create, publish there.")),
            None => Err(format!("The project's `origin` ({}) isn't on GitHub, and Loft only creates GitHub repos.", url.trim())),
        },
    }
}

/// Every file the first commit would hold, with its size: what git would add once the `.gitignore` has
/// the junk rules, so the user's own ignore files count too. Without a repo yet, git looks at the folder
/// through a throwaway one.
async fn files(dir: &Path, has_git: bool) -> Result<Vec<(String, u64)>, String> {
    let mut args = vec!["ls-files", "-z", "--cached", "--others", "--exclude-standard"];
    let excludes: Vec<String> = IGNORE.iter().map(|i| format!("--exclude={i}")).collect();
    args.extend(excludes.iter().map(String::as_str));
    let scratch = std::env::temp_dir().join(format!("loft-new-repo-{}", uuid::Uuid::new_v4()));
    let out = if has_git {
        crate::repos::git(dir, &args).await
    } else {
        let git_dir = scratch.join(".git");
        let r = async {
            std::fs::create_dir_all(&scratch)?;
            crate::repos::git(&scratch, &["init", "-q"]).await?;
            let (g, w) = (format!("--git-dir={}", git_dir.display()), format!("--work-tree={}", dir.display()));
            crate::repos::git(dir, &[&[g.as_str(), w.as_str()], args.as_slice()].concat()).await
        }
        .await;
        let _ = std::fs::remove_dir_all(&scratch);
        r
    };
    let out = out.map_err(|e| format!("Couldn't list the project's files: {e}"))?;
    let mut fs: Vec<(String, u64)> = out.split('\0').filter(|p| !p.is_empty()).map(|p| (p.to_string(), std::fs::metadata(dir.join(p)).map(|m| m.len()).unwrap_or(0))).collect();
    if !fs.iter().any(|(p, _)| p == ".gitignore") {
        fs.push((".gitignore".into(), gitignore_after(dir).len() as u64));
    }
    fs.sort();
    Ok(fs)
}

/// The project's `.gitignore` with the junk rules it lacks appended, or None when it has them all.
fn gitignore_update(dir: &Path) -> Option<String> {
    let have = std::fs::read_to_string(dir.join(".gitignore")).unwrap_or_default();
    let bare = |l: &str| l.trim().trim_matches('/').to_string();
    let missing: Vec<&str> = IGNORE.iter().copied().filter(|i| !have.lines().any(|l| bare(l) == bare(i))).collect();
    if missing.is_empty() {
        return None;
    }
    let sep = if have.is_empty() || have.ends_with('\n') { "" } else { "\n" };
    Some(format!("{have}{sep}{}\n", missing.join("\n")))
}

fn gitignore_after(dir: &Path) -> String {
    gitignore_update(dir).unwrap_or_else(|| std::fs::read_to_string(dir.join(".gitignore")).unwrap_or_default())
}

fn human(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// The token's user: login and id.
async fn me_on_github(state: &AppState) -> Result<(String, Option<i64>), String> {
    if crate::github::token(state).is_none() {
        return Err("GitHub isn't connected: the user needs to add a GitHub token in Loft's Settings or run `gh auth login`.".into());
    }
    let u = crate::github::get(state, "/user").await.map_err(|e| format!("Couldn't read the GitHub user of Loft's token: {e}"))?;
    let login = u["login"].as_str().ok_or("GitHub didn't say who the token belongs to.")?;
    Ok((login.to_string(), u["id"].as_i64()))
}

/// Where the user can create a repo: their own account first, then the organizations they administer or
/// whose members may create repos. A token that can't read its organizations offers the account only.
async fn owners(state: &AppState, login: &str) -> Vec<String> {
    let mut out = vec![login.to_string()];
    let memberships = crate::github::get(state, "/user/memberships/orgs?state=active&per_page=100").await.unwrap_or(Value::Null);
    for m in memberships.as_array().into_iter().flatten() {
        let Some(org) = m["organization"]["login"].as_str() else { continue };
        let can = m["role"] == "admin" || crate::github::get(state, &format!("/orgs/{org}")).await.is_ok_and(|o| o["members_can_create_repositories"] == true);
        if can && !out.iter().any(|o| o.eq_ignore_ascii_case(org)) {
            out.push(org.to_string());
        }
    }
    out
}

/// Files that usually hold secrets: env files, keys, certificates.
fn secret_like(path: &str) -> bool {
    let f = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    (f == ".env" || f.starts_with(".env.")) && !f.ends_with(".example") && !f.ends_with(".sample")
        || f.starts_with("id_rsa")
        || f.starts_with("id_ed25519")
        || [".pem", ".key", ".p12", ".pfx"].iter().any(|e| f.ends_with(e))
}

/// GitHub's own rule for repo names, applied to the folder's name.
fn repo_name(project: &str) -> String {
    let n: String = project.trim().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
    n.trim_matches('-').to_string()
}

async fn preview(state: &AppState, me: &Agent) -> Result<String, String> {
    let (dir, has_git) = greenfield_dir(state, me).await?;
    let (login, _) = me_on_github(state).await?;
    let owners = owners(state, &login).await;
    let fs = files(&dir, has_git).await?;
    let total = human(fs.iter().map(|(_, s)| s).sum());
    let name = repo_name(&me.project);
    let (n, s) = (fs.len(), if fs.len() == 1 { "" } else { "s" });

    // Secrets first, so the user sees them before saying yes; then as many files as fit. The tool result lists them all.
    let secrets: Vec<String> = fs.iter().map(|(p, _)| p).filter(|p| secret_like(p)).take(3).map(|p| format!("`{p}`")).collect();
    let warn = if secrets.is_empty() { String::new() } else { format!("Careful: it includes {}, which may hold secrets. ", secrets.join(", ")) };
    let head = format!("Create the GitHub repo `{login}/{name}` and publish this folder? {warn}The first commit holds {n} file{s} ({total}): ");
    let tail = ". Loft adds a `.gitignore` for `.DS_Store` and `.remember/`.";
    let more = |shown: usize| if shown < n { format!(" and {} more", n - shown) } else { String::new() };
    let mut listed: Vec<String> = vec![];
    for (p, _) in &fs {
        let with = [listed.as_slice(), &[format!("`{p}`")]].concat();
        if format!("{head}{}{}{tail}", with.join(", "), more(with.len())).chars().count() > MAX_QUESTION {
            break;
        }
        listed = with;
    }
    let more = more(listed.len());
    let mut options: Vec<String> = owners.iter().take(MAX_OWNERS).map(|o| format!("Yes, private under {o}")).collect();
    options.push("No, not now".into());
    let question = json!({
        "text": format!("{head}{}{more}{tail}", listed.join(", ")),
        "recommendation": format!("Yes, private under {login}"),
        "speak": format!("Create a private GitHub repo for this folder under {login} and publish its {n} file{s}, {total}? I recommend yes, private."),
        "kind": "single_choice",
        "options": options,
    });

    let state_line = if has_git { "is a git repo with no `origin` remote" } else { "has no git repo yet" };
    let mut list: Vec<String> = fs.iter().take(MAX_LISTED).map(|(p, s)| format!("- {p} ({})", human(*s))).collect();
    if n > MAX_LISTED {
        list.push(format!("- and {} more", n - MAX_LISTED));
    }
    Ok(format!(
        "The project folder {} {state_line}. Owners the user can create the repo under: {} (their account first). Default name: {name}.\n\nThe first commit: {n} file{s}, {total} (the `.gitignore` Loft writes or updates for `.DS_Store` and `.remember/` included):\n{}\n\nAsk it in your first round, in the user's language, as this question (private recommended; owners as options):\n{question}\n\nAfter the user's yes, in a round they sent, call `mcp__loft__create_repo` with that question's id and the owner they chose. Never write into the project folder yourself.",
        dir.display(),
        owners.join(", "),
        list.join("\n"),
    ))
}

/// True when the user's answer to `q` is a yes: the recommendation accepted, or an answer that starts with one.
fn said_yes(q: &Question) -> bool {
    const YES: &[&str] = &["yes", "y", "yep", "yeah", "sure", "ok", "okay", "go", "sim", "s", "claro", "pode", "beleza", "isso", "bora"];
    q.state == "answered" && (q.accepted || q.answer.as_deref().map(crate::grill::commands::normalize).is_some_and(|a| a.split(' ').next().is_some_and(|w| YES.contains(&w))))
}

/// The user's yes to the round question `qid` of the agent's latest grill, in a round they sent.
fn check_yes(state: &AppState, me: &Agent, qid: &str) -> Result<Question, String> {
    let grills = state.db.list_grills(&me.id).map_err(|e| e.to_string())?;
    let found = grills.first().into_iter().flat_map(|s| &s.rounds).find_map(|r| r.questions.iter().find(|q| q.id == qid).map(|q| (r, q)));
    let Some((round, q)) = found else {
        return Err("No question with that id in your grill. Ask the user first, as a round question (see `mcp__loft__preview_new_repo`), and pass its id once they said yes.".into());
    };
    if round.sent_at.is_none() {
        return Err("The user hasn't sent that round yet: you're woken with their answer when they do. End your turn with no reply text.".into());
    }
    if !said_yes(q) {
        let answer = match q.state.as_str() {
            "answered" | "discuss" => format!("\"{}\"", q.answer.as_deref().unwrap_or("")),
            "decided" => "left to you, which isn't a yes".into(),
            _ => "none".into(),
        };
        return Err(format!(
            "The user didn't say yes to creating the repo (their answer: {answer}), so nothing was created. Keep grilling without it, and ask again in a round before publishing the spec."
        ));
    }
    Ok(q.clone())
}

/// The owners named in `text` as whole words: "acme" in "Sim, acme, privado", not in "acme-labs".
fn owners_named<'a>(text: &str, owners: &'a [String]) -> Vec<&'a String> {
    let text = text.to_lowercase();
    let words: Vec<&str> = text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).collect();
    owners.iter().filter(|o| words.contains(&o.to_lowercase().as_str())).collect()
}

/// True when the user's own answer asks for a public repo ("public", "público", "pública") and says
/// nothing private or negative next to it ("not public", "privado"), which would make it unclear.
fn asks_public(answer: &str) -> bool {
    const NOT: &[&str] = &["not", "no", "dont", "never", "nao", "nem", "nunca", "sem"];
    let a = crate::grill::commands::normalize(answer);
    let words: Vec<&str> = a.split_whitespace().collect();
    words.iter().any(|w| w.starts_with("public")) && !words.iter().any(|w| w.starts_with("privat") || w.starts_with("privad") || NOT.contains(w))
}

/// The yes has to be to this repo: the question names `<owner>/<name>`. The owner is the one the answer
/// names (a picked option, the accepted recommendation or the user's own words), wherever it stands in
/// it, or else the one in the question. A repo is public only when the user wrote so themselves.
fn check_choice(q: &Question, owners: &[String], owner: &str, name: &str, private: bool) -> Result<(), String> {
    let text = q.text.to_lowercase();
    let slugs: Vec<&str> = text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '/'))).map(|w| w.trim_end_matches('.')).collect();
    let asked: Vec<&String> = owners.iter().filter(|o| slugs.contains(&format!("{o}/{name}").to_lowercase().as_str())).collect();
    if asked.is_empty() {
        return Err(format!(
            "That question doesn't name the repo `<owner>/{name}`, so it isn't a yes to creating it. Ask the question from `mcp__loft__preview_new_repo` (with the new name, if the user wants another one) and pass its id after their yes."
        ));
    }
    let n = crate::grill::commands::normalize;
    let answer = if q.accepted { q.recommendation.clone() } else { q.answer.clone().unwrap_or_default() };
    let own_words = !q.accepted && !q.options.iter().any(|o| n(o) == n(&answer));
    if !private && !(own_words && asks_public(&answer)) {
        return Err(format!("The user answered \"{answer}\", which doesn't ask for a public repo: call create_repo without `private: false`, or ask them again if they want it public."));
    }
    let chosen = match (owners_named(&answer, owners).as_slice(), asked.as_slice()) {
        ([o], _) | ([], [o]) => *o,
        ([], _) => return Err(format!("The question names more than one owner and the user's answer (\"{answer}\") none: ask them again which one.")),
        (named, _) => {
            let named: Vec<&str> = named.iter().map(|o| o.as_str()).collect();
            return Err(format!("The user's answer (\"{answer}\") names more than one owner ({}): ask them again which one.", named.join(", ")));
        }
    };
    if !chosen.eq_ignore_ascii_case(owner) {
        return Err(format!("The user answered \"{answer}\": create it under {chosen}, not {owner}."));
    }
    Ok(())
}

/// Runs git in the project folder with Loft's token for github.com, passed in the environment so it
/// never shows in the process list or lands in the repo's config.
async fn git_push(dir: &Path, token: &str) -> Result<(), String> {
    use base64::Engine;
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["push", "-u", "origin", "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
        .env("GIT_CONFIG_VALUE_0", format!("AUTHORIZATION: basic {auth}"))
        .output()
        .await
        .map_err(|e| format!("spawning git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

async fn create(state: &AppState, me: &Agent, args: &Value) -> Result<String, String> {
    let qid = args["question_id"].as_str().unwrap_or("").trim();
    if qid.is_empty() {
        return Err("question_id is required: the round question the user said yes to.".into());
    }
    let q = check_yes(state, me, qid)?;
    let (dir, has_git) = greenfield_dir(state, me).await?;
    let (login, id) = me_on_github(state).await?;
    let owner = args["owner"].as_str().map(str::trim).filter(|o| !o.is_empty()).unwrap_or(&login).to_string();
    let owners = owners(state, &login).await;
    let Some(owner) = owners.iter().find(|o| o.eq_ignore_ascii_case(&owner)).cloned() else {
        return Err(format!("The user can't create repos under {owner}. Owners: {}.", owners.join(", ")));
    };
    let name = args["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| repo_name(&me.project));
    if name.is_empty() || name.len() > 100 || name == "." || name == ".." || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(format!("\"{name}\" isn't a valid repo name: letters, digits, `.`, `_` and `-`, at most 100."));
    }
    let private = args["private"].as_bool().unwrap_or(true);
    check_choice(&q, &owners, &owner, &name, private)?;
    let slug = format!("{owner}/{name}");

    // GitHub first: a refusal (a taken name, a token without the right) leaves the folder untouched. A
    // repo Loft created before a failed push is reused rather than created twice.
    let repo = if matches!(state.db.published_issue(&slug, "repo", "created"), Ok(Some(_))) {
        crate::github::get(state, &format!("/repos/{slug}")).await.map_err(|e| format!("Couldn't read {slug}, which Loft created earlier: {e}"))?
    } else {
        let path = if owner == login { "/user/repos".to_string() } else { format!("/orgs/{owner}/repos") };
        let made = crate::github::request(state, reqwest::Method::POST, &path, Some(json!({ "name": name, "private": private }))).await;
        let repo = made.map_err(|e| {
            if e.to_string().contains("already exists") {
                format!("A repo named {name} already exists under {owner}: ask the user for another name, and pass it as `name`.")
            } else {
                format!("GitHub didn't create {slug}: {e}")
            }
        })?;
        state.db.record_published_issue(&slug, "repo", "created", 1).map_err(|e| e.to_string())?;
        repo
    };
    let clone_url = repo["clone_url"].as_str().ok_or_else(|| format!("GitHub didn't return the clone URL of {slug}."))?.to_string();
    let private = repo["private"].as_bool().unwrap_or(private);

    let fs = files(&dir, has_git).await?;
    let total = human(fs.iter().map(|(_, s)| s).sum());
    let git = |a: Vec<String>| {
        let dir = dir.clone();
        async move { crate::repos::git(&dir, &a.iter().map(String::as_str).collect::<Vec<_>>()).await.map_err(|e| format!("Couldn't publish the folder: {e}")) }
    };
    let s = |x: &[&str]| x.iter().map(|a| a.to_string()).collect::<Vec<String>>();
    if let Some(ignore) = gitignore_update(&dir) {
        std::fs::write(dir.join(".gitignore"), ignore).map_err(|e| format!("Couldn't write the .gitignore: {e}"))?;
    }
    if !has_git {
        git(s(&["init", "-q"])).await?;
    }
    git(s(&["add", "-A"])).await?;
    let staged = crate::repos::git(&dir, &["diff", "--cached", "--quiet"]).await.is_err();
    let born = crate::repos::git(&dir, &["rev-parse", "-q", "--verify", "HEAD"]).await.is_ok();
    if staged || !born {
        // Commits as the user's git identity, or as their GitHub account when git has none.
        let mut c = vec![];
        if crate::repos::git(&dir, &["config", "user.email"]).await.map(|e| e.trim().is_empty()).unwrap_or(true) {
            let email = id.map_or(format!("{login}@users.noreply.github.com"), |id| format!("{id}+{login}@users.noreply.github.com"));
            c = vec!["-c".into(), format!("user.name={login}"), "-c".into(), format!("user.email={email}")];
        }
        let first = if born { "Publish the project folder" } else { "First commit" };
        git([c, s(&["commit", "-q", "--allow-empty", "-m", first])].concat()).await?;
    }
    git(s(&["remote", "add", "origin", &clone_url])).await?;
    let token = crate::github::token(state).unwrap_or_default();
    if let Err(e) = git_push(&dir, &token).await {
        // Back to "no remote", so calling the tool again picks up from here.
        let _ = crate::repos::git(&dir, &["remote", "remove", "origin"]).await;
        return Err(format!("Created {slug} on GitHub, but pushing the folder failed: {e}. Call create_repo again with the same owner and name: the repo is reused."));
    }
    state.emit(crate::state::Event::Issues { project: me.project.trim().to_string() });
    let html = repo["html_url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{slug}"));
    let (n, fs_s) = (fs.len(), if fs.len() == 1 { "" } else { "s" });
    Ok(format!(
        "Created the {} GitHub repo {slug} and pushed the project folder ({n} file{fs_s}, {total}): {html}. Clone it into your workspace for your docs branch; never work in the project folder itself.",
        if private { "private" } else { "public" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::tests::{Fake, fake_github};
    use crate::grill::Routed;
    use crate::grill::tests::planner;
    use crate::planning::tests::state;

    /// The token's user and organizations, the same for every test on the shared fake: `acme` is
    /// administered, `members-only` lets members create repos, `locked` doesn't.
    fn owners_on(fake: &Fake) {
        fake.route("GET", "/user", 200, json!({ "login": "octo-me" }));
        fake.route("GET", "/user/memberships/orgs", 200, json!([
            { "organization": { "login": "acme" }, "role": "admin" },
            { "organization": { "login": "members-only" }, "role": "member" },
            { "organization": { "login": "locked" }, "role": "member" },
            { "organization": { "login": "open-org" }, "role": "admin" },
        ]));
        fake.route("GET", "/orgs/members-only", 200, json!({ "members_can_create_repositories": true }));
        fake.route("GET", "/orgs/locked", 200, json!({ "members_can_create_repositories": false }));
    }

    /// A planner whose project folder holds these files and no git repo.
    fn new_project(st: &AppState, name: &str, files: &[(&str, &str)]) -> (Agent, PathBuf) {
        let mut a = planner(st, name);
        a.project = name.into();
        st.db.save_agent(&a).unwrap();
        let dir = st.settings.boot().projects_dir.join(name);
        assert!(dir.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        for (path, body) in files {
            std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
            std::fs::write(dir.join(path), body).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        (a, dir)
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=T", "-c", "user.email=t@example.com"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// An empty bare repo standing in for the one GitHub creates: its path is the clone URL.
    fn remote(st: &AppState, name: &str) -> PathBuf {
        let r = st.cfg.storage_dir.parent().unwrap().join("remotes").join(format!("{name}.git"));
        std::fs::create_dir_all(&r).unwrap();
        git(&r, &["init", "-q", "--bare"]);
        r
    }

    fn pushed(remote: &Path) -> Vec<String> {
        git(remote, &["ls-tree", "-r", "--name-only", "HEAD"]).lines().map(String::from).collect()
    }

    async fn call(st: &AppState, a: &Agent, name: &str, args: Value) -> Result<String, String> {
        mcp_call(st, a, name, &args).await.unwrap()
    }

    /// Posts the repo question with the owners as options, answers it with `answer` if any and, with
    /// `send`, sends the round. Returns its id.
    fn ask(st: &AppState, a: &Agent, answer: Option<&str>, send: bool) -> String {
        let options: Vec<String> = ["octo-me", "acme", "members-only", "open-org"].iter().map(|o| format!("Yes, private under {o}")).chain(["No, not now".to_string()]).collect();
        let text = format!("Create the GitHub repo `octo-me/{}` and publish this folder?", a.project);
        ask_q(st, a, json!({ "text": text, "speak": "Create the repo?", "recommendation": "Yes, private under octo-me", "kind": "single_choice", "options": options }), answer, send)
    }

    fn ask_q(st: &AppState, a: &Agent, q: Value, answer: Option<&str>, send: bool) -> String {
        crate::grill::mcp_call(st, a, "grill_round", &json!({ "topic": "New app", "questions": [q] })).unwrap().unwrap();
        let s = crate::grill::active(st, &a.id).unwrap();
        let qid = s.rounds.last().unwrap().questions[0].id.clone();
        if let Some(answer) = answer {
            crate::grill::on_user_message(st, a, answer, Some(&json!({ "role": "answer", "session_id": s.id, "question_id": qid })), json!({})).unwrap();
        }
        if send {
            crate::grill::tests::send(st, a);
        }
        qid
    }

    /// Repo-creation bodies POSTed to `path` for this repo name.
    fn creations(fake: &Fake, path: &str, name: &str) -> Vec<Value> {
        fake.seen(path).into_iter().filter(|s| s.method == "POST" && s.path == path && s.body["name"] == name).map(|s| s.body).collect()
    }

    #[tokio::test]
    async fn a_yes_creates_the_private_repo_under_the_users_account_and_pushes_the_folder() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        let files = [("README.md", "# New\n"), ("src/main.rs", "fn main() {}\n"), (".DS_Store", "junk"), (".remember/today.md", "x"), ("sub/.DS_Store", "junk")];
        let (a, dir) = new_project(&st, "gf-new", &files);
        let r = remote(&st, "gf-new");
        fake.route("POST", "/user/repos", 201, json!({ "clone_url": r, "html_url": "https://github.com/octo-me/gf-new", "private": true }));

        // The grill starts by reading the documents and knows the project has no repo.
        let start = crate::grill::on_user_message(&st, &a, "grill me on the new app", Some(&json!({ "role": "start_request" })), json!({})).unwrap();
        let Routed::Handled { wake: Some(wake), .. } = start else { panic!("a start request wakes the agent") };
        assert!(wake.contains("read the project's existing documents as facts") && wake.contains("no GitHub repo yet"), "{wake}");

        let p = call(&st, &a, "preview_new_repo", json!({})).await.unwrap();
        assert!(p.contains("has no git repo yet. Owners the user can create the repo under: octo-me, acme, members-only, open-org (their account first). Default name: gf-new."), "{p}");
        assert!(p.contains("The first commit: 3 files, 40 B ("), "{p}");
        for f in ["- .gitignore (21 B)", "- README.md (6 B)", "- src/main.rs (13 B)"] {
            assert!(p.contains(f), "missing {f}: {p}");
        }
        assert!(!p.contains("DS_Store (") && !p.contains("today.md"), "junk is left out: {p}");
        assert!(p.contains(r#""text":"Create the GitHub repo `octo-me/gf-new` and publish this folder? The first commit holds 3 files (40 B): `.gitignore`, `README.md`, `src/main.rs`. Loft adds"#), "{p}");
        assert!(p.contains(r#""options":["Yes, private under octo-me","Yes, private under acme","Yes, private under members-only","Yes, private under open-org","No, not now"],"recommendation":"Yes, private under octo-me""#), "{p}");
        assert!(!dir.join(".git").exists() && !dir.join(".gitignore").exists(), "the preview writes nothing");

        let qid = ask(&st, &a, Some("yes"), true);
        let out = call(&st, &a, "create_repo", json!({ "question_id": qid })).await.unwrap();
        assert_eq!(out, "Created the private GitHub repo octo-me/gf-new and pushed the project folder (3 files, 40 B): https://github.com/octo-me/gf-new. Clone it into your workspace for your docs branch; never work in the project folder itself.");
        assert_eq!(creations(&fake, "/user/repos", "gf-new"), [json!({ "name": "gf-new", "private": true })]);
        assert_eq!(pushed(&r), [".gitignore", "README.md", "src/main.rs"]);
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), ".DS_Store\n.remember/\n");
        assert_eq!(git(&dir, &["remote", "get-url", "origin"]), r.to_str().unwrap());
        assert!(dir.join(".DS_Store").exists() && dir.join(".remember/today.md").exists(), "ignored files stay on disk");
        assert!(crate::grill::active(&st, &a.id).is_some(), "the grill goes on");
    }

    #[tokio::test]
    async fn an_organization_owner_and_a_git_repo_without_a_remote() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        let (a, dir) = new_project(&st, "gf-org", &[("a.txt", "a"), (".gitignore", "target\n/.DS_Store")]);
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "mine"]);
        for (p, body) in [("b.txt", "b"), ("target/out", "build"), (".remember/x", "x")] {
            std::fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
            std::fs::write(dir.join(p), body).unwrap();
        }
        let r = remote(&st, "gf-org");
        fake.route("POST", "/orgs/acme/repos", 201, json!({ "clone_url": r, "html_url": "https://github.com/acme/gf-org", "private": true }));

        let p = call(&st, &a, "preview_new_repo", json!({})).await.unwrap();
        assert!(p.contains("is a git repo with no `origin` remote") && p.contains("The first commit: 3 files"), "{p}");
        let qid = ask(&st, &a, Some("Yes, private under acme"), true);

        let err = call(&st, &a, "create_repo", json!({ "question_id": qid, "owner": "locked" })).await.unwrap_err();
        assert_eq!(err, "The user can't create repos under locked. Owners: octo-me, acme, members-only, open-org.");
        assert!(creations(&fake, "/orgs/locked/repos", "gf-org").is_empty());
        let err = call(&st, &a, "create_repo", json!({ "question_id": qid, "owner": "acme", "name": "no spaces" })).await.unwrap_err();
        assert!(err.starts_with("\"no spaces\" isn't a valid repo name"), "{err}");

        let out = call(&st, &a, "create_repo", json!({ "question_id": qid, "owner": "ACME" })).await.unwrap();
        assert!(out.starts_with("Created the private GitHub repo acme/gf-org and pushed the project folder (3 files"), "{out}");
        assert_eq!(creations(&fake, "/orgs/acme/repos", "gf-org"), [json!({ "name": "gf-org", "private": true })]);
        assert_eq!(pushed(&r), [".gitignore", "a.txt", "b.txt"]);
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "target\n/.DS_Store\n.remember/\n", "only the missing rule is added");
        assert_eq!(git(&r, &["log", "--format=%s", "HEAD"]), "Publish the project folder\nmine", "the user's history is kept");
    }

    #[tokio::test]
    async fn declining_keeps_the_grill_going_and_publishing_the_spec_asks_again() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        let (a, dir) = new_project(&st, "gf-no", &[("README.md", "# No\n")]);

        let qid = ask(&st, &a, Some("No, not now"), true);
        let err = call(&st, &a, "create_repo", json!({ "question_id": qid })).await.unwrap_err();
        assert_eq!(err, "The user didn't say yes to creating the repo (their answer: \"No, not now\"), so nothing was created. Keep grilling without it, and ask again in a round before publishing the spec.");
        assert_eq!(crate::grill::active(&st, &a.id).unwrap().status, "active", "the grill goes on");
        assert!(!dir.join(".git").exists() && !dir.join(".gitignore").exists(), "the folder is untouched");
        assert!(creations(&fake, "/user/repos", "gf-no").is_empty());

        // Publishing the spec sends the agent back to the question first.
        let err = crate::planning::mcp_call(&st, &a, "publish_spec", &json!({ "title": "New app", "body": "## Problem Statement\n\nX." })).await.unwrap().unwrap_err();
        assert!(err.contains("isn't a git repo, so there is no GitHub repo to publish to. Ask the user again first") && err.contains("mcp__loft__create_repo"), "{err}");

        // A question left to the agent, a made-up id, a yes still waiting in an unsent round: nothing either.
        let decided = ask(&st, &a, None, false);
        crate::grill::on_user_message(&st, &a, "decide for me", None, json!({})).unwrap();
        crate::grill::tests::send(&st, &a);
        assert!(call(&st, &a, "create_repo", json!({ "question_id": decided })).await.unwrap_err().contains("(their answer: left to you, which isn't a yes)"));
        assert!(call(&st, &a, "create_repo", json!({ "question_id": "nope" })).await.unwrap_err().starts_with("No question with that id"));
        let waiting = ask(&st, &a, Some("yes"), false);
        assert!(call(&st, &a, "create_repo", json!({ "question_id": waiting })).await.unwrap_err().starts_with("The user hasn't sent that round yet"));
        assert!(!dir.join(".git").exists());

        let mut plain = crate::tasks::tests::agent(&st, "gf-plain");
        plain.project = "gf-no".into();
        assert_eq!(call(&st, &plain, "preview_new_repo", json!({})).await.unwrap_err(), "Planning isn't on for you.");
    }

    #[tokio::test]
    async fn a_failed_push_is_retried_on_the_repo_already_created() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        let (a, dir) = new_project(&st, "gf-retry", &[("main.py", "print(1)\n")]);
        let gone = st.cfg.storage_dir.parent().unwrap().join("remotes").join("missing.git");
        fake.route("POST", "/orgs/members-only/repos", 201, json!({ "clone_url": gone, "html_url": "https://github.com/members-only/gf-retry" }));
        let qid = ask(&st, &a, Some("Sim, privado em members-only"), true);

        let err = call(&st, &a, "create_repo", json!({ "question_id": qid, "owner": "members-only" })).await.unwrap_err();
        assert!(err.starts_with("Created members-only/gf-retry on GitHub, but pushing the folder failed: ") && err.ends_with("the repo is reused."), "{err}");
        assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(["remote", "get-url", "origin"]).output().unwrap().status.code() != Some(0), "no remote is left behind");
        assert!(needs_repo(&st, &a));

        let r = remote(&st, "gf-retry");
        fake.route("GET", "/repos/members-only/gf-retry", 200, json!({ "clone_url": r, "html_url": "https://github.com/members-only/gf-retry", "private": true }));
        let out = call(&st, &a, "create_repo", json!({ "question_id": qid, "owner": "members-only" })).await.unwrap();
        assert!(out.starts_with("Created the private GitHub repo members-only/gf-retry"), "{out}");
        assert_eq!(creations(&fake, "/orgs/members-only/repos", "gf-retry").len(), 1, "created once");
        assert_eq!(pushed(&r), [".gitignore", "main.py"]);
        assert_eq!(git(&r, &["rev-list", "--count", "HEAD"]), "1", "the retry doesn't commit again");
        assert!(!needs_repo(&st, &a));
    }

    #[tokio::test]
    async fn the_yes_is_bound_to_the_repo_its_owner_and_its_visibility() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        let (a, dir) = new_project(&st, "gf-bind", &[("app.js", "1"), (".env", "KEY=1"), (".env.example", "KEY="), ("certs/server.pem", "pem")]);
        let r = remote(&st, "gf-bind");
        fake.route("POST", "/orgs/open-org/repos", 201, json!({ "clone_url": r, "html_url": "https://github.com/open-org/gf-bind", "private": false }));
        let create = |qid: &str, args: Value| {
            let (st, a) = (st.clone(), a.clone());
            let mut args = args;
            args["question_id"] = json!(qid);
            async move { call(&st, &a, "create_repo", args).await }
        };

        // Secret-looking files are called out before the yes.
        let p = call(&st, &a, "preview_new_repo", json!({})).await.unwrap();
        assert!(p.contains("publish this folder? Careful: it includes `.env`, `certs/server.pem`, which may hold secrets. The first commit holds"), "{p}");

        // A yes to another question isn't a yes to the repo.
        let other = ask_q(&st, &a, json!({ "text": "Use Postgres?", "speak": "Postgres?", "recommendation": "Yes" }), Some("yes"), true);
        assert!(create(&other, json!({})).await.unwrap_err().starts_with("That question doesn't name the repo `<owner>/gf-bind`"));

        // A picked option fixes its owner and a private repo.
        let acme = ask(&st, &a, Some("Yes, private under acme"), true);
        assert_eq!(create(&acme, json!({})).await.unwrap_err(), "The user answered \"Yes, private under acme\": create it under acme, not octo-me.");
        assert_eq!(create(&acme, json!({ "owner": "acme", "private": false })).await.unwrap_err(), "The user answered \"Yes, private under acme\", which doesn't ask for a public repo: call create_repo without `private: false`, or ask them again if they want it public.");
        let accepted = ask(&st, &a, Some("yes"), true);
        assert!(create(&accepted, json!({ "private": false })).await.unwrap_err().contains("which doesn't ask for a public repo"));
        assert!(create(&accepted, json!({ "owner": "open-org" })).await.unwrap_err().ends_with("create it under octo-me, not open-org."));

        // A comment to discuss is no yes either.
        let discuss = ask(&st, &a, None, false);
        let s = crate::grill::active(&st, &a.id).unwrap();
        crate::grill::on_user_message(&st, &a, "yes, but what goes in it?", Some(&json!({ "role": "answer", "session_id": s.id, "question_id": discuss, "discuss": true })), json!({})).unwrap();
        crate::grill::tests::send(&st, &a);
        assert!(create(&discuss, json!({})).await.unwrap_err().starts_with("The user didn't say yes to creating the repo (their answer: \"yes, but what goes in it?\")"));
        assert!(!dir.join(".git").exists(), "nothing was written");
        for path in ["/user/repos", "/orgs/acme/repos", "/orgs/open-org/repos"] {
            assert!(creations(&fake, path, "gf-bind").is_empty(), "nothing created on {path}");
        }

        // Only an answer the user wrote can ask for a public repo.
        let public = ask(&st, &a, Some("Yes, public under open-org"), true);
        let out = create(&public, json!({ "owner": "open-org", "private": false })).await.unwrap();
        assert!(out.starts_with("Created the public GitHub repo open-org/gf-bind and pushed the project folder (5 files"), "{out}");
        assert_eq!(creations(&fake, "/orgs/open-org/repos", "gf-bind"), [json!({ "name": "gf-bind", "private": false })]);
        assert_eq!(pushed(&r), [".env", ".env.example", ".gitignore", "app.js", "certs/server.pem"]);
    }

    #[tokio::test]
    async fn a_free_text_yes_stays_private_and_a_translated_option_fixes_the_owner_it_names() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        let (a, dir) = new_project(&st, "gf-odd", &[("app.js", "1")]);
        let r = remote(&st, "gf-odd");
        fake.route("POST", "/orgs/acme/repos", 201, json!({ "clone_url": r, "html_url": "https://github.com/acme/gf-odd", "private": true }));
        let create = |qid: &str, args: Value| {
            let (st, a) = (st.clone(), a.clone());
            let mut args = args;
            args["question_id"] = json!(qid);
            async move { call(&st, &a, "create_repo", args).await }
        };

        // A yes in the user's own words that doesn't say public is a private repo under the question's owner.
        for answer in ["bora", "yes go ahead", "pode", "pode, mas não público", "yes, public? no, keep it private", "sim, publique sem pressa"] {
            let qid = ask(&st, &a, Some(answer), true);
            let err = create(&qid, json!({ "private": false })).await.unwrap_err();
            assert!(err.contains("which doesn't ask for a public repo"), "{answer}: {err}");
        }
        let bora = ask(&st, &a, Some("bora"), true);
        assert!(create(&bora, json!({ "owner": "acme" })).await.unwrap_err().ends_with("create it under octo-me, not acme."));
        // Owners match as whole words: "acme-labs" isn't acme, so the question's owner stands.
        let labs = ask(&st, &a, Some("yes, under acme-labs"), true);
        assert!(create(&labs, json!({ "owner": "acme" })).await.unwrap_err().ends_with("create it under octo-me, not acme."));
        let two = ask(&st, &a, Some("yes, acme or open-org"), true);
        assert!(create(&two, json!({ "owner": "acme" })).await.unwrap_err().contains("names more than one owner (acme, open-org)"));

        // An option translated with the owner in the middle still fixes that owner, and a private repo.
        let translated = |answer: Option<&str>| {
            let options: Vec<String> = ["octo-me", "acme", "open-org"].iter().map(|o| format!("Sim, {o}, privado")).chain(["Não, agora não".to_string()]).collect();
            let q = json!({ "text": "Criar o repositório `octo-me/gf-odd` no GitHub e publicar esta pasta?", "speak": "Criar?", "recommendation": "Sim, octo-me, privado", "kind": "single_choice", "options": options });
            ask_q(&st, &a, q, answer, true)
        };
        let no = translated(Some("Não, agora não"));
        assert!(create(&no, json!({})).await.unwrap_err().starts_with("The user didn't say yes"));
        let nao = ask(&st, &a, Some("não, ainda não"), true);
        assert!(create(&nao, json!({})).await.unwrap_err().starts_with("The user didn't say yes"));
        let acme = translated(Some("Sim, acme, privado"));
        assert_eq!(create(&acme, json!({})).await.unwrap_err(), "The user answered \"Sim, acme, privado\": create it under acme, not octo-me.");
        assert!(create(&acme, json!({ "owner": "open-org" })).await.unwrap_err().ends_with("create it under acme, not open-org."));
        assert!(create(&acme, json!({ "owner": "acme", "private": false })).await.unwrap_err().contains("which doesn't ask for a public repo"));
        let recommended = translated(Some("yes"));
        assert!(create(&recommended, json!({ "owner": "acme" })).await.unwrap_err().ends_with("create it under octo-me, not acme."));
        assert!(!dir.join(".git").exists(), "nothing was written");
        for path in ["/user/repos", "/orgs/acme/repos", "/orgs/open-org/repos"] {
            assert!(creations(&fake, path, "gf-odd").is_empty(), "nothing created on {path}");
        }

        let out = create(&acme, json!({ "owner": "acme" })).await.unwrap();
        assert!(out.starts_with("Created the private GitHub repo acme/gf-odd"), "{out}");
        assert_eq!(creations(&fake, "/orgs/acme/repos", "gf-odd"), [json!({ "name": "gf-odd", "private": true })]);
    }

    #[tokio::test]
    async fn a_project_with_a_remote_is_left_alone() {
        let (fake, st) = (fake_github(), state());
        owners_on(&fake);
        for (name, origin, want) in [
            ("gf-done", "git@github.com:octo/gf-done.git", "The project is already on GitHub as octo/gf-done: there is nothing to create, publish there."),
            ("gf-gitlab", "https://gitlab.com/octo/x.git", "The project's `origin` (https://gitlab.com/octo/x.git) isn't on GitHub, and Loft only creates GitHub repos."),
        ] {
            let (a, dir) = new_project(&st, name, &[("x", "x")]);
            git(&dir, &["init", "-q"]);
            git(&dir, &["remote", "add", "origin", origin]);
            assert!(!needs_repo(&st, &a));
            assert_eq!(call(&st, &a, "preview_new_repo", json!({})).await.unwrap_err(), want);
            let start = crate::grill::on_user_message(&st, &a, "grill me", Some(&json!({ "role": "start_request" })), json!({})).unwrap();
            let Routed::Handled { wake: Some(wake), .. } = start else { panic!() };
            assert!(!wake.contains("no GitHub repo yet"));
        }
    }

    #[test]
    fn the_planning_prompt_covers_a_new_project() {
        let st = state();
        let p = crate::grill::prompt_section(&planner(&st, "gf-prompt"));
        for want in [
            "# Planning: new project", "Create the GitHub repo `<owner>/<project>` and publish this folder?", "mcp__loft__preview_new_repo", "mcp__loft__create_repo",
            "never write into it", "ask the question again", "read the project's existing documents",
        ] {
            assert!(p.contains(want), "missing {want}");
        }
    }
}
