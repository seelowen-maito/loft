//! The chat's slash commands: the list Loft reports for this agent (`GET /api/agents/{id}/commands`), the same one
//! the web and the iPhone show, with the same rules for what each does.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// With its `/`.
    pub name: String,
    pub description: String,
    /// `passthrough` (sent to the CLI as the message), `loft` (Loft runs it) or `terminal` (can't run here).
    pub tag: String,
    pub reason: Option<String>,
}

impl Command {
    pub fn terminal(&self) -> bool {
        self.tag == "terminal"
    }

    /// What picking or sending a terminal-only command says.
    pub fn terminal_note(&self) -> String {
        format!("{} can't run here. {}", self.name, self.reason.as_deref().unwrap_or("Claude Code runs it only in its own terminal app."))
    }
}

pub fn from_json(v: &Value) -> Vec<Command> {
    let text = |c: &Value, k: &str| c.get(k).and_then(|x| x.as_str()).map(String::from);
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some(Command { name: text(c, "name")?, description: text(c, "description").unwrap_or_default(), tag: text(c, "tag")?, reason: text(c, "reason") }))
        .collect()
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// A known command and the rest of the message (instructions, a topic, arguments or text to keep).
    Run(Command, String),
    /// `/something` alone that isn't a command.
    Unknown(String),
    /// An ordinary message.
    Message(String),
    Empty,
}

/// With no list loaded yet, every message is plain text.
pub fn parse(text: &str, list: &[Command]) -> Parsed {
    let t = text.trim();
    if t.is_empty() {
        return Parsed::Empty;
    }
    if t.starts_with('/') && !list.is_empty() {
        let (word, rest) = t.split_once(char::is_whitespace).map(|(w, r)| (w, r.trim())).unwrap_or((t, ""));
        let lower = word.to_lowercase();
        if let Some(c) = list.iter().find(|c| c.name == lower) {
            return Parsed::Run(c.clone(), rest.to_string());
        }
        if rest.is_empty() {
            return Parsed::Unknown(lower);
        }
    }
    Parsed::Message(t.to_string())
}

/// The commands the composer's menu offers while the user types a bare `/word`: names that start with it first,
/// then names that contain it, then descriptions that do. A bare `/` lists them all.
pub fn matching<'a>(text: &str, list: &'a [Command]) -> Vec<&'a Command> {
    let t = text.trim_start();
    if !t.starts_with('/') || t.contains(char::is_whitespace) {
        return vec![];
    }
    let q = t[1..].to_lowercase();
    let rank = |c: &Command| {
        let n = c.name[1..].to_lowercase();
        if n.starts_with(&q) { 0 } else if n.contains(&q) { 1 } else if c.description.to_lowercase().contains(&q) { 2 } else { 3 }
    };
    let mut hits: Vec<_> = list.iter().map(|c| (rank(c), c)).filter(|(r, _)| *r < 3).collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn list() -> Vec<Command> {
        from_json(&json!([
            { "name": "/clear", "description": "Clear the conversation.", "tag": "loft", "reason": null },
            { "name": "/compact", "description": "Summarize it.", "tag": "loft", "reason": null },
            { "name": "/doctor", "description": "Check the install.", "tag": "terminal", "reason": "Only in the terminal app." },
            { "name": "/review", "description": "Review a pull request.", "tag": "passthrough", "reason": null },
            { "name": "/ship", "description": "Clear the deck and ship.", "tag": "passthrough", "reason": null },
        ]))
    }

    #[test]
    fn reads_the_apis_list() {
        let l = list();
        assert_eq!(l.len(), 5);
        assert!(l[2].terminal());
        assert_eq!(l[2].terminal_note(), "/doctor can't run here. Only in the terminal app.");
        assert!(from_json(&json!({ "error": "x" })).is_empty());
    }

    #[test]
    fn parses_commands_and_messages() {
        let l = list();
        assert!(matches!(parse("/clear", &l), Parsed::Run(c, r) if c.name == "/clear" && r.is_empty()));
        assert!(matches!(parse("  /Compact keep the API notes ", &l), Parsed::Run(c, r) if c.name == "/compact" && r == "keep the API notes"));
        assert!(matches!(parse("/review 12", &l), Parsed::Run(c, r) if c.tag == "passthrough" && r == "12"));
        assert_eq!(parse("/nope", &l), Parsed::Unknown("/nope".into()));
        assert_eq!(parse("/usr/bin is odd", &l), Parsed::Message("/usr/bin is odd".into()));
        assert_eq!(parse("hello", &l), Parsed::Message("hello".into()));
        assert_eq!(parse("   ", &l), Parsed::Empty);
        assert_eq!(parse("/clear", &[]), Parsed::Message("/clear".into()), "no list yet: plain text");
    }

    #[test]
    fn menu_ranks_names_that_start_with_the_word_first() {
        let l = list();
        let n: Vec<_> = matching("/c", &l).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(n, vec!["/clear", "/compact", "/doctor", "/ship"], "doctor contains the c, ship matches by its description");
        assert_eq!(matching("/", &l).len(), l.len());
        assert!(matching("/clear now", &l).is_empty());
        assert!(matching("hi", &l).is_empty());
    }
}
