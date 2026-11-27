//! Each agent's processes on Windows: a Job Object per agent, which stands in for the Unix process session (see
//! processes.rs). A run's claude starts suspended, joins the agent's job and only then runs, so every process it
//! starts, and theirs, are in the job too: Windows puts a child in its parent's job, and nothing in an agent's job may
//! leave it. A detached server (`nohup … &`) or a daemon stays in it, whatever its parent. Stopping an agent ends its
//! whole job at once: Windows has no SIGTERM to let its processes clean up.
//!
//! A job's name lasts only while a handle to it is open (its processes don't hold one), so Loft keeps a handle on each
//! agent's job for as long as it runs. A Loft that restarted can't reach the jobs of the one before, so the agent's
//! `<id>.job` file in the agents' files records the job's processes (PID and start time) when a run starts and ends
//! and every minute: after a restart those still running are the agent's too, and stopping it ends them. What they
//! start after the restart, and what started in the minute before it, isn't known: those are nobody's. The agent's
//! next run gets a new job.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, UNICODE_STRING, WAIT_TIMEOUT};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
    Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, GetProcessTimes, OpenProcess, OpenThread, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess, WaitForSingleObject,
};

/// How a run's claude is created: suspended until it is in the agent's job, and with no console window (Loft may run
/// with no console of its own, and each console program would open one).
pub const CREATION_FLAGS: u32 = CREATE_SUSPENDED | CREATE_NO_WINDOW;

/// The exit code of a process Loft ended.
const STOPPED: u32 = 1;

/// A handle, closed when dropped.
struct Handle(HANDLE);

impl Handle {
    fn new(h: HANDLE) -> Option<Self> {
        (!h.is_null() && h != INVALID_HANDLE_VALUE).then_some(Handle(h))
    }
}

// A kernel handle may be used and closed from any thread.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The job's name: in this Windows session's namespace, unique to the storage folder (a test instance's agents have
/// jobs of their own) and the agent.
fn name(storage: &Path, agent_id: &str) -> Vec<u16> {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(storage.to_string_lossy().as_bytes());
    let hash: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    wide(&format!(r"Local\loft-{hash}-{agent_id}"))
}

fn marker(storage: &Path, agent_id: &str) -> PathBuf {
    storage.join(crate::agents::AGENT_FILES).join(format!("{agent_id}.job"))
}

/// Loft's handle on each agent's job, by the job's name.
static JOBS: Mutex<Vec<(Vec<u16>, Handle)>> = Mutex::new(Vec::new());

/// Runs `f` on the agent's job: the one Loft holds, else with `create` a new one (which Loft then holds).
fn with_job<R>(storage: &Path, agent_id: &str, create: bool, f: impl FnOnce(&Handle) -> R) -> std::io::Result<Option<R>> {
    let name = name(storage, agent_id);
    let mut jobs = JOBS.lock().unwrap();
    if let Some((_, job)) = jobs.iter().find(|(n, _)| *n == name) {
        return Ok(Some(f(job)));
    }
    if !create {
        return Ok(None);
    }
    let job = Handle::new(unsafe { CreateJobObjectW(std::ptr::null(), name.as_ptr()) }).ok_or_else(std::io::Error::last_os_error)?;
    let r = f(&job);
    jobs.push((name, job));
    Ok(Some(r))
}

/// Puts the suspended `child` (started with [`CREATION_FLAGS`]) in the agent's job, creating the job if Loft holds
/// none for it, then lets it run. On an error the child is still suspended: the caller ends it.
pub fn join(storage: &Path, agent_id: &str, child: &tokio::process::Child) -> std::io::Result<()> {
    let (Some(process), Some(pid)) = (child.raw_handle(), child.id()) else {
        return Err(std::io::Error::other("claude exited before it started"));
    };
    if let Some(dir) = marker(storage, agent_id).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let assign = |job: &Handle| if unsafe { AssignProcessToJobObject(job.0, process.cast()) } != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) };
    with_job(storage, agent_id, true, assign)?.unwrap_or_else(|| Err(std::io::Error::other("no job")))?;
    resume(pid);
    remember(storage, agent_id);
    Ok(())
}

/// Resumes the threads of `pid`, which was created suspended.
fn resume(pid: u32) {
    let Some(snap) = Handle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }) else { return };
    let mut t = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
    let mut more = unsafe { Thread32First(snap.0, &mut t) } != 0;
    while more {
        if t.th32OwnerProcessID == pid
            && let Some(thread) = Handle::new(unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, t.th32ThreadID) })
        {
            unsafe { ResumeThread(thread.0) };
        }
        more = unsafe { Thread32Next(snap.0, &mut t) } != 0;
    }
}

/// The agents that have had a job.
pub fn agents(storage: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(storage.join(crate::agents::AGENT_FILES)) else { return vec![] };
    dir.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".job").map(String::from)).collect()
}

/// The processes in the job Loft holds for the agent.
fn in_job(storage: &Path, agent_id: &str) -> Vec<u32> {
    with_job(storage, agent_id, false, |job| process_ids(job)).ok().flatten().unwrap_or_default()
}

/// The processes in `job`: the list's two counts, then the IDs, in a buffer grown until every process fits.
fn process_ids(job: &Handle) -> Vec<u32> {
    let first = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList) / std::mem::size_of::<usize>();
    let mut room = 256usize;
    loop {
        let mut buf = vec![0usize; first + room];
        let size = (buf.len() * std::mem::size_of::<usize>()) as u32;
        let ok = unsafe { QueryInformationJobObject(job.0, JobObjectBasicProcessIdList, buf.as_mut_ptr().cast(), size, std::ptr::null_mut()) } != 0;
        let list: &JOBOBJECT_BASIC_PROCESS_ID_LIST = unsafe { &*buf.as_ptr().cast() };
        let (assigned, listed) = (list.NumberOfAssignedProcesses as usize, (list.NumberOfProcessIdsInList as usize).min(room));
        if (ok && listed >= assigned) || room >= 1 << 16 {
            return buf[first..first + listed].iter().map(|&p| p as u32).collect();
        }
        room = (assigned + 64).max(room * 2);
    }
}

/// When `pid` started (FILETIME's 100 ns steps since 1601), while it runs.
fn started(pid: u32) -> Option<u64> {
    let p = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid) })?;
    if unsafe { WaitForSingleObject(p.0, 0) } != WAIT_TIMEOUT {
        return None;
    }
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    (unsafe { GetProcessTimes(p.0, &mut created, &mut exited, &mut kernel, &mut user) } != 0).then(|| ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
}

/// The processes the agent's file records that still run: the same PID, started at the same time.
fn recorded(storage: &Path, agent_id: &str) -> Vec<u32> {
    let text = std::fs::read_to_string(marker(storage, agent_id)).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let (pid, at) = l.split_once(' ')?;
            let (pid, at) = (pid.parse().ok()?, at.parse().ok()?);
            (started(pid) == Some(at)).then_some(pid)
        })
        .collect()
}

/// The agent's processes: those in its job, and those recorded before Loft restarted that still run.
pub fn pids(storage: &Path, agent_id: &str) -> Vec<u32> {
    let mut pids = in_job(storage, agent_id);
    for pid in recorded(storage, agent_id) {
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
}

/// Records the agent's processes in its file (see the module docs).
pub fn remember(storage: &Path, agent_id: &str) {
    let text: String = pids(storage, agent_id).into_iter().filter_map(|pid| Some(format!("{pid} {}\n", started(pid)?))).collect();
    let path = marker(storage, agent_id);
    let tmp = path.with_extension("job.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Records every agent's processes.
pub fn remember_all(storage: &Path) {
    for id in agents(storage) {
        remember(storage, &id);
    }
}

/// Ends every process of the agent: its whole job, and those recorded before Loft restarted. Whether it had any.
pub fn terminate(storage: &Path, agent_id: &str) -> bool {
    let left = recorded(storage, agent_id);
    let ended = with_job(storage, agent_id, false, |job| unsafe { TerminateJobObject(job.0, STOPPED) } != 0).ok().flatten() == Some(true);
    for &pid in &left {
        let _ = kill(pid);
    }
    ended || !left.is_empty()
}

/// Forgets the agent's job, when the agent is deleted (after [`terminate`]).
pub fn forget(storage: &Path, agent_id: &str) {
    let name = name(storage, agent_id);
    JOBS.lock().unwrap().retain(|(n, _)| *n != name);
    let _ = std::fs::remove_file(marker(storage, agent_id));
}

/// Ends `root` and the processes under it that are in the agent's job, leaving the job's others (a detached server
/// whose shell has exited) running: Windows' stand-in for the SIGTERM after which claude ends its own background
/// shells.
pub fn kill_tree(storage: &Path, agent_id: &str, root: u32) {
    let in_job: std::collections::HashSet<u32> = pids(storage, agent_id).into_iter().collect();
    let procs = table();
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        let children: Vec<u32> = procs.iter().filter(|p| p.ppid == parent && in_job.contains(&p.pid) && !tree.contains(&p.pid)).map(|p| p.pid).collect();
        tree.extend(children);
        i += 1;
    }
    for pid in tree {
        let _ = kill(pid);
    }
}

/// Ends `pid`. A process that has already ended, which a handle someone holds keeps in the table's place, is no
/// process (ERROR_INVALID_PARAMETER, as OpenProcess says), not a refusal: TerminateProcess says access denied.
pub fn kill(pid: u32) -> std::io::Result<()> {
    let p = Handle::new(unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) }).ok_or_else(std::io::Error::last_os_error)?;
    if unsafe { TerminateProcess(p.0, STOPPED) } == 0 {
        let e = std::io::Error::last_os_error();
        if unsafe { WaitForSingleObject(p.0, 0) } != WAIT_TIMEOUT {
            return Err(std::io::Error::from_raw_os_error(87));
        }
        return Err(e);
    }
    Ok(())
}

/// Whether `pid` is a running process.
pub fn alive(pid: u32) -> bool {
    Handle::new(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) }).is_some_and(|p| unsafe { WaitForSingleObject(p.0, 0) } == WAIT_TIMEOUT)
}

/// A process in the system's table.
pub struct Entry {
    pub pid: u32,
    pub ppid: u32,
    /// Its executable's full path, or only its file name when Loft may not open the process.
    pub exe: String,
    /// Its command line, when Loft may read it.
    pub command: Option<String>,
    pub age_secs: u64,
}

/// Every process.
pub fn table() -> Vec<Entry> {
    let Some(snap) = Handle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return vec![] };
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut list = vec![];
    let mut more = unsafe { Process32FirstW(snap.0, &mut e) } != 0;
    while more {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        let file = String::from_utf16_lossy(&e.szExeFile[..len]);
        let (exe, command, age_secs) = details(e.th32ProcessID);
        list.push(Entry { pid: e.th32ProcessID, ppid: e.th32ParentProcessID, exe: exe.unwrap_or(file), command, age_secs });
        more = unsafe { Process32NextW(snap.0, &mut e) } != 0;
    }
    list
}

/// The full path of `pid`'s executable, its command line and how long it has run, when Loft may open it.
fn details(pid: u32) -> (Option<String>, Option<String>, u64) {
    let Some(p) = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }) else { return (None, None, 0) };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let exe = (unsafe { QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]));
    // From 1601 to 1970, in FILETIME's steps.
    let age = started(pid).map_or(0, |ticks| {
        let at = SystemTime::UNIX_EPOCH + Duration::from_nanos(ticks.saturating_sub(116_444_736_000_000_000).saturating_mul(100));
        at.elapsed().map_or(0, |d| d.as_secs())
    });
    (exe, command_line(&p), age)
}

/// The command line of the process `p` (opened for limited queries): a UNICODE_STRING whose text follows it in the
/// buffer, which grows once to the size the first call asks for.
fn command_line(p: &Handle) -> Option<String> {
    let mut room = 1024usize;
    for _ in 0..2 {
        let mut buf = vec![0usize; room.div_ceil(std::mem::size_of::<usize>())];
        let size = (buf.len() * std::mem::size_of::<usize>()) as u32;
        let mut needed = 0u32;
        let status = unsafe { NtQueryInformationProcess(p.0, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), size, &mut needed) };
        if status >= 0 {
            let text: &UNICODE_STRING = unsafe { &*buf.as_ptr().cast() };
            if text.Buffer.is_null() {
                return None;
            }
            let chars = unsafe { std::slice::from_raw_parts(text.Buffer, text.Length as usize / 2) };
            return Some(String::from_utf16_lossy(chars));
        }
        if needed as usize <= room {
            return None;
        }
        room = needed as usize;
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A suspended `cmd` that starts a `ping` (which waits 30 s) in the background and waits on another: a tree two
    /// processes deep, with a child its parent doesn't wait for.
    pub(crate) fn spawn_tree(storage: &Path, agent_id: &str) -> tokio::process::Child {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.args(["/d", "/c", "start /b ping -n 30 127.0.0.1 >nul & ping -n 30 127.0.0.1 >nul"]).creation_flags(CREATION_FLAGS).kill_on_drop(true);
        let child = cmd.spawn().unwrap();
        join(storage, agent_id, &child).unwrap();
        child
    }

    /// The processes in the job, once its tree has started (`want` of them).
    pub(crate) fn wait_for(storage: &Path, agent_id: &str, want: usize) -> Vec<u32> {
        for _ in 0..100 {
            let pids = pids(storage, agent_id);
            if pids.len() >= want {
                return pids;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        pids(storage, agent_id)
    }

    #[tokio::test]
    async fn the_tree_a_run_starts_is_in_the_agents_job_and_stopping_ends_it() {
        let storage = crate::testdir::for_this_test("job-tree");
        let child = spawn_tree(&storage, "a1");
        let root = child.id().unwrap();
        let pids = wait_for(&storage, "a1", 3);
        assert!(pids.len() >= 3 && pids.contains(&root), "{pids:?}");
        assert_eq!(agents(&storage), vec!["a1".to_string()]);
        // Another agent's job holds none of them, and has none.
        assert!(super::pids(&storage, "b2").is_empty());
        assert!(terminate(&storage, "a1"));
        for _ in 0..100 {
            if pids.iter().all(|&p| !alive(p)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(pids.iter().all(|&p| !alive(p)), "{pids:?}");
        forget(&storage, "a1");
        assert!(agents(&storage).is_empty());
    }

    #[tokio::test]
    async fn killing_a_tree_leaves_the_jobs_other_processes() {
        let storage = crate::testdir::for_this_test("job-kill-tree");
        let first = spawn_tree(&storage, "a1");
        let second = spawn_tree(&storage, "a1");
        let (one, two) = (first.id().unwrap(), second.id().unwrap());
        let pids = wait_for(&storage, "a1", 6);
        kill_tree(&storage, "a1", one);
        std::thread::sleep(Duration::from_millis(300));
        let left = super::pids(&storage, "a1");
        assert!(!alive(one) && !left.contains(&one) && left.contains(&two), "{pids:?} -> {left:?}");
        assert!(left.len() >= 3, "{left:?}");
        terminate(&storage, "a1");
    }

    #[tokio::test]
    async fn after_a_restart_the_processes_recorded_are_still_the_agents() {
        let storage = crate::testdir::for_this_test("job-restart");
        let _child = spawn_tree(&storage, "a1");
        let pids = wait_for(&storage, "a1", 3);
        remember(&storage, "a1");
        // A Loft that restarted holds no handle on the job, so it can't reach it.
        let name = name(&storage, "a1");
        JOBS.lock().unwrap().retain(|(n, _)| *n != name);
        assert!(in_job(&storage, "a1").is_empty());
        let known = super::pids(&storage, "a1");
        assert!(pids.iter().all(|p| known.contains(p)), "{pids:?} vs {known:?}");
        assert!(terminate(&storage, "a1"));
        for _ in 0..100 {
            if pids.iter().all(|&p| !alive(p)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(pids.iter().all(|&p| !alive(p)), "{pids:?}");
        // Once they have ended, none is the agent's, even with the same PID again.
        assert!(super::pids(&storage, "a1").is_empty());
    }

    #[test]
    fn the_table_knows_this_process_and_its_executable() {
        let me = std::process::id();
        let t = table();
        let p = t.iter().find(|p| p.pid == me).unwrap();
        assert!(p.exe.ends_with(".exe"), "{}", p.exe);
        // The test binary runs with arguments of its own (the test filter, `--test-threads`) or at least its path.
        let command = p.command.as_deref().unwrap_or_default();
        let file = p.exe.rsplit('\\').next().unwrap();
        assert!(command.contains(file.trim_end_matches(".exe")), "{command:?} vs {}", p.exe);
        assert!(alive(me));
    }

    #[test]
    fn jobs_are_named_per_storage_folder_and_agent() {
        let (a, b) = (Path::new(r"C:\Users\me\loft\agent-local-storage"), Path::new(r"C:\Users\me\test\agent-local-storage"));
        assert_ne!(name(a, "x"), name(b, "x"));
        assert_ne!(name(a, "x"), name(a, "y"));
        assert!(String::from_utf16_lossy(&name(a, "x")).starts_with(r"Local\loft-"));
    }
}
