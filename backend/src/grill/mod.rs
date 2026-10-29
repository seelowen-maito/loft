//! "Grill me": a guided interview inside the normal chat, in rounds (ADR 0001). The agent posts a
//! round with `grill_round`: every question whose prerequisites are settled, each with a recommendation
//! worded so "yes" accepts it. Loft holds the user's answers without running the agent; sending the
//! round wakes the agent once with all of them, and it posts the next round or ends with a summary.
//! Rounds, sent answers, status rows and the summary are ordinary chat messages tagged with `meta.grill`,
//! so they render inline in the same conversation (and as plain chat on clients that don't know the tag).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

/// Most questions one round may hold.
pub const MAX_ROUND: usize = 12;
/// Most rounds one session may hold.
pub const MAX_ROUNDS: usize = 20;
const KINDS: &[&str] = &["free_text", "single_choice", "multi_choice", "yes_no", "number"];
const MAX_TEXT: usize = 500;
const MAX_SPEAK: usize = 300;
const MAX_RECOMMENDATION: usize = 500;
const MAX_OPTIONS: usize = 6;
const MAX_OPTION: usize = 60;
const MAX_ANSWER: usize = 4000;
/// Context size, in tokens, near the end of the smart zone (about 100k, past which models get noticeably
/// worse): a round sent beyond it, before the tickets are published, has the agent suggest `/compact`.
pub const COMPACT_AT: u64 = 80_000;
/// Before the grill's last call: the glossary and ADRs written along the way go up as one PR.
const DOCS_PR_STEP: &str = "if your docs branch has commits, push it and open its one PR with `gh pr create` (pass its URL as `docs_pr`; with no commits, open no PR), then ";

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
    /// In the order posted; only the last one can be open (not sent yet).
    #[serde(default)]
    pub rounds: Vec<Round>,
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
    /// The spec and tickets published while this grill ran, for the end-of-planning card.
    #[serde(default)]
    pub published: Option<Published>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Published {
    /// `owner/repo` they were published on.
    pub repo: String,
    #[serde(default)]
    pub spec: Option<IssueLink>,
    #[serde(default)]
    pub tickets: Vec<IssueLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueLink {
    /// The GitHub issue number; for a plan saved in Loft (no repo), the position in the plan, from 1.
    pub number: i64,
    #[serde(default)]
    pub title: String,
    /// The Loft issue, for a plan saved in Loft instead of on GitHub.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Round {
    /// 1-based.
    pub index: u32,
    pub questions: Vec<Question>,
    pub posted_at: String,
    /// When the user sent it; None while it waits for answers.
    #[serde(default)]
    pub sent_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub text: String,
    /// Short plain version for reading aloud.
    #[serde(default)]
    pub speak: String,
    pub kind: String,
    #[serde(default)]
    pub options: Vec<String>,
    /// The agent's proposed answer; "yes" accepts it.
    #[serde(default)]
    pub recommendation: String,
    /// open | answered | discuss (the answer is the user's comment) | decided (left to the agent: its
    /// recommendation stands) | carried (sent unanswered: Loft asks it again, unchanged, in the next round)
    pub state: String,
    #[serde(default)]
    pub answer: Option<String>,
    /// The answer is the recommendation, accepted as-is.
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub answered_at: Option<String>,
    /// The round it was first asked in, when it was carried into this one unanswered.
    #[serde(default)]
    pub from_round: Option<u32>,
    /// Uploads (the agent's `uploads/` folder) sent with a typed answer or comment.
    #[serde(default)]
    pub images: Vec<String>,
}

/// One lock per agent, held around every load, change and save of its grill session. Writers (answers
/// from the web and the phone, commands, the agent's tools, publishing, the sweeper) would otherwise
/// overwrite each other's changes to the session blob.
#[derive(Default)]
pub struct Locks(Mutex<HashMap<String, Arc<Mutex<()>>>>);

impl Locks {
    fn of(&self, agent_id: &str) -> Arc<Mutex<()>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).entry(agent_id.into()).or_default().clone()
    }
}

/// Runs `f` holding `agent_id`'s grill lock.
fn locked<T>(state: &AppState, agent_id: &str, f: impl FnOnce() -> T) -> T {
    let lock = state.grill_locks.of(agent_id);
    let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

pub fn active(state: &AppState, agent_id: &str) -> Option<Session> {
    state.db.active_grill(agent_id).ok()?
}

/// Notes what Loft just published for this agent on its running grill, for the end-of-planning card.
pub fn record_published(state: &AppState, agent_id: &str, repo: &str, spec: Option<IssueLink>, tickets: Vec<IssueLink>) {
    locked(state, agent_id, || record_published_locked(state, agent_id, repo, spec, tickets))
}

fn record_published_locked(state: &AppState, agent_id: &str, repo: &str, spec: Option<IssueLink>, tickets: Vec<IssueLink>) {
    let Some(mut s) = active(state, agent_id) else { return };
    let p = s.published.get_or_insert_with(Default::default);
    if p.repo != repo {
        *p = Published { repo: repo.into(), ..Default::default() };
    }
    if let Some(sp) = spec {
        // Publishing tickets names the spec by number only: keep the title publish_spec gave.
        if p.spec.as_ref().is_none_or(|old| old.number != sp.number || !sp.title.is_empty()) {
            p.spec = Some(sp);
        }
    }
    for t in tickets {
        if !p.tickets.iter().any(|o| o.number == t.number) {
            p.tickets.push(t);
        }
    }
    let _ = save(state, &mut s);
}

/// The end-of-planning card for a grill that published something: links to the spec and tickets, the
/// suggested next message for the composer after clearing context, and the roles the team lacks.
fn end_card(state: &AppState, me: &Agent, s: &Session) -> Option<Value> {
    let p = s.published.as_ref().filter(|p| p.spec.is_some() || !p.tickets.is_empty())?;
    // A plan with no repo was saved as Loft issues: links open them in Loft, and `repo` is empty.
    let url = |i: &IssueLink| if p.repo.is_empty() { format!("/tasks/{}", i.id) } else { format!("https://github.com/{}/issues/{}", p.repo, i.number) };
    let link = |i: &IssueLink| json!({ "number": i.number, "title": i.title, "url": url(i) });
    let next = match (&p.spec, p.tickets.as_slice()) {
        _ if p.repo.is_empty() => say(s, "Implement the plan: hand its ready Loft issues to the team", "Implemente o plano: entregue as issues prontas do Loft ao time"),
        (Some(sp), _) => say(s, &format!("Implement spec #{}: hand the ready tickets to the team", sp.number), &format!("Implemente a spec #{}: entregue os tickets prontos ao time", sp.number)),
        (None, [t]) => say(s, &format!("Implement ticket #{}", t.number), &format!("Implemente o ticket #{}", t.number)),
        (None, _) => say(s, "Implement the ready tickets: hand them to the team", "Implemente os tickets prontos: entregue-os ao time"),
    };
    let missing = crate::planning::missing_roles(state, &me.project);
    let setup = (!missing.is_empty()).then(|| {
        let roles = missing.iter().map(|r| if *r == "dev" { "Dev" } else { "reviewer" }).collect::<Vec<_>>().join(" or ");
        let plan = match (&p.spec, p.tickets.first()) {
            _ if p.repo.is_empty() => format!("“{}”, saved as Loft issues", p.spec.as_ref().or(p.tickets.first()).map_or("", |i| i.title.as_str())),
            (Some(sp), _) => format!("spec #{}: {}", sp.number, url(sp)),
            (None, Some(t)) => format!("ticket #{}: {}", t.number, url(t)),
            (None, None) => unreachable!("a card has a spec or a ticket"),
        };
        json!({
            "agent_id": crate::setup::LOFT_ID,
            "message": format!("Set up the team for the {} project: it has no {roles} yet. {} just planned {plan}. Propose the agents it needs and create the ones I approve.", me.project.trim(), me.name),
        })
    });
    Some(json!({
        "repo": p.repo,
        "spec": p.spec.as_ref().map(link),
        "tickets": p.tickets.iter().map(link).collect::<Vec<_>>(),
        "next_message": next,
        "missing_roles": missing,
        "setup": setup,
    }))
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// The round waiting for the user's answers, if any.
pub fn open_round(s: &Session) -> Option<&Round> {
    s.rounds.last().filter(|r| r.sent_at.is_none())
}

fn open_round_mut(s: &mut Session) -> Option<&mut Round> {
    s.rounds.last_mut().filter(|r| r.sent_at.is_none())
}

/// The session as clients see it (`GET /agents/{id}/grill` and the `grill` event): the latest round with
/// its answers so far, for the round bar and the cards.
pub fn view(s: &Session) -> Value {
    let last = s.rounds.last();
    json!({
        "session_id": s.id,
        "agent_id": s.agent_id,
        "state": s.status,
        "end_reason": s.end_reason,
        "topic": s.topic,
        "round": last.map(|r| r.index),
        "total": last.map(|r| r.questions.len()),
        "answered": last.map(|r| r.questions.iter().filter(|q| q.state != "open").count()),
        // The user sent the latest round and the agent is working on the next one.
        "waiting": last.is_some_and(|r| r.sent_at.is_some()) && s.status != "ended",
        "questions": last.map(|r| &r.questions),
        "started_at": s.started_at,
        "updated_at": s.updated_at,
        "notes_path": s.notes_path,
        // en | pt, for the phrases the app reads aloud: en until the first round sets it, null only for sessions saved before it was kept.
        "language": (!s.lang.is_empty()).then_some(&s.lang),
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

/// The chat message for a round: the questions as Markdown for clients without cards, and the questions
/// themselves in `meta.grill` for the ones with.
fn post_round(state: &AppState, s: &Session, r: &Round) {
    let rec = say(s, "Recommendation", "Recomendação");
    let mut text = format!("**{} {}**\n", say(s, "Round", "Rodada"), r.index);
    for (i, q) in r.questions.iter().enumerate() {
        text += &format!("\n{}. {}\n   _{rec}:_ {}\n", i + 1, q.text, q.recommendation);
    }
    let questions: Vec<Value> = r
        .questions
        .iter()
        .map(|q| json!({ "id": q.id, "text": q.text, "speak": q.speak, "kind": q.kind, "options": q.options, "recommendation": q.recommendation, "from_round": q.from_round }))
        .collect();
    let g = json!({ "session_id": s.id, "role": "round", "round": r.index, "topic": s.topic, "questions": questions });
    add_msg(state, &s.agent_id, None, "assistant", "text", text.trim_end(), json!({ "grill": g }));
}

/// One question's answer, as the agent reads it.
fn answer_line(q: &Question) -> String {
    let line: String = match q.state.as_str() {
        "answered" if q.accepted => "accepted the recommendation".into(),
        "answered" => format!("\"{}\"", q.answer.as_deref().unwrap_or("")),
        "discuss" => format!("wants to discuss first: \"{}\"", q.answer.as_deref().unwrap_or("")),
        "decided" => "left it to you: your recommendation is the decision".into(),
        "carried" => "no answer (Loft asks it again in the next round)".into(),
        _ => "no answer".into(),
    };
    if q.images.is_empty() || !matches!(q.state.as_str(), "answered" | "discuss") {
        return line;
    }
    let paths: Vec<String> = q.images.iter().map(|f| format!("uploads/{f}")).collect();
    format!("{line}, with images: {}", paths.join(", "))
}

/// The user's answers to a round, posted in chat when it is sent. Plain text: clients show user messages as typed.
fn answers_text(s: &Session, r: &Round) -> String {
    let mut out = format!("{} {}:\n", say(s, "Answers to round", "Respostas da rodada"), r.index);
    for (i, q) in r.questions.iter().enumerate() {
        let a = match q.state.as_str() {
            "answered" if q.accepted => format!("{} ({})", say(s, "Yes", "Sim"), q.recommendation),
            "answered" => q.answer.clone().unwrap_or_default(),
            "discuss" => format!("{} {}", say(s, "Let's discuss:", "Vamos discutir:"), q.answer.as_deref().unwrap_or("")),
            "decided" => say(s, &format!("Up to you ({})", q.recommendation), &format!("Você decide ({})", q.recommendation)),
            _ => format!("({})", say(s, "no answer", "sem resposta")),
        };
        let a = match q.images.len() {
            0 => a,
            n => format!("{} [{n} {}]", a.trim_end(), if n == 1 { say(s, "image", "imagem") } else { say(s, "images", "imagens") }),
        };
        out += &format!("\n{}. {}", i + 1, a.trim_start());
    }
    out
}

/// Validates the `questions` argument of `grill_round`.
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
            let recommendation = s("recommendation");
            if recommendation.is_empty() {
                return Err("every question needs a `recommendation`: your proposed answer".to_string());
            }
            if recommendation.chars().count() > MAX_RECOMMENDATION {
                return Err(format!("a recommendation is limited to {MAX_RECOMMENDATION} characters"));
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
                text,
                speak: clip(&s("speak"), MAX_SPEAK),
                kind,
                options,
                recommendation,
                state: "open".into(),
                answer: None,
                accepted: false,
                answered_at: None,
                from_round: None,
                images: vec![],
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
        rounds: vec![],
        created_at: now(),
        updated_at: now(),
        started_at: None,
        ended_at: None,
        notes_path: None,
        lang: "en".into(),
        summarized: false,
        seen_commands: vec![],
        published: None,
    }
}

fn finish(state: &AppState, s: &mut Session, reason: &str) -> Result<(), String> {
    finish_with(state, s, reason, json!({}))
}

fn finish_with(state: &AppState, s: &mut Session, reason: &str, extra: Value) -> Result<(), String> {
    s.status = "ended".into();
    s.end_reason = Some(reason.into());
    s.ended_at = Some(now());
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

fn internal(e: String) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e)
}

/// Every round so far with its answers, for prompts and `grill_state`.
fn transcript(s: &Session) -> String {
    let mut out = vec![];
    for r in &s.rounds {
        let mut block = format!("Round {}{}:", r.index, if r.sent_at.is_none() { " (waiting for the user)" } else { "" });
        for q in &r.questions {
            block += &format!("\n- [{}] {}\n  Recommendation: {}\n  Answer: {}", q.id, q.text, q.recommendation, answer_line(q));
        }
        out.push(block);
    }
    if out.is_empty() { "(no rounds yet)".into() } else { out.join("\n\n") }
}

/// The run after the user sends a round. `context` is the agent's context size in tokens, if known.
fn round_prompt(s: &Session, r: &Round, context: Option<u64>) -> String {
    // A long grill fills the context: before the spec is written on a degraded one, suggest compacting.
    let ticketed = s.published.as_ref().is_some_and(|p| !p.tickets.is_empty());
    let compact = match context {
        Some(n) if n >= COMPACT_AT && !ticketed => format!(
            "Your context holds about {}k tokens, near the end of the smart zone, and the tickets aren't published yet. Before anything else, write one short line to the user, in their language, suggesting they type `/compact` before the spec and tickets are written. Apart from that line, follow the steps below.\n\n",
            n / 1000
        ),
        _ => String::new(),
    };
    format!(
        "[Grill {} on \"{}\": the user sent round {}]\n\n{}\n\n{compact}Update your design tree with these answers, and record the terms and decisions they resolved in your docs branch (see Planning: docs). Questions left to you are decided: your recommendation stands. For each question the user wants to discuss, answer their comment in the new question's text and ask again, split into smaller questions if that helps. Unanswered questions are carried into the next round unchanged by Loft: don't ask them again (pass their ids in `drop` if one no longer matters). If decisions are still open, look up any facts they need with your sub-agents first, then post every question whose prerequisites are now settled as round {} with `mcp__loft__grill_round`. When every branch is resolved, with a project repo, the last round asks whether to turn this into a spec and tickets; if the user said yes, write and publish the spec (see Planning: spec), then post the ticket breakdown as a round (\"Publish these N tickets?\") and publish the tickets once the user says yes (see Planning: tickets). Then {DOCS_PR_STEP}finish with `mcp__loft__grill_end` and a summary in the user's language. Don't write a reply: the round or the summary is what the user sees.",
        s.id,
        s.topic,
        r.index,
        transcript(s),
        r.index + 1
    )
}

// ---- user side --------------------------------------------------------------

/// What `send_message` should do with a user message, given its optional `meta.grill`.
pub enum Routed {
    /// Not part of a grill: deliver as usual, with this extra meta.
    Normal(Value),
    /// Stored by the grill; wake the agent (quietly, trigger "grill") with this prompt if there is one.
    Handled { message: Option<Message>, wake: Option<String> },
}

/// Routes a message the user sent to `agent`. Only the user's own messages reach this; messages from
/// other agents, task briefs and wake-ups never count as answers.
pub fn on_user_message(state: &AppState, agent: &Agent, text: &str, grill: Option<&Value>, meta: Value) -> Result<Routed, ApiError> {
    locked(state, &agent.id, || route_user_message(state, agent, text, grill, meta))
}

fn route_user_message(state: &AppState, agent: &Agent, text: &str, grill: Option<&Value>, mut meta: Value) -> Result<Routed, ApiError> {
    let role = grill.and_then(|g| g["role"].as_str()).unwrap_or("");
    let conflict = |e: &str| ApiError(StatusCode::CONFLICT, e.into());

    // "Grill me…" typed (the web composer, old app builds) starts a grill like /grill does, and replaces an
    // open one rather than being taken as an answer.
    let typed_start = role.is_empty() && commands::is_start_request(text);
    if role == "start_request" || typed_start {
        let mut s = match active(state, &agent.id) {
            Some(s) if s.status == "preparing" => s,
            Some(mut old) => {
                finish(state, &mut old, "superseded").map_err(internal)?;
                new_session(&agent.id, text, "preparing")
            }
            None => new_session(&agent.id, text, "preparing"),
        };
        save(state, &mut s).map_err(internal)?;
        meta["grill"] = json!({ "session_id": s.id, "role": "start_request" });
        let message = add_msg(state, &agent.id, None, "user", "text", text, meta);
        let new_project = if crate::greenfield::needs_repo(state, agent) {
            " Your project has no GitHub repo yet: the first round also offers to create one (see Planning: new project)."
        } else {
            ""
        };
        let wake = format!(
            "{text}\n\n[Grill mode: the user wants to be grilled. Follow the grilling discipline in your instructions: read the project's existing documents as facts, look up the other facts you need with your sub-agents, then call `mcp__loft__grill_round` with the topic and the first round (every question whose prerequisites are already settled, each with a recommendation), in the language the user wrote in.{new_project}]"
        );
        return Ok(Routed::Handled { message, wake: Some(wake) });
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
        // No round yet: tagged answers can't belong anywhere, and plain text is context for the agent.
        return if tagged { Err(conflict("question_closed")) } else { Ok(Routed::Normal(meta)) };
    }

    // Untagged text (the web composer, old app builds) may be a command; anything else is ordinary chat,
    // never an answer: answers are tagged with their question (the cards, the app's voice loop).
    if !tagged {
        let commands::Parsed::Command(cmd) = commands::parse(text) else { return Ok(Routed::Normal(meta)) };
        meta["grill"] = json!({ "session_id": s.id, "role": "command", "command": cmd });
        let message = add_msg(state, &agent.id, None, "user", "text", text, meta);
        if commands::CLIENT_ONLY.contains(&cmd) {
            // Stepping through questions only means something by voice; say so rather than doing nothing.
            let hint = say(
                &s,
                "Skip, back and repeat step through questions by voice. Here, answer the cards in any order: questions left open are asked again in the next round.",
                "Pular, voltar e repetir servem para a voz. Aqui, responda os cartões em qualquer ordem: o que ficar aberto volta na próxima rodada.",
            );
            status_row(state, &s, &hint, json!({ "state": "hint", "command": cmd }));
            return Ok(Routed::Handled { message, wake: None });
        }
        let wake = apply_command(state, &mut s, cmd, None, message.as_ref().map(|m| m.id))?;
        return Ok(Routed::Handled { message, wake });
    }

    let qid = grill.and_then(|g| g["question_id"].as_str()).ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "an answer needs question_id".into()))?;
    let discuss = grill.is_some_and(|g| g["discuss"] == true);
    let images: Vec<String> = meta["images"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(String::from)).collect();
    record_answer(state, &mut s, qid, text, &images, discuss)?;
    Ok(Routed::Handled { message: None, wake: None })
}

/// True when `text` is a plain "yes", which accepts the recommendation.
fn is_yes(text: &str) -> bool {
    const YES: &[&str] = &["yes", "y", "yep", "yeah", "yes please", "sure", "ok", "okay", "agreed", "accept", "sim", "s", "pode ser", "claro", "beleza", "aceito", "concordo", "isso"];
    YES.contains(&commands::normalize(text).as_str())
}

/// Holds `text` and `images` (already validated uploads) as the answer to `qid` in the open round, without
/// waking the agent. The user may change it until the round is sent. With `discuss`, it is a comment instead
/// of a decision: the agent explains and asks again in the next round.
fn record_answer(state: &AppState, s: &mut Session, qid: &str, text: &str, images: &[String], discuss: bool) -> Result<(), ApiError> {
    let closed = || ApiError(StatusCode::CONFLICT, "question_closed".into());
    let q = open_round_mut(s).and_then(|r| r.questions.iter_mut().find(|q| q.id == qid)).ok_or_else(closed)?;
    q.images = images.to_vec();
    q.accepted = !discuss && images.is_empty() && is_yes(text);
    q.answer = Some(if q.accepted { q.recommendation.clone() } else { clip(text, MAX_ANSWER) });
    q.state = if discuss { "discuss" } else { "answered" }.into();
    q.answered_at = Some(now());
    resume_if_paused(state, s, None);
    save(state, s).map_err(internal)
}

/// Answering, deciding or sending while paused picks the grill up again, with the same row as Resume.
fn resume_if_paused(state: &AppState, s: &mut Session, extra: Option<Value>) {
    if s.status == "paused" {
        s.status = "active".into();
        let mut meta = json!({ "state": "resumed" });
        if let (Some(m), Some(Value::Object(e))) = (meta.as_object_mut(), extra) {
            m.extend(e);
        }
        status_row(state, s, &say(s, "Resumed", "Retomada"), meta);
    }
}

/// Applies a command to the running session and saves it. Returns a prompt to wake the agent with, if any.
/// `question_id` picks the question for `decide` (default: the first open one).
fn apply_command(state: &AppState, s: &mut Session, cmd: &str, question_id: Option<&str>, tagged_message_id: Option<i64>) -> Result<Option<String>, ApiError> {
    let row = |s: &Session, text: &str, st: &str| {
        let mut extra = json!({ "state": st, "command": cmd, "round": s.rounds.last().map(|r| r.index) });
        if let Some(id) = tagged_message_id {
            extra["tagged_message_id"] = json!(id);
        }
        status_row(state, s, text, extra);
    };
    let closed = || ApiError(StatusCode::CONFLICT, "round_closed".into());
    let mut wake = None;
    match cmd {
        "send" => {
            open_round(s).ok_or_else(closed)?;
            resume_if_paused(state, s, Some(json!({ "command": cmd })));
            let r = open_round_mut(s).ok_or_else(closed)?;
            r.sent_at = Some(now());
            for q in r.questions.iter_mut().filter(|q| q.state == "open") {
                q.state = "carried".into();
            }
            let r = r.clone();
            let text = answers_text(s, &r);
            let mut meta = json!({ "grill": { "session_id": s.id, "role": "answers", "round": r.index } });
            if let Some(id) = tagged_message_id {
                meta["grill"]["tagged_message_id"] = json!(id);
            }
            add_msg(state, &s.agent_id, None, "user", "text", &text, meta);
            let context = state.db.get_agent(&s.agent_id).ok().flatten().and_then(|a| a.context_tokens);
            // The answers' images reach the agent as chat images do: image blocks after the prompt.
            let images: Vec<String> = r.questions.iter().filter(|q| matches!(q.state.as_str(), "answered" | "discuss")).flat_map(|q| q.images.clone()).collect();
            wake = Some(round_prompt(s, &r, context) + &crate::uploads::markers(state, &s.agent_id, &images));
        }
        "decide" => {
            let r = open_round_mut(s).ok_or_else(closed)?;
            let (n, q) = match question_id {
                Some(id) => r.questions.iter_mut().enumerate().find(|(_, q)| q.id == id),
                None => r.questions.iter_mut().enumerate().find(|(_, q)| q.state == "open"),
            }
            .ok_or_else(|| ApiError(StatusCode::CONFLICT, "question_closed".into()))?;
            // Already left to the agent (the voice went back to it): nothing to do, and no second row.
            if q.state == "decided" {
                return Ok(None);
            }
            q.state = "decided".into();
            q.answer = Some(q.recommendation.clone());
            q.accepted = false;
            q.images.clear();
            q.answered_at = Some(now());
            let (qid, rec) = (q.id.clone(), q.recommendation.clone());
            resume_if_paused(state, s, Some(json!({ "command": cmd })));
            // The decision shows in the chat, so a typed "I don't know" never changes a card unseen.
            let text = say(s, &format!("Question {} left to the agent: {rec}", n + 1), &format!("Pergunta {} decidida pelo agente: {rec}", n + 1));
            let mut extra = json!({ "state": "decided", "command": cmd, "round": s.rounds.last().map(|r| r.index), "question_id": qid });
            if let Some(id) = tagged_message_id {
                extra["tagged_message_id"] = json!(id);
            }
            status_row(state, s, &text, extra);
        }
        "pause" => {
            s.status = "paused".into();
            row(s, &say(s, "Paused", "Pausada"), "paused");
        }
        "resume" => {
            if s.status == "paused" {
                s.status = "active".into();
                row(s, &say(s, "Resumed", "Retomada"), "resumed");
            }
        }
        "summarize" => {
            row(s, &say(s, "Summary requested", "Resumo pedido"), "command");
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
                "[Grill {} on \"{}\": the user stopped it]\n\n{}\n\n{DOCS_PR_STEP}call `mcp__loft__grill_end` with a summary of what was covered (the user's language), then reply with nothing.",
                s.id,
                s.topic,
                transcript(s)
            )));
        }
        // Stepping through questions is the voice client's job; there is nothing for the server to do.
        c if commands::CLIENT_ONLY.contains(&c) => return Ok(None),
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

/// `GET /api/grill/{id}`: one session with every round and answer.
pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(not_found());
    }
    let s = state.db.get_grill(&id)?.ok_or_else(not_found)?;
    let mut v = view(&s);
    v["rounds"] = json!(s.rounds);
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct CommandBody {
    command: String,
    question_id: Option<String>,
    client_id: Option<String>,
}

/// `POST /api/grill/{id}/command`: a button or a spoken command (send the round, decide for me, pause,
/// resume, end, summarize). A retry with the same `client_id` isn't applied twice.
pub async fn command(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<CommandBody>) -> Result<Json<Value>, ApiError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(not_found());
    }
    let agent_id = state.db.get_grill(&id)?.ok_or_else(not_found)?.agent_id;
    let (s, wake, duplicate) = locked(&state, &agent_id, || -> Result<_, ApiError> {
        // Read again under the lock: another writer may have changed it since.
        let mut s = state.db.get_grill(&id)?.ok_or_else(not_found)?;
        let client_id = b.client_id.map(|c| c.trim().chars().take(64).collect::<String>()).filter(|c| !c.is_empty());
        if client_id.as_ref().is_some_and(|c| s.seen_commands.contains(c)) {
            return Ok((s, None, true));
        }
        if s.status == "ended" || s.status == "preparing" {
            return Err(ApiError(StatusCode::CONFLICT, "grill_closed".into()));
        }
        let cmd = commands::COMMANDS.iter().find(|c| **c == b.command).copied().ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, format!("unknown command {}", b.command)))?;
        if let Some(c) = client_id {
            s.seen_commands.push(c);
            let n = s.seen_commands.len();
            s.seen_commands.drain(..n.saturating_sub(50));
        }
        let wake = apply_command(&state, &mut s, cmd, b.question_id.as_deref(), None)?;
        Ok((s, wake, false))
    })?;
    if duplicate {
        return Ok(Json(json!({ "ok": true, "duplicate": true, "session": view(&s) })));
    }
    if let Some(p) = wake {
        crate::agents::wake(&state, &s.agent_id, p, "grill");
    }
    Ok(Json(json!({ "ok": true, "session": view(&s) })))
}

/// `GET /api/grill/commands`: phrase lists for on-device matching. Clients cache by `version` / ETag.
pub async fn command_catalog() -> impl axum::response::IntoResponse {
    let etag = format!("\"grill-commands-{}\"", commands::VERSION);
    ([(axum::http::header::ETAG, etag), (axum::http::header::CACHE_CONTROL, "max-age=3600".to_string())], Json(commands::catalog()))
}

// ---- notes and timeouts -----------------------------------------------------------

/// True when the agent's project folder is a git repo. Without one, a grill's record is the notes file.
fn has_repo(state: &AppState, a: &Agent) -> bool {
    !a.project.trim().is_empty() && state.settings.boot().projects_dir.join(&a.project).join(".git").exists()
}

/// Writes the rounds and the summary to `notes/grill/<date>-<topic>.md` in the agent's workspace. The path
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
    out += &format!("\n## {}\n\n{}\n", say(s, "Summary", "Resumo"), summary.trim());
    for r in &s.rounds {
        out += &format!("\n## {} {}\n", say(s, "Round", "Rodada"), r.index);
        for (i, q) in r.questions.iter().enumerate() {
            let a = match q.state.as_str() {
                "answered" if q.accepted => format!("{} {}", say(s, "Accepted:", "Aceita:"), q.recommendation),
                "answered" => q.answer.clone().unwrap_or_default(),
                "discuss" => format!("**{}** {}", say(s, "To discuss:", "Para discutir:"), q.answer.as_deref().unwrap_or("")),
                "decided" => format!("**{}** {}", say(s, "Decided by the agent:", "Decidido pelo agente:"), q.recommendation),
                _ => format!("_{}_ ({} {})", say(s, "Not answered", "Sem resposta"), say(s, "recommended:", "recomendado:"), q.recommendation),
            };
            out += &format!("\n### {}. {}\n\n{a}\n", i + 1, q.text);
        }
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
    for open in state.db.open_grills().unwrap_or_default() {
        let lock = state.grill_locks.of(&open.agent_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        // Read again under the lock: an answer may have just come in.
        let Some(mut s) = state.db.get_grill(&open.id).ok().flatten() else { continue };
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
                let answered = s.rounds.iter().flat_map(|r| &r.questions).any(|q| q.state != "open" && q.state != "carried");
                s.summarized = !answered;
                if finish(state, &mut s, "timeout").is_ok() && answered {
                    wakes.push((
                        s.agent_id.clone(),
                        format!(
                            "[Grill {} on \"{}\" ended after a day without answers]\n\n{}\n\n{DOCS_PR_STEP}call `mcp__loft__grill_end` with a summary of what was covered (the user's language), then reply with nothing.",
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
        "text": { "type": "string", "description": "The question as shown in chat (Markdown, one decision, at most 500 characters), worded so that \"yes\" accepts the recommendation." },
        "recommendation": { "type": "string", "description": "Your proposed answer, grounded in what you looked up. The user accepts it with \"yes\"." },
        "speak": { "type": "string", "description": "A short plain version read aloud: no Markdown, links, ids or PR numbers; include the recommendation. Always give one." },
        "kind": { "type": "string", "enum": KINDS },
        "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers (at most 6). Required for single_choice / multi_choice." }
    }, "required": ["text", "recommendation", "speak"] });
    vec![
        json!({
            "name": "grill_round",
            "description": "Post a round of the grill in this chat: every question whose prerequisites are settled, each with a recommendation. The first round starts the grill. Questions the user sent unanswered are carried into it unchanged by Loft, before yours. Loft holds the user's answers and wakes you once, with all of them, when the user sends the round.",
            "inputSchema": { "type": "object", "properties": {
                "topic": { "type": "string", "description": "A few words, shown on the round bar. First round only." },
                "goal": { "type": "string", "description": "What the grill should decide. First round only." },
                "language": { "type": "string", "enum": ["en", "pt"], "description": "The language the user wrote in; questions, status rows and the summary follow it. First round only." },
                "questions": { "type": "array", "items": question, "description": format!("The new questions, most fundamental first. With the carried ones, at most {MAX_ROUND}. May be empty when questions carry over.") },
                "drop": { "type": "array", "items": { "type": "string" }, "description": "Ids of unanswered questions from the last round that no longer matter, so they aren't carried." }
            }, "required": ["questions"] }
        }),
        json!({
            "name": "grill_state",
            "description": "The running grill: every round so far with the user's answers.",
            "inputSchema": { "type": "object", "properties": {} }
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
            "description": "Finish the grill. `summary` (Markdown, the user's language) is posted in chat: decisions, open questions, what was left unanswered.",
            "inputSchema": { "type": "object", "properties": {
                "summary": { "type": "string" },
                "docs_pr": { "type": "string", "description": "The URL of the docs PR you opened for this grill's glossary and ADR changes. Leave it out when the grill resolved no terms or decisions: then there is no PR." }
            }, "required": ["summary"] }
        }),
    ]
}

/// What an MCP call did. Never wakes the caller: it is the one running.
pub fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value) -> Option<Result<String, String>> {
    if !name.starts_with("grill_") {
        return None;
    }
    Some(locked(state, &me.id, || match name {
        "grill_round" => {
            let mut qs = parse_questions(&args["questions"])?;
            let topic = args["topic"].as_str().unwrap_or("").trim();
            let mut s = match active(state, &me.id) {
                Some(s) => s,
                None => new_session(&me.id, topic, "preparing"),
            };
            if let Some(r) = open_round(&s) {
                return Err(format!("Round {} is still waiting for the user's answers; you're woken with them when the user sends it. End your turn with no reply text.", r.index));
            }
            if s.rounds.len() >= MAX_ROUNDS {
                return Err(format!("a grill holds at most {MAX_ROUNDS} rounds; wrap up with grill_end"));
            }
            // What the user sent unanswered comes back first, unchanged, unless the agent drops it.
            let drop: Vec<&str> = args["drop"].as_array().map(|d| d.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            let carried: Vec<Question> = s
                .rounds
                .last()
                .map(|r| {
                    r.questions
                        .iter()
                        .filter(|q| q.state == "carried" && !drop.contains(&q.id.as_str()))
                        .map(|q| Question { id: uuid::Uuid::new_v4().to_string(), state: "open".into(), answer: None, accepted: false, answered_at: None, from_round: q.from_round.or(Some(r.index)), images: vec![], ..q.clone() })
                        .collect()
                })
                .unwrap_or_default();
            if qs.is_empty() && carried.is_empty() {
                return Err("give at least one question".into());
            }
            if qs.len() + carried.len() > MAX_ROUND {
                return Err(if carried.is_empty() {
                    format!("a round holds at most {MAX_ROUND} questions; ask the rest once these are settled")
                } else {
                    format!("a round holds at most {MAX_ROUND} questions and {} carry over unanswered: post at most {} new ones, or `drop` carried ones that no longer matter", carried.len(), MAX_ROUND.saturating_sub(carried.len()))
                });
            }
            let n_carried = carried.len();
            qs.splice(0..0, carried);
            let first = s.status == "preparing";
            if first {
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
                status_row(state, &s, &say(&s, &format!("Grill started: {}", s.topic), &format!("Entrevista iniciada: {}", s.topic)), json!({ "state": "started" }));
            }
            let r = Round { index: s.rounds.len() as u32 + 1, questions: qs, posted_at: now(), sent_at: None };
            post_round(state, &s, &r);
            let (n, k) = (r.index, r.questions.len());
            s.rounds.push(r);
            save(state, &mut s)?;
            let carried = if n_carried > 0 { format!(" ({n_carried} carried over unanswered, first)") } else { String::new() };
            Ok(format!(
                "Round {n} posted with {k} question(s){carried} in grill {}; it is already in the chat. End your turn now with no reply text: don't mention, list or repeat the questions. You're woken with every answer when the user sends the round.",
                s.id
            ))
        }
        "grill_state" => {
            let s = active(state, &me.id).ok_or("No grill is running.")?;
            Ok(format!("Grill {} on \"{}\" ({}).\n\n{}", s.id, s.topic, s.status, transcript(&s)))
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
            // The user is answering a round: don't end under their feet.
            if let Some(r) = open_round(&s).filter(|_| s.status != "ended") {
                return Err(format!("Round {} is waiting for the user's answers, so the grill can't end yet. Reply with nothing; you're woken when the user sends it.", r.index));
            }
            let docs_pr = match args["docs_pr"].as_str().map(str::trim).filter(|u| !u.is_empty()) {
                None => None,
                Some(_) if !has_repo(state, me) => return Err("You have no project repo, so this grill has no docs PR. Call grill_end again without `docs_pr`.".into()),
                Some(u) => match crate::github::extract_pr_links(u).into_iter().next() {
                    Some((slug, n)) => Some(format!("https://github.com/{slug}/pull/{n}")),
                    None => return Err("docs_pr must look like https://github.com/<owner>/<repo>/pull/<number>".into()),
                },
            };
            if !has_repo(state, me) {
                s.notes_path = write_notes(state, &s, summary);
            }
            let content = match &docs_pr {
                Some(url) if !summary.contains(url.as_str()) => format!("{summary}\n\n{}: {url}", say(&s, "Docs PR", "PR de docs")),
                _ => summary.to_string(),
            };
            let card = end_card(state, me, &s);
            add_msg(state, &me.id, None, "assistant", "text", &content, json!({ "grill": { "session_id": s.id, "role": "summary", "final": true, "notes_path": s.notes_path, "docs_pr": docs_pr, "end_card": card } }));
            s.summarized = true;
            if s.status == "ended" {
                save(state, &mut s)?;
            } else {
                finish(state, &mut s, "done")?;
            }
            Ok(format!(
                "Grill ended; the summary is in the chat{}{}. End your turn now without any reply text.",
                s.notes_path.as_deref().map(|p| format!(" and the full record is in `{p}` in your workspace")).unwrap_or_default(),
                if card.is_some() { ", with the end-of-planning card (spec and ticket links, Clear context, and the team check)" } else { "" }
            ))
        }
        _ => Err(format!("unknown tool {name}")),
    }))
}

/// The system-prompt section every agent gets: what Planning overrides and what to do with a direct
/// request for issues. The grill's own instructions come with the grill (`brief`).
pub fn prompt_section() -> &'static str {
    r#"

# Planning: precedence
Loft's Planning instructions (the ones below, those Loft sends when a grill starts, and `mcp__loft__planning_guide`) override your standing instructions on how specs and tickets are written, labelled, linked and published (for example an `epic` label, "Part of #N" checklists, or ticket order kept in memory). Your standing instructions still set your role, your team and how work is implemented.

# Planning: direct requests
When the user asks for an issue or a plan without a grill ("create an issue for X"):
- Never start a grill unasked. Ask at most one short question in normal chat when something essential is missing; otherwise go ahead with sensible choices.
- Before writing a spec or tickets, call `mcp__loft__planning_guide` for their templates and rules, and follow it.
- One slice (a color, a copy change, one small behaviour): publish it as a single standalone ticket with `mcp__loft__publish_tickets` and no `spec`, then reply with its link.
- Several slices: write a spec from the conversation and show the tickets in chat (titles, what each delivers, blockers), asking "Publish these N tickets?". Only after the user's yes, publish the spec with `mcp__loft__publish_spec`, then the tickets.
- When the request looks big (many decisions still open), suggest a grill in one line ("This has a lot of open decisions: type `/grill <topic>` and I'll grill you on it."). Only the user starts a grill: Loft then sends you its instructions.
- Work the user sends you as a task stays a task: don't open issues for it."#
}

/// What a grill needs the agent to know, sent with the first grill run in a context (see `with_brief`).
const BRIEF: &str = r#"[Loft: Planning instructions for this grill. Follow them until the grill ends.]

# Planning: grilling
The user asked to be grilled on an idea: interview them relentlessly until you share an understanding of what to build, instead of replying normally.
- Design tree: lay out the decisions the idea needs as a tree, where a decision depends on the ones above it. Walk every branch until each decision is resolved.
- Frontier: each round asks every question whose prerequisites are settled, and nothing that depends on an open question; that waits for a later round. Post a round with `mcp__loft__grill_round`, most fundamental question first, one decision per question.
- Recommendations: give every question your recommended answer, and word the question so that "yes" accepts it ("Use Postgres for storage?", not "Which database?"). Give every question a `speak` version too: short and plain, no Markdown, links, ids or PR numbers, recommendation included.
- Start from what's written: before the first round, read the project's existing documents (README, `GLOSSARY.md`, `docs/`, `docs/adr/`, `CLAUDE.md`, notes, earlier specs) and take them as facts. Never ask what they already answer.
- Facts: never ask the user for a fact you can look up. Before each round, find facts (in the code, the docs, the project's history, the web) with your own sub-agents, several in parallel, inside the run. Don't message other agents on your team for this unless the user says so ("ask Dev").
- Decisions are the user's: ask for preferences, priorities and trade-offs, grounded in the facts you found.
- Ask in the language the user used (pass it as `language` in the first round), and write the summary in it too.
- Loft holds the answers until the user sends the round, then wakes you once with all of them. Update the tree and post the next round. An answer is one of: a decision; "left to you" (your recommendation is the decision); a comment the user wants to discuss (explain what they asked in the next round's question and ask again, split up if it helps); or none. Unanswered questions are carried into the next round unchanged by Loft: don't ask them again. When every branch is resolved, finish with `mcp__loft__grill_end` and a summary: decisions, open questions, what was left unanswered (with a project repo, offer a spec first: see Planning: spec).
- What the user types in chat while a round is open is ordinary chat, not an answer: reply briefly. The round stays open; don't post another one or repeat its questions.
- Never mention, list or repeat questions in your own replies: rounds and the summary appear in the chat by themselves. After a grill tool, end your turn with no text.
- The user can pause, resume, stop or ask for a summary so far; Loft handles that. You only hear about it when you have something to do: a summary so far (`mcp__loft__grill_summary`) or a stopped grill (`mcp__loft__grill_end`).

# Planning: docs
While you grill, build the project's language and record its hard decisions.
- Domain modeling: challenge every term the user uses against `GLOSSARY.md`; when a word is fuzzy or overloaded ("account", "job"), propose one precise term and ask about it in the round. Test terms and relations with concrete scenarios ("a Lead leaves the project: who leads?"), and check them against the code. When the user's words and the code disagree, ask which one is right.
- Glossary: write a term into `GLOSSARY.md` as soon as it is resolved, not at the end. Only terms that mean something to the user's domain, not implementation details; define what the thing is, in one or two sentences, in the glossary's language. Keep its format:

  ```md
  # <Project>

  <One sentence on what the project is.>

  ## <Group>

  **<Term>**:
  <What it is.>
  _Avoid_: <synonyms not to use>
  ```

- ADRs: offer an ADR, as a round question, only for a decision that is all three: hard to reverse, surprising to someone reading the code later without the context, and the result of a real trade-off. Everything else is a decision in the summary, not an ADR. Write accepted ones as `docs/adr/NNNN-<slug>.md`, numbered after the highest existing one:

  ```md
  # <The decision, as a short statement>

  Status: accepted (<YYYY-MM-DD>)

  ## Context
  <The forces and the facts that made this a choice.>

  ## Decision
  <What was decided, and what was rejected.>

  ## Consequences
  - <What gets easier, what gets harder, what was accepted.>
  ```

- Where: only when your project has a repo. Work in your clone in your workspace, never in the project folder: on the first resolved term or decision, pull the default branch and create `docs/<topic>` (a short slug of the grill's topic), and commit each glossary or ADR change on it as you go.
- If the repo has no `CLAUDE.md`, the docs branch adds a minimal one that tells agents to read `GLOSSARY.md` and `docs/adr/` and use their terms. Never touch an existing `CLAUDE.md`.
- One PR at the end: before `mcp__loft__grill_end` (also when the user stops the grill), push the branch and open one PR with `gh pr create` (title "Docs: <topic>", body listing the terms and ADRs it adds), and pass its URL as `docs_pr`. Loft shows it as a PR card. A grill that resolved no terms or decisions has no branch and opens no PR.

{SPEC}

# Planning: new project
When your project folder has no git repo or no GitHub remote yet (Loft says so when the grill starts):
- The first round also asks "Create the GitHub repo `<owner>/<project>` and publish this folder?". Call `mcp__loft__preview_new_repo` first: it gives the files and total size of the first commit, the owners the user can create it under (their account and the organizations their token can create repos in) and the question, with the owners as options and private as the recommendation. Ask it in the user's language, keeping the file list and the size.
- Read the project folder in place to learn from it, but never write into it, and never run git there: only Loft does.
- After the user's yes, in a round they sent, call `mcp__loft__create_repo` with that question's id and the owner they chose (public only if they asked for it). Loft writes a `.gitignore` for `.DS_Store` and `.remember/`, runs `git init`, commits everything else, creates the repo and pushes. Then clone it into your workspace for your docs branch.
- If they decline, keep grilling: there is no docs branch until the repo exists. Before publishing the spec, ask the question again in a round, and publish only after a yes and the repo is created. If they decline again, finish the grill without a spec.

{TICKETS}

# Planning: the end
- When a grill published a spec or tickets, `mcp__loft__grill_end` posts the end-of-planning card with your summary: the links, a Clear context button that leaves the next message ("Implement spec #N: hand the ready tickets to the team") in the user's composer, and, when your project has no Dev or no reviewer, a button that asks `loft` to set up the team. Don't repeat the links or the next step in the summary.
- Never create agents yourself, not even when the team is missing a role: agent creation stays with `loft`, through that button.
- When Loft tells you your context is near the end of the smart zone before the tickets are published, suggest `/compact` to the user in one line."#;

/// The spec and ticket rules, in the brief and from `planning_guide` for a direct request.
pub const SPEC: &str = r#"# Planning: spec
At the end of a grill, turn what was decided into a spec, published as a GitHub issue the user owns. A project with no GitHub repo (no git, no GitHub `origin`, or GitHub not connected) keeps its plan in Loft instead: the spec and tickets become Loft issues, and nothing goes to GitHub.
- Offer it: when every branch is resolved, the last round asks "Turn this into a spec and tickets?" (recommendation: yes). If the user says no, finish the grill as usual.
- Test seams: before writing the spec, put how it will be tested to the user as a round question with your recommendation: the seams the tests drive (an API, the tools, the UI) and what they assert on, following the prior art in the repo's tests ("Test it through the tools and the API, with a fake GitHub, like the existing grill tests?").
- Write it in the glossary's terms, respecting the existing ADRs, in the user's language, with this template:

  ```md
  ## Problem Statement
  <The problem, from the user's side.>

  ## Solution
  <What gets built, from the user's side.>

  ## User Stories
  1. As a <actor>, I want <feature>, so that <benefit>.

  ## Implementation Decisions
  - <The modules built or changed, their interfaces, schema and API changes, architectural decisions. No file paths or code: they go stale.>

  ## Testing Decisions
  - <What makes a good test here (behaviour from outside, not internals), the seams agreed in the round, prior art.>

  ## Out of Scope
  - <What this spec doesn't cover.>

  ## Further Notes
  - <Anything else: the glossary terms and ADRs it relies on.>
  ```

- Publish it with `mcp__loft__publish_spec` (title: what gets built, in a few words). Loft creates it on your project's repo with the `spec` label, assigned to the user, and creates missing labels. Without a GitHub repo it saves it as a Loft issue labelled `spec` and gives you its id. A retry with the same title returns the spec already published, so after a failure call it again rather than changing the title. Never create issues with `gh`.
- When it fails (GitHub refused it, for example), tell the user in one or two sentences what went wrong and what to fix, in their language, and keep the grill open so you can publish once it's fixed.
- Outside a grill, when the user asks for a spec from your conversation, write it the same way and publish it with the same tool."#;
pub const TICKETS: &str = r#"# Planning: tickets
Once the spec is published, split it into tickets the team can pick up.
- Vertical slices: each ticket is a thin tracer bullet through every layer it touches (data, API, UI, tests), verifiable on its own. Never one ticket per layer; prefer several thin slices to one thick one.
- Wide refactors go expand–contract: a first ticket adds the new way beside the old one, the next ones move the callers over, a last one removes the old way, so every ticket lands green.
- Blockers: a ticket is blocked by another only when it can't start before that one lands. Keep the chains short so the team can work in parallel.
- Approve first: put the breakdown to the user as one round question, "Publish these N tickets?" (recommendation: yes), listing every ticket's title, what it delivers and what blocks it. Publish nothing before their yes; if they want changes, change the breakdown and ask again.
- Write each ticket in the glossary's terms, in the user's language, with this template (no Parent for a standalone ticket; don't list blockers, Loft links them):

  ```md
  ## Parent

  #<spec number>

  ## What to build

  <The slice end to end, from the user's side.>

  ## Acceptance criteria

  - [ ] <A behaviour someone can check.>
  ```

- Publish them with `mcp__loft__publish_tickets`: the spec's number as `spec` (or the Loft issue id `publish_spec` gave, for a plan kept in Loft), and every ticket with `blocked_by` naming the titles of the tickets that block it. Loft creates them blockers first as sub-issues of the spec, labelled `ready-for-agent` and assigned to the user, and links the blockers as GitHub dependencies. Without a GitHub repo they become Loft issues under the spec, with the blockers as their dependencies. After a failure, tell the user what went wrong and call it again with the same tickets: what's published is skipped. Never create issues with `gh`."#;

/// The grill instructions in full.
pub fn brief() -> String {
    BRIEF.replace("{SPEC}", SPEC).replace("{TICKETS}", TICKETS)
}

/// `prompt` for a grill run, with the brief in front unless this context already has it. A new
/// session, `/clear` and a compaction (`grill_briefed` cleared) each lose it.
pub fn with_brief(state: &AppState, agent_id: &str, prompt: String) -> String {
    let mut fresh = false;
    let _ = state.db.update_agent(agent_id, |a| {
        fresh = !a.grill_briefed;
        a.grill_briefed = true;
    });
    if fresh { format!("{}\n\n{prompt}", brief()) } else { prompt }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tasks::tests::{agent, state};

    fn q(text: &str) -> Value {
        json!({ "text": text, "speak": text, "recommendation": format!("rec for {text}") })
    }

    /// A saved agent, ready to grill.
    pub(crate) fn planner(st: &AppState, name: &str) -> Agent {
        agent(st, name)
    }

    pub(crate) fn round(st: &AppState, a: &Agent, qs: &[&str]) -> Result<String, String> {
        mcp_call(st, a, "grill_round", &json!({ "topic": "Pricing", "questions": qs.iter().map(|t| q(t)).collect::<Vec<_>>() })).unwrap()
    }

    fn answer(st: &AppState, a: &Agent, qid: &str, text: &str) -> Routed {
        let s = active(st, &a.id).unwrap();
        on_user_message(st, a, text, Some(&json!({ "role": "answer", "session_id": s.id, "question_id": qid })), json!({})).unwrap()
    }

    pub(crate) fn send(st: &AppState, a: &Agent) -> Option<String> {
        let mut s = active(st, &a.id).unwrap();
        apply_command(st, &mut s, "send", None, None).unwrap()
    }

    fn qids(st: &AppState, a: &Agent) -> Vec<String> {
        active(st, &a.id).unwrap().rounds.last().unwrap().questions.iter().map(|q| q.id.clone()).collect()
    }

    #[test]
    fn concurrent_answers_are_all_kept_and_a_late_one_never_reopens_a_sent_round() {
        let st = state();
        let o = planner(&st, "Orchestrator");
        let texts: Vec<String> = (0..10).map(|i| format!("Q{i}?")).collect();
        round(&st, &o, &texts.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        let ids = qids(&st, &o);
        let sid = active(&st, &o.id).unwrap().id;
        // Ten answers at once, one per card, as quick taps or the web and the phone together would send them.
        let barrier = std::sync::Barrier::new(ids.len());
        std::thread::scope(|sc| {
            for (i, qid) in ids.iter().enumerate() {
                let (st, o, sid, barrier) = (&st, &o, &sid, &barrier);
                sc.spawn(move || {
                    barrier.wait();
                    on_user_message(st, o, &format!("answer {i}"), Some(&json!({ "role": "answer", "session_id": sid, "question_id": qid })), json!({})).ok().unwrap();
                });
            }
        });
        let s = active(&st, &o.id).unwrap();
        let got: Vec<Option<String>> = s.rounds[0].questions.iter().map(|q| q.answer.clone()).collect();
        assert_eq!(got, (0..10).map(|i| Some(format!("answer {i}"))).collect::<Vec<_>>());

        // A second round, answered while the user sends it: each answer either lands before the send or is refused.
        send(&st, &o);
        mcp_call(&st, &o, "grill_round", &json!({ "questions": texts.iter().map(|t| q(t)).collect::<Vec<_>>() })).unwrap().unwrap();
        let ids = qids(&st, &o);
        let barrier = std::sync::Barrier::new(ids.len() + 1);
        let kept: Vec<(String, bool)> = std::thread::scope(|sc| {
            let (st, o, barrier) = (&st, &o, &barrier);
            sc.spawn(move || {
                barrier.wait();
                on_user_message(st, o, "send", None, json!({})).ok().unwrap();
            });
            let answers: Vec<_> = ids
                .iter()
                .map(|qid| {
                    let sid = &sid;
                    sc.spawn(move || {
                        barrier.wait();
                        let ok = on_user_message(st, o, "yes", Some(&json!({ "role": "answer", "session_id": sid, "question_id": qid })), json!({})).is_ok();
                        (qid.clone(), ok)
                    })
                })
                .collect();
            answers.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let s = active(&st, &o.id).unwrap();
        let r = &s.rounds[1];
        assert!(r.sent_at.is_some(), "a late answer must not reopen the sent round");
        for (qid, ok) in kept {
            let q = r.questions.iter().find(|q| q.id == qid).unwrap();
            assert_eq!(q.answer.is_some(), ok, "an accepted answer is kept, a refused one isn't");
        }
    }

    #[tokio::test]
    async fn every_agent_can_grill() {
        let st = state();
        let o = agent(&st, "Anyone");
        assert!(brief().contains("# Planning: grilling") && !prompt_section().contains("# Planning: grilling"));
        assert_eq!(mcp_call(&st, &o, "remember", &json!({})), None);
        let r = on_user_message(&st, &o, "grill me on pricing", Some(&json!({ "role": "start_request" })), json!({})).unwrap();
        assert!(matches!(r, Routed::Handled { .. }));
        assert_eq!(active(&st, &o.id).unwrap().status, "preparing");
        assert!(round(&st, &o, &["Who pays?"]).is_ok());
    }

    #[test]
    fn the_old_grill_tools_are_gone() {
        let names: Vec<String> = mcp_tools().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["grill_round", "grill_state", "grill_summary", "grill_end"]);
        let st = state();
        let o = planner(&st, "Lead");
        for old in ["grill_start", "grill_ask", "grill_withdraw", "grill_record"] {
            assert!(mcp_call(&st, &o, old, &json!({})).unwrap().is_err(), "{old}");
        }
    }

    #[test]
    fn questions_are_validated() {
        let rec = |v: Value| { let mut v = v; v["recommendation"] = json!("r"); v };
        assert!(parse_questions(&json!([rec(json!({ "text": "" }))])).is_err());
        assert!(parse_questions(&json!([rec(json!({ "text": "x".repeat(501) }))])).is_err());
        assert!(parse_questions(&json!([rec(json!({ "text": "a", "kind": "essay" }))])).is_err());
        assert!(parse_questions(&json!([rec(json!({ "text": "a", "kind": "single_choice" }))])).is_err());
        assert!(parse_questions(&json!([rec(json!({ "text": "a", "kind": "single_choice", "options": ["1", "2", "3", "4", "5", "6", "7"] }))])).is_err());
        assert!(parse_questions(&json!([{ "text": "a" }])).unwrap_err().contains("recommendation"), "every question carries a recommendation");
        let ok = parse_questions(&json!([rec(json!({ "text": "a", "kind": "yes_no" })), rec(json!({ "text": "b" }))])).unwrap();
        assert_eq!((ok[0].kind.as_str(), ok[1].kind.as_str(), ok[0].state.as_str()), ("yes_no", "free_text", "open"));
        let st = state();
        let o = planner(&st, "Lead");
        let many: Vec<String> = (0..=MAX_ROUND).map(|i| format!("Q{i}?")).collect();
        assert!(round(&st, &o, &many.iter().map(String::as_str).collect::<Vec<_>>()).is_err());
    }

    #[test]
    fn posting_a_round_puts_every_question_in_the_chat_with_its_recommendation() {
        let st = state();
        let o = planner(&st, "Lead");
        let r = round(&st, &o, &["Charge monthly?", "Offer a free trial?"]).unwrap();
        assert!(r.contains("Round 1 posted") && r.contains("End your turn"), "{r}");
        let msgs = st.db.list_messages(&o.id, 50, None).unwrap();
        let posted: Vec<_> = msgs.iter().filter(|m| m.meta["grill"]["role"] == "round").collect();
        assert_eq!(posted.len(), 1, "one message per round");
        let m = posted[0];
        assert_eq!((m.role.as_str(), m.meta["grill"]["round"].clone()), ("assistant", json!(1)));
        assert!(m.content.contains("Charge monthly?") && m.content.contains("_Recommendation:_ rec for Offer a free trial?"), "{}", m.content);
        let qs = m.meta["grill"]["questions"].as_array().unwrap();
        assert_eq!((qs.len(), qs[1]["recommendation"].clone()), (2, json!("rec for Offer a free trial?")));
        let v = view(&active(&st, &o.id).unwrap());
        assert_eq!((v["state"].clone(), v["round"].clone(), v["total"].clone(), v["answered"].clone(), v["waiting"].clone()), (json!("active"), json!(1), json!(2), json!(0), json!(false)));
        assert_eq!(v["language"], json!("en"));
        let rows: Vec<String> = msgs.iter().filter(|m| m.kind == "notice").map(|m| m.content.clone()).collect();
        assert_eq!(rows, ["Grill started: Pricing"]);
        // A second round can't go up while the user is still answering the first.
        assert!(round(&st, &o, &["More?"]).unwrap_err().contains("Round 1 is still waiting"));
    }

    #[test]
    fn answers_are_held_without_waking_the_agent_and_sending_wakes_it_once_with_all_of_them() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["Charge monthly?", "Offer a free trial?", "Show prices in euros?"]).unwrap();
        let ids = qids(&st, &o);
        let before = st.db.list_messages(&o.id, 50, None).unwrap().len();

        // "yes" accepts the recommendation; other text is the answer; either can change before sending.
        let Routed::Handled { message, wake } = answer(&st, &o, &ids[0], "Yes") else { panic!() };
        assert!(message.is_none() && wake.is_none(), "an answer doesn't run the agent");
        let Routed::Handled { wake, .. } = answer(&st, &o, &ids[1], "Only 7 days") else { panic!() };
        assert!(wake.is_none());
        answer(&st, &o, &ids[1], "Only 14 days");
        let s = active(&st, &o.id).unwrap();
        let qs = &s.rounds[0].questions;
        assert_eq!((qs[0].state.as_str(), qs[0].accepted, qs[0].answer.as_deref()), ("answered", true, Some("rec for Charge monthly?")));
        assert_eq!((qs[1].accepted, qs[1].answer.as_deref()), (false, Some("Only 14 days")));
        assert_eq!(view(&s)["answered"], json!(2));
        assert_eq!(st.db.list_messages(&o.id, 50, None).unwrap().len(), before, "answers are held on the session, not posted");

        // Sending wakes the agent once, with every answer of the round; the unanswered one is carried.
        let wake = send(&st, &o).expect("sending wakes the agent");
        assert!(wake.contains("the user sent round 1") && wake.contains("accepted the recommendation") && wake.contains("\"Only 14 days\"") && wake.contains("no answer"), "{wake}");
        assert!(wake.contains("grill_round") && wake.contains("grill_end"));
        let s = active(&st, &o.id).unwrap();
        assert_eq!(s.rounds[0].questions[2].state, "carried");
        assert_eq!(view(&s)["waiting"], json!(true));
        let sent = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["role"] == "answers").unwrap();
        assert_eq!(sent.role, "user");
        assert!(sent.content.contains("1. Yes (rec for Charge monthly?)") && sent.content.contains("2. Only 14 days") && sent.content.contains("3. (no answer)"), "{}", sent.content);

        // The round is closed: answers and a second send are refused.
        let late = json!({ "role": "answer", "session_id": s.id, "question_id": ids[0] });
        assert_eq!(on_user_message(&st, &o, "No", Some(&late), json!({})).err().map(|e| (e.0, e.1)), Some((StatusCode::CONFLICT, "question_closed".into())));
        assert_eq!(apply_command(&st, &mut active(&st, &o.id).unwrap(), "send", None, None).err().map(|e| e.1), Some("round_closed".into()));

        // The next round appears in the chat, and its answers are new.
        assert!(round(&st, &o, &["Annual discount of 20%?"]).unwrap().contains("Round 2"));
        let s = active(&st, &o.id).unwrap();
        assert_eq!((view(&s)["round"].clone(), view(&s)["waiting"].clone(), view(&s)["answered"].clone()), (json!(2), json!(false), json!(0)));
        assert_eq!(st.db.list_messages(&o.id, 50, None).unwrap().iter().filter(|m| m.meta["grill"]["role"] == "round").count(), 2);
    }

    fn rows(st: &AppState, a: &Agent) -> Vec<String> {
        st.db.list_messages(&a.id, 100, None).unwrap().into_iter().filter(|m| m.kind == "notice").map(|m| m.content).collect()
    }

    #[test]
    fn typed_text_is_chat_not_an_answer_and_typed_commands_work() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?", "B?", "C?"]).unwrap();
        let qs = |st: &AppState| active(st, &o.id).unwrap().rounds[0].questions.clone();

        // A side remark (or a bare "yes") in the composer goes to the agent as chat; no card changes unseen.
        for text in ["wait, what does A mean?", "sim", "answer: skip"] {
            assert!(matches!(on_user_message(&st, &o, text, None, json!({})).unwrap(), Routed::Normal(_)), "{text:?}");
        }
        assert!(qs(&st).iter().all(|q| q.state == "open"));
        // "I don't know" leaves the first open question to the agent, and says so in the chat.
        let Routed::Handled { message, wake } = on_user_message(&st, &o, "I don't know", None, json!({})).unwrap() else { panic!() };
        assert_eq!((message.unwrap().meta["grill"]["command"].clone(), wake), (json!("decide"), None));
        assert_eq!((qs(&st)[0].state.as_str(), qs(&st)[0].answer.as_deref()), ("decided", Some("rec for A?")));
        assert_eq!(rows(&st, &o).last().unwrap(), "Question 1 left to the agent: rec for A?");
        // Skip, back and repeat only mean something by voice: the chat says so and the round is untouched.
        for text in ["skip", "go back", "repeat"] {
            let Routed::Handled { message, wake } = on_user_message(&st, &o, text, None, json!({})).unwrap() else { panic!() };
            assert!(message.is_some() && wake.is_none());
            assert!(rows(&st, &o).last().unwrap().starts_with("Skip, back and repeat step through questions by voice"), "{text:?}");
        }
        assert_eq!(qs(&st).iter().filter(|q| q.state == "open").count(), 2);
        // "send it" sends the round.
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "send it", None, json!({})).unwrap() else { panic!() };
        assert!(wake.unwrap().contains("left it to you"));
        // Tagged answers still need their question.
        let s = active(&st, &o.id).unwrap();
        assert_eq!(on_user_message(&st, &o, "x", Some(&json!({ "role": "answer", "session_id": s.id })), json!({})).err().map(|e| e.0), Some(StatusCode::BAD_REQUEST));
    }

    /// A PNG saved in `a`'s uploads, as `POST /agents/{id}/uploads` would.
    fn upload(st: &AppState, a: &Agent) -> String {
        let file = format!("{}.png", uuid::Uuid::new_v4());
        let d = crate::uploads::dir(st, &a.id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(&file), b"\x89PNG\r\n\x1a\n").unwrap();
        file
    }

    /// A grill answer sent the way the cards send it, through `POST /agents/{id}/messages`.
    async fn answer_with(st: &AppState, a: &Agent, qid: &str, text: &str, images: &[String], discuss: bool) -> Result<(), StatusCode> {
        let s = active(st, &a.id).unwrap();
        let body = json!({ "text": text, "images": images, "meta": { "grill": { "role": "answer", "session_id": s.id, "question_id": qid, "discuss": discuss } } });
        crate::api::send_message(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone()), axum::Json(serde_json::from_value(body).unwrap()))
            .await
            .map(|_| ())
            .map_err(|e| e.0)
    }

    #[tokio::test]
    async fn answers_carry_images_to_the_agent_with_the_round() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["Which layout?", "Which logo?", "Which colour?"]).unwrap();
        let ids = qids(&st, &o);
        let (shot, logo, mock) = (upload(&st, &o), upload(&st, &o), upload(&st, &o));
        // Text with an image, an image alone, and an image alone as a comment to discuss.
        answer_with(&st, &o, &ids[0], "Like this one", &[shot.clone()], false).await.unwrap();
        answer_with(&st, &o, &ids[1], "", &[logo.clone()], false).await.unwrap();
        answer_with(&st, &o, &ids[2], "  ", &[mock.clone()], true).await.unwrap();
        let qs = active(&st, &o.id).unwrap().rounds[0].questions.clone();
        assert_eq!((qs[0].state.as_str(), qs[0].answer.as_deref(), qs[0].images.clone()), ("answered", Some("Like this one"), vec![shot.clone()]));
        assert_eq!((qs[1].state.as_str(), qs[1].answer.as_deref(), qs[1].images.clone()), ("answered", Some(""), vec![logo.clone()]));
        assert_eq!((qs[2].state.as_str(), qs[2].images.clone()), ("discuss", vec![mock.clone()]));
        // The round lists each answer's image paths and carries the images as chat images do.
        let wake = send(&st, &o).unwrap();
        assert!(wake.contains(&format!("\"Like this one\", with images: uploads/{shot}")), "{wake}");
        assert!(wake.contains(&format!("\"\", with images: uploads/{logo}")), "{wake}");
        assert!(wake.contains(&format!("wants to discuss first: \"\", with images: uploads/{mock}")), "{wake}");
        for f in [&shot, &logo, &mock] {
            assert!(wake.contains(&format!("[loft-image:{f}]")), "{wake}");
        }
        assert!(crate::uploads::stdin_line(&st, &o.id, &wake).unwrap().contains("\"type\":\"image\""));
        // The chat's copy of the answers says how many images each had.
        let posted = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["role"] == json!("answers")).unwrap();
        assert!(posted.content.contains("1. Like this one [1 image]\n2. [1 image]\n3. Let's discuss: [1 image]"), "{}", posted.content);
    }

    #[tokio::test]
    async fn grill_answers_refuse_images_like_chat_messages() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["Which layout?"]).unwrap();
        let ids = qids(&st, &o);
        let other = planner(&st, "Dev");
        for images in [vec!["../../etc/passwd".to_string()], vec![format!("{}.png", uuid::Uuid::new_v4())], vec![upload(&st, &other)]] {
            assert_eq!(answer_with(&st, &o, &ids[0], "see image", &images, false).await, Err(StatusCode::BAD_REQUEST), "{images:?}");
        }
        assert_eq!(answer_with(&st, &o, &ids[0], "", &[], false).await, Err(StatusCode::BAD_REQUEST), "no text and no images");
        assert_eq!(active(&st, &o.id).unwrap().rounds[0].questions[0].state, "open");
        // A new answer replaces the images; Accept and Decide for me drop them.
        let f = upload(&st, &o);
        answer_with(&st, &o, &ids[0], "this", &[f], false).await.unwrap();
        answer(&st, &o, &ids[0], "yes");
        assert!(active(&st, &o.id).unwrap().rounds[0].questions[0].images.is_empty());
    }

    #[test]
    fn discuss_answers_reach_the_agent_as_comments_not_decisions() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["Charge monthly?", "Offer a trial?"]).unwrap();
        let ids = qids(&st, &o);
        let s = active(&st, &o.id).unwrap();
        let tag = json!({ "role": "answer", "session_id": s.id, "question_id": ids[0], "discuss": true });
        // Even "yes" is a comment when the user chose to discuss.
        on_user_message(&st, &o, "yes, but what about teams?", Some(&tag), json!({})).unwrap();
        let q = &active(&st, &o.id).unwrap().rounds[0].questions[0];
        assert_eq!((q.state.as_str(), q.accepted, q.answer.as_deref()), ("discuss", false, Some("yes, but what about teams?")));
        assert_eq!(view(&active(&st, &o.id).unwrap())["answered"], json!(1));
        // A plain answer afterwards turns it back into a decision.
        answer(&st, &o, &ids[1], "No trial");
        let wake = send(&st, &o).unwrap();
        assert!(wake.contains("wants to discuss first: \"yes, but what about teams?\"") && wake.contains("ask again, split into smaller questions"), "{wake}");
        let sent = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["role"] == "answers").unwrap();
        assert!(sent.content.contains("1. Let's discuss: yes, but what about teams?"), "{}", sent.content);
        // Discussed questions are the agent's to ask again: Loft doesn't carry them.
        round(&st, &o, &["Per-seat pricing for teams?"]).unwrap();
        let texts: Vec<String> = active(&st, &o.id).unwrap().rounds[1].questions.iter().map(|q| q.text.clone()).collect();
        assert_eq!(texts, ["Per-seat pricing for teams?"]);
    }

    #[test]
    fn a_partial_send_carries_open_questions_into_the_next_round_unchanged() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?", "B?", "C?"]).unwrap();
        let ids = qids(&st, &o);
        answer(&st, &o, &ids[0], "yes");
        let wake = send(&st, &o).unwrap();
        assert!(wake.contains("Loft asks it again in the next round") && wake.contains("don't ask them again"), "{wake}");

        let r = round(&st, &o, &["D?"]).unwrap();
        assert!(r.contains("2 carried over unanswered"), "{r}");
        let s = active(&st, &o.id).unwrap();
        let qs = &s.rounds[1].questions;
        let got: Vec<(&str, &str, &str, Option<u32>)> = qs.iter().map(|q| (q.text.as_str(), q.recommendation.as_str(), q.state.as_str(), q.from_round)).collect();
        assert_eq!(got, [("B?", "rec for B?", "open", Some(1)), ("C?", "rec for C?", "open", Some(1)), ("D?", "rec for D?", "open", None)]);
        assert!(qs.iter().all(|q| !ids.contains(&q.id)), "carried questions get new ids in the new round");
        let posted = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().filter(|m| m.meta["grill"]["role"] == "round").last().unwrap();
        assert_eq!(posted.meta["grill"]["questions"][0]["from_round"], json!(1));
        assert!(posted.content.contains("1. B?") && posted.content.contains("3. D?"), "{}", posted.content);

        // Carried again, they keep the round they were first asked in; the agent may drop one, or post only carried ones.
        send(&st, &o);
        let c = active(&st, &o.id).unwrap().rounds[1].questions[1].id.clone();
        mcp_call(&st, &o, "grill_round", &json!({ "questions": [], "drop": [c] })).unwrap().unwrap();
        let got: Vec<(String, Option<u32>)> = active(&st, &o.id).unwrap().rounds[2].questions.iter().map(|q| (q.text.clone(), q.from_round)).collect();
        assert_eq!(got, [("B?".to_string(), Some(1)), ("D?".to_string(), Some(2))]);
        // The round limit counts carried questions.
        send(&st, &o);
        let many: Vec<String> = (0..MAX_ROUND - 1).map(|i| format!("Q{i}?")).collect();
        let e = round(&st, &o, &many.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_err();
        assert!(e.contains("2 carry over unanswered: post at most 10 new ones"), "{e}");
    }

    #[test]
    fn decide_for_me_records_the_recommendation_and_shows_it_in_chat() {
        let st = state();
        let o = planner(&st, "Lead");
        mcp_call(&st, &o, "grill_round", &json!({ "topic": "Preços", "language": "pt", "questions": [q("Cobrar por mês?"), q("Oferecer teste grátis?")] })).unwrap().unwrap();
        let ids = qids(&st, &o);
        let mut s = active(&st, &o.id).unwrap();
        // The card's button names its question.
        assert!(apply_command(&st, &mut s, "decide", Some(&ids[1]), None).unwrap().is_none(), "deciding doesn't run the agent");
        let q = &active(&st, &o.id).unwrap().rounds[0].questions[1];
        assert_eq!((q.state.as_str(), q.answer.as_deref()), ("decided", Some("rec for Oferecer teste grátis?")));
        let row = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().filter(|m| m.kind == "notice").last().unwrap();
        assert_eq!(row.content, "Pergunta 2 decidida pelo agente: rec for Oferecer teste grátis?");
        assert_eq!((row.meta["grill"]["state"].clone(), row.meta["grill"]["question_id"].clone()), (json!("decided"), json!(ids[1])));
        // Deciding it again (the voice went back to it) succeeds and leaves it as it was.
        let notices = |st: &AppState| st.db.list_messages(&o.id, 50, None).unwrap().into_iter().filter(|m| m.kind == "notice").count();
        let (before, at) = (notices(&st), q.answered_at.clone());
        assert!(apply_command(&st, &mut active(&st, &o.id).unwrap(), "decide", Some(&ids[1]), None).unwrap().is_none());
        assert_eq!((notices(&st), active(&st, &o.id).unwrap().rounds[0].questions[1].answered_at.clone()), (before, at));
        assert_eq!(apply_command(&st, &mut active(&st, &o.id).unwrap(), "decide", Some("no-such-question"), None).err().map(|e| (e.0, e.1)), Some((StatusCode::CONFLICT, "question_closed".into())));
        let wake = send(&st, &o).unwrap();
        assert!(wake.contains("left it to you: your recommendation is the decision"), "{wake}");
        let sent = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["role"] == "answers").unwrap();
        assert!(sent.content.contains("2. Você decide (rec for Oferecer teste grátis?)"), "{}", sent.content);
        assert_eq!(apply_command(&st, &mut active(&st, &o.id).unwrap(), "decide", Some(&ids[0]), None).err().map(|e| e.1), Some("round_closed".into()));
    }

    #[test]
    fn start_request_prepares_a_session_and_a_new_start_supersedes_the_old_one() {
        let st = state();
        let o = planner(&st, "Lead");
        let start = json!({ "role": "start_request" });
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "Grill me on pricing", Some(&start), json!({})).unwrap() else { panic!() };
        let wake = wake.unwrap();
        assert!(wake.contains("grill_round") && wake.contains("sub-agents"), "{wake}");
        let prep = active(&st, &o.id).unwrap();
        assert_eq!((prep.status.as_str(), view(&prep)["round"].clone()), ("preparing", Value::Null));
        round(&st, &o, &["A?"]).unwrap();
        assert_eq!(active(&st, &o.id).unwrap().id, prep.id, "the first round fills the prepared session");
        // Typed while a round is open, a start request replaces the grill instead of answering it.
        let Routed::Handled { message, .. } = on_user_message(&st, &o, "Grill me about onboarding", None, json!({})).unwrap() else { panic!() };
        assert_eq!(message.unwrap().meta["grill"]["role"], json!("start_request"));
        let old = st.db.get_grill(&prep.id).unwrap().unwrap();
        assert_eq!((old.status.as_str(), old.end_reason.as_deref()), ("ended", Some("superseded")));
        assert!(old.rounds[0].questions[0].answer.is_none());
        assert_eq!(active(&st, &o.id).unwrap().status, "preparing");
    }

    #[test]
    fn pause_resume_and_stop() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?", "B?"]).unwrap();
        let id = active(&st, &o.id).unwrap().id;
        on_user_message(&st, &o, "hold on", None, json!({})).unwrap();
        assert_eq!(active(&st, &o.id).unwrap().status, "paused");
        // A second pause adds nothing; the button resumes, and so does an answer.
        apply_command(&st, &mut active(&st, &o.id).unwrap(), "resume", None, None).unwrap();
        apply_command(&st, &mut active(&st, &o.id).unwrap(), "resume", None, None).unwrap();
        on_user_message(&st, &o, "pause", None, json!({})).unwrap();
        answer(&st, &o, &qids(&st, &o)[0], "Postgres");
        assert_eq!(active(&st, &o.id).unwrap().status, "active");
        assert_eq!(rows(&st, &o), ["Grill started: Pricing", "Paused", "Resumed", "Paused", "Resumed"]);
        // Tagged answers from the app are never re-read as commands.
        answer(&st, &o, &qids(&st, &o)[1], "stop");
        assert_eq!(active(&st, &o.id).unwrap().rounds[0].questions[1].answer.as_deref(), Some("stop"));
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "That's enough", None, json!({})).unwrap() else { panic!() };
        assert!(wake.unwrap().contains("grill_end"));
        assert!(active(&st, &o.id).is_none());
        assert_eq!(st.db.get_grill(&id).unwrap().unwrap().end_reason.as_deref(), Some("stopped"));
        mcp_call(&st, &o, "grill_end", &json!({ "summary": "Postgres." })).unwrap().unwrap();
        assert!(st.db.get_grill(&id).unwrap().unwrap().summarized);
        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "again" })).unwrap().is_err(), "only once");
    }

    #[test]
    fn sending_or_deciding_while_paused_resumes_with_a_row() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?", "B?"]).unwrap();
        on_user_message(&st, &o, "pause", None, json!({})).unwrap();
        on_user_message(&st, &o, "decide for me", None, json!({})).unwrap();
        on_user_message(&st, &o, "pause", None, json!({})).unwrap();
        let Routed::Handled { wake, .. } = on_user_message(&st, &o, "send", None, json!({})).unwrap() else { panic!() };
        assert!(wake.is_some());
        assert_eq!(active(&st, &o.id).unwrap().status, "active");
        assert_eq!(rows(&st, &o), ["Grill started: Pricing", "Paused", "Resumed", "Question 1 left to the agent: rec for A?", "Paused", "Resumed"]);
    }

    #[test]
    fn language_is_guessed_when_the_agent_leaves_it_out() {
        assert_eq!(guess_lang("Me sabatina sobre o nome do próximo recurso. O que é o recurso? Quem vai usar?"), "pt");
        assert_eq!(guess_lang("Grill me on pricing. Who is the main buyer? What should it cost?"), "en");
        let st = state();
        let o = planner(&st, "Lead");
        mcp_call(&st, &o, "grill_round", &json!({ "topic": "Nome do recurso", "questions": [q("Qual é o nome que você prefere?")] })).unwrap().unwrap();
        on_user_message(&st, &o, "pausa", None, json!({})).ok().unwrap();
        let rows: Vec<String> = st.db.list_messages(&o.id, 20, None).unwrap().into_iter().filter(|m| m.kind == "notice").map(|m| m.content).collect();
        assert_eq!(rows, ["Entrevista iniciada: Nome do recurso", "Pausada"]);
        let posted = st.db.list_messages(&o.id, 20, None).unwrap().into_iter().find(|m| m.meta["grill"]["role"] == "round").unwrap();
        assert!(posted.content.starts_with("**Rodada 1**") && posted.content.contains("_Recomendação:_"), "{}", posted.content);
        assert_eq!(view(&active(&st, &o.id).unwrap())["language"], json!("pt"), "the app reads its phrases in it");
    }

    #[test]
    fn idle_grills_pause_then_end_and_still_get_a_summary() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?", "B?"]).unwrap();
        answer(&st, &o, &qids(&st, &o)[0], "a");
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
    fn the_agent_cannot_end_while_a_round_waits() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?"]).unwrap();
        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "x" })).unwrap().unwrap_err().contains("waiting for the user"));
        send(&st, &o);
        assert!(mcp_call(&st, &o, "grill_end", &json!({ "summary": "x" })).unwrap().is_ok());
    }

    #[test]
    fn ending_without_a_repo_posts_the_summary_and_writes_a_notes_file() {
        let st = state();
        let o = planner(&st, "Lead");
        mcp_call(&st, &o, "grill_round", &json!({ "topic": "Pricing / ../../etc", "questions": [q("Charge monthly?"), q("Offer a trial?")] })).unwrap().unwrap();
        answer(&st, &o, &qids(&st, &o)[0], "yes");
        send(&st, &o);
        let r = mcp_call(&st, &o, "grill_end", &json!({ "summary": "Monthly." })).unwrap().unwrap();
        let s = st.db.list_grills(&o.id).unwrap().remove(0);
        assert_eq!((s.status.as_str(), s.end_reason.as_deref()), ("ended", Some("done")));
        let path = s.notes_path.clone().unwrap();
        assert!(path.starts_with("notes/grill/") && path.ends_with("-pricing-etc.md"), "{path}");
        assert!(r.contains(&path));
        let text = std::fs::read_to_string(crate::agents::workspace(&st, &o.id).join(&path)).unwrap();
        assert!(text.contains("Monthly.") && text.contains("## Round 1") && text.contains("### 1. Charge monthly?\n\nAccepted: rec for Charge monthly?") && text.contains("_Not answered_"), "{text}");
        let summary = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["final"] == true).unwrap();
        assert_eq!((summary.content.as_str(), summary.meta["grill"]["notes_path"].clone()), ("Monthly.", json!(path)));
    }

    #[test]
    fn ending_in_a_project_repo_posts_the_summary_without_a_notes_file() {
        let st = state();
        let mut o = planner(&st, "Lead");
        o.project = "homebase".into();
        st.db.save_agent(&o).unwrap();
        let repo = st.settings.boot().projects_dir.join("homebase");
        assert!(repo.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        round(&st, &o, &["A?"]).unwrap();
        send(&st, &o);
        let r = mcp_call(&st, &o, "grill_end", &json!({ "summary": "Done." })).unwrap().unwrap();
        assert!(!r.contains("notes/"), "{r}");
        assert!(st.db.list_grills(&o.id).unwrap()[0].notes_path.is_none());
        assert!(!crate::agents::workspace(&st, &o.id).join("notes").join("grill").exists());
    }

    #[test]
    fn the_brief_goes_with_the_first_grill_run_and_again_once_the_context_lost_it() {
        let st = state();
        let o = planner(&st, "Lead");
        let first = with_brief(&st, &o.id, "round 1".into());
        assert!(first.starts_with("[Loft: Planning instructions for this grill.") && first.ends_with("\n\nround 1"), "{first}");
        assert_eq!(with_brief(&st, &o.id, "round 2".into()), "round 2", "the context already has it");
        // A compaction or a cleared session clears the mark.
        st.db.update_agent(&o.id, |a| a.grill_briefed = false).unwrap();
        assert!(with_brief(&st, &o.id, "round 3".into()).contains("# Planning: grilling"));
    }

    #[test]
    fn typing_grill_me_starts_a_grill_like_the_command() {
        let st = state();
        let o = planner(&st, "Lead");
        let Routed::Handled { message, wake } = on_user_message(&st, &o, "Grill me on pricing", None, json!({})).unwrap() else { panic!() };
        assert_eq!(message.unwrap().meta["grill"]["role"], json!("start_request"));
        assert!(wake.unwrap().contains("grill_round"));
        assert_eq!(active(&st, &o.id).unwrap().status, "preparing");
        // Mentioning a grill later on is ordinary chat.
        let other = planner(&st, "Dev");
        assert!(matches!(on_user_message(&st, &other, "We should grill the users first", None, json!({})).unwrap(), Routed::Normal(_)));
    }

    #[test]
    fn the_planning_prompt_teaches_domain_modeling_and_the_docs_formats() {
        let p = brief();
        for want in [
            "# Planning: docs",
            "Domain modeling",
            "concrete scenarios",
            "**<Term>**:",
            "_Avoid_:",
            "hard to reverse, surprising",
            "real trade-off",
            "docs/adr/NNNN-<slug>.md",
            "## Consequences",
            "never in the project folder",
            "`docs/<topic>`",
            "no `CLAUDE.md`",
            "`gh pr create`",
            "opens no PR",
        ] {
            assert!(p.contains(want), "missing {want:?}");
        }
    }

    #[test]
    fn the_end_of_a_grill_asks_for_the_docs_pr_first() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?"]).unwrap();
        let wake = send(&st, &o).unwrap();
        assert!(wake.contains("docs branch") && wake.contains("`gh pr create`") && wake.contains("open no PR"), "{wake}");
        round(&st, &o, &["B?"]).unwrap();
        let mut s = active(&st, &o.id).unwrap();
        let stopped = apply_command(&st, &mut s, "end", None, None).unwrap().unwrap();
        assert!(stopped.contains("`gh pr create`") && stopped.find("docs_pr") < stopped.find("grill_end"), "{stopped}");
    }

    #[test]
    fn ending_with_a_docs_pr_links_it_in_the_summary() {
        let st = state();
        let mut o = planner(&st, "Lead");
        round(&st, &o, &["A?"]).unwrap();
        send(&st, &o);
        // Without a repo there is nowhere for docs to go.
        let r = mcp_call(&st, &o, "grill_end", &json!({ "summary": "Done.", "docs_pr": "https://github.com/me/homebase/pull/7" })).unwrap();
        assert!(r.unwrap_err().contains("no project repo"));

        o.project = "homebase".into();
        st.db.save_agent(&o).unwrap();
        let repo = st.settings.boot().projects_dir.join("homebase");
        assert!(repo.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let r = mcp_call(&st, &o, "grill_end", &json!({ "summary": "Done.", "docs_pr": "not a pr" })).unwrap();
        assert!(r.unwrap_err().contains("docs_pr must look like"));
        mcp_call(&st, &o, "grill_end", &json!({ "summary": "Done.", "docs_pr": "https://github.com/me/homebase/pull/7" })).unwrap().unwrap();
        let summary = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["final"] == true).unwrap();
        assert_eq!(summary.content, "Done.\n\nDocs PR: https://github.com/me/homebase/pull/7");
        assert_eq!(summary.meta["grill"]["docs_pr"], json!("https://github.com/me/homebase/pull/7"));
    }

    #[test]
    fn a_grill_that_resolved_nothing_ends_without_a_docs_pr() {
        let st = state();
        let o = planner(&st, "Lead");
        round(&st, &o, &["A?"]).unwrap();
        send(&st, &o);
        mcp_call(&st, &o, "grill_end", &json!({ "summary": "Nothing settled." })).unwrap().unwrap();
        let summary = st.db.list_messages(&o.id, 50, None).unwrap().into_iter().find(|m| m.meta["grill"]["final"] == true).unwrap();
        assert_eq!((summary.content.as_str(), summary.meta["grill"]["docs_pr"].clone()), ("Nothing settled.", Value::Null));
    }

    #[tokio::test]
    async fn a_round_sent_near_the_smart_zone_limit_suggests_compact_until_the_tickets_are_out() {
        let st = state();
        let mut o = planner(&st, "Orchestrator");
        round(&st, &o, &["Who pays?"]).unwrap();
        o.context_tokens = Some(COMPACT_AT - 1);
        st.db.save_agent(&o).unwrap();
        assert!(!send(&st, &o).unwrap().contains("/compact"), "below the limit, no suggestion");

        round(&st, &o, &["Monthly?"]).unwrap();
        o.context_tokens = Some(COMPACT_AT + 12_000);
        st.db.save_agent(&o).unwrap();
        let wake = send(&st, &o).unwrap();
        assert!(wake.contains("about 92k tokens") && wake.contains("suggesting they type `/compact`"), "{wake}");

        // Once the tickets are published, the spec is written: no point compacting for it.
        record_published(&st, &o.id, "octo/x", Some(IssueLink { number: 4, title: "Pricing".into(), id: String::new() }), vec![IssueLink { number: 5, title: "Plans".into(), id: String::new() }]);
        round(&st, &o, &["Annual too?"]).unwrap();
        assert!(!send(&st, &o).unwrap().contains("/compact"));
    }
}
