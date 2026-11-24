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
        assert_eq!(out["1"], RunMinutes { attempt: 1, minutes: 105, done: true });
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

    #[test]
    fn a_branch_name_is_encoded_for_the_query() {
        assert_eq!(encode("spec/454-ci-seguro"), "spec/454-ci-seguro");
        assert_eq!(encode("a+b&c#d"), "a%2Bb%26c%23d");
    }
}
