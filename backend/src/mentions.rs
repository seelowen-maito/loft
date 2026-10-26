//! @-mentions: `@Web Dev …` in any chat also delivers that message to the named agent, and its
//! reply is copied back into the chat it was mentioned in, so a conversation can pull another agent in.
//!
//! A message that *starts* with mentions is meant for them, so the chat's own agent isn't woken for it;
//! a mention later in a message reaches both.

use serde_json::{Value, json};

use crate::agents;
use crate::db::Agent;
use crate::state::AppState;

/// Starts the prompt a mentioned agent gets; a run whose prompt holds it mirrors its reply back.
pub const TAG: &str = "[You were mentioned by the user in ";

/// The agents `text` mentions (other than `exclude`), in order, and whether the message opens with
/// mentions. Names match case-insensitively at an `@` that starts a word, longest name first, and must
/// end at a word boundary ("@Web Dev," but not "@Web Developer"). A name that goes on as a
/// file path ("@qa/report.md", "@QA.txt") isn't a mention; a sentence's closing period ("@QA.") is.
pub fn parse<'a>(text: &str, agents: &'a [Agent], exclude: &str) -> (Vec<&'a Agent>, bool) {
    let mut by_len: Vec<&Agent> = agents.iter().filter(|a| a.id != exclude && !a.name.trim().is_empty()).collect();
    by_len.sort_by_key(|a| std::cmp::Reverse(a.name.chars().count()));
    let lower = text.to_lowercase();
    let mut found: Vec<&Agent> = vec![];
    let mut leading = true;
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        let at_word_start = i == 0 || text[..i].chars().last().is_some_and(|p| p.is_whitespace() || "([{\"'".contains(p));
        if c == '@' && at_word_start {
            let rest = &lower[i + 1..];
            let hit = by_len.iter().find(|a| {
                let n = a.name.to_lowercase();
                let mut after = rest.get(n.len()..).unwrap_or("").chars();
                let path = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || "_-/.".contains(c));
                rest.starts_with(&n) && match after.next() {
                    None => true,
                    Some('/') => false,
                    Some('.') => !path(after.next()),
                    Some(e) => !e.is_alphanumeric() && e != '_' && e != '-',
                }
            });
            if let Some(a) = hit {
                if !found.iter().any(|f| f.id == a.id) {
                    found.push(a);
                }
                i += 1 + a.name.to_lowercase().len();
                continue;
            }
        }
        if !c.is_whitespace() && !",;:".contains(c) && found.is_empty() {
            leading = false;
        }
        i += c.len_utf8();
    }
    let leading = leading && !found.is_empty();
    (found, leading)
}

/// Delivers a user message from `from`'s chat to each mentioned agent, and remembers to copy each
/// one's reply back into `from`'s chat.
pub fn deliver(state: &AppState, from: &Agent, mentioned: &[&Agent], text: &str, message_id: Option<i64>) {
    for to in mentioned {
        agents::add_msg(state, &to.id, None, "user", "text", text, json!({
            "mentioned_in": { "id": from.id, "name": from.name },
            "mention_message_id": message_id,
        }));
        state.rt.lock().unwrap().mention_pending.entry(to.id.clone()).or_default().push(from.id.clone());
        let prompt = format!(
            "{TAG}{name}'s chat (agent id {id})]\n{text}\n\n[Your reply is shown in this chat and also in {name}'s chat. Answer the user; if {name} should act on it, say so or message it with `mcp__loft__message_agent`.]",
            name = from.name,
            id = from.id,
        );
        agents::wake(state, &to.id, prompt, "user");
    }
}

/// For a run starting with `prompt`: the chats to copy its reply into.
pub fn take_mirrors(state: &AppState, agent_id: &str, prompt: &str) -> Vec<String> {
    if !prompt.contains(TAG) {
        return vec![];
    }
    state.rt.lock().unwrap().mention_pending.remove(agent_id).unwrap_or_default()
}

/// Copies a mentioned agent's reply into the chats it was mentioned in.
pub fn mirror(state: &AppState, agent: &Agent, chats: &[String], reply: &str, failed: bool) {
    let text = match (reply.trim().is_empty(), failed) {
        (_, true) => format!("{} couldn't answer: its run failed.", agent.name),
        (true, false) => format!("{} finished without a reply.", agent.name),
        (false, false) => reply.trim().to_string(),
    };
    for chat in chats {
        agents::add_msg(state, chat, None, "assistant", "text", &text, json!({
            "from_agent": { "id": agent.id, "name": agent.name },
            "mention_reply": true,
        }));
    }
}

/// `meta.mentions` on the user's message: who it was delivered to.
pub fn meta(mentioned: &[&Agent]) -> Value {
    json!(mentioned.iter().map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::{agent, state};

    #[test]
    fn finds_mentions_by_name() {
        let st = state();
        let (orch, dev, qa) = (agent(&st, "Orchestrator"), agent(&st, "Web Dev"), agent(&st, "QA"));
        let _devx = agent(&st, "Web Developer");
        let all = st.db.list_agents().unwrap();
        let names = |text: &str| {
            let (m, lead) = parse(text, &all, &orch.id);
            (m.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), lead)
        };
        assert_eq!(names("@Web Dev can you check the backup numbers?"), (vec!["Web Dev".to_string()], true));
        assert_eq!(names("@web dev, @QA: both of you"), (vec!["Web Dev".into(), "QA".into()], true));
        assert_eq!(names("Plan this, and loop in @QA for tests"), (vec!["QA".into()], false));
        assert_eq!(names("@Web Developer please"), (vec!["Web Developer".into()], true), "longest name wins");
        assert_eq!(names("@Web Devs are busy").0, Vec::<String>::new(), "a mention ends at a word boundary");
        assert_eq!(names("mail me at sam@QA.com").0, Vec::<String>::new(), "only an @ that starts a word");
        assert_eq!(names("@Orchestrator hi").0, Vec::<String>::new(), "the chat's own agent isn't a mention");
        assert_eq!(names("@QA @QA twice").0, vec!["QA".to_string()]);
        assert_eq!(names("read @qa/report.md and @QA.txt").0, Vec::<String>::new(), "a file path isn't a mention");
        assert_eq!(names("ask @QA. Thanks").0, vec!["QA".to_string()], "a closing period still mentions");
        assert_eq!(names("no mentions here"), (vec![], false));
        let _ = (dev, qa);
    }
}
