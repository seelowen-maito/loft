//! loft-herdr: Loft's agents as agents in Herdr UI.
//!
//!   loft-herdr chat <agent-id>   one agent's chat (what each Herdr pane runs)
//!   loft-herdr sync              open a tab for every Loft agent that has none
//!   loft-herdr setup <url>       remember where Loft is
//!   loft-herdr agents            list Loft's agents

mod app;
mod commands;
mod composer;
mod config;
mod herdr;
mod inspector;
mod loft;
mod sync;
mod transcript;
mod ui;
mod util;

use std::io::stdout;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use crossterm::event::{self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event, KeyEventKind, MouseEventKind};
use crossterm::execute;

use app::{App, Msg};
use loft::{Live, Loft};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("chat") | None => chat(args.get(1).cloned()),
        Some("sync") => run_sync(),
        Some("setup") => setup(args.get(1)),
        Some("agents") => list(),
        Some("--version" | "version") => {
            println!("loft-herdr {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown command {other}. Use: chat <agent-id> | sync | setup <url> | agents")),
    };
    if let Err(e) = r {
        eprintln!("loft-herdr: {e}");
        std::process::exit(1);
    }
}

fn loft() -> Loft {
    Loft::new(&config::url())
}

fn run_sync() -> Result<()> {
    println!("{}", sync::sync(&loft(), &herdr::Herdr::connect())?);
    Ok(())
}

fn setup(url: Option<&String>) -> Result<()> {
    let url = config::normalize(url.ok_or_else(|| anyhow!("usage: loft-herdr setup <loft-url>"))?);
    config::save(&url)?;
    match Loft::new(&url).agents() {
        Ok(a) => println!("Loft at {url}: {} agents.", a.len()),
        Err(e) => println!("Saved {url}, but Loft didn't answer: {e}"),
    }
    Ok(())
}

fn list() -> Result<()> {
    for a in loft().agents()? {
        let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        println!("{}  {}  {}", s("id"), s("name"), s("status"));
    }
    Ok(())
}

fn chat(id: Option<String>) -> Result<()> {
    let id = id.or_else(|| std::env::var("LOFT_AGENT_ID").ok()).filter(|i| !i.is_empty()).ok_or_else(|| anyhow!("which agent? usage: loft-herdr chat <agent-id> (see loft-herdr agents)"))?;
    let loft = loft();
    let (tx, rx) = mpsc::channel::<Msg>();
    {
        let tx = tx.clone();
        loft.stream(move |l| tx.send(Msg::Live(l)).is_ok());
    }
    let reporter = herdr::Herdr::pane_id().map(|pane| herdr::reporter(pane, id.clone()));
    let mut app = App::new(loft.clone(), id, tx, reporter.as_ref().map(|r| r.0.clone()));
    app.reload();

    let mut term = start_terminal();
    // Agent ids Loft had when the chat opened; an update for one not in it is a new agent.
    let mut known: std::collections::HashSet<String> =
        loft.agents().unwrap_or_default().iter().filter_map(|a| a.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect();
    let result = (|| -> Result<()> {
        loop {
            term.draw(|f| ui::draw(f, &mut app))?;
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
                    Event::Paste(p) => app.paste(&p),
                    Event::Mouse(m) => match m.kind {
                        MouseEventKind::ScrollUp => app.scroll_by(3, true),
                        MouseEventKind::ScrollDown => app.scroll_by(3, false),
                        _ => {}
                    },
                    Event::FocusGained => app.on_focus(true),
                    Event::FocusLost => app.on_focus(false),
                    _ => {}
                }
            }
            while let Ok(m) = rx.try_recv() {
                // A new agent in Loft: open its tab too (one pane wins the sync lock).
                if let Msg::Live(Live::Event(e)) = &m
                    && e.get("type").and_then(|t| t.as_str()) == Some("agent_updated")
                    && let Some(new) = e.pointer("/agent/id").and_then(|i| i.as_str())
                    && !known.is_empty()
                    && known.insert(new.to_string())
                    && herdr::Herdr::pane_id().is_some()
                {
                    let loft = loft.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(fastrand_delay()));
                        let _ = sync::sync(&loft, &herdr::Herdr::connect());
                    });
                }
                app.on_msg(m);
            }
            app.tick += 1;
            if let Some(field) = app.editor.take() {
                let current = app.agent.get(field).and_then(|v| v.as_str()).unwrap_or("").to_string();
                stop_terminal();
                let edited = edit(&current);
                term = start_terminal();
                term.clear()?;
                match edited {
                    Some(t) if t.trim_end() == current.trim_end() => {}
                    other => app.edited(field, other),
                }
            }
            if app.quit {
                return Ok(());
            }
        }
    })();
    stop_terminal();
    if let Some((tx, handle)) = reporter {
        let _ = tx.send(herdr::Report::Release);
        let _ = handle.join();
    }
    result
}

/// Spreads the panes' syncs out a little so one takes the lock cleanly.
fn fastrand_delay() -> u64 {
    (std::process::id() as u64 * 37 + herdr::now_ms()) % 400
}

fn start_terminal() -> ratatui::DefaultTerminal {
    let t = ratatui::init();
    let _ = execute!(stdout(), EnableBracketedPaste, EnableFocusChange, EnableMouseCapture);
    t
}

fn stop_terminal() {
    let _ = execute!(stdout(), DisableBracketedPaste, DisableFocusChange, DisableMouseCapture);
    ratatui::restore();
}

/// Edits text in $VISUAL / $EDITOR (vi by default); None when the editor failed.
fn edit(text: &str) -> Option<String> {
    let path = std::env::temp_dir().join(format!("loft-herdr-{}.md", std::process::id()));
    std::fs::write(&path, text).ok()?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
    let mut parts = editor.split_whitespace();
    let status = std::process::Command::new(parts.next()?).args(parts).arg(&path).status().ok()?;
    let out = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    if status.success() { out } else { None }
}
