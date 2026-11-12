//! The `PreToolUse` hook on Bash that Loft gives every agent. Kills an agent runs by name (`kill`, `pkill`,
//! `killall`) go through Loft, which lets them stop only the agent's own processes and asks the user about any
//! other (processes.rs). This hook spots the kills that would skip it, because they don't run Loft's `kill`
//! function or the commands it puts first on PATH:
//! - the real command by path (`/bin/kill`, `/usr/bin/pkill`);
//! - through `sudo` or `doas`, which reset PATH, or `command` or `builtin`, which skip functions;
//! - a `kill` in another shell's script (`sh -c 'kill 123'`, `bash <<EOF`, `watch 'kill 1'`), where that shell's
//!   own builtin runs;
//! - with PATH emptied, changed or unset (`env -i kill 1`, `PATH=/bin; xargs kill`), which finds the real command;
//! - after Loft's functions are removed or replaced (`unset -f kill`, `unfunction kill`, `alias kill=...`).
//!
//! It refuses those, telling the agent to run the kill by name, where Loft asks the user about any process that
//! isn't the agent's. (A hook's "ask" doesn't reach the user: in auto mode Claude Code's classifier answers it.)
//! Everything else runs.
//!
//! Claude runs it before every Bash call, so it is a plain scan of the command's shell text, in this binary
//! (`loft kill-guard`) and before any runtime starts. It looks through wrappers (`xargs`, `find -exec`, `watch`,
//! `env -S`, ...), `sh -c`, `eval`, and shells that read their script from a heredoc or a piped `echo`.
//!
//! It also asks the user before a command changes one of the user's own project checkouts, which agents must leave
//! alone: they work in their own clones under `agent-local-storage`. Every git command but the ones that only read
//! (`status`, `log`, `diff`, `fetch`, `branch` listing, ...) and a build or install (`cargo build`, `pnpm install`,
//! `xcodebuild`) in a folder outside `agent-local-storage` goes to Loft (`POST /guard/<agent id>`), which asks the
//! user about each such folder under the projects folder. The folder is the command's working directory, as `cd`,
//! `cd -`, `pushd` and `popd` leave it (a subshell's stays in it), or the one a folder option names (`git -C`,
//! `--work-tree`, `--git-dir`, `GIT_DIR`, `GIT_WORK_TREE`, `--manifest-path`, `pnpm --dir`, `npm --prefix`).
//! Left out on purpose: `make`, `pnpm run <script>` and other scripts, which may do anything, and `export GIT_DIR`.
//!
//! Its limits, which the prompt rule covers: it reads shell text, so it doesn't see a script file the agent writes
//! and runs, a command name built at run time (`k=/bin/kill; $k 123`), or other languages sending a signal
//! (osascript, python, node). It fails open: input it can't read lets the command run.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// The argument that makes this binary run as the hook.
pub const ARG: &str = "kill-guard";
/// Where the hook asks Loft about a change to a user's checkout: `http://127.0.0.1:<port>/guard/<agent id>`, in
/// claude's environment.
pub const URL_VAR: &str = "LOFT_GUARD_URL";

const REFUSAL: &str = "Loft refused this command: it runs kill, pkill or killall by path, through sudo, `command` \
or `builtin`, as another shell's `kill`, with PATH changed, or after unsetting or redefining Loft's kill, where Loft \
can't tell whose processes it stops. Run kill, pkill or killall by name, right in your command: Loft lets them stop \
your own processes, and asks the user before stopping any other.";

/// Runs as the hook: reads Claude Code's hook input on stdin and exits 2, which blocks the call and shows stderr to
/// the agent, for a kill that skips Loft's or a change to a user's checkout the user didn't allow. Input it can't
/// read passes, and so does a change when Loft doesn't answer.
pub fn hook() -> ! {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if let Err(why) = check(&input) {
        eprintln!("{why}");
        std::process::exit(2);
    }
    let changes = outside_changes(&input);
    if let (false, Ok(url)) = (changes.is_empty(), std::env::var(URL_VAR)) {
        let body = json!({ "changes": changes });
        if let Ok(v) = crate::processes::post(&url, &body.to_string())
            && v["allow"] == false
        {
            eprintln!("{}", v["message"].as_str().unwrap_or_default());
            std::process::exit(2);
        }
    }
    std::process::exit(0)
}

/// The `hooks` of a settings file that installs the hook, run as `exe kill-guard`. It may wait for the user that long.
pub fn hooks(exe: &std::path::Path) -> Value {
    let command = format!("'{}' {ARG}", exe.to_string_lossy().replace('\'', r"'\''"));
    let timeout = crate::agents::TOOL_WAIT.as_secs();
    json!({ "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command, "timeout": timeout }] }] })
}

/// The changes a hook input's command makes to checkouts outside `agent-local-storage`.
fn outside_changes(input: &str) -> Vec<Change> {
    let Ok(v) = serde_json::from_str::<Value>(input) else { return vec![] };
    let Some(command) = v["tool_input"]["command"].as_str() else { return vec![] };
    let Some(cwd) = v["cwd"].as_str().map(PathBuf::from).or_else(|| std::env::current_dir().ok()) else { return vec![] };
    checkout_changes(command, &cwd).into_iter().filter(|c| !in_agent_storage(&c.folder)).collect()
}

fn check(input: &str) -> Result<(), &'static str> {
    let v: Value = serde_json::from_str(input).unwrap_or_default();
    match v["tool_input"]["command"].as_str() {
        Some(command) if unscoped(command) => Err(REFUSAL),
        _ => Ok(()),
    }
}

/// Whether `command` runs a kill that skips Loft's (see the module docs).
pub fn unscoped(command: &str) -> bool {
    let f = scan(command, 0, false);
    f.unscoped || (f.kill && f.path)
}

// ---- changes to a checkout ----------------------------------------------

/// A command that changes the checkout in `folder`.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub folder: PathBuf,
    /// The simple command that does it, as its words.
    pub command: String,
}

/// Package manager commands that build or install into the folder they run in.
const PM_CHANGES: &[&str] = &["install", "i", "ci", "add", "remove", "rm", "uninstall", "update", "up", "build"];
/// cargo commands that write into the folder (`target/`, `Cargo.lock`).
const CARGO_CHANGES: &[&str] = &["build", "b", "run", "r", "test", "t", "check", "c", "clean", "update", "install", "add", "remove"];

/// Where a command runs, as the commands before it leave it: its folder (None when the guard can't tell, as after
/// `cd "$dir"`), the folder before the last `cd` (for `cd -`) and the `pushd` stack. A `cd -` or `popd` past what the
/// command set goes to where it started: the agent's own folder, which may be inside the user's checkout.
#[derive(Clone, Default)]
struct Dirs {
    start: Option<PathBuf>,
    cwd: Option<PathBuf>,
    old: Option<PathBuf>,
    stack: Vec<Option<PathBuf>>,
}

impl Dirs {
    /// `path` from the current folder; an absolute one (`/x`, `~/x`, `$HOME/x`) even when that is unknown.
    fn at(&self, path: &str) -> Option<PathBuf> {
        let absolute = path.starts_with(['/', '~']) || path.starts_with("$HOME") || path.starts_with("${HOME}");
        resolve(if absolute { Path::new("/") } else { self.cwd.as_deref()? }, path)
    }

    fn cd(&mut self, to: Option<PathBuf>) {
        self.old = std::mem::replace(&mut self.cwd, to);
    }
}

/// Every change `command`, run in `cwd`, makes to a checkout (see the module docs), each folder once. A folder the
/// guard can't tell (`cd "$dir"`) lets the commands after it run.
pub fn checkout_changes(command: &str, cwd: &Path) -> Vec<Change> {
    let mut found = vec![];
    let start = Some(cwd.to_path_buf());
    changes(command, &mut Dirs { start: start.clone(), cwd: start, ..Default::default() }, 0, &mut found);
    let mut out: Vec<Change> = vec![];
    for c in found {
        // A symlink into a user's checkout counts as that checkout.
        let folder = real(c.folder);
        if !out.iter().any(|o| o.folder == folder) {
            out.push(Change { folder, ..c });
        }
    }
    out
}

fn changes(text: &str, dirs: &mut Dirs, depth: usize, out: &mut Vec<Change>) {
    if depth > MAX_DEPTH {
        return;
    }
    let cmds = parse(text);
    // The folders as each open subshell found them, to go back to when it ends.
    let mut saved: Vec<(usize, Dirs)> = vec![];
    for i in 0..cmds.len() {
        let scope = &cmds[i].scope;
        while saved.last().is_some_and(|(s, _)| !scope.contains(s)) {
            *dirs = saved.pop().unwrap().1;
        }
        for &s in &scope[saved.len()..] {
            saved.push((s, dirs.clone()));
        }
        let all = &cmds[i].words;
        let words = match unwrap(all, &mut false) {
            Run::Words(words) => words,
            Run::Text(text) => {
                changes(&text, dirs, depth + 1, out);
                continue;
            }
        };
        // `GIT_DIR=x git ...`, `env GIT_WORK_TREE=x git ...`
        let vars: Vec<&String> = all[..all.len() - words.len()].iter().filter(|w| assignment(w).is_some()).collect();
        let Some((name, args)) = words.split_first() else { continue };
        let folders = match base(name) {
            "cd" => {
                let to = match args.iter().find(|a| !matches!(a.as_str(), "-L" | "-P" | "-e" | "-@")).map(String::as_str) {
                    None => dirs::home_dir(),
                    Some("-") => dirs.old.clone().or(dirs.start.clone()),
                    Some(dir) => dirs.at(dir),
                };
                dirs.cd(to);
                continue;
            }
            "pushd" => {
                let to = match args.iter().find(|a| *a != "-n").map(String::as_str) {
                    // `pushd` swaps the top two folders; `pushd +1` rotates the stack.
                    None => dirs.stack.pop().unwrap_or_default(),
                    Some(d) if d.starts_with(['+', '-']) => None,
                    Some(dir) => dirs.at(dir),
                };
                let from = dirs.cwd.clone();
                dirs.stack.push(from);
                dirs.cd(to);
                continue;
            }
            "popd" => {
                let to = match dirs.stack.pop() {
                    Some(d) if args.is_empty() => d,
                    _ => dirs.start.clone(),
                };
                dirs.cd(to);
                continue;
            }
            "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                if let Some(t) = shell_script(&cmds, i, args) {
                    changes(&t, &mut dirs.clone(), depth + 1, out);
                }
                continue;
            }
            "eval" => {
                changes(&args.join(" "), dirs, depth + 1, out);
                continue;
            }
            "git" => git_changes(dirs, &vars, args),
            "cargo" => match flags(args, &["--manifest-path"]) {
                (manifest, Some(s)) if CARGO_CHANGES.contains(&s) => match manifest {
                    Some(m) => dirs.at(m).and_then(|m| m.parent().map(Path::to_path_buf)).into_iter().collect(),
                    None => dirs.cwd.clone().into_iter().collect(),
                },
                _ => vec![],
            },
            pm @ ("npm" | "pnpm" | "yarn" | "bun") => {
                let dir_flags: &[&str] = match pm {
                    "npm" => &["--prefix"],
                    "pnpm" => &["--dir", "-C"],
                    _ => &["--cwd"],
                };
                match flags(args, dir_flags) {
                    (dir, Some(s)) if PM_CHANGES.contains(&s) => dir.map_or(dirs.cwd.clone(), |d| dirs.at(d)).into_iter().collect(),
                    (dir, None) if pm == "yarn" => dir.map_or(dirs.cwd.clone(), |d| dirs.at(d)).into_iter().collect(), // a bare `yarn` installs
                    _ => vec![],
                }
            }
            "xcodebuild" => {
                let reads = ["-list", "-showBuildSettings", "-version", "-showsdks", "-showdestinations", "-help"];
                if args.iter().any(|a| reads.contains(&a.as_str())) { vec![] } else { dirs.cwd.clone().into_iter().collect() }
            }
            _ => vec![],
        };
        out.extend(folders.into_iter().map(|folder| Change { folder, command: words.join(" ") }));
    }
}

/// The value of the first of the folder `options` in `args` (`--dir x` or `--dir=x`), and the first other word
/// that isn't an option: the subcommand.
fn flags<'a>(args: &'a [String], options: &[&str]) -> (Option<&'a str>, Option<&'a str>) {
    let (mut value, mut sub) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if options.contains(&a.as_str()) {
            value = value.or(it.next().map(String::as_str));
        } else if let Some(v) = options.iter().find_map(|o| a.strip_prefix(o).and_then(|r| r.strip_prefix('='))) {
            value = value.or(Some(v));
        } else if !a.starts_with(['-', '+']) {
            sub = sub.or(Some(a.as_str()));
        }
    }
    (value, sub)
}

/// The folders a git command changes, past its global options: its work tree (`--work-tree`, `GIT_WORK_TREE`) and
/// the checkout its repository is in (`--git-dir`, `GIT_DIR`), each by default the repository git finds from the
/// folder (`-C`): the nearest one up, which for an agent's workspace that isn't a clone is the user's checkout around
/// it. Every subcommand changes them but the ones that only read (`git_reads`).
fn git_changes(dirs: &Dirs, vars: &[&String], args: &[String]) -> Vec<PathBuf> {
    let mut dirs = dirs.clone();
    let (mut tree, mut git_dir) = (None, None);
    for v in vars {
        if let Some(d) = v.strip_prefix("GIT_WORK_TREE=") {
            tree = Some(dirs.at(d));
        } else if let Some(d) = v.strip_prefix("GIT_DIR=") {
            git_dir = Some(dirs.at(d));
        }
    }
    let mut it = args.iter();
    let sub = loop {
        let Some(a) = it.next() else { return vec![] };
        let value = |o: &str| a.strip_prefix(o).and_then(|r| r.strip_prefix('='));
        match a.as_str() {
            "-C" => {
                let to = it.next().and_then(|d| dirs.at(d));
                dirs.cwd = to;
            }
            "--work-tree" => tree = Some(it.next().and_then(|d| dirs.at(d))),
            "--git-dir" => git_dir = Some(it.next().and_then(|d| dirs.at(d))),
            _ if value("--work-tree").is_some() => tree = Some(value("--work-tree").and_then(|d| dirs.at(d))),
            _ if value("--git-dir").is_some() => git_dir = Some(value("--git-dir").and_then(|d| dirs.at(d))),
            "-c" | "--namespace" | "--config-env" | "--super-prefix" => {
                it.next();
            }
            o if o.starts_with('-') => {}
            sub => break sub,
        }
    };
    let rest: Vec<&str> = it.map(String::as_str).collect();
    if git_reads(sub, &rest) || (sub == "config" && rest.iter().any(|a| matches!(*a, "--global" | "--system"))) {
        return vec![];
    }
    // `clone` and `init` make a new repository: in the folder they name, or the folder they run in, not the one around it.
    if let Some(valued) = match sub {
        "clone" => Some(&["-b", "--branch", "-o", "--origin", "-c", "--config", "--depth", "--reference", "--reference-if-able", "--template", "--separate-git-dir", "-u", "--upload-pack", "-j", "--jobs", "--filter", "--shallow-since", "--shallow-exclude", "--server-option", "--bundle-uri"][..]),
        "init" => Some(&["-b", "--initial-branch", "--template", "--separate-git-dir", "--object-format", "--ref-format"][..]),
        _ => None,
    } {
        let (mut words, mut git_dir) = (vec![], None);
        let mut it = rest.iter();
        while let Some(a) = it.next() {
            if *a == "--separate-git-dir" {
                git_dir = it.next().copied();
            } else if let Some(d) = a.strip_prefix("--separate-git-dir=") {
                git_dir = Some(d);
            } else if valued.contains(a) {
                it.next();
            } else if !a.starts_with('-') {
                words.push(*a);
            }
        }
        // `clone <repository> [<folder>]`, `init [<folder>]`, and the repository's own folder when it's apart.
        let target = words.get(if sub == "clone" { 1 } else { 0 });
        let target = target.map_or(dirs.cwd.clone(), |t| dirs.at(t));
        return [target, git_dir.and_then(|d| dirs.at(d))].into_iter().flatten().collect();
    }
    let found = dirs.cwd.map(repo_root);
    // The checkout a `.git` folder belongs to.
    let repo = git_dir.unwrap_or(found.clone()).map(|d| if d.ends_with(".git") { d.parent().map(Path::to_path_buf).unwrap_or(d) } else { d });
    [tree.unwrap_or(found), repo].into_iter().flatten().collect()
}

/// `path` with symlinks followed, even when it doesn't exist yet (a folder `clone` will make): its nearest folder that
/// exists is resolved, and the rest put back on. A dangling symlink on the way counts as where it points.
fn real(path: PathBuf) -> PathBuf {
    real_at(path, 0)
}

fn real_at(path: PathBuf, links: usize) -> PathBuf {
    for base in path.ancestors() {
        let rest = path.strip_prefix(base).unwrap_or(Path::new(""));
        if let Ok(r) = std::fs::canonicalize(base) {
            return r.join(rest);
        }
        // As many links as the system follows (a loop never resolves).
        if links < 40
            && let Ok(to) = std::fs::read_link(base)
        {
            let to = base.parent().map_or(to.clone(), |p| p.join(&to));
            return real_at(to.join(rest), links + 1);
        }
    }
    path
}

/// The checkout git works on from `folder`: the nearest folder up holding a `.git`, or `folder` when none does.
/// Symlinks are followed first, as git does.
fn repo_root(folder: PathBuf) -> PathBuf {
    let folder = real(folder);
    folder.ancestors().find(|d| d.join(".git").exists()).map(Path::to_path_buf).unwrap_or(folder)
}

/// Whether a git subcommand, with its arguments, only reads.
fn git_reads(sub: &str, rest: &[&str]) -> bool {
    let has = |opts: &[&str]| rest.iter().any(|a| opts.contains(a));
    let words = || rest.iter().filter(|a| !a.starts_with('-'));
    let first = rest.first().copied();
    match sub {
        "status" | "log" | "diff" | "show" | "fetch" | "rev-parse" | "rev-list" | "ls-files" | "ls-tree" | "ls-remote" | "blame"
        | "grep" | "describe" | "shortlog" | "cat-file" | "show-ref" | "for-each-ref" | "merge-base" | "name-rev"
        | "range-diff" | "cherry" | "check-ignore" | "count-objects" | "var" | "help" | "version" => true,
        // Listing: no name given, or a listing option (`--merged` and the like imply `--list`).
        "branch" => {
            let lists = ["--list", "-l", "--show-current", "--contains", "--no-contains", "--merged", "--no-merged", "--points-at"];
            let changes = ["-d", "-D", "--delete", "-m", "-M", "--move", "-c", "-C", "--copy", "-f", "--force", "-u", "--set-upstream-to", "--unset-upstream", "--edit-description"];
            !has(&changes) && !rest.iter().any(|a| a.starts_with("--set-upstream-to=")) && (words().next().is_none() || has(&lists))
        }
        "tag" => !has(&["-d", "--delete", "-a", "-s", "-f", "--force", "-m", "-F", "-u"]) && (words().next().is_none() || has(&["-l", "--list"])),
        "remote" => matches!(first, None | Some("-v" | "--verbose" | "show" | "get-url")),
        "stash" => matches!(first, Some("list" | "show")),
        "clean" => has(&["-n", "--dry-run"]),
        "worktree" => first == Some("list"),
        "submodule" => matches!(first, Some("status" | "summary")),
        "reflog" => matches!(first, None | Some("show" | "exists")),
        // `config <key>` reads it; `config <key> <value>` sets it.
        "config" => {
            let sets = rest.iter().any(|a| ["--unset", "--add", "--replace", "--remove", "--rename", "--edit", "-e"].iter().any(|o| a.starts_with(o)));
            has(&["--get", "--get-all", "--get-regexp", "--get-urlmatch", "--list", "-l"])
                || matches!(first, Some("get" | "list"))
                || (!sets && words().count() == 1)
        }
        _ => false,
    }
}

/// `path` from `cwd`, `~` and `$HOME` expanded and `..` folded. None when it holds another variable or a substitution.
fn resolve(cwd: &Path, path: &str) -> Option<PathBuf> {
    let home = || dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
    let path = match path {
        p if p == "~" || p.starts_with("~/") => format!("{}{}", home()?, &p[1..]),
        p if p.starts_with("$HOME") => format!("{}{}", home()?, &p[5..]),
        p if p.starts_with("${HOME}") => format!("{}{}", home()?, &p[7..]),
        p => p.to_string(),
    };
    if path.contains(['$', '`']) {
        return None;
    }
    let mut out = PathBuf::new();
    for c in cwd.join(path).components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    Some(out)
}

/// Whether `folder` is in an agent's workspace: under a folder named `agent-local-storage`.
pub fn in_agent_storage(folder: &Path) -> bool {
    folder.components().any(|c| c.as_os_str() == "agent-local-storage")
}

/// What a piece of shell text, or one simple command in it, does that the guard cares about.
#[derive(Default, Clone, Copy)]
struct Found {
    /// A kill that skips Loft's on its own: by path, through `sudo`, `command` or `builtin`, with PATH reset, in
    /// another shell, or after Loft's functions are removed or replaced (`unset -f kill`, `alias kill=...`).
    unscoped: bool,
    /// Any kill, pkill or killall.
    kill: bool,
    /// Changes PATH (`PATH=/bin; xargs kill` finds the real kill), or unsets it.
    path: bool,
}

impl Found {
    fn add(&mut self, f: Found) {
        self.unscoped |= f.unscoped;
        self.kill |= f.kill;
        self.path |= f.path;
    }
}

/// Shell text nested deeper than this (`sh -c "eval '...'"`) isn't scanned.
const MAX_DEPTH: usize = 8;

/// `shell`: the text runs in a shell of its own, which has no `kill` function from Loft.
fn scan(text: &str, depth: usize, shell: bool) -> Found {
    let mut f = Found::default();
    if depth > MAX_DEPTH {
        return f;
    }
    let cmds = parse(text);
    for i in 0..cmds.len() {
        f.add(scan_command(&cmds, i, depth, shell));
    }
    f
}

const KILLS: [&str; 3] = ["kill", "pkill", "killall"];

/// What one simple command does, through its wrappers and any shell text it hands on.
fn scan_command(cmds: &[Cmd], i: usize, depth: usize, shell: bool) -> Found {
    let mut f = Found::default();
    // A command that only assigns, PATH among them (`PATH=/bin; ...`, `path+=(/bin)`).
    f.path |= cmds[i].words.iter().all(|w| assignment(w).is_some()) && cmds[i].words.iter().any(|w| sets_path(w));
    let mut runs = vec![cmds[i].words.clone()];
    while let Some(words) = runs.pop() {
        let mut bypass = false;
        let words = match unwrap(&words, &mut bypass) {
            Run::Words(words) => words,
            Run::Text(text) => {
                f.add(scan(&text, depth + 1, true));
                continue;
            }
        };
        let Some((name, args)) = words.split_first() else { continue };
        match base(name) {
            k if KILLS.contains(&k) => {
                f.kill = true;
                f.unscoped |= bypass || name.contains('/') || (k == "kill" && shell);
            }
            "find" => runs.extend(find_execs(args)),
            "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                if let Some(text) = shell_script(cmds, i, args) {
                    f.add(scan(&text, depth + 1, true));
                }
            }
            "eval" => f.add(scan(&args.join(" "), depth + 1, shell)),
            // Removing or replacing Loft's functions, or pointing the name at the real command.
            // Only a name that is exactly kill, pkill or killall counts: `unset skills` or `alias skillcheck=...`
            // stop nothing.
            "unset" | "unfunction" | "unhash" | "disable" | "enable" | "alias" | "hash" | "functions" => {
                let maps = matches!(base(name), "alias" | "hash");
                let mut it = args.iter();
                while let Some(a) = it.next() {
                    if a.starts_with(['-', '+']) {
                        // bash's `hash -p /bin/kill k` points `k` at the path: the text after the `p`
                        // (`hash -p/bin/kill k`), or else the next word.
                        if let Some((_, attached)) = a.split_once('p').filter(|_| base(name) == "hash" && a.starts_with('-')) {
                            let path = if attached.is_empty() { it.next().map(String::as_str) } else { Some(attached) };
                            if let Some(path) = path {
                                f.add(scan(path, depth + 1, shell));
                            }
                        }
                        continue;
                    }
                    let (n, value) = a.split_once('=').filter(|_| maps).unzip();
                    let n = n.unwrap_or(a.as_str());
                    // zsh's `unset 'functions[kill]'` removes the function too.
                    f.unscoped |= KILLS.contains(&n) || subscript(n).is_some_and(|k| KILLS.contains(&k));
                    // What an alias or a hashed name runs: `alias k=/bin/kill` and zsh's `hash k=/bin/kill` reach
                    // the real kill under another name.
                    if let Some(value) = value {
                        f.add(scan(value, depth + 1, shell));
                    }
                }
                f.path |= name == "unset" && args.iter().any(|a| a == "PATH" || a == "path");
            }
            "export" | "typeset" | "declare" | "local" | "readonly" => f.path |= args.iter().any(|a| sets_path(a)),
            // zsh's `functions[kill]='/bin/kill "$@"'` redefines Loft's kill.
            _ => {
                let key = name.split_once('=').and_then(|(k, _)| subscript(k.strip_suffix('+').unwrap_or(k)));
                f.unscoped |= key.is_some_and(|k| KILLS.contains(&k));
            }
        }
    }
    f
}

/// Whether a word assigns PATH (or zsh's `path` array) so that another folder comes before Loft's: anything but
/// adding to the end (`PATH="$PATH:/opt/bin"`).
fn sets_path(w: &str) -> bool {
    let Some(name @ ("PATH" | "path")) = assignment(w) else { return false };
    let value = w.split_once('=').map_or("", |(_, v)| v);
    !(name == "PATH" && (value.starts_with("$PATH") || value.starts_with("${PATH}")) || w.starts_with("PATH+="))
}

/// The commands `find` runs with `-exec`, `-execdir`, `-ok` and `-okdir`.
fn find_execs(args: &[String]) -> Vec<Vec<String>> {
    let mut execs = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            execs.push(it.by_ref().take_while(|w| *w != ";" && *w != "+").cloned().collect());
        }
    }
    execs
}

/// The script a shell runs, when the guard can read it: its `-c` text, or what it reads on stdin (a heredoc, a
/// herestring, or an `echo`, `printf` or `cat <<EOF` piped in). None when it runs a script file.
fn shell_script(cmds: &[Cmd], i: usize, args: &[String]) -> Option<String> {
    let (mut rest, mut stdin) = (args, false);
    while let Some((a, tail)) = rest.split_first() {
        if a == "--" {
            rest = tail;
            break;
        }
        if !a.starts_with(['-', '+']) {
            break;
        }
        rest = tail;
        let flags = &a[1..];
        if a.starts_with("--") {
            if a == "--rcfile" || a == "--init-file" {
                rest = rest.get(1..).unwrap_or_default();
            }
            continue;
        }
        if a.starts_with('-') && flags.contains('c') {
            return rest.iter().find(|a| *a != "--").cloned(); // `sh -c -- 'script'`
        }
        stdin |= a.starts_with('-') && flags.contains('s');
        if flags.ends_with(['o', 'O']) {
            rest = rest.get(1..).unwrap_or_default();
        }
    }
    if !rest.is_empty() && !stdin {
        return None;
    }
    let c = &cmds[i];
    let mut text = c.stdin.clone();
    for up in cmds.iter().filter(|up| up.pipes.iter().any(|&(p, at)| c.pipes.contains(&(p, at + 1)))) {
        if let Run::Words([name, args @ ..]) = unwrap(&up.words, &mut false) {
            match base(name) {
                "echo" | "printf" => text += &args.join(" ").replace("\\n", "\n"),
                "cat" => text += &up.stdin,
                _ => {}
            }
            text.push('\n');
        }
    }
    Some(text)
}

fn base(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or_default()
}

/// The key of an element of a zsh special parameter that holds functions or aliases (`functions[kill]`,
/// `dis_functions[kill]`, `aliases[kill]`), quotes removed.
fn subscript(w: &str) -> Option<&str> {
    let (param, key) = w.strip_suffix(']')?.split_once('[')?;
    let params = ["functions", "dis_functions", "functions_source", "aliases", "dis_aliases", "galiases"];
    params.contains(&param).then(|| key.trim_matches(['"', '\'', '\\']))
}

/// The name a `NAME=value` (or `NAME+=value`) word assigns.
fn assignment(w: &str) -> Option<&str> {
    let (k, _) = w.split_once('=')?;
    let k = k.strip_suffix('+').unwrap_or(k);
    (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some(k)
}

const KEYWORDS: &[&str] = &["!", "{", "}", "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "esac"];

/// Whether the next word after `words` is in command position, where a keyword counts: past keywords and a
/// `function NAME`.
fn command_position(words: &[String]) -> bool {
    let words = match words {
        [f, _, rest @ ..] if f == "function" => rest,
        _ => words,
    };
    words.iter().all(|w| KEYWORDS.contains(&w.as_str()))
}

/// What a simple command runs once its wrappers are peeled off.
enum Run<'a> {
    Words(&'a [String]),
    /// Shell text a wrapper runs (`watch 'pkill x'`, `env -S '...'`).
    Text(String),
}

/// The command a simple command runs: past variable assignments, shell keywords and wrappers that run the
/// command they're given (`sudo`, `env`, `xargs`, `nohup`, `watch`, ...). `command -v` runs nothing. `bypass` is set
/// when a wrapper skips the shell's functions or PATH (`sudo`, `doas`, `command`, `builtin`).
fn unwrap<'a>(words: &'a [String], bypass: &mut bool) -> Run<'a> {
    let rest = |i: usize| words.get(i..).unwrap_or_default();
    let mut i = 0;
    while let Some(w) = words.get(i) {
        if assignment(w).is_some() || KEYWORDS.contains(&w.as_str()) {
            // `PATH=/bin xargs kill` finds the real kill.
            *bypass |= sets_path(w);
            i += 1;
            continue;
        }
        if w == "function" {
            i += 2; // and the function's name: `function f { pkill x; }`
            continue;
        }
        // Each wrapper with its options that take a value.
        let name = base(w);
        let valued: &[&str] = match name {
            "sudo" => &["-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T"],
            "doas" => &["-u", "-C"],
            "env" => &["-u", "-C", "-P"],
            "xargs" => &["-n", "-I", "-L", "-P", "-s", "-E", "-J", "-R", "-S", "-d", "-a"],
            "parallel" => &["-j", "-n", "-N", "-S", "-I", "-a", "-d", "-E", "--jobs", "--delay", "--timeout"],
            "nice" => &["-n"],
            "timeout" => &["-s", "-k"],
            "caffeinate" => &["-t", "-w"],
            "exec" => &["-a"],
            "stdbuf" => &["-i", "-o", "-e"],
            "arch" => &["-arch", "-e", "-d"],
            "watch" => &["-n", "--interval"],
            "script" => &["-t", "-T", "-E", "-I", "-O", "-B"],
            "command" | "builtin" | "nohup" | "time" | "chronic" => &[],
            _ => return Run::Words(rest(i)),
        };
        *bypass |= matches!(name, "sudo" | "doas" | "command" | "builtin");
        i += 1;
        while let Some(o) = words.get(i).filter(|o| o.starts_with('-')) {
            i += 1;
            match (name, o.as_str()) {
                (_, "--") => break,
                ("command", o) if o.contains(['v', 'V']) => return Run::Words(&[]),
                ("env", "-S" | "--split-string") => return Run::Text(rest(i).join(" ")),
                ("env", o) if o.starts_with("-S") || o.starts_with("--split-string=") => {
                    let split = o.strip_prefix("-S").or(o.strip_prefix("--split-string=")).unwrap_or_default();
                    return Run::Text(format!("{split} {}", rest(i).join(" ")));
                }
                ("script", "-c" | "--command") => return Run::Text(rest(i).first().cloned().unwrap_or_default()),
                // A command run with an emptied environment, another PATH or a variable unset (`env -u PATH`).
                ("env", o) if o == "-" || o.starts_with("--ignore") || o.starts_with("--unset") || (!o.starts_with("--") && o.contains(['i', 'P', 'u'])) => {
                    *bypass = true;
                    if valued.contains(&o) {
                        i += 1;
                    }
                }
                (_, o) if valued.contains(&o) => i += 1,
                _ => {}
            }
        }
        match name {
            "timeout" | "script" => i += 1, // the duration, the typescript file
            "watch" => return Run::Text(rest(i).join(" ")),
            "parallel" => return Run::Text(rest(i).iter().take_while(|w| !w.starts_with(":::")).cloned().collect::<Vec<_>>().join(" ")),
            _ => {}
        }
    }
    Run::Words(rest(i))
}

/// One simple command, and where its input comes from and its output goes.
#[derive(Default)]
struct Cmd {
    /// Its words with quotes removed. Redirections aren't words.
    words: Vec<String>,
    /// Each pipeline it sits in, from the outermost (one a compound command it's in sits in) in: the pipeline and
    /// its place there.
    pipes: Vec<(usize, usize)>,
    /// Its heredoc bodies and herestrings.
    stdin: String,
    /// The subshells it runs in (`( )`, `$( )`, backticks), outermost first, by number: a `cd` there stays there.
    scope: Vec<usize>,
}

#[derive(Clone, Copy)]
enum Redirect {
    /// The target is a file or a file descriptor (`2>&1`).
    File,
    /// `<<<`: the target is a herestring.
    Here,
}

/// A pipeline being built: one at the top, and one more inside each compound command open (`{`, `(`, `while`, `if`).
#[derive(Default)]
struct Frame {
    pipe: Option<usize>,
    /// The next command's place in it.
    place: usize,
    /// In a `case`: whether the next words are a pattern (up to its `)`).
    pattern: Option<bool>,
}

#[derive(Default)]
struct Parse {
    cmds: Vec<Cmd>,
    pipes: usize,
    /// The subshells open, and how many there have been.
    scope: Vec<usize>,
    scopes: usize,
}

impl Parse {
    fn open_scope(&mut self) {
        self.scopes += 1;
        self.scope.push(self.scopes);
    }
}

/// Splits shell text into its simple commands, including the commands inside `$(...)`, backticks and `<(...)`.
fn parse(text: &str) -> Vec<Cmd> {
    let c: Vec<char> = text.chars().collect();
    let mut p = Parse::default();
    lex(&mut p, &c, 0, None);
    p.cmds
}

/// The state of one `lex`: the top level or one substitution.
struct Level {
    /// The command being lexed.
    cur: Option<usize>,
    word: String,
    in_word: bool,
    /// What the next word is a redirection's target of.
    redirect: Option<Redirect>,
    frames: Vec<Frame>,
    /// Heredocs started on this line: their delimiter and command.
    heredocs: Vec<(String, usize)>,
}

impl Level {
    fn new() -> Self {
        Level { cur: None, word: String::new(), in_word: false, redirect: None, frames: vec![Frame::default()], heredocs: vec![] }
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    /// The command being lexed, started if need be.
    fn cmd(&mut self, p: &mut Parse) -> usize {
        *self.cur.get_or_insert_with(|| {
            p.cmds.push(Cmd { scope: p.scope.clone(), ..Default::default() });
            p.cmds.len() - 1
        })
    }

    fn end_word(&mut self, p: &mut Parse) {
        if !std::mem::take(&mut self.in_word) {
            self.word.clear();
            return;
        }
        let word = std::mem::take(&mut self.word);
        let id = self.cmd(p);
        let cmd = &mut p.cmds[id];
        match self.redirect.take() {
            Some(Redirect::File) => {}
            Some(Redirect::Here) => {
                cmd.stdin += &word;
                cmd.stdin.push('\n');
            }
            None => self.command_word(p, id, word),
        }
    }

    /// Adds a word to the command, opening or closing a compound command when it's a keyword in command position.
    fn command_word(&mut self, p: &mut Parse, id: usize, word: String) {
        if self.frame().pattern == Some(true) && word != "esac" {
            return; // a `case` pattern
        }
        let words = &mut p.cmds[id].words;
        let first = command_position(words);
        let opens_case = word == "in" && words.len() >= 2 && words[words.len() - 2] == "case" && command_position(&words[..words.len() - 2]);
        if first {
            match word.as_str() {
                "{" | "while" | "until" | "if" | "for" | "select" => self.frames.push(Frame::default()),
                "}" | "done" | "fi" | "esac" if self.frames.len() > 1 => drop(self.frames.pop()),
                _ => {}
            }
        }
        words.push(word);
        if opens_case {
            self.end(p);
            self.frames.push(Frame { pattern: Some(true), ..Default::default() });
        }
    }

    /// Ends the command being lexed: it takes its place in each open pipeline.
    fn end(&mut self, p: &mut Parse) {
        self.end_word(p);
        if let Some(id) = self.cur.take() {
            for f in &mut self.frames {
                let pipe = *f.pipe.get_or_insert_with(|| {
                    p.pipes += 1;
                    p.pipes
                });
                p.cmds[id].pipes.push((pipe, f.place));
            }
        }
    }

    /// `;`, `&&`, `||`, `&` or a newline: the next command starts a new pipeline.
    fn separate(&mut self, p: &mut Parse) {
        self.end(p);
        let f = self.frame();
        (f.pipe, f.place) = (None, 0);
    }

    /// `|`: the next command reads this one's output.
    fn pipe(&mut self, p: &mut Parse) {
        self.end(p);
        self.frame().place += 1;
    }
}

/// Lexes from `i` to the end, or past the `stop` (`)` or a backtick) that closes a substitution, adding the
/// commands it finds to `p`. Returns the index it stopped at.
fn lex(p: &mut Parse, c: &[char], mut i: usize, stop: Option<char>) -> usize {
    let mut l = Level::new();
    let scope = p.scope.len();
    if stop.is_some() {
        p.open_scope();
    }
    let mut depth = 0; // subshell parens open inside a `$(`
    let mut array = 0; // parens open in an array assignment, `pids=($(pgrep x))`: its elements stay in the word
    while let Some(&ch) = c.get(i) {
        let next = c.get(i + 1).copied();
        i += 1;
        let opens_array = ch == '(' && l.in_word && l.word.ends_with('=') && assignment(&l.word).is_some();
        // A `#` that starts an element comments out the rest of the line.
        if array > 0 && ch == '#' && i >= 2 && matches!(c[i - 2], ' ' | '\t' | '\n' | '(') {
            while c.get(i).is_some_and(|&n| n != '\n') {
                i += 1;
            }
            continue;
        }
        if opens_array || (array > 0 && matches!(ch, '(' | ')' | ' ' | '\t' | '\n')) {
            match ch {
                '(' => array += 1,
                ')' => array -= 1,
                _ => {}
            }
            l.word.push(ch);
            continue;
        }
        // In a `case` pattern (`pkill|killall)`), `|` and `(` are part of it and `)` ends it.
        if l.frame().pattern == Some(true) && matches!(ch, '|' | '(' | ')') {
            l.end_word(p);
            if ch == ')' {
                l.frame().pattern = Some(false);
            }
            continue;
        }
        match ch {
            '\\' => {
                if let Some(n) = next {
                    if n != '\n' {
                        l.word.push(n);
                        l.in_word = true;
                    }
                    i += 1;
                }
            }
            '$' if next == Some('\'') => {
                l.in_word = true;
                i = ansi_c_quoted(c, i + 1, &mut l.word);
            }
            '\'' => {
                l.in_word = true;
                while let Some(&q) = c.get(i) {
                    i += 1;
                    if q == '\'' {
                        break;
                    }
                    l.word.push(q);
                }
            }
            '"' => {
                l.in_word = true;
                while let Some(&q) = c.get(i) {
                    i += 1;
                    match q {
                        '"' => break,
                        '\\' => {
                            l.word.extend(c.get(i));
                            i += 1;
                        }
                        '$' if c.get(i) == Some(&'(') => {
                            l.cmd(p);
                            i = lex(p, c, i + 1, Some(')'));
                        }
                        '`' => {
                            l.cmd(p);
                            i = lex(p, c, i, Some('`'));
                        }
                        _ => l.word.push(q),
                    }
                }
            }
            '`' if stop == Some('`') => break,
            '`' => {
                l.in_word = true;
                l.cmd(p);
                i = lex(p, c, i, Some('`'));
            }
            '$' | '<' | '>' if next == Some('(') => {
                l.in_word = true;
                l.cmd(p);
                i = lex(p, c, i + 1, Some(')'));
            }
            '$' if next == Some('{') => {
                l.in_word = true;
                l.word.push('$');
                while let Some(&n) = c.get(i) {
                    i += 1;
                    l.word.push(n);
                    if n == '}' {
                        break;
                    }
                }
            }
            '#' if !l.in_word => {
                while c.get(i).is_some_and(|&n| n != '\n') {
                    i += 1;
                }
            }
            ' ' | '\t' => l.end_word(p),
            '(' => {
                if stop == Some(')') {
                    depth += 1;
                }
                l.end(p);
                l.frames.push(Frame::default());
                p.open_scope();
            }
            ')' if stop == Some(')') && depth == 0 => break,
            ')' => {
                if stop == Some(')') {
                    depth -= 1;
                }
                l.end(p);
                if l.frames.len() > 1 {
                    l.frames.pop();
                }
                if p.scope.len() > scope + stop.is_some() as usize {
                    p.scope.pop();
                }
            }
            ';' => {
                l.separate(p);
                // `;;`, `;&`, `;;&` end a `case` branch: a pattern comes next.
                if matches!(next, Some(';' | '&')) {
                    i += 1;
                    if c.get(i) == Some(&'&') {
                        i += 1;
                    }
                    if l.frame().pattern.is_some() {
                        l.frame().pattern = Some(true);
                    }
                }
            }
            '&' if next == Some('>') => {
                l.end_word(p);
                i = redirect(&mut l, c, i + 1, '>');
            }
            '&' => {
                if next == Some('&') {
                    i += 1;
                }
                l.separate(p);
            }
            '|' if next == Some('|') => {
                i += 1;
                l.separate(p);
            }
            '|' => {
                if next == Some('&') {
                    i += 1;
                }
                l.pipe(p);
            }
            '\n' => {
                l.separate(p);
                i = read_heredocs(p, c, i, &mut l.heredocs);
            }
            '<' | '>' => {
                if l.word.chars().all(|d| d.is_ascii_digit()) {
                    l.in_word = false; // `2>`: a file descriptor, not a word
                }
                l.end_word(p);
                if ch == '<' && next == Some('<') && c.get(i + 1) != Some(&'<') {
                    i += 1;
                    if c.get(i) == Some(&'-') {
                        i += 1;
                    }
                    while c.get(i).is_some_and(|&n| n == ' ' || n == '\t') {
                        i += 1;
                    }
                    let mut delimiter = String::new();
                    while let Some(&n) = c.get(i).filter(|&&n| !n.is_whitespace() && !";&|()<>".contains(n)) {
                        if !"'\"\\".contains(n) {
                            delimiter.push(n);
                        }
                        i += 1;
                    }
                    let id = l.cmd(p);
                    l.heredocs.push((delimiter, id));
                } else {
                    i = redirect(&mut l, c, i, ch);
                }
            }
            _ => {
                l.word.push(ch);
                l.in_word = true;
            }
        }
    }
    l.end(p);
    p.scope.truncate(scope);
    i
}

/// Reads a `$'...'` string from `i` (past its opening quote) into `word`, with its escapes (`\n`, `\x70`, `\160`)
/// decoded. Returns the index past its closing quote.
fn ansi_c_quoted(c: &[char], mut i: usize, word: &mut String) -> usize {
    while let Some(&q) = c.get(i) {
        i += 1;
        match q {
            '\'' => break,
            '\\' => {
                let Some(&e) = c.get(i) else { break };
                i += 1;
                let digits = |i: &mut usize, radix: u32, max: usize| {
                    let start = *i;
                    while *i - start < max && c.get(*i).is_some_and(|d| d.is_digit(radix)) {
                        *i += 1;
                    }
                    let s: String = c[start..*i].iter().collect();
                    u32::from_str_radix(&s, radix).ok().and_then(char::from_u32)
                };
                match e {
                    'n' => word.push('\n'),
                    't' => word.push('\t'),
                    'x' => word.extend(digits(&mut i, 16, 2)),
                    'u' => word.extend(digits(&mut i, 16, 4)),
                    'U' => word.extend(digits(&mut i, 16, 8)),
                    '0'..='7' => {
                        i -= 1;
                        word.extend(digits(&mut i, 8, 3));
                    }
                    _ => word.push(e),
                }
            }
            _ => word.push(q),
        }
    }
    i
}

/// Reads the rest of a redirection operator that started with `ch` (`>>`, `2>&`, `<<<`), from `i`, and marks the
/// next word as its target. Returns the index past the operator.
fn redirect(l: &mut Level, c: &[char], mut i: usize, ch: char) -> usize {
    let mut op = String::new();
    while let Some(&n) = c.get(i).filter(|&&n| "<>&|".contains(n)) {
        op.push(n);
        i += 1;
    }
    l.redirect = Some(if ch == '<' && op == "<<" { Redirect::Here } else { Redirect::File });
    i
}

/// Reads the bodies of the heredocs started on the line that ended just before `i` into their commands' stdin.
fn read_heredocs(p: &mut Parse, c: &[char], mut i: usize, heredocs: &mut Vec<(String, usize)>) -> usize {
    for (delimiter, id) in heredocs.drain(..) {
        while i < c.len() {
            let end = c[i..].iter().position(|&n| n == '\n').map_or(c.len(), |p| i + p);
            let line: String = c[i..end].iter().collect();
            i = end + 1;
            if line.trim() == delimiter {
                break;
            }
            p.cmds[id].stdin += &line;
            p.cmds[id].stdin.push('\n');
        }
    }
    i.min(c.len())
}

// ---- Loft's side: `POST /guard/<agent id>` ------------------------------

#[derive(serde::Deserialize)]
pub struct GuardRequest {
    changes: Vec<Change>,
}

/// Asks the user about each change to a checkout under the projects folder (that isn't an agent's workspace), one
/// after the other within one `ASK_TIMEOUT` (the hook's own timeout covers one), and answers the hook with whether
/// the command may run. It is Allow once or Deny only: there is no rule to save.
pub async fn handle(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    axum::extract::Path(agent_id): axum::extract::Path<String>,
    axum::Json(req): axum::Json<GuardRequest>,
) -> axum::Json<Value> {
    let projects = state.settings.get().projects_dir;
    let projects = std::fs::canonicalize(&projects).unwrap_or(projects);
    let deadline = tokio::time::Instant::now() + ASK_TIMEOUT;
    for change in req.changes {
        // Read again each time: an "Always allow" for one folder may cover the next.
        let Ok(Some(a)) = state.db.get_agent(&agent_id) else { break };
        if !change.folder.starts_with(&projects) || in_agent_storage(&change.folder) {
            continue;
        }
        let folder = change.folder.display().to_string();
        let reason = format!("{} wants to change your checkout at {folder}, not its own clone: {}", a.name, change.command);
        let input = json!({ "command": change.command, "folder": folder });
        let wait = deadline.saturating_duration_since(tokio::time::Instant::now());
        let d = crate::agents::ask_permission_for(&state, &a.id, "prompt", "Checkout", input, &reason, String::new(), wait).await;
        if d.allow {
            continue;
        }
        let why = d.message.unwrap_or_else(|| "The user denied it.".into());
        let ws = crate::agents::workspace(&state, &a.id);
        let message = format!(
            "Loft didn't run this: it changes {folder}, the user's own checkout, so it asked the user. {why} Work in your own clone in your workspace ({}): clone the project there if it isn't yet.",
            ws.display()
        );
        return axum::Json(json!({ "allow": false, "message": message }));
    }
    axum::Json(json!({ "allow": true }))
}

/// How long a change to a user's checkout waits for the user.
#[cfg(not(test))]
const ASK_TIMEOUT: std::time::Duration = crate::agents::PERMISSION_TIMEOUT;
#[cfg(test)]
const ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[cfg(test)]
mod tests {
    use super::*;

    // These are only ever scanned as text: nothing here runs a command.

    #[test]
    fn kills_by_name_run_through_loft_without_asking() {
        // Loft's `kill`, `pkill` and `killall` functions, or the commands of those names first on PATH, get these.
        for cmd in [
            "pkill -f vite",
            "killall node",
            "kill 1234",
            "kill -9 $!",
            "kill %1",
            "kill -9 -1",
            "kill 0",
            "cd x && pkill -f foo",
            "kill $(pgrep -f next)",
            "pgrep -f vite | xargs kill",
            "pgrep -f vite | xargs -n 1 kill -9",
            "PID=$(pgrep -f vite); kill $PID",
            "lsof -ti tcp:3000 -sTCP:LISTEN | xargs kill",
            "(cd /tmp; pkill x)",
            "echo $(kill 12)",
            "eval \"kill 12\"",
            "env FOO=1 pkill x",
            "nohup killall x >/dev/null 2>&1 &",
            "timeout 5 kill 12",
            "nice -n 5 kill 12",
            "exec kill 12",
            "find . -name x -exec kill {} \\;",
            "f() { kill 12; }; f",
            "if true; then killall node; fi",
            // Another shell finds pkill and killall on PATH: only its `kill` is a builtin.
            "bash -c 'pkill -f x'",
            "echo 'killall node' | sh",
            "watch 'pkill x'",
            "pgrep x | xargs -I{} sh -c 'echo {}'",
        ] {
            assert!(!unscoped(cmd), "should run: {cmd}");
        }
    }

    #[test]
    fn kills_that_skip_loft_are_refused() {
        for cmd in [
            "/bin/kill 1234",
            "/usr/bin/pkill -f vite",
            "cd x && /usr/bin/killall node",
            "sudo kill 1234",
            "sudo -u me pkill x",
            "doas killall node",
            "command kill 12",
            "builtin kill 12",
            "nohup /bin/kill 12 &",
            "pgrep x | xargs /bin/kill",
            "find . -exec /bin/kill {} \\;",
            "sh -c 'kill 12'",
            "bash -c -- \"kill -9 $PID\"",
            "zsh -lc 'cd /tmp; kill 12'",
            "bash -eo pipefail -c 'kill 12'",
            "PID=$(cat p); sh -c \"kill $PID\"",
            "pgrep x | xargs -I{} sh -c 'kill {}'",
            "echo 'kill 12' | sh",
            "printf 'cd /tmp\\nkill 12\\n' | zsh",
            "bash <<'EOF'\ncd /tmp\nkill 12\nEOF",
            "bash <<< 'kill 12'",
            "cat <<'EOF' | bash\nkill 12\nEOF",
            "watch -n 1 'kill 12'",
            "script -c 'kill 12' /dev/null",
            "eval \"sh -c 'kill 12'\"",
            "$'/bin/kill' 12",
            "sh -c \"eval 'kill 12'\"",
            // PATH emptied, changed or unset: the real kill is found.
            "env -i kill 123",
            "env - kill 123",
            "env --ignore-environment pkill x",
            "env PATH=/bin kill 123",
            "env -P /bin kill 123",
            "env -u PATH kill 123",
            "env --unset=PATH xargs kill",
            "PATH=/bin:/usr/bin xargs kill",
            "PATH=/bin; pgrep x | xargs kill",
            "export PATH=/bin:$PATH; echo 12 | xargs kill",
            "path=(/bin $path); find . -exec kill {} +",
            "unset PATH; echo 12 | xargs kill",
            "typeset -x PATH=/bin; xargs kill < pids",
            "sh -c 'PATH=/bin; xargs kill' < pids",
            // Loft's functions removed or replaced.
            "unset -f kill; kill 123",
            "unset -f pkill killall",
            "unset -f kill",
            "unset -fv -- kill",
            "unfunction pkill",
            "alias kill=/bin/kill",
            "alias pkill='command pkill'",
            "alias -g killall=true",
            "alias k='sudo kill'",
            "hash -d killall",
            "functions +x kill",
            "unset 'functions[kill]'",
            "unset \"functions[pkill]\"",
            "unset -v 'functions[killall]'",
            "unset 'functions[\"kill\"]'",
            "unset 'dis_functions[kill]'",
            "unset 'aliases[kill]'",
            "unhash -f kill",
            "unhash kill; kill 12",
            "functions[kill]='/bin/kill \"$@\"'; kill 12",
            "aliases[pkill]='command pkill'",
            "unfunction kill; kill 123",
            "disable kill",
            "disable -f kill; kill 12",
            "alias k=/bin/kill; k 12",
            "hash -p /bin/kill kill",
            "hash -p /bin/kill k",
            "hash -p /bin/kill k; k 12",
            "hash -rp /bin/kill k",
            "hash -p/bin/kill k",
            "hash -p/bin/kill k; k 12",
            "hash -rp/bin/kill k",
            "hash k=/bin/kill; k 12",
            "hash -r k=/usr/bin/pkill",
            "kill() { /bin/kill \"$@\"; }; kill 12",
            "function kill { command kill \"$@\"; }",
        ] {
            assert!(unscoped(cmd), "should refuse: {cmd}");
        }
    }

    #[test]
    fn path_changes_without_a_kill_and_appends_run() {
        for cmd in [
            "export PATH=\"$PWD/node_modules/.bin:$PATH\"; npm test",
            "PATH=/opt/bin:$PATH cargo build",
            "export PATH=\"$PATH:$HOME/bin\"; kill 12",
            "PATH+=:/opt/bin; kill 12",
            "PATH=${PATH}:/opt/bin pgrep x | xargs kill",
            "env FOO=1 kill 12",
            "env -C /tmp kill 12",
            "unset FOO; kill 12",
            "unalias ll; kill 12",
        ] {
            assert!(!unscoped(cmd), "should run: {cmd}");
        }
    }

    #[test]
    fn unsets_and_aliases_of_other_names_run() {
        // A name that merely contains "kill" leaves Loft's kill alone.
        for cmd in [
            "unset skills",
            "unset -f install_skills",
            "alias skillcheck='cargo test'",
            "alias killer=ls",
            "unfunction killjoy",
            "hash -d skill",
            "hash -p /usr/bin/true t; t",
            "hash -p/usr/bin/true t",
            "disable -f pkill_all",
            "functions skillset",
            "alias k=kill; k 12",
            "unset 'functions[skill]'",
            "unhash -f skillcheck",
            "functions[skillcheck]='cargo test'",
            "arr[kill]=1",
        ] {
            assert!(!unscoped(cmd), "should run: {cmd}");
        }
    }

    #[test]
    fn mentions_and_lookups_run() {
        for cmd in [
            "pgrep -f vite",
            "ps aux | grep vite",
            "grep -r pkill src",
            "echo \"don't pkill\"",
            "echo 'sudo kill 1'",
            "git commit -m \"$(cat <<'EOF'\nNever /bin/kill (or sudo kill)\nEOF\n)\"",
            "cat <<EOF > notes.md\nsh -c 'kill 1'\nEOF",
            "echo 'kill 12' > notes.md",
            "echo 'kill 12' | grep kill",
            "bash script.sh <<EOF\nkill 12\nEOF",
            "command -v kill",
            "command -V killall",
            "type pkill",
            "man kill",
            "case $1 in kill) echo no;; esac",
            "# /bin/kill 12\nls",
            "rg '/bin/kill' backend/src",
        ] {
            assert!(!unscoped(cmd), "should run: {cmd}");
        }
    }

    #[test]
    fn the_hook_refuses_only_a_kill_that_skips_loft() {
        let input = |cmd: &str| json!({ "tool_name": "Bash", "tool_input": { "command": cmd } }).to_string();
        assert!(check(&input("cd x && sudo kill -9 1234")).unwrap_err().contains("Run kill, pkill or killall by name"));
        // Kills by name, even of a pattern, go through Loft.
        for cmd in ["pkill -f vite", "killall node", "kill 1234", "pgrep x | xargs kill"] {
            assert!(check(&input(cmd)).is_ok(), "{cmd}");
        }
        assert!(check("not json").is_ok());
        assert!(check("{}").is_ok());
    }

    #[test]
    fn settings_run_this_binary_as_the_bash_hook() {
        let hooks = hooks(std::path::Path::new("/Apps/it's here/loft"));
        let hook = &hooks["PreToolUse"][0];
        assert_eq!(hook["matcher"], "Bash");
        assert_eq!(hook["hooks"][0]["command"], r"'/Apps/it'\''s here/loft' kill-guard");
        // It may wait for the user about a change to their checkout.
        assert_eq!(hook["hooks"][0]["timeout"], crate::agents::TOOL_WAIT.as_secs());
    }

    // A user's checkout and an agent's clone of it, as the Loft workspace sits inside the user's repo.
    const USER: &str = "/nowhere/dev/loft";
    const CLONE: &str = "/nowhere/dev/loft/agent-local-storage/abc/loft";

    /// The folders the hook would ask Loft about, for `cmd` run in `cwd`.
    fn asks_all(cmd: &str, cwd: &str) -> Vec<PathBuf> {
        let input = json!({ "tool_name": "Bash", "cwd": cwd, "tool_input": { "command": cmd } }).to_string();
        outside_changes(&input).into_iter().map(|c| c.folder).collect()
    }

    /// The one folder the hook would ask Loft about.
    fn asks(cmd: &str, cwd: &str) -> Option<PathBuf> {
        let all = asks_all(cmd, cwd);
        assert!(all.len() <= 1, "{cmd}: {all:?}");
        all.into_iter().next()
    }

    #[test]
    fn changes_to_a_users_checkout_go_to_loft_with_the_folder() {
        let user = Some(PathBuf::from(USER));
        for (cmd, cwd) in [
            // What happened on 2026-11-12.
            ("cd /nowhere/dev/loft; git checkout --detach 3ddf495c", CLONE),
            ("cd ../../..; git checkout main", CLONE),
            ("cd ../../.. && cargo build", CLONE),
            ("git -C /nowhere/dev/loft switch main", CLONE),
            ("git -C ../../.. pull --ff-only", CLONE),
            ("git --work-tree=/nowhere/dev/loft reset --hard", CLONE),
            ("git reset --hard origin/main", USER),
            ("git pull", USER),
            ("git merge feat", USER),
            ("git rebase main", USER),
            ("git commit -am wip", USER),
            ("git stash", USER),
            ("git stash pop", USER),
            ("git clean -fdx", USER),
            ("git -c core.pager=cat checkout -b x", USER),
            ("cargo build --release", USER),
            ("cargo test", USER),
            ("pnpm install", USER),
            ("npm ci", USER),
            ("yarn", USER),
            ("xcodebuild -scheme Loft -destination 'platform=iOS Simulator,id=1' build", USER),
            ("sh -c 'cd /nowhere/dev/loft && git pull'", CLONE),
            ("eval 'git checkout main'", USER),
            ("git status && git checkout main", USER),
        ] {
            assert_eq!(asks(cmd, cwd), user, "should ask: {cmd} (in {cwd})");
        }
        assert_eq!(asks("cd frontend && pnpm install", USER), Some(PathBuf::from("/nowhere/dev/loft/frontend")));
    }

    #[test]
    fn the_agents_own_clone_and_reading_run_without_asking() {
        for (cmd, cwd) in [
            // In the agent's own clone.
            ("git checkout --detach 3ddf495c", CLONE),
            ("git reset --hard origin/main && cargo build", CLONE),
            ("cd /nowhere/dev/loft/agent-local-storage/abc/loft && pnpm install", USER),
            ("git -C /nowhere/dev/loft/agent-local-storage/abc/loft switch main", USER),
            ("cd ..; git clone /nowhere/dev/loft", CLONE),
            // Reading the user's checkout.
            ("git status", USER),
            ("git log --oneline -5", USER),
            ("git diff HEAD~1", USER),
            ("git fetch origin", USER),
            ("git stash list", USER),
            ("git clean -n", USER),
            ("git -C /nowhere/dev/loft show HEAD", CLONE),
            ("cat README.md && rg checkout backend/src", USER),
            ("cargo --version", USER),
            ("xcodebuild -list", USER),
            ("pnpm run dev", USER),
            // A folder the guard can't tell.
            ("cd \"$DIR\" && git pull", CLONE),
        ] {
            assert_eq!(asks(cmd, cwd), None, "should run: {cmd} (in {cwd})");
        }
        assert!(asks("git pull", USER).is_some());
        assert!(outside_changes("not json").is_empty());
    }

    #[test]
    fn every_folder_a_command_changes_is_asked_about() {
        let (other, user) = (PathBuf::from("/nowhere/dev/other"), PathBuf::from(USER));
        assert_eq!(asks_all("git -C /nowhere/dev/other pull; git -C /nowhere/dev/loft pull", CLONE), [other.clone(), user.clone()]);
        assert_eq!(asks_all("git pull && cd ../other && cargo build && git -C ../loft pull", USER), [user.clone(), other]);
        // Each folder once.
        assert_eq!(asks_all("cd /nowhere/dev/loft; git pull; cargo build; pnpm install", CLONE), [user]);
    }

    #[test]
    fn cd_dash_pushd_and_popd_follow_the_folder() {
        let user = Some(PathBuf::from(USER));
        for cmd in [
            "cd /nowhere/dev/loft; cd /tmp; cd -; git pull",
            "pushd /nowhere/dev/loft; git pull",
            "pushd /nowhere/dev/loft; pushd /tmp; popd; git pull",
            "pushd /nowhere/dev/loft; pushd /tmp; pushd; git pull",
            "cd /nowhere/dev/loft; cd; cd -; git pull",
        ] {
            assert_eq!(asks(cmd, CLONE), user, "should ask: {cmd}");
        }
        for cmd in [
            // Back in the clone.
            "pushd /nowhere/dev/loft; popd; git pull",
            "cd /nowhere/dev/loft; cd -; git pull",
            // Where the shell was before this command can't be told: the folder it started in stands for it.
            "cd -; git pull",
            "popd; git pull",
        ] {
            assert_eq!(asks(cmd, CLONE), None, "should run: {cmd}");
        }
        assert_eq!(asks("cd -; git pull", USER), user);
        assert_eq!(asks("cd /tmp; popd; git pull", USER), user);
    }

    #[test]
    fn a_subshells_cd_stays_in_it() {
        let user = Some(PathBuf::from(USER));
        for cmd in ["(cd /nowhere/dev/loft); git pull", "(cd /nowhere/dev/loft && ls); git pull", "echo $(cd /nowhere/dev/loft; pwd); git pull", "x=`cd /nowhere/dev/loft`; git pull"] {
            assert_eq!(asks(cmd, CLONE), None, "should run: {cmd}");
        }
        for cmd in ["(cd /nowhere/dev/loft && git pull)", "(cd /tmp); cd /nowhere/dev/loft; git pull", "(cd /tmp; (cd /x)); cd /nowhere/dev/loft && git pull", "{ cd /nowhere/dev/loft; }; git pull"] {
            assert_eq!(asks(cmd, CLONE), user, "should ask: {cmd}");
        }
    }

    #[test]
    fn git_dir_and_work_tree_name_the_checkout() {
        let user = Some(PathBuf::from(USER));
        for cmd in [
            "git --git-dir=/nowhere/dev/loft/.git checkout main",
            "git --git-dir /nowhere/dev/loft/.git reset --hard",
            "GIT_DIR=/nowhere/dev/loft/.git git checkout main",
            "GIT_DIR=../../../.git git pull",
            "env GIT_WORK_TREE=/nowhere/dev/loft git checkout .",
            "GIT_WORK_TREE=/nowhere/dev/loft GIT_DIR=.git git checkout .",
            "git --work-tree /nowhere/dev/loft checkout .",
        ] {
            assert!(asks_all(cmd, CLONE).contains(&PathBuf::from(USER)), "should ask: {cmd}");
        }
        assert_eq!(asks("git --git-dir=/nowhere/dev/loft/.git log", CLONE), None);
        assert_eq!(asks("GIT_DIR=/nowhere/dev/loft/.git git status", CLONE), None);
        assert_eq!(asks("git --git-dir=/nowhere/dev/loft/agent-local-storage/abc/loft/.git pull", USER), user);
    }

    #[test]
    fn build_tools_folder_options_name_the_folder() {
        let user = Some(PathBuf::from(USER));
        for cmd in [
            "cargo build --manifest-path /nowhere/dev/loft/Cargo.toml",
            "cargo test --manifest-path=../../../Cargo.toml",
            "pnpm --dir /nowhere/dev/loft install",
            "pnpm -C /nowhere/dev/loft build",
            "pnpm install --dir=/nowhere/dev/loft",
            "npm --prefix /nowhere/dev/loft ci",
            "npm install --prefix=/nowhere/dev/loft",
            "yarn --cwd /nowhere/dev/loft",
        ] {
            assert_eq!(asks(cmd, CLONE), user, "should ask: {cmd}");
        }
        assert_eq!(asks("cargo build --manifest-path backend/Cargo.toml", USER), Some(PathBuf::from("/nowhere/dev/loft/backend")));
        assert_eq!(asks("pnpm -C /nowhere/dev/loft run dev", CLONE), None);
        assert_eq!(asks("pnpm -C frontend install", CLONE), None);
    }

    #[test]
    fn every_git_subcommand_asks_but_the_ones_that_only_read() {
        let user = Some(PathBuf::from(USER));
        for cmd in [
            "git add -A", "git rm x", "git mv a b", "git apply p.diff", "git branch -D main", "git branch -f main HEAD~1",
            "git branch -m old new", "git branch feat", "git tag -d v1", "git tag v2", "git worktree add ../x",
            "git worktree remove x", "git submodule update", "git config user.name x", "git config --unset user.name",
            "git gc", "git update-ref refs/heads/main HEAD", "git bisect start", "git init", "git notes add",
            "git remote add up x", "git stash push", "git reflog expire --all", "git -C /nowhere/dev/loft branch -D x",
        ] {
            assert_eq!(asks(cmd, USER), user, "should ask: {cmd}");
        }
        for cmd in [
            "git status", "git log -5", "git diff", "git show HEAD", "git fetch", "git rev-parse HEAD", "git ls-files",
            "git blame x", "git grep foo", "git describe", "git remote -v", "git remote", "git remote get-url origin",
            "git branch", "git branch -a", "git branch -vv", "git branch --show-current", "git branch --list 'feat/*'",
            "git branch --merged main", "git tag", "git tag -l 'v*'", "git worktree list", "git config user.name",
            "git config --get user.name", "git config --list", "git stash show", "git reflog", "git --version",
            "git help", "git submodule status", "git -c color.ui=never log",
        ] {
            assert_eq!(asks(cmd, USER), None, "should run: {cmd}");
        }
    }

    #[test]
    fn git_in_a_workspace_that_isnt_a_clone_changes_the_checkout_around_it() {
        let project = std::env::temp_dir().join(format!("loft-guard-{}", uuid::Uuid::new_v4())).join("loft");
        let ws = project.join("agent-local-storage/x");
        let clone = ws.join("loft");
        for d in [project.join(".git"), ws.join("notes"), clone.join(".git"), clone.join("backend")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let project = std::fs::canonicalize(&project).unwrap();
        let at = |p: &Path| p.display().to_string();
        for cwd in [&ws, &ws.join("notes")] {
            for cmd in ["git checkout main", "git reset --hard origin/main", "git stash"] {
                assert_eq!(asks(cmd, &at(cwd)), Some(project.clone()), "should ask: {cmd} (in {})", cwd.display());
            }
            assert_eq!(asks("git status", &at(cwd)), None);
            assert_eq!(asks("git -C loft pull", &at(&ws)), None);
        }
        // A new repository is where it's made, not in the checkout around it; global config is no checkout's.
        for cmd in ["git clone /nowhere/dev/loft", "git clone https://github.com/x/loft.git", "git clone -b main /src loft2", "git init scratch-repo", "git init", "git config --global user.name x", "git config --system core.x y"] {
            assert_eq!(asks(cmd, &at(&ws)), None, "should run: {cmd}");
        }
        assert_eq!(asks(&format!("git clone /src {}", at(&project.join("sub"))), &at(&ws)), Some(project.join("sub")));
        assert_eq!(asks("git init", &at(&project)), Some(project.clone()));
        assert_eq!(asks("git config user.name x", &at(&ws)), Some(project.clone()));
        // A symlink in the workspace to a folder of another checkout: git acts on the checkout around the target.
        let other = project.parent().unwrap().join("other");
        std::fs::create_dir_all(other.join(".git")).unwrap();
        std::fs::create_dir_all(other.join("src")).unwrap();
        std::os::unix::fs::symlink(other.join("src"), ws.join("link")).unwrap();
        assert_eq!(asks("cd link && git reset --hard", &at(&ws)), Some(other.clone()));
        assert_eq!(asks("git -C link checkout main", &at(&ws)), Some(other.clone()));
        // A new folder through that link is in the other checkout, though it doesn't exist yet.
        let new = std::fs::canonicalize(other.join("src")).unwrap().join("new");
        assert_eq!(asks("git clone /src link/new", &at(&ws)), Some(new.clone()));
        assert_eq!(asks("git init link/new", &at(&ws)), Some(new));
        // A dangling symlink: where it points, not where it sits.
        std::os::unix::fs::symlink(other.join("missing"), ws.join("dangling")).unwrap();
        std::os::unix::fs::symlink("../../../other/gone", ws.join("relative")).unwrap();
        assert_eq!(asks("git clone /src dangling", &at(&ws)), Some(other.join("missing")));
        assert_eq!(asks("git init dangling/sub", &at(&ws)), Some(other.join("missing/sub")));
        assert_eq!(asks("git clone /src relative", &at(&ws)), Some(other.join("gone")));
        // A loop of links doesn't hang: it stays where it is.
        std::os::unix::fs::symlink(ws.join("loop"), ws.join("loop")).unwrap();
        assert_eq!(asks("git clone /src loop", &at(&ws)), None);
        // A repository kept apart from its folder is judged by where it goes too.
        let apart = project.join("repo.git");
        for cmd in [format!("git clone --separate-git-dir {} /src mine", at(&apart)), format!("git init --separate-git-dir={} mine", at(&apart))] {
            assert_eq!(asks(&cmd, &at(&ws)), Some(apart.clone()), "should ask: {cmd}");
        }
        assert_eq!(asks("git clone --separate-git-dir ../x/mine.git /src mine", &at(&ws)), None);
        // Before this command, the shell may have been anywhere: the workspace stands for it.
        for cmd in ["cd -; git checkout main", "cd loft; cd /tmp; popd; git stash", "cd loft && popd && git reset --hard"] {
            assert_eq!(asks(cmd, &at(&ws)), Some(project.clone()), "should ask: {cmd}");
        }
        assert_eq!(asks("cd -; git checkout main", &at(&clone)), None);
        // In the agent's clone, or a folder of it: its own repository.
        assert_eq!(asks("git checkout main", &at(&clone)), None);
        assert_eq!(asks("git reset --hard", &at(&clone.join("backend"))), None);
        // A work tree given from the workspace: git still finds the user's repository from the folder.
        assert_eq!(asks(&format!("git --work-tree={} checkout .", at(&clone)), &at(&ws)), Some(project.clone()));
        std::fs::remove_dir_all(project.parent().unwrap()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loft_asks_the_user_about_a_change_under_the_projects_folder() {
        use axum::extract::{Path as P, State};
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Web QA");
        let projects = std::fs::canonicalize(st.settings.get().projects_dir).unwrap();
        let user = projects.join("loft");
        let guard = |folder: PathBuf| {
            let (st, id) = (st.clone(), a.id.clone());
            let req = GuardRequest { changes: vec![Change { folder, command: "git checkout --detach 3ddf495c".into() }] };
            tokio::spawn(async move { handle(State(st), P(id), axum::Json(req)).await.0 })
        };
        let pending = || st.db.list_permissions(Some(&a.id)).unwrap().into_iter().filter(|p| p.status == "pending").collect::<Vec<_>>();

        // Outside the projects folder, or in an agent's workspace there: no question.
        assert_eq!(guard(PathBuf::from("/tmp/elsewhere")).await.unwrap()["allow"], true);
        assert_eq!(guard(user.join("agent-local-storage/x/loft")).await.unwrap()["allow"], true);
        assert!(pending().is_empty());

        // Denied: the agent is told to use its own clone.
        let asking = guard(user.clone());
        crate::agents::tests::until("a permission request", 5, || pending().len() == 1).await;
        let req = pending().remove(0);
        assert_eq!(req.tool_name, "Checkout");
        assert_eq!(req.input["folder"], user.display().to_string());
        assert!(req.reason.contains("git checkout --detach 3ddf495c"), "{}", req.reason);
        crate::agents::decide(&st, &req.id, "deny", None, None).unwrap();
        let v = asking.await.unwrap();
        assert_eq!(v["allow"], false);
        assert!(v["message"].as_str().unwrap().contains("Work in your own clone in your workspace"), "{v}");

        // No rule to save: Always allow is refused, even with a rule posted, and the agent's tools stay as they were.
        let asking = guard(user.clone());
        crate::agents::tests::until("a second permission request", 5, || pending().len() == 1).await;
        let id = pending()[0].id.clone();
        assert_eq!(pending()[0].rule, "");
        let tools = st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools;
        assert!(crate::agents::decide(&st, &id, "allow_always", None, None).is_err());
        assert!(crate::agents::decide(&st, &id, "allow_always", Some(format!("Checkout({})", user.display())), None).is_err());
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools, tools);

        // Allowed once: this command runs, and the next change there asks again.
        crate::agents::decide(&st, &id, "allow_once", None, None).unwrap();
        assert_eq!(asking.await.unwrap()["allow"], true);
        let again = guard(user.join("frontend"));
        crate::agents::tests::until("the next change asking again", 5, || pending().len() == 1).await;
        crate::agents::decide(&st, &pending()[0].id, "deny", None, None).unwrap();
        assert_eq!(again.await.unwrap()["allow"], false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_checkout_a_command_changes_is_asked_about_in_turn() {
        use axum::extract::{Path as P, State};
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Web QA");
        let projects = std::fs::canonicalize(st.settings.get().projects_dir).unwrap();
        let (one, two) = (projects.join("one"), projects.join("two"));
        let change = |f: &PathBuf| Change { folder: f.clone(), command: "git pull".into() };
        let pending = || st.db.list_permissions(Some(&a.id)).unwrap().into_iter().filter(|p| p.status == "pending").collect::<Vec<_>>();
        let req = GuardRequest { changes: vec![change(&one), change(&two)] };
        let asking = tokio::spawn(handle(State(st.clone()), P(a.id.clone()), axum::Json(req)));

        // Allowing the first doesn't let the second run unasked.
        crate::agents::tests::until("the first question", 5, || pending().len() == 1).await;
        assert_eq!(pending()[0].input["folder"], one.display().to_string());
        crate::agents::decide(&st, &pending()[0].id, "allow", None, None).unwrap();
        crate::agents::tests::until("the second question", 5, || pending().first().is_some_and(|p| p.input["folder"] == two.display().to_string())).await;
        crate::agents::decide(&st, &pending()[0].id, "deny", None, None).unwrap();
        let v = asking.await.unwrap().0;
        assert_eq!(v["allow"], false);
        assert!(v["message"].as_str().unwrap().contains(&two.display().to_string()), "{v}");
    }
}
