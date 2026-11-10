//! An agent's chat as terminal lines: what the web chat shows, laid out for a fixed width.

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Default, Clone, Copy)]
pub struct View {
    /// Thinking in full instead of its first line.
    pub thinking: bool,
    /// Tool results under their calls.
    pub tools: bool,
    /// Messages before the latest /clear (hidden, not deleted, as on the web).
    pub earlier: bool,
}

pub const USER: Color = Color::Cyan;
pub const AGENT: Color = Color::Green;
pub const DIM: Color = Color::DarkGray;
pub const WARN: Color = Color::Yellow;
pub const BAD: Color = Color::Red;
pub const MARK: Color = Color::Magenta;

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Index of the first message to show: just after the latest clear, unless earlier ones are asked for.
pub fn first_shown(messages: &[Value], view: &View) -> usize {
    if view.earlier {
        return 0;
    }
    messages.iter().rposition(|m| s(m, "kind") == "clear").unwrap_or(0)
}

pub fn lines(messages: &[Value], perms: &HashMap<String, Value>, agent: &str, width: u16, view: &View) -> Vec<Line<'static>> {
    let w = (width as usize).max(10);
    let mut out: Vec<Line<'static>> = Vec::new();
    let start = first_shown(messages, view);
    if start > 0 {
        out.push(Line::styled(format!("{start} earlier messages hidden · ctrl+e shows them"), Style::new().fg(DIM)));
        out.push(Line::default());
    }
    let mut last_speaker = String::new();
    for m in &messages[start..] {
        let kind = s(m, "kind");
        let role = s(m, "role");
        let content = s(m, "content");
        let meta = m.get("meta").cloned().unwrap_or(Value::Null);
        let sub = meta.get("subagent").is_some_and(|x| !x.is_null());
        match (role.as_str(), kind.as_str()) {
            ("user", "text") => {
                let from = meta.get("from_agent").and_then(|f| f.get("name")).and_then(|n| n.as_str());
                let head = match from {
                    Some(f) if meta.get("fyi").and_then(|x| x.as_bool()) == Some(true) => format!("{f} → {agent} · FYI"),
                    Some(f) => format!("{f} → {agent}"),
                    None => "You".to_string(),
                };
                gap(&mut out);
                out.push(Line::styled(head.clone(), Style::new().fg(USER).add_modifier(Modifier::BOLD)));
                text_block(&mut out, &content, w, Style::new());
                if let Some(imgs) = meta.get("images").and_then(|i| i.as_array()).filter(|i| !i.is_empty()) {
                    out.push(Line::styled(format!("  [{} attachment(s), open the web chat to view]", imgs.len()), Style::new().fg(DIM)));
                }
                last_speaker = head;
            }
            ("assistant", "text") if !sub => {
                if last_speaker != agent {
                    gap(&mut out);
                    out.push(Line::styled(agent.to_string(), Style::new().fg(AGENT).add_modifier(Modifier::BOLD)));
                    last_speaker = agent.to_string();
                }
                markdown(&mut out, &content, w);
            }
            ("assistant", "thinking") if !sub => {
                let style = Style::new().fg(DIM).add_modifier(Modifier::ITALIC);
                if view.thinking {
                    for l in wrap(&format!("✻ {content}"), w.saturating_sub(2)) {
                        out.push(Line::styled(format!("  {l}"), style));
                    }
                } else {
                    let first = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                    out.push(Line::styled(clip(&format!("  ✻ {first}"), w), style));
                }
            }
            ("assistant", "tool_use") => {
                let name = tool_name(&s(&meta, "name"));
                let what = if content.trim().is_empty() || content.trim() == s(&meta, "name") || is_bare_word(&content) {
                    input_summary(meta.get("input"))
                } else {
                    content.clone()
                };
                let prefix = if sub { "  ↳ " } else { "  ⏺ " };
                let line = clip(&format!("{prefix}{name} {what}"), w);
                let mut split = (prefix.len() + name.len()).min(line.len());
                while !line.is_char_boundary(split) {
                    split -= 1;
                }
                let (a, b) = line.split_at(split);
                out.push(Line::from(vec![
                    Span::styled(a.to_string(), Style::new().fg(if sub { DIM } else { WARN })),
                    Span::styled(b.to_string(), Style::new().fg(DIM)),
                ]));
                last_speaker.clear();
            }
            ("tool", "tool_result") => {
                let err = meta.get("is_error").and_then(|e| e.as_bool()) == Some(true);
                if err || (view.tools && !sub) {
                    let style = Style::new().fg(if err { BAD } else { DIM });
                    let shown: Vec<&str> = content.lines().take(if view.tools { 6 } else { 1 }).collect();
                    for (i, l) in shown.iter().enumerate() {
                        let p = if i == 0 { "    ⎿ " } else { "      " };
                        out.push(Line::styled(clip(&format!("{p}{l}"), w), style));
                    }
                }
            }
            (_, "permission") => {
                let id = s(&meta, "request_id");
                let p = perms.get(&id);
                let status = p.map(|p| s(p, "status")).unwrap_or_default();
                let (label, color) = match status.as_str() {
                    "pending" => ("waiting for you", WARN),
                    "allowed_once" => ("allowed once", AGENT),
                    "allowed_always" => ("always allowed", AGENT),
                    "denied" => ("denied", BAD),
                    "expired" => ("expired", DIM),
                    _ => ("", DIM),
                };
                gap(&mut out);
                let mut spans = vec![Span::styled(format!("  ⚠ Permission: {content}"), Style::new().fg(WARN).add_modifier(Modifier::BOLD))];
                if !label.is_empty() {
                    spans.push(Span::styled(format!(" · {label}"), Style::new().fg(color)));
                }
                out.push(Line::from(spans));
                if let Some(p) = p {
                    let detail = input_summary(p.get("input"));
                    for l in wrap(&detail, w.saturating_sub(4)).into_iter().take(4) {
                        out.push(Line::styled(format!("    {l}"), Style::new().fg(DIM)));
                    }
                }
                last_speaker.clear();
            }
            (_, "error") => {
                gap(&mut out);
                for l in wrap(&format!("✗ {content}"), w.saturating_sub(2)) {
                    out.push(Line::styled(format!("  {l}"), Style::new().fg(BAD)));
                }
                last_speaker.clear();
            }
            (_, "notice") => {
                for l in wrap(&content, w.saturating_sub(4)) {
                    out.push(Line::styled(format!("  · {l}"), Style::new().fg(DIM)));
                }
            }
            (_, "run_end") => {
                if meta.get("stopped").and_then(|x| x.as_bool()) == Some(true) {
                    continue;
                }
                let mut parts = vec![if meta.get("is_error").and_then(|x| x.as_bool()) == Some(true) { "✗ run failed".to_string() } else { "✓ done".to_string() }];
                if let Some(ms) = meta.get("duration_ms").and_then(|x| x.as_f64()) {
                    parts.push(duration(ms));
                }
                if let Some(t) = meta.get("num_turns").and_then(|x| x.as_i64()) {
                    parts.push(format!("{t} turns"));
                }
                if let Some(c) = meta.get("cost_usd").and_then(|x| x.as_f64()) {
                    parts.push(format!("est. ${c:.3}"));
                }
                out.push(Line::styled(format!("  {}", parts.join(" · ")), Style::new().fg(DIM)));
                last_speaker.clear();
            }
            (_, "clear" | "compact" | "resume") => {
                gap(&mut out);
                out.push(Line::styled(rule(&content, w), Style::new().fg(MARK)));
                last_speaker.clear();
            }
            _ => {}
        }
    }
    out
}

fn gap(out: &mut Vec<Line<'static>>) {
    if out.last().is_some_and(|l| !l.spans.is_empty()) {
        out.push(Line::default());
    }
}

fn text_block(out: &mut Vec<Line<'static>>, text: &str, w: usize, style: Style) {
    for para in text.lines() {
        for l in wrap(para, w.saturating_sub(2)) {
            out.push(Line::styled(format!("  {l}"), style));
        }
    }
}

/// Just enough Markdown for a terminal: code blocks, headings, bullets, and no `**`.
fn markdown(out: &mut Vec<Line<'static>>, text: &str, w: usize) {
    let mut code = false;
    for raw in text.lines() {
        let t = raw.trim_start();
        if t.starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            out.push(Line::styled(clip(&format!("  │ {raw}"), w), Style::new().fg(Color::Blue)));
            continue;
        }
        let plain = raw.replace("**", "");
        if let Some(h) = t.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim().replace("**", "");
            for l in wrap(&h, w.saturating_sub(2)) {
                out.push(Line::styled(format!("  {l}"), Style::new().add_modifier(Modifier::BOLD)));
            }
            continue;
        }
        let indent = raw.len() - t.len();
        let (lead, body) = if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            (format!("{}• ", " ".repeat(indent)), b.replace("**", ""))
        } else if let Some(dot) = t.find(". ").filter(|&d| d > 0 && d <= 3 && t[..d].chars().all(|c| c.is_ascii_digit())) {
            (format!("{}{} ", " ".repeat(indent), &t[..=dot]), t[dot + 2..].replace("**", ""))
        } else {
            (String::new(), plain.clone())
        };
        let pad = lead.width();
        for (i, l) in wrap(&body, w.saturating_sub(2 + pad)).into_iter().enumerate() {
            let p = if i == 0 { lead.clone() } else { " ".repeat(pad) };
            out.push(Line::raw(format!("  {p}{l}")));
        }
    }
}

/// `mcp__loft__set_avatar` reads as `loft·set_avatar`.
pub fn tool_name(name: &str) -> String {
    match name.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
        Some((server, tool)) => format!("{server}·{tool}"),
        None => name.to_string(),
    }
}

/// A one-word label (such as an input key) says less than the input itself.
fn is_bare_word(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.len() < 24 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What a tool call or permission request is about, on one line.
pub fn input_summary(input: Option<&Value>) -> String {
    let Some(i) = input else { return String::new() };
    for k in ["command", "file_path", "path", "pattern", "url", "query", "description", "prompt"] {
        if let Some(v) = i.get(k).and_then(|v| v.as_str()) {
            return v.to_string();
        }
    }
    let c = serde_json::to_string(i).unwrap_or_default();
    if c == "{}" || c == "null" { String::new() } else { c }
}

fn duration(ms: f64) -> String {
    let secs = ms / 1000.0;
    if secs < 60.0 { format!("{secs:.1}s") } else { format!("{}m {:02}s", (secs / 60.0) as u64, (secs % 60.0) as u64) }
}

fn rule(text: &str, w: usize) -> String {
    let t = format!(" {} ", text.trim());
    let side = w.saturating_sub(t.width()) / 2;
    format!("{}{}{}", "─".repeat(side.min(30)), t, "─".repeat(side.min(30)))
}

/// Cuts a line to `w` columns, with an ellipsis when it was longer.
pub fn clip(text: &str, w: usize) -> String {
    let text = text.replace('\t', "  ");
    if text.width() <= w {
        return text;
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw + 1 > w {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// Word-wraps one paragraph to `w` columns; words longer than a line are broken.
pub fn wrap(text: &str, w: usize) -> Vec<String> {
    let w = w.max(4);
    let text = text.replace('\t', "  ");
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in para.split(' ') {
            let ww = word.width();
            if used > 0 && used + 1 + ww > w {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            if ww > w {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > w {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push(c);
                    used += cw;
                }
                continue;
            }
            if used > 0 {
                line.push(' ');
                used += 1;
            }
            line.push_str(word);
            used += ww;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn render(msgs: &[Value], view: View) -> Vec<String> {
        lines(msgs, &HashMap::new(), "lofty", 60, &view).iter().map(text).collect()
    }

    #[test]
    fn wraps_words_and_breaks_long_ones() {
        assert_eq!(wrap("one two three four", 9), vec!["one two", "three", "four"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 10), vec![""]);
    }

    #[test]
    fn clips_with_an_ellipsis() {
        assert_eq!(clip("hello world", 6), "hello…");
        assert_eq!(clip("short", 10), "short");
    }

    #[test]
    fn shows_a_conversation_like_the_web_chat() {
        let msgs = vec![
            json!({"role":"user","kind":"text","content":"hi","meta":{}}),
            json!({"role":"assistant","kind":"thinking","content":"Let me think\nmore","meta":{"subagent":null}}),
            json!({"role":"assistant","kind":"tool_use","content":"","meta":{"name":"Bash","input":{"command":"ls"}}}),
            json!({"role":"tool","kind":"tool_result","content":"a\nb","meta":{"is_error":false}}),
            json!({"role":"assistant","kind":"text","content":"**Done.**\n- one","meta":{"subagent":null}}),
            json!({"role":"system","kind":"run_end","content":"","meta":{"duration_ms":31068,"num_turns":3}}),
        ];
        let out = render(&msgs, View::default());
        assert_eq!(out[0], "You");
        assert_eq!(out[1], "  hi");
        assert!(out.contains(&"  ✻ Let me think".to_string()));
        assert!(out.contains(&"  ⏺ Bash ls".to_string()));
        assert!(!out.iter().any(|l| l.contains('⎿')), "results hidden by default");
        assert!(out.contains(&"lofty".to_string()));
        assert!(out.contains(&"  Done.".to_string()));
        assert!(out.contains(&"  • one".to_string()));
        assert!(out.contains(&"  ✓ done · 31.1s · 3 turns".to_string()));

        let out = render(&msgs, View { tools: true, ..View::default() });
        assert!(out.contains(&"    ⎿ a".to_string()));
    }

    #[test]
    fn tool_lines_name_the_tool_once() {
        let msgs = vec![
            json!({"role":"assistant","kind":"tool_use","content":"ToolSearch","meta":{"name":"ToolSearch","input":{"query":"select:Read"}}}),
            json!({"role":"assistant","kind":"tool_use","content":"avatar","meta":{"name":"mcp__loft__set_avatar","input":{"svg":"<svg/>"}}}),
        ];
        let out = render(&msgs, View::default());
        assert_eq!(out[0], "  ⏺ ToolSearch select:Read");
        assert!(out[1].starts_with("  ⏺ loft·set_avatar "), "{}", out[1]);
    }

    #[test]
    fn numbered_lists_wrap_under_their_text() {
        let msgs = vec![json!({"role":"assistant","kind":"text","content":"1. Back up the database before you start","meta":{}})];
        let out: Vec<String> = lines(&msgs, &HashMap::new(), "lofty", 24, &View::default()).iter().map(text).collect();
        assert_eq!(out[1], "  1. Back up the");
        assert_eq!(out[2], "     database before you");
    }

    #[test]
    fn hides_what_came_before_the_latest_clear() {
        let msgs = vec![
            json!({"role":"user","kind":"text","content":"old","meta":{}}),
            json!({"role":"system","kind":"clear","content":"Context cleared.","meta":{}}),
            json!({"role":"user","kind":"text","content":"new","meta":{}}),
        ];
        let out = render(&msgs, View::default());
        assert!(out[0].starts_with("1 earlier messages hidden"));
        assert!(!out.iter().any(|l| l.contains("old")));
        assert!(render(&msgs, View { earlier: true, ..View::default() }).iter().any(|l| l.contains("old")));
    }

    #[test]
    fn names_the_agent_that_sent_a_message() {
        let msgs = vec![json!({"role":"user","kind":"text","content":"ping","meta":{"from_agent":{"id":"x","name":"Findmeal QA"}}})];
        assert_eq!(render(&msgs, View::default())[0], "Findmeal QA → lofty");
    }

    #[test]
    fn shows_a_permission_with_its_state() {
        let msgs = vec![json!({"role":"system","kind":"permission","content":"Bash","meta":{"request_id":"r1"}})];
        let perms = HashMap::from([("r1".to_string(), json!({"status":"pending","input":{"command":"rm -rf build"}}))]);
        let out: Vec<String> = lines(&msgs, &perms, "lofty", 60, &View::default()).iter().map(text).collect();
        assert_eq!(out[0], "  ⚠ Permission: Bash · waiting for you");
        assert_eq!(out[1], "    rm -rf build");
    }
}
