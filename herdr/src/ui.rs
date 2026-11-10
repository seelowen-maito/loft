//! Draws the chat: header, transcript, prompts, composer, sidebar and overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use serde_json::Value;

use crate::app::{App, Overlay};
use crate::transcript::{self, AGENT, BAD, DIM, MARK, USER, WARN, clip, input_summary, wrap};
use crate::util;

const SPIN: [&str; 4] = ["◐", "◓", "◑", "◒"];

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let (chat, side) = if app.sidebar {
        if area.width >= 90 {
            let sw = (area.width * 2 / 5).clamp(36, 56);
            let [c, s] = Layout::horizontal([Constraint::Min(40), Constraint::Length(sw)]).areas(area);
            (Some(c), Some(s))
        } else {
            (None, Some(area))
        }
    } else {
        (Some(area), None)
    };
    if let Some(c) = chat {
        draw_chat(f, app, c);
    }
    if let Some(s) = side {
        let (agent, perms, focused) = (app.agent.clone(), app.perms.clone(), app.sidebar_focus);
        app.inspector.draw(f, s, &agent, &perms, focused);
    }
    if app.overlay.is_some() {
        draw_overlay(f, app, area);
    }
}

fn draw_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let w = area.width as usize;
    let name = app.name();

    // Composer rows first: they decide how much room the transcript gets.
    let inner_w = area.width.saturating_sub(4) as usize;
    let (rows, cursor) = app.composer.layout(inner_w);
    let composer_h = (rows.len().clamp(1, 8) + 2) as u16;
    let menu = app.menu();
    let menu_h = if menu.is_empty() { 0 } else { (menu.len() * 2).min(10) as u16 + 2 };
    let perm_lines = permission_lines(app, w);
    let grill_lines = grill_lines(app, w);
    let max_extra = area.height / 2;
    let perm_h = (perm_lines.len() as u16).min(max_extra);
    let grill_h = (grill_lines.len() as u16).min(max_extra.saturating_sub(perm_h));

    let [header, body, perm, grill, menu_area, comp, foot] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(perm_h),
        Constraint::Length(grill_h),
        Constraint::Length(menu_h),
        Constraint::Length(composer_h),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header, &name);

    // Transcript, scrolled from the bottom.
    let mut lines = transcript::lines(&app.messages, &app.perms, &name, area.width.saturating_sub(1), &app.view);
    if app.running() {
        lines.push(Line::default());
        lines.push(Line::styled(format!("  {} {}", SPIN[(app.tick as usize / 2) % 4], working_text(app)), Style::new().fg(AGENT)));
    } else if !app.loaded {
        lines.push(Line::styled("  Loading…", Style::new().fg(DIM)));
    } else if app.messages.is_empty() {
        lines.push(Line::styled(format!("  No messages yet. Say hi to {name}."), Style::new().fg(DIM)));
    }
    let h = body.height as usize;
    app.max_scroll = lines.len().saturating_sub(h);
    app.scroll = app.scroll.min(app.max_scroll);
    let end = lines.len() - app.scroll;
    let start = end.saturating_sub(h);
    let shown: Vec<Line> = lines[start..end].to_vec();
    f.render_widget(Paragraph::new(shown), body);
    if app.scroll > 0 {
        let tag = format!(" ↓ {} more · PgDn ", app.scroll);
        let r = Rect { x: body.x + body.width.saturating_sub(tag.len() as u16 + 1), y: body.y + body.height - 1, width: tag.len() as u16, height: 1 };
        f.render_widget(Paragraph::new(tag).style(Style::new().fg(Color::Black).bg(Color::Gray)), r);
    }

    f.render_widget(Paragraph::new(perm_lines), perm);
    f.render_widget(Paragraph::new(grill_lines), grill);

    if !menu.is_empty() {
        let sel = app.menu_sel.min(menu.len() - 1);
        let mut ml = Vec::new();
        for (i, c) in menu.iter().enumerate() {
            let on = i == sel;
            let style = if on { Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::new() };
            ml.push(Line::styled(format!("{}{}", if on { "▸ " } else { "  " }, c.name), style));
            ml.push(Line::styled(format!("    {}", clip(c.description, w.saturating_sub(8))), Style::new().fg(DIM)));
        }
        f.render_widget(Paragraph::new(ml).block(Block::default().borders(Borders::ALL).border_style(Style::new().fg(DIM)).title(" Commands ")), menu_area);
    }

    // Composer.
    let focused = !(app.sidebar && app.sidebar_focus) && app.overlay.is_none();
    let border = if focused { Color::Cyan } else { DIM };
    let block = Block::default().borders(Borders::ALL).border_style(Style::new().fg(border));
    let visible = (composer_h - 2) as usize;
    let first = cursor.0.saturating_sub(visible.saturating_sub(1));
    let text: Vec<Line> = if app.composer.is_empty() {
        let hint = if app.deleted {
            "This agent was deleted.".to_string()
        } else if app.grill_round().is_some() {
            "Answer the selected question… (empty enter accepts the recommendation)".to_string()
        } else {
            format!("Message {name}… (/ for commands)")
        };
        vec![Line::styled(clip(&hint, inner_w), Style::new().fg(DIM))]
    } else {
        rows.iter().skip(first).take(visible).map(|r| Line::raw(format!("{r}"))).collect()
    };
    let inner = block.inner(comp);
    f.render_widget(block, comp);
    f.render_widget(Paragraph::new(text), Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner });
    if focused {
        f.set_cursor_position(Position { x: inner.x + 1 + cursor.1 as u16, y: inner.y + (cursor.0 - first) as u16 });
    }

    // Footer: a note, or what the keys do.
    let footer = match &app.note {
        Some(n) => Line::styled(clip(&format!(" {}", n.text), w), Style::new().fg(if n.error { BAD } else { USER })),
        None => {
            let mut hints = vec!["enter send", "alt+enter newline", "/ commands", "ctrl+s sidebar"];
            if app.running() {
                hints.push("esc stop");
            }
            if app.scroll == 0 && app.max_scroll > 0 {
                hints.push("pgup scroll");
            }
            hints.push("ctrl+t thinking · ctrl+o tools");
            Line::styled(clip(&format!(" {}", hints.join(" · ")), w), Style::new().fg(DIM))
        }
    };
    f.render_widget(Paragraph::new(footer), foot);
}

fn working_text(app: &App) -> String {
    let since = app
        .messages
        .iter()
        .rev()
        .find(|m| str_of(m, "kind") == "run_start")
        .and_then(|m| util::epoch(str_of(m, "created_at")));
    let mut t = "working".to_string();
    if let Some(s) = since {
        t.push_str(&format!(" · {}", util::span(util::now() - s)));
    }
    let status = str_of(&app.agent, "status_text");
    if !status.is_empty() {
        t.push_str(&format!(" · {status}"));
    }
    if let Some(subs) = app.agent.get("subagents").and_then(|s| s.as_array()).filter(|s| !s.is_empty()) {
        t.push_str(&format!(" · {} subagent{}", subs.len(), if subs.len() == 1 { "" } else { "s" }));
    }
    let queued = app.agent.get("queued_messages").and_then(|q| q.as_array()).map(|q| q.len()).unwrap_or(0);
    if queued > 0 {
        t.push_str(&format!(" · {queued} queued"));
    }
    t.push_str(" · esc to stop");
    t
}

fn draw_header(f: &mut Frame, app: &App, area: Rect, name: &str) {
    let a = &app.agent;
    let status = str_of(a, "status");
    let (dot, color, label) = if app.pending().is_some() {
        ("●", WARN, "needs you".to_string())
    } else if app.running() {
        (SPIN[(app.tick as usize / 2) % 4], AGENT, "working".to_string())
    } else if status == "error" {
        ("●", BAD, "error".to_string())
    } else if status == "waiting" {
        ("●", WARN, "waiting".to_string())
    } else {
        ("○", DIM, "idle".to_string())
    };
    let mut first = vec![
        Span::styled(format!(" {dot} "), Style::new().fg(color)),
        Span::styled(name.to_string(), Style::new().add_modifier(Modifier::BOLD)),
    ];
    let title = str_of(a, "title");
    if !title.is_empty() {
        first.push(Span::styled(format!("  {title}"), Style::new().fg(DIM)));
    }
    let role = str_of(a, "role");
    if !role.is_empty() && role != "custom" {
        first.push(Span::styled(format!("  [{}]", role.replace('_', " ")), Style::new().fg(MARK)));
    }
    first.push(Span::styled(format!("  {label}"), Style::new().fg(color)));

    let mut second: Vec<String> = Vec::new();
    if let Some(m) = a.get("model").and_then(|m| m.as_str()).filter(|m| !m.is_empty()) {
        second.push(m.to_string());
    }
    if let Some(t) = a.get("context_tokens").and_then(|t| t.as_f64()).filter(|t| *t > 0.0) {
        second.push(format!("{} context", util::tokens(t)));
    }
    if let Some(n) = a.get("next_run_at").and_then(|n| n.as_str()).and_then(util::epoch) {
        let d = n - util::now();
        if d > 0 {
            second.push(format!("next run in {}", util::span(d)));
        }
    }
    if app.view.earlier {
        second.push("showing earlier messages".into());
    }
    let mut line2 = vec![Span::styled(format!("   {}", second.join(" · ")), Style::new().fg(DIM))];
    if let Some(e) = &app.offline {
        line2.push(Span::styled(format!("  offline: {}", clip(e, 60)), Style::new().fg(BAD)));
    }
    let _ = Line::default();
    f.render_widget(Paragraph::new(vec![Line::from(first), Line::from(line2)]), area);
}

fn permission_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let Some(p) = app.pending() else { return vec![] };
    let tool = str_of(p, "tool_name").to_string();
    let mut out = vec![Line::styled(clip(&format!(" ⚠ {} wants to use {tool}", app.name()), w), Style::new().fg(WARN).add_modifier(Modifier::BOLD))];
    let reason = str_of(p, "reason");
    for l in wrap(&input_summary(p.get("input")), w.saturating_sub(4)).into_iter().take(6) {
        out.push(Line::styled(format!("   {l}"), Style::new()));
    }
    if !reason.is_empty() {
        out.push(Line::styled(clip(&format!("   {reason}"), w), Style::new().fg(DIM)));
    }
    let more = app.perms.values().filter(|p| str_of(p, "status") == "pending").count().saturating_sub(1);
    let tail = if more > 0 { format!("  (+{more} more)") } else { String::new() };
    out.push(Line::styled(format!("   y allow once · a always allow · n deny{tail}"), Style::new().fg(WARN)));
    out
}

fn grill_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let Some(g) = app.grill.as_ref() else { return vec![] };
    let state = str_of(g, "state");
    let topic = str_of(g, "topic");
    let head = Style::new().fg(MARK).add_modifier(Modifier::BOLD);
    if state == "preparing" || g.get("waiting").and_then(|x| x.as_bool()) == Some(true) {
        let what = if state == "preparing" { "preparing the first questions" } else { "working on the next round" };
        return vec![Line::styled(clip(&format!(" ✦ Grill: {topic} · {} is {what}", app.name()), w), head)];
    }
    if state == "paused" {
        return vec![Line::styled(clip(&format!(" ✦ Grill: {topic} · paused (resume it on the web)"), w), head)];
    }
    let Some(qs) = app.grill_round() else { return vec![] };
    let round = g.get("round").and_then(|r| r.as_i64()).unwrap_or(1);
    let answered = qs.iter().filter(|q| str_of(q, "state") != "open").count();
    let mut out = vec![Line::styled(clip(&format!(" ✦ Grill: {topic} · round {round} · {answered}/{} answered", qs.len()), w), head)];
    let sel = app.grill_sel.min(qs.len().saturating_sub(1));
    for (i, q) in qs.iter().enumerate() {
        let st = str_of(q, "state");
        let glyph = match st {
            "open" => "○",
            "decided" => "◆",
            "discuss" => "?",
            _ => "●",
        };
        let on = i == sel;
        let style = if on { Style::new().add_modifier(Modifier::BOLD) } else if st == "open" { Style::new() } else { Style::new().fg(DIM) };
        if on {
            for (j, l) in wrap(str_of(q, "text"), w.saturating_sub(6)).into_iter().enumerate() {
                out.push(Line::styled(format!(" {} {l}", if j == 0 { format!("▸{glyph}") } else { "  ".into() }), style));
            }
            let rec = str_of(q, "recommendation");
            if !rec.is_empty() {
                for l in wrap(&format!("Recommendation: {rec}"), w.saturating_sub(6)).into_iter().take(4) {
                    out.push(Line::styled(format!("    {l}"), Style::new().fg(AGENT)));
                }
            }
            if let Some(a) = q.get("answer").and_then(|a| a.as_str()) {
                out.push(Line::styled(clip(&format!("    Your answer: {a}"), w), Style::new().fg(USER)));
            }
        } else {
            out.push(Line::styled(clip(&format!("  {glyph} {}", str_of(q, "text")), w), style));
        }
    }
    let ready = if answered == qs.len() { " · ctrl+g send the round" } else { "" };
    out.push(Line::styled(clip(&format!("   ↑↓ question · enter answer · empty enter accepts · alt+d leave to agent{ready}"), w), Style::new().fg(DIM)));
    out
}

fn draw_overlay(f: &mut Frame, app: &App, area: Rect) {
    let w = area.width.saturating_sub(8).min(90);
    let (title, lines): (&str, Vec<Line>) = match app.overlay.as_ref().unwrap() {
        Overlay::Sessions { list, current, sel } => {
            let mut l = vec![Line::styled(" ↑↓ choose · enter resume · esc cancel", Style::new().fg(DIM)), Line::default()];
            for (i, s) in list.iter().enumerate() {
                let on = i == *sel;
                let id = str_of(s, "id");
                let cur = current.as_deref() == Some(id);
                let when = util::ago(str_of(s, "started_at"));
                let from = s.get("from_agent").and_then(|f| f.as_str()).map(|f| format!("{f}: ")).unwrap_or_default();
                let tokens = s.get("context_tokens").and_then(|t| t.as_f64()).map(|t| format!(" · {}", util::tokens(t))).unwrap_or_default();
                let head = format!("{}{when}{tokens}{}", if on { "▸ " } else { "  " }, if cur { " · current" } else { "" });
                l.push(Line::styled(head, if on { Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::new().fg(DIM) }));
                l.push(Line::raw(format!("    {}", clip(&format!("{from}{}", str_of(s, "first_message").replace('\n', " ")), w as usize - 6))));
            }
            (" Resume a session ", l)
        }
        Overlay::Confirm { text, .. } => {
            let mut l: Vec<Line> = wrap(text, w as usize - 4).into_iter().map(|t| Line::raw(format!(" {t}"))).collect();
            l.push(Line::default());
            l.push(Line::styled(" y yes · n no", Style::new().fg(WARN)));
            (" Are you sure? ", l)
        }
    };
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height.saturating_sub(h)) / 2, width: w, height: h };
    // Keep the selected session in view.
    let skip = match app.overlay.as_ref().unwrap() {
        Overlay::Sessions { sel, .. } => (2 + sel * 2 + 2).saturating_sub(h as usize - 2),
        _ => 0,
    };
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()).block(Block::default().borders(Borders::ALL).border_style(Style::new().fg(Color::Cyan)).title(title)),
        r,
    );
}
