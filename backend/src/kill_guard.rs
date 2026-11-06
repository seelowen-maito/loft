//! The `PreToolUse` hook on Bash that Loft gives every agent: it refuses commands that kill processes by name
//! or pattern (`pkill -f vite`, `killall node`, `pgrep -f next | xargs kill`). Those hit every process on the Mac
//! whose command line matches: other agents' claude and their running Bash commands (Claude Code runs each one
//! as `zsh -c '<command>'`), and the user's own stdio MCP servers. Kills by PID or port stay allowed.
//!
//! Claude runs it before every Bash call, so it is a plain scan of the command's shell text, in this binary
//! (`loft kill-guard`) and before any runtime starts. It refuses:
//! - `pkill` and `killall`, and a `kill` that hits every process the user owns (`kill -9 -1`) or a whole process
//!   group (`kill 0`, `kill -- -1234`);
//! - a `kill` fed by a lookup (`pgrep`, `pidof`, `ps` other than `ps -p`, `lsof -c`): later in the same pipeline
//!   (`pgrep x | xargs kill`), inside the kill's `$(...)`, or through a variable or file the command fills from the
//!   lookup (`PID=$(pgrep x); kill $PID`). A lookup that only checks (`kill 123; ps aux | grep vite`) runs.
//!
//! It looks through wrappers (`sudo`, `xargs`, `find -exec`, `watch`, `env -S`, ...), `sh -c`, `eval`, and shells
//! that read their script from a heredoc or a piped `echo`. Its limits, which the prompt rule covers: it can't see
//! into a script file the agent writes and runs, a command name built from a variable (`p=pkill; $p`), or another
//! language sending a signal (osascript, python, node). It fails open: input it can't read lets the command run,
//! since failing closed would block every Bash call.

use std::collections::HashSet;
use std::io::Read;

use serde_json::Value;

/// The argument that makes this binary run as the hook.
pub const ARG: &str = "kill-guard";

const REFUSAL: &str = "Loft refused this command: it kills processes by name or pattern (pkill, killall, or kill \
fed by pgrep, pidof or ps), or every process (kill -1, kill 0). That also hits other agents' running commands and \
servers and the user's MCP servers on this Mac. Stop only your own processes: by port, `lsof -ti tcp:<port> \
-sTCP:LISTEN | xargs kill`, or by the PID you saved when you started it (`kill <pid>`, `kill $!`). If you must look \
a PID up, do it in one command, check the process is yours (started from your workspace), then kill it by number \
in another.";

/// Runs as the hook: reads Claude Code's hook input on stdin and exits 2, which blocks the call and shows
/// stderr to the agent, for a kill by name. Input it can't read passes.
pub fn hook() -> ! {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if let Err(why) = check(&input) {
        eprintln!("{why}");
        std::process::exit(2);
    }
    std::process::exit(0)
}

/// The `hooks` of a settings file that installs the hook, run as `exe kill-guard`.
pub fn hooks(exe: &std::path::Path) -> Value {
    let command = format!("'{}' {ARG}", exe.to_string_lossy().replace('\'', r"'\''"));
    serde_json::json!({ "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command }] }] })
}

fn check(input: &str) -> Result<(), &'static str> {
    let v: Value = serde_json::from_str(input).unwrap_or_default();
    match v["tool_input"]["command"].as_str() {
        Some(command) if kills_by_name(command) => Err(REFUSAL),
        _ => Ok(()),
    }
}

/// Whether `command` kills processes by name or pattern, or every process (see the module docs).
pub fn kills_by_name(command: &str) -> bool {
    scan(command, 0).refuse
}

/// What a piece of shell text, or one simple command in it, does that the guard cares about.
#[derive(Default, Clone, Copy)]
struct Scan {
    /// Refused on its own: `pkill`, `killall`, a kill of everything, or (for text) a kill fed by a lookup.
    refuse: bool,
    /// Runs `kill`.
    kill: bool,
    /// Looks processes up by name.
    lookup: bool,
}

impl Scan {
    fn add(&mut self, s: Scan) {
        self.refuse |= s.refuse;
        self.kill |= s.kill;
        self.lookup |= s.lookup;
    }
}

/// Shell text nested deeper than this (`sh -c "eval '...'"`) isn't scanned.
const MAX_DEPTH: usize = 8;

fn scan(text: &str, depth: usize) -> Scan {
    if depth > MAX_DEPTH {
        return Scan::default();
    }
    let cmds = parse(text);
    let each: Vec<Scan> = (0..cmds.len()).map(|i| scan_command(&cmds, i, depth)).collect();
    let mut s = Scan::default();
    each.iter().for_each(|e| s.add(*e));
    if !s.refuse && s.kill && s.lookup {
        s.refuse = fed(&cmds, &each).into_iter().zip(&each).any(|(fed, e)| fed && e.kill);
    }
    s
}

/// What one simple command runs, through its wrappers and any shell text it hands on.
fn scan_command(cmds: &[Cmd], i: usize, depth: usize) -> Scan {
    let mut s = Scan::default();
    let mut runs = vec![cmds[i].words.clone()];
    while let Some(words) = runs.pop() {
        let words = match unwrap(&words) {
            Run::Words(words) => words,
            Run::Text(text) => {
                s.add(scan(&text, depth + 1));
                continue;
            }
        };
        let Some((name, args)) = words.split_first() else { continue };
        let has = |flag: &str| args.iter().any(|a| a.starts_with(flag));
        match base(name) {
            "pkill" | "killall" => s.refuse = true,
            "kill" => {
                s.kill = true;
                s.refuse |= hits_everything(args);
            }
            "pgrep" | "pidof" => s.lookup = true,
            "ps" if !has("-p") => s.lookup = true,
            "lsof" if args.iter().any(|a| lsof_by_name(a)) => s.lookup = true,
            "find" => runs.extend(find_execs(args)),
            "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                if let Some(text) = shell_script(cmds, i, args) {
                    s.add(scan(&text, depth + 1));
                }
            }
            "eval" => s.add(scan(&args.join(" "), depth + 1)),
            _ => {}
        }
    }
    s
}

/// Which commands get a lookup's output: from a lookup earlier in their pipeline or in their `$(...)`, or through a
/// variable or file filled from one.
fn fed(cmds: &[Cmd], each: &[Scan]) -> Vec<bool> {
    let mut fed = vec![false; cmds.len()];
    let (mut vars, mut files) = (HashSet::new(), HashSet::new());
    loop {
        let before = (fed.iter().filter(|f| **f).count(), vars.len(), files.len());
        let carries: Vec<bool> = fed.iter().zip(each).map(|(f, e)| *f || e.lookup).collect();
        for (i, c) in cmds.iter().enumerate() {
            // A `$(...)`/`<(...)` feeds its parent; a `>(...)` reads it.
            let from = |j: usize| {
                carries[j]
                    && ((cmds[j].parent == Some(i) && !cmds[j].reads_parent)
                        || (c.parent == Some(j) && c.reads_parent)
                        || piped(&cmds[j], c))
            };
            // What a compound command reads (`done < <(pgrep x)`) reaches every command inside it.
            let into = |j: usize| fed[j] && closes(&cmds[j]) && cmds[j].pipes.last().is_some_and(|at| c.pipes.len() > cmds[j].pipes.len() && c.pipes.contains(at));
            let now = (0..cmds.len()).any(|j| from(j) || into(j))
                || c.words.iter().any(|w| vars.iter().any(|v: &String| uses(w, v)))
                || c.words.iter().chain(&c.reads).any(|w| files.contains(w.trim_start_matches("./")));
            fed[i] |= now;
        }
        for c in cmds.iter().enumerate().filter(|(i, _)| fed[*i] || each[*i].lookup).map(|(_, c)| c) {
            vars.extend(assigns(c));
            files.extend(writes(c).map(|f| f.trim_start_matches("./").to_string()));
        }
        if before == (fed.iter().filter(|f| **f).count(), vars.len(), files.len()) {
            return fed;
        }
    }
}

/// Whether `c` ends a compound command (`done < file`).
fn closes(c: &Cmd) -> bool {
    c.words.first().is_some_and(|w| matches!(w.as_str(), "done" | "}" | "fi" | "esac"))
}

/// Whether `a` sits earlier than `b` in a pipeline, so `b` reads its output.
fn piped(a: &Cmd, b: &Cmd) -> bool {
    a.pipes.iter().any(|&(p, at)| b.pipes.iter().any(|&(q, bt)| p == q && at < bt))
}

/// Whether `word` expands the variable `var` (`$var`, `${var}`).
fn uses(word: &str, var: &str) -> bool {
    word.match_indices('$').any(|(i, _)| {
        let rest = &word[i + 1..];
        let rest = rest.strip_prefix('{').unwrap_or(rest);
        rest.strip_prefix(var).is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
    })
}

/// The variables a command sets: `NAME=...` (also after `export`/`local`), `for NAME in`, `read NAME`.
fn assigns(c: &Cmd) -> Vec<String> {
    let mut vars: Vec<String> = c.words.iter().filter_map(|w| assignment(w)).map(String::from).collect();
    if let Run::Words([name, args @ ..]) = unwrap(&c.words) {
        match name.as_str() {
            "for" | "select" => vars.extend(args.first().cloned()),
            "read" | "mapfile" | "readarray" => {
                vars.extend(args.iter().filter(|a| !a.starts_with('-')).cloned());
                vars.extend(["REPLY".to_string(), "MAPFILE".to_string()]);
            }
            _ => {}
        }
    }
    vars
}

/// The files a command writes: its `>` targets and what it `tee`s to.
fn writes(c: &Cmd) -> impl Iterator<Item = &String> {
    let tee = match unwrap(&c.words) {
        Run::Words([name, args @ ..]) if base(name) == "tee" => args,
        _ => &[],
    };
    c.writes.iter().chain(tee.iter().filter(|a| !a.starts_with('-'))).filter(|f| !f.starts_with("/dev/"))
}

/// Whether `kill`'s arguments signal every process the user owns (`-1`) or a whole process group (`0`, `-1234`).
/// A first `-N` is the signal: `kill -1 1234` sends HUP to 1234.
fn hits_everything(args: &[String]) -> bool {
    let mut operands = match args.first().map(String::as_str) {
        Some("-l" | "-L") => return false,
        Some("-s" | "-n") => args.get(2..).unwrap_or_default(),
        Some(a) if a.starts_with('-') => &args[1..],
        _ => args,
    };
    if operands.first().is_some_and(|a| a == "--") {
        operands = &operands[1..];
    }
    operands.iter().any(|a| {
        let n = a.strip_prefix('-').unwrap_or(a);
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (a.starts_with('-') || n.bytes().all(|b| b == b'0'))
    })
}

/// Whether an `lsof` option picks processes by command name: `-c`, also in a group (`-tc`, `-ct`).
fn lsof_by_name(arg: &str) -> bool {
    let Some(flags) = arg.strip_prefix('-').filter(|f| !f.starts_with('-')) else { return false };
    for f in flags.chars() {
        if f == 'c' {
            return true;
        }
        if "dDeFgikKopsSTuxz".contains(f) {
            return false; // the rest is this option's value (`-itcp:3000`)
        }
    }
    false
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
            return rest.first().cloned();
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
        if let Run::Words([name, args @ ..]) = unwrap(&up.words) {
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
/// command they're given (`sudo`, `env`, `xargs`, `nohup`, `watch`, ...). `command -v` runs nothing.
fn unwrap(words: &[String]) -> Run<'_> {
    let rest = |i: usize| words.get(i..).unwrap_or_default();
    let mut i = 0;
    while let Some(w) = words.get(i) {
        if assignment(w).is_some() || KEYWORDS.contains(&w.as_str()) {
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
    /// The command whose word holds this one in a `$(...)`, backticks, `<(...)` or `>(...)`.
    parent: Option<usize>,
    /// In a `>(...)`: it reads the parent's output, where a `$(...)` gives the parent its own.
    reads_parent: bool,
    /// Each pipeline it sits in, from the outermost (one a compound command it's in sits in) in: the pipeline and
    /// its place there.
    pipes: Vec<(usize, usize)>,
    /// Files redirected in (`<`) and out (`>`, `>>`).
    reads: Vec<String>,
    writes: Vec<String>,
    /// Its heredoc bodies and herestrings.
    stdin: String,
}

#[derive(Clone, Copy)]
enum Redirect {
    In,
    Out,
    /// `2>&1`: the target is a file descriptor.
    Fd,
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
}

/// Splits shell text into its simple commands, including the commands inside `$(...)`, backticks and `<(...)`.
fn parse(text: &str) -> Vec<Cmd> {
    let c: Vec<char> = text.chars().collect();
    let mut p = Parse::default();
    lex(&mut p, &c, 0, None, None, false);
    p.cmds
}

/// The state of one `lex`: the top level or one substitution.
struct Level {
    parent: Option<usize>,
    reads_parent: bool,
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
    fn new(parent: Option<usize>, reads_parent: bool) -> Self {
        Level { parent, reads_parent, cur: None, word: String::new(), in_word: false, redirect: None, frames: vec![Frame::default()], heredocs: vec![] }
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    /// The command being lexed, started if need be.
    fn cmd(&mut self, p: &mut Parse) -> usize {
        let (parent, reads_parent) = (self.parent, self.reads_parent);
        *self.cur.get_or_insert_with(|| {
            p.cmds.push(Cmd { parent, reads_parent, ..Default::default() });
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
            Some(Redirect::In) => cmd.reads.push(word),
            Some(Redirect::Out) => cmd.writes.push(word),
            Some(Redirect::Fd) => {}
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
fn lex(p: &mut Parse, c: &[char], mut i: usize, stop: Option<char>, parent: Option<usize>, reads_parent: bool) -> usize {
    let mut l = Level::new(parent, reads_parent);
    let mut depth = 0; // subshell parens open inside a `$(`
    let mut array = 0; // parens open in an array assignment, `pids=($(pgrep x))`: its elements stay in the word
    while let Some(&ch) = c.get(i) {
        let next = c.get(i + 1).copied();
        i += 1;
        let opens_array = ch == '(' && l.in_word && l.word.ends_with('=') && assignment(&l.word).is_some();
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
                            let id = l.cmd(p);
                            i = lex(p, c, i + 1, Some(')'), Some(id), false);
                        }
                        '`' => {
                            let id = l.cmd(p);
                            i = lex(p, c, i, Some('`'), Some(id), false);
                        }
                        _ => l.word.push(q),
                    }
                }
            }
            '`' if stop == Some('`') => break,
            '`' => {
                l.in_word = true;
                let id = l.cmd(p);
                i = lex(p, c, i, Some('`'), Some(id), false);
            }
            '$' | '<' | '>' if next == Some('(') => {
                l.in_word = true;
                let id = l.cmd(p);
                i = lex(p, c, i + 1, Some(')'), Some(id), ch == '>');
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
    l.redirect = Some(match ch {
        _ if op.ends_with('&') => Redirect::Fd,
        '<' if op == "<<" => Redirect::Here,
        '<' => Redirect::In,
        _ => Redirect::Out,
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    // These are only ever scanned as text: nothing here runs a command.

    #[test]
    fn refuses_kills_by_name_wherever_they_sit() {
        for cmd in [
            "pkill -f foo",
            "killall node",
            "cd x && pkill -f foo",
            "foo; killall node",
            "kill $(pgrep -f next)",
            "kill -9 `pgrep vite`",
            "pgrep -f vite | xargs kill",
            "pgrep -f vite | xargs -n 1 kill -9",
            "ps aux | grep vite | awk '{print $2}' | xargs kill",
            "PID=$(pgrep -f vite); kill $PID",
            "kill $(lsof -c node -t)",
            "sudo pkill node",
            "sudo -u me killall node",
            "/usr/bin/pkill x",
            "\\pkill x",
            "\"pkill\" x",
            "(cd /tmp; pkill x)",
            "echo $(pkill x)",
            "echo \"$(killall x)\"",
            "diff <(pkill x) y",
            "bash -c 'pkill -f x'",
            "zsh -lc \"killall node\"",
            "eval \"pkill x\"",
            "env FOO=1 pkill x",
            "FOO=1 killall x",
            "nohup killall x >/dev/null 2>&1 &",
            "timeout 5 pkill x",
            "xargs pkill < names",
            "if true; then killall node; fi",
            "{ pkill x; }",
            "true\npkill x",
            "npm test || pkill -f vite",
            "cat <<EOF\nhi\nEOF\npkill x",
            "git commit -m \"$(cat <<'EOF'\nDon't (really\nEOF\n)\" && pkill x",
            "f() { pkill x; }; f",
        ] {
            assert!(kills_by_name(cmd), "should refuse: {cmd}");
        }
    }

    #[test]
    fn refuses_kills_of_every_process_or_a_process_group() {
        for cmd in ["kill -9 -1", "kill -- -1", "kill -TERM 0", "kill 0", "kill -- 0", "kill -TERM -1234", "kill -s KILL -1", "kill -9 -- -1", "kill 1234 -1", "/bin/kill -1 -1", "sudo kill -9 -1", "bash -c 'kill -9 -1'"] {
            assert!(kills_by_name(cmd), "should refuse: {cmd}");
        }
        // A first `-1` is the signal (HUP).
        for cmd in ["kill -1 1234", "kill 1234", "kill $!", "kill -9 $PID", "kill -l", "kill -s HUP 1234", "lsof -ti tcp:3000 -sTCP:LISTEN | xargs kill"] {
            assert!(!kills_by_name(cmd), "should allow: {cmd}");
        }
    }

    #[test]
    fn a_lookup_counts_only_when_it_feeds_the_kill() {
        for cmd in [
            "pgrep x | while read p; do kill $p; done",
            "pgrep x | while IFS= read -r p; do kill -9 \"$p\"; done",
            "for p in $(pgrep x); do kill $p; done",
            "pids=$(pgrep x)\nsleep 1\nkill ${pids}",
            "A=$(pgrep x); B=$A; kill $B",
            "export PID=`pidof node` && kill -9 $PID",
            "pgrep -f vite > p; kill $(cat p)",
            "pgrep -f vite > /tmp/p; xargs kill < /tmp/p",
            "pgrep -f vite | tee p; kill $(cat ./p)",
            "kill $(pgrep x | head -1)",
            "kill $(cat $(pgrep -f x))",
            "{ pgrep x; } | xargs kill",
            "(pgrep x) | xargs kill",
            "pgrep x | xargs -I{} sh -c 'kill {}'",
            "PID=$(pgrep x); sh -c \"kill $PID\"",
            "sh -c 'pgrep x | xargs kill'",
            "pgrep x | parallel kill",
            "ps -eo pid,comm | awk '/vite/ {print $1}' | xargs kill",
            "while read p; do kill $p; done < <(pgrep x)",
            "pgrep x > p; while read q; do kill $q; done < p",
            "xargs kill < <(pgrep x)",
            "pgrep x > p; kill $(<p)",
            "pids=($(pgrep x)); kill ${pids[@]}",
            "f() { local pids=($(pgrep -f vite)); kill \"${pids[@]}\"; }",
            "declare -a pids=($(pgrep x)); kill ${pids[*]}",
            "pids=(`pgrep x`)\nkill $pids",
            "pids+=( $(pidof node) ); kill -9 ${pids[0]}",
            "pgrep x > >(xargs kill)",
            "pgrep x | tee >(xargs kill)",
        ] {
            assert!(kills_by_name(cmd), "should refuse: {cmd}");
        }
        for cmd in [
            "kill 123; sleep 1; ps aux | grep vite",
            "lsof -ti tcp:3000 -sTCP:LISTEN | xargs kill; pgrep -fl next",
            "kill $PID; sleep 1; ps -p $PID",
            "pgrep -q vite && kill 1234",
            "if pgrep -q vite; then kill $(cat server.pid); fi",
            "{ pgrep x; kill 12; }",
            "PID=$(cat server.pid); kill $PID; pgrep -fl vite",
            "pgrep -fl x > procs.txt; kill 12",
            "ps aux 2>&1 | grep x; kill 1",
            "kill 12 && ps aux | grep vite | wc -l",
            "command -v pkill",
            "command -V killall",
            "type pkill",
            "type killall",
            "case $1 in killall) echo nope;; esac",
            "case \"$x\" in\n  pkill|killall) echo no ;;\n  *) kill 12 ;;\nesac",
            "pids=(12 34); kill ${pids[@]}; pgrep -fl vite",
            "files=(a b); pgrep -fl x > procs.txt",
            "diff <(pgrep -fl x) >(cat); kill 12",
            "function f { echo hi; }; f",
        ] {
            assert!(!kills_by_name(cmd), "should allow: {cmd}");
        }
    }

    #[test]
    fn refuses_kills_hidden_behind_wrappers_and_stdin_shells() {
        for cmd in [
            "echo 'pkill x' | sh",
            "echo 'pkill x' | bash -s",
            "printf 'cd /tmp\\npkill x\\n' | zsh",
            "bash <<'EOF'\ncd /tmp\npkill x\nEOF",
            "sh <<EOF\nkillall node\nEOF",
            "bash <<< 'pkill x'",
            "cat <<'EOF' | bash\npkill x\nEOF",
            "find . -exec pkill {} \\;",
            "find . -name x -exec pkill {} \\;",
            "find /tmp -execdir killall {} +",
            "find . -ok pkill {} ';'",
            "watch pkill x",
            "watch -n 1 'pkill -f x'",
            "stdbuf -o0 pkill x",
            "stdbuf -o 0 pkill x",
            "arch -arm64 pkill x",
            "arch -arch arm64 killall x",
            "script -q /dev/null pkill x",
            "script -c 'pkill x' /dev/null",
            "env -S 'pkill x'",
            "env -S'pkill x'",
            "nice -n 5 pkill x",
            "nice -10 pkill x",
            "chronic pkill x",
            "parallel pkill ::: a b",
            "doas pkill x",
            "lsof -tc node | xargs kill",
            "lsof -ct node | xargs kill",
            "kill $(lsof -t -cnode)",
            "echo edge case covered in tests && pkill x",
            "echo case x in y; killall node",
            "bash -eo pipefail -c 'pkill x'",
            "bash -euo pipefail -c 'pkill x'",
            "zsh -o errexit -c 'killall node'",
            "function f { pkill x; }; f",
            "function f() { pkill x; }",
        ] {
            assert!(kills_by_name(cmd), "should refuse: {cmd}");
        }
        for cmd in [
            "cat <<EOF > notes.md\npkill x\nEOF",
            "echo 'pkill x' > notes.md",
            "echo 'pkill x' | grep kill",
            "bash script.sh <<EOF\npkill x\nEOF",
            "find . -name '*.rs' -exec grep -l pkill {} +",
            "watch -n 1 'lsof -ti tcp:3000'",
            "lsof -itcp:3000 -t | xargs kill",
            "lsof -i :3000",
            "stdbuf -o0 cargo test",
        ] {
            assert!(!kills_by_name(cmd), "should allow: {cmd}");
        }
    }

    #[test]
    fn allows_kills_by_pid_or_port_and_mere_mentions() {
        for cmd in [
            "kill 1234",
            "kill -9 1234",
            "kill $!",
            "kill %1",
            "kill -TERM \"$PID\"",
            "lsof -ti tcp:3000 -sTCP:LISTEN | xargs kill",
            "kill $(lsof -ti tcp:3000)",
            "kill $(cat server.pid)",
            "kill $PID; sleep 1; ps -p $PID",
            "pgrep -f vite",
            "ps aux | grep vite",
            "grep -r pkill src",
            "echo \"don't pkill\"",
            "echo 'killall node'",
            "git commit -m 'Never pkill -f; use kill $(lsof -ti tcp:3000)'",
            "git commit -F - <<'EOF'\nDon't pkill -f (or killall)\nEOF",
            "git commit -m \"$(cat <<'EOF'\nDon't pkill -f (or killall)\nEOF\n)\"",
            "cat <<EOF > notes.md\npkill x\nEOF",
            "echo hi > pkill.txt",
            "# pkill -f old way\nkill 12",
            "ls # killall",
            "man pkill",
            "which killall",
            "cargo test 2>&1 | tail -5",
            "rg 'pkill|killall' backend/src",
        ] {
            assert!(!kills_by_name(cmd), "should allow: {cmd}");
        }
    }

    #[test]
    fn the_hook_input_blocks_only_a_bash_kill_by_name() {
        let input = |cmd: &str| serde_json::json!({ "tool_name": "Bash", "tool_input": { "command": cmd } }).to_string();
        assert!(check(&input("cd x && pkill -f foo")).unwrap_err().contains("lsof -ti tcp:<port>"));
        assert!(check(&input("kill -9 -1")).is_err());
        assert!(check(&input("kill 1234")).is_ok());
        assert!(check("not json").is_ok());
        assert!(check("{}").is_ok());
    }

    #[test]
    fn settings_run_this_binary_as_the_bash_hook() {
        let hooks = hooks(std::path::Path::new("/Apps/it's here/loft"));
        let hook = &hooks["PreToolUse"][0];
        assert_eq!(hook["matcher"], "Bash");
        assert_eq!(hook["hooks"][0]["command"], r"'/Apps/it'\''s here/loft' kill-guard");
    }
}
