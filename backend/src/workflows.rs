//! The workflow check: on a private repo, where GitHub bills every Actions minute, Loft reads `.github/workflows`
//! when a spec is published and when its Delivery starts, and warns (it never blocks) about three ways the
//! workflows spend more than they need on a draft spec PR: `push` and `pull_request` both running for the spec
//! branch, no `concurrency` with `cancel-in-progress`, and heavy jobs running on the draft. With the warnings goes an
//! estimate of the minutes each ticket will spend. A public repo, or workflows with none of the three, get nothing.

use crate::db::Delivery;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Pushes to the spec branch per ticket, as measured on Deliveries (spec #453).
pub const PUSHES_PER_TICKET: f64 = 1.8;

/// A job taking this many minutes a run or more is heavy.
const HEAVY_MINUTES: u32 = 5;

/// A job's minutes per run when the repo has no run of it to measure: a guess, per matrix combination.
const GUESS_MINUTES: u32 = 3;

/// Workflow files read at most.
const MAX_FILES: usize = 20;

/// Recent completed runs of each workflow measured.
const RUNS_PER_WORKFLOW: usize = 3;

/// What the check found, kept on the Delivery while there is something to warn about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub warnings: Vec<Warning>,
    /// The Actions minutes each ticket will spend, rounded up: every workflow that runs on a push to the spec
    /// branch, as many times as it runs for one, times [`PUSHES_PER_TICKET`].
    pub minutes_per_ticket: u32,
    /// The minutes come from the repo's recent runs; false when none of its jobs had one, so they're a guess.
    pub measured: bool,
    /// The estimate's line, as the cards show it.
    pub text: String,
    pub checked_at: String,
}

/// One workflow's problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    /// "push_and_pull_request", "no_cancel_in_progress" or "heavy_in_draft".
    pub kind: String,
    /// Its file: `.github/workflows/ci.yml`.
    pub workflow: String,
    /// What's wrong and how to fix it, as the cards show it.
    pub text: String,
}

/// Checks the workflows of `repo`'s Delivery of #spec again and keeps what it found on the Delivery: nothing when
/// there's nothing to warn about. When GitHub can't be read the Delivery keeps what it had. What it has after.
pub async fn record(state: &AppState, repo: &str, spec: i64) -> Option<Check> {
    let d = state.db.get_delivery(repo, spec).ok().flatten()?;
    if d.local || d.branch.is_empty() {
        return None;
    }
    let found = match check(state, &d).await {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!("delivery {repo}#{spec}: couldn't check the workflows: {e}");
            return d.workflows;
        }
    };
    if d.workflows != found {
        let keep = found.clone();
        crate::delivery::update(state, repo, spec, |d| d.workflows = keep);
    }
    found
}

/// The reply's lines about `c`, for the agent that published the spec.
pub fn reply_lines(c: &Check) -> String {
    let list: String = c.warnings.iter().map(|w| format!("\n- {}", w.text)).collect();
    format!("\nThe repo is private and its workflows spend more Actions minutes than they need to on the draft spec PR. Loft warns on the end-of-planning card and the Delivery card, and changes nothing:{list}\n{}", c.text)
}

/// The check of `d`'s repo, on its branch: None when the repo is public, has no workflows or nothing to warn about.
async fn check(state: &AppState, d: &Delivery) -> anyhow::Result<Option<Check>> {
    let repo = &d.repo;
    let meta = crate::github::get(state, &format!("/repos/{repo}")).await?;
    if meta["private"] != true {
        return Ok(None);
    }
    let base = crate::delivery::pr_base(state, d).or_else(|| meta["default_branch"].as_str().map(String::from)).unwrap_or_else(|| "main".into());
    let at = format!("ref={}", encode(&d.branch));
    let listed = match crate::github::get(state, &format!("/repos/{repo}/contents/.github/workflows?{at}")).await {
        Ok(v) => v,
        Err(e) if crate::github::is_gone(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut files = vec![];
    for f in listed.as_array().into_iter().flatten().filter(|f| f["type"] == "file").take(MAX_FILES) {
        let Some(path) = f["path"].as_str().filter(|p| p.ends_with(".yml") || p.ends_with(".yaml")) else { continue };
        let v = crate::github::get(state, &format!("/repos/{repo}/contents/{}?{at}", encode(path))).await?;
        let Some(text) = decode(&v) else { continue };
        if let Some(doc) = yaml::parse(&text) {
            files.push((path.to_string(), doc));
        }
    }
    let flows: Vec<Flow> = files.iter().filter_map(|(path, doc)| Flow::of(path, doc, &d.branch, &base)).collect();
    if flows.is_empty() {
        return Ok(None);
    }
    let measured = history(state, repo).await;
    Ok(judge(&flows, &measured))
}

/// The text of a file GitHub's contents API gave.
fn decode(v: &Value) -> Option<String> {
    use base64::Engine;
    let raw: String = v["content"].as_str()?.chars().filter(|c| !c.is_whitespace()).collect();
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(raw).ok()?).ok()
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// What each workflow's recent completed runs took: by its file, one list per run of (job name, minutes).
type Measured = HashMap<String, Vec<Vec<(String, u32)>>>;

/// The repo's recent completed runs, [`RUNS_PER_WORKFLOW`] of each workflow. Empty when GitHub can't say: the
/// estimate is then a guess.
async fn history(state: &AppState, repo: &str) -> Measured {
    let mut out: Measured = HashMap::new();
    let Ok(v) = crate::github::get(state, &format!("/repos/{repo}/actions/runs?status=completed&per_page=50")).await else { return out };
    for r in v["workflow_runs"].as_array().into_iter().flatten() {
        let (Some(path), Some(id)) = (r["path"].as_str(), r["id"].as_i64()) else { continue };
        // A run's path can carry `@ref`.
        let path = path.split('@').next().unwrap_or(path).to_string();
        if out.get(&path).is_some_and(|runs| runs.len() >= RUNS_PER_WORKFLOW) {
            continue;
        }
        let Ok(jobs) = crate::github::get(state, &format!("/repos/{repo}/actions/runs/{id}/jobs?filter=latest&per_page=100")).await else { continue };
        let jobs = jobs["jobs"].as_array().into_iter().flatten().map(|j| (j["name"].as_str().unwrap_or("").to_string(), crate::actions::job_minutes(j))).collect();
        out.entry(path).or_default().push(jobs);
    }
    out
}

/// A workflow that runs on pushes to the spec branch, as far as the check needs it.
struct Flow {
    path: String,
    /// It runs on a push to the spec branch, and on the spec PR's `synchronize`.
    push: bool,
    pull_request: bool,
    /// It has `concurrency` with `cancel-in-progress`, for the whole workflow or every job.
    cancels: bool,
    jobs: Vec<Job>,
}

struct Job {
    /// Its `name`, else its id: how GitHub names its runs' jobs.
    label: String,
    /// It doesn't run on a draft PR: its `if` or one it `needs` tests `draft`.
    skips_draft: bool,
    /// Its runner is macOS or Windows.
    big_runner: Option<&'static str>,
    /// Matrix combinations: a run has that many of it.
    matrix: u32,
}

impl Flow {
    /// The workflow at `path`, when it runs on a push to `branch`, whose PR goes to `base`.
    fn of(path: &str, doc: &Value, branch: &str, base: &str) -> Option<Flow> {
        let on = events(&doc["on"]);
        let push = on.get("push").is_some_and(|f| runs_for(f, branch, true));
        let pull_request = on.get("pull_request").is_some_and(|f| syncs(f) && runs_for(f, base, false));
        if !push && !pull_request {
            return None;
        }
        let ids: Vec<(&String, &Value)> = doc["jobs"].as_object().map(|m| m.iter().collect()).unwrap_or_default();
        // A job needing one that skips drafts skips them too.
        let mut skips: Vec<bool> = ids.iter().map(|(_, j)| j["if"].as_str().is_some_and(|c| c.to_lowercase().contains("draft"))).collect();
        loop {
            let mut more = false;
            for (i, (_, j)) in ids.iter().enumerate() {
                if !skips[i] && strings(&j["needs"]).iter().any(|n| ids.iter().position(|(id, _)| id.as_str() == n.as_str()).is_some_and(|k| skips[k])) {
                    (skips[i], more) = (true, true);
                }
            }
            if !more {
                break;
            }
        }
        let jobs: Vec<Job> = ids
            .iter()
            .zip(skips)
            .map(|((id, j), skips_draft)| {
                let runner = runs_on(&j["runs-on"]);
                let big_runner = if runner.contains("macos") { Some("macOS") } else if runner.contains("windows") { Some("Windows") } else { None };
                Job { label: j["name"].as_str().filter(|n| !n.contains("${{")).map_or_else(|| id.to_string(), String::from), skips_draft, big_runner, matrix: matrix_size(&j["strategy"]["matrix"]) }
            })
            .collect();
        let job_cancels = !ids.is_empty() && ids.iter().all(|(_, j)| cancels(&j["concurrency"]));
        Some(Flow { path: path.into(), push, pull_request, cancels: cancels(&doc["concurrency"]) || job_cancels, jobs })
    }

    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// The minutes of a run of `job` in workflow `path`: the average of the recent runs that had it, else a guess.
fn job_minutes(measured: &Measured, path: &str, job: &Job) -> (u32, bool) {
    let mine = |name: &str| name == job.label || name.starts_with(&format!("{} (", job.label)) || name.starts_with(&format!("{} / ", job.label));
    let runs: Vec<u32> = measured.get(path).into_iter().flatten().filter(|run| run.iter().any(|(n, _)| mine(n))).map(|run| run.iter().filter(|(n, _)| mine(n)).map(|(_, m)| m).sum()).collect();
    if runs.is_empty() {
        (GUESS_MINUTES * job.matrix, false)
    } else {
        (runs.iter().sum::<u32>().div_ceil(runs.len() as u32), true)
    }
}

/// The warnings about `flows` and the estimate, or None when nothing is wrong.
fn judge(flows: &[Flow], measured: &Measured) -> Option<Check> {
    let mut warnings = vec![];
    let (mut per_push, mut any_measured) = (0u32, false);
    for f in flows {
        let name = f.name();
        if f.push && f.pull_request {
            warnings.push(Warning {
                kind: "push_and_pull_request".into(),
                workflow: f.path.clone(),
                text: format!("`{name}` runs twice on every push to the spec branch, once for `push` and once for `pull_request`. Limit `push` to the base branch, like `push: {{ branches: [main] }}`."),
            });
        }
        if !f.cancels {
            warnings.push(Warning {
                kind: "no_cancel_in_progress".into(),
                workflow: f.path.clone(),
                text: format!("`{name}` has no `concurrency` with `cancel-in-progress`, so the run of an older push keeps going after a newer push. Add `concurrency: {{ group: \"${{{{ github.workflow }}}}-${{{{ github.ref }}}}\", cancel-in-progress: true }}`."),
            });
        }
        let mut heavy = vec![];
        let mut run = 0;
        for j in &f.jobs {
            let (m, from_runs) = job_minutes(measured, &f.path, j);
            any_measured |= from_runs;
            if j.skips_draft {
                continue;
            }
            run += m;
            match j.big_runner {
                Some(os) => heavy.push(format!("`{}` ({os})", j.label)),
                None if m >= HEAVY_MINUTES => heavy.push(format!("`{}` (about {m} min)", j.label)),
                None => {}
            }
        }
        per_push += run * (u32::from(f.push) + u32::from(f.pull_request));
        if !heavy.is_empty() {
            warnings.push(Warning {
                kind: "heavy_in_draft".into(),
                workflow: f.path.clone(),
                text: format!("`{name}` runs heavy jobs on the draft spec PR, where every ticket pushes: {}. Skip them in drafts with `if: github.event.pull_request.draft == false`.", heavy.join(", ")),
            });
        }
    }
    if warnings.is_empty() {
        return None;
    }
    let minutes_per_ticket = (f64::from(per_push) * PUSHES_PER_TICKET).ceil() as u32;
    let how = if any_measured { "measured on the repo's recent runs" } else { "a guess: its workflows have no recent runs to measure" };
    Some(Check {
        warnings,
        minutes_per_ticket,
        measured: any_measured,
        text: format!("Estimate: about {minutes_per_ticket} min of GitHub Actions per ticket ({PUSHES_PER_TICKET} pushes to the spec branch per ticket, {how})."),
        checked_at: crate::db::now(),
    })
}

/// A workflow's `on`, as each event with its filters (null when it has none).
fn events(on: &Value) -> serde_json::Map<String, Value> {
    match on {
        Value::String(e) => [(e.clone(), Value::Null)].into_iter().collect(),
        Value::Array(list) => list.iter().filter_map(|e| Some((e.as_str()?.to_string(), Value::Null))).collect(),
        Value::Object(m) => m.clone(),
        _ => Default::default(),
    }
}

/// A string or a list of them, as a list.
fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(l) => l.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

/// Whether an event with these filters runs for `branch`: the pushed branch for `push`, the PR's base for
/// `pull_request`. A push filtered only by tags runs for no branch.
fn runs_for(filters: &Value, branch: &str, push: bool) -> bool {
    if filters.get("branches").is_some() {
        // In order: a later `!pattern` takes back what an earlier pattern matched.
        return strings(&filters["branches"]).iter().fold(false, |hit, p| match p.strip_prefix('!') {
            Some(not) => hit && !glob(not.as_bytes(), branch.as_bytes()),
            None => hit || glob(p.as_bytes(), branch.as_bytes()),
        });
    }
    if filters.get("branches-ignore").is_some() {
        return !strings(&filters["branches-ignore"]).iter().any(|p| glob(p.as_bytes(), branch.as_bytes()));
    }
    !(push && (filters.get("tags").is_some() || filters.get("tags-ignore").is_some()))
}

/// A `pull_request` with these filters runs again when the PR's branch gets a push (`synchronize`, a default type).
fn syncs(filters: &Value) -> bool {
    filters.get("types").is_none_or(|t| strings(t).iter().any(|t| t == "synchronize"))
}

/// GitHub's branch filter patterns: `*` matches anything but `/`, `**` anything, `?` one character.
fn glob(p: &[u8], s: &[u8]) -> bool {
    match p {
        [] => s.is_empty(),
        [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| glob(rest, &s[i..])),
        [b'*', rest @ ..] => (0..=s.len()).take_while(|&i| i == 0 || s[i - 1] != b'/').any(|i| glob(rest, &s[i..])),
        [b'?', rest @ ..] => !s.is_empty() && glob(rest, &s[1..]),
        [c, rest @ ..] => s.first() == Some(c) && glob(rest, &s[1..]),
    }
}

/// `concurrency` that cancels the run in progress: `cancel-in-progress` set to anything but false (an
/// expression counts). A bare group string only queues.
fn cancels(v: &Value) -> bool {
    v.get("cancel-in-progress").is_some_and(|c| !matches!(c, Value::Null | Value::Bool(false)))
}

/// A job's `runs-on`, lowercased, its labels joined.
fn runs_on(v: &Value) -> String {
    match v {
        Value::Object(m) => m.values().map(runs_on).collect::<Vec<_>>().join(" "),
        _ => strings(v).join(" ").to_lowercase(),
    }
}

/// How many jobs a `strategy.matrix` makes: the product of its lists, less `exclude`, plus `include` when it has
/// no lists. An expression counts as one.
fn matrix_size(m: &Value) -> u32 {
    let Some(m) = m.as_object() else { return 1 };
    let count = |k: &str| m.get(k).and_then(Value::as_array).map_or(0, |a| a.len() as u32);
    let lists: Vec<u32> = m.iter().filter(|(k, _)| *k != "include" && *k != "exclude").map(|(_, v)| v.as_array().map_or(1, |a| a.len() as u32)).collect();
    let n = if lists.is_empty() { count("include") } else { lists.iter().product::<u32>().saturating_sub(count("exclude")) };
    n.max(1)
}

/// The YAML of workflow files, as JSON: block maps and lists, flow `[...]` and `{...}`, quoted and plain scalars,
/// block scalars (`|`, `>`) and comments. Anchors, tags and multi-line plain scalars aren't read.
pub(crate) mod yaml {
    use serde_json::{Map, Value};

    struct Line {
        indent: usize,
        /// Without its indent, comment or trailing spaces: empty for a blank or comment line.
        text: String,
    }

    pub fn parse(src: &str) -> Option<Value> {
        let lines: Vec<Line> = src
            .lines()
            .filter(|l| !matches!(l.trim_end(), "---" | "..."))
            .map(|l| {
                let body = l.trim_start_matches(' ');
                Line { indent: l.len() - body.len(), text: uncomment(body).trim_end().to_string() }
            })
            .collect();
        let mut p = Parser { lines, at: 0 };
        let v = p.node(0);
        v.is_object().then_some(v)
    }

    /// `line` without its comment: a `#` at its start or after a space, outside quotes.
    fn uncomment(line: &str) -> &str {
        let mut quote: Option<char> = None;
        let mut prev = ' ';
        for (i, c) in line.char_indices() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                // A quote opens a scalar only where one can start.
                None if (c == '\'' || c == '"') && " [{,:-".contains(prev) => quote = Some(c),
                None if c == '#' && prev.is_whitespace() => return &line[..i],
                None => {}
            }
            prev = c;
        }
        line
    }

    fn is_item(text: &str) -> bool {
        text == "-" || text.starts_with("- ")
    }

    /// A `key: value` line's key and value (empty when the value is on the lines below).
    fn split_key(text: &str) -> Option<(String, String)> {
        if text.starts_with(['[', '{']) {
            return None;
        }
        let (key, rest) = if let Some(q) = text.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let end = text[1..].find(q)? + 1;
            (text[1..end].to_string(), text[end + 1..].trim_start())
        } else {
            let at = text.char_indices().find(|&(i, c)| c == ':' && text[i + 1..].chars().next().is_none_or(char::is_whitespace))?.0;
            (text[..at].trim_end().to_string(), &text[at..])
        };
        let value = rest.strip_prefix(':')?;
        (value.is_empty() || value.starts_with(char::is_whitespace)).then(|| (key, value.trim().to_string()))
    }

    struct Parser {
        lines: Vec<Line>,
        at: usize,
    }

    impl Parser {
        /// The next line with something on it.
        fn next(&mut self) -> Option<&Line> {
            while self.lines.get(self.at).is_some_and(|l| l.text.is_empty()) {
                self.at += 1;
            }
            self.lines.get(self.at)
        }

        /// The node on the next lines, if they're indented at least `min`.
        fn node(&mut self, min: usize) -> Value {
            match self.next() {
                Some(l) if l.indent >= min => {
                    let indent = l.indent;
                    if is_item(&l.text) { self.list(indent) } else { self.map(indent) }
                }
                _ => Value::Null,
            }
        }

        fn list(&mut self, indent: usize) -> Value {
            let mut out = vec![];
            while let Some(l) = self.next().filter(|l| l.indent == indent && is_item(&l.text)) {
                let rest = l.text[1..].trim_start().to_string();
                if rest.is_empty() {
                    self.at += 1;
                    out.push(self.node(indent + 1));
                } else if is_item(&rest) || split_key(&rest).is_some() {
                    // The item's node starts on this line: read it again at its own column.
                    let column = indent + l.text.len() - rest.len();
                    self.lines[self.at] = Line { indent: column, text: rest };
                    out.push(self.node(column));
                } else {
                    self.at += 1;
                    out.push(self.value(rest, indent));
                }
            }
            Value::Array(out)
        }

        fn map(&mut self, indent: usize) -> Value {
            let mut out = Map::new();
            while let Some(l) = self.next().filter(|l| l.indent == indent && !is_item(&l.text)) {
                let Some((key, value)) = split_key(&l.text) else {
                    // Not a key (a plain scalar on several lines): skipped.
                    self.at += 1;
                    continue;
                };
                self.at += 1;
                let v = if value.is_empty() {
                    match self.next() {
                        Some(n) if n.indent > indent => self.node(indent + 1),
                        // A list may sit at its key's own indent.
                        Some(n) if n.indent == indent && is_item(&n.text) => self.list(indent),
                        _ => Value::Null,
                    }
                } else {
                    self.value(value, indent)
                };
                out.insert(key, v);
            }
            Value::Object(out)
        }

        /// A value written on the line of its key or `-` (indented `indent`): a block scalar, a flow collection,
        /// maybe over several lines, or a scalar.
        fn value(&mut self, v: String, indent: usize) -> Value {
            if v.starts_with(['|', '>']) {
                let mut text = vec![];
                while let Some(l) = self.lines.get(self.at).filter(|l| l.text.is_empty() || l.indent > indent) {
                    text.push(l.text.clone());
                    self.at += 1;
                }
                return Value::String(text.join("\n").trim().to_string());
            }
            if v.starts_with(['[', '{']) {
                let mut s = v;
                while !closed(&s) && self.at < self.lines.len() {
                    s.push(' ');
                    s.push_str(&self.lines[self.at].text);
                    self.at += 1;
                }
                let chars: Vec<char> = s.chars().collect();
                return flow(&chars, &mut 0);
            }
            scalar(&v)
        }
    }

    /// Every bracket of a flow collection is closed.
    fn closed(s: &str) -> bool {
        let (mut depth, mut quote) = (0i32, None);
        for c in s.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '[' | '{') => depth += 1,
                (None, ']' | '}') => depth -= 1,
                _ => {}
            }
        }
        depth <= 0
    }

    fn flow(cs: &[char], i: &mut usize) -> Value {
        let skip = |i: &mut usize| {
            while cs.get(*i).is_some_and(|c| c.is_whitespace()) {
                *i += 1;
            }
        };
        skip(i);
        match cs.get(*i) {
            Some('[') => {
                *i += 1;
                let mut out = vec![];
                loop {
                    skip(i);
                    match cs.get(*i) {
                        None => break,
                        Some(']') => {
                            *i += 1;
                            break;
                        }
                        Some(',') => *i += 1,
                        _ => out.push(flow(cs, i)),
                    }
                }
                Value::Array(out)
            }
            Some('{') => {
                *i += 1;
                let mut out = Map::new();
                loop {
                    skip(i);
                    match cs.get(*i) {
                        None => break,
                        Some('}') => {
                            *i += 1;
                            break;
                        }
                        Some(',') => *i += 1,
                        _ => {
                            let key = match flow_scalar(cs, i, true) {
                                Value::String(s) => s,
                                v => v.to_string(),
                            };
                            skip(i);
                            let v = if cs.get(*i) == Some(&':') {
                                *i += 1;
                                flow(cs, i)
                            } else {
                                Value::Null
                            };
                            out.insert(key, v);
                        }
                    }
                }
                Value::Object(out)
            }
            _ => flow_scalar(cs, i, false),
        }
    }

    /// A scalar in a flow collection: up to its `,`, `]` or `}`, or for a key its `:`.
    fn flow_scalar(cs: &[char], i: &mut usize, key: bool) -> Value {
        let start = *i;
        if let Some(&q) = cs.get(*i).filter(|c| **c == '"' || **c == '\'') {
            *i += 1;
            while cs.get(*i).is_some_and(|c| *c != q) {
                *i += 1;
            }
            *i = (*i + 1).min(cs.len());
            return scalar(&cs[start..*i].iter().collect::<String>());
        }
        while let Some(&c) = cs.get(*i) {
            let ends_key = key && c == ':' && cs.get(*i + 1).is_none_or(|n| n.is_whitespace() || ",}]".contains(*n));
            if ",]}".contains(c) || ends_key {
                break;
            }
            *i += 1;
        }
        scalar(&cs[start..*i].iter().collect::<String>())
    }

    fn scalar(s: &str) -> Value {
        let s = s.trim();
        if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
            return Value::String(s[1..s.len() - 1].replace("''", "'"));
        }
        if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            return Value::String(s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\"));
        }
        match s {
            "" | "~" | "null" | "Null" | "NULL" => Value::Null,
            "true" | "True" | "TRUE" => Value::Bool(true),
            "false" | "False" | "FALSE" => Value::Bool(false),
            _ => s.parse::<i64>().map_or_else(|_| Value::String(s.into()), Value::from),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_yaml_of_a_workflow() {
        let doc = yaml::parse(
            "# CI\n\
             name: CI\n\
             on:\n  push:\n    branches: [main, 'release/**']\n  pull_request:\n    types: [opened, synchronize]\n\
             concurrency:\n  group: ${{ github.workflow }}-${{ github.ref }}  # per ref\n  cancel-in-progress: true\n\
             jobs:\n  test:\n    runs-on: ${{ matrix.os }}\n    if: github.event.pull_request.draft == false\n    strategy:\n      matrix:\n        os: [ubuntu-latest, macos-latest]\n        node:\n          - 18\n          - 20\n    steps:\n      - uses: actions/checkout@v4\n      - name: Test\n        run: |\n          npm ci # deps\n\n          npm test\n      - run: echo \"a: b\"\n  lint: { runs-on: ubuntu-latest, needs: [test] }\n",
        )
        .unwrap();
        assert_eq!(doc["on"]["push"]["branches"], json!(["main", "release/**"]));
        assert_eq!(doc["on"]["pull_request"]["types"], json!(["opened", "synchronize"]));
        assert_eq!(doc["concurrency"], json!({ "group": "${{ github.workflow }}-${{ github.ref }}", "cancel-in-progress": true }));
        let test = &doc["jobs"]["test"];
        assert_eq!(test["strategy"]["matrix"], json!({ "os": ["ubuntu-latest", "macos-latest"], "node": [18, 20] }));
        assert_eq!(test["steps"][0], json!({ "uses": "actions/checkout@v4" }));
        assert_eq!(test["steps"][1], json!({ "name": "Test", "run": "npm ci\n\nnpm test" }));
        assert_eq!(test["steps"][2], json!({ "run": "echo \"a: b\"" }));
        assert_eq!(doc["jobs"]["lint"], json!({ "runs-on": "ubuntu-latest", "needs": ["test"] }));
        // `on` as a list, and a list at its key's indent.
        let doc = yaml::parse("on: [push, pull_request]\njobs:\n  a:\n    steps:\n    - run: x\n    - run: y\n").unwrap();
        assert_eq!(doc["on"], json!(["push", "pull_request"]));
        assert_eq!(doc["jobs"]["a"]["steps"], json!([{ "run": "x" }, { "run": "y" }]));
        assert_eq!(yaml::parse("on: push\n").unwrap()["on"], "push");
    }

    #[test]
    fn branch_filters_match_like_github() {
        let f = |v: Value, b: &str, push: bool| runs_for(&v, b, push);
        assert!(f(Value::Null, "spec/1-x", true));
        assert!(!f(json!({ "branches": ["main"] }), "spec/1-x", true));
        assert!(f(json!({ "branches": ["spec/*"] }), "spec/1-x", true));
        assert!(!f(json!({ "branches": ["*"] }), "spec/1-x", true), "`*` stops at `/`");
        assert!(f(json!({ "branches": ["**"] }), "spec/1-x", true));
        assert!(!f(json!({ "branches": ["**", "!spec/**"] }), "spec/1-x", true));
        assert!(!f(json!({ "branches-ignore": ["spec/**"] }), "spec/1-x", true));
        assert!(!f(json!({ "tags": ["v*"] }), "spec/1-x", true), "a push of tags only");
        assert!(f(json!({ "tags": ["v*"] }), "main", false));
        assert!(syncs(&Value::Null));
        assert!(!syncs(&json!({ "types": ["opened", "ready_for_review"] })));
    }

    fn flow(text: &str) -> Option<Flow> {
        Flow::of(".github/workflows/ci.yml", &yaml::parse(text).unwrap(), "spec/7-x", "main")
    }

    #[test]
    fn each_pattern_warns_and_a_clean_workflow_doesnt() {
        let kinds = |c: &Option<Check>| c.as_ref().map(|c| c.warnings.iter().map(|w| w.kind.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let none = Measured::new();
        // Both triggers, no concurrency, a macOS job on the draft.
        let bad = flow("on: [push, pull_request]\njobs:\n  ios:\n    runs-on: macos-latest\n").unwrap();
        let c = judge(&[bad], &none);
        assert_eq!(kinds(&c), ["push_and_pull_request", "no_cancel_in_progress", "heavy_in_draft"]);
        let c = c.unwrap();
        // One job guessed at 3 min, run twice a push, 1.8 pushes a ticket.
        assert_eq!((c.minutes_per_ticket, c.measured), (11, false));
        assert!(c.text.contains("about 11 min of GitHub Actions per ticket"), "{}", c.text);
        assert!(c.warnings[2].text.contains("`ios` (macOS)"), "{}", c.warnings[2].text);

        // Push only for main, cancelling, heavy job skipped in drafts: nothing to say.
        let clean = "on:\n  push:\n    branches: [main]\n  pull_request:\nconcurrency:\n  group: ci-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\njobs:\n  ios:\n    runs-on: macos-latest\n    if: ${{ !github.event.pull_request.draft }}\n  after:\n    runs-on: windows-latest\n    needs: ios\n  lint:\n    runs-on: ubuntu-latest\n";
        assert_eq!(judge(&[flow(clean).unwrap()], &none), None);
        // A workflow that never runs for the spec branch isn't looked at.
        assert!(flow("on:\n  push:\n    branches: [main]\njobs:\n  a:\n    runs-on: macos-latest\n").is_none());
        assert!(flow("on:\n  pull_request:\n    branches: [develop]\njobs:\n  a:\n    runs-on: macos-latest\n").is_none());
        // A group only queues; a long Linux job measured on recent runs is heavy.
        let queued = flow("on: pull_request\nconcurrency: ci\njobs:\n  test:\n    name: Test\n    runs-on: ubuntu-latest\n    strategy:\n      matrix:\n        n: [1, 2]\n").unwrap();
        let measured: Measured = [(".github/workflows/ci.yml".to_string(), vec![vec![("Test (1)".to_string(), 6), ("Test (2)".to_string(), 6)], vec![("Test (1)".to_string(), 4), ("Test (2)".to_string(), 5)]])].into_iter().collect();
        let c = judge(&[queued], &measured);
        assert_eq!(kinds(&c), ["no_cancel_in_progress", "heavy_in_draft"]);
        let c = c.unwrap();
        // (12 + 9) / 2 rounded up, once a push: 11 × 1.8.
        assert_eq!((c.minutes_per_ticket, c.measured), (20, true));
        assert!(c.warnings[1].text.contains("`Test` (about 11 min)"), "{}", c.warnings[1].text);
    }
}
