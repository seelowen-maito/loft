//! Recognising grill commands in text the user typed or dictated, for clients that don't tag them
//! (the web composer, old app builds). Deliberately conservative: a message is a command only if the
//! *whole* short utterance is one of the phrases below; anything else is an answer. The app's buttons
//! and its own voice matching send commands explicitly and never go through this.

use std::sync::LazyLock;

use serde_json::{Value, json};

/// Bump when a phrase list changes, so clients refresh their cached copy.
pub const VERSION: &str = "2";
/// Longer messages are always answers.
pub const MAX_WORDS: usize = 6;

/// Commands the server applies.
pub const COMMANDS: &[&str] = &["skip", "back", "pause", "resume", "end", "summarize", "decide", "repeat"];

/// (command, English phrases, Portuguese phrases), already normalised (see `normalize`).
const PHRASES: &[(&str, &[&str], &[&str])] = &[
    (
        "skip",
        &["skip", "skip it", "skip this", "skip this one", "skip question", "skip this question", "pass", "i pass", "next", "next question", "next one", "move on", "go on"],
        &["pula", "pular", "pula ai", "pula essa", "pula esta", "pula essa pergunta", "passa", "passo", "proxima", "proxima pergunta", "segue", "vai pra proxima", "vamos pra proxima"],
    ),
    (
        "back",
        &["back", "go back", "previous", "previous question", "last question", "back one"],
        &["volta", "voltar", "volta uma", "anterior", "pergunta anterior", "volta pra anterior"],
    ),
    (
        "pause",
        &["pause", "pause it", "pause the grill", "hold on", "wait", "later", "lets pause"],
        &["pausa", "pausar", "espera", "espera ai", "depois", "pausa ai", "vamos pausar"],
    ),
    (
        "resume",
        &["resume", "unpause", "lets continue", "continue", "keep going", "carry on"],
        &["retoma", "retomar", "continua", "continuar", "vamos continuar", "bora"],
    ),
    (
        "end",
        &["stop", "end", "end it", "end the grill", "stop the grill", "thats enough", "enough", "were done", "im done", "finish"],
        &["para", "parar", "chega", "chega por hoje", "acabou", "termina", "terminar", "encerra", "encerrar"],
    ),
    (
        "summarize",
        &["summarize", "summarise", "summary", "summary so far", "sum up", "recap", "where are we"],
        &["resumo", "faz um resumo", "resume ai", "resumo ate agora", "onde estamos", "recapitula"],
    ),
    (
        "decide",
        &["i dont know", "dont know", "no idea", "you decide", "decide for me", "your call", "up to you", "you choose"],
        &["nao sei", "sei la", "decide voce", "voce decide", "tanto faz", "voce escolhe", "escolhe voce", "decide ai"],
    ),
    (
        "repeat",
        &["repeat", "repeat that", "repeat the question", "say again", "say that again", "again", "what was the question", "pardon"],
        &["repete", "repetir", "de novo", "repete a pergunta", "qual era a pergunta", "como"],
    ),
];

/// Verb + what is being stopped, in any combination: "stop grill session", "end the interview",
/// "encerra a sessão por hoje". Generated so that natural wordings match without listing each one.
/// (verbs, objects, tails) for one language.
type Combos = (&'static [&'static str], &'static [&'static str], &'static [&'static str]);
/// (command, [English, Portuguese]).
const VERB_PHRASES: &[(&str, [Combos; 2])] = &[
    (
        "end",
        [
            (
                &["stop", "end", "finish", "quit", "exit", "close", "cancel", "wrap up", "stop the", "end the", "finish the", "quit the", "exit the", "close the", "cancel the", "wrap up the", "stop this", "end this", "finish this", "close this", "cancel this", "wrap up this", "done with the", "done with this", "were done with the", "im done with the"],
                &["grill", "grilling", "grill session", "grilling session", "interview", "interview session", "session", "questions", "questioning", "q and a"],
                &["", "for now", "for today"],
            ),
            (
                &["para", "parar", "pare", "encerra", "encerrar", "encerre", "termina", "terminar", "termine", "finaliza", "finalizar", "finalize", "fecha", "fechar", "cancela", "cancelar", "sai", "sair", "sair da", "sair do", "acaba", "acabar", "acaba com", "para a", "para o", "parar a", "parar o", "encerra a", "encerra o", "encerrar a", "encerrar o", "termina a", "termina o", "terminar a", "terminar o", "finaliza a", "finaliza o", "finalizar a", "finalizar o", "fecha a", "fecha o", "cancela a", "cancela o", "acaba com a", "acaba com o", "para essa", "encerra essa", "termina essa", "finaliza essa"],
                &["grill", "entrevista", "sessao", "sessao do grill", "sessao de grill", "sessao de perguntas", "perguntas", "questionario"],
                &["", "por hoje", "por agora"],
            ),
        ],
    ),
    (
        "pause",
        [
            (
                &["pause", "pause the", "pause this", "hold the", "put the"],
                &["grill", "grill session", "interview", "session", "questions"],
                &["", "for now", "for a bit", "for a moment", "on hold"],
            ),
            (
                &["pausa", "pausar", "pause", "pausa a", "pausa o", "pausar a", "pausar o", "pausa essa"],
                &["grill", "entrevista", "sessao", "sessao do grill", "perguntas"],
                &["", "por agora", "um pouco", "um minuto"],
            ),
        ],
    ),
    (
        "resume",
        [
            (
                &["resume", "resume the", "continue", "continue the", "unpause", "unpause the", "restart the", "back to the"],
                &["grill", "grill session", "interview", "session", "questions"],
                &[""],
            ),
            (
                &["retoma", "retomar", "retoma a", "retoma o", "retomar a", "retomar o", "continua", "continuar", "continua a", "continua o", "continuar a", "continuar o", "volta pra", "volta para", "voltar pra"],
                &["grill", "entrevista", "sessao", "sessao do grill", "perguntas"],
                &[""],
            ),
        ],
    ),
];

/// Ways of asking to stop that don't name the session.
const EXTRA_END: (&[&str], &[&str]) = (
    &["stop asking", "stop asking me", "stop asking questions", "stop asking me questions", "no more questions", "enough questions", "thats enough questions", "thats enough for today", "enough for today", "lets stop here", "lets end here", "stop here", "end here", "im finished", "were finished", "were done here", "end session", "stop session", "close session"],
    &["para de perguntar", "pare de perguntar", "para com as perguntas", "chega de perguntas", "sem mais perguntas", "nao quero mais perguntas", "chega por agora", "vamos parar", "vamos parar por aqui", "para por aqui", "para por hoje", "vamos encerrar", "vamos terminar", "pode parar", "pode encerrar", "terminei", "acabei"],
);

/// (command, English phrases, Portuguese phrases).
type Entry = (&'static str, Vec<String>, Vec<String>);
/// The phrase table: the fixed lists above plus every verb/object/tail combination, deduplicated.
static TABLE: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    PHRASES
        .iter()
        .map(|(cmd, en, pt)| {
            let mut langs = [en.iter().map(|p| p.to_string()).collect::<Vec<_>>(), pt.iter().map(|p| p.to_string()).collect()];
            for (c, parts) in VERB_PHRASES {
                if c != cmd {
                    continue;
                }
                for (lang, (verbs, objects, tails)) in langs.iter_mut().zip(parts) {
                    for v in *verbs {
                        for o in *objects {
                            for t in *tails {
                                lang.push([*v, *o, *t].iter().filter(|w| !w.is_empty()).copied().collect::<Vec<_>>().join(" "));
                            }
                        }
                    }
                }
            }
            if *cmd == "end" {
                langs[0].extend(EXTRA_END.0.iter().map(|p| p.to_string()));
                langs[1].extend(EXTRA_END.1.iter().map(|p| p.to_string()));
            }
            for l in &mut langs {
                let mut seen = std::collections::HashSet::new();
                l.retain(|p| seen.insert(p.clone()));
            }
            let [en, pt] = langs;
            (*cmd, en, pt)
        })
        .collect()
});

/// Openers that don't change what was asked: "can we stop the interview" is "stop the interview".
/// Only taken off the start, and only when what's left is a command.
const LEAD_INS: &[&str] = &["lets", "let us", "can we", "could we", "can you", "could you", "i want to", "i wanna", "i would like to", "id like to", "we can", "we should", "i think we can", "vamos", "pode", "podemos", "quero", "eu quero", "da pra", "dá pra", "por favor"];

/// Words dropped from the ends before matching ("uh, skip please" → "skip").
const FILLERS: &[&str] = &["uh", "um", "uhm", "hum", "hmm", "er", "ah", "eh", "please", "pls", "okay", "ok", "then", "just", "por", "favor", "entao", "tipo", "hey", "now", "here", "agora", "aqui"];

/// What a message during a grill is.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    Command(&'static str),
    /// An answer, with an escape prefix (`answer:`) removed.
    Answer(String),
}

/// Lowercase, accents and apostrophes removed, punctuation to spaces, single spaces.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '\'' | '’' => continue,
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

fn lookup(phrase: &str) -> Option<&'static str> {
    if phrase.is_empty() {
        return None;
    }
    if let Some((cmd, _, _)) = TABLE.iter().find(|(_, en, pt)| en.iter().chain(pt).any(|p| p == phrase)) {
        return Some(cmd);
    }
    // One typo is forgiven in phrases of five letters or more ("skipp", "pasue"), but not for `end`,
    // whose phrases are short words that are easy to hit by accident.
    TABLE
        .iter()
        .filter(|(cmd, _, _)| *cmd != "end")
        .find(|(_, en, pt)| en.iter().chain(pt).any(|p| p.len() >= 5 && within_one_edit(p, phrase)))
        .map(|(cmd, _, _)| *cmd)
}

/// `lookup`, also trying `words` without an opener ("can we", "vamos") at the start.
fn lookup_words(words: &[&str]) -> Option<&'static str> {
    let joined = words.join(" ");
    lookup(&joined).or_else(|| {
        LEAD_INS
            .iter()
            .map(|l| normalize(l))
            .filter_map(|l| joined.strip_prefix(&format!("{l} ")).map(str::to_string))
            .find_map(|rest| lookup(&rest))
    })
}

/// Sorts a message the user sent during a grill into a command or an answer.
pub fn parse(text: &str) -> Parsed {
    let t = text.trim();
    // Escapes: `/skip` is always a command (if known); `answer: …` or a leading quote is always an answer.
    if let Some(rest) = t.strip_prefix('/') {
        let n = normalize(rest);
        if let Some(cmd) = COMMANDS.iter().find(|c| **c == n).copied().or_else(|| lookup(&n)) {
            return Parsed::Command(cmd);
        }
    }
    for prefix in ["answer:", "resposta:"] {
        if t.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix)) {
            return Parsed::Answer(t[prefix.len()..].trim().to_string());
        }
    }
    if t.starts_with(['"', '“', '\'']) {
        return Parsed::Answer(t.to_string());
    }
    let n = normalize(t);
    // Fillers only come off the ends ("uh, skip please"): in the middle they may be real words ("faz um resumo").
    let mut words: &[&str] = &n.split(' ').collect::<Vec<_>>();
    let all: Vec<&str> = words.to_vec();
    words = &all;
    while words.first().is_some_and(|w| FILLERS.contains(w)) {
        words = &words[1..];
    }
    while words.last().is_some_and(|w| FILLERS.contains(w)) {
        words = &words[..words.len() - 1];
    }
    // Openers ("can we …") don't count towards the length: they're stripped before matching.
    if words.is_empty() || words.len() > MAX_WORDS + 3 {
        return Parsed::Answer(t.to_string());
    }
    let found = if words.len() <= MAX_WORDS { lookup_words(words) } else { None }.or_else(|| {
        let joined = words.join(" ");
        LEAD_INS
            .iter()
            .map(|l| normalize(l))
            .filter_map(|l| joined.strip_prefix(&format!("{l} ")).map(str::to_string))
            .find(|rest| rest.split(' ').count() <= MAX_WORDS)
            .and_then(|rest| lookup(&rest))
    });
    match found {
        Some(cmd) => Parsed::Command(cmd),
        None => Parsed::Answer(t.to_string()),
    }
}

/// How a request for a new interview opens ("grill me on pricing", "me entrevista sobre o onboarding"),
/// after `normalize` and an optional opener ("can you", "pode").
const START_OPENERS: &[&str] = &[
    "grill me", "grill us", "lets grill", "lets do a grill", "start a grill", "start a new grill", "new grill", "another grill",
    "interview me", "start an interview", "start a new interview", "new interview", "another interview", "lets do an interview",
    "quiz me", "question me",
    "me sabatina", "me sabatine", "me entrevistar", "me sabatinar", "me entrevista", "me entreviste", "me grelha", "me faz um grill", "faz um grill", "faz uma entrevista",
    "comeca um grill", "comeca uma entrevista", "inicia um grill", "inicia uma entrevista", "novo grill", "nova entrevista", "outra entrevista", "outro grill",
];

/// Polite openers before a start request: "can you grill me", "pode me entrevistar".
const START_LEAD_INS: &[&str] = &["can you", "could you", "please", "pls", "ok", "okay", "now", "and now", "pode", "agora", "entao", "por favor"];

/// True when `text` asks for a new interview. Only the start of the message counts, so an answer that
/// mentions interviews later on ("we would interview users first") isn't one.
pub fn is_start_request(text: &str) -> bool {
    let n = normalize(text);
    let starts = |s: &str| START_OPENERS.iter().any(|o| s == *o || s.starts_with(&format!("{o} ")));
    starts(&n) || START_LEAD_INS.iter().map(|l| normalize(l)).any(|l| n.strip_prefix(&format!("{l} ")).is_some_and(starts))
}

/// `GET /api/grill/commands`: the phrase lists, for on-device matching of spoken commands.
pub fn catalog() -> Value {
    let mut commands = serde_json::Map::new();
    for (cmd, en, pt) in TABLE.iter() {
        commands.insert(cmd.to_string(), json!({ "en": en, "pt": pt }));
    }
    json!({
        "version": VERSION,
        "max_words": MAX_WORDS,
        "fillers": FILLERS,
        "lead_ins": LEAD_INS.iter().map(|l| normalize(l)).collect::<Vec<_>>(),
        // `repeat` re-reads the card on the device; the server re-posts the question for clients without one.
        "client_only": ["repeat"],
        "commands": commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_short_utterances_are_commands() {
        let cases = [
            ("Skip", "skip"),
            ("skip this one please", "skip"),
            ("Uh, next question.", "skip"),
            ("Pula essa!", "skip"),
            ("próxima pergunta", "skip"),
            ("go back", "back"),
            ("Volta", "back"),
            ("hold on", "pause"),
            ("Espera aí", "pause"),
            ("resume", "resume"),
            ("That's enough", "end"),
            ("chega", "end"),
            ("summary so far?", "summarize"),
            ("faz um resumo", "summarize"),
            ("I don't know", "decide"),
            ("não sei", "decide"),
            ("decide for me", "decide"),
            ("say that again", "repeat"),
            ("repete", "repeat"),
            ("summery", "summarize"),
            ("previus question", "back"),
            ("/skip", "skip"),
            ("/pula", "skip"),
            ("/end", "end"),
            // Reported 2026-10-22: these were taken as answers and the interview went on.
            ("stop grill session", "end"),
            ("Stop the grill session.", "end"),
            ("end the interview", "end"),
            ("stop the interview", "end"),
            ("end session", "end"),
            ("Can we stop the grill session now?", "end"),
            ("let's end the interview for today", "end"),
            ("I want to stop the interview", "end"),
            ("stop asking me questions", "end"),
            ("no more questions", "end"),
            ("wrap up the session", "end"),
            ("Encerra a sessão", "end"),
            ("para a entrevista", "end"),
            ("vamos encerrar a entrevista por hoje", "end"),
            ("chega de perguntas", "end"),
            ("pode parar a sessão do grill", "end"),
            ("pause the interview", "pause"),
            ("pausa a entrevista", "pause"),
            ("resume the interview", "resume"),
            ("continua a entrevista", "resume"),
        ];
        for (text, cmd) in cases {
            assert_eq!(parse(text), Parsed::Command(cmd), "{text:?}");
        }
    }

    #[test]
    fn real_answers_stay_answers() {
        for text in [
            "Skip the database part and use Postgres",
            "Next quarter, probably",
            "I don't know yet, maybe yearly billing",
            "Stop sending emails on weekends",
            "Pause the subscription when the card fails",
            "Stop the session from expiring",
            "end users",
            "close the deal",
            "we can stop worrying about scale",
            "para o cliente final",
            "Let's end with yearly billing",
            "yes",
            "no",
            "Monthly",
            "8 dollars",
            "não sei se vale a pena cobrar por isso",
            "and",   // one edit from "end", but `end` is exact-match only
            "send",
            "\"skip\"",
            "answer: skip",
            "/frobnicate",
            "skipp",
            "pró",
            "é",
        ] {
            assert!(matches!(parse(text), Parsed::Answer(_)), "{text:?} was taken as a command");
        }
        assert_eq!(parse("answer: skip"), Parsed::Answer("skip".into()));
        assert_eq!(parse("Resposta: pula"), Parsed::Answer("pula".into()));
    }

    #[test]
    fn start_requests_are_recognised_only_at_the_start() {
        for text in ["Grill me about the pricing for a small todo app", "grill me", "Can you interview me on the onboarding?", "let's do a grill on billing", "Me entrevista sobre o onboarding", "pode me entrevistar sobre preços", "nova entrevista sobre o app"] {
            assert!(is_start_request(text), "{text:?}");
        }
        for text in ["We would interview users first", "monthly, grill me later", "grilled cheese", "a new grill for the patio", "Interviews are done by HR", "me"] {
            assert!(!is_start_request(text), "{text:?}");
        }
    }

    #[test]
    fn normalizes_accents_punctuation_and_apostrophes() {
        assert_eq!(normalize("  Não, É  ÓTIMO!! don't "), "nao e otimo dont");
        assert!(within_one_edit("pause", "pasue") == false && within_one_edit("pause", "paus") && within_one_edit("skip", "skipp"));
    }

    #[test]
    fn catalog_lists_every_command() {
        let c = catalog();
        // The app downloads and caches this; keep it small.
        assert!(c.to_string().len() < 80_000, "{} bytes", c.to_string().len());
        for (cmd, _, _) in PHRASES {
            assert!(c["commands"][cmd]["en"].as_array().is_some_and(|a| !a.is_empty()));
            assert!(c["commands"][cmd]["pt"].as_array().is_some_and(|a| !a.is_empty()));
        }
        assert_eq!(c["max_words"], MAX_WORDS);
    }
}
