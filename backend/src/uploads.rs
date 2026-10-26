//! Images pasted into the chat. Each is saved in the agent's `uploads/` folder, listed in its
//! message's `meta.images`, and sent to claude as an image block. They live as long as the
//! chat history: cleared with it, and with the agent.

use std::path::PathBuf;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::agents::workspace;
use crate::api::{ApiError, ApiResult, bad, not_found};
use crate::state::AppState;

/// The API takes images up to 5 MB base64-encoded, which is this many raw bytes.
pub const MAX_BYTES: usize = 3_750_000;
/// A request to the API is capped at 32 MB; keep a message's images (base64) well under it.
const MAX_MESSAGE_BYTES: u64 = 20_000_000;

const TYPES: [(&str, &str); 4] = [("png", "image/png"), ("jpg", "image/jpeg"), ("gif", "image/gif"), ("webp", "image/webp")];

pub fn dir(state: &AppState, agent_id: &str) -> PathBuf {
    workspace(state, agent_id).join("uploads")
}

pub fn remove_all(state: &AppState, agent_id: &str) {
    let _ = std::fs::remove_dir_all(dir(state, agent_id));
}

/// The media type of a stored upload's name (`<uuid>.<ext>`); `None` for anything else.
fn media_type(file: &str) -> Option<&'static str> {
    let (stem, ext) = file.split_once('.')?;
    uuid::Uuid::parse_str(stem).ok()?;
    TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t)
}

/// What the bytes are, by their signature; the client's Content-Type isn't trusted.
fn sniff(b: &[u8]) -> Option<&'static str> {
    match b {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

pub async fn upload(State(state): State<AppState>, Path(id): Path<String>, body: Bytes) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    if body.len() > MAX_BYTES {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "image is over 3.75 MB".into()));
    }
    let ext = sniff(&body).ok_or_else(|| bad("only PNG, JPEG, GIF and WebP images are supported"))?;
    let file = format!("{}.{ext}", uuid::Uuid::new_v4());
    let d = dir(&state, &id);
    std::fs::create_dir_all(&d).map_err(anyhow::Error::from)?;
    std::fs::write(d.join(&file), &body).map_err(anyhow::Error::from)?;
    Ok(Json(json!({ "file": file })))
}

pub async fn get(State(state): State<AppState>, Path((id, file)): Path<(String, String)>) -> Response {
    let Some(ty) = media_type(&file) else { return not_found().into_response() };
    match std::fs::read(dir(&state, &id).join(&file)) {
        // Names are random and never reused, so the bytes behind one never change.
        Ok(b) => ([(header::CONTENT_TYPE, ty), (header::CACHE_CONTROL, "private, max-age=31536000, immutable")], b).into_response(),
        Err(_) => not_found().into_response(),
    }
}

/// Check the images a message names: each must be an upload of this agent.
pub fn validate(state: &AppState, agent_id: &str, images: &[String]) -> Result<(), ApiError> {
    if images.len() > 10 {
        return Err(bad("at most 10 images per message"));
    }
    let mut total = 0;
    for f in images {
        let size = media_type(f).and_then(|_| std::fs::metadata(dir(state, agent_id).join(f)).ok()).filter(|m| m.is_file());
        total += size.ok_or_else(|| bad(format!("unknown image {f}")))?.len();
    }
    if total > MAX_MESSAGE_BYTES {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "these images add up to over 20 MB; send them in a few messages".into()));
    }
    Ok(())
}

const MARK: &str = "[loft-image:";

/// The prompt for a user message: its text, then a marker per image in `meta.images`.
/// Markers survive queueing (prompts are joined as text) and become image blocks in `stdin_line`.
pub fn prompt(text: &str, meta: &Value) -> String {
    let mut p = text.to_string();
    for f in meta["images"].as_array().into_iter().flatten().filter_map(|f| f.as_str()) {
        p.push_str(&format!("\n{MARK}{f}]"));
    }
    p
}

/// A stream-json user message for `prompt` if it carries image markers; `None` means send it as plain text.
pub fn stdin_line(state: &AppState, agent_id: &str, prompt: &str) -> Option<String> {
    if !prompt.contains(MARK) {
        return None;
    }
    let mut content: Vec<Value> = Vec::new();
    let mut text = String::new();
    let mut rest = prompt;
    while let Some(i) = rest.find(MARK) {
        text.push_str(&rest[..i]);
        let after = &rest[i + MARK.len()..];
        let Some(end) = after.find(']') else { break };
        let file = &after[..end];
        let bytes = media_type(file).and_then(|ty| Some((ty, std::fs::read(dir(state, agent_id).join(file)).ok()?)));
        match bytes {
            Some((ty, b)) => {
                // Also say where it is, so the agent can attach it to an issue or PR.
                text.push_str(&format!("[Image attached: uploads/{file}]"));
                content.push(json!({ "type": "text", "text": std::mem::take(&mut text) }));
                content.push(json!({ "type": "image", "source": { "type": "base64", "media_type": ty, "data": base64(&b) } }));
            }
            None => text.push_str("[An image was attached but has since been deleted.]"),
        }
        rest = &after[end + 1..];
    }
    text.push_str(rest);
    if !text.trim().is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    Some(json!({ "type": "user", "message": { "role": "user", "content": content } }).to_string() + "\n")
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn names_must_be_uuid_with_known_ext() {
        assert_eq!(media_type("2f1c4a8e-1d7b-4c43-9a43-0f6f0d7c1a11.png"), Some("image/png"));
        assert_eq!(media_type("../../etc/passwd"), None);
        assert_eq!(media_type("2f1c4a8e-1d7b-4c43-9a43-0f6f0d7c1a11.svg"), None);
        assert_eq!(sniff(b"\x89PNG\r\n"), Some("png"));
        assert_eq!(sniff(b"<svg"), None);
    }

    #[test]
    fn prompt_lists_images_after_text() {
        let p = prompt("look", &json!({ "images": ["a.png", "b.jpg"] }));
        assert_eq!(p, "look\n[loft-image:a.png]\n[loft-image:b.jpg]");
        assert_eq!(prompt("plain", &json!({})), "plain");
    }
}
