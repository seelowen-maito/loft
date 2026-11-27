//! GitHub Actions minutes: how long the workflow runs of a branch took, each job rounded up to the minute as
//! GitHub bills it. A Delivery keeps its branch's runs, so its card and its done summary say what it spent.
//!
//! Each owner (an org or an account) has its month too: the sum of its private repos Loft knows, weighted as GitHub
//! uses up included minutes (macOS jobs 10×, Windows 2×), or the org's own billed minutes when the token may read them (`admin:org`), against a quota the user sets in Settings. Reaching 70%
//! and 90% of it is a Needs you item each, once a month.

use crate::api::{ApiResult, bad};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Runs and jobs per page, GitHub's most.
const PER_PAGE: usize = 100;

/// Pages read at most from one listing: 1,000 runs, or jobs in one run.
const MAX_PAGES: usize = 10;

/// What one workflow run took, as a Delivery keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunMinutes {
    /// The run's attempt when it was read: a rerun is read again.
    pub attempt: u32,
    /// Its jobs' minutes, every attempt's, each finished job rounded up.
    pub minutes: u32,
    /// The run had completed: it isn't read again until a rerun.
    pub done: bool,
    /// `minutes` as they use up an owner's included minutes: each job's times its runner's [`job_weight`].
    #[serde(default)]
    pub weighted: u32,
}

/// The total of `runs`: the time their jobs took.
pub fn total(runs: &BTreeMap<String, RunMinutes>) -> u32 {
    runs.values().map(|r| r.minutes).sum()
}

/// The weighted total of `runs`: what they used up of their owner's included minutes.
pub fn weighted_total(runs: &BTreeMap<String, RunMinutes>) -> u32 {
    runs.values().map(|r| r.weighted).sum()
}

/// How many of an owner's included minutes one minute of `job` uses up: 10 on a macOS runner, 2 on Windows, 1 on
/// Linux, by the runner's labels.
pub fn job_weight(job: &Value) -> u32 {
    let labels = job["labels"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect::<Vec<_>>();
    if labels.iter().any(|l| l.starts_with("macos")) {
        10
    } else if labels.iter().any(|l| l.starts_with("windows")) {
        2
    } else {
        1
    }
}

/// The minutes GitHub bills for `job`: its time rounded up to the minute. A skipped job, one still running and one
/// on a self-hosted runner cost none.
pub fn job_minutes(job: &Value) -> u32 {
    if job["conclusion"] == "skipped" || job["labels"].as_array().is_some_and(|l| l.iter().any(|x| x == "self-hosted")) {
        return 0;
    }
    let at = |k: &str| job[k].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let (Some(start), Some(end)) = (at("started_at"), at("completed_at")) else { return 0 };
    let secs = end.signed_duration_since(start).num_seconds().max(0);
    u32::try_from((secs + 59) / 60).unwrap_or(u32::MAX)
}

/// The workflow runs of `branch` on `repo` created since `since` (RFC 3339), with what each took: `known`, with the
/// runs read now added or replaced. A completed run `known` has at the same attempt isn't read again, and a run
/// GitHub no longer lists keeps what it took.
pub async fn of_branch(state: &AppState, repo: &str, branch: &str, since: &str, known: &BTreeMap<String, RunMinutes>) -> anyhow::Result<BTreeMap<String, RunMinutes>> {
    of_runs(state, repo, &format!("branch={}&", encode(branch)), since, known).await
}

/// [`of_branch`] for the runs `filter` (`key=value&`, or empty for all of them) lists.
async fn of_runs(state: &AppState, repo: &str, filter: &str, since: &str, known: &BTreeMap<String, RunMinutes>) -> anyhow::Result<BTreeMap<String, RunMinutes>> {
    let mut out = known.clone();
    let created = since.get(..10).map(|day| format!("created=%3E%3D{day}&")).unwrap_or_default();
    for page in 1..=MAX_PAGES {
        let v = crate::github::get(state, &format!("/repos/{repo}/actions/runs?{filter}{created}per_page={PER_PAGE}&page={page}")).await?;
        let runs = v["workflow_runs"].as_array().cloned().unwrap_or_default();
        for r in &runs {
            let Some(id) = r["id"].as_i64() else { continue };
            let attempt = r["run_attempt"].as_u64().and_then(|a| u32::try_from(a).ok()).unwrap_or(1);
            if known.get(&id.to_string()).is_some_and(|k| k.done && k.attempt == attempt) {
                continue;
            }
            let (minutes, weighted) = run_minutes(state, repo, id).await?;
            out.insert(id.to_string(), RunMinutes { attempt, minutes, done: r["status"] == "completed", weighted });
        }
        let listed = v["total_count"].as_u64().map_or(usize::MAX, |n| n as usize);
        if runs.len() < PER_PAGE || page * PER_PAGE >= listed {
            break;
        }
    }
    Ok(out)
}

/// The minutes of run `id`'s jobs, every attempt's, and weighted (see [`job_weight`]).
async fn run_minutes(state: &AppState, repo: &str, id: i64) -> anyhow::Result<(u32, u32)> {
    let (mut minutes, mut weighted) = (0, 0);
    for page in 1..=MAX_PAGES {
        let v = crate::github::get(state, &format!("/repos/{repo}/actions/runs/{id}/jobs?filter=all&per_page={PER_PAGE}&page={page}")).await?;
        let jobs = v["jobs"].as_array().cloned().unwrap_or_default();
        minutes += jobs.iter().map(job_minutes).sum::<u32>();
        weighted += jobs.iter().map(|j| job_minutes(j).saturating_mul(job_weight(j))).sum::<u32>();
        let listed = v["total_count"].as_u64().map_or(usize::MAX, |n| n as usize);
        if jobs.len() < PER_PAGE || page * PER_PAGE >= listed {
            break;
        }
    }
    Ok((minutes, weighted))
}

// ---- Each owner's month, against its quota ----

/// How often each repo's month, and each org's billing, is read again.
const MONTH_EVERY: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// How long a repo's own facts (private or not, whose) are kept before they're read again.
const META_HOURS: i64 = 6;

/// How far back from the last read the next one lists runs: GitHub lists a run by the day it was created, and one
/// queued and run for longer than this isn't read again. A rerun of an older run isn't either.
const OVERLAP_DAYS: i64 = 2;

/// The warning lines, in percent of an owner's quota.
pub const LINES: [u32; 2] = [70, 90];

/// The largest quota, in minutes.
const MAX_QUOTA: u64 = 1_000_000;

/// A repo's Actions minutes in a month, as Loft keeps it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoMonth {
    /// Only a private repo's minutes are billed, so a public one's runs aren't read.
    #[serde(default)]
    pub private: bool,
    /// Its owner's kind on GitHub: `Organization` or `User`.
    #[serde(default)]
    pub owner_type: String,
    /// When `private` and `owner_type` were read (RFC 3339).
    #[serde(default)]
    pub meta_at: String,
    /// The month's runs, by id.
    #[serde(default)]
    pub runs: BTreeMap<String, RunMinutes>,
    /// When its runs were last read (RFC 3339), empty before the first read.
    #[serde(default)]
    pub read_at: String,
}

/// What Loft keeps of an owner: the quota the user set, the org's billed minutes, and the lines reached.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Owner {
    /// Minutes a month; none set, no warnings.
    #[serde(default)]
    pub quota: Option<u32>,
    /// The org's minutes as GitHub bills them, read when the token may (`admin:org`).
    #[serde(default)]
    pub billed: Option<Billed>,
    /// `YYYY-MM:<line>` -> when the month's minutes first reached that line of the quota (RFC 3339).
    #[serde(default)]
    pub crossed: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Billed {
    pub month: String,
    pub minutes: u32,
}

/// An owner's month: its minutes, where they come from, and its quota.
#[derive(Debug, Clone, Default)]
pub struct OwnerMonth {
    pub owner: String,
    /// `Organization` or `User`; empty when no repo of it was read yet.
    pub kind: String,
    /// The month's minutes: `billed` when GitHub gave the org's own, else the sum of its private repos.
    pub minutes: u32,
    pub billed: bool,
    /// Its repos read this month: (`owner/repo`, minutes, private).
    pub repos: Vec<(String, u32, bool)>,
    pub quota: Option<u32>,
    pub crossed: BTreeMap<String, String>,
}

/// The month `now` is in, `YYYY-MM`, as GitHub bills it (UTC).
pub fn month_of(now: DateTime<Utc>) -> String {
    now.format("%Y-%m").to_string()
}

fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0).single().unwrap_or(now)
}

fn owner_of(repo: &str) -> &str {
    repo.split('/').next().unwrap_or(repo)
}

fn at(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

/// `minutes` reached `line` percent of `quota`.
fn reached(minutes: u32, quota: u32, line: u32) -> bool {
    u64::from(minutes) * 100 >= u64::from(quota) * u64::from(line)
}

/// What was read of `repo` in `month`.
pub fn repo_month(state: &AppState, repo: &str, month: &str) -> Option<RepoMonth> {
    state.db.actions_month(repo, month).ok().flatten().and_then(|d| serde_json::from_str(&d).ok())
}

fn owners(state: &AppState) -> BTreeMap<String, Owner> {
    state.db.actions_owners().unwrap_or_default().into_iter().map(|(o, d)| (o, serde_json::from_str(&d).unwrap_or_default())).collect()
}

fn save_owner(state: &AppState, owner: &str, o: &Owner) {
    if let Err(e) = serde_json::to_string(o).map_err(anyhow::Error::from).and_then(|d| state.db.save_actions_owner(owner, &d)) {
        tracing::warn!("actions: saving owner {owner}: {e}");
    }
}

/// Each owner's month at `now`: the owners of the repos read this month, and those with a quota.
pub fn months(state: &AppState, now: DateTime<Utc>) -> Vec<OwnerMonth> {
    let month = month_of(now);
    let saved = owners(state);
    let mut by: BTreeMap<String, OwnerMonth> = BTreeMap::new();
    for (repo, data) in state.db.actions_months(&month).unwrap_or_default() {
        let Ok(m) = serde_json::from_str::<RepoMonth>(&data) else { continue };
        let o = by.entry(owner_of(&repo).to_string()).or_default();
        let minutes = if m.private { weighted_total(&m.runs) } else { 0 };
        if m.private {
            o.minutes += minutes;
        }
        if !m.owner_type.is_empty() {
            o.kind = m.owner_type.clone();
        }
        o.repos.push((repo, minutes, m.private));
    }
    for (owner, o) in &saved {
        if o.quota.is_some() {
            by.entry(owner.clone()).or_default();
        }
    }
    by.into_iter()
        .map(|(owner, mut u)| {
            if let Some(o) = saved.get(&owner) {
                if let Some(b) = o.billed.as_ref().filter(|b| b.month == month && u.kind == "Organization") {
                    (u.minutes, u.billed) = (b.minutes, true);
                }
                (u.quota, u.crossed) = (o.quota, o.crossed.clone());
            }
            u.owner = owner;
            u
        })
        .collect()
}

/// The repos Loft knows on GitHub: its projects' `origin`s.
async fn known_repos(state: &AppState) -> BTreeSet<String> {
    crate::repos::list(state, false).await.into_iter().filter_map(|r| r.github).collect()
}

/// `key` wasn't read in the last `MONTH_EVERY`; it counts as read now.
fn due(state: &AppState, key: &str) -> bool {
    let mut rt = state.rt.lock().unwrap();
    let due = rt.actions_month_read.get(key).is_none_or(|t| t.elapsed() >= MONTH_EVERY);
    if due {
        rt.actions_month_read.insert(key.to_string(), std::time::Instant::now());
    }
    due
}

/// Reads the month of each repo Loft knows, and each org's billing, every `MONTH_EVERY`, then notes the quota lines
/// reached.
pub async fn poller(state: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    crate::github::as_source("actions", async {
        loop {
            tick.tick().await;
            follow(&state, Utc::now()).await;
        }
    })
    .await
}

/// One tick of [`poller`], as at `now`.
pub(crate) async fn follow(state: &AppState, now: DateTime<Utc>) {
    let month = month_of(now);
    let mut orgs = BTreeSet::new();
    for repo in known_repos(state).await {
        if due(state, &repo)
            && let Err(e) = read_repo(state, &repo, now).await
            && !crate::github::is_paused(&e)
        {
            tracing::warn!("actions: {repo}: {e}");
        }
        if repo_month(state, &repo, &month).is_some_and(|m| m.private && m.owner_type == "Organization") {
            orgs.insert(owner_of(&repo).to_string());
        }
    }
    for org in orgs {
        if due(state, &format!("org:{org}")) {
            read_billed(state, &org, now).await;
        }
    }
    note_crossed(state, now);
}

/// Reads `repo`'s runs this month since a little before its last read, and its own facts when they're old.
async fn read_repo(state: &AppState, repo: &str, now: DateTime<Utc>) -> anyhow::Result<()> {
    let month = month_of(now);
    let mut m = repo_month(state, repo, &month).unwrap_or_default();
    if at(&m.meta_at).is_none_or(|t| now - t >= chrono::Duration::hours(META_HOURS)) {
        let r = crate::github::get(state, &format!("/repos/{repo}")).await?;
        m.private = r["private"].as_bool().unwrap_or(false);
        m.owner_type = r["owner"]["type"].as_str().unwrap_or_default().to_string();
        m.meta_at = now.to_rfc3339();
    }
    if m.private {
        let since = at(&m.read_at).map_or(month_start(now), |t| (t - chrono::Duration::days(OVERLAP_DAYS)).max(month_start(now)));
        m.runs = of_runs(state, repo, "", &since.to_rfc3339(), &m.runs).await?;
    }
    m.read_at = now.to_rfc3339();
    state.db.save_actions_month(repo, &month, &serde_json::to_string(&m)?)?;
    Ok(())
}

/// GitHub turned the request down for good: the token may not read it, or it isn't there.
fn refused(e: &anyhow::Error) -> bool {
    let m = e.to_string();
    crate::github::is_gone(e) || ((m.starts_with("GitHub 403") || m.starts_with("GitHub 401")) && !m.contains("rate limit"))
}

/// Reads `org`'s billed minutes this month: kept when the token may read them, dropped when it may not, and left as
/// they were on any other error.
async fn read_billed(state: &AppState, org: &str, now: DateTime<Utc>) {
    let got = billed_minutes(state, org, now).await;
    let mut o = owners(state).remove(org).unwrap_or_default();
    o.billed = match got {
        Ok(minutes) => minutes.map(|minutes| Billed { month: month_of(now), minutes }),
        Err(e) => {
            if !crate::github::is_paused(&e) {
                tracing::warn!("actions: billing of {org}: {e}");
            }
            return;
        }
    };
    save_owner(state, org, &o);
}

/// `org`'s Actions minutes this month as GitHub bills them; none when the token may not read its billing.
async fn billed_minutes(state: &AppState, org: &str, now: DateTime<Utc>) -> anyhow::Result<Option<u32>> {
    let minutes = |q: f64| u32::try_from(q.max(0.0).ceil() as u64).unwrap_or(u32::MAX);
    let usage = format!("/organizations/{org}/settings/billing/usage?year={}&month={}", now.year(), now.month());
    match crate::github::get(state, &usage).await {
        Ok(v) => {
            let items = v["usageItems"].as_array().cloned().unwrap_or_default();
            let is = |i: &Value, k: &str, want: &str| i[k].as_str().is_some_and(|s| s.eq_ignore_ascii_case(want));
            Ok(Some(minutes(items.iter().filter(|i| is(i, "product", "actions") && is(i, "unitType", "minutes")).filter_map(|i| i["quantity"].as_f64()).sum())))
        }
        // An org not on GitHub's newer billing answers the older endpoint.
        Err(e) if crate::github::is_gone(&e) => match crate::github::get(state, &format!("/orgs/{org}/settings/billing/actions")).await {
            Ok(v) => Ok(v["total_minutes_used"].as_f64().map(minutes)),
            Err(e) if refused(&e) => Ok(None),
            Err(e) => Err(e),
        },
        Err(e) if refused(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Notes when each owner's month first reached each line of its quota, and forgets the lines of other months.
pub(crate) fn note_crossed(state: &AppState, now: DateTime<Utc>) {
    let month = month_of(now);
    for u in months(state, now) {
        let mut o = owners(state).remove(&u.owner).unwrap_or_default();
        let before = o.crossed.clone();
        o.crossed.retain(|k, _| k.starts_with(&format!("{month}:")));
        if let Some(quota) = o.quota {
            for line in LINES.into_iter().filter(|&l| reached(u.minutes, quota, l)) {
                o.crossed.entry(format!("{month}:{line}")).or_insert_with(|| now.to_rfc3339());
            }
        }
        if o.crossed != before {
            save_owner(state, &u.owner, &o);
        }
    }
}

/// An owner over a line of its quota this month, for Needs you.
pub struct Warning {
    pub owner: String,
    pub month: String,
    pub line: u32,
    pub minutes: u32,
    pub quota: u32,
    pub billed: bool,
    /// When the month's minutes reached the line.
    pub at: String,
}

/// The owners over a line of their quota: the highest line each reached this month and is still over.
pub fn warnings(state: &AppState, now: DateTime<Utc>) -> Vec<Warning> {
    let month = month_of(now);
    months(state, now)
        .into_iter()
        .filter_map(|u| {
            let quota = u.quota?;
            let line = LINES.into_iter().rev().find(|&l| reached(u.minutes, quota, l) && u.crossed.contains_key(&format!("{month}:{l}")))?;
            let at = u.crossed[&format!("{month}:{line}")].clone();
            Some(Warning { owner: u.owner, month: month.clone(), line, minutes: u.minutes, quota, billed: u.billed, at })
        })
        .collect()
}

fn month_json(u: &OwnerMonth) -> Value {
    json!({
        "owner": u.owner, "kind": u.kind, "minutes": u.minutes, "source": if u.billed { "billing" } else { "repos" }, "quota": u.quota,
        "repos": u.repos.iter().map(|(repo, minutes, private)| json!({ "repo": repo, "minutes": minutes, "private": private })).collect::<Vec<_>>(),
    })
}

fn months_json(state: &AppState, now: DateTime<Utc>) -> Value {
    json!({ "month": month_of(now), "owners": months(state, now).iter().map(month_json).collect::<Vec<_>>() })
}

/// `GET /api/actions/owners`: each owner's Actions minutes this month and its quota.
pub async fn owners_get(State(state): State<AppState>) -> Json<Value> {
    Json(months_json(&state, Utc::now()))
}

/// `PUT /api/actions/owners/{owner}` with `{ "quota": <minutes> }`, or `null` for none: sets the owner's monthly quota.
/// Every owner's month after it.
pub async fn set_quota(State(state): State<AppState>, Path(owner): Path<String>, Json(body): Json<Value>) -> ApiResult<Value> {
    let owner = owner.trim();
    if owner.is_empty() || owner.len() > 39 || !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(bad(format!("`{owner}` isn't a GitHub account or org.")));
    }
    let quota = match &body["quota"] {
        Value::Null => None,
        q => match q.as_u64() {
            Some(0) => None,
            Some(n) if n <= MAX_QUOTA => u32::try_from(n).ok(),
            _ => return Err(bad("The quota is a whole number of minutes, up to 1,000,000, or empty for none.")),
        },
    };
    let now = Utc::now();
    let mut o = owners(&state).remove(owner).unwrap_or_default();
    o.quota = quota;
    save_owner(&state, owner, &o);
    note_crossed(&state, now);
    Ok(Json(months_json(&state, now)))
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    project: String,
}

/// `GET /api/actions/project?project=<name>`: the project's repo minutes this month, and its owner's month and quota.
pub async fn project(State(state): State<AppState>, Query(q): Query<ProjectQuery>) -> ApiResult<Value> {
    let project = crate::project_settings::known_project(&state, &q.project)?;
    let repo = match crate::repos::github_repo(&state, &project).await {
        Ok(r) => r,
        Err(why) => return Ok(Json(json!({ "project": project, "repo": null, "why": format!("{why}.") }))),
    };
    let now = Utc::now();
    let m = repo_month(&state, &repo, &month_of(now));
    let owner = months(&state, now).into_iter().find(|u| u.owner == owner_of(&repo)).unwrap_or_else(|| OwnerMonth { owner: owner_of(&repo).to_string(), ..Default::default() });
    Ok(Json(json!({
        "project": project, "repo": repo, "month": month_of(now), "read": m.is_some(),
        "private": m.as_ref().map(|m| m.private), "minutes": m.as_ref().filter(|m| m.private).map_or(0, |m| weighted_total(&m.runs)),
        "owner": month_json(&owner),
    })))
}

/// `branch` for a query string: a branch name can hold `/`, `#`, `&` or `+`.
fn encode(branch: &str) -> String {
    branch.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_job_is_billed_rounded_up_to_the_minute() {
        let job = |start: &str, end: Value| json!({ "conclusion": "success", "started_at": start, "completed_at": end, "labels": ["ubuntu-latest"] });
        assert_eq!(job_minutes(&job("2026-11-24T10:00:00Z", json!("2026-11-24T10:00:01Z"))), 1);
        assert_eq!(job_minutes(&job("2026-11-24T10:00:00Z", json!("2026-11-24T10:01:00Z"))), 1);
        assert_eq!(job_minutes(&job("2026-11-24T10:00:00Z", json!("2026-11-24T10:01:01Z"))), 2);
        assert_eq!(job_minutes(&job("2026-11-24T10:00:00Z", json!("2026-11-24T10:00:00Z"))), 0);
        assert_eq!(job_minutes(&job("2026-11-24T10:00:00Z", Value::Null)), 0, "still running");
        let mut skipped = job("2026-11-24T10:00:00Z", json!("2026-11-24T10:00:03Z"));
        skipped["conclusion"] = json!("skipped");
        assert_eq!(job_minutes(&skipped), 0);
        let mut own = job("2026-11-24T10:00:00Z", json!("2026-11-24T10:05:00Z"));
        own["labels"] = json!(["self-hosted", "macOS"]);
        assert_eq!(job_minutes(&own), 0, "GitHub doesn't bill a self-hosted runner");
    }

    #[tokio::test]
    async fn an_owners_month_weights_macos_and_windows_jobs_as_github_uses_up_included_minutes() {
        assert_eq!([json!(["ubuntu-latest"]), json!(["macos-14"]), json!(["macOS-latest-xlarge"]), json!(["windows-2022"]), json!(null)].iter().map(|l| job_weight(&json!({ "labels": l }))).collect::<Vec<_>>(), vec![1, 10, 10, 2, 1]);
        let st = crate::tasks::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "app", Some("https://github.com/mobile/app.git"));
        let repo = "mobile/app";
        fake.route("GET", &format!("/repos/{repo}"), 200, json!({ "private": true, "owner": { "type": "User" } }));
        fake.route("GET", &format!("/repos/{repo}/actions/runs"), 200, json!({ "total_count": 1, "workflow_runs": [run(1)] }));
        let job = |secs: i64, label: &str| json!({ "conclusion": "success", "started_at": "2026-11-24T10:00:00Z", "completed_at": (chrono::DateTime::parse_from_rfc3339("2026-11-24T10:00:00Z").unwrap() + chrono::Duration::seconds(secs)).to_rfc3339(), "labels": [label] });
        // 2 min on Linux, 2 on macOS, 3 on Windows: 7 minutes of jobs, 2 + 20 + 6 of the included ones.
        fake.route("GET", &format!("/repos/{repo}/actions/runs/1/jobs"), 200, json!({ "total_count": 3, "jobs": [job(120, "ubuntu-latest"), job(120, "macos-14"), job(180, "windows-latest")] }));
        let out = of_branch(&st, repo, "spec/1-x", "2026-11-24T09:00:00Z", &BTreeMap::new()).await.unwrap();
        assert_eq!((total(&out), weighted_total(&out)), (7, 28));
        follow(&st, Utc::now()).await;
        let v = owners_now(&st).await;
        assert_eq!((&owner(&v, "mobile")["minutes"], &owner(&v, "mobile")["repos"][0]["minutes"]), (&json!(28), &json!(28)));
    }

    /// A finished job of a minute.
    fn minute() -> Value {
        json!({ "conclusion": "success", "started_at": "2026-11-24T10:00:00Z", "completed_at": "2026-11-24T10:01:00Z", "labels": ["ubuntu-latest"] })
    }

    /// A completed run, first attempt.
    fn run(id: i64) -> Value {
        json!({ "id": id, "run_attempt": 1, "status": "completed" })
    }

    #[tokio::test]
    async fn a_branch_over_one_page_of_runs_or_jobs_counts_every_page() {
        let st = crate::github::tests::state_with_token();
        let fake = crate::github::tests::own_fake(&st);
        let repo = "octo/many-runs";
        let runs = format!("/repos/{repo}/actions/runs");
        let jobs = |id: i64| format!("/repos/{repo}/actions/runs/{id}/jobs");
        // 130 runs over two pages; GitHub's total_count can be more than it lists, so the short page ends the listing.
        let page = |ids: std::ops::RangeInclusive<i64>| json!({ "total_count": 1000, "workflow_runs": ids.map(run).collect::<Vec<_>>() });
        fake.in_turn("GET", &runs, vec![(200, page(1..=100)), (200, page(101..=130)), (200, page(500..=500))]);
        for id in 2..=130 {
            fake.route("GET", &jobs(id), 200, json!({ "total_count": 1, "jobs": [minute()] }));
        }
        // Run 1 has 105 jobs over two pages.
        let jobs_page = |n: usize| json!({ "total_count": 105, "jobs": vec![minute(); n] });
        fake.in_turn("GET", &jobs(1), vec![(200, jobs_page(100)), (200, jobs_page(5)), (200, jobs_page(100))]);

        let out = of_branch(&st, repo, "spec/1-x", "2026-11-24T09:00:00Z", &BTreeMap::new()).await.unwrap();
        assert_eq!(out.len(), 130);
        assert_eq!(out["1"], RunMinutes { attempt: 1, minutes: 105, done: true, weighted: 105 });
        assert_eq!(total(&out), 105 + 129);
        let listed: Vec<_> = fake.seen(&format!("{runs}?")).into_iter().map(|s| s.path).collect();
        assert!(listed.len() == 2 && listed[0].ends_with("&page=1") && listed[1].ends_with("&page=2"), "{listed:?}");
        let read: Vec<_> = fake.seen(&format!("{}?", jobs(1))).into_iter().map(|s| s.path).collect();
        assert!(read.len() == 2 && read[1].ends_with("&page=2"), "{read:?}");

        // A full page that is all GitHub has ends the listing too.
        let fake = crate::github::tests::own_fake(&st);
        fake.in_turn("GET", &runs, vec![(200, json!({ "total_count": 100, "workflow_runs": (1..=100).map(run).collect::<Vec<_>>() })), (200, page(500..=500))]);
        for id in 1..=100 {
            fake.route("GET", &jobs(id), 200, json!({ "total_count": 1, "jobs": [minute()] }));
        }
        let out = of_branch(&st, repo, "spec/1-x", "2026-11-24T09:00:00Z", &BTreeMap::new()).await.unwrap();
        assert_eq!((out.len(), total(&out)), (100, 100));
        assert_eq!(fake.seen(&format!("{runs}?")).len(), 1);
    }

    // ---- Each owner's month (#472) ----

    /// A project folder in the projects folder with this `origin`, if any.
    fn project(st: &AppState, name: &str, origin: Option<&str>) {
        let dir = st.settings.boot().projects_dir.join(name);
        assert!(dir.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        if let Some(o) = origin {
            git(&["remote", "add", "origin", o]);
        }
    }

    /// A repo on the fake: private or not, whose, and its runs this month with their jobs' seconds.
    fn repo_on(fake: &crate::github::tests::Fake, repo: &str, private: bool, kind: &str, runs: &[(i64, &[i64])]) {
        fake.route("GET", &format!("/repos/{repo}"), 200, json!({ "private": private, "owner": { "type": kind } }));
        let listed: Vec<Value> = runs.iter().map(|(id, _)| run(*id)).collect();
        fake.route("GET", &format!("/repos/{repo}/actions/runs"), 200, json!({ "total_count": listed.len(), "workflow_runs": listed }));
        for (id, secs) in runs {
            let start = chrono::DateTime::parse_from_rfc3339("2026-11-24T10:00:00Z").unwrap();
            let jobs: Vec<Value> = secs.iter().map(|s| json!({ "conclusion": "success", "started_at": start.to_rfc3339(), "completed_at": (start + chrono::Duration::seconds(*s)).to_rfc3339() })).collect();
            fake.route("GET", &format!("/repos/{repo}/actions/runs/{id}/jobs"), 200, json!({ "total_count": jobs.len(), "jobs": jobs }));
        }
    }

    async fn owners_now(st: &AppState) -> Value {
        owners_get(State(st.clone())).await.0
    }

    fn owner<'a>(v: &'a Value, name: &str) -> &'a Value {
        v["owners"].as_array().unwrap().iter().find(|o| o["owner"] == name).unwrap_or_else(|| panic!("no {name} in {v:#}"))
    }

    async fn quota(st: &AppState, owner: &str, q: Value) -> Result<Value, (axum::http::StatusCode, String)> {
        set_quota(State(st.clone()), Path(owner.to_string()), Json(json!({ "quota": q }))).await.map(|j| j.0).map_err(|e| (e.0, e.1))
    }

    /// The Needs you items about Actions quotas: (id, title, reason).
    fn quota_items(st: &AppState) -> Vec<(String, String, String)> {
        let v = crate::needs_you::snapshot(st).unwrap();
        v["items"].as_array().unwrap().iter().filter(|i| i["type"] == "actions_quota").map(|i| (i["id"].as_str().unwrap().into(), i["title"].as_str().unwrap().into(), i["reason"].as_str().unwrap().into())).collect()
    }

    /// Reads every repo and org again at once, as if `MONTH_EVERY` had passed.
    fn again(st: &AppState) {
        st.rt.lock().unwrap().actions_month_read.clear();
    }

    #[tokio::test]
    async fn an_owners_month_sums_its_private_repos_and_warns_at_70_and_90_percent_of_its_quota_once_a_month() {
        let st = crate::tasks::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "acme-api", Some("https://github.com/acme/api.git"));
        project(&st, "acme-site", Some("git@github.com:acme/site.git"));
        project(&st, "tool", Some("https://github.com/solo-dev/tool"));
        project(&st, "scratch", None);
        // 2 min 1 s is 3 minutes, and 4 more; the public site's runs are free, so they aren't read.
        repo_on(&fake, "acme/api", true, "Organization", &[(1, &[121, 240])]);
        repo_on(&fake, "acme/site", false, "Organization", &[(9, &[600])]);
        repo_on(&fake, "solo-dev/tool", true, "User", &[(5, &[30])]);
        // The token may not read the org's billing: its month is the sum of its repos.
        fake.route("GET", "/organizations/acme/settings/billing/usage", 403, json!({ "message": "Must have admin rights" }));

        let now = Utc::now();
        let month = month_of(now);
        follow(&st, now).await;
        let v = owners_now(&st).await;
        assert_eq!(v["month"], json!(month));
        let acme = owner(&v, "acme");
        assert_eq!((&acme["minutes"], &acme["source"], &acme["kind"], &acme["quota"]), (&json!(7), &json!("repos"), &json!("Organization"), &Value::Null));
        assert_eq!(acme["repos"], json!([{ "repo": "acme/api", "minutes": 7, "private": true }, { "repo": "acme/site", "minutes": 0, "private": false }]));
        assert_eq!((&owner(&v, "solo-dev")["minutes"], &owner(&v, "solo-dev")["kind"]), (&json!(1), &json!("User")));
        assert!(fake.seen("/repos/acme/site/actions").is_empty(), "a public repo's runs aren't read");
        let listed = fake.seen("/repos/acme/api/actions/runs?")[0].path.clone();
        assert_eq!(listed, format!("/repos/acme/api/actions/runs?created=%3E%3D{month}-01&per_page=100&page=1"));
        assert!(fake.seen("/orgs/").is_empty(), "a 403 isn't the older billing's cue");
        assert!(fake.seen("/organizations/solo-dev").is_empty(), "an account has no org billing");
        assert!(quota_items(&st).is_empty(), "no quota, no warning");

        // Within `MONTH_EVERY` nothing is read again.
        let reads = fake.seen("/repos/").len();
        follow(&st, now).await;
        assert_eq!(fake.seen("/repos/").len(), reads);

        // A quota the minutes are past 70% of: one item, with a push, at once.
        assert_eq!(quota(&st, "acme", json!(0.5)).await.unwrap_err().0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(quota(&st, "acme", json!(2_000_000)).await.unwrap_err().1, "The quota is a whole number of minutes, up to 1,000,000, or empty for none.");
        assert_eq!(quota(&st, "acme/../x", json!(10)).await.unwrap_err().1, "`acme/../x` isn't a GitHub account or org.");
        let v = quota(&st, "acme", json!(10)).await.unwrap();
        assert_eq!(owner(&v, "acme")["quota"], json!(10));
        let id70 = format!("actions_quota:acme:{month}:70");
        assert_eq!(quota_items(&st), vec![(id70.clone(), "acme used 70% of its GitHub Actions minutes".into(), "7 of 10 minutes this month, past 70% of the quota.".into())]);
        let item = crate::needs_you::collect(&st).unwrap().into_iter().find(|i| i.kind == "actions_quota").unwrap();
        let push = crate::needs_you::to_push(&item);
        assert_eq!((push.title.as_str(), push.kind), ("GitHub Actions: acme at 70% of its quota", "delivery_stalled"), "phones get it with Delivery stalled");

        // Two minutes more: 90% takes 70%'s place.
        repo_on(&fake, "acme/api", true, "Organization", &[(1, &[121, 240]), (2, &[90])]);
        again(&st);
        follow(&st, now).await;
        let id90 = format!("actions_quota:acme:{month}:90");
        assert_eq!(quota_items(&st).into_iter().map(|i| i.0).collect::<Vec<_>>(), vec![id90.clone()]);
        // The second read lists runs from a little before the first, and doesn't read a completed run again.
        assert!(fake.seen("/repos/acme/api/actions/runs?").last().unwrap().path.contains(&format!("created=%3E%3D{month}-")));
        assert_eq!(fake.seen("/repos/acme/api/actions/runs/1/jobs").len(), 1);

        // Done: it doesn't come back this month, however often the month is read.
        let _ = crate::needs_you::dismiss_item(State(st.clone()), Path(id90.clone())).await.unwrap();
        again(&st);
        follow(&st, now).await;
        assert!(quota_items(&st).is_empty());
        let crossed = |st: &AppState| owners(st)["acme"].crossed.keys().cloned().collect::<Vec<_>>();
        assert_eq!(crossed(&st), vec![format!("{month}:70"), format!("{month}:90")]);

        // A higher quota: under the lines, nothing to warn about.
        quota(&st, "acme", json!(100)).await.unwrap();
        assert!(warnings(&st, now).is_empty());
        quota(&st, "acme", json!(10)).await.unwrap();
        assert_eq!(warnings(&st, now).iter().map(|w| w.line).collect::<Vec<_>>(), vec![90], "the line reached this month stays reached");

        // Next month starts from nothing, and warns again once it gets there.
        let next = month_start(now) + chrono::Duration::days(40);
        assert!(warnings(&st, next).is_empty());
        again(&st);
        follow(&st, next).await;
        let w = warnings(&st, next);
        assert_eq!(w.iter().map(|w| (w.month.clone(), w.line, w.minutes)).collect::<Vec<_>>(), vec![(month_of(next), 90, 9)]);
        assert_eq!(crossed(&st), vec![format!("{}:70", month_of(next)), format!("{}:90", month_of(next))], "last month's lines are forgotten");

        // No quota, no warnings.
        quota(&st, "acme", Value::Null).await.unwrap();
        assert!(warnings(&st, next).is_empty());
        assert_eq!(months(&st, next).iter().find(|u| u.owner == "acme").map(|u| (u.minutes, u.quota)), Some((9, None)));
    }

    #[tokio::test]
    async fn with_admin_org_an_orgs_month_is_what_github_bills_it() {
        let st = crate::tasks::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "big", Some("https://github.com/bigco/big"));
        repo_on(&fake, "bigco/big", true, "Organization", &[(1, &[60])]);
        let usage = "/organizations/bigco/settings/billing/usage";
        fake.route("GET", usage, 200, json!({ "usageItems": [
            { "product": "actions", "unitType": "Minutes", "quantity": 1200.5 },
            { "product": "Actions", "unitType": "minutes", "quantity": 300 },
            { "product": "actions", "unitType": "GigabyteHours", "quantity": 99 },
            { "product": "packages", "unitType": "Minutes", "quantity": 50 },
        ] }));
        let now = Utc::now();
        follow(&st, now).await;
        let o = owner(&owners_now(&st).await, "bigco").clone();
        assert_eq!((&o["minutes"], &o["source"]), (&json!(1501), &json!("billing")));
        assert_eq!(o["repos"][0]["minutes"], json!(1), "its repos are still listed");
        assert!(fake.seen(usage)[0].path.ends_with(&format!("?year={}&month={}", now.year(), now.month())));

        // An org not on the newer billing: the older endpoint.
        fake.route("GET", usage, 404, json!({ "message": "Not Found" }));
        fake.route("GET", "/orgs/bigco/settings/billing/actions", 200, json!({ "total_minutes_used": 2400, "included_minutes": 3000 }));
        again(&st);
        follow(&st, now).await;
        assert_eq!(owner(&owners_now(&st).await, "bigco")["minutes"], json!(2400));

        // A server error keeps what was read; a token without `admin:org` falls back to the repos.
        fake.route("GET", "/orgs/bigco/settings/billing/actions", 502, json!({ "message": "Bad Gateway" }));
        again(&st);
        follow(&st, now).await;
        assert_eq!(owner(&owners_now(&st).await, "bigco")["minutes"], json!(2400));
        fake.route("GET", "/orgs/bigco/settings/billing/actions", 403, json!({ "message": "Must have admin rights to Repository." }));
        again(&st);
        follow(&st, now).await;
        let o = owner(&owners_now(&st).await, "bigco").clone();
        assert_eq!((&o["minutes"], &o["source"]), (&json!(1), &json!("repos")));

        // The billed minutes count against the quota too.
        fake.route("GET", usage, 200, json!({ "usageItems": [{ "product": "actions", "unitType": "Minutes", "quantity": 2800 }] }));
        again(&st);
        follow(&st, now).await;
        quota(&st, "bigco", json!(3000)).await.unwrap();
        let items = quota_items(&st);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].2, "2800 of 3000 minutes this month, past 90% of the quota.");
    }

    #[tokio::test]
    async fn a_projects_page_shows_its_repos_minutes_and_its_owners_quota() {
        let st = crate::tasks::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "shop", Some("https://github.com/acme2/shop"));
        project(&st, "docs", Some("https://github.com/acme2/docs"));
        project(&st, "local", None);
        repo_on(&fake, "acme2/shop", true, "User", &[(1, &[600])]);
        repo_on(&fake, "acme2/docs", true, "User", &[(2, &[300])]);
        let read = |name: &'static str| project_get(&st, name);
        let v = read("shop").await.unwrap();
        assert_eq!((&v["repo"], &v["read"], &v["minutes"], &v["owner"]["quota"]), (&json!("acme2/shop"), &json!(false), &json!(0), &Value::Null), "nothing read yet");

        follow(&st, Utc::now()).await;
        quota(&st, "acme2", json!(60)).await.unwrap();
        let v = read("shop").await.unwrap();
        assert_eq!((&v["read"], &v["private"], &v["minutes"]), (&json!(true), &json!(true), &json!(10)));
        assert_eq!((&v["owner"]["owner"], &v["owner"]["minutes"], &v["owner"]["quota"]), (&json!("acme2"), &json!(15), &json!(60)));

        let v = read("local").await.unwrap();
        assert_eq!((&v["repo"], &v["why"]), (&Value::Null, &json!("The project local has no `origin` remote.")));
        assert_eq!(read("nowhere").await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_failed_read_keeps_the_month_and_a_repos_facts_are_read_again_after_six_hours() {
        let st = crate::tasks::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        project(&st, "app", Some("https://github.com/facts/app"));
        repo_on(&fake, "facts/app", true, "User", &[(1, &[60])]);
        let at = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let minutes = |st: &AppState, now| months(st, now).iter().find(|u| u.owner == "facts").map(|u| u.minutes);
        let listed = |fake: &crate::github::tests::Fake| fake.seen("/repos/facts/app/actions/runs?").last().unwrap().path.clone();

        // The 1st of the month lists from the 1st; the next read, from two days before the last one.
        let first = at("2026-11-01T08:00:00Z");
        follow(&st, first).await;
        assert!(listed(&fake).contains("created=%3E%3D2026-11-01&"), "{}", listed(&fake));
        assert_eq!(minutes(&st, first), Some(1));
        let later = at("2026-11-20T08:00:00Z");
        again(&st);
        follow(&st, later).await;
        again(&st);
        follow(&st, at("2026-11-21T09:00:00Z")).await;
        assert!(listed(&fake).contains("created=%3E%3D2026-11-18&"), "{}", listed(&fake));

        // Its facts are kept six hours.
        let facts = |fake: &crate::github::tests::Fake| fake.seen("/repos/facts/app").iter().filter(|s| s.path == "/repos/facts/app").count();
        assert_eq!(facts(&fake), 3, "read on the 1st, the 20th and the 21st");
        again(&st);
        follow(&st, at("2026-11-21T13:00:00Z")).await;
        assert_eq!(facts(&fake), 3, "within six hours of the 21st 09:00 read");

        // It went public: its runs are no longer read and it costs nothing.
        repo_on(&fake, "facts/app", false, "User", &[(1, &[60]), (2, &[600])]);
        let reads = fake.seen("/repos/facts/app/actions").len();
        again(&st);
        let now = at("2026-11-21T16:00:00Z");
        follow(&st, now).await;
        assert_eq!(fake.seen("/repos/facts/app/actions").len(), reads);
        assert_eq!(minutes(&st, now), Some(0));

        // A GitHub error keeps what the month had.
        repo_on(&fake, "facts/app", true, "User", &[(1, &[60]), (2, &[600])]);
        again(&st);
        let now = at("2026-11-22T16:00:00Z");
        follow(&st, now).await;
        assert_eq!(minutes(&st, now), Some(11));
        fake.route("GET", "/repos/facts/app/actions/runs", 200, json!({ "total_count": 3, "workflow_runs": [run(1), run(2), run(3)] }));
        fake.route("GET", "/repos/facts/app/actions/runs/3/jobs", 500, json!({ "message": "Server Error" }));
        again(&st);
        follow(&st, at("2026-11-23T16:00:00Z")).await;
        assert_eq!(minutes(&st, now), Some(11));
        assert_eq!(repo_month(&st, "facts/app", "2026-11").unwrap().read_at, "2026-11-22T16:00:00+00:00", "a failed read isn't a read");
    }

    async fn project_get(st: &AppState, name: &str) -> Result<Value, (axum::http::StatusCode, String)> {
        super::project(State(st.clone()), Query(ProjectQuery { project: name.into() })).await.map(|j| j.0).map_err(|e| (e.0, e.1))
    }

    #[test]
    fn a_branch_name_is_encoded_for_the_query() {
        assert_eq!(encode("spec/454-ci-seguro"), "spec/454-ci-seguro");
        assert_eq!(encode("a+b&c#d"), "a%2Bb%26c%23d");
    }
}
