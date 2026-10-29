//! An agent's past Sessions, read from Claude Code's own transcripts (`~/.claude/projects/<workspace>/<id>.jsonl`)
//! for `/resume`. Sessions cleared before Loft kept any record of them are there too.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct PastSession {
    pub id: String,
    /// When its first message was sent (RFC 3339).
    pub started_at: String,
    pub first_message: String,
    /// The agent that sent the first message, when it was another agent's (`first_message` is then that
    /// message, without Loft's wrapper).
    pub from_agent: Option<String>,
    /// Tokens in context at its end: what resuming it writes back into the prompt cache.
    pub context_tokens: Option<u64>,
}

/// Claude Code's folder, as `claude` finds it.
fn claude_dir() -> PathBuf {
    std::env::var("CLAUDE_CONFIG_DIR").map(Into::into).unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claude"))
}

/// Where Claude Code keeps the transcripts of sessions run in `cwd`: the real path with every UTF-16 unit that
/// isn't an ASCII letter or digit turned into `-`, as its JavaScript regex does, so an emoji becomes `--`.
/// Very long names are cut and given a hash suffix, so those are found by their first 200 characters.
pub fn transcripts_dir(cwd: &Path) -> PathBuf {
    let projects = claude_dir().join("projects");
    let real = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let name: String = real
        .to_string_lossy()
        .chars()
        .flat_map(|c| if c.is_ascii_alphanumeric() { vec![c] } else { vec!['-'; c.len_utf16()] })
        .collect();
    if name.len() <= 200 {
        return projects.join(name);
    }
    std::fs::read_dir(&projects)
        .ok()
        .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&name[..200]))))
        .unwrap_or_else(|| projects.join(&name[..200]))
}

/// The Sessions in a transcripts folder, newest first. Files without a first message (a session that never
/// got going) are left out.
pub fn list(dir: &Path) -> Vec<PastSession> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<PastSession> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|p| read(&p))
        .collect();
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

/// One Session, if `id` is a transcript in `dir`.
pub fn find(dir: &Path, id: &str) -> Option<PastSession> {
    // A Session id is a uuid; anything else could walk out of the folder.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return None;
    }
    read(&dir.join(format!("{id}.jsonl")))
}

fn read(file: &Path) -> Option<PastSession> {
    let id = file.file_stem()?.to_string_lossy().into_owned();
    let mut started_at = None;
    let mut first_message = None;
    let mut context = None;
    for line in BufReader::new(std::fs::File::open(file).ok()?).lines().map_while(Result::ok) {
        // Most lines are tool output; only parse the ones that can matter.
        if first_message.is_some() && !line.contains("\"usage\"") && !line.contains("compact_boundary") {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        if started_at.is_none() {
            started_at = ev["timestamp"].as_str().map(String::from);
        }
        if first_message.is_none() && ev["type"] == "user" && ev["isMeta"] != true && ev["isSidechain"] != true {
            first_message = text_of(&ev["message"]["content"]);
        }
        context = context_step(&ev).unwrap_or(context);
    }
    let text: String = first_message?;
    let (from_agent, first_message) = match from_agent(&text) {
        Some((name, body)) => (Some(name.to_string()), body),
        None => (None, text.as_str()),
    };
    let first_message = first_message.chars().take(300).collect();
    Some(PastSession { id, started_at: started_at?, first_message, from_agent, context_tokens: context })
}

/// A message from another agent, as `agents::message_from_agent` wraps it: the sender's name and the message.
fn from_agent(text: &str) -> Option<(&str, &str)> {
    let (head, body) = text.strip_prefix("[Message from agent ")?.split_once(", not from the user]\n")?;
    let name = head.rsplit_once(" (id ")?.0;
    let body = body.rsplit_once("\n\n[Your replies in this chat are seen by the user").map_or(body, |(b, _)| b);
    Some((name, body.trim()))
}

/// A user message's text: a plain string, or its first text block.
fn text_of(content: &Value) -> Option<String> {
    let s = match content {
        Value::String(s) => s.as_str(),
        Value::Array(blocks) => blocks.iter().find(|b| b["type"] == "text")?["text"].as_str()?,
        _ => return None,
    };
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Context size after one transcript event, if it says: the last main-thread reply's input, or the compacted
/// size if a compaction came after it.
fn context_step(ev: &Value) -> Option<Option<u64>> {
    match (ev["type"].as_str(), ev["subtype"].as_str()) {
        (Some("assistant"), _) if !ev["isSidechain"].as_bool().unwrap_or(false) => {
            let u = &ev["message"]["usage"];
            u.is_object().then(|| Some(["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"].iter().filter_map(|k| u[*k].as_u64()).sum()))
        }
        (Some("system"), Some("compact_boundary")) => Some(ev["compactMetadata"]["postTokens"].as_u64()),
        _ => None,
    }
}

/// Context size from the end of a Claude Code session transcript, wherever it lives.
pub fn context_of(sid: &str) -> Option<u64> {
    let projects = claude_dir().join("projects");
    let file = std::fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(format!("{sid}.jsonl"))).find(|p| p.exists())?;
    let mut last = None;
    for line in BufReader::new(std::fs::File::open(file).ok()?).lines().map_while(Result::ok) {
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        last = context_step(&ev).unwrap_or(last);
    }
    last
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("loft-sessions-{}", uuid::Uuid::new_v4()))
    }

    pub fn write(dir: &Path, id: &str, lines: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{id}.jsonl")), lines.join("\n")).unwrap();
    }

    #[test]
    fn lists_sessions_newest_first_with_date_first_message_and_size() {
        let dir = tmp();
        write(&dir, "aaaa-1", &[
            r#"{"type":"custom-title","title":"x"}"#,
            r#"{"type":"queue-operation","timestamp":"2026-10-20T09:00:00Z"}"#,
            r#"{"type":"user","isMeta":true,"timestamp":"2026-10-20T09:00:01Z","message":{"content":"<meta>"}}"#,
            r#"{"type":"user","timestamp":"2026-10-20T09:00:02Z","message":{"content":"  Fix the login bug  "}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"usage":{"input_tokens":999999}}}"#,
            r#"{"type":"assistant","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":40000,"cache_creation_input_tokens":2000}}}"#,
        ]);
        write(&dir, "bbbb-2", &[
            r#"{"type":"user","timestamp":"2026-10-27T15:30:00Z","message":{"content":[{"type":"image"},{"type":"text","text":"Look at this"}]}}"#,
            r#"{"type":"assistant","message":{"usage":{"input_tokens":5,"cache_read_input_tokens":150000}}}"#,
            r#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"postTokens":3000}}"#,
        ]);
        // Never got a message: not a Session to go back to.
        write(&dir, "cccc-3", &[r#"{"type":"queue-operation","timestamp":"2026-10-28T10:00:00Z"}"#]);
        std::fs::write(dir.join("notes.txt"), "not a transcript").unwrap();

        let s = list(&dir);
        assert_eq!(s.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["bbbb-2", "aaaa-1"]);
        assert_eq!(s[0].first_message, "Look at this");
        assert_eq!(s[0].context_tokens, Some(3000), "the compacted size");
        assert_eq!(s[1].started_at, "2026-10-20T09:00:00Z");
        assert_eq!(s[1].first_message, "Fix the login bug");
        assert_eq!(s[1].context_tokens, Some(42010), "the main thread's last reply, not a subagent's");
        assert!(list(&dir.join("missing")).is_empty());
    }

    #[test]
    fn a_session_another_agent_started_shows_the_sender_and_its_message() {
        let dir = tmp();
        let wrapped = serde_json::to_string(&format!(
            "[Message from agent Loft Lead (id c682), not from the user]\n#100 passed review. Next: #101 {}\n\n[Your replies in this chat are seen by the user, not by Loft Lead. To answer Loft Lead, call `mcp__loft__message_agent` with agent \"c682\".]",
            "x".repeat(400),
        ))
        .unwrap();
        write(&dir, "aaaa-1", &[&format!(r#"{{"type":"user","timestamp":"2026-10-20T09:00:00Z","message":{{"content":{wrapped}}}}}"#)]);
        write(&dir, "bbbb-2", &[r#"{"type":"user","timestamp":"2026-10-21T09:00:00Z","message":{"content":"[Message from agent is how they start]"}}"#]);
        write(&dir, "cccc-3", &[r#"{"type":"user","timestamp":"2026-10-22T09:00:00Z","message":{"content":"[Message from agent Loft Reviewer (id 8158), not from the user]\nRound 1 failed.\n\n[Your replies in this chat are seen by the user, not by Loft Reviewer. To answer Loft Reviewer, call `mcp__loft__message_agent` with agent \"8158\".]"}}"#]);
        let from_agent = find(&dir, "aaaa-1").unwrap();
        assert_eq!(from_agent.from_agent.as_deref(), Some("Loft Lead"));
        assert!(from_agent.first_message.starts_with("#100 passed review. Next: #101 xxx"), "{}", from_agent.first_message);
        assert_eq!(from_agent.first_message.chars().count(), 300, "cut after the wrapper is gone");
        let short = find(&dir, "cccc-3").unwrap();
        assert_eq!((short.from_agent.as_deref(), short.first_message.as_str()), (Some("Loft Reviewer"), "Round 1 failed."), "without the reply hint");
        let from_user = find(&dir, "bbbb-2").unwrap();
        assert_eq!((from_user.from_agent, from_user.first_message.as_str()), (None, "[Message from agent is how they start]"));
    }

    #[test]
    fn find_only_reads_transcripts_in_the_folder() {
        let root = tmp();
        let (dir, other) = (root.join("agent"), root.join("other-agent"));
        let hi = [r#"{"type":"user","timestamp":"2026-10-20T09:00:00Z","message":{"content":"hi"}}"#];
        write(&dir, "aaaa-1", &hi);
        // Another workspace's real transcript, one `..` away.
        write(&other, "cccc-2", &hi);
        assert_eq!(find(&dir, "aaaa-1").unwrap().first_message, "hi");
        assert!(find(&dir, "bbbb").is_none());
        assert!(find(&dir, "../other-agent/cccc-2").is_none(), "no reading another folder's transcript");
        assert!(find(&dir, "").is_none());
    }

    #[test]
    fn the_folder_is_named_like_claude_code_names_it() {
        let dir = tmp();
        let ws = dir.join("agent.local_1");
        std::fs::create_dir_all(&ws).unwrap();
        let name = transcripts_dir(&ws).file_name().unwrap().to_string_lossy().into_owned();
        let real = ws.canonicalize().unwrap().to_string_lossy().replace(['/', '.', '_'], "-");
        assert_eq!(name, real);
        assert!(name.ends_with("-agent-local-1"));
    }

    #[test]
    fn an_emoji_in_the_path_counts_as_two_characters() {
        let ws = tmp().join("rocket 🚀 café");
        std::fs::create_dir_all(&ws).unwrap();
        let name = transcripts_dir(&ws).file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-rocket----caf-"), "{name}");
    }
}
