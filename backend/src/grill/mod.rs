//! "Grill me": a guided interview inside the normal chat. The agent plans questions with the `grill_*`
//! MCP tools; Loft keeps them in a queue and posts the next one the moment the user answers, so
//! the interview doesn't wait for a new agent run between questions. Questions, answers, status rows
//! and the summary are ordinary chat messages tagged with `meta.grill`, so they render inline in the
//! same conversation (and as plain chat on clients that don't know the tag).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agents::add_msg;
use crate::api::{ApiError, not_found};
use crate::db::{Agent, Message, now};
use crate::state::{AppState, Event};

pub mod commands;

/// Most questions one session may hold, follow-ups included.
pub const MAX_QUESTIONS: usize = 40;
const KINDS: &[&str] = &["free_text", "single_choice", "multi_choice", "yes_no", "number"];
const MAX_TEXT: usize = 500;
const MAX_SPEAK: usize = 300;
const MAX_OPTIONS: usize = 6;
const MAX_OPTION: usize = 60;
const MAX_ANSWER: usize = 4000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub topic: String,
    #[serde(default)]
    pub goal: String,
    /// preparing | active | paused | ended
    pub status: String,
    /// done | stopped | timeout | superseded
    #[serde(default)]
    pub end_reason: Option<String>,
    #[serde(default)]
    pub questions: Vec<Question>,
    /// The question waiting for an answer, if any.
    #[serde(default)]
    pub current: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    /// Where the notes file was written, relative to the agent's workspace.
    #[serde(default)]
    pub notes_path: Option<String>,
    /// en | pt: the language the user started in, for the status rows.
    #[serde(default)]
    pub lang: String,
    /// The agent has posted the final summary.
    #[serde(default)]
    pub summarized: bool,
    /// `client_id`s of recent commands, so a retried request isn't applied twice.
    #[serde(default)]
    pub seen_commands: Vec<String>,
    /// The last command matched from untagged text, which the user may undo for a few seconds.
    #[serde(default)]
    pub undo: Option<Undo>,
}

/// State before a command the backend inferred from text, to put back if the user says it was an answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Undo {
    pub message_id: i64,
    pub text: String,
    pub command: String,
    pub at: String,
    pub status: String,
    pub current: Option<String>,
    pub questions: Vec<Question>,
}

/// How long "Treated as Skip · Undo" stays available.
const UNDO_SECS: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    /// 1-based position in the order asked; set when the question is posted.
    #[serde(default)]
    pub index: Option<u32>,
    pub text: String,
    /// Short plain version for reading aloud.
    #[serde(default)]
    pub speak: String,
    pub kind: String,
    #[serde(default)]
    pub options: Vec<String>,
    /// queued | asked | answered | skipped | delegated | withdrawn
    pub status: String,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub answer_message_id: Option<i64>,
    /// user | agent (a "decide for me" answer)
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub asked_at: Option<String>,
    #[serde(default)]
    pub answered_at: Option<String>,
}

/// Grill mode is on per agent. Unset means on for the Orchestrator only, for now.
pub fn enabled(a: &Agent) -> bool {
    a.grill.unwrap_or(a.name == "Orchestrator")
}

pub fn active(state: &AppState, agent_id: &str) -> Option<Session> {
    state.db.list_grills(agent_id).ok()?.into_iter().find(|s| s.status != "ended")
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

fn total(s: &Session) -> usize {
    s.questions.iter().filter(|q| q.status != "withdrawn").count()
}

fn current(s: &Session) -> Option<&Question> {
    s.current.as_deref().and_then(|id| s.questions.iter().find(|q| q.id == id))
}

/// The session as clients see it (`GET /agents/{id}/grill` and the `grill` event).
pub fn view(s: &Session) -> Value {
    let cur = current(s);
    json!({
        "session_id": s.id,
        "agent_id": s.agent_id,
        "state": s.status,
        "end_reason": s.end_reason,
        "topic": s.topic,
        "index": cur.and_then(|q| q.index),
        "total": if s.status == "preparing" { Value::Null } else { json!(total(s)) },
        "answered": s.questions.iter().filter(|q| q.status == "answered").count(),
        "current": cur.map(|q| json!({ "question_id": q.id, "index": q.index })),
        "started_at": s.started_at,
        "updated_at": s.updated_at,
        "notes_path": s.notes_path,
    })
}

fn save(state: &AppState, s: &mut Session) -> Result<(), String> {
    s.updated_at = now();
    state.db.save_grill(s).map_err(|e| e.to_string())?;
    state.emit(Event::Grill { session: view(s) });
    Ok(())
}

/// en or pt, by counting common words of each.
fn guess_lang(text: &str) -> &'static str {
    const PT: &[&str] = &["que", "nao", "voce", "uma", "um", "para", "como", "sobre", "do", "da", "os", "as", "e", "esta", "qual", "quem", "me", "de", "o", "com", "isso", "vai"];
    const EN: &[&str] = &["the", "what", "which", "who", "is", "are", "you", "how", "about", "should", "do", "of", "to", "a", "and", "it", "for", "with", "this", "will"];
    let n = commands::normalize(text);
    let words: Vec<&str> = n.split(' ').collect();
    let pt = words.iter().filter(|w| PT.contains(w)).count();
    let en = words.iter().filter(|w| EN.contains(w)).count();
    if pt > en { "pt" } else { "en" }
}

/// Status-row text in the session's language.
fn say(s: &Session, en: &str, pt: &str) -> String {
    if s.lang == "pt" { pt.to_string() } else { en.to_string() }
}

/// A small inline row in the chat ("Grill started: pricing", "Paused").
fn status_row(state: &AppState, s: &Session, text: &str, extra: Value) {
    let mut g = json!({ "session_id": s.id, "role": "status" });
    if let (Some(g), Some(extra)) = (g.as_object_mut(), extra.as_object()) {
        g.extend(extra.clone());
    }
    add_msg(state, &s.agent_id, None, "system", "notice", text, json!({ "grill": g }));
}

/// Posts the next queued question. False when the queue is empty.
fn ask_next(state: &AppState, s: &mut Session) -> bool {
    let asked = s.questions.iter().filter(|q| q.index.is_some()).count() as u32;
    let total = total(s);
    let Some(q) = s.questions.iter_mut().find(|q| q.status == "queued") else {
        s.current = None;
        return false;
    };
    q.status = "asked".into();
    q.index = Some(asked + 1);
    q.asked_at = Some(now());
    s.current = Some(q.id.clone());
    post_question(state, s, &s.questions.iter().find(|x| s.current.as_deref() == Some(&x.id)).unwrap().clone(), total, false);
    true
}

/// The chat message for a question. `repeat` marks a re-post (after "back", "repeat" or "resume").
fn post_question(state: &AppState, s: &Session, q: &Question, total: usize, repeat: bool) {
    let mut g = json!({
        "session_id": s.id, "role": "question", "question_id": q.id, "index": q.index,
        "total": total, "topic": s.topic, "kind": q.kind, "allow_free_text": true,
    });
    if !q.options.is_empty() {
        g["options"] = json!(q.options);
    }
    if !q.speak.is_empty() {
        g["speak"] = json!(q.speak);
    }
    if repeat {
        g["repeat"] = json!(true);
    }
    add_msg(state, &s.agent_id, None, "assistant", "text", &q.text, json!({ "grill": g }));
}

/// Validates the `questions` argument of `grill_start` / `grill_ask`.
fn parse_questions(v: &Value) -> Result<Vec<Question>, String> {
    let arr = v.as_array().ok_or("questions must be an array")?;
    arr.iter()
        .map(|q| {
            let s = |k: &str| q[k].as_str().unwrap_or("").trim().to_string();
            let text = s("text");
            if text.is_empty() {
                return Err("every question needs `text`".to_string());
            }
            if text.chars().count() > MAX_TEXT {
                return Err(format!("question text is limited to {MAX_TEXT} characters; split it up"));
            }
            let kind = if s("kind").is_empty() { "free_text".to_string() } else { s("kind") };
            if !KINDS.contains(&kind.as_str()) {
                return Err(format!("kind must be one of {}", KINDS.join(" | ")));
            }
            let options: Vec<String> = q["options"]
                .as_array()
                .map(|o| o.iter().filter_map(|x| x.as_str()).map(|x| clip(x, MAX_OPTION)).filter(|x| !x.is_empty()).collect())
                .unwrap_or_default();
            if options.len() > MAX_OPTIONS {
                return Err(format!("at most {MAX_OPTIONS} options per question"));
            }
            if kind.ends_with("_choice") && options.is_empty() {
                return Err(format!("a {kind} question needs `options`"));
            }
            Ok(Question {
                id: uuid::Uuid::new_v4().to_string(),
                index: None,
                text,
                speak: clip(&s("speak"), MAX_SPEAK),
                kind,
                options,
                status: "queued".into(),
                answer: None,
                answer_message_id: None,
                source: None,
                asked_at: None,
                answered_at: None,
            })
        })
        .collect()
}

fn new_session(agent_id: &str, topic: &str, status: &str) -> Session {
    Session {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: agent_id.into(),
        topic: clip(topic, 120),
        goal: String::new(),
        status: status.into(),
        end_reason: None,
        questions: vec![],
        current: None,
        created_at: now(),
        updated_at: now(),
        started_at: None,
        ended_at: None,
        notes_path: None,
        lang: "en".into(),
        summarized: false,
        seen_commands: vec![],
        undo: None,
    }
}

fn finish(state: &AppState, s: &mut Session, reason: &str) -> Result<(), String> {
    finish_with(state, s, reason, json!({}))
}

fn finish_with(state: &AppState, s: &mut Session, reason: &str, extra: Value) -> Result<(), String> {
    s.status = "ended".into();
    s.end_reason = Some(reason.into());
    s.ended_at = Some(now());
    s.current = None;
    for q in s.questions.iter_mut().filter(|q| q.status == "queued" || q.status == "asked") {
        q.status = "withdrawn".into();
    }
    s.undo = None;
    let mut meta = json!({ "state": "ended", "end_reason": reason });
    if let (Some(m), Some(extra)) = (meta.as_object_mut(), extra.as_object()) {
        m.extend(extra.clone());
    }
    let text = if reason == "superseded" {
        say(s, "Grill ended: a new one was requested", "Entrevista encerrada: uma nova foi pedida")
    } else {
        say(s, "Grill ended", "Entrevista encerrada")
    };
    status_row(state, s, &text, meta);
    save(state, s)
}

fn queue_empty_prompt(s: &Session) -> String {
    format!(
        "[Grill {} on \"{}\": every queued question is answered]\n\n{}\n\nAsk follow-ups with `mcp__loft__grill_ask`, or finish with `mcp__loft__grill_end` and a summary in the user's language.",
        s.id,
        s.topic,
        transcript(s)
    )
}

fn internal(e: String) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e)
}

/// The Q&A so far, for prompts and `grill_state`.
fn transcript(s: &Session) -> String {
    let mut out = vec![];
    for q in s.questions.iter().filter(|q| q.index.is_some()) {
        let a = match q.status.as_str() {
            "answered" => format!("A ({}): {}", q.source.as_deref().unwrap_or("user"), q.answer.as_deref().unwrap_or("")),
            other => format!("({other})"),
        };
        out.push(format!("Q{} [{}]: {}\n{a}", q.index.unwrap_or(0), q.id, q.text));
    }
    let queued: Vec<String> = s.questions.iter().filter(|q| q.status == "queued").map(|q| format!("- [{}] {}", q.id, q.text)).collect();
    if !queued.is_empty() {
        out.push(format!("Still queued:\n{}", queued.join("\n")));
    }
    out.join("\n\n")
}

// ---- user side --------------------------------------------------------------

/// A prompt to run the agent with in the background. Grill runs are quiet: only errors reach the chat.
pub struct Wake {
    pub prompt: String,
    /// "grill" (the agent's own model) or "grill_revise" (the cheaper revision model)
    pub trigger: &'static str,
}

fn quiet(prompt: String) -> Wake {
    Wake { prompt, trigger: "grill" }
}

/// After an answer: let the agent adjust the queue, on the cheaper model, while the user is already on
/// the next question.
fn revise_prompt(s: &Session, q: &Question) -> String {
    let queued: Vec<String> = s.questions.iter().filter(|q| q.status == "queued").map(|q| format!("- [{}] {}", q.id, q.text)).collect();
    format!(
        "[Grill {} on \"{}\": question {} answered]\nQ{}: {}\nA: {}\n\nOn screen now: {}\nStill queued:\n{}\n\nIf this answer makes a queued question pointless, call `mcp__loft__grill_withdraw`; if it calls for a follow-up, call `mcp__loft__grill_ask` with `position: next`. Otherwise do nothing. Don't write a reply: this run isn't shown to the user.",
        s.id,
        s.topic,
        q.index.unwrap_or(0),
        q.index.unwrap_or(0),
        q.text,
        q.answer.as_deref().unwrap_or(""),
        current(s).map(|c| c.text.as_str()).unwrap_or("(nothing)"),
        if queued.is_empty() { "(nothing)".to_string() } else { queued.join("\n") }
    )
}

/// What `send_message` should do with a user message, given its optional `meta.grill`.
pub enum Routed {
    /// Not part of a grill: deliver as usual, with this extra meta.
    Normal(Value),
    /// Stored by the grill; wake the agent with this prompt if there is one.
    Handled { message: Option<Message>, wake: Option<Wake> },
}

/// Routes a message the user sent to `agent`. Only the user's own messages reach this; messages from
/// other agents, task briefs and wake-ups never count as answers.
pub fn on_user_message(state: &AppState, agent: &Agent, text: &str, grill: Option<&Value>, mut meta: Value) -> Result<Routed, ApiError> {
    let role = grill.and_then(|g| g["role"].as_str()).unwrap_or("");
    let conflict = |e: &str| ApiError(StatusCode::CONFLICT, e.into());

    // A new request typed while an interview is open (web, old app builds) replaces it rather than
    // being taken as an answer to the open question.
    let typed_start = role.is_empty() && enabled(agent) && active(state, &agent.id).is_some() && commands::is_start_request(text);
    if role == "start_request" || typed_start {
        if !enabled(agent) {
            return Ok(Routed::Normal(meta));
        }
        let mut s = match active(state, &agent.id) {
            Some(s) if s.status == "preparing" => s,
            Some(mut old) => {
                finish(state, &mut old, "superseded").map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
                new_session(&agent.id, text, "preparing")
            }
            None => new_session(&agent.id, text, "preparing"),
        };
        save(state, &mut s).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        meta["grill"] = json!({ "session_id": s.id, "role": "start_request" });
        let message = add_msg(state, &agent.id, None, "user", "text", text, meta);
        let wake = format!(
            "{text}\n\n[Grill mode: the user wants to be interviewed. Call `mcp__loft__grill_start` with the topic and a first batch of 3-8 questions, in the language the user wrote in.]"
        );
        return Ok(Routed::Handled { message, wake: Some(quiet(wake)) });
    }

    if !role.is_empty() && role != "answer" {
        return Err(ApiError(StatusCode::BAD_REQUEST, format!("unknown grill role {role}")));
    }

    let tagged = role == "answer";
    let Some(mut s) = active(state, &agent.id) else {
        return if tagged { Err(conflict("grill_closed")) } else { Ok(Routed::Normal(meta)) };
    };
    if tagged && grill.and_then(|g| g["session_id"].as_str()).is_some_and(|id| id != s.id) {
        return Err(conflict("grill_closed"));
    }
    if s.status == "preparing" {
        // No questions yet: tagged answers can't belong anywhere, and plain text is context for the agent.
        return if tagged { Err(conflict("question_closed")) } else { Ok(Routed::Normal(meta)) };
    }
    s.undo = None;

    // Untagged text (web, old app builds) may be a command; app-tagged answers never are.
    let mut text = text.to_string();
    if !tagged {
        match commands::parse(&text) {
            commands::Parsed::Command(cmd) => {
                meta["grill"] = json!({ "session_id": s.id, "role": "command", "command": cmd });
                let message = add_msg(state, &agent.id, None, "user", "text", &text, meta);
                let before = s.clone();
                let wake = apply_command(state, &mut s, cmd, message.as_ref().map(|m| m.id))?.map(quiet);
                if let Some(m) = &message {
                    if cmd != "end" && s.status != "ended" {
                        s.undo = Some(Undo {
                            message_id: m.id,
                            text: text.clone(),
                            command: cmd.into(),
                            at: now(),
                            status: before.status,
                            current: before.current,
                            questions: before.questions,
                        });
                        state.db.save_grill(&s).map_err(|e| internal(e.to_string()))?;
                    }
                }
                return Ok(Routed::Handled { message, wake });
            }
            commands::Parsed::Answer(a) => text = a,
        }
    }

    let qid = match grill.and_then(|g| g["question_id"].as_str()) {
        Some(id) => id.to_string(),
        None if tagged => return Err(ApiError(StatusCode::BAD_REQUEST, "an answer needs question_id".into())),
        // Untagged text while a question waits (or the grill is paused on one) is its answer.
        None => match &s.current {
            Some(id) => id.clone(),
            None => return Ok(Routed::Normal(meta)),
        },
    };
    let (message, wake) = record_answer(state, &mut s, &qid, &text, meta)?;
    Ok(Routed::Handled { message, wake })
}

/// Stores `text` as the user's answer to `qid`, then asks the next question.
fn record_answer(state: &AppState, s: &mut Session, qid: &str, text: &str, mut meta: Value) -> Result<(Option<Message>, Option<Wake>), ApiError> {
    let conflict = |e: &str| ApiError(StatusCode::CONFLICT, e.into());
    let Some(q) = s.questions.iter_mut().find(|q| q.id == qid) else {
        return Err(conflict("question_closed"));
    };
    if q.status != "asked" {
        return Err(conflict("question_closed"));
    }
    meta["grill"] = json!({ "session_id": s.id, "role": "answer", "question_id": q.id, "index": q.index });
    let answer = clip(text, MAX_ANSWER);
    let message = add_msg(state, &s.agent_id, None, "user", "text", &answer, meta);
    q.status = "answered".into();
    q.answer = Some(answer);
    q.answer_message_id = message.as_ref().map(|m| m.id);
    q.source = Some("user".into());
    q.answered_at = Some(now());
    if s.status == "paused" {
        s.status = "active".into();
        status_row(state, s, &say(s, "Resumed", "Retomada"), json!({ "state": "resumed" }));
    }
    let more = s.current.as_deref() != Some(qid) || ask_next(state, s);
    save(state, s).map_err(internal)?;
    let answered = s.questions.iter().find(|q| q.id == qid).cloned();
    let wake = if more {
        answered.map(|q| Wake { prompt: revise_prompt(s, &q), trigger: "grill_revise" })
    } else {
        Some(quiet(queue_empty_prompt(s)))
    };
    Ok((message, wake))
}

/// Applies a command to the running session and saves it. Returns a prompt to wake the agent with, if any.
fn apply_command(state: &AppState, s: &mut Session, cmd: &str, tagged_message_id: Option<i64>) -> Result<Option<String>, ApiError> {
    let row = |s: &Session, text: &str, st: &str, index: Option<u32>| {
        let mut extra = json!({ "state": st, "command": cmd, "index": index });
        if let Some(id) = tagged_message_id {
            extra["tagged_message_id"] = json!(id);
        }
        status_row(state, s, text, extra);
    };
    let cur = current(s).cloned();
    let need_current = || cur.clone().ok_or_else(|| ApiError(StatusCode::CONFLICT, "question_closed".into()));
    let mut wake = None;
    match cmd {
        "skip" | "decide" => {
            let q = need_current()?;
            let n = q.index.unwrap_or(0);
            if let Some(x) = s.questions.iter_mut().find(|x| x.id == q.id) {
                x.status = if cmd == "skip" { "skipped" } else { "delegated" }.into();
                x.answered_at = Some(now());
            }
            if s.status == "paused" {
                s.status = "active".into();
            }
            if cmd == "skip" {
                row(s, &say(s, &format!("Skipped question {n}"), &format!("Pergunta {n} pulada")), "command", Some(n));
            } else {
                row(s, &say(s, &format!("Question {n} left to the agent"), &format!("Pergunta {n} fica a critério do agente")), "command", Some(n));
                wake = Some(format!(
                    "[Grill {}: the user left question {n} to you]\nQ{n} [{}]: {}\n\nDecide it and call `mcp__loft__grill_record` with that question_id and your choice in one short line. The grill goes on meanwhile; don't post anything else.",
                    s.id, q.id, q.text
                ));
            }
            if !ask_next(state, s) {
                let empty = queue_empty_prompt(s);
                wake = Some(match wake {
                    Some(w) => format!("{w}\n\n{empty}"),
                    None => empty,
                });
            }
        }
        "back" => {
            let cur_index = cur.as_ref().and_then(|q| q.index).unwrap_or(u32::MAX);
            let prev = s
                .questions
                .iter()
                .filter(|q| q.index.is_some_and(|i| i < cur_index) && ["answered", "skipped", "delegated"].contains(&q.status.as_str()))
                .max_by_key(|q| q.index)
                .cloned()
                .ok_or_else(|| ApiError(StatusCode::CONFLICT, "no_previous_question".into()))?;
            // The question on screen goes back to the front of the queue; the previous one is asked again.
            if let Some(c) = &cur {
                let pos = s.questions.iter().position(|q| q.id == c.id).unwrap();
                let mut c = s.questions.remove(pos);
                c.status = "queued".into();
                c.index = None;
                c.asked_at = None;
                let at = s.questions.iter().position(|q| q.id == prev.id).unwrap() + 1;
                s.questions.insert(at, c);
            }
            if let Some(p) = s.questions.iter_mut().find(|q| q.id == prev.id) {
                p.status = "asked".into();
            }
            s.current = Some(prev.id.clone());
            if s.status == "paused" {
                s.status = "active".into();
            }
            let n = prev.index.unwrap_or(0);
            row(s, &say(s, &format!("Back to question {n}"), &format!("De volta à pergunta {n}")), "command", Some(n));
            post_question(state, s, &prev, total(s), true);
        }
        "repeat" => {
            let q = need_current()?;
            post_question(state, s, &q, total(s), true);
        }
        "pause" => {
            s.status = "paused".into();
            row(s, &say(s, "Paused", "Pausada"), "paused", cur.as_ref().and_then(|q| q.index));
        }
        "resume" => {
            let was_paused = s.status == "paused";
            s.status = "active".into();
            if was_paused {
                row(s, &say(s, "Resumed", "Retomada"), "resumed", cur.as_ref().and_then(|q| q.index));
            }
            if let Some(q) = &cur {
                post_question(state, s, q, total(s), true);
            }
        }
        "summarize" => {
            row(s, &say(s, "Summary requested", "Resumo pedido"), "command", cur.as_ref().and_then(|q| q.index));
            wake = Some(format!(
                "[Grill {} on \"{}\": the user asked for a summary so far]\n\n{}\n\nCall `mcp__loft__grill_summary` with a short summary in the user's language (decided, still open). The grill continues; don't post anything else.",
                s.id,
                s.topic,
                transcript(s)
            ));
        }
        "end" => {
            let mut extra = json!({ "command": "end" });
            if let Some(id) = tagged_message_id {
                extra["tagged_message_id"] = json!(id);
            }
            finish_with(state, s, "stopped", extra).map_err(internal)?;
            return Ok(Some(format!(
                "[Grill {} on \"{}\": the user stopped it]\n\n{}\n\nCall `mcp__loft__grill_end` with a summary of what was covered (the user's language), then reply with nothing.",
                s.id,
                s.topic,
                transcript(s)
            )));
        }
        other => return Err(ApiError(StatusCode::BAD_REQUEST, format!("unknown command {other}"))),
    }
    save(state, s).map_err(internal)?;
    Ok(wake)
}

// ---- HTTP ---------------------------------------------------------------------

/// `GET /api/agents/{id}/grill`: the agent's open session, or 404.
pub async fn get_active(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    active(&state, &id).map(|s| Json(view(&s))).ok_or_else(not_found)
}

/// `GET /api/grill/{id}`: one session with every question and answer.
pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(not_found());
    }
    let s = state.db.get_grill(&id)?.ok_or_else(not_found)?;
    let mut v = view(&s);
    v["questions"] = json!(s.questions.iter().filter(|q| q.index.is_some()).collect::<Vec<_>>());
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct CommandBody {
    command: String,
    question_id: Option<String>,
    client_id: Option<String>,
}

/// `POST /api/grill/{id}/command`: a button or a spoken command from the app. Answered in chat with a
/// status row; a retry with the same `client_id` isn't applied twice.
pub async fn command(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<CommandBody>) -> Result<Json<Value>, ApiError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(not_found());
    }
    let mut s = state.db.get_grill(&id)?.ok_or_else(not_found)?;
    let client_id = b.client_id.map(|c| c.trim().chars().take(64).collect::<String>()).filter(|c| !c.is_empty());
    if client_id.as_ref().is_some_and(|c| s.seen_commands.contains(c)) {
        return Ok(Json(json!({ "ok": true, "duplicate": true, "session": view(&s) })));
    }
    if s.status == "ended" || s.status == "preparing" {
        return Err(ApiError(StatusCode::CONFLICT, "grill_closed".into()));
    }
    let cmd = commands::COMMANDS.iter().find(|c| **c == b.command).copied().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, format!("unknown command {}", b.command)))?;
    if let Some(q) = &b.question_id {
        if s.current.as_deref() != Some(q) && ["skip", "decide", "back"].contains(&cmd) {
            return Err(ApiError(StatusCode::CONFLICT, "question_closed".into()));
        }
    }
    s.undo = None;
    if let Some(c) = client_id {
        s.seen_commands.push(c);
        let n = s.seen_commands.len();
        s.seen_commands.drain(..n.saturating_sub(50));
    }
    if let Some(p) = apply_command(&state, &mut s, cmd, None)? {
        crate::agents::wake(&state, &s.agent_id, p, "grill");
    }
    Ok(Json(json!({ "ok": true, "session": view(&s) })))
}

#[derive(Deserialize)]
pub struct UndoBody {
    message_id: i64,
}

/// `POST /api/grill/{id}/undo`: "that was my answer, not a command". Within a few seconds of a command
/// the backend inferred from text, puts the session back and records the text as the answer.
pub async fn undo(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<UndoBody>) -> Result<Json<Value>, ApiError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(not_found());
    }
    let mut s = state.db.get_grill(&id)?.ok_or_else(not_found)?;
    if let Some(p) = undo_command(&state, &mut s, b.message_id)? {
        crate::agents::wake(&state, &s.agent_id, p, "grill");
    }
    Ok(Json(json!({ "ok": true, "session": view(&s) })))
}

/// Puts the session back as it was before the command matched from message `message_id`, and records
/// that message as the answer. Returns a prompt to wake the agent with, if the queue ran out.
fn undo_command(state: &AppState, s: &mut Session, message_id: i64) -> Result<Option<String>, ApiError> {
    let b = UndoBody { message_id };
    let u = s.undo.take().filter(|u| u.message_id == b.message_id).ok_or_else(|| ApiError(StatusCode::CONFLICT, "nothing_to_undo".into()))?;
    let age = chrono::DateTime::parse_from_rfc3339(&u.at).map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds()).unwrap_or(i64::MAX);
    if age > UNDO_SECS || s.status == "ended" {
        return Err(ApiError(StatusCode::CONFLICT, "nothing_to_undo".into()));
    }
    let qid = u.current.clone().ok_or_else(|| ApiError(StatusCode::CONFLICT, "nothing_to_undo".into()))?;
    // The command may have put the next question on screen already; keep it instead of posting it twice.
    let on_screen = current(&s).cloned();
    s.status = if u.status == "paused" { "active".into() } else { u.status };
    s.current = u.current;
    s.questions = u.questions;
    let q = s.questions.iter_mut().find(|q| q.id == qid && q.status == "asked").ok_or_else(|| ApiError(StatusCode::CONFLICT, "nothing_to_undo".into()))?;
    q.status = "answered".into();
    q.answer = Some(clip(&u.text, MAX_ANSWER));
    q.answer_message_id = Some(u.message_id);
    q.source = Some("user".into());
    q.answered_at = Some(now());
    status_row(state, s, &say(s, "Treated as an answer", "Considerada uma resposta"), json!({ "state": "command", "command": "undo", "tagged_message_id": u.message_id, "question_id": qid }));
    let shown = on_screen.and_then(|o| s.questions.iter_mut().find(|q| q.id == o.id && q.status == "queued").map(|q| (q, o)));
    let more = match shown {
        Some((q, o)) => {
            q.status = "asked".into();
            q.index = o.index;
            q.asked_at = o.asked_at;
            s.current = Some(o.id);
            true
        }
        None => ask_next(state, s),
    };
    save(state, s).map_err(internal)?;
    Ok((!more).then(|| queue_empty_prompt(s)))
}

/// `GET /api/grill/commands`: phrase lists for on-device matching. Clients cache by `version` / ETag.
pub async fn command_catalog() -> impl axum::response::IntoResponse {
    let etag = format!("\"grill-commands-{}\"", commands::VERSION);
    ([(axum::http::header::ETAG, etag), (axum::http::header::CACHE_CONTROL, "max-age=3600".to_string())], Json(commands::catalog()))
}

// ---- notes and timeouts -----------------------------------------------------------

/// Writes the Q&A and the summary to `notes/grill/<date>-<topic>.md` in the agent's workspace. The path
/// is built here from the date and a `[a-z0-9-]` slug, never from text the agent or user supplied as-is.
fn write_notes(state: &AppState, s: &Session, summary: &str) -> Option<String> {
    let slug: String = commands::normalize(&s.topic).split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    let slug = if slug.is_empty() { "grill".to_string() } else { slug.trim_matches('-').to_string() };
    let dir = crate::agents::workspace(state, &s.agent_id).join("notes").join("grill");
    std::fs::create_dir_all(&dir).ok()?;
    let mut name = format!("{}-{slug}.md", &s.created_at[..10]);
    if dir.join(&name).exists() {
        name = format!("{}-{slug}-{}.md", &s.created_at[..10], &s.id[..8]);
    }
    let mut out = format!("# {}\n\n{} · {}\n", s.topic, &s.created_at[..10], say(s, "grill", "entrevista"));
    if !s.goal.is_empty() {
        out += &format!("\n{}\n", s.goal);
    }
    out += &format!("\n## {}\n\n{}\n\n## {}\n", say(s, "Summary", "Resumo"), summary.trim(), say(s, "Questions and answers", "Perguntas e respostas"));
    for q in s.questions.iter().filter(|q| q.index.is_some()) {
        let a = match q.status.as_str() {
            "answered" if q.source.as_deref() == Some("agent") => format!("**{}** {}", say(s, "Decided by the agent:", "Decidido pelo agente:"), q.answer.as_deref().unwrap_or("")),
            "answered" => q.answer.clone().unwrap_or_default(),
            "skipped" => format!("_{}_", say(s, "Skipped", "Pulada")),
            "delegated" => format!("_{}_", say(s, "Left to the agent, not decided", "Deixada para o agente, sem decisão")),
            _ => format!("_{}_", say(s, "Not answered", "Sem resposta")),
        };
        out += &format!("\n### {}. {}\n\n{a}\n", q.index.unwrap_or(0), q.text);
    }
    std::fs::write(dir.join(&name), out).ok()?;
    Some(format!("notes/grill/{name}"))
}

/// No answer for this long pauses a grill; this long paused ends it.
const PAUSE_AFTER_MINS: i64 = 30;
const END_AFTER_HOURS: i64 = 24;
/// A start request the agent never followed up on.
const PREPARING_FOR_MINS: i64 = 10;

/// Applies idle timeouts to every open grill. Returns the agents to wake with a summary request.
fn sweep(state: &AppState, now: chrono::DateTime<chrono::Utc>) -> Vec<(String, String)> {
    let mut wakes = vec![];
    for mut s in state.db.open_grills().unwrap_or_default() {
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&s.updated_at) else { continue };
        let idle = now.signed_duration_since(at);
        match s.status.as_str() {
            "preparing" if idle.num_minutes() >= PREPARING_FOR_MINS => {
                s.summarized = true;
                let _ = finish(state, &mut s, "timeout");
            }
            "active" if idle.num_minutes() >= PAUSE_AFTER_MINS => {
                s.status = "paused".into();
                status_row(state, &s, &say(&s, "Paused: no answer for 30 minutes", "Pausada: sem resposta há 30 minutos"), json!({ "state": "paused", "reason": "idle" }));
                let _ = save(state, &mut s);
            }
            "paused" if idle.num_hours() >= END_AFTER_HOURS => {
                let answered = s.questions.iter().any(|q| q.status == "answered");
                s.summarized = !answered;
                if finish(state, &mut s, "timeout").is_ok() && answered {
                    wakes.push((
                        s.agent_id.clone(),
                        format!(
                            "[Grill {} on \"{}\" ended after a day without answers]\n\n{}\n\nCall `mcp__loft__grill_end` with a summary of what was covered (the user's language), then reply with nothing.",
                            s.id,
                            s.topic,
                            transcript(&s)
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    wakes
}

/// Background task: checks open grills once a minute.
pub async fn sweeper(state: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        tick.tick().await;
        for (agent_id, prompt) in sweep(&state, chrono::Utc::now()) {
            crate::agents::wake(&state, &agent_id, prompt, "grill");
        }
    }
}

// ---- MCP tools ------------------------------------------------------------------

pub fn mcp_tools() -> Vec<Value> {
    let question = json!({ "type": "object", "properties": {
        "text": { "type": "string", "description": "The question as shown in chat (Markdown, one question, at most 500 characters)." },
        "speak": { "type": "string", "description": "A short plain version read aloud: no Markdown, links, ids or PR numbers; include the options. Always give one." },
        "kind": { "type": "string", "enum": KINDS },
        "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers (at most 6). Required for single_choice / multi_choice." }
    }, "required": ["text", "speak"] });
    vec![
        json!({
            "name": "grill_start",
            "description": "Start a grill (guided interview) in this chat. Loft posts the questions one at a time, as soon as the previous one is answered, so plan a first batch up front.",
            "inputSchema": { "type": "object", "properties": {
                "topic": { "type": "string", "description": "A few words, shown on the banner." },
                "goal": { "type": "string", "description": "What the interview should decide." },
                "language": { "type": "string", "enum": ["en", "pt"], "description": "The language the user wrote in; questions, status rows and the summary follow it." },
                "questions": { "type": "array", "items": question, "description": "First batch, in order: 3-8 questions." }
            }, "required": ["topic", "questions"] }
        }),
        json!({
            "name": "grill_ask",
            "description": "Add follow-up questions to the running grill. `position: next` asks them before the rest of the queue.",
            "inputSchema": { "type": "object", "properties": {
                "questions": { "type": "array", "items": question },
                "position": { "type": "string", "enum": ["next", "end"] }
            }, "required": ["questions"] }
        }),
        json!({
            "name": "grill_withdraw",
            "description": "Drop queued questions an earlier answer made pointless.",
            "inputSchema": { "type": "object", "properties": {
                "question_ids": { "type": "array", "items": { "type": "string" } }
            }, "required": ["question_ids"] }
        }),
        json!({
            "name": "grill_state",
            "description": "The running grill: every question asked so far with its answer, and the queue.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "grill_record",
            "description": "Record your decision for a question the user left to you (\"decide for me\"). One short line.",
            "inputSchema": { "type": "object", "properties": {
                "question_id": { "type": "string" },
                "answer": { "type": "string" }
            }, "required": ["question_id", "answer"] }
        }),
        json!({
            "name": "grill_summary",
            "description": "Post a short summary so far when the user asks for one. The grill continues.",
            "inputSchema": { "type": "object", "properties": {
                "summary": { "type": "string" }
            }, "required": ["summary"] }
        }),
        json!({
            "name": "grill_end",
            "description": "Finish the grill. `summary` (Markdown, the user's language) is posted in chat: decisions, open questions, what was skipped.",
            "inputSchema": { "type": "object", "properties": {
                "summary": { "type": "string" }
            }, "required": ["summary"] }
        }),
    ]
}

/// What an MCP call did, plus a prompt to wake the agent with (never for the caller's own run).
pub fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    if !name.starts_with("grill_") {
        return None;
    }
    if !enabled(me) {
        return Some(Err("Grill mode isn't enabled for you.".into()));
    }
    Some((|| match name {
        "grill_start" => {
            let mut qs = parse_questions(&args["questions"])?;
            if qs.is_empty() {
                return Err("give at least one question".into());
            }
            qs.truncate(MAX_QUESTIONS);
            let topic = args["topic"].as_str().unwrap_or("").trim();
            let mut s = match active(state, &me.id) {
                Some(s) if s.status == "preparing" => s,
                Some(mut old) => {
                    finish(state, &mut old, "superseded")?;
                    new_session(&me.id, topic, "active")
                }
                None => new_session(&me.id, topic, "active"),
            };
            if !topic.is_empty() {
                s.topic = clip(topic, 120);
            }
            s.goal = clip(args["goal"].as_str().unwrap_or(""), 1000);
            s.lang = match args["language"].as_str().map(|l| l.to_lowercase()) {
                Some(l) if l.starts_with("pt") => "pt".into(),
                Some(l) if l.starts_with("en") => "en".into(),
                // Models often leave it out: go by the user's request (a prepared session's topic) and the questions.
                _ => guess_lang(&format!("{} {}", s.topic, qs.iter().map(|q| q.text.as_str()).collect::<Vec<_>>().join(" "))).into(),
            };
            s.status = "active".into();
            s.started_at = Some(now());
            s.questions = qs;
            status_row(state, &s, &say(&s, &format!("Grill started: {}", s.topic), &format!("Entrevista iniciada: {}", s.topic)), json!({ "state": "started" }));
            ask_next(state, &mut s);
            save(state, &mut s)?;
            Ok(format!(
                "Grill {} started with {} questions; question 1 is already in the chat. End your turn now with no reply text: don't mention, list or repeat the questions. Answers come back to you when the queue runs out.",
                s.id,
                s.questions.len()
            ))
        }
        "grill_ask" => {
            let mut s = active(state, &me.id).filter(|s| s.status != "preparing").ok_or("No grill is running. Call grill_start.")?;
            let known: Vec<String> = s.questions.iter().filter(|q| q.status != "withdrawn").map(|q| commands::normalize(&q.text)).collect();
            let given = parse_questions(&args["questions"])?;
            let n_given = given.len();
            // A follow-up that only repeats a question already asked or queued is dropped.
            let qs: Vec<Question> = given.into_iter().filter(|q| !known.contains(&commands::normalize(&q.text))).collect();
            if qs.is_empty() && n_given > 0 {
                return Err("Those questions were already asked or are queued. Ask something new, or finish with grill_end.".into());
            }
            if total(&s) + qs.len() > MAX_QUESTIONS {
                return Err(format!("a grill holds at most {MAX_QUESTIONS} questions; wrap up with grill_end"));
            }
            let n = qs.len();
            if args["position"].as_str() == Some("next") {
                let at = s.questions.iter().position(|q| q.status == "queued").unwrap_or(s.questions.len());
                s.questions.splice(at..at, qs);
            } else {
                s.questions.extend(qs);
            }
            // Nothing on screen (the queue had run out): ask right away.
            if s.current.is_none() && s.status == "active" {
                ask_next(state, &mut s);
            }
            save(state, &mut s)?;
            Ok(format!("Added {n} question(s). End your turn with no reply text: they appear in the chat on their own."))
        }
        "grill_withdraw" => {
            let mut s = active(state, &me.id).ok_or("No grill is running.")?;
            let ids: Vec<&str> = args["question_ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
            let mut n = 0;
            for q in s.questions.iter_mut().filter(|q| q.status == "queued" && ids.contains(&q.id.as_str())) {
                q.status = "withdrawn".into();
                n += 1;
            }
            save(state, &mut s)?;
            Ok(format!("Withdrew {n} queued question(s)."))
        }
        "grill_state" => {
            let s = active(state, &me.id).ok_or("No grill is running.")?;
            Ok(format!("Grill {} on \"{}\" ({}).\n\n{}", s.id, s.topic, s.status, transcript(&s)))
        }
        "grill_record" => {
            let mut s = active(state, &me.id).ok_or("No grill is running.")?;
            let qid = args["question_id"].as_str().unwrap_or("");
            let answer = clip(args["answer"].as_str().unwrap_or(""), 500);
            if answer.is_empty() {
                return Err("answer is required".into());
            }
            let Some(q) = s.questions.iter_mut().find(|q| q.id == qid) else {
                return Err("No question with that id. Call grill_state.".into());
            };
            if q.status != "delegated" {
                // The user went back and answered it, or skipped it, before you got here.
                return Ok("The user has dealt with that question themselves; nothing to record. Reply with nothing.".into());
            }
            q.status = "answered".into();
            q.answer = Some(answer.clone());
            q.source = Some("agent".into());
            let n = q.index.unwrap_or(0);
            status_row(state, &s, &say(&s, &format!("Agent decided question {n}: {answer}"), &format!("O agente decidiu a pergunta {n}: {answer}")), json!({ "state": "command", "command": "decide", "index": n }));
            save(state, &mut s)?;
            Ok("Recorded.".into())
        }
        "grill_summary" => {
            let s = active(state, &me.id).ok_or("No grill is running.")?;
            let summary = args["summary"].as_str().unwrap_or("").trim();
            if summary.is_empty() {
                return Err("summary is required".into());
            }
            add_msg(state, &me.id, None, "assistant", "text", summary, json!({ "grill": { "session_id": s.id, "role": "summary", "final": false } }));
            Ok("Posted. Reply with nothing; the grill continues.".into())
        }
        "grill_end" => {
            // The running grill, or one the user just stopped that still needs its summary.
            let mut s = active(state, &me.id)
                .or_else(|| state.db.list_grills(&me.id).ok()?.into_iter().next().filter(|s| s.status == "ended" && !s.summarized && s.end_reason.as_deref() != Some("superseded")))
                .ok_or("No grill is running.")?;
            let summary = args["summary"].as_str().unwrap_or("").trim();
            if summary.is_empty() {
                return Err("summary is required".into());
            }
            // A follow-up may have gone on screen since this run started; don't end under the user's feet.
            if let Some(q) = current(&s).filter(|_| s.status != "ended") {
                return Err(format!(
                    "Question {} is on screen waiting for the user's answer, so the grill can't end yet. Reply with nothing; you're woken again when the queue runs out.",
                    q.index.unwrap_or(0)
                ));
            }
            s.notes_path = write_notes(state, &s, summary);
            add_msg(state, &me.id, None, "assistant", "text", summary, json!({ "grill": { "session_id": s.id, "role": "summary", "final": true, "notes_path": s.notes_path } }));
            s.summarized = true;
            if s.status == "ended" {
                save(state, &mut s)?;
            } else {
                finish(state, &mut s, "done")?;
            }
            Ok(format!(
                "Grill ended; the summary is in the chat{}. End your turn now without any reply text.",
                s.notes_path.as_deref().map(|p| format!(" and the full Q&A is in `{p}` in your workspace")).unwrap_or_default()
            ))
        }
        _ => Err(format!("unknown tool {name}")),
    })())
}

/// The system-prompt section for agents with grill mode on.
pub fn prompt_section(a: &Agent) -> &'static str {
    if !enabled(a) {
        return "";
    }
    r#"

# Grill mode
When the user asks to be grilled, interviewed or quizzed on something ("grill me on this", "me sabatina", "me entrevista sobre isso"), run a guided interview instead of a normal reply:
- Call `mcp__loft__grill_start` with a short `topic`, a `goal`, and a first batch of 3-8 questions, ordered from broad to specific. Loft shows them one at a time and posts the next the moment the user answers, so you aren't asked between questions.
- One question per item, answerable in a sentence. Give every question a `speak` version: short and plain, no Markdown, links, ids or PR numbers, options included. Use `kind` and `options` when there are obvious choices.
- Ask in the language the user used (pass it as `language`), and write the summary in it too.
- Never mention, list or repeat questions in your own replies: they appear in the chat by themselves, and talking about them spoils what comes next. After a grill tool, end your turn with no text.
- Never re-ask a question that was already answered; a follow-up must ask something new.
- When the queue runs out you get the answers: add follow-ups with `grill_ask` or finish with `grill_end` and a summary (decisions, open questions, what was skipped). After `grill_end`, reply with nothing: the summary is the reply.
- End the final summary with a one-line offer to turn it into a task (in the user's language). Only if the user says yes, call `mcp__loft__create_task` with the decisions as the description and the notes file path that `grill_end` gives you.
- The user can skip, go back, pause or stop at any time; Loft handles that. You only hear about it when you have something to do: a question left to you ("decide for me", answer it with `grill_record`), a summary so far (`grill_summary`), or a stopped grill (`grill_end`)."#
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::{agent, state};

    fn q(text: &str) -> Value {
        json!({ "text": text, "speak": text })
    }

    #[test]
    fn enabled_for_the_orchestrator_by_default() {
        let st = state();
        let mut o = agent(&st, "Orchestrator");
        let mut x = agent(&st, "Helper");
        assert!(enabled(&o) && !enabled(&x));
        o.grill = Some(false);
        x.grill = Some(true);
        assert!(!enabled(&o) && enabled(&x));
        assert_eq!(mcp_call(&st, &agent(&st, "Other"), "grill_state", &json!({})), Some(Err("Grill mode isn't enabled for you.".into())));
        assert_eq!(mcp_call(&st, &o, "remember", &json!({})), None);
    }

    #[test]
    fn questions_are_validated() {
        assert!(parse_questions(&json!([{ "text": "" }])).is_err());
        assert!(parse_questions(&json!([{ "text": "x".repeat(501) }])).is_err());
        assert!(parse_questions(&json!([{ "text": "a", "kind": "essay" }])).is_err());
        assert!(parse_questions(&json!([{ "text": "a", "kind": "single_choice" }])).is_err());
        assert!(parse_questions(&json!([{ "text": "a", "kind": "single_choice", "options": ["1", "2", "3", "4", "5", "6", "7"] }])).is_err());
        let ok = parse_questions(&json!([{ "text": "a", "kind": "yes_no" }, { "text": "b" }])).unwrap();
        assert_eq!((ok[0].kind.as_str(), ok[1].kind.as_str()), ("yes_no", "free_text"));
    }

    #[test]
    fn answers_post_the_next_question_and_wake_the_agent_when_the_queue_runs_out() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        let r = mcp_call(&st, &o, "grill_start", &json!({ "topic": "Pricing", "questions": [q("Monthly or yearly?"), q("Free trial?")] }));
        assert!(r.unwrap().is_ok());
        let s = active(&st, &o.id).unwrap();
        let q1 = s.current.clone().unwrap();
        let msgs = st.db.list_messages(&o.id, 50, None).unwrap();
        let question = msgs.iter().find(|m| m.meta["grill"]["role"] == "question").unwrap();
        assert_eq!((question.role.as_str(), question.content.as_str()), ("assistant", "Monthly or yearly?"));
        assert_eq!((question.meta["grill"]["index"].clone(), question.meta["grill"]["total"].clone()), (json!(1), json!(2)));

        // Untagged text answers the question on screen; the next one follows right away.
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "Yearly", None, json!({})).unwrap() else { panic!() };
        // The next question is up already; the agent revises the queue in the background, on the cheaper model.
        let wake = wake.unwrap();
        assert_eq!(wake.trigger, "grill_revise");
        assert!(wake.prompt.contains("Yearly") && wake.prompt.contains("Free trial?"));
        let s = active(&st, &o.id).unwrap();
        assert_ne!(s.current.as_deref(), Some(q1.as_str()));
        assert_eq!(s.questions[0].answer.as_deref(), Some("Yearly"));

        // An answer to a closed question, or to a session that ended, is refused with a code the app can show.
        let late = json!({ "role": "answer", "session_id": s.id, "question_id": q1 });
        assert_eq!(on_user_message(&st, &o, "Monthly", Some(&late), json!({})).err().map(|e| (e.0, e.1)), Some((StatusCode::CONFLICT, "question_closed".into())));

        let q2 = json!({ "role": "answer", "session_id": s.id, "question_id": s.current });
        let Routed::Handled { wake, message } = on_user_message(&st, &o, "No", Some(&q2), json!({ "client_id": "c1" })).unwrap() else { panic!() };
        assert_eq!(message.unwrap().meta["client_id"], "c1");
        let wake = wake.expect("the agent is woken when the queue runs out");
        assert_eq!(wake.trigger, "grill");
        assert!(wake.prompt.contains("Yearly") && wake.prompt.contains("No"));

        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "Yearly, no trial." })).unwrap().is_ok());
        assert!(active(&st, &o.id).is_none());
        assert_eq!(on_user_message(&st, &o, "x", Some(&q2), json!({})).err().map(|e| e.1), Some("grill_closed".into()));
        // With no session, plain text is ordinary chat again.
        assert!(matches!(on_user_message(&st, &o, "hello", None, json!({})).unwrap(), Routed::Normal(_)));
    }

    #[test]
    fn repeated_follow_ups_are_dropped() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("Monthly or yearly?"), q("B?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "yearly", None, json!({})).ok().unwrap();
        assert!(mcp_call(&st, &o, "grill_ask", &json!({ "questions": [q("**Monthly or yearly?**")] })).unwrap().is_err());
        mcp_call(&st, &o, "grill_ask", &json!({ "questions": [q("monthly or yearly"), q("New one?")] })).unwrap().unwrap();
        assert_eq!(total(&active(&st, &o.id).unwrap()), 3);
    }

    #[test]
    fn follow_ups_can_jump_the_queue_and_restart_an_empty_one() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?"), q("B?")] })).unwrap().unwrap();
        mcp_call(&st, &o, "grill_ask", &json!({ "questions": [q("A2?")], "position": "next" })).unwrap().unwrap();
        on_user_message(&st, &o, "a", None, json!({})).ok().unwrap();
        assert_eq!(current(&active(&st, &o.id).unwrap()).unwrap().text, "A2?");
        on_user_message(&st, &o, "a2", None, json!({})).ok().unwrap();
        on_user_message(&st, &o, "b", None, json!({})).ok().unwrap();
        assert!(active(&st, &o.id).unwrap().current.is_none());
        mcp_call(&st, &o, "grill_ask", &json!({ "questions": [q("C?")] })).unwrap().unwrap();
        let s = active(&st, &o.id).unwrap();
        assert_eq!((current(&s).unwrap().text.as_str(), current(&s).unwrap().index), ("C?", Some(4)));
    }

    #[test]
    fn start_request_prepares_a_session_and_a_new_start_supersedes_the_old_one() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        let start = json!({ "role": "start_request" });
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "Grill me on pricing", Some(&start), json!({})).unwrap() else { panic!() };
        assert!(wake.unwrap().prompt.contains("grill_start"));
        let prep = active(&st, &o.id).unwrap();
        assert_eq!((prep.status.as_str(), view(&prep)["total"].clone()), ("preparing", Value::Null));
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "Pricing", "questions": [q("A?")] })).unwrap().unwrap();
        assert_eq!(active(&st, &o.id).unwrap().id, prep.id, "grill_start fills the prepared session");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "Other", "questions": [q("B?")] })).unwrap().unwrap();
        let old = st.db.get_grill(&prep.id).unwrap().unwrap();
        assert_eq!((old.status.as_str(), old.end_reason.as_deref()), ("ended", Some("superseded")));
        // A helper without grill mode treats the request as plain chat.
        assert!(matches!(on_user_message(&st, &agent(&st, "Helper"), "Grill me", Some(&start), json!({})).unwrap(), Routed::Normal(_)));
    }

    #[test]
    fn a_typed_start_request_replaces_the_open_grill_instead_of_answering_it() {
        for paused in [false, true] {
            let st = state();
            let o = agent(&st, "Orchestrator");
            mcp_call(&st, &o, "grill_start", &json!({ "topic": "Settings screen", "questions": [q("Who uses it?"), q("B?")] })).unwrap().unwrap();
            let old = active(&st, &o.id).unwrap().id;
            if paused {
                on_user_message(&st, &o, "pause", None, json!({})).unwrap();
                assert_eq!(active(&st, &o.id).unwrap().status, "paused");
            }
            let Routed::Handled { message, wake } = on_user_message(&st, &o, "Grill me about the pricing for a small todo app", None, json!({})).unwrap() else { panic!() };
            let old = st.db.get_grill(&old).unwrap().unwrap();
            assert_eq!((old.status.as_str(), old.end_reason.as_deref()), ("ended", Some("superseded")), "paused: {paused}");
            assert!(old.questions.iter().all(|q| q.answer.is_none()), "the request was recorded as an answer");
            let new = active(&st, &o.id).unwrap();
            assert_eq!(new.status, "preparing");
            assert_eq!(message.unwrap().meta["grill"]["role"], json!("start_request"));
            assert!(wake.unwrap().prompt.contains("grill_start"));
            let rows: Vec<_> = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().map(|m| m.content).collect();
            assert!(rows.iter().any(|c| c == "Grill ended: a new one was requested"), "{rows:?}");
        }
    }

    #[test]
    fn typed_commands_skip_decide_and_go_back() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?"), q("B?"), q("C?"), q("D?")] })).unwrap().unwrap();
        let qs = |st: &AppState| active(st, &o.id).unwrap();

        // "pula" skips A and asks B; the user's message is kept, tagged as a command.
        let Routed::Handled { message, wake } = on_user_message(&st, &o, "Pula essa", None, json!({})).unwrap() else { panic!() };
        assert_eq!(message.unwrap().meta["grill"]["command"], "skip");
        assert!(wake.is_none());
        assert_eq!((qs(&st).questions[0].status.as_str(), current(&qs(&st)).unwrap().text.as_str()), ("skipped", "B?"));
        let rows = st.db.list_messages(&o.id, 50, None).unwrap();
        let row = rows.iter().rev().find(|m| m.meta["grill"]["role"] == "status").unwrap();
        assert_eq!(row.content, "Skipped question 1");
        assert!(row.meta["grill"]["tagged_message_id"].is_i64(), "lets the app offer Undo");

        // "I don't know" leaves B to the agent, which is woken to decide while C goes up.
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "I don't know", None, json!({})).unwrap() else { panic!() };
        let wake = wake.unwrap();
        assert_eq!(wake.trigger, "grill");
        assert!(wake.prompt.contains("grill_record") && wake.prompt.contains("B?"));
        let b = qs(&st).questions[1].clone();
        assert_eq!(b.status, "delegated");
        mcp_call(&st, &o, "grill_record", &json!({ "question_id": b.id, "answer": "Go with yearly" })).unwrap().unwrap();
        assert_eq!(qs(&st).questions[1].source.as_deref(), Some("agent"));

        // An answer, then "go back" re-asks it; the question that was on screen comes next again.
        on_user_message(&st, &o, "c answer", None, json!({})).ok().unwrap();
        assert_eq!(current(&qs(&st)).unwrap().text, "D?");
        on_user_message(&st, &o, "go back", None, json!({})).ok().unwrap();
        let s = qs(&st);
        assert_eq!((current(&s).unwrap().text.as_str(), current(&s).unwrap().index), ("C?", Some(3)));
        on_user_message(&st, &o, "c, better answer", None, json!({})).ok().unwrap();
        let s = qs(&st);
        assert_eq!(s.questions.iter().find(|x| x.text == "C?").unwrap().answer.as_deref(), Some("c, better answer"));
        assert_eq!((current(&s).unwrap().text.as_str(), current(&s).unwrap().index), ("D?", Some(4)));
    }

    #[test]
    fn pause_resume_and_long_text_is_an_answer() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?"), q("B?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "hold on", None, json!({})).ok().unwrap();
        assert_eq!(active(&st, &o.id).unwrap().status, "paused");
        // Any answer resumes, and it lands on the paused question.
        on_user_message(&st, &o, "Skip the database part and use Postgres", None, json!({})).ok().unwrap();
        let s = active(&st, &o.id).unwrap();
        assert_eq!((s.status.as_str(), s.questions[0].answer.as_deref()), ("active", Some("Skip the database part and use Postgres")));
        // Tagged answers from the app are never re-read as commands.
        let tag = json!({ "role": "answer", "session_id": s.id, "question_id": s.current });
        on_user_message(&st, &o, "stop", Some(&tag), json!({})).ok().unwrap();
        assert_eq!(active(&st, &o.id).unwrap().questions[1].answer.as_deref(), Some("stop"));
    }

    #[test]
    fn stop_ends_at_once_and_the_agent_still_writes_the_summary() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?"), q("B?")] })).unwrap().unwrap();
        let id = active(&st, &o.id).unwrap().id;
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "That's enough", None, json!({})).unwrap() else { panic!() };
        assert!(wake.unwrap().prompt.contains("grill_end"));
        assert!(active(&st, &o.id).is_none());
        let s = st.db.get_grill(&id).unwrap().unwrap();
        assert_eq!(s.end_reason.as_deref(), Some("stopped"));
        mcp_call(&st, &o, "grill_end", &json!({ "summary": "Nothing decided." })).unwrap().unwrap();
        assert!(st.db.get_grill(&id).unwrap().unwrap().summarized);
        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "again" })).unwrap().is_err(), "only once");
    }

    #[test]
    fn natural_stop_wordings_end_the_grill_instead_of_answering() {
        for text in ["stop grill session", "Can we end the interview now?", "encerra a sessão do grill"] {
            let st = state();
            let o = agent(&st, "Orchestrator");
            mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?"), q("B?")] })).unwrap().unwrap();
            let id = active(&st, &o.id).unwrap().id;
            on_user_message(&st, &o, text, None, json!({})).unwrap();
            let s = st.db.get_grill(&id).unwrap().unwrap();
            assert_eq!((s.status.as_str(), s.end_reason.as_deref()), ("ended", Some("stopped")), "{text:?}");
            assert!(s.questions.iter().all(|q| q.answer.is_none()), "{text:?} was recorded as an answer");
        }
    }

    #[test]
    fn undo_turns_a_misread_command_back_into_the_answer() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("What comes after design?"), q("B?")] })).unwrap().unwrap();
        let Routed::Handled { message, .. } = on_user_message(&st, &o, "next", None, json!({})).unwrap() else { panic!() };
        let mid = message.unwrap().id;
        let mut s = active(&st, &o.id).unwrap();
        assert_eq!(s.questions[0].status, "skipped");
        assert!(undo_command(&st, &mut s, mid + 1).is_err(), "only the matched message can be undone");
        let mut s = active(&st, &o.id).unwrap();
        undo_command(&st, &mut s, mid).unwrap();
        let s = active(&st, &o.id).unwrap();
        let posts = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().filter(|m| m.content == "B?").count();
        assert_eq!(posts, 1, "B is already on screen, so it isn't posted again");
        assert!(undo_command(&st, &mut active(&st, &o.id).unwrap(), mid).is_err(), "only once");
        assert_eq!((s.questions[0].status.as_str(), s.questions[0].answer.as_deref()), ("answered", Some("next")));
        assert_eq!(current(&s).unwrap().text, "B?");
    }

    #[test]
    fn language_is_guessed_when_the_agent_leaves_it_out() {
        assert_eq!(guess_lang("Me sabatina sobre o nome do próximo recurso. O que é o recurso? Quem vai usar?"), "pt");
        assert_eq!(guess_lang("Grill me on pricing. Who is the main buyer? What should it cost?"), "en");
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "Nome do recurso", "questions": [q("Qual é o nome que você prefere?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "pausa", None, json!({})).ok().unwrap();
        let rows: Vec<String> = st.db.list_messages(&o.id, 20, None).unwrap().into_iter().filter(|m| m.kind == "notice").map(|m| m.content).collect();
        assert_eq!(rows, ["Entrevista iniciada: Nome do recurso", "Pausada"]);
    }

    #[test]
    fn commands_need_a_question_on_screen() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?")] })).unwrap().unwrap();
        let mut s = active(&st, &o.id).unwrap();
        assert_eq!(apply_command(&st, &mut s, "back", None).err().map(|e| e.1), Some("no_previous_question".into()));
        apply_command(&st, &mut s, "skip", None).unwrap();
        assert_eq!(apply_command(&st, &mut s, "skip", None).err().map(|e| e.1), Some("question_closed".into()));
        assert_eq!(apply_command(&st, &mut s, "launch", None).err().map(|e| e.0), Some(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn idle_grills_pause_then_end_and_still_get_a_summary() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?"), q("B?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "a", None, json!({})).ok().unwrap();
        let now = chrono::Utc::now();
        assert!(sweep(&st, now + chrono::Duration::minutes(29)).is_empty());
        assert_eq!(active(&st, &o.id).unwrap().status, "active");
        sweep(&st, now + chrono::Duration::minutes(31));
        assert_eq!(active(&st, &o.id).unwrap().status, "paused");
        assert!(sweep(&st, now + chrono::Duration::hours(23)).is_empty());
        let wakes = sweep(&st, now + chrono::Duration::hours(25));
        assert_eq!(wakes.len(), 1);
        assert!(wakes[0].1.contains("grill_end") && wakes[0].1.contains("A?"));
        assert!(active(&st, &o.id).is_none());
        // A start request nobody followed up on just expires.
        on_user_message(&st, &o, "Grill me", Some(&json!({ "role": "start_request" })), json!({})).ok().unwrap();
        assert!(sweep(&st, chrono::Utc::now() + chrono::Duration::minutes(11)).is_empty());
        assert!(active(&st, &o.id).is_none());
    }

    #[test]
    fn the_agent_cannot_end_while_a_question_waits() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "T", "questions": [q("A?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "a", None, json!({})).ok().unwrap();
        // A revision run adds a follow-up, which goes on screen right away...
        mcp_call(&st, &o, "grill_ask", &json!({ "questions": [q("A2?")] })).unwrap().unwrap();
        // ...while the run woken for the empty queue tries to wrap up.
        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "x" })).unwrap().unwrap_err().contains("on screen"));
        on_user_message(&st, &o, "a2", None, json!({})).ok().unwrap();
        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "x" })).unwrap().is_ok());
    }

    #[test]
    fn the_end_writes_a_notes_file_in_the_workspace() {
        let st = state();
        let o = agent(&st, "Orchestrator");
        mcp_call(&st, &o, "grill_start", &json!({ "topic": "Pricing / ../../etc", "questions": [q("Monthly or yearly?"), q("Trial?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "Yearly", None, json!({})).ok().unwrap();
        on_user_message(&st, &o, "skip", None, json!({})).ok().unwrap();
        let r = mcp_call(&st, &o, "grill_end", &json!({ "summary": "Yearly." })).unwrap().unwrap();
        let s = st.db.list_grills(&o.id).unwrap().remove(0);
        let path = s.notes_path.clone().unwrap();
        assert!(path.starts_with("notes/grill/") && path.ends_with("-pricing-etc.md"), "{path}");
        assert!(r.contains(&path));
        let text = std::fs::read_to_string(crate::agents::workspace(&st, &o.id).join(&path)).unwrap();
        assert!(text.contains("Yearly.") && text.contains("### 1. Monthly or yearly?") && text.contains("_Skipped_"));
        let summary = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["final"] == true).unwrap();
        assert_eq!(summary.meta["grill"]["notes_path"], json!(path));
    }
}
