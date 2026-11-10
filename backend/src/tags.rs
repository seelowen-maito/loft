//! `#` tags in a chat message: `#180` (a spec, issue or PR in the agent's project), `owner/repo#12` (in another
//! repo) or `#<id>` (a Loft-only issue). A known tag is stored on the message as a chip and described to the
//! agent in a "Tagged" block; an unknown one stays plain text.

use serde_json::{Value, json};

use crate::db::Agent;
use crate::state::AppState;

/// At most this many tags are resolved per message.
const MAX: usize = 8;

#[derive(Debug, PartialEq)]
enum Target {
    Number(Option<String>, i64),
    Loft(String),
}

/// The tags `text` carries, as typed, in order and without repeats. A tag starts a word.
fn parse(text: &str) -> Vec<(String, Target)> {
    let mut out: Vec<(String, Target)> = vec![];
    for (i, _) in text.match_indices('#') {
        let before = &text[..i];
        let slug_start = before.rfind(|c: char| !(c.is_ascii_alphanumeric() || "._/-".contains(c))).map_or(0, |p| p + before[p..].chars().next().map_or(1, char::len_utf8));
        let word_start = before[..slug_start].chars().last().is_none_or(|p| p.is_whitespace() || "([{\"'".contains(p));
        let slug = &before[slug_start..];
        let slug_ok = slug.is_empty() || slug.split_once('/').is_some_and(|(o, r)| !o.is_empty() && !r.is_empty() && !r.contains('/'));
        if !word_start || !slug_ok {
            continue;
        }
        let rest = &text[i + 1..];
        let word: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let after = rest[word.len()..].chars().next();
        let ended = after.is_none_or(|c| !(c.is_alphanumeric() || c == '/' || (c == '.' && rest[word.len() + 1..].starts_with(|n: char| n.is_alphanumeric()))));
        let slug = (!slug.is_empty()).then(|| slug.to_lowercase());
        let target = if word.chars().all(|c| c.is_ascii_digit()) && !word.is_empty() && word.len() < 10 {
            Target::Number(slug.clone(), word.parse().unwrap())
        } else if slug.is_none() && uuid::Uuid::parse_str(&word).is_ok() {
            Target::Loft(word.to_lowercase())
        } else {
            continue;
        };
        if !ended {
            continue;
        }
        let shown = format!("{}#{word}", &text[slug_start..i]);
        if !out.iter().any(|(s, _)| *s == shown) {
            out.push((shown, target));
        }
    }
    out
}

/// The known tags of `text`, as the message's `meta.tags`: `{ tag, kind (spec | issue | pr), title, state, url, task_id? }`.
/// A bare `#N` means the agent's project; other tags that GitHub or Loft doesn't have are left out.
pub fn resolve(state: &AppState, agent: &Agent, text: &str) -> Vec<Value> {
    let found = parse(text);
    if found.is_empty() {
        return vec![];
    }
    let tasks = state.db.list_tasks().unwrap_or_default();
    let prs = state.db.list_pull_requests(None).unwrap_or_default();
    let project = agent.project.trim();
    let mut out = vec![];
    for (tag, target) in found.into_iter().take(MAX) {
        let task = match &target {
            Target::Loft(id) => tasks.iter().find(|t| t.id == *id),
            Target::Number(slug, n) => tasks.iter().find(|t| {
                t.github.as_ref().is_some_and(|g| g.number == *n && match slug {
                    Some(s) => g.repo.to_lowercase() == *s,
                    None => !project.is_empty() && t.project == project,
                })
            }),
        };
        if let Some(t) = task {
            let spec = t.labels.iter().any(|l| l.eq_ignore_ascii_case("spec"));
            let state = t.github.as_ref().map_or(t.status.replace('_', " "), |g| g.state.clone());
            out.push(json!({ "tag": tag, "kind": if spec { "spec" } else { "issue" }, "title": t.title, "state": state, "url": t.github.as_ref().map(|g| g.url.clone()), "task_id": t.id }));
        } else if let Target::Number(slug, n) = &target {
            let pr = prs.iter().find(|p| p.number == *n && match slug {
                Some(s) => p.slug.to_lowercase() == *s,
                None => !project.is_empty() && p.slug.rsplit('/').next() == Some(project),
            });
            if let Some(p) = pr {
                let s = if p.state.is_empty() { "open" } else { p.state.as_str() };
                out.push(json!({ "tag": tag, "kind": "pr", "title": p.title, "state": s, "url": p.url }));
            }
        }
    }
    out
}

/// The "Tagged" block an agent's prompt gets for its message's `tags`; empty when there are none.
pub fn block(tags: &Value) -> String {
    let lines: Vec<String> = tags
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let s = |k: &str| t[k].as_str().unwrap_or("");
            let link = t["url"].as_str().map_or_else(|| format!("Loft issue {}", s("task_id")), String::from);
            format!("- {} {} “{}” ({}) {}", s("kind"), s("tag"), crate::tasks::inert(s("title")), crate::tasks::inert(s("state")), link)
        })
        .collect();
    if lines.is_empty() { String::new() } else { format!("[Tagged by the user:\n{}]", lines.join("\n")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(text: &str) -> Vec<String> {
        parse(text).into_iter().map(|(s, _)| s).collect()
    }

    #[test]
    fn finds_tags_that_start_a_word() {
        assert_eq!(nums("look at #180, octo/loft#12 and (#7)."), ["#180", "octo/loft#12", "#7"]);
        assert_eq!(nums("#5 #5"), ["#5"]);
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(nums(&format!("see #{id}")), [format!("#{id}")]);
        // Not tags: inside a word, a colour, a heading, a path, a longer name.
        assert!(nums("abc#12 #fff ## a/b/c#4 x#9 #12abc #12/x file.md#3 #3.5").is_empty(), "{:?}", nums("abc#12 #fff ## a/b/c#4 x#9 #12abc #12/x file.md#3 #3.5"));
    }

    #[tokio::test]
    async fn known_tags_reach_the_agents_prompt_and_unknown_ones_stay_text() {
        use crate::tasks::tests::{agent, busy, state};
        let st = state();
        let mut a = agent(&st, "Worker");
        a.project = "loft".into();
        st.db.save_agent(&a).unwrap();
        busy(&st, &a);
        let link = |repo: &str, n: i64| crate::db::GithubLink { repo: repo.into(), number: n, url: format!("https://github.com/{repo}/issues/{n}"), state: "open".into(), ..Default::default() };
        let (mut spec, _) = crate::tasks::create(&st, &json!({ "title": "Tag things", "labels": ["spec"], "project": "loft" }), "user").unwrap();
        spec.github = Some(link("octo/loft", 199));
        st.db.save_task(&spec).unwrap();
        let (mut other, _) = crate::tasks::create(&st, &json!({ "title": "Elsewhere [x]", "project": "api" }), "user").unwrap();
        other.github = Some(link("octo/api", 5));
        st.db.save_task(&other).unwrap();
        let (local, _) = crate::tasks::create(&st, &json!({ "title": "Only in Loft", "project": "loft" }), "user").unwrap();
        let pr = crate::db::PullRequest { id: "p1".into(), agent_id: a.id.clone(), slug: "octo/loft".into(), number: 200, url: "https://github.com/octo/loft/pull/200".into(), title: "Spec PR".into(), state: "open".into(), ..Default::default() };
        st.db.save_pull_request(&pr).unwrap();

        let text = format!("see #199, #200, octo/api#5, #{}, #5 and #999", local.id);
        let r = crate::api::send_message(
            axum::extract::State(st.clone()),
            axum::extract::Path(a.id.clone()),
            axum::Json(serde_json::from_value(json!({ "text": text })).unwrap()),
        )
        .await
        .ok()
        .unwrap()
        .0;
        let tags = r["message"]["meta"]["tags"].as_array().unwrap();
        let got: Vec<(&str, &str)> = tags.iter().map(|t| (t["tag"].as_str().unwrap(), t["kind"].as_str().unwrap())).collect();
        assert_eq!(got, [("#199", "spec"), ("#200", "pr"), ("octo/api#5", "issue"), (&*format!("#{}", local.id), "issue")], "#5 is another project's, #999 is unknown");
        let q = st.rt.lock().unwrap().queued.get(&a.id).map(|q| q.join("\n---\n")).unwrap_or_default();
        assert!(q.contains("[Tagged by the user:\n- spec #199 “Tag things” (open) https://github.com/octo/loft/issues/199\n- pr #200 “Spec PR” (open) https://github.com/octo/loft/pull/200\n- issue octo/api#5 “Elsewhere (x)” (open) https://github.com/octo/api/issues/5\n"), "{q}");
        assert!(q.contains(&format!("- issue #{} “Only in Loft”", local.id)) && !q.contains("#999 “"), "{q}");
    }
}
