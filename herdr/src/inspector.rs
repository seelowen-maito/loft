//! The sidebar: the web inspector's Config, Access, Memory and Files tabs, plus the chat's actions.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::{Value, json};

use crate::composer::Composer;
use crate::transcript::{AGENT, BAD, DIM, WARN, clip, input_summary, wrap};
use crate::util;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Actions,
    Config,
    Access,
    Memory,
    Files,
}

const TABS: [(Tab, &str); 5] = [(Tab::Actions, "Actions"), (Tab::Config, "Config"), (Tab::Access, "Access"), (Tab::Memory, "Memory"), (Tab::Files, "Files")];

/// What the sidebar asks the chat to do.
#[derive(Debug, PartialEq)]
pub enum Do {
    Nothing,
    /// `POST /agents/{id}/{what}`: run, stop, restart, reset-session, read.
    Post(&'static str),
    Compact,
    Resume,
    Code,
    ClearHistory,
    ToggleEarlier,
    ToggleThinking,
    ToggleTools,
    Patch(Value),
    /// Edit a long text field in $EDITOR.
    Edit(&'static str),
    Decide(String, &'static str),
    AddMemory(String),
    DeleteMemory(i64, String),
    LoadMemories,
    LoadFiles,
    Unfocus,
}

const ACTIONS: &[(&str, &str)] = &[
    ("Run now", "Wake the agent now, as its schedule would"),
    ("Stop run", "Stop the run in flight (esc in the chat)"),
    ("Restart run", "Stop a run that went quiet; queued messages run next"),
    ("Clear context", "/clear: start a fresh session; memory is kept"),
    ("Compact", "/compact: summarize the session in place"),
    ("Resume a past session", "/resume"),
    ("Open in VS Code", "/code"),
    ("Show earlier messages", "Before the latest /clear (alt+e)"),
    ("Show full thinking", "ctrl+t"),
    ("Show tool output", "ctrl+o"),
    ("Clear chat history", "Delete every message in this chat"),
];

fn action_do(i: usize) -> Do {
    match i {
        0 => Do::Post("run"),
        1 => Do::Post("stop"),
        2 => Do::Post("restart"),
        3 => Do::Post("reset-session"),
        4 => Do::Compact,
        5 => Do::Resume,
        6 => Do::Code,
        7 => Do::ToggleEarlier,
        8 => Do::ToggleThinking,
        9 => Do::ToggleTools,
        _ => Do::ClearHistory,
    }
}

type Choices = &'static [(&'static str, &'static str)];

const MODELS: Choices = &[("sonnet", "Sonnet · balanced"), ("opus", "Opus · most capable"), ("fable", "Fable"), ("haiku", "Haiku · fastest")];
const SUB_MODELS: Choices = &[("", "Default"), ("sonnet", "Sonnet"), ("opus", "Opus"), ("fable", "Fable"), ("haiku", "Haiku")];
const EFFORTS: Choices = &[("", "Default"), ("low", "low"), ("medium", "medium"), ("high", "high"), ("xhigh", "xhigh"), ("max", "max")];
const MODES: Choices = &[("auto", "Auto"), ("default", "Ask"), ("acceptEdits", "Accept edits"), ("plan", "Plan only"), ("bypassPermissions", "Bypass")];
const ROLES: Choices = &[("lead", "Lead"), ("dev", "Dev"), ("code_reviewer", "Code reviewer"), ("qa", "QA"), ("security", "Security"), ("custom", "Custom")];
const INTERVALS: Choices = &[("0", "Manual only"), ("15", "Every 15 min"), ("60", "Hourly"), ("360", "Every 6 hours"), ("1440", "Daily")];

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Long,
    Choice(Choices),
    /// A choice stored as a number.
    Number(Choices),
}

const FIELDS: &[(&str, &str, Kind)] = &[
    ("name", "Name", Kind::Text),
    ("title", "Title", Kind::Text),
    ("group", "Group", Kind::Text),
    ("role", "Role", Kind::Choice(ROLES)),
    ("model", "Model", Kind::Choice(MODELS)),
    ("effort", "Effort", Kind::Choice(EFFORTS)),
    ("permission_mode", "Permissions", Kind::Choice(MODES)),
    ("interval_minutes", "Schedule", Kind::Number(INTERVALS)),
    ("subagent_model", "Subagent model", Kind::Choice(SUB_MODELS)),
    ("prompt", "Instructions", Kind::Long),
    ("personality", "Personality", Kind::Long),
];

fn field_value(agent: &Value, key: &str) -> String {
    match agent.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn label_of(choices: Choices, value: &str) -> String {
    choices.iter().find(|(v, _)| *v == value).map(|(_, l)| l.to_string()).unwrap_or_else(|| if value.is_empty() { "Default".into() } else { value.to_string() })
}

/// A schedule the list doesn't name, such as one set by an agent: "Every 7 days".
fn every(minutes: i64) -> String {
    match minutes {
        m if m <= 0 => "Manual only".into(),
        m if m % 1440 == 0 => format!("Every {} day{}", m / 1440, if m == 1440 { "" } else { "s" }),
        m if m % 60 == 0 => format!("Every {} hours", m / 60),
        m => format!("Every {m} min"),
    }
}

/// The next (or previous) value of a choice field.
fn cycle(choices: Choices, value: &str, back: bool) -> &'static str {
    let i = choices.iter().position(|(v, _)| *v == value);
    let n = choices.len();
    let next = match (i, back) {
        (None, _) => 0,
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
    };
    choices[next].0
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Action(usize),
    Field(usize),
    Pending(String),
    Mode,
    Heading(&'static str),
    Tool(&'static str, usize),
    AddTool(&'static str),
    AddMemory,
    Memory(usize),
    File(usize),
    Info(String),
}

impl Row {
    fn selectable(&self) -> bool {
        !matches!(self, Row::Heading(_) | Row::Info(_))
    }
}

enum Editing {
    Field(&'static str),
    Tool(&'static str),
    Memory,
}

pub struct Inspector {
    pub tab: Tab,
    sel: usize,
    editing: Option<(Editing, Composer)>,
    pub memories: Option<Vec<Value>>,
    pub files: Option<Vec<Value>>,
    open_memory: Option<i64>,
}

fn strings(agent: &Value, key: &str) -> Vec<String> {
    agent.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

pub fn pending(perms: &HashMap<String, Value>) -> Vec<&Value> {
    let mut p: Vec<&Value> = perms.values().filter(|p| p.get("status").and_then(|s| s.as_str()) == Some("pending")).collect();
    p.sort_by_key(|p| p.get("created_at").and_then(|c| c.as_str()).unwrap_or("").to_string());
    p
}

impl Inspector {
    pub fn new() -> Self {
        Self { tab: Tab::Actions, sel: 0, editing: None, memories: None, files: None, open_memory: None }
    }

    fn rows(&self, agent: &Value, perms: &HashMap<String, Value>) -> Vec<Row> {
        match self.tab {
            Tab::Actions => (0..ACTIONS.len()).map(Row::Action).collect(),
            Tab::Config => (0..FIELDS.len()).map(Row::Field).collect(),
            Tab::Access => {
                let mut rows = Vec::new();
                let p = pending(perms);
                if !p.is_empty() {
                    rows.push(Row::Heading("Waiting for you"));
                    rows.extend(p.iter().map(|p| Row::Pending(p.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string())));
                }
                rows.push(Row::Mode);
                for (key, title) in [("allowed_tools", "Allowed tools"), ("disallowed_tools", "Denied tools")] {
                    rows.push(Row::Heading(title));
                    rows.extend((0..strings(agent, key).len()).map(|i| Row::Tool(key, i)));
                    rows.push(Row::AddTool(key));
                }
                rows
            }
            Tab::Memory => match &self.memories {
                None => vec![Row::Info("Loading…".into())],
                Some(m) => {
                    let mut rows = vec![Row::AddMemory];
                    if m.is_empty() {
                        rows.push(Row::Info("No memories yet.".into()));
                    }
                    rows.extend((0..m.len()).map(Row::Memory));
                    rows
                }
            },
            Tab::Files => {
                let mut rows = vec![Row::Info(agent.get("workspace").and_then(|w| w.as_str()).unwrap_or("").to_string())];
                match &self.files {
                    None => rows.push(Row::Info("Loading…".into())),
                    Some(f) if f.is_empty() => rows.push(Row::Info("The workspace is empty.".into())),
                    Some(f) => rows.extend((0..f.len()).map(Row::File)),
                }
                rows
            }
        }
    }

    fn fix_sel(&mut self, rows: &[Row]) {
        if rows.is_empty() {
            self.sel = 0;
            return;
        }
        self.sel = self.sel.min(rows.len() - 1);
        if !rows[self.sel].selectable() {
            if let Some(i) = (self.sel..rows.len()).chain((0..self.sel).rev()).find(|&i| rows[i].selectable()) {
                self.sel = i;
            }
        }
    }

    fn step(&mut self, rows: &[Row], down: bool) {
        let mut i = self.sel;
        loop {
            let next = if down { i + 1 } else { i.wrapping_sub(1) };
            if next >= rows.len() {
                return;
            }
            i = next;
            if rows[i].selectable() {
                self.sel = i;
                return;
            }
        }
    }

    pub fn set_tab(&mut self, tab: Tab) -> Do {
        self.tab = tab;
        self.sel = 0;
        self.editing = None;
        match tab {
            Tab::Memory if self.memories.is_none() => Do::LoadMemories,
            Tab::Files if self.files.is_none() => Do::LoadFiles,
            _ => Do::Nothing,
        }
    }

    pub fn key(&mut self, k: KeyEvent, agent: &Value, perms: &HashMap<String, Value>) -> Do {
        if let Some((what, c)) = &mut self.editing {
            match k.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Enter => {
                    let text = c.text.trim().to_string();
                    let done = match what {
                        Editing::Field(key) => Do::Patch(json!({ *key: text })),
                        Editing::Tool(key) if !text.is_empty() => {
                            let mut list = strings(agent, key);
                            if !list.contains(&text) {
                                list.push(text);
                            }
                            Do::Patch(json!({ *key: list }))
                        }
                        Editing::Memory if !text.is_empty() => Do::AddMemory(text),
                        _ => Do::Nothing,
                    };
                    self.editing = None;
                    return done;
                }
                KeyCode::Backspace => c.backspace(),
                KeyCode::Delete => c.delete(),
                KeyCode::Left => c.left(),
                KeyCode::Right => c.right(),
                KeyCode::Home => c.home(),
                KeyCode::End => c.end(),
                KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => c.delete_word(),
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => c.delete_line(),
                KeyCode::Char(ch) if !k.modifiers.contains(KeyModifiers::CONTROL) => c.insert(ch),
                _ => {}
            }
            return Do::Nothing;
        }
        let rows = self.rows(agent, perms);
        self.fix_sel(&rows);
        let tab_index = TABS.iter().position(|(t, _)| *t == self.tab).unwrap_or(0);
        match k.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => Do::Unfocus,
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('[') => self.set_tab(TABS[(tab_index + TABS.len() - 1) % TABS.len()].0),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(']') => self.set_tab(TABS[(tab_index + 1) % TABS.len()].0),
            KeyCode::Char(c @ '1'..='5') => self.set_tab(TABS[(c as u8 - b'1') as usize].0),
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(&rows, false);
                Do::Nothing
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(&rows, true);
                Do::Nothing
            }
            KeyCode::Char('r') if matches!(self.tab, Tab::Memory | Tab::Files) => {
                if self.tab == Tab::Memory {
                    Do::LoadMemories
                } else {
                    Do::LoadFiles
                }
            }
            code => {
                let Some(row) = rows.get(self.sel).cloned() else { return Do::Nothing };
                self.on_row(row, code, agent)
            }
        }
    }

    fn on_row(&mut self, row: Row, code: KeyCode, agent: &Value) -> Do {
        let activate = matches!(code, KeyCode::Enter | KeyCode::Char(' '));
        let back = matches!(code, KeyCode::Char('-') | KeyCode::Backspace);
        let delete = matches!(code, KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace);
        match row {
            Row::Action(i) if activate => action_do(i),
            Row::Field(i) => {
                let (key, _, kind) = FIELDS[i];
                let value = field_value(agent, key);
                match kind {
                    Kind::Choice(c) if activate || back => Do::Patch(json!({ key: cycle(c, &value, back) })),
                    Kind::Number(c) if activate || back => Do::Patch(json!({ key: cycle(c, &value, back).parse::<i64>().unwrap_or(0) })),
                    Kind::Text if activate => {
                        self.editing = Some((Editing::Field(key), Composer::with(&value)));
                        Do::Nothing
                    }
                    Kind::Long if activate || code == KeyCode::Char('e') => Do::Edit(key),
                    _ => Do::Nothing,
                }
            }
            Row::Pending(id) => match code {
                KeyCode::Char('y') | KeyCode::Enter => Do::Decide(id, "allow_once"),
                KeyCode::Char('a') => Do::Decide(id, "allow_always"),
                KeyCode::Char('n') => Do::Decide(id, "deny"),
                _ => Do::Nothing,
            },
            Row::Mode if activate || back => Do::Patch(json!({ "permission_mode": cycle(MODES, &field_value(agent, "permission_mode"), back) })),
            Row::Tool(key, i) if delete => {
                let mut list = strings(agent, key);
                if i < list.len() {
                    list.remove(i);
                }
                Do::Patch(json!({ key: list }))
            }
            Row::AddTool(key) if activate => {
                self.editing = Some((Editing::Tool(key), Composer::default()));
                Do::Nothing
            }
            Row::AddMemory if activate => {
                self.editing = Some((Editing::Memory, Composer::default()));
                Do::Nothing
            }
            Row::Memory(i) => {
                let Some(m) = self.memories.as_ref().and_then(|m| m.get(i)) else { return Do::Nothing };
                let id = m.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                if activate {
                    self.open_memory = if self.open_memory == Some(id) { None } else { Some(id) };
                    Do::Nothing
                } else if delete {
                    Do::DeleteMemory(id, m.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string())
                } else {
                    Do::Nothing
                }
            }
            _ => Do::Nothing,
        }
    }

    pub fn hints(&self) -> &'static str {
        if self.editing.is_some() {
            return "enter save · esc cancel";
        }
        match self.tab {
            Tab::Actions => "↑↓ choose · enter run · ←→ tabs · tab back to chat",
            Tab::Config => "enter change · - previous · ←→ tabs · tab back to chat",
            Tab::Access => "y allow · a always · n deny · enter change · d remove",
            Tab::Memory => "enter open · d forget · r reload · ←→ tabs",
            Tab::Files => "r reload · ←→ tabs · tab back to chat",
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, agent: &Value, perms: &HashMap<String, Value>, focused: bool) {
        let border = if focused { Color::Cyan } else { DIM };
        let block = Block::default().borders(Borders::LEFT).border_style(Style::new().fg(border));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let w = inner.width.saturating_sub(2) as usize;
        let mut lines: Vec<Line> = Vec::new();

        let mut tabs = vec![Span::raw(" ")];
        let pending_count = pending(perms).len();
        for (t, name) in TABS {
            let style = if t == self.tab { Style::new().fg(Color::Black).bg(if focused { Color::Cyan } else { Color::Gray }) } else { Style::new().fg(DIM) };
            let label = if t == Tab::Access && pending_count > 0 { format!(" {name} {pending_count} ") } else { format!(" {name} ") };
            tabs.push(Span::styled(label, style));
        }
        lines.push(Line::from(tabs));
        lines.push(Line::default());

        let rows = self.rows(agent, perms);
        self.fix_sel(&rows);
        let mut sel_line = 0;
        for (i, row) in rows.iter().enumerate() {
            let selected = focused && i == self.sel;
            if i == self.sel {
                sel_line = lines.len();
            }
            let mark = if selected { "▸ " } else { "  " };
            let hi = |s: Style| if selected { s.add_modifier(Modifier::BOLD) } else { s };
            match row {
                Row::Action(a) => {
                    lines.push(Line::styled(format!(" {mark}{}", ACTIONS[*a].0), hi(Style::new())));
                    if selected {
                        for l in wrap(ACTIONS[*a].1, w.saturating_sub(4)) {
                            lines.push(Line::styled(format!("     {l}"), Style::new().fg(DIM)));
                        }
                    }
                }
                Row::Field(fi) => {
                    let (key, label, kind) = FIELDS[*fi];
                    let value = field_value(agent, key);
                    let shown = match kind {
                        Kind::Choice(c) => label_of(c, &value),
                        Kind::Number(c) => match c.iter().find(|(v, _)| *v == value) {
                            Some((_, l)) => l.to_string(),
                            None => every(value.parse().unwrap_or(0)),
                        },
                        Kind::Text | Kind::Long => {
                            let first = value.lines().next().unwrap_or("").to_string();
                            if first.is_empty() { "—".into() } else { first }
                        }
                    };
                    lines.push(Line::styled(format!(" {mark}{label}"), Style::new().fg(DIM)));
                    match &self.editing {
                        Some((Editing::Field(k), c)) if *k == key => lines.push(edit_line(c, w)),
                        _ => lines.push(Line::styled(format!("     {}", clip(&shown, w.saturating_sub(4))), hi(Style::new()))),
                    }
                    if selected && matches!(kind, Kind::Long) {
                        lines.push(Line::styled("     enter edits in $EDITOR", Style::new().fg(DIM)));
                    }
                }
                Row::Heading(h) => {
                    if lines.len() > 2 {
                        lines.push(Line::default());
                    }
                    lines.push(Line::styled(format!("  {h}"), Style::new().fg(DIM).add_modifier(Modifier::BOLD)));
                }
                Row::Info(t) => {
                    for l in wrap(t, w.saturating_sub(2)) {
                        lines.push(Line::styled(format!("  {l}"), Style::new().fg(DIM)));
                    }
                }
                Row::Pending(id) => {
                    if let Some(p) = perms.get(id) {
                        let tool = p.get("tool_name").and_then(|t| t.as_str()).unwrap_or("");
                        lines.push(Line::styled(format!(" {mark}⚠ {tool}"), hi(Style::new().fg(WARN))));
                        for l in wrap(&input_summary(p.get("input")), w.saturating_sub(4)).into_iter().take(if selected { 8 } else { 2 }) {
                            lines.push(Line::styled(format!("     {l}"), Style::new().fg(DIM)));
                        }
                        if selected {
                            lines.push(Line::styled("     y allow once · a always · n deny", Style::new().fg(WARN)));
                        }
                    }
                }
                Row::Mode => {
                    lines.push(Line::styled(format!(" {mark}Permission mode"), Style::new().fg(DIM)));
                    lines.push(Line::styled(format!("     {}", label_of(MODES, &field_value(agent, "permission_mode"))), hi(Style::new())));
                }
                Row::Tool(key, ti) => {
                    let t = strings(agent, key).get(*ti).cloned().unwrap_or_default();
                    let color = if *key == "allowed_tools" { AGENT } else { BAD };
                    lines.push(Line::styled(format!(" {mark}{}", clip(&t, w.saturating_sub(3))), hi(Style::new().fg(color))));
                }
                Row::AddTool(key) => match &self.editing {
                    Some((Editing::Tool(k), c)) if k == key => lines.push(edit_line(c, w)),
                    _ => lines.push(Line::styled(format!(" {mark}+ add a rule, e.g. Bash(npm *)"), hi(Style::new().fg(DIM)))),
                },
                Row::AddMemory => match &self.editing {
                    Some((Editing::Memory, c)) => lines.push(edit_line(c, w)),
                    _ => lines.push(Line::styled(format!(" {mark}+ remember something"), hi(Style::new().fg(DIM)))),
                },
                Row::Memory(mi) => {
                    let Some(m) = self.memories.as_ref().and_then(|m| m.get(*mi)) else { continue };
                    let title = m.get("title").and_then(|t| t.as_str()).unwrap_or("");
                    lines.push(Line::styled(format!(" {mark}{}", clip(title, w.saturating_sub(3))), hi(Style::new())));
                    let id = m.get("id").and_then(|x| x.as_i64());
                    if id.is_some() && id == self.open_memory {
                        let body = m.get("content").and_then(|t| t.as_str()).unwrap_or("");
                        for l in wrap(if body.is_empty() { "(no body)" } else { body }, w.saturating_sub(4)) {
                            lines.push(Line::styled(format!("     {l}"), Style::new().fg(DIM)));
                        }
                        lines.push(Line::styled(format!("     {}", util::ago(m.get("created_at").and_then(|c| c.as_str()).unwrap_or(""))), Style::new().fg(DIM)));
                    }
                }
                Row::File(fi) => {
                    let Some(e) = self.files.as_ref().and_then(|v| v.get(*fi)) else { continue };
                    let path = e.get("path").and_then(|p| p.as_str()).unwrap_or("");
                    let depth = e.get("depth").and_then(|d| d.as_u64()).unwrap_or(0) as usize;
                    let dir = e.get("dir").and_then(|d| d.as_bool()) == Some(true);
                    let repo = e.get("is_repo").and_then(|d| d.as_bool()) == Some(true);
                    let name = path.rsplit('/').next().unwrap_or(path);
                    let size = e.get("size").and_then(|s| s.as_f64()).filter(|_| !dir).map(util::bytes).unwrap_or_default();
                    let icon = if repo { "⎇ " } else if dir { "▸ " } else { "  " };
                    let left = format!(" {}{}{icon}{name}", if selected { "›" } else { " " }, "  ".repeat(depth));
                    let pad = w.saturating_sub(left.chars().count() + size.len());
                    lines.push(Line::from(vec![
                        Span::styled(clip(&left, w.saturating_sub(size.len() + 1)), if dir { Style::new().fg(Color::Blue) } else { Style::new() }),
                        Span::styled(format!("{}{size}", " ".repeat(pad)), Style::new().fg(DIM)),
                    ]));
                }
            }
        }
        let hint_h = 2u16;
        let body_h = inner.height.saturating_sub(hint_h) as usize;
        let skip = sel_line.saturating_sub(body_h.saturating_sub(4));
        let body: Vec<Line> = lines.into_iter().skip(skip).collect();
        f.render_widget(Paragraph::new(body), Rect { height: inner.height.saturating_sub(hint_h), ..inner });
        let hints = Rect { y: inner.y + inner.height.saturating_sub(hint_h), height: hint_h, ..inner };
        let text: Vec<Line> = wrap(if focused { self.hints() } else { "tab or ctrl+s to use the sidebar" }, w).into_iter().map(|l| Line::styled(format!(" {l}"), Style::new().fg(DIM))).collect();
        f.render_widget(Paragraph::new(text), hints);
    }
}

fn edit_line(c: &Composer, w: usize) -> Line<'static> {
    let (rows, _) = c.layout(w.saturating_sub(6));
    let last = rows.last().cloned().unwrap_or_default();
    Line::from(vec![Span::styled("   ✎ ", Style::new().fg(Color::Cyan)), Span::raw(last), Span::styled("▏", Style::new().fg(Color::Cyan))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: crossterm::event::KeyEventState::NONE }
    }

    #[test]
    fn cycles_choices_both_ways() {
        assert_eq!(cycle(MODELS, "sonnet", false), "opus");
        assert_eq!(cycle(MODELS, "sonnet", true), "haiku");
        assert_eq!(cycle(EFFORTS, "unknown", false), "");
        assert_eq!(every(10080), "Every 7 days");
        assert_eq!(every(120), "Every 2 hours");
    }

    #[test]
    fn config_changes_go_out_as_patches() {
        let agent = json!({"model":"sonnet","interval_minutes":60,"allowed_tools":["Bash"],"disallowed_tools":[]});
        let perms = HashMap::new();
        let mut i = Inspector::new();
        i.set_tab(Tab::Config);
        // Down to Model (row 4).
        for _ in 0..4 {
            i.key(key(KeyCode::Down), &agent, &perms);
        }
        assert_eq!(i.key(key(KeyCode::Enter), &agent, &perms), Do::Patch(json!({"model":"opus"})));
        for _ in 0..3 {
            i.key(key(KeyCode::Down), &agent, &perms);
        }
        assert_eq!(i.key(key(KeyCode::Enter), &agent, &perms), Do::Patch(json!({"interval_minutes":360})));
    }

    #[test]
    fn answers_and_edits_access() {
        let agent = json!({"allowed_tools":["Bash","WebFetch"],"disallowed_tools":[]});
        let perms = HashMap::from([("p1".to_string(), json!({"id":"p1","status":"pending","tool_name":"Bash","input":{"command":"ls"}}))]);
        let mut i = Inspector::new();
        i.set_tab(Tab::Access);
        assert_eq!(i.key(key(KeyCode::Char('a')), &agent, &perms), Do::Decide("p1".into(), "allow_always"));
        // Pending, Mode, then the first allowed tool.
        i.key(key(KeyCode::Down), &agent, &perms);
        i.key(key(KeyCode::Down), &agent, &perms);
        assert_eq!(i.key(key(KeyCode::Char('d')), &agent, &perms), Do::Patch(json!({"allowed_tools":["WebFetch"]})));
    }

    #[test]
    fn opening_memory_asks_to_load_it_once() {
        let mut i = Inspector::new();
        assert_eq!(i.set_tab(Tab::Memory), Do::LoadMemories);
        i.memories = Some(vec![]);
        assert_eq!(i.set_tab(Tab::Memory), Do::Nothing);
    }
}
