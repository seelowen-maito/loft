//! GitHub Actions minutes: how long the workflow runs of a branch took, each job rounded up to the minute as
//! GitHub bills it. A Delivery keeps its branch's runs, so its card and its done summary say what it spent.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

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
}

/// The total of `runs`.
pub fn total(runs: &BTreeMap<String, RunMinutes>) -> u32 {
    runs.values().map(|r| r.minutes).sum()
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
    let mut out = known.clone();
    let created = since.get(..10).map(|day| format!("&created=%3E%3D{day}")).unwrap_or_default();
    for page in 1..=MAX_PAGES {
        let v = crate::github::get(state, &format!("/repos/{repo}/actions/runs?branch={}{created}&per_page={PER_PAGE}&page={page}", encode(branch))).await?;
        let runs = v["workflow_runs"].as_array().cloned().unwrap_or_default();
        for r in &runs {
            let Some(id) = r["id"].as_i64() else { continue };
            let attempt = r["run_attempt"].as_u64().and_then(|a| u32::try_from(a).ok()).unwrap_or(1);
            if known.get(&id.to_string()).is_some_and(|k| k.done && k.attempt == attempt) {
                continue;
            }
            let minutes = run_minutes(state, repo, id).await?;
            out.insert(id.to_string(), RunMinutes { attempt, minutes, done: r["status"] == "completed" });
        }
        let listed = v["total_count"].as_u64().map_or(usize::MAX, |n| n as usize);
        if runs.len() < PER_PAGE || page * PER_PAGE >= listed {
            break;
        }
    }
    Ok(out)
}

/// The minutes of run `id`'s jobs, every attempt's.
async fn run_minutes(state: &AppState, repo: &str, id: i64) -> anyhow::Result<u32> {
    let mut minutes = 0;
    for page in 1..=MAX_PAGES {
        let v = crate::github::get(state, &format!("/repos/{repo}/actions/runs/{id}/jobs?filter=all&per_page={PER_PAGE}&page={page}")).await?;
        let jobs = v["jobs"].as_array().cloned().unwrap_or_default();
        minutes += jobs.iter().map(job_minutes).sum::<u32>();
        let listed = v["total_count"].as_u64().map_or(usize::MAX, |n| n as usize);
        if jobs.len() < PER_PAGE || page * PER_PAGE >= listed {
            break;
        }
    }
    Ok(minutes)
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

    #[test]
    fn a_branch_name_is_encoded_for_the_query() {
        assert_eq!(encode("spec/454-ci-seguro"), "spec/454-ci-seguro");
        assert_eq!(encode("a+b&c#d"), "a%2Bb%26c%23d");
    }
}
