//! Backups of Loft's database (`.loft/loft.db`) and `settings.json`: one a day while Loft runs, plus
//! Back up now in Settings. Each is a dated folder in the backups folder, owner-only, made with SQLite's
//! `VACUUM INTO` (a consistent copy while Loft writes, never a file copy of the live WAL database).
//! Restoring one is manual: see docs/backups.md.

use std::collections::HashSet;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::api::ApiResult;
use crate::state::AppState;

/// Under the home folder, when Settings names no other.
const DEFAULT_DIR: &str = "Library/Application Support/Loft/backups";
/// iCloud Drive's own folder under the home folder, there only once iCloud Drive is on.
const ICLOUD_DRIVE: &str = "Library/Mobile Documents/com~apple~CloudDocs";
/// A backup's folder name: when it was made, in local time.
const NAME: &str = "%Y-%m-%d_%H%M%S";
const KEEP_DAILY: usize = 7;
const KEEP_WEEKLY: usize = 4;
/// A day's backup waits for no agent to be running, but not past this since the last one.
const FORCE_AFTER_HOURS: i64 = 36;
/// After a failure, the daily backup tries again no sooner than this.
const RETRY_AFTER_MINUTES: i64 = 60;
/// A failure shows in Needs you once the last good backup is older than this.
pub const ALERT_AFTER_HOURS: i64 = 48;

/// What Settings and Needs you show: the backups folder, the newest good backup in it, and the last
/// attempt's error (cleared by a good one).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub dir: PathBuf,
    pub last: Option<Last>,
    pub error: Option<Failure>,
    /// iCloud Drive → Loft backups, Settings' one-click choice; none when iCloud Drive is off on this Mac.
    pub icloud: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Last {
    pub at: DateTime<Local>,
    /// The database and settings together.
    pub bytes: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub message: String,
    pub at: DateTime<Local>,
}

/// The backups folder Settings names, or the default one.
pub fn dir(s: &crate::settings::Settings) -> PathBuf {
    match s.backup_dir.trim() {
        "" => dirs::home_dir().unwrap_or_default().join(DEFAULT_DIR),
        d => PathBuf::from(d),
    }
}

/// The iCloud Drive choice for this Mac's user.
fn icloud() -> Option<PathBuf> {
    dirs::home_dir().and_then(|h| icloud_in(&h))
}

/// iCloud Drive's "Loft backups" folder under `home`, when iCloud Drive is on there. The folder itself is made
/// by the first backup; iCloud Drive's is never made, or backups would sit in a folder that doesn't sync.
fn icloud_in(home: &Path) -> Option<PathBuf> {
    let drive = home.join(ICLOUD_DRIVE);
    drive.is_dir().then(|| drive.join("Loft backups"))
}

/// Backs up `db` (and `settings`, when there is one) into a new folder under `root` named for `at`, and
/// checks the copy. Blocking. The folder appears only once it's complete.
pub fn make(db: &Path, settings: &Path, root: &Path, at: NaiveDateTime) -> Result<Last> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root).with_context(|| format!("can't create {}", root.display()))?;
    let done = root.join(at.format(NAME).to_string());
    // Back up now twice in the same second: the backup just made is as fresh, so it is the answer.
    if done.join("loft.db").is_file() {
        return Ok(found(root, at));
    }
    let partial = done.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::DirBuilder::new().mode(0o700).create(&partial).with_context(|| format!("can't create {}", partial.display()))?;
    let made = (|| -> Result<u64> {
        let out = partial.join("loft.db");
        // Never create: a missing database is an error, not an empty backup.
        let src = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        src.execute("VACUUM INTO ?1", [out.to_string_lossy()])?;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600))?;
        let check: String = Connection::open_with_flags(&out, OpenFlags::SQLITE_OPEN_READ_ONLY)?.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            bail!("the copy failed its integrity check: {check}");
        }
        let mut bytes = std::fs::metadata(&out)?.len();
        if settings.exists() {
            let to = partial.join("settings.json");
            std::fs::copy(settings, &to)?;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o600))?;
            bytes += std::fs::metadata(&to)?.len();
        }
        Ok(bytes)
    })();
    let bytes = match made.and_then(|b| std::fs::rename(&partial, &done).map(|_| b).map_err(Into::into)) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&partial);
            return Err(e);
        }
    };
    Ok(Last { at: local(at), bytes, path: done })
}

fn local(at: NaiveDateTime) -> DateTime<Local> {
    Local.from_local_datetime(&at).earliest().unwrap_or_else(|| at.and_utc().with_timezone(&Local))
}

/// The backups in `root`, by when they were made. Anything else there (other files, unfinished
/// `.partial` folders) is never counted or touched.
fn list(root: &Path) -> Vec<NaiveDateTime> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    rd.flatten()
        .filter(|e| e.path().join("loft.db").is_file())
        .filter_map(|e| NaiveDateTime::parse_from_str(&e.file_name().to_string_lossy(), NAME).ok())
        .collect()
}

/// The newest backup in `root`.
pub fn newest(root: &Path) -> Option<Last> {
    list(root).into_iter().max().map(|at| found(root, at))
}

/// The backup in `root` made at `at`.
fn found(root: &Path, at: NaiveDateTime) -> Last {
    let path = root.join(at.format(NAME).to_string());
    let size = |f: &str| std::fs::metadata(path.join(f)).map(|m| m.len()).unwrap_or(0);
    Last { at: local(at), bytes: size("loft.db") + size("settings.json"), path }
}

/// Why `dir` can't hold backups, if it can't: in Loft's folder agents can read every chat, and in a git
/// work tree the GitHub token in `settings.json` could be committed and pushed.
pub fn refuse(dir: &Path, loft: Option<&Path>) -> Option<&'static str> {
    if loft.is_some_and(|l| inside(dir, l)) {
        return Some("Backups folder can't be inside Loft's folder, where agents can read it");
    }
    if resolved(dir).ancestors().any(|a| a.join(".git").exists()) {
        return Some("Backups folder can't be inside a git repository, where it could be committed and pushed");
    }
    None
}

/// Whether `dir` is inside `loft` once symlinks are resolved. `dir` may not exist yet: its deepest
/// existing folder is resolved and the rest added back.
pub fn inside(dir: &Path, loft: &Path) -> bool {
    resolved(dir).starts_with(resolved(loft))
}

fn resolved(p: &Path) -> PathBuf {
    let mut rest = vec![];
    let mut cur = p;
    loop {
        if let Ok(real) = cur.canonicalize() {
            return rest.iter().rev().fold(real, |acc, name| acc.join(name));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Which backups to keep: the newest of each of the last 7 days that have one, and the newest of each
/// of the last 4 weeks that have one.
fn keep(mut times: Vec<NaiveDateTime>) -> HashSet<NaiveDateTime> {
    times.sort_unstable_by(|a, b| b.cmp(a));
    let mut kept = HashSet::new();
    let (mut days, mut weeks) = (vec![], vec![]);
    for t in times {
        let (day, week) = (t.date(), (t.iso_week().year(), t.iso_week().week()));
        if days.len() < KEEP_DAILY && !days.contains(&day) {
            days.push(day);
            kept.insert(t);
        }
        if weeks.len() < KEEP_WEEKLY && !weeks.contains(&week) {
            weeks.push(week);
            kept.insert(t);
        }
    }
    kept
}

/// Deletes the backups in `root` that `keep` doesn't keep.
pub fn prune(root: &Path) {
    let times = list(root);
    let kept = keep(times.clone());
    for t in times.into_iter().filter(|t| !kept.contains(t)) {
        if let Err(e) = std::fs::remove_dir_all(root.join(t.format(NAME).to_string())) {
            tracing::warn!("deleting an old backup: {e}");
        }
    }
}

/// One backup at a time, whether the day's or Back up now.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Makes a backup now and prunes old ones, then records the result for Settings and Needs you.
pub async fn run(state: &AppState) -> Status {
    let _one = RUNNING.lock().await;
    let root = dir(&state.settings.get());
    let db = state.db.path().unwrap_or_default();
    let settings = db.with_file_name("settings.json");
    let loft = state.cfg.storage_dir.parent().map(Path::to_path_buf);
    let r = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || {
            // Settings checked this when it was saved, but a symlink can change since.
            let made = if let Some(why) = refuse(&root, loft.as_deref()) {
                Err(anyhow::anyhow!("{why}: {}", root.display()))
            } else {
                make(&db, &settings, &root, Local::now().naive_local())
            };
            if made.is_ok() {
                prune(&root);
            }
            (made, newest(&root))
        })
        .await
    };
    let (made, last) = r.unwrap_or_else(|e| (Err(anyhow::anyhow!("backup task failed: {e}")), None));
    let mut rt = state.rt.lock().unwrap();
    let s = &mut rt.backup;
    s.dir = root;
    s.last = last;
    match made {
        Ok(_) => s.error = None,
        Err(e) => {
            tracing::warn!("backup failed: {e:#}");
            s.error = Some(Failure { message: format!("{e:#}"), at: Local::now() });
        }
    }
    s.clone()
}

/// Whether the day's backup should run now: none yet today, Loft quiet (or the last one too old to keep
/// waiting), and no failure within the retry wait.
fn due(s: &Status, now: DateTime<Local>, quiet: bool) -> bool {
    if s.error.as_ref().is_some_and(|e| now - e.at < chrono::Duration::minutes(RETRY_AFTER_MINUTES)) {
        return false;
    }
    match &s.last {
        None => true,
        Some(l) if l.at.date_naive() == now.date_naive() => false,
        Some(l) => quiet || now - l.at >= chrono::Duration::hours(FORCE_AFTER_HOURS),
    }
}

/// The day's backup. Looks at the backups folder at start and whenever Settings points elsewhere;
/// otherwise each tick reads only what the last backup recorded.
pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(5 * 60));
    loop {
        tick.tick().await;
        let root = dir(&state.settings.get());
        if state.rt.lock().unwrap().backup.dir != root {
            let r = root.clone();
            let last = tokio::task::spawn_blocking(move || newest(&r)).await.unwrap_or(None);
            state.rt.lock().unwrap().backup = Status { dir: root, last, ..Default::default() };
        }
        let go = {
            let rt = state.rt.lock().unwrap();
            due(&rt.backup, Local::now(), rt.running.is_empty())
        };
        if go {
            run(&state).await;
        }
    }
}

/// `GET /api/backups`: the folder, the last backup and the last error.
pub async fn get(State(state): State<AppState>) -> Json<Status> {
    let mut s = state.rt.lock().unwrap().backup.clone();
    // Before the first tick, or right after Settings moved the folder.
    s.dir = dir(&state.settings.get());
    s.icloud = icloud();
    Json(s)
}

/// `POST /api/backups`: Back up now. Answers once it's done: the status, or a 500 with the error (which
/// `GET` then shows too).
pub async fn now(State(state): State<AppState>) -> ApiResult<Status> {
    let mut s = run(&state).await;
    s.icloud = icloud();
    match s.error {
        Some(e) => Err(crate::api::ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.message)),
        None => Ok(Json(s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn temp() -> PathBuf {
        let d = crate::testdir::for_this_test("backup");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A state whose backups go to a temp folder, never the real one.
    fn state_backing_up_to(root: &Path) -> AppState {
        let st = crate::tasks::tests::state();
        let mut s = st.settings.get();
        s.backup_dir = root.to_string_lossy().into();
        st.settings.save(s).unwrap();
        st
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().mode() & 0o777
    }

    #[tokio::test]
    async fn a_backup_made_while_loft_writes_has_every_committed_row_and_is_owner_only() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        let root = temp().join("backups");
        let st = state_backing_up_to(&root);
        // Enough data that the copy takes a while, all checkpointed into the database file: what the
        // writer commits from here on lives only in the WAL, which a plain file copy would miss.
        for i in 0..20 {
            crate::tasks::tests::agent(&st, &format!("Agent {i}"));
        }
        {
            let c = st.db.conn();
            c.execute_batch("CREATE TABLE filler(b BLOB); WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000) INSERT INTO filler SELECT randomblob(8000) FROM n;").unwrap();
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
        }
        // Loft keeps writing through its own connection, before and during the backup.
        let (written, stop) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        let writer = {
            let (st, written, stop) = (st.clone(), written.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    crate::tasks::tests::agent(&st, &format!("During {}", written.load(Ordering::SeqCst)));
                    written.fetch_add(1, Ordering::SeqCst);
                }
            })
        };
        while written.load(Ordering::SeqCst) < 20 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let before = written.load(Ordering::SeqCst);
        let s = run(&st).await;
        let after = written.load(Ordering::SeqCst);
        stop.store(true, Ordering::SeqCst);
        writer.join().unwrap();
        assert!(after > before, "Loft wrote while the backup ran ({before} → {after})");

        assert!(s.error.is_none(), "{s:?}");
        let last = s.last.expect("a backup");
        assert!(last.path.starts_with(&root) && last.bytes > 0);
        assert_eq!((mode(&root), mode(&last.path)), (0o700, 0o700));
        assert_eq!((mode(&last.path.join("loft.db")), mode(&last.path.join("settings.json"))), (0o600, 0o600));

        let copy = crate::db::Db::open(&last.path.join("loft.db")).unwrap();
        let check: String = copy.conn().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok");
        let names: Vec<String> = copy.list_agents().unwrap().into_iter().map(|a| a.name).collect();
        let missing: Vec<String> = (0..20).map(|i| format!("Agent {i}")).chain((0..before).map(|i| format!("During {i}"))).filter(|n| !names.contains(n)).collect();
        assert!(missing.is_empty(), "everything committed before the backup started is in it; missing {missing:?}");
        let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(last.path.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["backup_dir"], serde_json::json!(root.to_string_lossy()));
    }

    #[tokio::test]
    async fn two_backups_in_the_same_second_both_succeed_with_one_folder() {
        let base = temp();
        let st = state_backing_up_to(&base.join("backups"));
        let db = st.db.path().unwrap();
        let at = t("2026-11-08 03:00");
        let a = make(&db, &base.join("none.json"), &base.join("backups"), at).unwrap();
        let b = make(&db, &base.join("none.json"), &base.join("backups"), at).unwrap();
        assert_eq!((a.path.clone(), a.bytes), (b.path, b.bytes));
        // Back up now pressed twice at once.
        let (x, y) = tokio::join!(run(&st), run(&st));
        assert!(x.error.is_none() && y.error.is_none(), "{x:?} {y:?}");
        let names: Vec<String> = std::fs::read_dir(base.join("backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".partial")), "{names:?}");
    }

    #[tokio::test]
    async fn a_failed_backup_is_recorded_until_a_good_one_and_leaves_nothing_behind() {
        let base = temp();
        // A file where the backups folder should be.
        std::fs::write(base.join("taken"), "").unwrap();
        let st = state_backing_up_to(&base.join("taken"));
        let Err(crate::api::ApiError(code, message)) = now(State(st.clone())).await else { panic!("Back up now answers an error") };
        assert_eq!(code, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(message.contains("can't create"), "{message}");
        let s = get(State(st.clone())).await.0;
        assert!(s.last.is_none());
        assert!(s.error.as_ref().is_some_and(|e| e.message.contains("can't create")), "Settings and Needs you see it: {s:?}");

        let mut set = st.settings.get();
        set.backup_dir = base.join("ok").to_string_lossy().into();
        st.settings.save(set).unwrap();
        let s = now(State(st.clone())).await.expect("a good backup").0;
        assert!(s.error.is_none() && s.last.is_some(), "a good backup clears the error");

        // A folder that became unsafe after Settings saved it: a repo was made around it. The backup
        // fails and says why; Settings still saves everything else.
        std::fs::create_dir_all(base.join(".git")).unwrap();
        let s = run(&st).await;
        assert!(s.error.as_ref().is_some_and(|e| e.message.contains("git repository")), "{s:?}");
        let body = serde_json::json!({ "projects_dir": st.settings.get().projects_dir, "claude_bin": "claude-other", "backup_dir": st.settings.get().backup_dir });
        assert!(crate::api::put_settings(State(st.clone()), Json(body)).await.is_ok(), "other settings still save");
        assert_eq!(st.settings.get().claude_bin, "claude-other");

        // A database that isn't there fails without leaving a half-made folder.
        let root = base.join("missing-db");
        let err = make(&base.join("nope.db"), &base.join("nope.json"), &root, t("2026-11-08 03:00")).unwrap_err();
        assert!(!err.to_string().is_empty());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "no .partial folder is left");
    }

    #[test]
    fn retention_keeps_seven_daily_and_four_weekly() {
        // Two backups a day (03:00 and 15:00) for 60 days, up to Sunday 2026-11-08.
        let end = t("2026-11-08 15:00");
        let all: Vec<NaiveDateTime> = (0..120).map(|i| end - chrono::Duration::hours(12 * i)).collect();
        let mut kept: Vec<NaiveDateTime> = keep(all).into_iter().collect();
        kept.sort();
        let want: Vec<NaiveDateTime> = [
            // The newest of each of the 3 weeks before this one (ISO weeks end on Sunday).
            "2026-10-18 15:00", "2026-10-25 15:00",
            "2026-11-01 15:00",
            // The newest of each of the last 7 days; today's is this week's too.
            "2026-11-02 15:00", "2026-11-03 15:00", "2026-11-04 15:00", "2026-11-05 15:00", "2026-11-06 15:00", "2026-11-07 15:00", "2026-11-08 15:00",
        ].iter().map(|s| t(s)).collect();
        assert_eq!(kept, want);
    }

    #[test]
    fn prune_deletes_only_old_backups() {
        let root = temp();
        let end = t("2026-11-08 03:00");
        for i in 0..40 {
            let d = root.join((end - chrono::Duration::days(i)).format(NAME).to_string());
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("loft.db"), "x").unwrap();
        }
        std::fs::write(root.join("notes.txt"), "mine").unwrap();
        std::fs::create_dir_all(root.join("2020-01-01_000000.partial")).unwrap();
        prune(&root);
        assert_eq!(list(&root).len(), 10, "7 days + the 3 earlier weeks");
        assert!(root.join("notes.txt").exists() && root.join("2020-01-01_000000.partial").exists(), "anything else is left alone");
        assert_eq!(newest(&root).unwrap().at.naive_local(), end);
    }

    #[test]
    fn the_days_backup_waits_for_a_quiet_moment_but_not_forever() {
        let now = local(t("2026-11-08 10:00"));
        let at = |s: &str| Status { last: Some(Last { at: local(t(s)), bytes: 1, path: PathBuf::new() }), ..Default::default() };
        assert!(due(&Status::default(), now, false), "never backed up: at once");
        assert!(!due(&at("2026-11-08 01:00"), now, true), "one today already");
        assert!(due(&at("2026-11-07 20:00"), now, true), "a new day, quiet");
        assert!(!due(&at("2026-11-07 20:00"), now, false), "a new day, agents busy: waits");
        assert!(due(&at("2026-11-06 20:00"), now, false), "36 hours: no more waiting");
        let mut failed = at("2026-11-07 20:00");
        failed.error = Some(Failure { message: "disk gone".into(), at: now - chrono::Duration::minutes(10) });
        assert!(!due(&failed, now, true), "just failed: waits an hour");
        failed.error.as_mut().unwrap().at = now - chrono::Duration::minutes(61);
        assert!(due(&failed, now, true));
    }

    #[test]
    fn failures_show_in_needs_you_after_two_days_without_a_good_backup() {
        let st = crate::tasks::tests::state();
        let items = |st: &AppState| crate::needs_you::snapshot(st).unwrap()["items"].as_array().unwrap().iter().filter(|i| i["type"] == "backup_failed").cloned().collect::<Vec<_>>();
        let now = Local::now();
        let set = |last_hours_ago: Option<i64>, failed: bool| {
            st.rt.lock().unwrap().backup = Status {
                dir: "/Volumes/Gone/backups".into(),
                last: last_hours_ago.map(|h| Last { at: now - chrono::Duration::hours(h), bytes: 1, path: PathBuf::new() }),
                error: failed.then(|| Failure { message: "can't create /Volumes/Gone/backups".into(), at: Local::now() }),
                icloud: None,
            };
        };
        set(Some(47), true);
        assert!(items(&st).is_empty(), "a good one within two days: not yet");
        set(Some(49), false);
        assert!(items(&st).is_empty(), "no failure: nothing to say");
        set(Some(49), true);
        let i = items(&st);
        assert_eq!(i.len(), 1);
        assert_eq!((i[0]["urgency"].clone(), i[0]["actions"][0]["path"].clone()), (serde_json::json!(1), serde_json::json!("/api/backups")));
        assert!(i[0]["text"].as_str().unwrap().contains("/Volumes/Gone/backups"));
        let id = i[0]["id"].clone();
        set(Some(49), true);
        assert_eq!(items(&st)[0]["id"], id, "the same item while retries keep failing");
        set(None, true);
        assert_eq!(items(&st).len(), 1, "never a good one");
    }

    #[tokio::test]
    async fn icloud_drive_is_offered_only_when_it_is_on_and_takes_the_next_backup() {
        let home = temp();
        assert_eq!(icloud_in(&home), None, "iCloud Drive off: no choice, and nothing made");
        std::fs::create_dir_all(home.join(ICLOUD_DRIVE)).unwrap();
        let icloud = icloud_in(&home).expect("iCloud Drive on");
        assert_eq!(icloud, home.join("Library/Mobile Documents/com~apple~CloudDocs/Loft backups"));
        assert!(!icloud.exists(), "the first backup makes it");

        // Chosen in Settings, like any folder, then the next backup lands there in the same layout.
        let st = crate::tasks::tests::state();
        let put = |dir: &Path| crate::api::put_settings(State(st.clone()), Json(serde_json::json!({ "projects_dir": st.settings.get().projects_dir, "backup_dir": dir })));
        assert!(put(&icloud).await.is_ok());
        let s = now(State(st.clone())).await.expect("a good backup").0;
        let last = s.last.expect("a backup");
        assert_eq!(last.path.parent(), Some(icloud.as_path()));
        assert!(NaiveDateTime::parse_from_str(&last.path.file_name().unwrap().to_string_lossy(), NAME).is_ok(), "a dated folder");
        assert!(last.path.join("loft.db").is_file() && last.path.join("settings.json").is_file());
        assert_eq!((mode(&icloud), mode(&last.path)), (0o700, 0o700));

        // The same checks as any folder: iCloud Drive linked into Loft's folder, or in a repo, is refused.
        let into_loft = temp().join("home");
        std::fs::create_dir_all(st.cfg.storage_dir.join("agent-1")).unwrap();
        std::fs::create_dir_all(into_loft.join("Library/Mobile Documents")).unwrap();
        std::os::unix::fs::symlink(st.cfg.storage_dir.join("agent-1"), into_loft.join(ICLOUD_DRIVE)).unwrap();
        assert!(put(&icloud_in(&into_loft).unwrap()).await.is_err(), "through a symlink into Loft");
        std::fs::create_dir_all(home.join(".git")).unwrap();
        assert!(put(&home.join(ICLOUD_DRIVE).join("Other backups")).await.is_err_and(|e| e.1.contains("git repository")));
    }

    #[tokio::test]
    async fn settings_refuse_a_backups_folder_that_is_relative_or_inside_loft_or_a_repo() {
        let st = crate::tasks::tests::state();
        let put = |dir: String| crate::api::put_settings(State(st.clone()), Json(serde_json::json!({ "projects_dir": st.settings.get().projects_dir, "backup_dir": dir })));
        assert!(put("backups".into()).await.is_err(), "relative");
        assert!(put("/Volumes/Disk/../../tmp".into()).await.is_err(), "with ..");
        let inside = st.cfg.storage_dir.join("agent-1/backups");
        assert!(put(inside.to_string_lossy().into()).await.is_err(), "agents could read it");
        // A symlink outside that leads inside.
        let link = temp().join("looks-outside");
        std::fs::create_dir_all(st.cfg.storage_dir.join("agent-1")).unwrap();
        std::os::unix::fs::symlink(st.cfg.storage_dir.join("agent-1"), &link).unwrap();
        assert!(put(link.join("backups").to_string_lossy().into()).await.is_err(), "through a symlink");
        // Inside a git work tree, wherever it is.
        let repo = temp();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let in_repo = put(repo.join("docs/backups").to_string_lossy().into()).await;
        assert!(in_repo.is_err_and(|e| e.1.contains("git repository")), "the token could be committed");
        // The projects folder is no reason to refuse: with it set to ~, the default folder still saves.
        let home = dirs::home_dir().unwrap();
        let at_home = crate::api::put_settings(State(st.clone()), Json(serde_json::json!({ "projects_dir": home, "backup_dir": "" }))).await;
        assert!(at_home.is_ok(), "{:?}", at_home.err());
        assert_eq!(st.settings.get().projects_dir, home);
        assert!(put("/Volumes/Disk/Loft backups".into()).await.is_ok());
        assert_eq!(st.settings.get().backup_dir, "/Volumes/Disk/Loft backups");
        // A client that doesn't know the field keeps it.
        let _ = crate::api::put_settings(State(st.clone()), Json(serde_json::json!({ "projects_dir": st.settings.get().projects_dir }))).await.unwrap();
        assert_eq!(st.settings.get().backup_dir, "/Volumes/Disk/Loft backups");
        // The iPhone sends only the folder: everything else stays as saved, even when changed on the web since.
        let mut web = st.settings.get();
        web.ssh_host = "mac.example".into();
        st.settings.save(web).unwrap();
        let before = st.settings.get();
        assert!(put("/Volumes/Other".into()).await.is_ok());
        let _ = crate::api::put_settings(State(st.clone()), Json(serde_json::json!({ "backup_dir": "/Volumes/Disk/Loft backups" }))).await.unwrap();
        assert_eq!(st.settings.get(), before, "only the folder changed, and back");
    }
}
