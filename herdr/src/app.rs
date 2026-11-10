//! One Loft agent's chat in a terminal: state, keys and live updates. Drawing is in `ui.rs`.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

use crate::commands::{self, Parsed};
use crate::composer::Composer;
use crate::herdr::{Presence, Report, presence};
use crate::inspector::{self, Do, Inspector};
use crate::loft::{Live, Loft};
use crate::transcript::View;

pub enum Msg {
    Live(Live),
    Done(&'static str, Result<Value, String>),
}

pub struct Note {
    pub text: String,
    pub error: bool,
}

pub enum Overlay {
    Sessions { list: Vec<Value>, current: Option<String>, sel: usize },
    Confirm { text: String, then: Confirmed },
}

#[derive(Clone)]
pub enum Confirmed {
    ClearHistory,
    DeleteMemory(i64),
}

pub struct App {
    pub loft: Loft,
    pub id: String,
    tx: Sender<Msg>,
    pub agent: Value,
    pub messages: Vec<Value>,
    pub perms: HashMap<String, Value>,
    pub grill: Option<Value>,
    pub grill_sel: usize,
    pub view: View,
    /// Transcript lines scrolled up from the bottom.
    pub scroll: usize,
    pub max_scroll: usize,
    pub composer: Composer,
    pub menu_sel: usize,
    /// Esc closed the command menu; it opens again when the text changes.
    pub menu_closed: bool,
    pub note: Option<Note>,
    pub connected: bool,
    pub offline: Option<String>,
    pub sidebar: bool,
    pub sidebar_focus: bool,
    pub inspector: Inspector,
    pub overlay: Option<Overlay>,
    pub quit: bool,
    quit_armed: bool,
    pub deleted: bool,
    pub loaded: bool,
    /// A long field to edit in $EDITOR, picked up by the main loop.
    pub editor: Option<&'static str>,
    pub tick: u64,
    report: Option<Sender<Report>>,
    last_presence: Option<Presence>,
    pub focused: bool,
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn client_id() -> String {
    format!("herdr-{}-{}", std::process::id(), crate::herdr::now_ms())
}

impl App {
    pub fn new(loft: Loft, id: String, tx: Sender<Msg>, report: Option<Sender<Report>>) -> Self {
        Self {
            loft,
            id,
            tx,
            agent: Value::Null,
            messages: vec![],
            perms: HashMap::new(),
            grill: None,
            grill_sel: 0,
            view: View::default(),
            scroll: 0,
            max_scroll: 0,
            composer: Composer::default(),
            menu_sel: 0,
            menu_closed: false,
            note: None,
            connected: false,
            offline: None,
            sidebar: false,
            sidebar_focus: false,
            inspector: Inspector::new(),
            overlay: None,
            quit: false,
            quit_armed: false,
            deleted: false,
            loaded: false,
            editor: None,
            tick: 0,
            report,
            last_presence: None,
            focused: true,
        }
    }

    pub fn name(&self) -> String {
        let n = str_of(&self.agent, "name");
        if n.is_empty() { "agent".into() } else { n.to_string() }
    }

    pub fn running(&self) -> bool {
        self.agent.get("running").and_then(|v| v.as_bool()) == Some(true)
    }

    /// Runs a Loft call on a worker thread; its result comes back as `Msg::Done(what, …)`.
    pub fn work(&self, what: &'static str, f: impl FnOnce(&Loft) -> anyhow::Result<Value> + Send + 'static) {
        let loft = self.loft.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = f(&loft).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Done(what, r));
        });
    }

    pub fn reload(&self) {
        let id = self.id.clone();
        self.work("agent", move |l| l.agent(&id));
        let id = self.id.clone();
        self.work("messages", move |l| l.messages(&id).map(Value::Array));
        let id = self.id.clone();
        self.work("perms", move |l| l.permissions(&id).map(Value::Array));
        let id = self.id.clone();
        self.work("grill", move |l| l.get(&format!("/agents/{id}/grill")));
    }

    fn say(&mut self, text: impl Into<String>) {
        self.note = Some(Note { text: text.into(), error: false });
    }
    fn fail(&mut self, text: impl Into<String>) {
        self.note = Some(Note { text: text.into(), error: true });
    }

    pub fn pending(&self) -> Option<&Value> {
        inspector::pending(&self.perms).into_iter().next()
    }

    /// The grill's open round, when it has questions for the user to answer now.
    pub fn grill_round(&self) -> Option<&Vec<Value>> {
        let g = self.grill.as_ref()?;
        if str_of(g, "state") != "active" || g.get("waiting").and_then(|w| w.as_bool()) == Some(true) {
            return None;
        }
        g.get("questions")?.as_array().filter(|q| !q.is_empty())
    }

    fn grill_open(&self) -> bool {
        self.grill_round().is_some_and(|qs| qs.iter().any(|q| str_of(q, "state") == "open"))
    }

    /// Tells Herdr when the state it shows should change.
    pub fn report(&mut self) {
        if self.agent.is_null() {
            return;
        }
        let p = presence(&self.agent, self.pending().map(|p| str_of(p, "tool_name").to_string()).as_deref(), self.grill_open());
        if self.last_presence.as_ref() != Some(&p) {
            if let Some(tx) = &self.report {
                let _ = tx.send(Report::Presence(p.clone()));
            }
            self.last_presence = Some(p);
        }
    }

    fn mark_read(&mut self) {
        if self.focused && self.agent.get("unread_reply").and_then(|v| v.as_bool()) == Some(true) {
            if let Some(a) = self.agent.as_object_mut() {
                a.insert("unread_reply".into(), json!(false));
            }
            let id = self.id.clone();
            self.work("read", move |l| l.action(&id, "read"));
        }
    }

    pub fn on_focus(&mut self, focused: bool) {
        self.focused = focused;
        if focused {
            self.mark_read();
        }
    }

    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Live(Live::Connected) => {
                if !self.connected && self.loaded {
                    self.reload();
                }
                self.connected = true;
                self.offline = None;
            }
            Msg::Live(Live::Disconnected(e)) => {
                self.connected = false;
                self.offline = Some(e);
            }
            Msg::Live(Live::Event(e)) => self.on_event(e),
            Msg::Done(what, r) => self.on_done(what, r),
        }
        self.report();
    }

    fn on_event(&mut self, e: Value) {
        match str_of(&e, "type") {
            "agent_updated" => {
                if let Some(a) = e.get("agent").filter(|a| str_of(a, "id") == self.id) {
                    self.agent = a.clone();
                    self.mark_read();
                }
            }
            "agent_deleted" if str_of(&e, "agent_id") == self.id => {
                self.deleted = true;
                self.fail(format!("{} was deleted in Loft. Press ctrl+c to close this pane.", self.name()));
            }
            "message" => {
                let Some(m) = e.get("message").filter(|m| str_of(m, "agent_id") == self.id) else { return };
                let id = m.get("id").and_then(|i| i.as_i64());
                match self.messages.iter_mut().find(|x| x.get("id").and_then(|i| i.as_i64()) == id) {
                    Some(old) => *old = m.clone(),
                    None => {
                        self.messages.push(m.clone());
                        if self.scroll > 0 {
                            // Keep the reader's place while new lines arrive below.
                            self.scroll += 1;
                        }
                    }
                }
            }
            "permission" => {
                if let Some(p) = e.get("request").filter(|p| str_of(p, "agent_id") == self.id) {
                    self.perms.insert(str_of(p, "id").to_string(), p.clone());
                }
            }
            "memory" if str_of(&e, "agent_id") == self.id && self.inspector.memories.is_some() => self.load_memories(),
            "grill" => {
                if let Some(g) = e.get("session").filter(|g| str_of(g, "agent_id") == self.id) {
                    self.grill = (str_of(g, "state") != "ended").then(|| g.clone());
                }
            }
            _ => {}
        }
    }

    fn on_done(&mut self, what: &'static str, r: Result<Value, String>) {
        let v = match r {
            Ok(v) => v,
            Err(e) => {
                if what == "message" {
                    let (err, text) = e.split_once('\u{0}').unwrap_or((e.as_str(), ""));
                    if self.composer.is_empty() {
                        self.composer.set(text);
                    }
                    self.fail(format!("Not sent: {err}"));
                } else if what == "grill" {
                    self.grill = None;
                } else if what == "agent" && !self.loaded {
                    self.fail(format!("Couldn't load the agent: {e}"));
                } else if what != "read" {
                    self.fail(e);
                }
                return;
            }
        };
        match what {
            "agent" => {
                self.agent = v;
                self.loaded = true;
                self.mark_read();
            }
            "messages" => {
                self.messages = v.as_array().cloned().unwrap_or_default();
                self.scroll = 0;
            }
            "perms" => {
                self.perms = v.as_array().into_iter().flatten().map(|p| (str_of(p, "id").to_string(), p.clone())).collect();
            }
            "grill" => self.grill = (v.is_object() && str_of(&v, "state") != "ended").then_some(v),
            "send" | "message" => {
                if v.get("queued").and_then(|q| q.as_bool()) == Some(true) {
                    self.say("Queued: it goes in when the current run ends.");
                }
            }
            "patch" => {
                if let Some(n) = v.get("notice").and_then(|n| n.as_str()).filter(|n| !n.is_empty()) {
                    self.say(n.to_string());
                } else {
                    self.say("Saved.");
                }
                if v.get("id").is_some() {
                    self.agent = v;
                }
            }
            "sessions" => {
                let list = v.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
                if list.is_empty() {
                    self.say("No past sessions yet.");
                } else {
                    let current = v.get("current").and_then(|c| c.as_str()).map(str::to_string);
                    self.overlay = Some(Overlay::Sessions { list, current, sel: 0 });
                }
            }
            "resume" => self.say("Resumed that session."),
            "reset" => self.say("Context cleared. Memory carries over."),
            "compact" => self.say("Compacting…"),
            "clear-history" => {
                self.messages.clear();
                self.say("Chat history cleared.");
            }
            "editor" => {
                let path = str_of(&v, "path").to_string();
                match v.get("url").and_then(|u| u.as_str()) {
                    Some(url) => {
                        open_url(url);
                        self.say(format!("Opening {path} in VS Code over Remote-SSH"));
                    }
                    None => self.say(format!("Opened {path} in VS Code")),
                }
            }
            "memories" => self.inspector.memories = Some(v.as_array().cloned().unwrap_or_default()),
            "files" => self.inspector.files = Some(v.as_array().cloned().unwrap_or_default()),
            "memory-added" | "memory-deleted" => self.load_memories(),
            "decide" => {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    self.perms.insert(id.to_string(), v.clone());
                }
            }
            "grill-cmd" => {
                if let Some(s) = v.get("session") {
                    self.grill = (str_of(s, "state") != "ended").then(|| s.clone());
                }
            }
            "run" => self.say("Woke the agent."),
            "stop" => self.say("Stopped."),
            "restart" => self.say("Restarted the run."),
            _ => {}
        }
    }

    fn load_memories(&self) {
        let id = self.id.clone();
        self.work("memories", move |l| l.memories(&id).map(Value::Array));
    }

    fn load_files(&self) {
        let id = self.id.clone();
        self.work("files", move |l| l.files(&id).map(Value::Array));
    }

    // ---- input --------------------------------------------------------------

    pub fn menu(&self) -> Vec<&'static commands::Command> {
        if self.menu_closed { vec![] } else { commands::matching(&self.composer.text) }
    }

    pub fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        if ctrl && k.code == KeyCode::Char('c') {
            if !self.composer.is_empty() {
                self.composer.take();
            } else if self.quit_armed || self.deleted {
                self.quit = true;
            } else {
                self.quit_armed = true;
                self.say("Press ctrl+c again to close this chat.");
            }
            return;
        }
        self.quit_armed = false;
        if self.overlay.is_some() {
            return self.overlay_key(k);
        }
        // Keys that work everywhere.
        match k.code {
            KeyCode::F(2) => return self.toggle_sidebar(),
            KeyCode::Char('s') if ctrl => return self.toggle_sidebar(),
            KeyCode::Char('t') if ctrl => return self.view.thinking = !self.view.thinking,
            KeyCode::Char('o') if ctrl => return self.view.tools = !self.view.tools,
            KeyCode::Char('e') if alt => return self.view.earlier = !self.view.earlier,
            KeyCode::Char('r') if ctrl => return self.post("run"),
            KeyCode::Char('l') if ctrl => return self.reload(),
            KeyCode::PageUp => return self.scroll_by(10, true),
            KeyCode::PageDown => return self.scroll_by(10, false),
            _ => {}
        }
        if self.sidebar && self.sidebar_focus {
            let d = self.inspector.key(k, &self.agent, &self.perms);
            return self.run_do(d);
        }
        self.composer_key(k, ctrl, alt);
    }

    pub fn scroll_by(&mut self, n: usize, up: bool) {
        self.scroll = if up { (self.scroll + n).min(self.max_scroll) } else { self.scroll.saturating_sub(n) };
    }

    fn toggle_sidebar(&mut self) {
        if self.sidebar && !self.sidebar_focus {
            self.sidebar_focus = true;
        } else {
            self.sidebar = !self.sidebar;
            self.sidebar_focus = self.sidebar;
        }
    }

    fn composer_key(&mut self, k: KeyEvent, ctrl: bool, alt: bool) {
        let menu = self.menu();
        let empty = self.composer.is_empty();
        match k.code {
            KeyCode::Enter if alt || k.modifiers.contains(KeyModifiers::SHIFT) => self.composer.insert('\n'),
            KeyCode::Char('j') if ctrl => self.composer.insert('\n'),
            KeyCode::Enter => {
                if let Some(c) = menu.get(self.menu_sel.min(menu.len().saturating_sub(1))) {
                    if self.composer.text.trim().to_lowercase() != c.name {
                        return self.pick(c.name);
                    }
                }
                self.submit();
            }
            KeyCode::Tab => {
                if let Some(c) = menu.get(self.menu_sel.min(menu.len().saturating_sub(1))) {
                    self.composer.set(&format!("{} ", c.name));
                } else if self.sidebar {
                    self.sidebar_focus = true;
                }
            }
            KeyCode::Esc => {
                if !menu.is_empty() {
                    self.menu_closed = true;
                } else if self.running() && empty {
                    self.post("stop");
                } else if self.note.is_some() {
                    self.note = None;
                }
            }
            KeyCode::Up if !menu.is_empty() => self.menu_sel = self.menu_sel.saturating_sub(1),
            KeyCode::Down if !menu.is_empty() => self.menu_sel = (self.menu_sel + 1).min(menu.len() - 1),
            KeyCode::Up if empty && self.grill_round().is_some() => self.grill_sel = self.grill_sel.saturating_sub(1),
            KeyCode::Down if empty && self.grill_round().is_some() => {
                let n = self.grill_round().map(|q| q.len()).unwrap_or(1);
                self.grill_sel = (self.grill_sel + 1).min(n.saturating_sub(1));
            }
            KeyCode::Up => {
                if !self.composer.vertical(false) {
                    self.scroll_by(3, true);
                }
            }
            KeyCode::Down => {
                if !self.composer.vertical(true) {
                    self.scroll_by(3, false);
                }
            }
            KeyCode::Char(c @ ('y' | 'a' | 'n')) if empty && !ctrl && !alt && self.pending().is_some() => {
                let id = str_of(self.pending().unwrap(), "id").to_string();
                let decision = match c {
                    'y' => "allow_once",
                    'a' => "allow_always",
                    _ => "deny",
                };
                self.decide(id, decision);
            }
            KeyCode::Char('g') if ctrl => self.grill_command("send", None),
            KeyCode::Char('d') if alt => {
                if let Some(q) = self.selected_question() {
                    self.grill_command("decide", Some(q));
                }
            }
            KeyCode::Backspace if alt || ctrl => self.composer.delete_word(),
            KeyCode::Backspace => self.composer.backspace(),
            KeyCode::Delete => self.composer.delete(),
            KeyCode::Left => self.composer.left(),
            KeyCode::Right => self.composer.right(),
            KeyCode::Home => self.composer.home(),
            KeyCode::End => self.composer.end(),
            KeyCode::Char('e') if ctrl => self.composer.end(),
            KeyCode::Char('w') if ctrl => self.composer.delete_word(),
            KeyCode::Char('u') if ctrl => self.composer.delete_line(),
            KeyCode::Char(c) if !ctrl => {
                self.composer.insert(c);
                self.menu_closed = false;
                self.menu_sel = 0;
                self.note = None;
            }
            _ => {}
        }
    }

    pub fn paste(&mut self, text: &str) {
        if self.overlay.is_some() {
            return;
        }
        if self.sidebar && self.sidebar_focus {
            return;
        }
        self.composer.insert_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
    }

    fn selected_question(&self) -> Option<String> {
        let qs = self.grill_round()?;
        qs.get(self.grill_sel.min(qs.len().saturating_sub(1))).map(|q| str_of(q, "id").to_string())
    }

    /// A command picked from the menu: /compact and /grill wait for their text, the others run.
    fn pick(&mut self, name: &'static str) {
        self.menu_sel = 0;
        match name {
            "/grill" => {
                self.composer.set("/grill ");
                self.say("Add what to plan, then press Enter.");
            }
            "/compact" => {
                self.composer.set("/compact ");
                self.say("Add instructions to steer the summary (optional), then press Enter.");
            }
            _ => {
                self.composer.set(name);
                self.submit();
            }
        }
    }

    fn submit(&mut self) {
        if self.deleted {
            return;
        }
        let text = self.composer.text.clone();
        let id = self.id.clone();
        match commands::parse(&text) {
            Parsed::Empty => {
                if let Some(q) = self.selected_question() {
                    // An empty Enter on a grill question accepts its recommendation.
                    self.answer(q, "yes".into());
                }
            }
            Parsed::Run("/clear", rest) => {
                self.composer.set(&rest);
                self.work("reset", move |l| l.action(&id, "reset-session"));
            }
            Parsed::Run("/compact", instructions) => {
                self.composer.take();
                self.work("compact", move |l| l.compact(&id, &instructions));
            }
            Parsed::Run("/resume", rest) => {
                self.composer.set(&rest);
                self.work("sessions", move |l| l.get(&format!("/agents/{id}/sessions")));
            }
            Parsed::Run("/code", rest) => {
                self.composer.set(&rest);
                self.work("editor", move |l| l.action(&id, "open-editor"));
            }
            Parsed::Run("/grill", topic) => {
                self.composer.take();
                let text = if topic.is_empty() { "Grill me".to_string() } else { format!("Grill me on {topic}") };
                let cid = client_id();
                self.work("send", move |l| l.grill_start(&id, &text, &cid));
            }
            Parsed::Run(other, _) => self.fail(format!("{other} isn't available here.")),
            Parsed::Unknown(name) => self.fail(format!("Unknown command {name}. Available: {}", commands::names())),
            Parsed::Message(t) => {
                if let Some(q) = self.selected_question() {
                    self.composer.take();
                    return self.answer(q, t);
                }
                self.composer.take();
                self.scroll = 0;
                let cid = client_id();
                let tx = self.tx.clone();
                let loft = self.loft.clone();
                // On failure, the text goes back in the composer.
                std::thread::spawn(move || {
                    let r = loft.send(&id, &t, &cid).map_err(|e| format!("{e}\u{0}{t}"));
                    let _ = tx.send(Msg::Done("message", r));
                });
            }
        }
    }

    fn answer(&mut self, question: String, text: String) {
        let Some(session) = self.grill.as_ref().map(|g| str_of(g, "session_id").to_string()) else { return };
        let id = self.id.clone();
        self.work("grill-answer", move |l| {
            l.post(&format!("/agents/{id}/messages"), json!({ "text": text, "images": [], "meta": { "grill": { "role": "answer", "session_id": session, "question_id": question, "discuss": false } } }))
        });
        let n = self.grill_round().map(|q| q.len()).unwrap_or(1);
        self.grill_sel = (self.grill_sel + 1).min(n.saturating_sub(1));
    }

    fn grill_command(&mut self, command: &'static str, question: Option<String>) {
        let Some(session) = self.grill.as_ref().map(|g| str_of(g, "session_id").to_string()) else { return };
        self.work("grill-cmd", move |l| {
            l.post(&format!("/grill/{session}/command"), json!({ "command": command, "question_id": question, "client_id": client_id() }))
        });
    }

    fn decide(&mut self, id: String, decision: &'static str) {
        self.work("decide", move |l| l.decide(&id, decision));
    }

    fn post(&mut self, what: &'static str) {
        let id = self.id.clone();
        let label: &'static str = match what {
            "run" => "run",
            "stop" => "stop",
            "restart" => "restart",
            "reset-session" => "reset",
            _ => "read",
        };
        self.work(label, move |l| l.action(&id, what));
    }

    fn run_do(&mut self, d: Do) {
        let id = self.id.clone();
        match d {
            Do::Nothing => {}
            Do::Unfocus => self.sidebar_focus = false,
            Do::Post(what) => self.post(what),
            Do::Compact => {
                self.sidebar_focus = false;
                self.composer.set("/compact ");
                self.say("Add instructions to steer the summary (optional), then press Enter.");
            }
            Do::Resume => self.work("sessions", move |l| l.get(&format!("/agents/{id}/sessions"))),
            Do::Code => self.work("editor", move |l| l.action(&id, "open-editor")),
            Do::ClearHistory => {
                self.overlay = Some(Overlay::Confirm { text: format!("Delete every message in {}'s chat? Its session and memory stay.", self.name()), then: Confirmed::ClearHistory })
            }
            Do::ToggleEarlier => self.view.earlier = !self.view.earlier,
            Do::ToggleThinking => self.view.thinking = !self.view.thinking,
            Do::ToggleTools => self.view.tools = !self.view.tools,
            Do::Patch(p) => self.work("patch", move |l| l.update_agent(&id, p)),
            Do::Edit(field) => self.editor = Some(field),
            Do::Decide(pid, decision) => self.decide(pid, decision),
            Do::AddMemory(title) => self.work("memory-added", move |l| l.add_memory(&id, &title)),
            Do::DeleteMemory(mid, title) => self.overlay = Some(Overlay::Confirm { text: format!("Forget \"{title}\"?"), then: Confirmed::DeleteMemory(mid) }),
            Do::LoadMemories => self.load_memories(),
            Do::LoadFiles => self.load_files(),
        }
    }

    /// Saves a long field edited in $EDITOR.
    pub fn edited(&mut self, field: &'static str, text: Option<String>) {
        match text {
            Some(t) => {
                let id = self.id.clone();
                let t = t.trim_end().to_string();
                self.work("patch", move |l| l.update_agent(&id, json!({ field: t })));
            }
            None => self.fail("The editor didn't save; nothing changed."),
        }
    }

    fn overlay_key(&mut self, k: KeyEvent) {
        let Some(o) = self.overlay.as_mut() else { return };
        match o {
            Overlay::Sessions { list, sel, .. } => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
                KeyCode::Up | KeyCode::Char('k') => *sel = sel.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1).min(list.len().saturating_sub(1)),
                KeyCode::Enter => {
                    let session = list.get(*sel).map(|s| str_of(s, "id").to_string()).unwrap_or_default();
                    self.overlay = None;
                    let id = self.id.clone();
                    self.work("resume", move |l| l.resume_session(&id, &session));
                }
                _ => {}
            },
            Overlay::Confirm { then, .. } => match k.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let then = then.clone();
                    self.overlay = None;
                    let id = self.id.clone();
                    match then {
                        Confirmed::ClearHistory => self.work("clear-history", move |l| l.clear_messages(&id)),
                        Confirmed::DeleteMemory(mid) => self.work("memory-deleted", move |l| l.delete_memory(&id, mid)),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => self.overlay = None,
                _ => {}
            },
        }
    }
}

/// Hands a link (a VS Code Remote-SSH URL) to this machine's opener.
fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}
