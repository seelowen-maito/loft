//! The chat's slash commands: the same ones, with the same rules, as the web composer.

pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command { name: "/clear", description: "Clear the conversation context and start fresh. Memory is kept." },
    Command { name: "/compact", description: "Summarize the conversation so later runs carry less context. Add instructions after it to steer the summary." },
    Command { name: "/resume", description: "Go back to one of this agent's past sessions." },
    Command { name: "/code", description: "Open this agent's workspace folder in VS Code." },
    Command { name: "/grill", description: "Plan an idea: the agent grills you on it in rounds of questions, each with a recommendation. Add the topic after it." },
];

#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// A known command and the rest of the message (instructions, a topic, or text to keep).
    Run(&'static str, String),
    /// `/something` alone that isn't a command.
    Unknown(String),
    /// An ordinary message.
    Message(String),
    Empty,
}

pub fn parse(text: &str) -> Parsed {
    let t = text.trim();
    if t.is_empty() {
        return Parsed::Empty;
    }
    if t.starts_with('/') {
        let (word, rest) = t.split_once(char::is_whitespace).map(|(w, r)| (w, r.trim())).unwrap_or((t, ""));
        let lower = word.to_lowercase();
        if let Some(c) = COMMANDS.iter().find(|c| c.name == lower) {
            return Parsed::Run(c.name, rest.to_string());
        }
        if rest.is_empty() {
            return Parsed::Unknown(lower);
        }
    }
    Parsed::Message(t.to_string())
}

/// The commands the composer's menu offers while the user types a bare `/word`.
pub fn matching(text: &str) -> Vec<&'static Command> {
    let t = text.trim_start();
    if !t.starts_with('/') || t.contains(char::is_whitespace) {
        return vec![];
    }
    let lower = t.to_lowercase();
    COMMANDS.iter().filter(|c| c.name.starts_with(&lower)).collect()
}

pub fn names() -> String {
    COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_messages() {
        assert_eq!(parse("/clear"), Parsed::Run("/clear", "".into()));
        assert_eq!(parse("  /Compact keep the API notes "), Parsed::Run("/compact", "keep the API notes".into()));
        assert_eq!(parse("/grill a dark mode"), Parsed::Run("/grill", "a dark mode".into()));
        assert_eq!(parse("/nope"), Parsed::Unknown("/nope".into()));
        assert_eq!(parse("/usr/bin is odd"), Parsed::Message("/usr/bin is odd".into()));
        assert_eq!(parse("hello"), Parsed::Message("hello".into()));
        assert_eq!(parse("   "), Parsed::Empty);
    }

    #[test]
    fn menu_matches_the_typed_prefix() {
        let n: Vec<_> = matching("/c").iter().map(|c| c.name).collect();
        assert_eq!(n, vec!["/clear", "/compact", "/code"]);
        assert_eq!(matching("/").len(), COMMANDS.len());
        assert!(matching("/clear now").is_empty());
        assert!(matching("hi").is_empty());
    }
}
