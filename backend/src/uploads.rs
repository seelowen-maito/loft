//! Images and videos attached in the chat. Each is saved in the agent's `uploads/` folder and listed in
//! its message's `meta.images`. An image is sent to claude as an image block; a video as up to 30 frames
//! (made with ffmpeg when the message is sent) plus its path. They live as long as the chat history:
//! cleared with it, and with the agent.

use std::path::{Path as FsPath, PathBuf};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::agents::workspace;
use crate::api::{ApiError, ApiResult, bad, not_found};
use crate::state::AppState;

/// The API takes images up to 5 MB base64-encoded, which is this many raw bytes.
pub const MAX_BYTES: usize = 3_750_000;
/// A request to the API is capped at 32 MB; keep a message's images (base64) well under it.
const MAX_MESSAGE_BYTES: u64 = 20_000_000;
/// What a video's frames add to a message (30 JPEGs at 1024 px, base64), counted in place of the video itself.
const VIDEO_ALLOWANCE: u64 = 6_000_000;

/// Videos aren't sent as they are (only their frames), so they may be much bigger.
pub const MAX_VIDEO_BYTES: usize = 200_000_000;
/// Frames sent per video: about one a second, spread evenly over longer videos.
const MAX_FRAMES: usize = 30;
/// All of a video's frames must be made within this, so a slow file can't hold the run's start.
const FRAMES_BUDGET: Duration = Duration::from_secs(60);

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

const MP4_BRANDS: [&[u8]; 8] = [b"isom", b"iso2", b"mp41", b"mp42", b"avc1", b"M4V ", b"dash", b"MSNV"];

/// What the bytes are, by their signature; the client's Content-Type isn't trusted.
fn sniff(b: &[u8]) -> Option<&'static str> {
    match b {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        [_, _, _, _, b'f', b't', b'y', b'p', b'q', b't', b' ', b' ', ..] => Some("mov"),
        // Only MP4's brands: HEIC, AVIF and M4A are `ftyp` files too.
        [_, _, _, _, b'f', b't', b'y', b'p', brand @ ..] if MP4_BRANDS.iter().any(|b| brand.starts_with(b)) => Some("mp4"),
        // Matroska's EBML header, whose DocType says WebM.
        [0x1A, 0x45, 0xDF, 0xA3, ..] if b[..b.len().min(64)].windows(4).any(|w| w == b"webm") => Some("webm"),
        _ => None,
    }
}

/// Saves the body as it arrives, so a big video never sits in memory.
pub async fn upload(State(state): State<AppState>, Path(id): Path<String>, body: Body) -> ApiResult<Value> {
    use tokio::io::AsyncWriteExt;
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let mut stream = body.into_data_stream();
    // Sniffing looks at up to the first 64 bytes.
    let mut head = Vec::new();
    while head.len() < 64 {
        match stream.next().await {
            Some(chunk) => head.extend_from_slice(&chunk.map_err(|_| bad("upload interrupted"))?),
            None => break,
        }
    }
    let ext = sniff(&head).ok_or_else(|| bad("only PNG, JPEG, GIF and WebP images and MP4, MOV and WebM videos are supported"))?;
    let file = format!("{}.{ext}", uuid::Uuid::new_v4());
    let (max, too_big) = if is_video(&file) { (MAX_VIDEO_BYTES, "video is over 200 MB") } else { (MAX_BYTES, "image is over 3.75 MB") };
    let d = dir(&state, &id);
    tokio::fs::create_dir_all(&d).await.map_err(anyhow::Error::from)?;
    let path = d.join(&file);
    let mut out = tokio::fs::File::create(&path).await.map_err(anyhow::Error::from)?;
    let mut size = 0;
    let mut chunk = Some(Ok(Bytes::from(head)));
    let result = loop {
        let Some(c) = chunk else { break Ok(()) };
        let c = match c {
            Ok(c) => c,
            Err(_) => break Err(bad("upload interrupted")),
        };
        size += c.len();
        if size > max {
            break Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, too_big.into()));
        }
        if let Err(e) = out.write_all(&c).await {
            break Err(anyhow::Error::from(e).into());
        }
        chunk = stream.next().await;
    };
    // tokio's File finishes its last write in the background: flush, so the file is whole once we answer.
    let result = match result {
        Ok(()) => out.flush().await.map_err(|e| anyhow::Error::from(e).into()),
        e => e,
    };
    if let Err(e) = result {
        drop(out);
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
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
        // The video itself isn't sent, only its frames.
        total += if is_video(f) { VIDEO_ALLOWANCE } else { size };
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
        let Some(size) = std::fs::metadata(dir(state, agent_id).join(f)).ok().map(|m| if is_video(f) { VIDEO_ALLOWANCE } else { m.len() }) else { continue };
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

/// A tool's stdout, or `None` if it's missing, fails or takes over `limit`.
async fn run_tool(name: &str, args: &[&std::ffi::OsStr], limit: Duration) -> Option<Vec<u8>> {
    let mut cmd = tokio::process::Command::new(find_tool(name)?);
    cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(limit, cmd.output()).await.ok()?.ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

/// The video's length and its frames (JPEG, at most 1024 px) with their timestamps, both in seconds:
/// one a second up to MAX_FRAMES, then MAX_FRAMES spread evenly. Stops with what it has after FRAMES_BUDGET.
/// `None` when ffmpeg can't make any.
async fn frames(video: &FsPath) -> Option<(f64, Vec<(f64, Vec<u8>)>)> {
    let os = |s: &'static str| std::ffi::OsStr::new(s);
    let probe = run_tool("ffprobe", &[os("-v"), os("error"), os("-show_entries"), os("format=duration"), os("-of"), os("csv=p=0"), video.as_os_str()], Duration::from_secs(30)).await?;
    let length: f64 = String::from_utf8_lossy(&probe).trim().parse().ok().filter(|d: &f64| d.is_finite() && *d > 0.0)?;
    let n = (length.ceil() as usize).clamp(1, MAX_FRAMES);
    let mut out = Vec::new();
    let start = Instant::now();
    for i in 0..n {
        let Some(left) = FRAMES_BUDGET.checked_sub(start.elapsed()) else { break };
        let t = i as f64 * length / n as f64;
        let at = format!("{t:.3}");
        let args = [
            os("-v"), os("error"), os("-ss"), std::ffi::OsStr::new(&at), os("-i"), video.as_os_str(), os("-frames:v"), os("1"),
            os("-vf"), os("scale='min(1024,iw)':'min(1024,ih)':force_original_aspect_ratio=decrease"), os("-q:v"), os("4"), os("-f"), os("mjpeg"), os("pipe:1"),
        ];
        if let Some(jpeg) = run_tool("ffmpeg", &args, left.min(Duration::from_secs(30))).await {
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
        // HEIC, AVIF and M4A share the container, not the brand.
        for brand in [b"heic", b"avif", b"M4A "] {
            assert_eq!(sniff(&[b"\0\0\0\x18ftyp".as_slice(), brand].concat()), None);
        }
        assert_eq!(sniff(b"\0\0\0\x18ftypmp42"), Some("mp4"));
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
        upload(State(st.clone()), Path(id.to_string()), Body::from(b)).await.map(|r| r.0["file"].as_str().unwrap().to_string()).map_err(|e| e.0)
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
        assert_eq!(upload_bytes(&st, &a.id, b"\0\0\0\x18ftypheic".to_vec()).await, Err(StatusCode::BAD_REQUEST));
        let mut big = b"\0\0\0\x20ftypisom".to_vec();
        big.resize(MAX_VIDEO_BYTES + 1, 0);
        assert_eq!(upload_bytes(&st, &a.id, big).await, Err(StatusCode::PAYLOAD_TOO_LARGE));
        // Images keep their own limit.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(MAX_BYTES + 1, 0);
        assert_eq!(upload_bytes(&st, &a.id, png).await, Err(StatusCode::PAYLOAD_TOO_LARGE));
        // A big video counts a flat allowance, not its size, and a message takes one video.
        let mut v = b"\0\0\0\x20ftypisom".to_vec();
        v.resize(30_000_000, 0);
        let (v1, v2) = (upload_bytes(&st, &a.id, v.clone()).await.unwrap(), upload_bytes(&st, &a.id, v).await.unwrap());
        assert!(validate(&st, &a.id, std::slice::from_ref(&v1)).is_ok());
        assert_eq!(validate(&st, &a.id, &[v1, v2]).err().map(|e| e.0), Some(StatusCode::BAD_REQUEST));
    }

    #[tokio::test]
    async fn a_video_counts_a_flat_allowance_toward_a_messages_size() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let mut v = b"\0\0\0\x20ftypisom".to_vec();
        v.resize(1000, 0);
        let video = upload_bytes(&st, &a.id, v).await.unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(MAX_BYTES, 0);
        let pngs = [upload_bytes(&st, &a.id, png.clone()).await.unwrap(), upload_bytes(&st, &a.id, png.clone()).await.unwrap(), upload_bytes(&st, &a.id, png.clone()).await.unwrap(), upload_bytes(&st, &a.id, png.clone()).await.unwrap()];
        // Each upload is whole on disk once it's answered, or the sizes below would come up short.
        assert!(pngs.iter().all(|f| std::fs::metadata(dir(&st, &a.id).join(f)).unwrap().len() == MAX_BYTES as u64));
        // 4 x 3.75 MB = 15 MB fits alone, but not with a video's 6 MB.
        assert!(validate(&st, &a.id, &pngs).is_ok());
        let with_video = [pngs.to_vec(), vec![video.clone()]].concat();
        assert_eq!(validate(&st, &a.id, &with_video).err().map(|e| e.0), Some(StatusCode::PAYLOAD_TOO_LARGE));
        // markers() stops at the same point: the video's marker doesn't fit after the images.
        let m = markers(&st, &a.id, &with_video);
        assert!(m.contains(&pngs[3]) && !m.contains(&video), "{m}");
        // And a lone video counts 6 MB, not its own tiny size: it fits with 3 images (11.25 MB + 6 MB) but not with 4.
        assert!(markers(&st, &a.id, &[pngs[0].clone(), video.clone()]).contains(&video));
    }

    #[tokio::test]
    async fn get_answers_a_range_request_with_206() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let mut v = b"\0\0\0\x20ftypisom".to_vec();
        v.resize(1000, 7);
        let file = upload_bytes(&st, &a.id, v).await.unwrap();
        let req = Request::builder().header(header::RANGE, "bytes=0-99").body(Body::empty()).unwrap();
        let r = get(State(st.clone()), Path((a.id.clone(), file)), req).await;
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.headers()[header::CONTENT_RANGE], "bytes 0-99/1000");
    }

    #[tokio::test]
    async fn a_video_over_30_seconds_gets_30_frames_spread_evenly() {
        let Some(ffmpeg) = find_tool("ffmpeg") else { return eprintln!("skipped: no ffmpeg to make the test video") };
        let dir = std::env::temp_dir().join(format!("loft-frames-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("long.mp4");
        let made = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=duration=60:size=64x48:rate=1", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(made.success());
        let (length, frames) = frames(&src).await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(length.round(), 60.0);
        assert_eq!(frames.len(), MAX_FRAMES);
        for (i, (t, _)) in frames.iter().enumerate() {
            assert!((t - i as f64 * length / MAX_FRAMES as f64).abs() < 1e-6, "frame {i} at {t}");
        }
    }

    #[test]
    fn prompt_lists_images_after_text() {
        let p = prompt("look", &json!({ "images": ["a.png", "b.jpg"] }));
        assert_eq!(p, "look\n[loft-image:a.png]\n[loft-image:b.jpg]");
        assert_eq!(prompt("plain", &json!({})), "plain");
    }
}
