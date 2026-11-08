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
//! Its limits, which the prompt rule covers: it reads shell text, so it doesn't see a script file the agent writes
//! and runs, a command name built at run time (`k=/bin/kill; $k 123`), or other languages sending a signal
//! (osascript, python, node). It fails open: input it can't read lets the command run.

use std::io::Read;

use serde_json::{Value, json};

/// The argument that makes this binary run as the hook.
pub const ARG: &str = "kill-guard";

const REFUSAL: &str = "Loft refused this command: it runs kill, pkill or killall by path, through sudo, `command` \
or `builtin`, as another shell's `kill`, with PATH changed, or after unsetting or redefining Loft's kill, where Loft \
can't tell whose processes it stops. Run kill, pkill or killall by name, right in your command: Loft lets them stop \
your own processes, and asks the user before stopping any other.";

/// Runs as the hook: reads Claude Code's hook input on stdin and exits 2, which blocks the call and shows stderr to
/// the agent, for a kill that skips Loft's. Input it can't read passes.
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
    json!({ "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command }] }] })
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
            "unset" | "unfunction" | "disable" | "enable" | "alias" | "hash" | "functions" => {
                f.unscoped |= args.iter().any(|a| a.contains("kill"));
                f.path |= name == "unset" && args.iter().any(|a| a == "PATH" || a == "path");
            }
            "export" | "typeset" | "declare" | "local" | "readonly" => f.path |= args.iter().any(|a| sets_path(a)),
            _ => {}
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
            p.cmds.push(Cmd::default());
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
            "unfunction kill; kill 123",
            "disable kill",
            "disable -f kill; kill 12",
            "alias k=/bin/kill; k 12",
            "hash -p /bin/kill kill",
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
    }
}
