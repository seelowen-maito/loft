//! The `PreToolUse` hook on Bash that Loft gives every agent: it refuses commands that kill processes by name
//! or pattern (`pkill -f vite`, `killall node`, `pgrep -f next | xargs kill`). Those hit every process on the Mac
//! whose command line matches: other agents' claude and their running Bash commands (Claude Code runs each one
//! as `zsh -c '<command>'`), and the user's own stdio MCP servers. Kills by PID or port stay allowed.
//!
//! Claude runs it before every Bash call, so it is a plain scan of the command's shell words, in this binary
//! (`loft kill-guard`) and before any runtime starts. It can't see into a script file the agent writes and runs;
//! the prompt rule covers that.

use std::io::Read;

use serde_json::Value;

/// The argument that makes this binary run as the hook.
pub const ARG: &str = "kill-guard";

const REFUSAL: &str = "Loft refused this command: it kills processes by name or pattern (pkill, killall, or kill \
fed by pgrep, pidof or ps). That also hits other agents' running commands and servers and the user's MCP servers \
on this Mac. Stop only your own processes: by port, `lsof -ti tcp:<port> -sTCP:LISTEN | xargs kill`, or by the PID \
you saved when you started it (`kill <pid>`, `kill $!`). If you must look a PID up, do it in one command, check the \
process is yours (started from your workspace), then kill it by number in another.";

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

/// Whether `command` runs `pkill` or `killall`, or both runs `kill` and looks processes up by name (`pgrep`,
/// `pidof`, `ps` other than `ps -p`, `lsof -c`), wherever they sit: after `&&` or `;`, in a pipe, a subshell, a
/// `$(...)`, behind `sudo`/`xargs`/`env`, or in a `sh -c '...'`. Words that are only arguments (`grep pkill`,
/// `echo "don't pkill"`) and heredoc bodies don't count.
pub fn kills_by_name(command: &str) -> bool {
    let (mut kill, mut lookup) = (false, false);
    let mut texts = vec![command.to_string()];
    while let Some(text) = texts.pop() {
        for words in simple_commands(&text) {
            let Some((name, args)) = unwrap(&words).split_first() else { continue };
            let has = |flag: &str| args.iter().any(|a| a.starts_with(flag));
            match name.rsplit('/').next().unwrap_or_default() {
                "pkill" | "killall" => return true,
                "kill" => kill = true,
                "pgrep" | "pidof" => lookup = true,
                "ps" if !has("-p") => lookup = true,
                "lsof" if has("-c") => lookup = true,
                "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                    let c = args.iter().position(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('c'));
                    texts.extend(c.and_then(|i| args.get(i + 1)).cloned());
                }
                "eval" => texts.push(args.join(" ")),
                _ => {}
            }
        }
    }
    kill && lookup
}

const KEYWORDS: &[&str] = &["!", "{", "}", "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "esac"];

/// The command a simple command runs: past variable assignments, shell keywords and wrappers that run the
/// command they're given (`sudo`, `env`, `xargs`, `nohup`, ...).
fn unwrap(words: &[String]) -> &[String] {
    let mut i = 0;
    while let Some(w) = words.get(i) {
        let assignment = w.split_once('=').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        if assignment || KEYWORDS.contains(&w.as_str()) {
            i += 1;
            continue;
        }
        // Each wrapper with its options that take a value.
        let name = w.rsplit('/').next().unwrap_or_default();
        let valued: &[&str] = match name {
            "sudo" => &["-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T"],
            "env" => &["-u", "-C", "-S", "-P"],
            "xargs" => &["-n", "-I", "-L", "-P", "-s", "-E", "-J", "-R", "-S", "-d", "-a"],
            "nice" => &["-n"],
            "timeout" => &["-s", "-k"],
            "caffeinate" => &["-t", "-w"],
            "exec" => &["-a"],
            "command" | "builtin" | "nohup" | "time" => &[],
            _ => return &words[i..],
        };
        i += 1;
        while let Some(o) = words.get(i).filter(|o| o.starts_with('-')) {
            i += 1;
            if o == "--" {
                break;
            }
            if valued.contains(&o.as_str()) {
                i += 1;
            }
        }
        if name == "timeout" {
            i += 1; // the duration
        }
    }
    &words[i..]
}

/// Splits shell text into its simple commands, each a list of words with quotes removed, including the
/// commands inside `$(...)`, backticks and `<(...)`. Redirection targets and heredoc bodies are left out.
fn simple_commands(text: &str) -> Vec<Vec<String>> {
    let c: Vec<char> = text.chars().collect();
    let mut cmds = vec![];
    lex(&c, 0, None, &mut cmds);
    cmds
}

#[derive(Default)]
struct Words {
    words: Vec<String>,
    word: String,
    in_word: bool,
    /// The next word is a redirection's target.
    redirect: bool,
}

impl Words {
    fn end_word(&mut self) {
        if self.in_word && !std::mem::take(&mut self.redirect) {
            self.words.push(std::mem::take(&mut self.word));
        }
        self.word.clear();
        self.in_word = false;
    }

    fn end(&mut self, cmds: &mut Vec<Vec<String>>) {
        self.end_word();
        if !self.words.is_empty() {
            cmds.push(std::mem::take(&mut self.words));
        }
    }
}

/// Lexes from `i` to the end, or past the `stop` (`)` or a backtick) that closes a substitution, adding the
/// commands it finds to `cmds`. Returns the index it stopped at.
fn lex(c: &[char], mut i: usize, stop: Option<char>, cmds: &mut Vec<Vec<String>>) -> usize {
    let mut w = Words::default();
    let mut heredocs: Vec<String> = vec![]; // delimiters whose bodies start at the next newline
    let mut depth = 0; // subshell parens open inside a `$(`
    while let Some(&ch) = c.get(i) {
        let next = c.get(i + 1).copied();
        i += 1;
        match ch {
            '\\' => {
                if let Some(n) = next {
                    if n != '\n' {
                        w.word.push(n);
                        w.in_word = true;
                    }
                    i += 1;
                }
            }
            '\'' => {
                w.in_word = true;
                while let Some(&q) = c.get(i) {
                    i += 1;
                    if q == '\'' {
                        break;
                    }
                    w.word.push(q);
                }
            }
            '"' => {
                w.in_word = true;
                while let Some(&q) = c.get(i) {
                    i += 1;
                    match q {
                        '"' => break,
                        '\\' => {
                            w.word.extend(c.get(i));
                            i += 1;
                        }
                        '$' if c.get(i) == Some(&'(') => i = lex(c, i + 1, Some(')'), cmds),
                        '`' => i = lex(c, i, Some('`'), cmds),
                        _ => w.word.push(q),
                    }
                }
            }
            '`' if stop == Some('`') => break,
            '`' => {
                w.in_word = true;
                i = lex(c, i, Some('`'), cmds);
            }
            '$' | '<' | '>' if next == Some('(') => {
                w.in_word = true;
                i = lex(c, i + 1, Some(')'), cmds);
            }
            '$' if next == Some('{') => {
                w.in_word = true;
                while let Some(&n) = c.get(i) {
                    i += 1;
                    w.word.push(n);
                    if n == '}' {
                        break;
                    }
                }
            }
            '#' if !w.in_word => {
                while c.get(i).is_some_and(|&n| n != '\n') {
                    i += 1;
                }
            }
            ' ' | '\t' => w.end_word(),
            '(' if stop == Some(')') => {
                depth += 1;
                w.end(cmds);
            }
            ')' if stop == Some(')') && depth == 0 => break,
            ')' if stop == Some(')') => {
                depth -= 1;
                w.end(cmds);
            }
            '\n' | ';' | '&' | '|' | '(' | ')' => {
                w.end(cmds);
                if ch == '\n' {
                    i = skip_heredocs(c, i, &mut heredocs);
                }
            }
            '<' | '>' => {
                if w.word.chars().all(|d| d.is_ascii_digit()) {
                    w.in_word = false; // `2>`: a file descriptor, not a word
                }
                w.end_word();
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
                    heredocs.push(delimiter);
                } else {
                    while c.get(i).is_some_and(|&n| "<>&|".contains(n)) {
                        i += 1;
                    }
                    w.redirect = true;
                }
            }
            _ => {
                w.word.push(ch);
                w.in_word = true;
            }
        }
    }
    w.end(cmds);
    i
}

/// Skips the bodies of the heredocs started on the line that ended just before `i`.
fn skip_heredocs(c: &[char], mut i: usize, heredocs: &mut Vec<String>) -> usize {
    for delimiter in heredocs.drain(..) {
        while i < c.len() {
            let end = c[i..].iter().position(|&n| n == '\n').map_or(c.len(), |p| i + p);
            let line: String = c[i..end].iter().collect();
            i = end + 1;
            if line.trim() == delimiter {
                break;
            }
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
        ] {
            assert!(kills_by_name(cmd), "should refuse: {cmd}");
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
