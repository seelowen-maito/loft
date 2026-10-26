//! iMessage replies: texts from the notification address go to the default agent, whose answer is texted back.
//! Reads the Messages database, which needs Full Disk Access (see signing.rs for keeping it across updates).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

use crate::state::AppState;

const POLL: Duration = Duration::from_secs(3);

/// Handles compare loosely: emails by case, phone numbers by digits.
fn normalize(handle: &str) -> String {
    let h = handle.trim();
    if h.contains('@') {
        h.to_lowercase()
    } else {
        h.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect()
    }
}

/// Recent macOS leaves `text` empty and keeps the body in `attributedBody`, an archived NSAttributedString.
fn body_text(blob: &[u8]) -> Option<String> {
    let at = blob.windows(8).position(|w| w == b"NSString")? + 8;
    // Skip the class header up to the '+' that precedes the string's length.
    let rest = &blob[at..];
    let plus = rest.iter().position(|&b| b == b'+')?;
    let rest = &rest[plus + 1..];
    let (len, start) = match *rest.first()? {
        0x81 => (u16::from_le_bytes([*rest.get(1)?, *rest.get(2)?]) as usize, 3),
        0x82 => (u32::from_le_bytes([*rest.get(1)?, *rest.get(2)?, *rest.get(3)?, *rest.get(4)?]) as usize, 5),
        n => (n as usize, 1),
    };
    let bytes = rest.get(start..start + len)?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

struct Incoming {
    rowid: i64,
    from: String,
    text: String,
}

fn read_since(db: &Path, after: i64) -> Result<(Vec<Incoming>, i64)> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("can't open {}", db.display()))?;
    let max: i64 = conn.query_row("SELECT COALESCE(MAX(ROWID), 0) FROM message", [], |r| r.get(0))?;
    if after < 0 {
        return Ok((Vec::new(), max));
    }
    // Plain incoming messages only: no reactions, edits or group events.
    let mut stmt = conn.prepare(
        "SELECT m.ROWID, h.id, m.text, m.attributedBody FROM message m JOIN handle h ON h.ROWID = m.handle_id
         WHERE m.ROWID > ?1 AND m.is_from_me = 0 AND m.item_type = 0 AND m.associated_message_type = 0
         ORDER BY m.ROWID",
    )?;
    let rows = stmt.query_map([after], |r| {
        let text: Option<String> = r.get(2)?;
        let body: Option<Vec<u8>> = r.get(3)?;
        Ok(Incoming {
            rowid: r.get(0)?,
            from: r.get(1)?,
            text: text.filter(|t| !t.trim().is_empty()).or_else(|| body.as_deref().and_then(body_text)).unwrap_or_default(),
        })
    })?;
    Ok((rows.collect::<rusqlite::Result<_>>()?, max))
}

pub async fn inbox(state: AppState, data_dir: PathBuf) {
    let db = dirs::home_dir().unwrap_or_default().join("Library/Messages/chat.db");
    let cursor_file = data_dir.join("imessage_cursor");
    // Last message ROWID handled; -1 until first read, which starts from "now" instead of replaying history.
    let mut cursor: i64 = std::fs::read_to_string(&cursor_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(-1);
    loop {
        tokio::time::sleep(POLL).await;
        let m = state.settings.get().imessage;
        if !m.enabled || m.agent_id.is_empty() || m.to.trim().is_empty() {
            // Off: forget the position, so turning it on later doesn't replay what arrived meanwhile.
            if cursor != -1 {
                cursor = -1;
                let _ = std::fs::remove_file(&cursor_file);
            }
            *state.inbox_error.lock().unwrap() = None;
            continue;
        }
        let (db2, after) = (db.clone(), cursor);
        let read = tokio::task::spawn_blocking(move || read_since(&db2, after)).await;
        let (messages, max) = match read {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
                *state.inbox_error.lock().unwrap() =
                    Some(format!("Can't read Messages ({e:#}). Give loft Full Disk Access in System Settings → Privacy & Security: {exe}"));
                continue;
            }
            Err(_) => continue,
        };
        *state.inbox_error.lock().unwrap() = None;
        let me = normalize(&m.to);
        for msg in messages.iter().filter(|x| normalize(&x.from) == me && !x.text.trim().is_empty()) {
            tracing::info!("imessage {} -> agent {}", msg.rowid, m.agent_id);
            if state.db.get_agent(&m.agent_id).ok().flatten().is_none() {
                crate::notify::send_reply(&state, "Your default agent no longer exists. Pick another one in loft's Settings.");
                continue;
            }
            state.rt.lock().unwrap().imessage_reply.insert(m.agent_id.clone());
            crate::agents::enqueue(&state, &m.agent_id, Some(msg.text.trim().to_string()), "imessage");
        }
        if max != cursor {
            cursor = max;
            let _ = std::fs::write(&cursor_file, cursor.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles() {
        assert_eq!(normalize(" You@iCloud.com "), "you@icloud.com");
        assert_eq!(normalize("+1 (555) 010-0100"), "+15550100100");
    }

    #[test]
    fn attributed_body() {
        let mut blob = b"\x04\x0bstreamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x12NSAttributedString\x00\x84\x84\x08NSObject\x00\x85\x92\x84\x84\x84\x08NSString\x01\x94\x84\x01+".to_vec();
        blob.push(5);
        blob.extend_from_slice(b"hello\x86\x84");
        assert_eq!(body_text(&blob).as_deref(), Some("hello"));
        let long = "x".repeat(300);
        let mut blob = b"..NSString\x01\x94\x84\x01+\x81".to_vec();
        blob.extend_from_slice(&(300u16).to_le_bytes());
        blob.extend_from_slice(long.as_bytes());
        assert_eq!(body_text(&blob), Some(long));
    }
}
