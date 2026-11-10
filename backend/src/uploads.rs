//! Images and videos attached in the chat. Each is saved in the agent's `uploads/` folder and listed in
//! its message's `meta.images`. An image is sent to claude as an image block; a video as up to 30 frames
//! (made with ffmpeg when the message is sent) plus its path. They live as long as the chat history:
//! cleared with it, and with the agent.

use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Request, State};
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

/// Videos aren't sent as they are (only their frames), so they may be much bigger.
pub const MAX_VIDEO_BYTES: usize = 200_000_000;
/// Frames sent per video: about one a second, spread evenly over longer videos.
const MAX_FRAMES: usize = 30;

const TYPES: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("mp4", "video/mp4"),
    ("mov", "video/quicktime"),
    ("webm", "video/webm"),
];

fn is_video(file: &str) -> bool {
    media_type(file).is_some_and(|t| t.starts_with("video/"))
}

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
        [_, _, _, _, b'f', b't', b'y', b'p', b'q', b't', b' ', b' ', ..] => Some("mov"),
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => Some("mp4"),
        // Matroska's EBML header, whose DocType says WebM.
        [0x1A, 0x45, 0xDF, 0xA3, ..] if b[..b.len().min(64)].windows(4).any(|w| w == b"webm") => Some("webm"),
        _ => None,
    }
}

pub async fn upload(State(state): State<AppState>, Path(id): Path<String>, body: Bytes) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let ext = sniff(&body).ok_or_else(|| bad("only PNG, JPEG, GIF and WebP images and MP4, MOV and WebM videos are supported"))?;
    let file = format!("{}.{ext}", uuid::Uuid::new_v4());
    if is_video(&file) && body.len() > MAX_VIDEO_BYTES {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "video is over 200 MB".into()));
    }
    if !is_video(&file) && body.len() > MAX_BYTES {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, "image is over 3.75 MB".into()));
    }
    let d = dir(&state, &id);
    tokio::fs::create_dir_all(&d).await.map_err(anyhow::Error::from)?;
    tokio::fs::write(d.join(&file), &body).await.map_err(anyhow::Error::from)?;
    Ok(Json(json!({ "file": file })))
}

pub async fn get(State(state): State<AppState>, Path((id, file)): Path<(String, String)>, req: Request) -> Response {
    let Some(ty) = media_type(&file) else { return not_found().into_response() };
    let path = dir(&state, &id).join(&file);
    if !path.is_file() {
        return not_found().into_response();
    }
    // ServeFile answers Range requests, which Safari needs to play a video.
    match tower_http::services::ServeFile::new_with_mime(path, &ty.parse().expect("a known type")).try_call(req).await {
        Ok(mut r) => {
            // Names are random and never reused, so the bytes behind one never change.
            r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("private, max-age=31536000, immutable"));
            r.map(axum::body::Body::new).into_response()
        }
        Err(_) => not_found().into_response(),
    }
}

/// Check the images (and videos) a message names: each must be an upload of this agent.
pub fn validate(state: &AppState, agent_id: &str, images: &[String]) -> Result<(), ApiError> {
    if images.len() > 10 {
        return Err(bad("at most 10 images per message"));
    }
    // One video's frames are already as many images as a message should carry.
    if images.iter().filter(|f| is_video(f)).count() > 1 {
        return Err(bad("at most one video per message"));
    }
    let mut total = 0;
    for f in images {
        let size = media_type(f).and_then(|_| std::fs::metadata(dir(state, agent_id).join(f)).ok()).filter(|m| m.is_file());
        let size = size.ok_or_else(|| bad(format!("unknown image {f}")))?.len();
        // A video's frames are small (a few MB for all 30); the video itself isn't sent.
        total += if is_video(f) { 0 } else { size };
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

/// Markers (as in `prompt`) for `files`, in order, while their total stays under a message's limit. The rest
/// are left out: the grill's round prompt also lists every image's path, so the agent can still read them.
pub fn markers(state: &AppState, agent_id: &str, files: &[String]) -> String {
    let mut total = 0;
    let mut out = String::new();
    for f in files {
        let Some(size) = std::fs::metadata(dir(state, agent_id).join(f)).ok().map(|m| if is_video(f) { 0 } else { m.len() }) else { continue };
        if total + size > MAX_MESSAGE_BYTES {
            break;
        }
        total += size;
        out.push_str(&format!("\n{MARK}{f}]"));
    }
    out
}

/// A stream-json user message for `prompt` if it carries image markers; `None` means send it as plain text.
/// A video's marker becomes its path and length, then its frames, each after its timestamp.
pub async fn stdin_line(state: &AppState, agent_id: &str, prompt: &str) -> Option<String> {
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
        let path = dir(state, agent_id).join(file);
        let bytes = match media_type(file) {
            Some(_) if is_video(file) => None,
            Some(ty) => tokio::fs::read(&path).await.ok().map(|b| (ty, b)),
            None => None,
        };
        match bytes {
            None if is_video(file) && path.is_file() => match frames(&path).await {
                Some((length, frames)) => {
                    text.push_str(&format!(
                        "[Video attached: uploads/{file}, {} long. {} frames from it follow, each after its timestamp; to look closer, pull more frames from that file with ffmpeg.]",
                        clock(length),
                        frames.len()
                    ));
                    for (t, jpeg) in frames {
                        text.push_str(&format!("\n[Frame at {}]", clock(t)));
                        content.push(json!({ "type": "text", "text": std::mem::take(&mut text) }));
                        content.push(json!({ "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": base64(&jpeg) } }));
                    }
                }
                None => text.push_str(&format!(
                    "[Video attached: uploads/{file}. No frames could be made from it (ffmpeg is missing or failed on it); look at it with ffmpeg yourself if you can.]"
                )),
            },
            None if is_video(file) => text.push_str("[A video was attached but has since been deleted.]"),
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

/// `m:ss.s`, as frame timestamps and lengths are shown to the agent.
fn clock(secs: f64) -> String {
    let m = (secs / 60.0).floor();
    format!("{m}:{:04.1}", secs - m * 60.0)
}

/// Where a tool may be: on PATH or Homebrew's (a background service has a short PATH).
fn find_tool(name: &str) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect::<Vec<_>>());
    on_path.chain([FsPath::new("/opt/homebrew/bin").join(name), FsPath::new("/usr/local/bin").join(name)]).find(|p| p.is_file())
}

/// A tool's stdout, or `None` if it's missing, fails or takes over 30 s.
async fn run_tool(name: &str, args: &[&std::ffi::OsStr]) -> Option<Vec<u8>> {
    let mut cmd = tokio::process::Command::new(find_tool(name)?);
    cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output()).await.ok()?.ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// The video's length and its frames (JPEG, at most 1024 px) with their timestamps, both in seconds:
/// one a second up to MAX_FRAMES, then MAX_FRAMES spread evenly. `None` when ffmpeg can't make any.
async fn frames(video: &FsPath) -> Option<(f64, Vec<(f64, Vec<u8>)>)> {
    let os = |s: &'static str| std::ffi::OsStr::new(s);
    let probe = run_tool("ffprobe", &[os("-v"), os("error"), os("-show_entries"), os("format=duration"), os("-of"), os("csv=p=0"), video.as_os_str()]).await?;
    let length: f64 = String::from_utf8_lossy(&probe).trim().parse().ok().filter(|d: &f64| d.is_finite() && *d > 0.0)?;
    let n = (length.ceil() as usize).clamp(1, MAX_FRAMES);
    let mut out = Vec::new();
    for i in 0..n {
        let t = i as f64 * length / n as f64;
        let at = format!("{t:.3}");
        let args = [
            os("-v"), os("error"), os("-ss"), std::ffi::OsStr::new(&at), os("-i"), video.as_os_str(), os("-frames:v"), os("1"),
            os("-vf"), os("scale='min(1024,iw)':'min(1024,ih)':force_original_aspect_ratio=decrease"), os("-q:v"), os("4"), os("-f"), os("mjpeg"), os("pipe:1"),
        ];
        if let Some(jpeg) = run_tool("ffmpeg", &args).await {
            out.push((t, jpeg));
        }
    }
    (!out.is_empty()).then_some((length, out))
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
        assert_eq!(sniff(b"\0\0\0\x20ftypisom"), Some("mp4"));
        assert_eq!(sniff(b"\0\0\0\x14ftypqt  "), Some("mov"));
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm"), Some("webm"));
        assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x88matroska"), None);
        assert_eq!(media_type("2f1c4a8e-1d7b-4c43-9a43-0f6f0d7c1a11.mov"), Some("video/quicktime"));
    }

    #[test]
    fn clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(4.25), "0:04.2");
        assert_eq!(clock(83.0), "1:23.0");
    }

    async fn upload_bytes(st: &AppState, id: &str, b: Vec<u8>) -> Result<String, StatusCode> {
        upload(State(st.clone()), Path(id.to_string()), Bytes::from(b)).await.map(|r| r.0["file"].as_str().unwrap().to_string()).map_err(|e| e.0)
    }

    async fn send(st: &AppState, id: &str, files: &[String]) -> Result<(), StatusCode> {
        let body = json!({ "text": "what happens here?", "images": files });
        crate::api::send_message(State(st.clone()), Path(id.to_string()), Json(serde_json::from_value(body).unwrap())).await.map(|_| ()).map_err(|e| e.0)
    }

    /// What the agent was sent: the fake claude logs its stdin.
    async fn sent(st: &AppState, id: &str) -> String {
        let log = crate::agents::workspace(st, id).join("prompts.log");
        crate::agents::tests::until("the run", 20, || log.exists()).await;
        crate::agents::tests::until("the run to end", 20, || !crate::agents::is_running(st, id)).await;
        std::fs::read_to_string(log).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_video_reaches_the_agent_as_frames_with_timestamps_and_its_path() {
        let Some(ffmpeg) = find_tool("ffmpeg") else { return eprintln!("skipped: no ffmpeg to make the test video") };
        let (st, a) = crate::agents::tests::fake_claude_state();
        let src = st.cfg.storage_dir.parent().unwrap().join("clip.mp4");
        let made = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=duration=3:size=64x48:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(made.success());
        let file = upload_bytes(&st, &a.id, std::fs::read(&src).unwrap()).await.unwrap();
        assert!(file.ends_with(".mp4"), "{file}");
        send(&st, &a.id, std::slice::from_ref(&file)).await.unwrap();
        let line = sent(&st, &a.id).await;
        let msg: Value = serde_json::from_str(line.split("\n---\n").next().unwrap()).unwrap();
        let content = msg["message"]["content"].as_array().unwrap();
        let texts: Vec<&str> = content.iter().filter_map(|c| c["text"].as_str()).collect();
        let images: Vec<&Value> = content.iter().filter(|c| c["type"] == "image").collect();
        assert!(texts[0].starts_with(&format!("what happens here?\n[Video attached: uploads/{file}, 0:03.0 long. 3 frames from it follow")), "{texts:?}");
        assert_eq!(texts.iter().filter(|t| t.contains("[Frame at ")).count(), 3);
        assert!(texts[0].ends_with("[Frame at 0:00.0]") && texts[1] == "\n[Frame at 0:01.0]" && texts[2] == "\n[Frame at 0:02.0]", "{texts:?}");
        assert_eq!(images.len(), 3);
        assert!(images.iter().all(|i| i["source"]["media_type"] == "image/jpeg" && i["source"]["data"].as_str().unwrap().starts_with("/9j/")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_video_ffmpeg_cant_read_still_goes_with_its_path_and_a_note() {
        let (st, a) = crate::agents::tests::fake_claude_state();
        let mut junk = b"\0\0\0\x20ftypisom".to_vec();
        junk.extend([7u8; 500]);
        let file = upload_bytes(&st, &a.id, junk).await.unwrap();
        send(&st, &a.id, std::slice::from_ref(&file)).await.unwrap();
        let line = sent(&st, &a.id).await;
        assert!(line.contains(&format!("[Video attached: uploads/{file}. No frames could be made from it")), "{line}");
        assert!(!line.contains("\"type\":\"image\""), "{line}");
    }

    #[tokio::test]
    async fn videos_of_other_types_or_over_200_mb_are_refused() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        // An AVI, and Matroska that isn't WebM.
        assert_eq!(upload_bytes(&st, &a.id, b"RIFF\0\0\0\0AVI LIST".to_vec()).await, Err(StatusCode::BAD_REQUEST));
        assert_eq!(upload_bytes(&st, &a.id, b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x88matroska".to_vec()).await, Err(StatusCode::BAD_REQUEST));
        let mut big = b"\0\0\0\x20ftypisom".to_vec();
        big.resize(MAX_VIDEO_BYTES + 1, 0);
        assert_eq!(upload_bytes(&st, &a.id, big).await, Err(StatusCode::PAYLOAD_TOO_LARGE));
        // Images keep their own limit.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(MAX_BYTES + 1, 0);
        assert_eq!(upload_bytes(&st, &a.id, png).await, Err(StatusCode::PAYLOAD_TOO_LARGE));
        // A big video doesn't count toward a message's 20 MB, but a message takes one video.
        let mut v = b"\0\0\0\x20ftypisom".to_vec();
        v.resize(30_000_000, 0);
        let (v1, v2) = (upload_bytes(&st, &a.id, v.clone()).await.unwrap(), upload_bytes(&st, &a.id, v).await.unwrap());
        assert!(validate(&st, &a.id, std::slice::from_ref(&v1)).is_ok());
        assert_eq!(validate(&st, &a.id, &[v1, v2]).err().map(|e| e.0), Some(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn prompt_lists_images_after_text() {
        let p = prompt("look", &json!({ "images": ["a.png", "b.jpg"] }));
        assert_eq!(p, "look\n[loft-image:a.png]\n[loft-image:b.jpg]");
        assert_eq!(prompt("plain", &json!({})), "plain");
    }
}
