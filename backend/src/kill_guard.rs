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
//! Left out on purpose: `make`, `pnpm run <script>` and other scripts, which may do anything.
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
    // Loft gives every run a GIT_CEILING_DIRECTORIES (agents.rs), which claude passes on to this hook and the command.
    let ceiling: Vec<PathBuf> = std::env::var_os("GIT_CEILING_DIRECTORIES").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    let changes = outside_changes(&input, &ceiling);
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

/// The changes a hook input's command makes to checkouts outside `agent-local-storage`, run under the git `ceiling`.
fn outside_changes(input: &str, ceiling: &[PathBuf]) -> Vec<Change> {
    let Ok(v) = serde_json::from_str::<Value>(input) else { return vec![] };
    let Some(command) = v["tool_input"]["command"].as_str() else { return vec![] };
    let Some(cwd) = v["cwd"].as_str().map(PathBuf::from).or_else(|| std::env::current_dir().ok()) else { return vec![] };
    // Loft's ceiling is the storage folder.
    let storage = ceiling.first().map(|c| real(c.clone())).unwrap_or_default();
    checkout_changes(command, &cwd, ceiling).into_iter().filter(|c| !in_agent_storage(&c.folder, &storage)).collect()
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
    /// The folders this command's `cd`s left, each of which zsh's autopushd may have stacked.
    left: Vec<Option<PathBuf>>,
    /// A `popd`, a bare `pushd` or a `cd` that may fail ran: `cd -` may go to a folder the guard's `old` doesn't hold.
    popped: bool,
    /// The folders git doesn't look in or above for a repository, from a folder below one (GIT_CEILING_DIRECTORIES).
    ceiling: Vec<PathBuf>,
    /// The storage folder Loft uses (the run's ceiling, whatever the command does to git's).
    storage: PathBuf,
    /// Other folders the shell may be in: `cd lnk/..` goes to the folder holding `lnk` for the shell and to the one
    /// holding what `lnk` points at for git and the kernel.
    alts: Vec<PathBuf>,
    /// A command ran before this one: it may have made a folder, a link or a `.git` that isn't there yet.
    made: bool,
    /// An earlier command wrote a `.git` (`echo 'gitdir: x' > t/.git`): git may follow it anywhere.
    dotgit: bool,
    /// An earlier `cd` named a folder the guard can't list (a glob, braces).
    unknown: bool,
    /// What earlier `ln`, `mv`, `cp`, `rm` and the like of this command name, as written and with links followed: a folder
    /// among them (or below one) may be swapped for a link before git runs.
    touched: Vec<PathBuf>,
    /// What earlier `rm`, `rmdir`, `unlink` and the source of `mv` name, as written and with links followed: a `cd` through
    /// one and up (`gone/..`) fails.
    removed: Vec<PathBuf>,
    /// The `&&` list the command being checked is in (see `Cmd::chain`).
    chain: usize,
    /// Where the shell stays when a `cd` fails: not for the rest of that `cd`'s `&&` list, which then doesn't run, but for
    /// what follows it (`pending_chain` is the list). They become `alts` at the first command of another list.
    pending: Vec<PathBuf>,
    pending_chain: usize,
    /// The command being checked is negated (`! cd x`) or not last in its pipeline: a failed `cd` doesn't stop the list.
    goes_on: bool,
    /// An earlier `ln`, `mv`, `cp` or `rm` names a path the guard can't tell (`"$LOCK"`).
    unreadable: bool,
    /// A function named `cd`, `pushd`, `popd` or `chdir` was defined, or `disable` ran: a `cd` may not be the builtin.
    shadowed: bool,
    /// The shell has `cd`'d (or `pushd`/`popd`'d) since the command began.
    moved: bool,
    /// GIT_DIR or GIT_WORK_TREE appears somewhere in the command so far (in any word, assigned, exported, unset or
    /// hidden in a name): git may use a repository the guard can't follow, so every git call after it is unknown.
    git_env: bool,
}

impl Dirs {
    /// `path` from the current folder; an absolute one (`/x`, `~/x`, `$HOME/x`) even when that is unknown. With `..`
    /// after a link going up from where the link points, as git and the kernel do.
    fn at(&self, path: &str) -> Option<PathBuf> {
        self.resolve(path, true)
    }

    /// The same with `..` folded as text, as the shell does for `cd` and Node does for a path it is given.
    fn at_text(&self, path: &str) -> Option<PathBuf> {
        self.resolve(path, false)
    }

    fn resolve(&self, path: &str, physical: bool) -> Option<PathBuf> {
        let absolute = path.starts_with(['/', '~']) || path.starts_with("$HOME") || path.starts_with("${HOME}");
        resolve(if absolute { Path::new("/") } else { self.cwd.as_deref()? }, path, physical)
    }

    /// autopushd stacks the folder every `cd` leaves, which the stack here doesn't hold: after a `popd` or bare `pushd` the
    /// shell may be in any of the folders `left` (those before it) lists, whatever the stack says. A folder the guard
    /// couldn't tell, or more than 8 places, is lost.
    fn add_left(&mut self, left: Vec<Option<PathBuf>>) {
        let mut alts = std::mem::take(&mut self.alts);
        for f in left {
            match f {
                None => return self.lose(),
                Some(f) if self.cwd.as_ref() != Some(&f) && !alts.contains(&f) => alts.push(f),
                Some(_) => {}
            }
        }
        if alts.len() > 8 {
            return self.lose();
        }
        self.alts = alts;
    }

    /// Whether a `cd` into `folder` works: it is there, or a command of the line names it.
    fn there(&self, folder: &Path) -> bool {
        !missing(folder) || self.touched.iter().any(|m| folder.starts_with(m))
    }

    /// Whether a `cd` to `path`, as written, fails: each folder it goes through (up from one too) must be there.
    fn fails(&self, path: &Path) -> bool {
        let mut at = PathBuf::new();
        for c in path.components() {
            match c {
                Component::ParentDir => {
                    if !self.there(&at) {
                        return true;
                    }
                    at.pop();
                }
                c => at.push(c),
            }
        }
        !self.there(&at)
    }

    /// A command of another `&&` list than the one a `cd` failed in: the shell may have stayed where that `cd` left it.
    fn flush(&mut self, chain: usize) {
        self.chain = chain;
        if self.pending.is_empty() || chain == self.pending_chain {
            return;
        }
        self.popped = true;
        for a in std::mem::take(&mut self.pending) {
            if !self.alts.contains(&a) {
                self.alts.push(a);
            }
        }
        if self.alts.len() > 8 {
            self.lose();
        }
    }

    /// `path` from the current folder as written, with `..` not folded (the folder the OS looks up).
    fn written(&self, path: &str) -> Option<PathBuf> {
        let home = || dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
        match path {
            p if p == "~" || p.starts_with("~/") => Some(PathBuf::from(format!("{}{}", home()?, &p[1..]))),
            p if p.starts_with('/') => Some(PathBuf::from(p)),
            p => Some(self.cwd.as_deref()?.join(p)),
        }
    }

    /// A `cd` that may not run, or not in this shell (`Cmd::maybe`), left the shell in `places` too: they are seen by what
    /// follows, as after a `cd` that fails.
    fn may_stay(&mut self, places: Vec<PathBuf>) {
        self.pending_chain = if self.goes_on { usize::MAX } else { self.chain };
        for a in places {
            if !self.pending.contains(&a) {
                self.pending.push(a);
            }
        }
        if self.pending.len() > 8 {
            self.lose();
        }
    }

    fn cd(&mut self, to: Option<PathBuf>) {
        if !self.left.contains(&self.cwd) {
            self.left.push(self.cwd.clone());
        }
        self.old = std::mem::replace(&mut self.cwd, to);
    }

    /// Whether the folder the shell is in (`cwd`, links followed) may be swapped for a link by an earlier `ln`, `mv` or
    /// `cp` of the command, or sits behind a path the guard couldn't read. Such a path matters only when something besides
    /// where the command started names the folder: a `cd` or, for `named`, an option.
    fn swapped(&self, cwd: &Path, named: bool) -> bool {
        self.touched.iter().any(|t| cwd.starts_with(t) || self.cwd.as_ref().is_some_and(|c| c.starts_with(t))) || (self.unreadable && (self.moved || named))
    }

    /// Whether `path` goes through a folder an earlier `ln`, `mv` or `cp` names and then up (`t/..`): the link isn't
    /// there yet, so the guard folds the `..` back to where `t` sits, while the shell lands beside what it points at.
    fn passes(&self, path: &str) -> bool {
        self.passes_in(path, &self.touched)
    }

    fn passes_in(&self, path: &str, folders: &[PathBuf]) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        // Each `..`, with what is before it folded as text (`real/../t/..` goes up from `t`; a first `..` from the folder
        // the shell is in).
        (0..parts.len()).filter(|&u| parts[u] == "..").any(|u| {
            let before = if u == 0 { ".".to_string() } else { parts[..u].join("/") };
            self.at_text(&before).is_some_and(|p| folders.iter().any(|t| p.starts_with(t) || real(p.clone()).starts_with(t)))
        })
    }

    /// `env -C <folder>` for each of `chdir`: a relative one goes from the one before. One through a link that isn't there
    /// yet lands somewhere the guard can't tell.
    fn enter(&mut self, chdir: &[&str]) {
        for d in chdir {
            // A variable or a substitution: where env goes is known only to the shell.
            if folder_expands(d) {
                self.lose();
                continue;
            }
            self.unknown |= self.passes(d);
            self.cwd = self.at(d);
            // env goes there for real: into a link that isn't there yet, the shell ends where it points.
            self.unknown |= self.cwd.as_ref().is_some_and(|c| self.touched.iter().any(|t| c.starts_with(t)));
        }
    }

    /// The shell is in a folder the guard can't tell (a stack entry: `cd +1`): the topmost checkout around stands for it.
    fn lose(&mut self) {
        self.alts = self.cwd.as_deref().or(self.start.as_deref()).map(outermost_checkout).into_iter().collect();
        self.unknown = true;
        self.cd(None);
    }

    /// What a subshell (or a shell run by `sh -c`) made stays made when it ends.
    fn keep_made(&mut self, inner: &Dirs) {
        self.made |= inner.made;
        self.dotgit |= inner.dotgit;
        self.unreadable |= inner.unreadable;
        self.git_env |= inner.git_env;
        self.touched.extend(inner.touched.iter().cloned());
        self.removed.extend(inner.removed.iter().cloned());
    }

    /// `cd` to a glob or braces: the folder the literal part names, with the topmost checkout around as another place.
    fn hide(&mut self, dir: &str) {
        let hidden = self.cwd.as_deref().or(self.start.as_deref()).map(outermost_checkout);
        let literal = literal_folder(self, dir);
        self.alts = hidden.into_iter().collect();
        self.unknown = true;
        self.cd(literal);
    }

    /// `cd dir` (`-P`: `physical`): the shell goes where the text fold says, and the folder the physical one says is kept
    /// as another place it may be.
    fn move_to(&mut self, dir: &str, physical: bool) {
        // `cd -P` into or out of a link that isn't there yet puts the shell where it points, which the guard can't tell.
        let under = |p: Option<&PathBuf>| p.is_some_and(|c| self.touched.iter().any(|t| c.starts_with(t)));
        if physical && (under(self.cwd.as_ref()) || under(self.at_text(dir).as_ref()) || self.passes(dir)) {
            return self.hide(dir);
        }
        let to = if physical { self.at(dir) } else { self.at_text(dir) };
        let mut alts: Vec<PathBuf> = vec![];
        let from_here = self.at(dir);
        let from_alts: Vec<Option<PathBuf>> = self.alts.iter().map(|a| resolve(a, dir, true)).collect();
        for p in std::iter::once(from_here).chain(from_alts).flatten() {
            if Some(&p) != to.as_ref() && !alts.contains(&p) && alts.len() < 8 {
                alts.push(p);
            }
        }
        // A folder that isn't there (and no `ln`, `mv`, ... of the command names) may be a `cd` that fails: the shell stays
        // where it was, or where an earlier failed `cd` left it.
        // The path as written counts too: `nope/..` folds to a folder that is there, but the shell refuses it.
        let fails = to.as_ref().is_some_and(|t| !self.there(t)) || self.written(dir).is_some_and(|r| self.fails(&r));
        // So does one through a folder an earlier `rm` or `mv` of the command names and then up (`gone/..`).
        if self.passes_in(dir, &self.removed) || fails {
            // The rest of this `&&` list only runs when the `cd` worked.
            // A `cd` that doesn't stop the list has its places seen by the next command, whatever list that is in.
            self.pending_chain = if self.goes_on { usize::MAX } else { self.chain };
            for a in self.alts.iter().cloned().chain(self.cwd.clone()) {
                if !self.pending.contains(&a) {
                    self.pending.push(a);
                }
            }
            if self.pending.len() > 8 {
                return self.lose();
            }
        }
        self.alts = alts;
        self.cd(to);
    }
}

/// Whether a folder isn't there, so a `cd` to it fails.
#[cfg(not(test))]
fn missing(folder: &Path) -> bool {
    !folder.is_dir()
}

/// The same, with the tests' made-up `/nowhere` folders counted as there, but for the ones named `nope*`.
#[cfg(test)]
fn missing(folder: &Path) -> bool {
    !folder.is_dir() && !(folder.starts_with("/nowhere") && !folder.components().any(|c| c.as_os_str().to_string_lossy().starts_with("nope")))
}

/// Every change `command`, run in `cwd` with git's `ceiling` folders (GIT_CEILING_DIRECTORIES), makes to a checkout
/// (see the module docs), each folder once. A folder the guard can't tell (`cd "$dir"`) lets the commands after it run.
/// The ceiling counts only for a command that is plain git (`plain_git`); any other can move it, so it has none.
pub fn checkout_changes(command: &str, cwd: &Path, ceiling: &[PathBuf]) -> Vec<Change> {
    let mut found = vec![];
    let start = Some(cwd.to_path_buf());
    let ceiling: Vec<PathBuf> = ceiling.iter().filter(|c| c.is_absolute()).map(|c| real(c.clone())).collect();
    let storage = ceiling.first().cloned().unwrap_or_default();
    let ceiling = if plain_git(command, cwd, &ceiling) { ceiling } else { vec![] };
    changes(command, &mut Dirs { start: start.clone(), cwd: start, ceiling, storage, ..Default::default() }, 0, &mut found);
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
    // The folders as each open subshell found them, to go back to when it ends. What the subshell made stays made.
    let mut saved: Vec<(usize, Dirs)> = vec![];
    let mut pending_dotgit = false;
    for i in 0..cmds.len() {
        let scope = &cmds[i].scope;
        while saved.last().is_some_and(|(s, _)| !scope.contains(s)) {
            let inner = std::mem::replace(dirs, saved.pop().unwrap().1);
            dirs.keep_made(&inner);
        }
        for &s in &scope[saved.len()..] {
            saved.push((s, dirs.clone()));
        }
        dirs.flush(cmds[i].chain);
        dirs.goes_on = cmds[i].piped || cmds[i].or || cmds[i].words.first().is_some_and(|w| w == "!");
        let all = &cmds[i].words;
        let family = |n: &str| matches!(base(n), "cd" | "chdir" | "pushd" | "popd");
        // A function named like one of them, however it is written (`function cd`, `{ cd() { … } }`, `f cd()`), a
        // `functions[cd]=…`, an `autoload cd`, or `disable`, anywhere in the command.
        let defines = (cmds[i].defines && all.iter().any(|n| family(n)))
            || all.iter().position(|f| f == "function").is_some_and(|f| all[f + 1..].iter().take_while(|n| !KEYWORDS.contains(&n.as_str())).any(|n| family(n)))
            || all.iter().any(|w| w.strip_prefix("functions[").is_some_and(|r| r.split(']').next().is_some_and(family)))
            || (all.iter().any(|w| w == "autoload") && all.iter().any(|n| family(n)));
        dirs.shadowed |= defines || all.iter().any(|w| w == "disable");
        // `cd() { ... }` isn't a `cd`.
        if cmds[i].defines && defines {
            continue;
        }
        // A `.git` an earlier command names (`> t/.git`, `of=t/.git`, `ln -s x .git`) matters from the next on.
        dirs.dotgit |= std::mem::take(&mut pending_dotgit);
        pending_dotgit = names_dotgit(all, &cmds[i].targets);
        dirs.git_env |= mentions_git_env(all, &cmds[i].targets, &cmds[i].stdin);
        let words = match unwrap(all, &mut false) {
            Run::Words(words) => words,
            Run::Text(text, used) => {
                // Text `env -C <folder> -S '...'` runs is run from that folder.
                let mut inner = dirs.clone();
                inner.enter(&env_chdir(&all[..used]));
                changes(&text, &mut inner, depth + 1, out);
                dirs.keep_made(&inner);
                dirs.made = true;
                continue;
            }
        };
        let live = &cmds[i].live[all.len() - words.len()..];
        // `GIT_DIR=x git ...`, `env GIT_WORK_TREE=x git ...`
        let vars: Vec<&String> = all[..all.len() - words.len()].iter().filter(|w| assignment(w).is_some()).collect();
        let Some((name, args)) = words.split_first() else {
            // Only assignments (`PATH=./bin:$PATH;`): they change what the commands after it run or read.
            dirs.made |= all.iter().any(|w| assignment(w).is_some());
            continue;
        };
        // `env $NAME=x cmd`, `env $ASSIGNMENT cmd`: env sets whatever the expansion says, so what runs is unknown.
        if expands(name) && all[..all.len() - words.len()].iter().any(|w| base(w) == "env") {
            out.extend(unknown_in(dirs, None).into_iter().map(|folder| Change { folder, command: words.join(" ") }));
            dirs.made = true;
            continue;
        }
        let (args, live) = (args, &live[1..]);
        let name = base(name);
        let chdir = env_chdir(&all[..all.len() - words.len()]);
        // GNU coreutils under their Homebrew names (`gmv`, `gln`, `gcp`).
        let tool = name.strip_prefix('g').filter(|n| matches!(*n, "ln" | "mv" | "cp" | "rm" | "install")).unwrap_or(name);
        if matches!(tool, "ln" | "mv" | "cp" | "rm" | "rmdir" | "unlink" | "install" | "rsync" | "ditto") {
            // Braces or a glob: it may name any folder. (A removal can't make a link or a `.git`.)
            if !matches!(tool, "rm" | "rmdir" | "unlink") && args.iter().zip(live).any(|(a, &l)| hides(a, l)) {
                dirs.dotgit = true;
            }
            // What the command writes into, when it only puts files in a folder (`cp x .`), isn't swapped: skipped when
            // it can't make a link or a `.git` there. Anything else it names may be.
            let copies = matches!(tool, "mv" | "cp" | "install" | "rsync" | "ditto");
            let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
            let dest = args.iter().position(|a| a == "-t" || a == "--target-directory").and_then(|i| args.get(i + 1)).or_else(|| args.iter().find_map(|a| a.strip_prefix("--target-directory=").map(|_| a))).or(paths.last().copied().filter(|_| copies));
            let links = args.iter().any(|a| {
                let long = a.starts_with("--");
                (long && a.contains("link")) || (!long && a.starts_with('-') && a.chars().any(|c| if tool == "rsync" { matches!(c, 'l' | 'a') } else { matches!(c, 's' | 'l') }))
            });
            let git = paths.iter().any(|p| Path::new(p).file_name().is_some_and(|n| n == ".git"));
            let here = |p: &Path| dirs.cwd.as_ref().is_some_and(|c| c.starts_with(p));
            // `ln <target>` and `ln <target>... <folder>` make `<name of target>` in the folder: `.` when there is one path.
            if tool == "ln" {
                let into = dest.map(String::as_str).or(if paths.len() == 1 { Some(".") } else { paths.last().map(|p| p.as_str()).filter(|p| dirs.at_text(p).is_some_and(|d| d.is_dir())) });
                let sources = paths.iter().filter(|p| Some(p.as_str()) != into || paths.len() == 1);
                if let Some(folder) = into.and_then(|i| dirs.at_text(i)) {
                    for name in sources.filter_map(|p| Path::new(p.as_str()).file_name()) {
                        dirs.touched.extend([real(folder.join(name)), folder.join(name)]);
                    }
                }
            }
            for a in &paths {
                let gone = matches!(tool, "rm" | "rmdir" | "unlink") || (tool == "mv" && dest.is_none_or(|d| d != a.as_str()));
                if let Some(p) = dirs.at_text(a).filter(|_| gone) {
                    dirs.removed.extend([real(p.clone()), p]);
                }
            }
            for a in paths {
                match dirs.at_text(a) {
                    Some(p) if copies && !links && !git && dest.is_some_and(|d| d == a) && here(&p) => {}
                    Some(p) => dirs.touched.extend([real(p.clone()), p]),
                    None => dirs.unreadable = true,
                }
            }
        }
        let moved = dirs.moved;
        if matches!(name, "cd" | "chdir" | "pushd" | "popd") {
            dirs.moved = true;
        }
        // Another program's `cd` doesn't move the shell, for the commands after it too.
        let foreign = matches!(name, "cd" | "chdir" | "pushd" | "popd") && (dirs.shadowed || !shell_builtin(&all[..all.len() - words.len()], &cmds[i].quoted, &words[0]));
        dirs.goes_on |= foreign;
        let maybe = cmds[i].maybe || cmds[i].piped || foreign;
        let before: Vec<PathBuf> = dirs.alts.iter().cloned().chain(dirs.cwd.clone()).collect();
        let folders = match name {
            // zsh's other name for `cd`.
            "cd" | "chdir" => {
                let (physical, i) = dir_word(args);
                match i.map(|i| (args[i].as_str(), live[i])) {
                    None => {
                        dirs.alts.clear();
                        dirs.cd(dirs::home_dir());
                    }
                    Some(("-", _)) => {
                        // After a failed `cd` or a `popd` the folder `cd -` goes to may be any the command left.
                        let keep = !dirs.alts.is_empty() || dirs.popped;
                        let left = dirs.left.clone();
                        if !keep {
                            dirs.alts.clear();
                        }
                        dirs.cd(dirs.old.clone().or(dirs.start.clone()));
                        if keep {
                            dirs.add_left(left);
                        }
                    }
                    // zsh's `cd -2`, `cd +1`: a folder of the stack, which the guard doesn't follow. After `--` it's a folder.
                    Some((d, _)) if stack_entry(d) && !after_dashes(args, i) => dirs.lose(),
                    // A variable or a substitution: the guard can't tell where the shell lands.
                    Some((d, _)) if expands(d) => dirs.lose(),
                    // A glob or braces: the shell may go to any folder it lists. The literal part of the path names
                    // where they start, and the checkout around stands for the rest.
                    Some((dir, true)) if hides(dir, true) => dirs.hide(dir),
                    Some((dir, _)) => dirs.move_to(dir, physical),
                }
                if maybe {
                    dirs.may_stay(before);
                }
                continue;
            }
            "pushd" => {
                let (physical, i) = dir_word(args);
                let from = dirs.cwd.clone();
                let left = dirs.left.clone();
                dirs.popped = true;
                match i.map(|i| (args[i].as_str(), live[i])) {
                    // `pushd` swaps the top two folders; `pushd +1` rotates the stack.
                    None => {
                        dirs.alts.clear();
                        let to = dirs.stack.pop();
                        dirs.stack.push(from);
                        match to {
                            None if moved => dirs.cd(dirs.start.clone()),
                            Some(None) => dirs.lose(),
                            to => dirs.cd(to.flatten()),
                        }
                        dirs.add_left(left);
                    }
                    Some((d, _)) if ((stack_entry(d) || d == "-") && !after_dashes(args, i)) || expands(d) => {
                        dirs.stack.push(from);
                        dirs.lose();
                    }
                    Some((dir, true)) if hides(dir, true) => {
                        dirs.stack.push(from);
                        dirs.hide(dir);
                    }
                    Some((dir, _)) => {
                        dirs.stack.push(from);
                        dirs.move_to(dir, physical);
                    }
                }
                if maybe {
                    dirs.may_stay(before);
                }
                continue;
            }
            "popd" => {
                // `popd`, `popd -q`, `popd +0`: the top folder. Another entry, or `-n`, isn't one the guard follows.
                let (_, i) = dir_word(args);
                // With zsh's pushdminus `popd +0` removes the bottom entry, so it isn't the top one either.
                let plain = !args.iter().any(|a| a == "-n") && i.is_none();
                let left = dirs.left.clone();
                dirs.popped = true;
                match dirs.stack.pop() {
                    _ if !plain => dirs.lose(),
                    // The folder pushed was one the guard lost.
                    Some(None) => dirs.lose(),
                    Some(to) => {
                        dirs.alts.clear();
                        dirs.cd(to);
                    }
                    None => {
                        dirs.alts.clear();
                        dirs.cd(dirs.start.clone());
                    }
                }
                dirs.add_left(left);
                if maybe {
                    dirs.may_stay(before);
                }
                continue;
            }
            "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                if let Some(t) = shell_script(&cmds, i, args) {
                    let mut inner = dirs.clone();
                    inner.enter(&chdir);
                    changes(&t, &mut inner, depth + 1, out);
                    dirs.keep_made(&inner);
                }
                dirs.made = true;
                continue;
            }
            "eval" => {
                // The text numbers its lists from zero: what is pending before and after it is seen by what follows.
                dirs.flush(usize::MAX);
                changes(&args.join(" "), dirs, depth + 1, out);
                dirs.flush(usize::MAX);
                if maybe {
                    dirs.may_stay(before);
                }
                dirs.made = true;
                continue;
            }
            _ => {
                // `env -C <folder> cmd` runs cmd there.
                let (before, was_unknown, was_alts) = (dirs.cwd.clone(), dirs.unknown, dirs.alts.clone());
                dirs.enter(&chdir);
                let mut folders = command_folders(name, args, live, &vars, dirs);
                // Each other folder the shell may be in.
                for alt in dirs.alts.clone() {
                    let mut at = Dirs { cwd: Some(alt), alts: vec![], ..dirs.clone() };
                    folders.extend(command_folders(name, args, live, &vars, &mut at));
                }
                // Each folder it names, not only the one it runs in (`pnpm -C t install`), may be a link an earlier `ln`, `mv` or
                // `cp` of this command makes (git checks its own).
                let here = dirs.cwd.clone().map(real);
                let swapped = name != "git" && folders.iter().any(|f| f.as_os_str() != UNKNOWN && dirs.swapped(&real(f.clone()), Some(real(f.clone())) != here));
                if swapped {
                    folders.extend(unknown_in(dirs, None));
                }
                if dirs.unknown && !folders.is_empty() {
                    folders.push(PathBuf::from(UNKNOWN));
                }
                dirs.cwd = before;
                dirs.unknown = was_unknown;
                dirs.alts = was_alts;
                folders
            }
        };
        dirs.made |= !makes_nothing(all, words, live);
        out.extend(folders.into_iter().map(|folder| Change { folder, command: words.join(" ") }));
    }
    // The last command's `.git` counts for whatever runs after this text: `sh -c 'ln -s x .git'; git reset`.
    dirs.dotgit |= pending_dotgit;
}

/// Whether a command (`git -C dir fetch`, `git commit -m x`) can't make a folder, a link or a `.git`: git's own history
/// and remote commands, which don't write the work tree (`pull`, `checkout` do). Strict: the word `git` itself, with
/// nothing before it (no assignment, `env` or wrapper: they can name a program git runs), no word the shell expands,
/// and only options from a short list (git takes long ones by any unique prefix, so any other may run a program).
fn makes_nothing(all: &[String], words: &[String], live: &[bool]) -> bool {
    const OPTIONS: &[&str] = &["--all", "--amend", "--no-edit", "--tags", "--prune", "--dry-run", "--quiet", "--verbose", "--short", "--stat", "--oneline", "--porcelain", "--set-upstream", "--graph", "--name-only", "--force-with-lease", "-sb"];
    let [git, args @ ..] = words else { return false };
    let args = match args {
        [c, _, rest @ ..] if c == "-C" => rest,
        _ => args,
    };
    git == "git"
        && all.len() == words.len()
        && !live.iter().any(|&l| l)
        && !words.iter().any(|w| w.contains(['$', '`']))
        && args.first().is_some_and(|sub| matches!(sub.as_str(), "fetch" | "push" | "commit" | "add" | "status" | "log" | "diff" | "show"))
        && args[1..].iter().all(|a| match a.strip_prefix('-') {
            None | Some("") | Some("-") => true,
            // `-p` is a patch for these; `add -p` and `commit -p` are interactive, `fetch -p` prunes.
            Some("p") => matches!(args[0].as_str(), "log" | "diff" | "show"),
            Some(o) if o.starts_with('-') => OPTIONS.contains(&a.as_str()),
            Some(o) => OPTIONS.contains(&a.as_str()) || o.chars().all(|c| "aAmqvunsf".contains(c)),
        })
}

/// Where cd's and pushd's folder is among `args`, past their options (`-L`, `-P`, `-q`, bundles like `-LP`, up to and
/// including `--`), and whether `-P` is among them (zsh takes it over `-L` in any order). A lone `-` and a stack entry (`-2`, `+1`) are the word.
fn dir_word(args: &[String]) -> (bool, Option<usize>) {
    let mut physical = false;
    for (i, a) in args.iter().enumerate() {
        if a == "--" {
            return (physical, Some(i + 1).filter(|&n| n < args.len()));
        }
        if a.len() < 2 || !a.starts_with('-') || stack_entry(a) {
            return (physical, Some(i));
        }
        physical |= a.contains('P');
    }
    (physical, None)
}

/// Whether the word at `i` follows a `--`: a folder, whatever it looks like.
fn after_dashes(args: &[String], i: Option<usize>) -> bool {
    i.is_some_and(|i| i > 0 && args[i - 1] == "--")
}

/// `-2` or `+1`: an entry of the directory stack.
fn stack_entry(word: &str) -> bool {
    word.len() > 1 && word.starts_with(['-', '+']) && word[1..].bytes().all(|b| b.is_ascii_digit())
}

/// Each `-C <folder>`, `-C<folder>`, `--chdir=<folder>` or `--chdir <folder>` of the `env`s before a command, in order
/// (a relative one goes from the one before), as one option or bundled (`-iC <folder>`).
fn env_chdir(prefix: &[String]) -> Vec<&str> {
    let mut found = vec![];
    let mut it = prefix.iter().skip_while(|w| base(w) != "env").skip(1);
    while let Some(o) = it.next() {
        if let Some(d) = o.strip_prefix("--chdir=") {
            found.push(d);
        } else if o == "--chdir" {
            found.extend(it.next().map(String::as_str));
        } else if o.starts_with('-') && !o.starts_with("--") {
            // A bundle: options up to the first that takes a value, which is the rest of the word or the next word.
            let Some(at) = o.find(['C', 'u', 'P', 'S']) else { continue };
            let rest = &o[at + 1..];
            let value = if rest.is_empty() { it.next().map(String::as_str) } else { Some(rest) };
            if o.as_bytes()[at] == b'C' {
                found.extend(value);
            }
        }
    }
    found
}

/// Whether a command writes or names a path that ends in `.git` (also as `of=x/.git` or `--output=x/.git`), other than
/// as a plain `GIT_DIR=` or `GIT_WORK_TREE=` word, which `mentions_git_env` already covers.
fn names_dotgit(words: &[String], targets: &[String]) -> bool {
    let ends = |p: &str| Path::new(p).file_name().is_some_and(|n| n == ".git");
    words.iter().filter(|w| !w.starts_with("GIT_DIR=") && !w.starts_with("GIT_WORK_TREE=")).chain(targets).any(|w| ends(w) || w.split_once('=').is_some_and(|(_, v)| ends(v)))
}

/// Whether a command sets, exports, unsets or reads GIT_DIR or GIT_WORK_TREE: a word that starts with the name or
/// has `NAME=` in it, or whose name (before any `=`) is a glob or an expansion of `GIT_*` (`GIT_D*`, `${!GIT_*}`), even
/// through quotes. Text a shell runs (`eval`, `sh -c`, a heredoc) counts if it names either at all. A mention in a
/// message (`'fix GIT_DIR handling'`) or a use of the value (`echo $GIT_DIR`) doesn't.
fn mentions_git_env(words: &[String], targets: &[String], stdin: &str) -> bool {
    let named = |t: &str| t.contains("GIT_DIR") || t.contains("GIT_WORK_TREE");
    let hit = |w: &str| {
        let name = w.split('=').next().unwrap_or(w);
        w.starts_with("GIT_DIR") || w.starts_with("GIT_WORK_TREE") || w.contains("GIT_DIR=") || w.contains("GIT_WORK_TREE=") || (name.starts_with("GIT_") && name.contains(['$', '*', '?', '[', '{'])) || name.contains("${!GIT")
    };
    let shell = words.iter().any(|w| matches!(base(w), "eval" | "sh" | "bash" | "zsh" | "dash" | "ksh"));
    words.iter().chain(targets).any(|w| hit(w) || (shell && named(w))) || (shell && named(stdin))
}

/// The folders one command (not a `cd`, a shell or an `eval`) changes, run in `dirs`.
fn command_folders(name: &str, args: &[String], live: &[bool], vars: &[&String], dirs: &mut Dirs) -> Vec<PathBuf> {
    match name {
        "git" => git_changes(dirs, vars, args, live),
        "cargo" => match flags(args, &["--manifest-path"], CARGO_CHANGES) {
            (manifests, Some(s)) if CARGO_CHANGES.contains(&s) => match manifests[..] {
                [] => dirs.cwd.clone().into_iter().collect(),
                _ => manifests
                    .iter()
                    .flat_map(|&m| match m {
                        m if folder_expands(m) => unknown_in(dirs, None),
                        m => dirs.at(m).and_then(|f| f.parent().map(Path::to_path_buf)).into_iter().chain(dirs.passes(m).then(|| PathBuf::from(UNKNOWN))).collect(),
                    })
                    .collect(),
            },
            _ => vec![],
        },
        pm @ ("npm" | "pnpm" | "yarn" | "bun") => {
            let dir_flags: &[&str] = match pm {
                "npm" => &["--prefix", "-C"],
                "pnpm" => &["--dir", "-C"],
                _ => &["--cwd"],
            };
            // Node folds `..` as text, the kernel after the links: both are asked about.
            let folders = |dir: Option<&str>| match dir {
                None => dirs.cwd.clone().into_iter().collect::<Vec<_>>(),
                Some(d) if folder_expands(d) => unknown_in(dirs, None),
                Some(d) => [dirs.at_text(d), dirs.at(d)].into_iter().flatten().chain(dirs.passes(d).then(|| PathBuf::from(UNKNOWN))).collect(),
            };
            let each = |dirs: Vec<&str>| if dirs.is_empty() { folders(None) } else { dirs.into_iter().flat_map(|d| folders(Some(d))).collect() };
            match flags(args, dir_flags, PM_CHANGES) {
                (dir, Some(s)) if PM_CHANGES.contains(&s) => each(dir),
                (dir, None) if pm == "yarn" => each(dir), // a bare `yarn` installs
                _ => vec![],
            }
        }
        "xcodebuild" => {
            let reads = ["-list", "-showBuildSettings", "-version", "-showsdks", "-showdestinations", "-help"];
            if args.iter().any(|a| reads.contains(&a.as_str())) {
                return vec![];
            }
            // The folder of the project or workspace it names builds there too.
            let named = args.windows(2).filter(|w| matches!(w[0].as_str(), "-project" | "-workspace")).map(|w| w[1].as_str());
            let named: Vec<PathBuf> = named.flat_map(|n| if folder_expands(n) { unknown_in(dirs, None) } else { dirs.at(n).and_then(|p| p.parent().map(Path::to_path_buf)).into_iter().chain(dirs.passes(n).then(|| PathBuf::from(UNKNOWN))).collect() }).collect();
            dirs.cwd.clone().into_iter().chain(named).collect()
        }
        _ => vec![],
    }
}

/// Every value of the folder `options` in `args` (`--dir x`, `--dir=x`, `-Cx`, `-sC x`, `--dir` by a prefix of three or
/// more letters, as npm takes it), since a tool may take the last of a repeated one. And the subcommand: the first word
/// that isn't the word after an option, or is in `subs`; else the first word. (`--filter x` takes a value the guard can't
/// tell from a subcommand.)
fn flags<'a>(args: &'a [String], options: &[&str], subs: &[&str]) -> (Vec<&'a str>, Option<&'a str>) {
    let (mut values, mut words, mut after_option) = (vec![], vec![], false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let was = std::mem::replace(&mut after_option, false);
        let name = a.split_once('=').map_or(a.as_str(), |(n, _)| n);
        let long = |o: &&str| o.starts_with("--") && name.len() > 2 && o.starts_with(name);
        let short = |o: &&str| !o.starts_with("--") && a.starts_with('-') && !a.starts_with("--") && a[1..].contains(&o[1..]);
        if options.iter().any(|o| long(o)) {
            match a.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
                Some(v) => values.push(v),
                None => values.extend(it.next().map(String::as_str)),
            }
        } else if let Some(o) = options.iter().find(|o| short(o)) {
            let rest = a[1..].split_once(&o[1..]).map_or("", |(_, r)| r);
            let rest = rest.strip_prefix('=').unwrap_or(rest);
            match rest {
                "" => values.extend(it.next().map(String::as_str)),
                v => values.push(v),
            }
        } else if !a.starts_with(['-', '+']) {
            words.push((a.as_str(), was));
        } else {
            after_option = true;
        }
    }
    let sub = words.iter().find(|w| !w.1 || subs.contains(&w.0)).or(words.first()).map(|w| w.0);
    (values, sub)
}

/// Git's own subcommands. Anything else may be an alias, which can run any shell command (`!…`).
const GIT_BUILTINS: &[&str] = &[
    "add", "am", "apply", "blame", "branch", "checkout", "cherry-pick", "clean", "clone", "commit", "config", "describe",
    "diff", "fetch", "grep", "init", "log", "ls-files", "merge", "mv", "pull", "push", "rebase", "remote", "reset", "restore",
    "revert", "rev-parse", "rm", "show", "stash", "status", "submodule", "switch", "tag", "worktree",
];

/// Git's global options that take no value and don't change where git looks or what it runs.
const GIT_PLAIN_OPTIONS: &[&str] = &["-p", "--paginate", "-P", "--no-pager", "--no-replace-objects", "--no-optional-locks", "--literal-pathspecs", "--no-advice"];

/// Whether `command` is plain git, so it runs with the run's GIT_CEILING_DIRECTORIES: only `git <subcommand> <args>`,
/// one or more joined by `&&` or `;`, each run directly (`plain_shell`), with only the global options git can't be
/// moved by and a built-in subcommand (`plain_git_args`). Anything else may move the ceiling, so the guard walks up.
fn plain_git(command: &str, cwd: &Path, ceiling: &[PathBuf]) -> bool {
    let cmds = if plain_shell(command) { parse(command) } else { vec![] };
    !cmds.is_empty()
        && cmds.iter().all(|c| match c.words.split_first() {
            Some((git, args)) if git == "git" => plain_git_args(args, cwd, ceiling),
            _ => false,
        })
}

/// Whether `command` has no shell feature but quotes, `;` and `&&`: no wrapper syntax, pipe, redirection, subshell,
/// expansion, glob, history, comment, background job or second line.
fn plain_shell(command: &str) -> bool {
    !command.contains(['$', '`', '\\', '|', '<', '>', '(', ')', '{', '}', '*', '?', '[', ']', '~', '#', '!', '\n']) && !command.replace("&&", "").contains('&')
}

/// Whether git's `args` are global options from `GIT_PLAIN_OPTIONS` or a `-C` inside a `ceiling` folder, then a
/// built-in subcommand. Any other option before the subcommand may take a value (`--namespace x`) or move git.
fn plain_git_args(args: &[String], cwd: &Path, ceiling: &[PathBuf]) -> bool {
    let mut it = args.iter().map(String::as_str);
    loop {
        match it.next() {
            Some("-C") => {
                if !it.next().is_some_and(|d| inside_ceiling(d, cwd, ceiling)) {
                    return false;
                }
            }
            Some(o) if GIT_PLAIN_OPTIONS.contains(&o) => {}
            Some(sub) => return GIT_BUILTINS.contains(&sub),
            None => return false,
        }
    }
}

/// Whether `dir`, from `cwd` with symlinks followed, is in one of the `ceiling` folders.
fn inside_ceiling(dir: &str, cwd: &Path, ceiling: &[PathBuf]) -> bool {
    resolve(cwd, dir, true).map(real).is_some_and(|d| ceiling.iter().any(|c| d.starts_with(c)))
}

/// What the guard reports for a folder it can't name: Loft asks the user about the projects folder as a whole.
pub const UNKNOWN: &str = "/loft-unknown-folder";

/// The folders a git command changes, past its global options: its work tree (`--work-tree`, `GIT_WORK_TREE`) and
/// the checkout its repository is in (`--git-dir`, `GIT_DIR`), each by default the repository git finds from the
/// folder (`-C`): the nearest one up, which for an agent's workspace that isn't a clone is the user's checkout around
/// it. Every subcommand changes them but the ones that only read (`git_reads`).
///
/// What the shell expands (braces, globs) or an earlier command of this run may still make (a folder, a link, a
/// `.git` file) hides the folder or repository: then the answer is `unknown_in`.
fn git_changes(outer: &mut Dirs, vars: &[&String], args: &[String], live: &[bool]) -> Vec<PathBuf> {
    let mut unknown = false;
    let mut folders = git_folders(outer, vars, args, live, &mut unknown);
    if unknown {
        folders.push(PathBuf::from(UNKNOWN));
    }
    folders
}

/// `git_changes` but for the `unknown` answer a change makes beside the folders.
fn git_folders(outer: &mut Dirs, vars: &[&String], args: &[String], live: &[bool], unknown: &mut bool) -> Vec<PathBuf> {
    let mut dirs = outer.clone();
    let (mut tree, mut git_dir) = (None, None);
    let mut relative = false;
    let absolute = |d: &str| d.starts_with(['/', '~']) || d.starts_with("$HOME") || d.starts_with("${HOME}");
    for v in vars {
        if let Some(d) = v.strip_prefix("GIT_WORK_TREE=") {
            relative |= !absolute(d);
            tree = Some(dirs.at(d));
        } else if let Some(d) = v.strip_prefix("GIT_DIR=") {
            relative |= !absolute(d);
            git_dir = Some(dirs.at(d));
        }
    }
    // Braces the shell expands before git sees them (`git -C {../..,reset} --hard`).
    let Some(args) = expand_args(args, live) else { return unknown_in(&dirs, None) };
    let hid = |k: usize| args.get(k).is_some_and(|(w, live)| hides(w, *live));
    let mut i = 0;
    let mut dir_options = false;
    let sub = loop {
        let Some((a, _)) = args.get(i) else { return vec![] };
        i += 1;
        let a = a.as_str();
        let value = |o: &str| a.strip_prefix(o).and_then(|r| r.strip_prefix('='));
        let takes_value = matches!(a, "-C" | "--work-tree" | "--git-dir" | "-c" | "--namespace" | "--config-env" | "--super-prefix");
        // A variable or a substitution in a folder option: the shell tells git where, the guard can't.
        let dir_option = matches!(a, "-C" | "--work-tree" | "--git-dir");
        if (dir_option && args.get(i).is_some_and(|(d, _)| folder_expands(d))) || ["--work-tree", "--git-dir"].iter().any(|o| value(o).is_some_and(folder_expands)) {
            return unknown_in(&dirs, None);
        }
        if hid(i - 1) || (takes_value && hid(i)) {
            let path = if takes_value && hid(i) { args.get(i).map(|w| w.0.as_str()) } else { value("--git-dir").or(value("--work-tree")) };
            return unknown_in(&dirs, path);
        }
        let rel_next = args.get(i).is_some_and(|(d, _)| !absolute(d));
        // `-C t/..` after a link that isn't there yet goes up from where it will point.
        let up = (matches!(a, "-C" | "--work-tree" | "--git-dir") && args.get(i).is_some_and(|(d, _)| dirs.passes(d))) || ["--work-tree", "--git-dir"].iter().any(|o| value(o).is_some_and(|d| dirs.passes(d)));
        let mut next = || {
            i += 1;
            args.get(i - 1).and_then(|(d, _)| dirs.at(d))
        };
        match a {
            _ if up => return unknown_in(&dirs, None),
            "-C" => {
                dir_options = true;
                let to = next();
                dirs.cwd = to;
            }
            "--work-tree" => {
                relative |= rel_next;
                tree = Some(next());
            }
            "--git-dir" => {
                relative |= rel_next;
                git_dir = Some(next());
            }
            _ if value("--work-tree").is_some() => {
                relative |= value("--work-tree").is_some_and(|d| !absolute(d));
                tree = Some(value("--work-tree").and_then(|d| dirs.at(d)));
            }
            _ if value("--git-dir").is_some() => {
                relative |= value("--git-dir").is_some_and(|d| !absolute(d));
                git_dir = Some(value("--git-dir").and_then(|d| dirs.at(d)));
            }
            // `-c core.worktree=x` names the work tree as `--work-tree` does.
            "-c" => {
                // A key from an expansion (`-c $K=x`, `-c $E`) may be `core.worktree`.
                if args.get(i).is_some_and(|(w, _)| expands(w.split_once('=').map_or(w.as_str(), |(k, _)| k))) {
                    return unknown_in(&dirs, None);
                }
                if let Some((_, v)) = args.get(i).and_then(|(w, _)| w.split_once('=')).filter(|(k, _)| k.eq_ignore_ascii_case("core.worktree")) {
                    // A variable or a substitution, as in any folder option.
                    if folder_expands(v) {
                        return unknown_in(&dirs, None);
                    }
                    tree = Some(dirs.at(v));
                }
                i += 1;
            }
            "--config-env" if args.get(i).is_some_and(|(w, _)| w.to_ascii_lowercase().starts_with("core.worktree")) => {
                return unknown_in(&dirs, None);
            }
            _ if a.to_ascii_lowercase().starts_with("--config-env=core.worktree") => return unknown_in(&dirs, None),
            "--namespace" | "--config-env" | "--super-prefix" => i += 1,
            o if o.starts_with('-') => {}
            sub => break sub,
        }
    };
    let rest: Vec<&str> = args[i..].iter().map(|(w, _)| w.as_str()).collect();
    if git_reads(sub, &rest) || (sub == "config" && rest.iter().any(|a| matches!(*a, "--global" | "--system"))) {
        return vec![];
    }
    // GIT_DIR or GIT_WORK_TREE anywhere in the command: git may use a repository the guard can't follow. A relative
    // one is read from the folder `-C` moves to, not from the one before it.
    *unknown |= dirs.git_env || (relative && dir_options);
    // `clone` and `init` make a new repository: in the folder they name, or the folder they run in, not the one around it.
    if let Some(valued) = match sub {
        "clone" => Some(&["--branch", "--origin", "--config", "--depth", "--reference", "--reference-if-able", "--template", "--separate-git-dir", "--upload-pack", "--jobs", "--filter", "--shallow-since", "--shallow-exclude", "--server-option", "--bundle-uri", "-b", "-o", "-c", "-u", "-j"][..]),
        "init" => Some(&["--initial-branch", "--template", "--separate-git-dir", "--object-format", "--ref-format", "-b"][..]),
        _ => None,
    } {
        let (mut words, mut git_dir) = (vec![], None);
        let mut it = rest.iter();
        while let Some(a) = it.next() {
            // Git takes a long option by any unique prefix (`--orig`, `--dep`, `--separate-g`).
            let (name, inline) = a.split_once('=').map_or((*a, None), |(n, v)| (n, Some(v)));
            let long = if name.starts_with("--") { valued.iter().copied().find(|v| *v == name).or_else(|| valued.iter().copied().find(|v| v.starts_with(name))) } else { valued.iter().copied().find(|v| *v == name) };
            match (long, inline) {
                (Some("--separate-git-dir"), Some(d)) => git_dir = Some(d),
                (Some("--separate-git-dir"), None) => git_dir = it.next().copied(),
                (Some(_), None) => drop(it.next()),
                (Some(_), Some(_)) => {}
                (None, _) if !a.starts_with('-') => words.push(*a),
                (None, _) => {}
            }
        }
        // `clone <repository> [<folder>]`, `init [<folder>]`, and the repository's own folder when it's apart.
        let target = words.get(if sub == "clone" { 1 } else { 0 });
        let target = target.map_or(dirs.cwd.clone(), |t| dirs.at(t));
        return [target, git_dir.and_then(|d| dirs.at(d))].into_iter().flatten().collect();
    }
    if let Some(cwd) = dirs.cwd.clone().map(real) {
        // A part that isn't there when the hook looks can be made before git runs: a link to any checkout (`ln -s`,
        // a clone, `git checkout`, `tar`), a `.git` file. Nothing about it is known, not even a clone's own folder
        // (it may bring a link, or an agent's `--work-tree` may name a checkout).
        let start = outer.start.clone().map(real);
        let near = start.as_ref().is_none_or(|s| cwd != *s) && (in_agent_storage(&cwd, &dirs.storage) || start.is_some_and(|s| cwd.starts_with(s)));
        // A work tree or repository named through a link that isn't there yet.
        let named = tree.iter().chain(git_dir.iter()).flatten().any(|p| dirs.swapped(&real(p.clone()), true));
        let swapped = named || dirs.swapped(&cwd, dir_options || tree.is_some() || git_dir.is_some());
        if dirs.dotgit || swapped || (dirs.made && near && !cwd.exists()) {
            return unknown_in(&dirs, None);
        }
    }
    let found = dirs.cwd.map(|d| repo_root(d, &dirs.ceiling));
    // A `.git` file or link points at the repository, which may be another checkout's.
    let linked = found.as_deref().map(linked_repos).unwrap_or_default();
    // The checkout a `.git` folder belongs to.
    let repos: Vec<PathBuf> = match git_dir {
        Some(d) => d.into_iter().collect(),
        None if linked.is_empty() => found.clone().into_iter().collect(),
        None => linked,
    };
    let repos = repos.into_iter().map(|d| if d.ends_with(".git") { d.parent().map(Path::to_path_buf).unwrap_or(d) } else { d });
    tree.unwrap_or(found).into_iter().chain(repos).collect()
}

/// What a command whose folder, work tree or repository the guard can't name may change: the folder the literal part of
/// `path` names (`<projects>/lof?` is `<projects>`), the topmost checkout around where it runs, and, for Loft, the
/// projects folder (`UNKNOWN`).
fn unknown_in(dirs: &Dirs, path: Option<&str>) -> Vec<PathBuf> {
    let literal = path.and_then(|p| literal_folder(dirs, p));
    let hidden = dirs.cwd.as_deref().or(dirs.start.as_deref()).map(outermost_checkout);
    literal.into_iter().chain(hidden).chain([PathBuf::from(UNKNOWN)]).collect()
}

/// The folder a path with a glob or braces starts from: the parts before the first one that has one.
fn literal_folder(dirs: &Dirs, path: &str) -> Option<PathBuf> {
    let mut keep = vec![];
    for part in path.split('/') {
        if part.contains(['*', '?', '[', '{']) {
            break;
        }
        keep.push(part);
    }
    let prefix = keep.join("/");
    dirs.at(if prefix.is_empty() && path.starts_with('/') { "/" } else if prefix.is_empty() { "." } else { &prefix })
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
            return if rest.as_os_str().is_empty() { r } else { r.join(rest) };
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
/// Symlinks are followed first, as git does. From below a `ceiling` folder, git doesn't look in it or above.
fn repo_root(folder: PathBuf, ceiling: &[PathBuf]) -> PathBuf {
    let folder = real(folder);
    let below = |d: &Path| ceiling.iter().any(|c| d != c && d.starts_with(c));
    let stop = ceiling.iter().any(|c| folder != *c && folder.starts_with(c));
    folder.ancestors().take_while(|d| !stop || below(d)).find(|d| d.join(".git").exists()).map(Path::to_path_buf).unwrap_or(folder)
}

/// Where a `.git` in `root` that isn't a plain folder (a file `gitdir: x`, or a link to another `.git`) sends git: the
/// checkout that holds that repository, and the one that holds the common repository its `commondir` file names.
fn linked_repos(root: &Path) -> Vec<PathBuf> {
    use std::io::Read;
    // The first line of a small file, with `prefix` taken off.
    let line = |file: &Path, prefix: &str| -> Option<String> {
        let mut text = String::new();
        std::fs::File::open(file).ok()?.take(4096).read_to_string(&mut text).ok()?;
        text.lines().next()?.strip_prefix(prefix).map(|l| l.trim().to_string())
    };
    let dot = root.join(".git");
    let Ok(meta) = std::fs::symlink_metadata(&dot) else { return vec![] };
    let target = real(dot.clone());
    let gitdir = if meta.is_file() || (meta.is_symlink() && target.is_file()) {
        let from = if meta.is_file() { root } else { target.parent().unwrap_or(root) };
        match line(&target, "gitdir:") {
            Some(d) => real(from.join(d)),
            None => return vec![],
        }
    } else if meta.is_symlink() {
        target
    } else {
        return vec![];
    };
    // The checkout holding a `.git` folder: the one it is in, else the repository's folder itself.
    let checkout = |d: &Path| d.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git")).and_then(Path::parent).map_or(d.to_path_buf(), Path::to_path_buf);
    let common = line(&gitdir.join("commondir"), "").map(|c| real(gitdir.join(c)));
    [Some(checkout(&gitdir)), common.map(|c| checkout(&c))].into_iter().flatten().collect()
}

/// The topmost checkout around `folder`: where a command whose target the guard can't list may act.
fn outermost_checkout(folder: &Path) -> PathBuf {
    let folder = real(folder.to_path_buf());
    folder.ancestors().filter(|d| d.join(".git").exists()).last().map(Path::to_path_buf).unwrap_or(folder)
}

/// Whether the shell turns `word` into names the guard can't list: it has a glob or braces (`{a,b}`, `{1..3}`) outside
/// quotes (`live`, from the lexer). The ones a git word has are expanded first (`expand_args`).
/// A folder word with a variable, a command substitution or a backtick: where it goes is known only to the shell.
fn expands(word: &str) -> bool {
    word.contains(['$', '`'])
}

/// `expands` for a folder option's value: `$HOME` and `${HOME}` at the start are the home folder, which the guard knows.
/// `$PWD` and `$(pwd)` are not: a command can set `PWD`, redefine `pwd`, or run in another folder than the shell expands it in.
fn folder_expands(word: &str) -> bool {
    let rest = ["$HOME", "${HOME}"].iter().find_map(|h| word.strip_prefix(h).filter(|r| r.is_empty() || r.starts_with('/'))).unwrap_or(word);
    expands(rest)
}

fn hides(word: &str, live: bool) -> bool {
    let braces = word.find('{').is_some_and(|i| !word[..i].ends_with('$') && word[i..].contains('}') && (word[i..].contains(',') || word[i..].contains("..")));
    live && (word.contains(['*', '?', '[']) || braces)
}

/// `args` with the braces of each unquoted word expanded as the shell does, each with whether it still may hold a glob.
/// None when it can't tell: braces in a word that is partly quoted, or an expansion too long to list.
fn expand_args(args: &[String], live: &[bool]) -> Option<Vec<(String, bool)>> {
    let mut out = vec![];
    for (a, &live) in args.iter().zip(live) {
        if !live || !a.contains('{') {
            out.push((a.clone(), live));
        } else if a.contains(char::is_whitespace) {
            return None;
        } else {
            let mut budget = 4096;
            out.extend(expand_braces(a, &mut budget)?.into_iter().filter(|w| !w.is_empty()).map(|w| (w, true)));
        }
    }
    Some(out)
}

/// The words the shell makes of `word` by brace expansion: `{a,b}` is two. `${x}`, `{x}` and `{}` are literal, and a
/// sequence (`{1..3}`) stays as it is (`hides`). None past `budget` words.
fn expand_braces(word: &str, budget: &mut usize) -> Option<Vec<String>> {
    let b = word.as_bytes();
    let Some(open) = (0..b.len()).find(|&i| b[i] == b'{' && (i == 0 || b[i - 1] != b'$')) else { return Some(vec![word.to_string()]) };
    let (mut depth, mut commas, mut close) = (0, vec![], None);
    for (i, c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b',' if depth == 1 => commas.push(i),
            _ => {}
        }
    }
    let Some(close) = close.filter(|_| !commas.is_empty()) else { return Some(vec![word.to_string()]) };
    let (pre, post) = (&word[..open], &word[close + 1..]);
    let mut out = vec![];
    let mut start = open + 1;
    for end in commas.iter().chain([&close]) {
        out.extend(expand_braces(&format!("{pre}{}{post}", &word[start..*end]), budget)?);
        start = end + 1;
    }
    *budget = budget.checked_sub(out.len())?;
    Some(out)
}

/// Whether a git subcommand, with its arguments, only reads.
fn git_reads(sub: &str, rest: &[&str]) -> bool {
    let has = |opts: &[&str]| rest.iter().any(|a| opts.contains(a));
    let words = || rest.iter().filter(|a| !a.starts_with('-'));
    let first = rest.first().copied();
    match sub {
        "status" | "log" | "diff" | "show" | "fetch" | "rev-parse" | "rev-list" | "ls-files" | "ls-tree" | "ls-remote" | "blame"
        | "grep" | "describe" | "shortlog" | "cat-file" | "show-ref" | "for-each-ref" | "merge-base" | "name-rev"
        | "range-diff" | "cherry" | "check-ignore" | "count-objects" | "var" | "help" | "version" | "patch-id" | "diff-tree"
        | "diff-index" | "diff-files" | "check-ref-format" | "check-attr" | "show-branch" | "whatchanged" | "verify-commit"
        | "verify-tag" => true,
        // Plumbing that writes only with an option: `-w` stores the object, `--lost-found` keeps the dangling ones. Git takes bundled short options (`-wt blob`) and unique long prefixes (`--lost-f`, `--write-t`), so any `w` in a short option and any `--l*` or `--w*` asks. `merge-tree` writes by default, so it always asks.
        "hash-object" | "fsck" => !rest.iter().any(|a| a.starts_with("--l") || a.starts_with("--w") || (a.starts_with('-') && !a.starts_with("--") && a.contains('w'))),
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

/// `path` from `cwd`, `~` and `$HOME` expanded and `..` folded (after the links on the way are followed when `physical`). None when it holds another variable or a substitution.
fn resolve(cwd: &Path, path: &str, physical: bool) -> Option<PathBuf> {
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
            // Git and the kernel follow a link before they go up: `lnk/..` is the folder holding what `lnk` points at.
            // The shell's `cd` folds the text (`physical` is false).
            Component::ParentDir => {
                if physical {
                    out = real(out);
                }
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    Some(out)
}

/// Whether the real path `folder` is in an agent's workspace: under `storage`, the real path of the storage folder
/// Loft uses (storage.rs), not any folder named `agent-local-storage`.
pub fn in_agent_storage(folder: &Path, storage: &Path) -> bool {
    crate::storage::holds_in(folder, storage)
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
            Run::Text(text, _) => {
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

const KEYWORDS: &[&str] = &["!", "{", "}", "always", "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "esac"];

/// Whether the next word after `words` is in command position, where a keyword counts: past keywords and a
/// `function NAME...`. A quoted word (`quoted` says which) is no keyword.
fn command_position(words: &[String], quoted: &[bool]) -> bool {
    let keyword = |i: usize| !quoted.get(i).copied().unwrap_or(false) && (KEYWORDS.contains(&words[i].as_str()) || words[i] == "time");
    let from = if words.first().is_some_and(|f| f == "function") && !quoted[0] {
        let names = (1..words.len()).take_while(|&i| !keyword(i)).count();
        if names == 0 {
            return false;
        }
        1 + names
    } else {
        0
    };
    (from..words.len()).all(keyword)
}

/// Whether a `cd`, `pushd` or `popd` is the shell's own: what `unwrap` peeled off before it (`prefix`, with `quoted` for each
/// word) is, in this order, unquoted keywords, `time` and `!` (and a `function`'s names), then unquoted assignments to
/// valid names, then at most one of `builtin`, `noglob`, `nocorrect` and `-`; and its name (`name`, as written) isn't a
/// path. Another program's `cd` (`env cd x`, `/usr/bin/cd x`, `"time" cd x`) succeeds and the shell stays where it was.
fn shell_builtin(prefix: &[String], quoted: &[bool], name: &str) -> bool {
    if name.contains('/') {
        return false;
    }
    // 0: keywords, 1: assignments, 2: a modifier.
    let (mut phase, mut names, mut timed) = (0, false, false);
    for (w, &q) in prefix.iter().zip(quoted) {
        if names {
            if q || !KEYWORDS.contains(&w.as_str()) {
                continue;
            }
            names = false;
        }
        let keyword = !q && (KEYWORDS.contains(&w.as_str()) || matches!(w.as_str(), "time" | "function"));
        let assign = !q && assignment(w).is_some_and(|k| !k.starts_with(|c: char| c.is_ascii_digit()));
        let modifier = !q && matches!(w.as_str(), "builtin" | "noglob" | "nocorrect" | "-");
        if keyword && phase == 0 && !(w == "time" && std::mem::replace(&mut timed, true)) {
            names = w == "function";
        } else if assign && phase <= 1 {
            phase = 1;
        } else if modifier && phase < 2 {
            phase = 2;
        } else {
            return false;
        }
    }
    true
}

/// What a simple command runs once its wrappers are peeled off.
enum Run<'a> {
    Words(&'a [String]),
    /// Shell text a wrapper runs (`watch 'pkill x'`, `env -S '...'`), and how many of the words before it the wrappers used.
    Text(String, usize),
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
            // and the function's names: `function f { pkill x; }`, `function f g { pkill x; }`
            i += 1;
            while words.get(i).is_some_and(|n| !KEYWORDS.contains(&n.as_str())) {
                i += 1;
            }
            continue;
        }
        // `env -iC <folder>`: a bundle whose last option takes the next word.
        let ends_valued = |o: &str| !o.starts_with("--") && o.find(['C', 'u', 'P', 'S']).is_some_and(|at| at + 1 == o.len());
        // Each wrapper with its options that take a value.
        let name = base(w);
        let valued: &[&str] = match name {
            "sudo" => &["-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T"],
            "doas" => &["-u", "-C"],
            "env" => &["-u", "-C", "-P", "--chdir"],
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
            "command" | "builtin" | "nohup" | "time" | "chronic" | "repeat" | "noglob" | "nocorrect" | "-" => &[],
            _ => return Run::Words(rest(i)),
        };
        *bypass |= matches!(name, "sudo" | "doas" | "command" | "builtin");
        i += 1;
        while let Some(o) = words.get(i).filter(|o| o.starts_with('-')) {
            i += 1;
            match (name, o.as_str()) {
                (_, "--") => break,
                ("command", o) if o.contains(['v', 'V']) => return Run::Words(&[]),
                // `env -S '...'`, `-iS '...'`, `-iS'...'`: env splits the string into its own options and a command.
                ("env", "--split-string") => return Run::Text(format!("env {}", rest(i).join(" ")), i),
                ("env", o) if o.starts_with("--split-string=") => return Run::Text(format!("env {} {}", &o["--split-string=".len()..], rest(i).join(" ")), i),
                ("env", o) if !o.starts_with("--") && o[1..].find(['C', 'u', 'P', 'S']).is_some_and(|at| o.as_bytes()[at + 1] == b'S') => {
                    let attached = &o[o[1..].find('S').unwrap_or_default() + 2..];
                    return Run::Text(format!("env {attached} {}", rest(i).join(" ")), i);
                }
                ("script", "-c" | "--command") => return Run::Text(rest(i).first().cloned().unwrap_or_default(), i),
                // A command run with an emptied environment, another PATH or a variable unset (`env -u PATH`).
                ("env", o) if o == "-" || o.starts_with("--ignore") || o.starts_with("--unset") || (!o.starts_with("--") && o.contains(['i', 'P', 'u'])) => {
                    *bypass = true;
                    if valued.contains(&o) || ends_valued(o) {
                        i += 1;
                    }
                }
                (_, o) if valued.contains(&o) || (name == "env" && ends_valued(o)) => i += 1,
                _ => {}
            }
        }
        match name {
            "timeout" | "script" | "repeat" => i += 1, // the duration, the typescript file, the count
            "watch" => return Run::Text(rest(i).join(" "), i),
            "parallel" => return Run::Text(rest(i).iter().take_while(|w| !w.starts_with(":::")).cloned().collect::<Vec<_>>().join(" "), i),
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
    /// Whether each word has a `*`, `?`, `[` or `{` the shell expands: one outside any quotes. A quoted one is a plain
    /// character, and a partly quoted word with an unquoted one counts.
    live: Vec<bool>,
    /// Whether each word has a quoted or escaped part: such a word is no keyword.
    quoted: Vec<bool>,
    /// The files its redirections write or read.
    targets: Vec<String>,
    /// Each pipeline it sits in, from the outermost (one a compound command it's in sits in) in: the pipeline and
    /// its place there.
    pipes: Vec<(usize, usize)>,
    /// Its heredoc bodies and herestrings.
    stdin: String,
    /// The subshells it runs in (`( )`, `$( )`, backticks), outermost first, by number: a `cd` there stays there.
    scope: Vec<usize>,
    /// Its `&&` list: the commands joined by `&&` or `|` since the last `;`, newline, `||` or `&`. The ones after a `cd` in
    /// it only run when that `cd` worked.
    chain: usize,
    /// It is not the last command of its pipeline.
    piped: bool,
    /// It may not run, or not in the shell the next command runs in: it follows `&&`, `||` or `|`, sits in an `if`, a loop
    /// or a `case`, or is sent to the background with `&`. A `cd` of it may leave the shell where it was.
    maybe: bool,
    /// A `||` came before it in its list (`a || cd x && b`: `b` runs when the `cd` didn't).
    or: bool,
    /// `NAME()` follows it: it is a function's name.
    defines: bool,
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
    /// An `if`, a loop, a `case`, a function body or a group that may not run: its commands may not run.
    cond: bool,
    /// The command that opened it, and whether `end` closes it (zsh `foreach`).
    start: usize,
    foreach: bool,
}

#[derive(Default)]
struct Parse {
    cmds: Vec<Cmd>,
    pipes: usize,
    /// The subshells open, and how many there have been.
    scope: Vec<usize>,
    scopes: usize,
    /// The `&&` list being lexed.
    chain: usize,
    /// A `||` came before in the list being lexed.
    or: bool,
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
    /// The word has an unquoted `*`, `?`, `[` or `{`.
    live: bool,
    /// What the next word is a redirection's target of.
    redirect: Option<Redirect>,
    frames: Vec<Frame>,
    /// Heredocs started on this line: their delimiter and command.
    heredocs: Vec<(String, usize)>,
    /// A `&&`, `||` or `|` ended the line so far: a newline continues the list.
    open_list: bool,
    /// The word has a quoted or escaped part: it is never a keyword.
    quoted: bool,
    /// Where the group (or `if`, loop) that just closed began: a `|` or `&` after it puts all of it in a subshell.
    closed: Option<(usize, bool)>,
    /// The group a zsh `always` follows: its `{` block goes on with it.
    always: Option<(usize, bool)>,
    /// A `case` was opened and its `in` (or zsh's `{`) hasn't come.
    case_pending: bool,
    /// The command the last `(` ended: `()` right after it makes it a function's name.
    paren_cmd: Option<usize>,
    /// A function's name was defined (`f()`, `function f`, `()`): its body, a `{` group, may not run.
    func_next: bool,
}

impl Level {
    fn new() -> Self {
        Level { cur: None, word: String::new(), in_word: false, live: false, redirect: None, frames: vec![Frame::default()], heredocs: vec![], open_list: false, quoted: false, closed: None, always: None, paren_cmd: None, case_pending: false, func_next: false }
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    /// The command being lexed, started if need be.
    fn cmd(&mut self, p: &mut Parse) -> usize {
        *self.cur.get_or_insert_with(|| {
            let maybe = self.open_list || self.frames.iter().any(|f| f.cond);
            self.open_list = false;
            p.cmds.push(Cmd { scope: p.scope.clone(), chain: p.chain, maybe, or: p.or, ..Default::default() });
            p.cmds.len() - 1
        })
    }

    fn end_word(&mut self, p: &mut Parse) {
        let quoted = std::mem::take(&mut self.quoted);
        if !std::mem::take(&mut self.in_word) {
            self.word.clear();
            self.live = false;
            return;
        }
        let word = std::mem::take(&mut self.word);
        let live = std::mem::take(&mut self.live);
        let id = self.cmd(p);
        let cmd = &mut p.cmds[id];
        match self.redirect.take() {
            Some(Redirect::File) => cmd.targets.push(word),
            Some(Redirect::Here) => {
                cmd.stdin += &word;
                cmd.stdin.push('\n');
            }
            None => self.command_word(p, id, word, live, quoted),
        }
    }

    /// Adds a word to the command, opening or closing a compound command when it's a keyword in command position.
    /// A quoted or escaped word is never a keyword.
    fn command_word(&mut self, p: &mut Parse, id: usize, word: String, live: bool, quoted: bool) {
        let closed = self.closed.take();
        if word != "{" {
            self.always = None;
        }
        let keyword = !quoted;
        if self.frame().pattern == Some(true) && !(keyword && matches!(word.as_str(), "esac" | "}")) {
            return; // a `case` pattern
        }
        let first = keyword && command_position(&p.cmds[id].words, &p.cmds[id].quoted);
        // A function's body that isn't a group (`f() cd x`) is the command that follows.
        if self.func_next && p.cmds[id].words.is_empty() && word != "{" {
            p.cmds[id].maybe = true;
            self.func_next = false;
        }
        if first {
            let cond = Frame { cond: true, start: id, ..Default::default() };
            match word.as_str() {
                "{" => {
                    // The command the `{` begins (`{ cd x; }`) is in the group.
                    let after = self.always.take();
                    let cond = p.cmds[id].maybe || self.func_next || after.is_some_and(|a| a.1);
                    p.cmds[id].maybe = cond;
                    self.func_next = false;
                    self.frames.push(Frame { cond, start: after.map_or(id, |a| a.0), ..Default::default() });
                }
                "while" | "until" | "if" | "for" | "select" => self.frames.push(cond),
                "foreach" => self.frames.push(Frame { foreach: true, ..cond }),
                // zsh `repeat 0 cd x` and `repeat 0; do cd x; done`.
                "repeat" => {
                    p.cmds[id].maybe = true;
                    self.frames.push(cond);
                }
                "case" => self.case_pending = true,
                "always" => self.always = closed,
                "function" => self.func_next = true,
                "}" | "done" | "fi" | "esac" if self.frames.len() > 1 => self.pop(),
                "end" if self.frames.len() > 1 && self.frame().foreach => self.pop(),
                _ => {}
            }
        }
        // `in`, or zsh's `{`, after a `case`'s word.
        let opens_case = keyword && self.case_pending && (word == "in" || (word == "{" && !first));
        // A `}` ends a group (zsh needs no `;` before it) and the command in it: what follows isn't in the list the
        // group's last `cd` stopped, and `always { ... }` is a command of its own.
        let closes = keyword && word == "}" && self.frames.len() > 1;
        let word_is_always = word == "always";
        p.cmds[id].words.push(word);
        p.cmds[id].live.push(live);
        // `always` is a keyword only right after a group's `}`: anywhere else it is a command's name.
        p.cmds[id].quoted.push(quoted || (word_is_always && closed.is_none()));
        if closes {
            if !first {
                self.pop();
            }
            p.chain += 1;
            return self.end(p);
        }
        if opens_case {
            self.case_pending = false;
            self.end(p);
            self.frames.push(Frame { pattern: Some(true), cond: true, start: id, ..Default::default() });
        }
    }

    /// Closes the compound command open.
    fn pop(&mut self) {
        if let Some(f) = self.frames.pop() {
            self.closed = Some((f.start, f.cond));
        }
    }

    /// A `|` or `&` right after a group, `if` or loop closed: all of it runs in a subshell.
    fn background_group(&mut self, p: &mut Parse) {
        if let Some((start, _)) = self.closed.take() {
            for c in &mut p.cmds[start..] {
                c.maybe = true;
            }
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
        self.case_pending = false;
        let f = self.frame();
        (f.pipe, f.place) = (None, 0);
    }

    /// `|`: the next command reads this one's output.
    fn pipe(&mut self, p: &mut Parse) {
        self.end_word(p);
        if let Some(id) = self.cur {
            p.cmds[id].piped = true;
        }
        self.background_group(p);
        self.end(p);
        self.open_list = true;
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
                        l.quoted = true;
                    }
                    i += 1;
                }
            }
            '$' if next == Some('\'') => {
                l.in_word = true;
                l.quoted = true;
                i = ansi_c_quoted(c, i + 1, &mut l.word);
            }
            '\'' => {
                l.in_word = true;
                l.quoted = true;
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
                l.quoted = true;
                while let Some(&q) = c.get(i) {
                    i += 1;
                    match q {
                        '"' => break,
                        '\\' => {
                            l.word.extend(c.get(i));
                            i += 1;
                        }
                        '$' if c.get(i) == Some(&'(') => {
                            l.word.push('$');
                            l.cmd(p);
                            i = lex(p, c, i + 1, Some(')'));
                        }
                        '`' => {
                            l.word.push('$');
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
                l.word.push('$');
                l.cmd(p);
                i = lex(p, c, i, Some('`'));
            }
            '$' | '<' | '>' if next == Some('(') => {
                l.in_word = true;
                if ch == '$' {
                    l.word.push('$');
                }
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
                let named = if l.in_word { Some(l.cmd(p)) } else { l.cur };
                l.end(p);
                l.paren_cmd = named;
                l.frames.push(Frame::default());
                p.open_scope();
            }
            ')' if stop == Some(')') && depth == 0 => break,
            ')' => {
                if stop == Some(')') {
                    depth -= 1;
                }
                l.end(p);
                // `()` after a name, or alone (zsh): a function whose body follows.
                if c[..i - 1].iter().rev().find(|ch| !matches!(ch, ' ' | '\t')) == Some(&'(') {
                    l.func_next = true;
                    if let Some(id) = l.paren_cmd.take() {
                        p.cmds[id].defines = true;
                    }
                }
                if l.frames.len() > 1 {
                    l.frames.pop();
                }
                if p.scope.len() > scope + stop.is_some() as usize {
                    p.scope.pop();
                }
            }
            ';' => {
                l.separate(p);
                p.chain += 1;
                p.or = false;
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
                // `&&` goes on with the list, `&` ends it.
                if next == Some('&') {
                    i += 1;
                    l.separate(p);
                    l.open_list = true;
                } else {
                    // A command sent to the background runs in a subshell.
                    l.end_word(p);
                    if let Some(id) = l.cur {
                        p.cmds[id].maybe = true;
                    }
                    l.background_group(p);
                    l.separate(p);
                    p.chain += 1;
                    p.or = false;
                }
            }
            '|' if next == Some('|') => {
                i += 1;
                l.separate(p);
                p.chain += 1;
                p.or = true;
                l.open_list = true;
            }
            '|' => {
                if next == Some('&') {
                    i += 1;
                }
                l.pipe(p);
            }
            '\n' => {
                // `case x` + newline + `in` goes on.
                let case_pending = l.case_pending;
                l.separate(p);
                l.case_pending = case_pending;
                if !l.open_list {
                    p.chain += 1;
                    p.or = false;
                }
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
                l.live |= matches!(ch, '*' | '?' | '[' | '{');
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
    let storage = std::fs::canonicalize(&state.cfg.storage_dir).unwrap_or_else(|_| state.cfg.storage_dir.clone());
    let deadline = tokio::time::Instant::now() + ASK_TIMEOUT;
    for change in req.changes {
        // Read again each time: an "Always allow" for one folder may cover the next.
        let Ok(Some(a)) = state.db.get_agent(&agent_id) else { break };
        // A folder the hook couldn't name: the user is asked about the projects folder as a whole.
        let unknown = change.folder == Path::new(UNKNOWN);
        let at = if unknown { projects.clone() } else { change.folder.clone() };
        if !unknown && (!at.starts_with(&projects) || in_agent_storage(&at, &storage)) {
            continue;
        }
        let folder = at.display().to_string();
        let reason = if unknown {
            format!("{} runs a command that may change one of your checkouts under {folder} (it can't tell which), not its own clone: {}", a.name, change.command)
        } else {
            format!("{} wants to change your checkout at {folder}, not its own clone: {}", a.name, change.command)
        };
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
            "env -iS'kill 1'",
            "env -S '-i kill 1'",
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
    const STORAGE: &str = "/nowhere/dev/loft/agent-local-storage";
    const CLONE: &str = "/nowhere/dev/loft/agent-local-storage/abc/loft";

    /// The folders the hook would ask Loft about, for `cmd` run in `cwd`, with Loft's ceiling at `STORAGE`.
    fn asks_all(cmd: &str, cwd: &str) -> Vec<PathBuf> {
        let input = json!({ "tool_name": "Bash", "cwd": cwd, "tool_input": { "command": cmd } }).to_string();
        outside_changes(&input, &[PathBuf::from(STORAGE)]).into_iter().map(|c| c.folder).collect()
    }

    #[test]
    fn only_the_storage_folder_loft_uses_is_trusted_not_any_of_that_name() {
        // A folder an agent made, or the old one in the checkout, named like the storage folder.
        for stray in ["/nowhere/dev/loft/agent-local-storage-old/x", "/nowhere/elsewhere/agent-local-storage/x", "/nowhere/agent-local-storage/x"] {
            let input = json!({ "tool_name": "Bash", "cwd": stray, "tool_input": { "command": "cargo build" } }).to_string();
            assert_eq!(outside_changes(&input, &[PathBuf::from("/data/loft/loft-1a2b3c4d/agent-local-storage")]), vec![Change { folder: stray.into(), command: "cargo build".into() }]);
            assert!(!in_agent_storage(Path::new(stray), Path::new("/data/loft/loft-1a2b3c4d/agent-local-storage")));
        }
        // With no ceiling (not run by Loft), nothing is trusted.
        let input = json!({ "tool_name": "Bash", "cwd": CLONE, "tool_input": { "command": "cargo build" } }).to_string();
        assert_eq!(outside_changes(&input, &[]).len(), 1);
        assert_eq!(asks("cargo build", CLONE), None, "the agent's own clone");
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
            ("pnpm run build", USER),
            ("npm run build", USER),
            // A folder option written as the guard reads it, inside the agent's own clone.
            ("pnpm -Cfrontend test", CLONE),
            ("npm -C frontend test", CLONE),
            ("pnpm --filter web -C frontend build", CLONE),
        ] {
            assert_eq!(asks(cmd, cwd), None, "should run: {cmd} (in {cwd})");
        }
        assert!(asks("git pull", USER).is_some());
        assert!(outside_changes("not json", &[]).is_empty());
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
    fn a_cd_to_a_folder_named_by_a_variable_asks() {
        for cmd in [
            "cd /nowhere/dev/loft && cd /nowhere/dev/loft/agent-local-storage && cd $OLDPWD && git reset --hard",
            "D=/nowhere/dev/loft; cd $D && git reset --hard",
            "cd \"$(git rev-parse --show-toplevel)\" && git reset --hard",
            "cd `pwd` && git reset --hard",
            "pushd $X && git reset --hard",
            "cd ${D}/sub && git reset --hard",
            "cd -P $D && git reset --hard",
            "popd $X; git reset --hard",
        ] {
            assert!(!asks_all(cmd, CLONE).is_empty(), "should ask: {cmd}");
        }
        for cmd in ["cd -; git pull", "cd /nowhere/dev/loft/agent-local-storage/abc/loft && git pull", "cd loft && git pull"] {
            assert_eq!(asks(cmd, CLONE), None, "should run: {cmd}");
        }
    }

    #[test]
    fn a_cd_that_may_not_run_leaves_the_folder_before_it_as_a_place() {
        let (u, w) = (USER, "/nowhere/dev/loft/agent-local-storage/abc");
        for cmd in [
            format!("cd {u}; [ -d {w}/zzz ] && cd {w}; pnpm install"),
            format!("cd {u}; false && cd {w}; pnpm install"),
            format!("cd {u}; true || cd {w}; pnpm install"),
            format!("cd {u}; if false; then cd {w}; fi; pnpm install"),
            format!("cd {u}; while false; do cd {w}; done; pnpm install"),
            format!("cd {u}; for x in; do cd {w}; done; pnpm install"),
            format!("cd {u}; case $X in a) cd {w};; esac; pnpm install"),
            format!("cd {u}; cd {w} | cat; pnpm install"),
            format!("cd {u}; cd {w} &; pnpm install"),
            format!("cd {u}; cd {w} & pnpm install"),
        ] {
            assert!(!asks_all(&cmd, CLONE).is_empty(), "should ask: {cmd}");
        }
        for cmd in [
            format!("cd {w} && pnpm install"),
            "cd backend && cargo test".to_string(),
            format!("cd {w}; pnpm install"),
            format!("[ -d {w} ] && cd {w}; pnpm install"),
            format!("if true; then cd {w}; pnpm install; fi"),
            format!("(cd {u}; false && cd {w}); pnpm install"),
        ] {
            assert_eq!(asks(&cmd, CLONE), None, "should run: {cmd}");
        }
    }

    #[test]
    fn a_cd_in_a_group_function_or_form_that_may_not_run_still_leaves_the_folder_before_it_as_a_place() {
        let (u, w) = (USER, "/nowhere/dev/loft/agent-local-storage/abc");
        for tail in [
            // `||` before the cd, then `&&`.
            format!("true || cd {w} && pnpm install"),
            format!("[ -d {w} ] || cd {w} && cargo build"),
            format!("true || pushd {w} && pnpm install"),
            format!("true || true || cd {w} && git reset --hard"),
            // A group that may not run, or is piped or sent to the background.
            format!("false && {{ true; cd {w}; }}; pnpm install"),
            format!("true || {{ true; cd {w}; }}; pnpm install"),
            format!("{{ cd {w}; }} | cat; pnpm install"),
            format!("{{ cd {w}; true; }} |& cat; pnpm install"),
            format!("true | {{ true; cd {w}; }} | cat; pnpm install"),
            format!("{{ cd {w}; }} & pnpm install"),
            format!("{{ {{ cd {w}; }}; }} & pnpm install"),
            format!("{{ cd {w}; }} > /dev/null & pnpm install"),
            // A quoted or escaped `}`, `fi`, `done` or `esac` closes nothing.
            format!("if false; then echo \"}}\"; cd {w}; fi; pnpm install"),
            format!("if false; then echo '}}' >> x.json; cd {w}; fi; pnpm install"),
            format!("if false; then echo \\}}; cd {w}; fi; pnpm install"),
            format!("for x in; do echo \"}}\"; cd {w}; done; pnpm install"),
            format!("case $X in a) echo \"}}\"; cd {w};; esac; pnpm install"),
            format!("if false; then \"fi\"; cd {w}; fi; pnpm install"),
            format!("for x in; do \"done\"; cd {w}; done; pnpm install"),
            format!("for x in; do \\done; cd {w}; done; pnpm install"),
            format!("case $X in a) \"esac\"; cd {w};; esac; pnpm install"),
            // popd.
            "false && popd; pnpm install".to_string(),
            "if false; then popd; fi; pnpm install".to_string(),
            "popd | cat; pnpm install".to_string(),
            "popd & pnpm install".to_string(),
            "[ -d zzz ] && popd -q; git reset --hard".to_string(),
            // A function body counts as run where it is defined.
            format!("f() {{ cd {w}; }}; pnpm install"),
            format!("function f {{ cd {w}; }}; pnpm install"),
            format!("f() {{ cd {w}; }}; f | cat; pnpm install"),
            format!("f() {{ cd {w}; }}; f & pnpm install"),
            format!("f() {{ cd {w}; }}; false && f; pnpm install"),
            format!("f()\n{{ cd {w}; }}; pnpm install"),
            format!("f ( ) {{ cd {w}; }}; pnpm install"),
            format!("() {{ cd {w} }} | cat; pnpm install"),
            format!("() {{ cd {w} }} & pnpm install"),
            // eval.
            format!("false && eval 'cd {w}'; pnpm install"),
            format!("if false; then eval 'cd {w}'; fi; pnpm install"),
            format!("eval 'cd {w}' | cat; pnpm install"),
            format!("eval 'cd {w}' & pnpm install"),
            // Forms the lexer opens a frame for only with care.
            format!("case $X\nin a) cd {w};; esac; pnpm install"),
            format!("time case $X in a) cd {w};; esac; pnpm install"),
            format!("case $X {{ a) cd {w};; }}; pnpm install"),
            format!("repeat 0; do cd {w}; done; pnpm install"),
            format!("repeat 0 cd {w}; pnpm install"),
            format!("foreach x ($EMPTY) cd {w}; end; pnpm install"),
            format!("time while false; do cd {w}; done; pnpm install"),
            // A `cd` another program runs doesn't move the shell.
            format!("env cd {w}; pnpm install"),
            format!("env cd {w} && pnpm install"),
            format!("nohup cd {w} && pnpm install"),
            format!("nice cd {w} && pnpm install"),
            format!("timeout 5 cd {w} && pnpm install"),
            format!("sudo cd {w} && pnpm install"),
            format!("xargs cd {w} && pnpm install"),
            format!("command time cd {w} && pnpm install"),
            format!("command cd {w} && pnpm install"),
            format!("/usr/bin/cd {w} && pnpm install"),
            format!("\\time cd {w} && pnpm install"),
            format!("\"time\" cd {w} && pnpm install"),
            format!("\"!\" cd {w} && pnpm install"),
            format!("if false; \"then\" cd {w}; fi && pnpm install"),
            format!("env pushd {w} && pnpm install"),
            format!("pushd {u}; env popd && pnpm install"),
            // Prefix words in any other order are a command of their own.
            format!("noglob time cd {w}; pnpm install"),
            format!("noglob ! cd {w}; pnpm install"),
            format!("noglob FOO=1 cd {w}; pnpm install"),
            format!("noglob nocorrect cd {w}; pnpm install"),
            format!("builtin time cd {w}; pnpm install"),
            format!("builtin FOO=1 cd {w}; pnpm install"),
            format!("time time cd {w}; pnpm install"),
            format!("\"FOO=1\" cd {w}; pnpm install"),
            format!("FOO\\=1 cd {w}; pnpm install"),
            format!("1FOO=1 cd {w}; pnpm install"),
            // A `cd` redefined, or disabled.
            format!("cd() {{ :; }}; cd {w}; pnpm install"),
            format!("function cd {{ :; }}; cd {w}; pnpm install"),
            format!("function cd {{ :; }}; pushd {w}; pnpm install"),
            format!("pushd() {{ :; }}; pushd {w}; pnpm install"),
            format!("function cd() {{ :; }}; cd {w}; pnpm install"),
            format!("{{ cd() {{ :; }} }}; cd {w}; pnpm install"),
            format!("f() {{ cd() {{ :; }}; }}; f; cd {w}; pnpm install"),
            format!("! function cd {{ :; }}; cd {w}; pnpm install"),
            format!("f cd() {{ :; }}; cd {w}; pnpm install"),
            format!("functions[cd]=':'; cd {w}; pnpm install"),
            format!("autoload cd; cd {w}; pnpm install"),
            "autoload -Uz popd; popd; pnpm install".to_string(),
            format!("always cd {w}; pnpm install"),
            format!("disable cd; cd {w}; pnpm install"),
            format!("disable -r cd; cd {w} && pnpm install"),
            // zsh `always`: its block goes on with the group before it.
            format!("{{ cd {w} }} always {{ true }} | cat; pnpm install"),
            format!("{{ cd {w} }} always {{ true }} & pnpm install"),
            format!("{{ cd {w} }} always {{ true }} |& cat; pnpm install"),
            format!("false && {{ true }} always {{ cd {w} }}; pnpm install"),
            format!("true || {{ true }} always {{ cd {w} }}; pnpm install"),
        ] {
            let cmd = format!("cd {u}; {tail}");
            assert!(!asks_all(&cmd, CLONE).is_empty(), "should ask: {cmd}");
        }
        // A quoted `time` or `!` is no keyword: `case` isn't opened and what follows isn't swallowed. And a function with
        // more than one name still has its body read.
        for cmd in [
            format!("\"time\" case x in; git -C {u} reset --hard"),
            format!("\\time case; echo in; git -C {u} reset --hard"),
            format!("\"!\" case; echo in; git -C {u} reset --hard"),
            format!("\"!\" case x in; git -C {u} reset --hard"),
            format!("case x; echo in; git -C {u} reset --hard"),
            format!("function f g {{ cd {u}; }}; f; pnpm install"),
            format!("function f g h {{ cd {u}; }}; pnpm install"),
            // An anonymous function after a `popd` or `pushd` of its own list doesn't hide it.
            format!("pushd {u}; pushd {w}; popd; () {{ :; }}; pnpm install"),
            format!("cd {u}; cd {w}; popd; () {{ :; }}; pnpm install"),
            // zsh's `-` precommand modifier before a plain command.
            format!("- cd {u}; pnpm install"),
        ] {
            assert!(!asks_all(&cmd, CLONE).is_empty(), "should ask: {cmd}");
        }
        // The same from the agent's own clone: where it started is no checkout, so the places count nothing.
        for cmd in [
            format!("true || cd {w} && pnpm install"),
            format!("{{ cd {w}; }} | cat; pnpm install"),
            format!("if false; then echo \"}}\"; cd {w}; fi; pnpm install"),
            "popd | cat; pnpm install".to_string(),
            format!("f() {{ cd {w}; }}; pnpm install"),
            format!("false && eval 'cd {w}'; pnpm install"),
            format!("case $X\nin a) cd {w};; esac; pnpm install"),
            format!("repeat 0; do cd {w}; done; pnpm install"),
            // What the guard ran before keeps running.
            format!("cd {w} && pnpm install"),
            "cd backend && cargo test".to_string(),
            "cd backend || exit 1; cargo test".to_string(),
            "cd backend && pnpm dev &".to_string(),
            "for x in a b; do cd backend; cargo test; cd ..; done".to_string(),
            "case $X in a) cd backend;; esac; cargo test".to_string(),
            "{ cd backend; cargo test; }".to_string(),
            "f() { cd backend; cargo test; }; f".to_string(),
            "function f g { cd backend; cargo test; }; f".to_string(),
            "builtin cd backend && cargo test".to_string(),
            "- cd backend && cargo test".to_string(),
            "time ! cd backend || cargo test".to_string(),
            "FOO=1 BAR=2 builtin cd backend && cargo test".to_string(),
            "noglob cd backend && cargo test".to_string(),
            "cd backend; cd() { :; }".to_string(),
            "f() { :; }; () { :; }; cd backend && cargo test".to_string(),
            "autoload zargs; cd backend && cargo test".to_string(),
            "functions[g]=':'; cd backend && cargo test".to_string(),
            "noglob cd backend && cargo test".to_string(),
            "nocorrect cd backend && cargo test".to_string(),
            "time cd backend && cargo test".to_string(),
            "! cd backend || cargo test".to_string(),
            "FOO=1 cd backend && cargo test".to_string(),
            "\\cd backend && cargo test".to_string(),
            "if true; then cd backend && cargo test; fi".to_string(),
            "{ cd backend } always { true }; cargo test".to_string(),
            "eval 'cd backend && cargo test'".to_string(),
            "echo \"}\" && cargo test".to_string(),
        ] {
            assert_eq!(asks(&cmd, CLONE), None, "should run: {cmd}");
        }
    }

    #[test]
    fn git_commands_that_make_nothing_dont_hide_the_folder_of_the_next() {
        const WS: &str = "/nowhere/dev/loft/agent-local-storage/abc";
        // The own clone, which isn't there for the guard to look at: an earlier fetch, commit or push can't have made it.
        for cmd in [
            "git -C ./loft fetch && git -C ./loft pull",
            "git -C ./loft status; git -C ./loft log -1",
            "git -C loft fetch && git -C loft pull",
            "git -C loft commit -m x && git -C loft push",
            "cd loft && git commit -m x && git push",
            "cd loft/backend && git add -A && git commit -m x && git push",
            "cd loft && git fetch && git pull",
            "git commit -am x && git push -u origin main",
            "git fetch --prune --tags && git -C loft push --set-upstream origin x",
            "git log --graph --oneline -p && git status -sb && git diff --name-only && git -C loft push --force-with-lease",
            "git show -p && git diff -p && git -C loft push",
        ] {
            assert_eq!(asks(cmd, WS), None, "should run: {cmd}");
        }
        assert_eq!(asks("cd backend && git commit -m x && git push", CLONE), None);
        assert_eq!(asks("cd backend && git push", CLONE), None);
        // The same aimed at the user's checkout.
        for cmd in [
            "git -C ./loft fetch && git -C ./loft pull",
            "git -C ./loft status; git -C ./loft log -1; git -C ./loft push",
            "git -C loft commit -m x && git -C loft push",
            "cd loft && git commit -m x && git push",
            "cd loft/backend && git add -A && git commit -m x && git push",
        ] {
            assert!(asks_all(cmd, "/nowhere/dev").contains(&PathBuf::from(USER)) || asks_all(cmd, "/nowhere/dev").iter().any(|f| f.starts_with(USER)), "should ask: {cmd}");
        }
        assert!(!asks_all("cd backend && git commit -m x && git push", USER).is_empty());
        // What may make the folder (a pull writes the work tree, a clone, a mkdir, an option that runs a program) still hides it.
        for cmd in [
            "git pull && git -C ./loft push",
            "git clone x loft && git -C ./loft push",
            "mkdir loft && git -C ./loft push",
            "git fetch --upload-pack=./mk && git -C ./loft push",
            "git -c core.x=y commit -m x && git -C ./loft push",
            // Options git takes by a unique prefix, the environment, a program named git, and what the shell expands.
            "git fetch --upload-pa='ln -s /x lnk;:' . && git -C ./loft push",
            "git fetch --upl ./mk . && git -C ./loft push",
            "git push --receive-pa=./mk . main && git -C ./loft push",
            "git push --exe=./mk . main && git -C ./loft push",
            "GIT_SSH_COMMAND=./mk git fetch ssh://h/r && git -C ./loft push",
            "env GIT_SSH_COMMAND=./mk git fetch ssh://h/r && git -C ./loft push",
            "GIT_EDITOR=./mk git commit && git -C ./loft push",
            "EDITOR=./mk git commit && git -C ./loft push",
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.fsmonitor GIT_CONFIG_VALUE_0=./mk git status && git -C ./loft push",
            "GIT_EXTERNAL_DIFF=./mk git diff && git -C ./loft push",
            "PATH=./bin:$PATH git fetch && git -C ./loft push",
            "git fetch {--upload-pack=./mk,.} && git -C ./loft push",
            "git fetch $OPT . && git -C ./loft push",
            "O=--upload-pack=./mk; git fetch $O . && git -C ./loft push",
            "git fetch \"$@\" && git -C ./loft push",
            "xargs git fetch < args && git -C ./loft push",
            "./git fetch && git -C ./loft push",
            "/tmp/x/git status && git -C ./loft push",
            "sudo git fetch && git -C ./loft push",
            "command git fetch && git -C ./loft push",
            "git fetch --upload-pa='ln -s /x lnk;:' . && cd lnk && git push",
            // A variable the shell already exports, set by a command of its own.
            "PATH=./bin:$PATH; git fetch; git -C ./loft push",
            "HOME=/x; git status; git -C ./loft push",
            "XDG_CONFIG_HOME=/x; git status; git -C ./loft push",
            "EDITOR=./mk; git commit; git -C ./loft push",
            "PATH=./bin:$PATH; git fetch && cd lnk && git push",
            "HOME=/x; git status && cd lnk && git push",
            "export PATH=./bin:$PATH; git fetch; git -C ./loft push",
            "typeset -x HOME=/x; git status; git -C ./loft push",
            "declare -x EDITOR=./mk; git commit; git -C ./loft push",
            "git add -p && git -C ./loft push",
            "git commit -p && git -C ./loft push",
            "git fetch -p && git -C ./loft push",
            "git checkout main && cd loft && git push",
        ] {
            assert!(!asks_all(cmd, WS).is_empty(), "should ask: {cmd}");
        }
    }

    #[test]
    fn a_folder_option_with_a_variable_asks_and_chdir_is_cd() {
        const WORKSPACE: &str = "/nowhere/dev/loft/agent-local-storage/abc";
        for cmd in [
            "D=/nowhere/dev/loft; git -C \"$D\" reset --hard",
            "D=/nowhere/dev/loft; git --git-dir=$D/.git --work-tree=$D reset --hard",
            "D=/nowhere/dev/loft; git --git-dir $D/.git --work-tree $D reset --hard",
            "for d in /nowhere/dev/loft; do git -C $d reset --hard; done",
            "git -C $(git rev-parse --show-toplevel) reset --hard",
            "D=/nowhere/dev/loft; pnpm -C $D install",
            "D=/nowhere/dev/loft; pnpm --dir=$D install",
            "pnpm -C $OLDPWD install",
            "D=/nowhere/dev/loft; npm --prefix $D install",
            "D=/nowhere/dev/loft; cargo --manifest-path $D/Cargo.toml build",
            "D=/nowhere/dev/loft; xcodebuild -project $D/X.xcodeproj build",
            "D=/nowhere/dev/loft; env -C $D pnpm install",
            "D=/nowhere/dev/loft; git -c core.worktree=$D reset --hard",
            "D=/nowhere/dev/loft; git -c core.worktree=$D checkout .",
            "D=/nowhere/dev/loft; chdir $D && pnpm install",
            "K=core.worktree; git -c $K=/nowhere/dev/loft checkout .",
            "E=core.worktree=/nowhere/dev/loft; git -c $E checkout .",
            "pnpm -C/nowhere/dev/loft install",
            "npm -C /nowhere/dev/loft install",
            "pnpm --filter x -C /nowhere/dev/loft install",
            "pnpm --filter x --silent -C /nowhere/dev/loft install",
            "G=GIT_DIR; env $G=/nowhere/dev/loft/.git git reset --hard",
            "E=GIT_DIR=/nowhere/dev/loft/.git; env $E git reset --hard",
            // An option before the subcommand, a word after it; bundled, abbreviated and repeated folder options.
            "cd /nowhere/dev/loft && npm -s install lodash",
            "cd /nowhere/dev/loft && pnpm --silent add lodash",
            "cd /nowhere/dev/loft && cargo -q test some_test",
            "cd /nowhere/dev/loft && cargo +nightly test name",
            "npm --prefix /nowhere/dev/loft --silent install lodash",
            "pnpm -C /nowhere/dev/loft --silent add lodash",
            "pnpm --filter x --silent add lodash -C /nowhere/dev/loft",
            "npm -sC /nowhere/dev/loft install",
            "npm --prefi /nowhere/dev/loft install",
            "npm --prefi=/nowhere/dev/loft install",
            "npm -C . -C /nowhere/dev/loft install",
            "npm --prefix . -C /nowhere/dev/loft install",
        ] {
            for cwd in [CLONE, WORKSPACE] {
                assert!(!asks_all(cmd, cwd).is_empty(), "should ask in {cwd}: {cmd}");
            }
        }
        // `$PWD` and `$(pwd)` are variables like any other: they ask, and the plain forms run.
        for cmd in [
            "git -C \"$PWD\" status",
            "git -C ${PWD} pull",
            "git -C \"$(pwd)\" pull",
            "pnpm --dir \"$PWD/frontend\" install",
            "cargo build --manifest-path \"$PWD/backend/Cargo.toml\"",
            // The command sets `PWD`, redefines `pwd`, or runs the option in another folder than the shell expands it in.
            "PWD=/nowhere/dev/loft; git -C \"$PWD\" reset --hard",
            "export PWD=/nowhere/dev/loft; git -C \"$PWD\" reset --hard",
            "read PWD <<< /nowhere/dev/loft; git -C \"$PWD\" reset --hard",
            "for PWD in /nowhere/dev/loft; do git -C \"$PWD\" reset --hard; done",
            "pwd() { echo /nowhere/dev/loft; }; git -C \"$(pwd)\" reset --hard",
            "alias pwd='echo /nowhere/dev/loft'; git -C \"$(pwd)\" reset --hard",
            "env -C /nowhere/dev/loft git -C \"$PWD\" reset --hard",
            "sh -c \"cd /nowhere/dev/loft && git -C $PWD reset --hard\"",
            "eval \"cd /nowhere/dev/loft; git -C $PWD reset --hard\"",
        ] {
            assert!(!asks_all(cmd, CLONE).is_empty(), "should ask: {cmd}");
        }
        for cmd in ["git -C . status", "git status", "pnpm --dir frontend install", "cargo build --manifest-path backend/Cargo.toml"] {
            assert_eq!(asks(cmd, CLONE), None, "should run: {cmd}");
        }
        // `chdir` goes where `cd` goes.
        assert_eq!(asks("chdir /nowhere/dev/loft && git pull", CLONE), Some(PathBuf::from(USER)));
        assert_eq!(asks("chdir loft && git pull", CLONE), None);
        // Literal folders behave as before.
        for cmd in ["git -C loft status", "git -C loft reset --hard", "pnpm -C . install", "cargo --manifest-path Cargo.toml build", "env -C . pnpm install"] {
            assert_eq!(asks(cmd, CLONE), None, "should run: {cmd}");
        }
        assert!(!asks_all("git -C /nowhere/dev/loft reset --hard", CLONE).is_empty());
        // What env -C $D runs is that command's folder only.
        assert_eq!(asks("env -C $D echo hi; git status", CLONE), None);
    }

    #[test]
    fn zsh_autopushd_and_a_failed_cd_before_a_semicolon_ask() {
        for cmd in [
            "cd /nowhere/dev/loft && cd /nowhere/dev/loft/agent-local-storage && popd && git reset --hard",
            "cd /nowhere/dev/loft; cd /nowhere/dev/loft/agent-local-storage; pushd; git reset --hard",
            "pushd /nowhere/dev/loft && pushd /tmp && popd +0 && git reset --hard",
            "cd /nowhere/dev/loft; cd /tmp/loft-guard-no-such-folder; git reset --hard",
            "cd /nowhere/dev/loft && cd /tmp/loft-guard-no-such-folder ; cargo build",
        ] {
            assert!(!asks_all(cmd, CLONE).is_empty(), "should ask: {cmd}");
        }
        for cmd in ["cd /nowhere/dev/loft/agent-local-storage/abc/loft && cd src && popd && git pull", "pushd /tmp && popd && git pull", "cd nope; git pull"] {
            assert_eq!(asks(cmd, CLONE), None, "should run: {cmd}");
        }
    }

    #[test]
    fn autopushd_and_failed_cds_in_longer_forms_ask() {
        let (u, w) = (USER, "/nowhere/dev/loft/agent-local-storage/abc");
        // More than 8 places the shell may be: the guard gives up and asks.
        let many: String = (0..10).map(|n| format!("cd {w}/a{n}; ")).collect();
        for cmd in [
            format!("pushd -q /tmp; cd {u}; cd {w}; popd -q; git reset --hard"),
            format!("cd {u}; cd /tmp; cd {w}; popd; popd; git reset --hard"),
            format!("cd {u}; cd {w}; popd; pushd; pushd; git reset --hard"),
            format!("cd {u}; cd {w}; pushd; pushd; pushd; git reset --hard"),
            format!("cd {w}; {many}cd {u}; cd {w}; popd; git reset --hard"),
            // A failed cd after other commands, and after another failed one.
            format!("cd {u}; mkdir -p /tmp/x; cd {w}/nope; pnpm install"),
            format!("cd {u}; touch /tmp/x; cd {w}/nope; pnpm install"),
            format!("cd {u}; cd {w}/nope; cd {w}/nope2; git reset --hard"),
            // `cd -` after a failed cd or a popd.
            format!("cd {u}; cd {w}; cd {w}/nope; cd -; pnpm install"),
            format!("cd {u}; cd {w}; cd {w}/nope; cd -; git reset --hard"),
            format!("cd /tmp; cd {u}; cd {w}/nope; cd -; cd -; pnpm install"),
            format!("cd {u}; cd {w}; popd; cd -; cd -; pnpm install"),
            format!("cd {u}; cd {w}; pushd; cd -; cd -; pnpm install"),
            // A cd that works after a failed one doesn't make `cd -` known.
            format!("cd {u}; cd {w}/nope; cd {w}; cd -; pnpm install"),
            // A path through a missing folder and back up is refused too.
            format!("cd {u}; rm -rf {w}/real; cd {w}/real/..; pnpm install"),
            format!("cd {u}; mv {w}/real {w}/r2; cd {w}/real/..; pnpm install"),
            format!("cd {u}; cd {w}/nope/..; pnpm install"),
            format!("cd {u}; cd {w}/nope.txt/..; pnpm install"),
            format!("cd {u}; cd {w}/nope/../real; pnpm install"),
            format!("cd {u}; cd {w}/nope; cd {w}; cd -; git reset --hard"),
            format!("cd {u}; cd {w}/nope; cd; cd -; pnpm install"),
            format!("cd {u}; cd {w}/nope.txt; cd {w}; cd -; pnpm install"),
            format!("({u:?}; cd {u}; cd {w}/nope; cd {w}; cd -; pnpm install)").replacen(&format!("{u:?}; "), "", 1),
        ] {
            let asked = asks_all(&cmd, CLONE);
            assert!(asked.contains(&PathBuf::from(u)) || asked.iter().any(|f| f.as_os_str() == UNKNOWN), "should ask: {cmd}");
        }
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
            "git remote add up x", "git hash-object -w f", "git fsck --lost-found", "git merge-tree --write-tree a b", "git merge-tree a b c", "git hash-object -wt blob f", "git hash-object -wtblob f", "git fsck --lost-f", "git hash-object --write-t f", "git stash push", "git reflog expire --all", "git -C /nowhere/dev/loft branch -D x",
        ] {
            assert_eq!(asks(cmd, USER), user, "should ask: {cmd}");
        }
        for cmd in [
            "git status", "git log -5", "git diff", "git show HEAD", "git fetch", "git rev-parse HEAD", "git ls-files",
            "git blame x", "git grep foo", "git describe", "git remote -v", "git remote", "git remote get-url origin",
            "git branch", "git branch -a", "git branch -vv", "git branch --show-current", "git branch --list 'feat/*'",
            "git branch --merged main", "git tag", "git tag -l 'v*'", "git worktree list", "git config user.name",
            "git config --get user.name", "git config --list", "git stash show", "git reflog", "git --version",
            "git help", "git submodule status", "git diff a b | git patch-id --stable", "git diff-tree -r HEAD", "git diff-index HEAD",
            "git diff-files", "git hash-object f", "git check-ref-format refs/heads/x", "git check-attr -a f",
            "git show-branch", "git whatchanged", "git verify-commit HEAD", "git verify-tag v1", "git fsck", "git -c color.ui=never log",
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
            // The agent's own clone, in the storage folder Loft uses.
            let input = json!({ "tool_name": "Bash", "cwd": at(&ws), "tool_input": { "command": "git -C loft pull" } }).to_string();
            assert_eq!(outside_changes(&input, &[project.join("agent-local-storage")]), vec![]);
        }
        // A new repository is where it's made, not in the checkout around it; global config is no checkout's.
        let storage = [project.join("agent-local-storage")];
        for cmd in ["git clone /nowhere/dev/loft", "git clone https://github.com/x/loft.git", "git clone -b main /src loft2", "git init scratch-repo", "git init", "git config --global user.name x", "git config --system core.x y"] {
            let input = json!({ "tool_name": "Bash", "cwd": at(&ws), "tool_input": { "command": cmd } }).to_string();
            assert_eq!(outside_changes(&input, &storage), vec![], "should run: {cmd}");
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
        let in_storage_at = |cmd: &str, cwd: &Path| {
            let input = json!({ "tool_name": "Bash", "cwd": at(cwd), "tool_input": { "command": cmd } }).to_string();
            outside_changes(&input, &storage).into_iter().map(|c| c.folder).collect::<Vec<_>>()
        };
        let in_storage = |cmd: &str| in_storage_at(cmd, &ws);
        assert_eq!(in_storage("git clone /src loop"), Vec::<PathBuf>::new());
        // A repository kept apart from its folder is judged by where it goes too.
        let apart = project.join("repo.git");
        for cmd in [format!("git clone --separate-git-dir {} /src mine", at(&apart)), format!("git init --separate-git-dir={} mine", at(&apart))] {
            assert_eq!(in_storage(&cmd), vec![apart.clone()], "should ask: {cmd}");
        }
        assert_eq!(in_storage("git clone --separate-git-dir ../x/mine.git /src mine"), Vec::<PathBuf>::new());
        // Before this command, the shell may have been anywhere: the workspace stands for it.
        for cmd in ["cd -; git checkout main", "cd loft && popd && git reset --hard"] {
            assert_eq!(asks(cmd, &at(&ws)), Some(project.clone()), "should ask: {cmd}");
        }
        // autopushd: the popd goes back to where the command began or to the folder a cd left; both are asked about.
        assert!(asks_all("cd loft; cd /tmp; popd; git stash", &at(&ws)).contains(&project));
        assert_eq!(in_storage_at("cd -; git checkout main", &clone), Vec::<PathBuf>::new());
        // The guard doesn't guess what a mkdir or cargo new does: a cd after one with `;` may fail, and the shell stays.
        std::fs::write(ws.join("notes/a.txt"), "").unwrap();
        let (pr, w) = (project.display(), ws.display());
        let nope = format!("cd {w}/nope; pnpm install");
        for cmd in [
            format!("cd {pr}; mkdir {w}/nope/deep; cd {w}/nope/deep; pnpm install"),
            format!("cd {pr}; mkdir -p {w}/notes/a.txt/x; cd {w}/notes/a.txt/x; pnpm install"),
            format!("cd {pr}; mkdir {w}/notes/a.txt; cd {w}/notes/a.txt; pnpm install"),
            format!("cd {pr}; mkdir {w}/made; cd {w}/made/sub; pnpm install"),
            format!("cd {pr}; mkdir {w}/made5; cd {w}/made5; pnpm install"),
            format!("cd {pr}; mkdir -m 755 {w}/out; cd {w}/out; pnpm install"),
            format!("cd {pr}; cargo new {w}/made6; cd {w}/made6; cargo build"),
            format!("cd {pr}; [ -d {w}/out ] || mkdir {w}/out; cd {w}/out; pnpm install"),
            format!("cd {pr}; if [ ! -d {w}/out ]; then mkdir {w}/out; fi; cd {w}/out; pnpm install"),
            // A mkdir that doesn't run, or fails.
            format!("cd {pr}; [ -f {w}/flag ] &&\nmkdir {w}/nope; {nope}"),
            format!("cd {pr}; [ -f {w}/flag ] || # note\nmkdir {w}/nope; {nope}"),
            format!("cd {pr}; f() {{ mkdir {w}/nope; }}; {nope}"),
            format!("cd {pr}; cat <<EOF\nmkdir {w}/nope\nEOF\n{nope}"),
            format!("cd {pr}; echo a # b; mkdir {w}/nope\n{nope}"),
            format!("cd {pr}; xargs mkdir {w}/nope </dev/null; {nope}"),
            format!("cd {pr}; sudo -n mkdir {w}/nope; {nope}"),
            format!("cd {pr}; mkdir -pm bad {w}/nope; {nope}"),
            format!("cd {pr}; mkdir --version {w}/nope; {nope}"),
            format!("cd {pr}; mkdir {w}/zz/../nope; {nope}"),
            format!("cd {pr}; mkdir {w}/nope > {w}/zzz/log; {nope}"),
            format!("cd {pr}; cargo new {w}/nope --vcs; {nope}"),
            // A failed cd that doesn't stop the list: negated, not last in its pipeline, or text with its own lists.
            format!("cd {pr}; ! cd {w}/nope && pnpm install"),
            format!("cd {pr}; cd {w}/nope | cat && pnpm install"),
            format!("cd {pr}; cd {w}/nope | true && pnpm install"),
            format!("cd {pr}; cd {w}/nope |& cat && pnpm install"),
            format!("cd {pr}; eval 'true; true; cd {w}/nope'; pnpm install"),
            // A `{ }` group holding the failed cd and a `&&`.
            format!("cd {pr}; {{ cd {w}/nope && true }} | cat && pnpm install"),
            format!("cd {pr}; {{ cd {w}/nope }} always {{ pnpm install }}"),
            // After `||` anywhere past the cd, the rest runs when it failed.
            format!("cd {pr}; cd {w}/nope || true && pnpm install"),
            format!("cd {pr}; cd {w}/nope && true || pnpm install"),
        ] {
            let cmd = cmd.replace("\\n", "\n");
            assert!(in_storage_at(&cmd, &ws).contains(&project), "should ask: {cmd}");
        }
        // What follows a cd through `&&` only runs when it worked, whatever the mkdir did.
        for cmd in [
            format!("cd {pr} && mkdir {w}/nope/deep && cd {w}/nope/deep && pnpm install"),
            format!("cd {pr} && mkdir -p {w}/made4/deep && cd {w}/made4/deep && pnpm install"),
            format!("cd {pr} && mkdir -m 755 {w}/out && cd {w}/out && cd .. && git add -A"),
            format!("cd {pr} && cd {w}/nope &&\n pnpm install"),
            format!("cd {pr} && cd {w}/nope |\n cat && pnpm install"),
        ] {
            let cmd = cmd.replace("\\n", "\n");
            assert_eq!(in_storage_at(&cmd, &ws).iter().filter(|f| **f != project).count(), 0, "should run: {cmd}");
        }
        // A folder the command makes and leaves again is no failed cd: these ran on main and still run.
        for cmd in ["mkdir -p out && cd out && cd .. && git add -A", "cargo new foo && cd foo && cargo build && cd .. && git add -A", "{ cd backend && cargo test; }", "{ cd backend && cargo test }", "{ cd backend && cargo test; cd ..; }", "mkdir -m 755 out && cd out && cd .. && git add -A", "cd backend && mkdir -p tmp && cd tmp && cd ../.. && git add -A"] {
            assert_eq!(in_storage_at(cmd, &clone), Vec::<PathBuf>::new(), "should run: {cmd}");
            std::fs::create_dir_all(clone.join("backend")).unwrap();
            assert_eq!(in_storage_at(cmd, &clone.join("backend")), Vec::<PathBuf>::new(), "should run in backend/: {cmd}");
        }
        // A cd that fails in the agent's own clone stays in it; from the workspace it may stay in the checkout around.
        assert_eq!(in_storage_at("cd nope; git checkout main", &clone), Vec::<PathBuf>::new());
        assert_eq!(asks("cd nope; git checkout main", &at(&ws)), Some(project.clone()));
        // In the agent's clone, or a folder of it: its own repository.
        assert_eq!(in_storage_at("git checkout main", &clone), Vec::<PathBuf>::new());
        assert_eq!(in_storage_at("git reset --hard", &clone.join("backend")), Vec::<PathBuf>::new());
        // A work tree given from the workspace: git still finds the user's repository from the folder.
        assert_eq!(in_storage(&format!("git --work-tree={} checkout .", at(&clone))), vec![project.clone()]);
        std::fs::remove_dir_all(project.parent().unwrap()).unwrap();
    }

    /// #578: the folder or repository git really touches is hidden by a link, a brace, a glob or a `.git` file.
    #[test]
    fn links_braces_globs_and_git_files_that_hide_the_target_ask() {
        let root = std::env::temp_dir().join(format!("loft-guard-{}", uuid::Uuid::new_v4()));
        let project = root.join("loft");
        let ws = project.join("agent-local-storage/x");
        let clone = ws.join("loft");
        let other = root.join("other");
        for d in [project.join(".git"), clone.join(".git"), other.join(".git/worktrees/w"), other.join("src")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let root = std::fs::canonicalize(&root).unwrap();
        let (project, ws, clone, other) = (root.join("loft"), root.join("loft/agent-local-storage/x"), root.join("loft/agent-local-storage/x/loft"), root.join("other"));
        let at = |p: &Path| p.display().to_string();
        // As Loft runs the hook: its ceiling is the storage folder.
        let storage = [root.join("loft/agent-local-storage")];
        let asks_all = |cmd: &str, cwd: &str| {
            let input = json!({ "tool_name": "Bash", "cwd": cwd, "tool_input": { "command": cmd } }).to_string();
            outside_changes(&input, &storage).into_iter().map(|c| c.folder).collect::<Vec<_>>()
        };
        std::os::unix::fs::symlink(other.join("src"), ws.join("lnk")).unwrap();
        // `k`: a link to a folder of the clone; `sl`: a `.git` that is a link; `gc`: a `.git` file with a `commondir`.
        std::fs::create_dir_all(clone.join("a/b")).unwrap();
        std::os::unix::fs::symlink(clone.join("a/b"), ws.join("k")).unwrap();
        std::fs::create_dir_all(ws.join("sl")).unwrap();
        std::os::unix::fs::symlink(other.join(".git"), ws.join("sl/.git")).unwrap();
        std::fs::create_dir_all(ws.join("gc")).unwrap();
        std::fs::write(ws.join("gc/.git"), "gitdir: ../fake\n").unwrap();
        std::fs::create_dir_all(ws.join("fake")).unwrap();
        std::fs::write(ws.join("fake/commondir"), format!("{}\n", at(&other.join(".git")))).unwrap();
        // A `.git` file, absolute and relative, and one to a repository kept apart.
        for (name, text) in [("gf", format!("gitdir: {}\n", at(&other.join(".git/worktrees/w")))), ("gr", "gitdir: ../../../../other/.git\n".to_string())] {
            std::fs::create_dir_all(ws.join(name)).unwrap();
            std::fs::write(ws.join(name).join(".git"), text).unwrap();
        }
        let has = |cmd: &str, cwd: &Path, folder: &Path| {
            let all = asks_all(cmd, &at(cwd));
            assert!(all.contains(&folder.to_path_buf()), "should ask about {}: {cmd} (in {}) -> {all:?}", folder.display(), cwd.display());
        };
        // `..` after a link goes up from what the link points at.
        has("git -C lnk/.. reset --hard", &ws, &other);
        has("cd lnk/.. && git reset --hard", &ws, &other);
        // A link that arrives with the clone.
        has("git clone -q src t && git -C t/l reset --hard", &clone, &project);
        has("git clone -q src t && cd t/l && git reset --hard", &clone, &project);
        // Braces the shell expands, and ones it lists as a sequence.
        has("git -C {../..,reset} --hard", &clone, &project);
        has(&format!("git -C loft {{--git-dir={}/.git,checkout,other}}", at(&project)), &ws, &project);
        has("git {-C,../..} reset --hard", &clone, &project);
        has("git -C {1..2} reset --hard", &clone, &project);
        has("cd {../..,x} && git reset --hard", &clone, &project);
        // Globs in a git or cd word.
        has("git -C lo* reset --hard", &clone, &project);
        has("git -C ../../* checkout main", &clone, &project);
        has("cd ../../?? && git checkout main", &clone, &project);
        has("git --git-dir=../../../*/.git checkout main", &clone, &project);
        // A `.git` file's `gitdir:`.
        has("git -C gf reset --hard", &ws, &other);
        has("cd gr && git checkout main", &ws, &other);
        let unknown = PathBuf::from(UNKNOWN);
        // Quoted braces are plain characters: they don't hide the `-C` that comes after.
        for cmd in ["git -c 'x.y={b,c}' -C ../../.. reset --hard", "git -c x.y=\\{b,c\\} -C ../../.. reset --hard", "git -c \"alias.x={a,status}\" -C ../../.. reset --hard"] {
            has(cmd, &clone, &project);
        }
        // `..` after a link is folded as text by `cd` and `pushd`, and after the link by git: both are asked about.
        for cmd in ["cd k/../.. && git reset --hard", "cd k && cd ../.. && git reset --hard", "pushd k/../.. && git reset --hard", "cd k/.. && GIT_CEILING_DIRECTORIES= git reset --hard", "pnpm --dir k/../../.. install"] {
            has(cmd, &ws, &project);
        }
        has("cd lnk/.. && git reset --hard", &ws, &project);
        // A glob names the folder its literal part does, wherever the command runs.
        let elsewhere = std::env::temp_dir();
        for cmd in [format!("git -C {}/lof? reset --hard", at(&root)), format!("cd {}/lof? && git reset --hard", at(&root)), format!("git --git-dir={}/lof?/.git checkout other", at(&root))] {
            has(&cmd, &elsewhere, &root);
            has(&cmd, &elsewhere, &unknown);
        }
        // Holes in the expansion: many words, a quoted part, an empty word, too many to list.
        let many = "--no-pager,".repeat(70);
        for cmd in [format!("git {{{many}-C,../../..,reset,--hard}}"), "git {,-C} ../../.. reset --hard".to_string()] {
            has(&cmd, &clone, &project);
        }
        for cmd in ["git {-C,../../..,-c,\"x.y=b c\"} reset --hard".to_string(), format!("git {} reset --hard", "{a,b}".repeat(13))] {
            has(&cmd, &clone, &unknown);
        }
        // What the same command made before git runs: a link, a clone's link, a checkout's, an archive's, a `.git` file.
        for (cmd, cwd) in [
            ("(git clone -q src t); git -C t/l reset --hard", &clone),
            ("sh -c 'git clone -q src t'; git -C t/l reset --hard", &clone),
            ("git clone -q --orig o src t && git -C t/l reset --hard", &clone),
            ("git clone -q --dep 1 src t && git -C t/l reset --hard", &clone),
            ("git checkout -q withlink && git -C l reset --hard", &clone),
            ("git pull -q && git -C l reset --hard", &clone),
            ("git worktree add -q wt b && git -C wt/l reset --hard", &clone),
            ("tar xf x.tar && git -C t/l reset --hard", &clone),
            ("ln -s ../../../../other t && git -C t reset --hard", &ws),
            ("ln -s ../../../../other t; cd t && git reset --hard", &ws),
            ("ln -s ../../../../other t && cd t && pnpm install", &ws),
            ("mv a t && cd t && cargo build", &ws),
            ("cp -s ../../../../other t; cd t; xcodebuild build", &ws),
            // `ln <target>` makes `./<name of target>`.
            ("ln -s ../../../../other && cd other && pnpm install", &ws),
            ("ln -s ../../../../other/src && cd src && cargo build", &ws),
            // A folder option into the link, not only the shell's folder.
            ("ln -s ../../../../other t && pnpm -C t install", &ws),
            ("ln -s ../../../../other t && pnpm --dir=t install", &ws),
            ("ln -s ../../../../other t && npm install --prefix t", &ws),
            ("ln -s ../../../../other t && yarn --cwd t install", &ws),
            ("ln -s ../../../../other t && bun install --cwd t", &ws),
            ("ln -s ../../../../other t && cargo build --manifest-path t/Cargo.toml", &ws),
            ("ln -s ../../../../other/src t && cargo build --manifest-path t/../Cargo.toml", &ws),
            ("ln -s ../../../../other t && xcodebuild -project t/X.xcodeproj build", &ws),
            // A physical `cd` out of the link lands beside what it points at.
            ("ln -s ../../../../other/src t && cd -P t/.. && pnpm install", &ws),
            ("ln -s ../../../../other/src t && cd t && cd -P .. && git reset --hard", &ws),
            ("echo 'gitdir: x' > a/.git; git -C a checkout other", &clone),
            ("mkdir t && echo 'gitdir: x' > t/.git && git -C t checkout other", &clone),
            ("mkdir t && cp g t/.git && git -C t checkout other", &clone),
        ] {
            has(cmd, cwd, &unknown);
        }
        // The folder a tool names itself, with no link: `env -C`, `xcodebuild -project`.
        let o = at(&other);
        for cmd in [format!("env -C {o} pnpm install"), format!("xcodebuild -project {o}/X.xcodeproj build"), format!("xcodebuild -workspace {o}/X.xcworkspace -scheme X build")] {
            assert_eq!(asks_all(&cmd, &at(&ws)), vec![other.clone()], "should ask: {cmd}");
        }
        // After `cd -P` through a link, or `..` before one, later folders stay unknown; so do env's bundled, repeated and nested -C.
        for cmd in [
            "ln -s ../../../../other/src t && cd -P t && cd .. && pnpm install".to_string(),
            "ln -s ../../../../other/src t && cd -P t && cd .. && git reset --hard".to_string(),
            "ln -s ../../../../other/src t && cd -P real/../t/.. && pnpm install".to_string(),
            "ln -s ../../../../other/src t && cargo build --manifest-path real/../t/../Cargo.toml".to_string(),
        ] {
            assert!(asks_all(&cmd, &at(&ws)).iter().any(|f| f.as_os_str() == UNKNOWN), "should ask: {cmd}");
        }
        for cmd in [format!("env -iC {o} pnpm install"), format!("env -C {o}/src env -C .. pnpm install"), format!("env -C /tmp -C {o} pnpm install"), format!("env -C /tmp env -C {o} pnpm install"), format!("env -i -uX -C{o} pnpm install"), format!("env --chdir {o} pnpm install")] {
            assert_eq!(asks_all(&cmd, &at(&ws)), vec![other.clone()], "should ask: {cmd}");
        }
        // A relative `-C` of a nested env goes from the outer one.
        assert_eq!(asks_all(&format!("env -C {o} env -C src pnpm install"), &at(&ws)), vec![other.join("src")]);
        // `cd` and `pushd` options are not the folder: `--`, zsh's `-q`, bundles.
        for cmd in ["cd -- {o} && git reset --hard", "cd -- {o} && pnpm install", "cd -q {o} && pnpm install", "cd -LP {o} && pnpm install", "cd -P -- {o} && pnpm install", "cd -Pe {o} && git reset --hard", "pushd -- {o} && pnpm install", "pushd -q {o} && git reset --hard", "pushd -LP {o} && cargo build"] {
            let cmd = cmd.replace("{o}", &o);
            assert!(asks_all(&cmd, &at(&ws)).contains(&other), "should ask: {cmd}");
        }
        // `..` after a fresh link for git -C and env -C, and text run from env -C's folder.
        for cmd in ["ln -s ../../../../other/src t && git -C t/.. reset --hard".to_string(), "ln -s ../../../../other/src t && env -C real/../t/.. pnpm install".to_string()] {
            assert!(asks_all(&cmd, &at(&ws)).iter().any(|f| f.as_os_str() == UNKNOWN), "should ask: {cmd}");
        }
        for cmd in [format!("env -C {o} sh -c 'pnpm install'"), format!("env -C {o} -S 'pnpm install'"), format!("env -C {o} -iS 'git reset --hard'")] {
            assert!(asks_all(&cmd, &at(&ws)).contains(&other), "should ask: {cmd}");
        }
        for cmd in ["cd -- loft && pnpm install", "cd -P loft && pnpm install", "cd -q -- loft && cargo build", "cd - && pnpm install", "cd -- -2 && pnpm install", "pushd -- loft && git reset --hard"] {
            assert_eq!(asks_all(cmd, &at(&ws)), Vec::<PathBuf>::new(), "should run: {cmd}");
        }
        // `popd` with `-q` is a plain one; another entry, or `-n`, leaves the folder unknown; a lost folder stays lost.
        for cmd in [
            "pushd {o} && pushd ~ && popd -q && git reset --hard", "pushd -q {o} && pushd -q .. && popd -q && cargo build", "pushd {o} && pushd ~ && popd +0 && pnpm install",
            "pushd {o} && pushd ~ && popd +1 && pnpm install", "pushd {o} && pushd ~ && popd -0 && pnpm install", "pushd {o} && pushd ~ && popd -n && pnpm install",
            "cd {o} && cd +0 && pushd . && popd && pnpm install", "cd {o} && cd +0 && pushd sub && popd && git reset --hard", "cd {o}; cd +0; pushd .; popd; pnpm install", "pushd {o} && pushd .. && popd +0 && pnpm install",
            // A work tree or repository named through a fresh link.
            "ln -s ../../../../other t && git --git-dir=t/.git --work-tree=t reset --hard", "ln -s ../../../../other t && git --git-dir t/.git reset --hard", "ln -s ../../../../other t && GIT_DIR=t/.git git reset --hard",
        ] {
            let cmd = cmd.replace("{o}", &o);
            assert!(!asks_all(&cmd, &at(&ws)).is_empty(), "should ask: {cmd}");
        }
        for cmd in ["pushd loft && pushd .. && popd && popd && pnpm install", "pushd loft && popd -q && pnpm install", "git --git-dir=loft/.git --work-tree=loft status"] {
            assert_eq!(asks_all(cmd, &at(&ws)), Vec::<PathBuf>::new(), "should run: {cmd}");
        }
        // A stack entry is a folder the guard can't tell; a folder after `--` named like one is a folder. -P in any order.
        for cmd in [
            "cd {o} && cd +0 && pnpm install", "cd {o}; cd -0; git reset --hard", "pushd {o} && pushd ~ && cd +1 && pnpm install", "pushd {o} && pushd ~ && cd -q +1 && git reset --hard",
            "pushd {o} && pushd ~ && pushd +1 && pnpm install", "cd -2 && pnpm install", "pushd +1 && pnpm install",
            "ln -s ../../../../other/src t && cd -P -L t/.. && pnpm install", "ln -s ../../../../other/src t && cd -PL t/.. && git reset --hard", "ln -s ../../../../other/src t && pushd -P -L t/.. && pnpm install",
            // A `..` from inside a fresh link.
            "ln -s ../../../../other/src t && cd t && git -C .. reset --hard", "ln -s ../../../../other/src t && git -C t -C .. reset --hard", "ln -s ../../../../other/src t && env -C t env -C .. pnpm install",
            "ln -s ../../../../other/src t && env -C t sh -c 'cd .. && pnpm install'", "ln -s ../../../../other/src t && git --git-dir t/../.git --work-tree t/.. reset --hard", "ln -s ../../../../other/src t && git --git-dir=t/../.git reset --hard",
            // Options inside env's -S string, and an attached one.
            "env -S '-C {o} pnpm install'", "env -iS '-C {o} pnpm install'", "env -iS'pnpm -C {o} install'", "env -S '-i -C {o} pnpm install'",
        ] {
            let cmd = cmd.replace("{o}", &o);
            assert!(!asks_all(&cmd, &at(&ws)).is_empty(), "should ask: {cmd}");
        }
        // The folder env goes to is applied once, however the text reaches it.
        for cmd in ["watch env -C src pnpm install", "parallel env -C src pnpm install ::: 1", "env -S 'env -C src pnpm install'"] {
            assert_eq!(asks_all(cmd, &at(&other)), vec![other.join("src")], "should ask about src only: {cmd}");
        }
        // The agent's own clone, reached by `cd`, is no link: it runs.
        for cmd in ["cd loft && pnpm install", "cd loft && cargo build", "pnpm -C loft install"] {
            assert_eq!(asks_all(cmd, &at(&ws)), Vec::<PathBuf>::new(), "should run: {cmd}");
        }
        // A workspace outside every checkout (the storage moved out of the project): nothing around it to name, yet a link
        // made in the same command still asks.
        let apart = root.join("apart/agent-local-storage/x");
        std::fs::create_dir_all(&apart).unwrap();
        for cmd in [format!("ln -s {} t && git -C t reset --hard", at(&other)), format!("ln -s {} t; cd t && git reset --hard", at(&other))] {
            has(&cmd, &apart, &unknown);
        }
        // GIT_DIR or GIT_WORK_TREE anywhere in the command: every git call after it is unknown (an inline one is also
        // resolved), whatever way it is set, hidden or undone.
        let o = at(&other);
        for cmd in [
            format!("export GIT_DIR={o}/.git; git reset --hard"),
            format!("GIT_DIR={o}/.git; git reset --hard"),
            format!("declare -x GIT_DIR={o}/.git && git reset --hard"),
            format!("export GIT_WORK_TREE={o}; git checkout ."),
            "export GIT_DIR=../../../other/.git; git reset --hard".to_string(),
            format!("(export GIT_DIR={o}/.git); git reset --hard"),
            format!("f() {{ export GIT_DIR={o}/.git; }}; f; git reset --hard"),
            format!("if true; then GIT_DIR={o}/.git; fi; git reset --hard"),
            format!("export GIT_D\"I\"R={o}/.git; git reset --hard"),
            format!("sh -c 'export GIT_DIR={o}/.git'; git reset --hard"),
            format!("eval \"export GIT_DIR={o}/.git\"; git reset --hard"),
            "unset -m 'GIT_D*'; git reset --hard".to_string(),
            "unset ${!GIT_*}; git reset --hard".to_string(),
            format!("env GIT_DIR={o}/.git git reset --hard"),
            format!("export GIT_DIR={o}/.git; unset GIT_DIR; git reset --hard"),
            format!("read GIT_DIR <<< {o}/.git; git reset --hard"),
            "sh -c 'unset GIT_DIR'; git reset --hard".to_string(),
            "bash -c 'f=GIT_WORK_TREE; echo $f'; git reset --hard".to_string(),
            format!("bash <<EOF\nexport GIT_DIR={o}/.git\nEOF\ngit reset --hard"),
            format!("echo 'export GIT_DIR={o}/.git' | sh; git reset --hard"),
            "export GIT_DIR; git reset --hard".to_string(),
            "typeset -x GIT_WORK_TREE; git reset --hard".to_string(),
        ] {
            has(&cmd, &apart, &unknown);
            has(&cmd, &clone, &unknown);
        }
        has(&format!("GIT_DIR={o}/.git git reset --hard"), &apart, &other);
        has(&format!("ln -s {o}/.git .git && git reset --hard"), &apart, &unknown);
        // A relative one is read from the folder `-C` moves to.
        for cmd in ["git --git-dir=../fake/.git -C loft reset --hard", "git --git-dir ../fake/.git -C loft reset --hard", "GIT_DIR=../fake git -C loft reset --hard", "git --work-tree=.. -C loft checkout .", "GIT_WORK_TREE=. git -C loft checkout ."] {
            has(cmd, &ws, &unknown);
        }
        // Naming a path that ends in `.git`: a redirect, `of=`, `--output=`, a tool's argument.
        for cmd in ["printf x > t/.git; git reset --hard", "dd if=x of=t/.git; git reset --hard", "curl --output=t/.git u; git reset --hard", "tee .git < x; git reset --hard", "echo hi | tee t/.git && git add .", "cat x >> .git; git reset --hard"] {
            has(cmd, &clone, &unknown);
        }
        // A `.git` named by the last command inside `sh -c`, `bash -c` or `eval` still counts after it.
        for cmd in [format!("sh -c 'ln -s {o}/.git .git'; git reset --hard"), format!("eval 'ln -s {o}/.git .git'; git reset --hard"), "bash -c \"cp ../pf .git\"; git reset --hard".to_string(), "(cp ../pf .git); git reset --hard".to_string()] {
            has(&cmd, &clone, &unknown);
        }
        // ln, mv, cp and their GNU names with braces or a glob may name any folder.
        for cmd in ["ln -s {a,b} t; git reset --hard", "mv lo* old; git -C loft reset --hard", "gmv loft old && gln -s x loft; git -C loft reset --hard", "gln -s {a,b} t; git reset --hard", "gcp -s x . && git add .", "cp x? y; git reset --hard"] {
            has(cmd, &clone, &unknown);
        }
        assert_eq!(asks_all(&format!("export GIT_DIR={o}/.git; git status"), &at(&apart)), Vec::<PathBuf>::new());
        // What can happen to a new repository's folder, or to git's environment, between the clone or init and the git call.
        let dest = format!("ln -s {o} t && git clone -q /src t; git -C t reset --hard");
        for cmd in [
            dest,
            "git clone -q /src d && cd d && git reset --hard".to_string(),
            format!("export GIT_DIR={o}/.git; git init m; git -C m reset --hard"),
            format!("GIT_DIR={o}/.git git init m && git -C m reset --hard"),
            format!("git init m && export GIT_DIR={o}/.git && git -C m reset --hard"),
            format!("git clone -q /src m && GIT_WORK_TREE={o} git -C m checkout -f ."),
            "git init m && HOME=/x git -C m reset --hard".to_string(),
            format!("git clone -q /src m && git -C m -c core.worktree={o} checkout -f ."),
            "git init m && rm -rf m/.git && GIT_CEILING_DIRECTORIES= git -C m reset --hard".to_string(),
            format!("git init m && rm -rf m && ln -s {o} m && git -C m reset --hard"),
            format!("git init m && echo 'gitdir: {o}/.git' > m/.git && git -C m reset --hard"),
        ] {
            for cwd in [&ws, &clone] {
                let all = asks_all(&cmd, &at(cwd));
                assert!(all.contains(&unknown) || all.contains(&other), "should ask: {cmd} (in {}) -> {all:?}", cwd.display());
            }
        }
        // `-c core.worktree` on a folder that is there names the work tree.
        has(&format!("git -C k -c core.worktree={o} checkout -f ."), &ws, &other);
        has("git -c core.WorkTree=../../../other checkout -f .", &ws, &other);
        has("git --config-env=core.worktree=WT checkout -f .", &ws, &unknown);
        // A `.git` that is a link to another `.git`, and one whose `commondir` names another repository.
        has("git -C sl checkout other", &ws, &other);
        has("git -C gc branch -D main", &ws, &other);
        // An existing folder swapped for a link, or removed, moved or copied over, before git runs.
        for cmd in [
            format!("mv loft loft.old && ln -s {o} loft && git -C loft reset --hard"),
            format!("rm -rf loft && ln -s {o} loft; git -C loft reset --hard"),
            format!("mv loft x && ln -s {o} loft && git -C loft/backend reset --hard"),
            format!("cd loft && mv ../loft ../old && ln -s {o} ../loft && git reset --hard"),
            format!("cp -r {o} loft && git -C loft checkout main"),
            "mv \"$a\" \"$b\"; git -C loft reset --hard".to_string(),
            "rm -f \"$LOCK\" && git -C . add .".to_string(),
            "rm -f \"$LOCK\"; cd loft && git add .".to_string(),
            "rm -f \"$LOCK\"; git --work-tree=. add .".to_string(),
        ] {
            has(&cmd, &ws, &unknown);
        }
        // Copying into the folder git runs in asks only when it could bring a link or a `.git`.
        for cmd in [format!("cp -rs {o}/.git . && git reset --hard"), format!("cp -r {o}/.git . && git reset --hard"), format!("cp -r {o}/.git ../loft && git reset --hard"), "ln -s x . && git add .".to_string(), "rsync -a src/ . && git add .".to_string(), "cp --symbolic-link x . && git add .".to_string(), "cp -l x . && git add .".to_string()] {
            has(&cmd, &clone, &unknown);
        }
        // What an agent does in its own clone still asks nothing.
        for cmd in [
            "git commit -m 'fix GIT_DIR handling'",
            "echo $GIT_DIR && git add .",
            "GIT_SSH_COMMAND=\"ssh -i $KEY\" git push",
            "rm -rf lo* && git add .",
            "rm -f build/*.o; git commit -m x",
            "cp ../x.txt . && git add .",
            "cp -r ../x . && git add .",
            "mv ../x.txt . && git add .",
            "install -m 644 ../x.txt . && git add .",
            "cp -t . ../x.txt && git add .",
            "rm -f \"$LOCK\" && git add .",
            "rm -f \"$LOCK\"; git commit -m x",
            "rm -f .git/index.lock && git add .",
            "mv notes.txt old.txt && git add .",
            "cp ../x.txt notes.txt && git -C . commit -m x",
            "mkdir -p t && git -C t init -q",
            "echo hi > notes.txt && git add notes.txt",
            "cd -P . && git commit -m x",
            "cd k/.. && git status",
            "git commit -m 'fix {a,b} and *'",
            "git commit -m \"use {x} here\"",
            "git log --format='%h {%s}' -- '*.rs'",
            "git checkout -b feat/{x}",
            "git -C . reset --hard",
            "git -C ${HOME}/x status",
        ] {
            assert_eq!(asks_all(cmd, &at(&clone)), Vec::<PathBuf>::new(), "should run: {cmd}");
        }
        // A clone is run in a command of its own: git on the new folder in the same command asks, whatever it names.
        let user = at(&project);
        for cmd in [
            "git clone -q src t && git -C t reset --hard".to_string(),
            "git clone -q src t && cd t && git checkout -b x".to_string(),
            "git clone -q --depth 1 src t && git -C t reset --hard".to_string(),
            "git -C loft clone x ../y && git -C y add *.md".to_string(),
            format!("git clone x ny && git -C ny --work-tree={user} reset --hard"),
            format!("git clone x ny && git -C ny --git-dir={user}/.git checkout other"),
            format!("git clone x ny && GIT_DIR={user}/.git git -C ny checkout other"),
            format!("git clone x ny && GIT_WORK_TREE={user} git -C ny checkout ."),
        ] {
            has(&cmd, &ws, &unknown);
            has(&cmd, &clone, &unknown);
        }
        // Next to it, in its own command, the clone is known.
        assert_eq!(asks_all("git -C loft pull", &at(&ws)), Vec::<PathBuf>::new());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_stops_at_the_storage_folder_unless_the_command_moves_the_ceiling_or_the_repository() {
        let project = std::env::temp_dir().join(format!("loft-guard-{}", uuid::Uuid::new_v4())).join("loft");
        let storage = project.join("agent-local-storage");
        let ws = storage.join("x");
        for d in [project.join(".git"), ws.join("notes"), ws.join("loft/.git")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let project = std::fs::canonicalize(&project).unwrap();
        let at = |p: &Path| p.display().to_string();
        let asks_under = |cmd: &str, cwd: &Path, ceiling: &[PathBuf]| {
            let input = json!({ "tool_name": "Bash", "cwd": at(cwd), "tool_input": { "command": cmd } }).to_string();
            outside_changes(&input, ceiling).into_iter().map(|c| c.folder).collect::<Vec<_>>()
        };
        let ceiling = [storage.clone()];
        // Git fails there, as Loft's ceiling stops it: nothing to ask the user.
        for cwd in [&ws, &ws.join("notes")] {
            for cmd in ["git checkout main", "git reset --hard origin/main", "git stash", "git -C notes checkout main"] {
                assert_eq!(asks_under(cmd, cwd, &ceiling), Vec::<PathBuf>::new(), "should run: {cmd} (in {})", cwd.display());
            }
        }
        for cmd in ["git checkout main && git stash", "git status; git reset --hard origin/main", "git -C notes checkout main && git -C notes stash", "git commit -m 'a message'", "git --no-pager -C notes checkout main"] {
            assert_eq!(asks_under(cmd, &ws, &ceiling), Vec::<PathBuf>::new(), "should run: {cmd}");
        }
        assert_eq!(asks_under("git checkout main", &ws.join("loft"), &ceiling), Vec::<PathBuf>::new());
        // The ceiling unset or changed, or a repository named: git can reach the user's checkout, so the user is asked.
        for cmd in [
            "GIT_CEILING_DIRECTORIES= git checkout main",
            // Wrappers and other shells that drop the ceiling before git runs.
            "env -i bash -c 'git checkout main'",
            "sudo sh -c 'git checkout main'",
            "env -u GIT_CEILING_DIRECTORIES sh -c 'git stash'",
            "exec -c git checkout main",
            "env -i watch 'git checkout main'",
            "env -i eval 'git checkout main'",
            "env -i script -c 'git checkout main'",
            "bash <<EOF\nunset GIT_CEILING_DIRECTORIES\ngit checkout main\nEOF",
            "echo 'unset GIT_CEILING_DIRECTORIES; git checkout main' | sh",
            // Names hidden by quoting, globs or expansion.
            "unset GIT_CEILING_DIRECT\"\"ORIES; git checkout main",
            "un''set GIT_CEILING_DIRECTORIES; git checkout main",
            "unset -m 'GIT_CEIL*'; git checkout main",
            "unset ${!GIT_CEIL*}; git checkout main",
            "v=GIT_CEILING; unset ${v}_DIRECTORIES; git checkout main",
            "export GIT_D\"\"IR=../../.git; git checkout main",
            "G\"\"IT_DIR=../../.git git checkout main",
            "read GIT_CEILING_DIRECT''ORIES <<< ''; git checkout main",
            "for GIT_CEILING_DIRECTORIES in ''; do git checkout main; done",
            "let GIT_CEILING_DIRECTORIES=0; git checkout main",
            "x=GIT_CEILING_DIRECTORIES; (( x = 0 )); git checkout main",
            "sh -c 'un\"\"set GIT_CEILING_DIRECTORIES; git checkout main'",
            "trap 'unset GIT_CEILING_DIRECTORIES' DEBUG; git checkout main",
            "alias git='env -i git'; git checkout main",
            "source ./unceil.sh; git checkout main",
            "unset GIT_CEILING_DIRECTORIES; git checkout main",
            "export GIT_CEILING_DIRECTORIES=/tmp && git stash",
            "env -u GIT_CEILING_DIRECTORIES git checkout main",
            "env -i git checkout main",
            "sudo git checkout main",
            "export GIT_DIR=../../.git; git checkout main",
            "git --git-dir ../../.git checkout main",
            "git -C ../.. checkout main",
            "cd ../.. && git checkout main",
            // Words joined into shell text, where the inner quotes go only when that shell runs it.
            "echo 'un\"\"set' 'G\"\"IT_CEILING_DIRECTORIES' ';' git checkout main | sh",
            "echo 'e\"\"nv' -i git checkout main | sh",
            "watch -n 99 'un\"\"set' 'G\"\"IT_CEILING_DIRECTORIES' ';' git checkout main",
            "parallel 'un\"\"set' 'G\"\"IT_CEILING_DIRECTORIES' ';' git checkout main ::: 1",
            // Config or an alias that runs a shell, outside any repository.
            "git -c alias.x='!git -C ../.. checkout main' x",
            "git -c alias.x='!cd ../.. && git checkout main' x",
            "git --config-env=alias.x=X x",
            "git --exec-path=/tmp/bin checkout main",
            "git x",
            // An option before the subcommand that takes a value: git runs the alias `x`, not `checkout`.
            "git --namespace checkout x",
            "git --attr-source checkout x",
            "git --bare checkout main",
            "git --unknown-option checkout main",
            // A startup file the shell reads before the command.
            "BASH_ENV=f bash -c 'git checkout main'",
            "ZDOTDIR=. zsh -c 'git checkout main'",
            "bash --rcfile f -c 'git checkout main'",
            "bash --init-file f -i -c 'git checkout main'",
            "HOME=. git checkout main",
            "PATH=. git checkout main",
            // A -C out of the storage folder, and anything run in the background or another shell.
            "git -C /tmp checkout main; git checkout main",
            "git checkout main & git checkout main",
            "git checkout main\ngit checkout main",
            "xargs git checkout <<< main",
            "(git checkout main)",
        ] {
            assert!(asks_under(cmd, &ws, &ceiling).contains(&project), "should ask: {cmd}");
        }
        // Git ignores a relative ceiling, and so does the guard.
        assert_eq!(asks_under("git checkout main", &ws, &[PathBuf::from("agent-local-storage")]), vec![project.clone()]);
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
        let clone = st.cfg.storage_dir.join("x/loft");
        std::fs::create_dir_all(&clone).unwrap();
        assert_eq!(guard(std::fs::canonicalize(&clone).unwrap()).await.unwrap()["allow"], true);
        assert!(pending().is_empty());

        // Denied: the agent is told to use its own clone.
        let asking = guard(user.clone());
        crate::agents::tests::until("a permission request", 5, || pending().len() == 1).await;
        let req = pending().remove(0);
        assert_eq!(req.tool_name, "Checkout");
        assert_eq!(req.input["folder"], user.display().to_string());
        assert!(req.reason.contains("git checkout --detach 3ddf495c"), "{}", req.reason);
        crate::agents::decide(&st, &req.id, "deny", None, None, None).unwrap();
        let v = asking.await.unwrap();
        assert_eq!(v["allow"], false);
        assert!(v["message"].as_str().unwrap().contains("Work in your own clone in your workspace"), "{v}");

        // No rule to save: Always allow is refused, even with a rule posted, and the agent's tools stay as they were.
        let asking = guard(user.clone());
        crate::agents::tests::until("a second permission request", 5, || pending().len() == 1).await;
        let id = pending()[0].id.clone();
        assert_eq!(pending()[0].rule, "");
        let tools = st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools;
        assert!(crate::agents::decide(&st, &id, "allow_always", None, None, None).is_err());
        assert!(crate::agents::decide(&st, &id, "allow_always", Some(format!("Checkout({})", user.display())), None, None).is_err());
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools, tools);

        // Allowed once: this command runs, and the next change there asks again.
        crate::agents::decide(&st, &id, "allow_once", None, None, None).unwrap();
        assert_eq!(asking.await.unwrap()["allow"], true);
        let again = guard(user.join("frontend"));
        crate::agents::tests::until("the next change asking again", 5, || pending().len() == 1).await;
        crate::agents::decide(&st, &pending()[0].id, "deny", None, None, None).unwrap();
        assert_eq!(again.await.unwrap()["allow"], false);

        // A folder the hook couldn't name: the user is asked about the projects folder as a whole.
        let asking = guard(PathBuf::from(UNKNOWN));
        crate::agents::tests::until("a request for the unknown folder", 5, || pending().len() == 1).await;
        let req = pending().remove(0);
        assert_eq!(req.input["folder"], projects.display().to_string());
        assert!(req.reason.contains("can't tell which"), "{}", req.reason);
        crate::agents::decide(&st, &req.id, "deny", None, None, None).unwrap();
        assert_eq!(asking.await.unwrap()["allow"], false);
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
        crate::agents::decide(&st, &pending()[0].id, "allow", None, None, None).unwrap();
        crate::agents::tests::until("the second question", 5, || pending().first().is_some_and(|p| p.input["folder"] == two.display().to_string())).await;
        crate::agents::decide(&st, &pending()[0].id, "deny", None, None, None).unwrap();
        let v = asking.await.unwrap().0;
        assert_eq!(v["allow"], false);
        assert!(v["message"].as_str().unwrap().contains(&two.display().to_string()), "{v}");
    }
}
