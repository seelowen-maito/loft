//! Each agent's processes on Windows: a Job Object per agent, which stands in for the Unix process session (see
//! processes.rs). A run's claude starts suspended, joins the agent's job and only then runs, so every process it
//! starts, and theirs, are in the job too: Windows puts a child in its parent's job, and nothing in an agent's job may
//! leave it. A detached server (`nohup … &`) or a daemon stays in it, whatever its parent.
//!
//! The job is named after Loft's storage folder and the agent, so a Loft that restarted finds it again; it lasts while
//! any of its processes runs. A `<id>.job` file in the agents' files says which agents have one. Stopping an agent ends
//! its whole job at once: Windows has no SIGTERM to let its processes clean up.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
    Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList, OpenJobObjectW, QueryInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, GetProcessTimes, OpenProcess, OpenThread, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess, WaitForSingleObject,
};

/// How a run's claude is created: suspended until it is in the agent's job, and with no console window (Loft may run
/// with no console of its own, and each console program would open one).
pub const CREATION_FLAGS: u32 = CREATE_SUSPENDED | CREATE_NO_WINDOW;

/// The job access rights Loft asks for (winnt.h's, which windows-sys keeps in another feature).
const JOB_OBJECT_QUERY: u32 = 0x0004;
const JOB_OBJECT_TERMINATE: u32 = 0x0008;

/// The exit code of a process Loft ended.
const STOPPED: u32 = 1;

/// A handle, closed when dropped.
struct Handle(HANDLE);

impl Handle {
    fn new(h: HANDLE) -> Option<Self> {
        (!h.is_null() && h != INVALID_HANDLE_VALUE).then_some(Handle(h))
    }
}

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

/// Puts the suspended `child` (started with [`CREATION_FLAGS`]) in the agent's job, creating the job if it has none
/// now, then lets it run. On an error the child is still suspended: the caller ends it.
pub fn join(storage: &Path, agent_id: &str, child: &tokio::process::Child) -> std::io::Result<()> {
    let (Some(process), Some(pid)) = (child.raw_handle(), child.id()) else {
        return Err(std::io::Error::other("claude exited before it started"));
    };
    let marker = marker(storage, agent_id);
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&marker, "")?;
    let job = Handle::new(unsafe { CreateJobObjectW(std::ptr::null(), name(storage, agent_id).as_ptr()) }).ok_or_else(std::io::Error::last_os_error)?;
    if unsafe { AssignProcessToJobObject(job.0, process.cast()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    resume(pid);
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

fn open(storage: &Path, agent_id: &str, access: u32) -> Option<Handle> {
    Handle::new(unsafe { OpenJobObjectW(access, 0, name(storage, agent_id).as_ptr()) })
}

/// The agents that have had a job.
pub fn agents(storage: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(storage.join(crate::agents::AGENT_FILES)) else { return vec![] };
    dir.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".job").map(String::from)).collect()
}

/// The processes in the agent's job: none when it has no job (no process of its own is left).
pub fn pids(storage: &Path, agent_id: &str) -> Vec<u32> {
    let Some(job) = open(storage, agent_id, JOB_OBJECT_QUERY) else { return vec![] };
    // The list's two counts, then the IDs; grown until every assigned process fits.
    let mut room = 256usize;
    loop {
        let mut buf = vec![0usize; 2 + room];
        let size = (buf.len() * std::mem::size_of::<usize>()) as u32;
        let ok = unsafe { QueryInformationJobObject(job.0, JobObjectBasicProcessIdList, buf.as_mut_ptr().cast(), size, std::ptr::null_mut()) } != 0;
        let counts: [u32; 2] = unsafe { std::ptr::read(buf.as_ptr().cast()) };
        let (assigned, listed) = (counts[0] as usize, counts[1] as usize);
        if ok && listed >= assigned {
            return buf[2..2 + listed.min(room)].iter().map(|&p| p as u32).collect();
        }
        if room >= 1 << 16 {
            return buf[2..2 + listed.min(room)].iter().map(|&p| p as u32).collect();
        }
        room = (assigned + 64).max(room * 2);
    }
}

/// Ends every process in the agent's job. Whether it had one.
pub fn terminate(storage: &Path, agent_id: &str) -> bool {
    open(storage, agent_id, JOB_OBJECT_TERMINATE).is_some_and(|job| unsafe { TerminateJobObject(job.0, STOPPED) } != 0)
}

/// Forgets the agent's job, when the agent is deleted (after [`terminate`]).
pub fn forget(storage: &Path, agent_id: &str) {
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

/// Ends `pid`.
pub fn kill(pid: u32) -> std::io::Result<()> {
    let p = Handle::new(unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) }).ok_or_else(std::io::Error::last_os_error)?;
    if unsafe { TerminateProcess(p.0, STOPPED) } == 0 {
        return Err(std::io::Error::last_os_error());
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
        let (exe, age_secs) = details(e.th32ProcessID);
        list.push(Entry { pid: e.th32ProcessID, ppid: e.th32ParentProcessID, exe: exe.unwrap_or(file), age_secs });
        more = unsafe { Process32NextW(snap.0, &mut e) } != 0;
    }
    list
}

/// The full path of `pid`'s executable and how long it has run, when Loft may open it.
fn details(pid: u32) -> (Option<String>, u64) {
    let Some(p) = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }) else { return (None, 0) };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let exe = (unsafe { QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]));
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let age = if unsafe { GetProcessTimes(p.0, &mut created, &mut exited, &mut kernel, &mut user) } != 0 {
        // FILETIME counts 100 ns steps since 1601.
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        let started = SystemTime::UNIX_EPOCH + Duration::from_nanos(ticks.saturating_sub(116_444_736_000_000_000).saturating_mul(100));
        started.elapsed().map_or(0, |d| d.as_secs())
    } else {
        0
    };
    (exe, age)
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

    #[test]
    fn the_table_knows_this_process_and_its_executable() {
        let me = std::process::id();
        let t = table();
        let p = t.iter().find(|p| p.pid == me).unwrap();
        assert!(p.exe.ends_with(".exe"), "{}", p.exe);
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
