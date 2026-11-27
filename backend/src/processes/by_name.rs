//! How an agent's `pkill`, `killall` and `taskkill` find their processes on Windows, which has no `pgrep` or `killall`
//! to ask: by a process's name (its executable's file name, `node.exe`, which matches with or without `.exe`) or, with
//! `pkill -f`, its command line. Names match whatever their case, as Windows' do. `pkill` and `killall -r` read the
//! pattern as an extended regular expression, like `pgrep`; `taskkill /IM` takes `*` wildcards.
//!
//! It only finds the processes: Loft then stops the agent's own and asks the user about any other, as for `kill`.
//! Windows has no signals, so every signal but 0 ends the process.

use std::collections::HashSet;

use super::{Output, Plan, SIGKILL, SIGTERM, names, signal_of};

/// A process as these tools see it.
pub(super) struct Named<'a> {
    pub pid: i32,
    pub ppid: i32,
    pub age_secs: u64,
    /// Its executable's file name.
    pub name: &'a str,
    /// Its command line (or its executable, when Loft may not read it).
    pub command: &'a str,
}

/// `pkill [-signal] [-f] [-x] [-i] [-n | -o] [-P ppid,...] [--] pattern`: the processes whose name (with `-f`, command
/// line) matches, but the caller and what it runs under, which pkill leaves out too.
pub(super) fn pkill(args: &[String], procs: &[Named], caller_side: &HashSet<i32>) -> Plan {
    let usage = |why: &str| Plan::Done(Output::error(2, format!("pkill: {why}\nusage: pkill [-signal] [-f] [-x] [-i] [-n | -o] [-P ppid,...] pattern\n")));
    let mut signal = SIGTERM;
    let mut rest = args;
    if let Some(sig) = args.first().and_then(|a| signal_of(a.strip_prefix('-')?)) {
        (signal, rest) = (sig, &args[1..]);
    }
    let (mut full, mut exact, mut newest, mut oldest) = (false, false, false, false);
    let mut parents: Option<Vec<i32>> = None;
    let mut patterns = vec![];
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--" => {
                patterns.extend(it.by_ref().cloned());
                break;
            }
            "--signal" => match it.next().map(|s| (s, signal_of(s))) {
                Some((_, Some(sig))) => signal = sig,
                Some((s, None)) => return usage(&format!("unknown signal {s}")),
                None => return usage("--signal needs a signal"),
            },
            "--full" => full = true,
            "--exact" => exact = true,
            "--ignore-case" => {}
            "--newest" => newest = true,
            "--oldest" => oldest = true,
            "--parent" | "-P" => match it.next().map(|v| v.split(',').map(str::parse).collect::<Result<Vec<i32>, _>>()) {
                Some(Ok(p)) => parents = Some(p),
                _ => return usage("-P needs process IDs, separated by commas"),
            },
            s if s.starts_with("--") => return usage(&format!("Loft's pkill on Windows has no {s}")),
            s if s.starts_with('-') && s.len() > 1 => {
                for (at, c) in s[1..].char_indices() {
                    match c {
                        'f' => full = true,
                        'x' => exact = true,
                        'i' => {}
                        'n' => newest = true,
                        'o' => oldest = true,
                        // `-P1234`
                        'P' => match s[at + 2..].split(',').map(str::parse).collect::<Result<Vec<i32>, _>>() {
                            Ok(p) if at + 2 < s.len() => {
                                parents = Some(p);
                                break;
                            }
                            _ => return usage("-P needs process IDs, separated by commas"),
                        },
                        c => return usage(&format!("Loft's pkill on Windows has no -{c} (it knows -f, -x, -i, -n, -o and -P)")),
                    }
                }
            }
            _ => patterns.push(a.clone()),
        }
    }
    if patterns.len() > 1 {
        return usage("only one pattern can be provided");
    }
    if patterns.is_empty() && parents.is_none() {
        return usage("no matching criteria specified");
    }
    if newest && oldest {
        return usage("-n and -o can't go together");
    }
    let pattern = match patterns.first().map(|p| Pattern::new(p, exact)) {
        Some(Ok(p)) => Some(p),
        Some(Err(why)) => return Plan::Done(Output::error(2, format!("pkill: invalid pattern: {why}\n"))),
        None => None,
    };
    let mut found: Vec<&Named> = procs
        .iter()
        .filter(|p| !caller_side.contains(&p.pid))
        .filter(|p| parents.as_ref().is_none_or(|ps| ps.contains(&p.ppid)))
        .filter(|p| pattern.as_ref().is_none_or(|pat| if full { pat.matches_command(p.command) } else { pat.matches_name(p.name) }))
        .collect();
    if newest || oldest {
        found.sort_by_key(|p| p.age_secs);
        found = if newest { found.into_iter().take(1).collect() } else { found.into_iter().last().into_iter().collect() };
    }
    if found.is_empty() {
        return Plan::Done(Output { code: 1, ..Default::default() });
    }
    Plan::Signal { signal, pids: found.iter().map(|p| p.pid).collect(), out: Output::default() }
}

/// `killall [-signal | -s signal] [-r] [-q] [-e] [-I] [--] name...`, like Linux's: the processes of each name, but the
/// caller and what it runs under. Each name that matches no process is said and makes it fail, the others still stop.
/// `-l` lists the signals.
pub(super) fn killall(args: &[String], procs: &[Named], caller_side: &HashSet<i32>) -> Plan {
    let usage = |why: &str| Plan::Done(Output::error(2, format!("killall: {why}\nusage: killall [-signal | -s signal] [-r] [-q] [-e] [-I] name...\n")));
    let (mut signal, mut regex, mut quiet) = (SIGTERM, false, false);
    let mut wanted = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--" => {
                wanted.extend(it.by_ref().cloned());
                break;
            }
            "-l" | "--list" => return Plan::Done(Output { stdout: format!("{}\n", names()), ..Default::default() }),
            "-s" | "--signal" => match it.next().map(|s| (s, signal_of(s))) {
                Some((_, Some(sig))) => signal = sig,
                Some((s, None)) => return usage(&format!("unknown signal {s}")),
                None => return usage("-s needs a signal"),
            },
            "-r" | "--regexp" => regex = true,
            "-q" | "--quiet" => quiet = true,
            // Exact names are all it matches, and whatever their case.
            "-e" | "--exact" | "-I" | "--ignore-case" => {}
            s if s.starts_with('-') && s.len() > 1 => match signal_of(&s[1..]) {
                Some(sig) => signal = sig,
                None => return usage(&format!("Loft's killall on Windows has no {s} (it knows -s, -r, -q, -e, -I and -l)")),
            },
            _ => wanted.push(a.clone()),
        }
    }
    if wanted.is_empty() {
        return usage("no process name given");
    }
    let mut out = Output::default();
    let mut pids = vec![];
    for name in &wanted {
        let matches: Box<dyn Fn(&str) -> bool> = if regex {
            match Pattern::new(name, true) {
                Ok(p) => Box::new(move |n| p.matches_name(n)),
                Err(why) => return Plan::Done(Output::error(1, format!("killall: bad regular expression {name}: {why}\n"))),
            }
        } else {
            Box::new(|n| same_name(n, name))
        };
        let found: Vec<i32> = procs.iter().filter(|p| !caller_side.contains(&p.pid) && matches(p.name)).map(|p| p.pid).collect();
        if found.is_empty() {
            if !quiet {
                out.stderr += &format!("{name}: no process found\n");
            }
            out.code = 1;
        }
        pids.extend(found.into_iter().filter(|p| !pids.contains(p)).collect::<Vec<_>>());
    }
    Plan::Signal { signal, pids, out }
}

/// `taskkill { /PID pid | /IM image }... [/T] [/F]`, options in any case and written `/PID`, `//PID` (Git Bash's way to
/// keep a slash) or `-PID`: the processes named, and with `/T` the processes under them (but the caller and what it
/// runs under). An image takes `*` wildcards. Windows ends a process the same way with or without `/F`.
pub(super) fn taskkill(args: &[String], procs: &[Named], caller_side: &HashSet<i32>) -> Plan {
    let error = |code: i32, why: String| Plan::Done(Output::error(code, format!("ERROR: {why}\n")));
    let (mut pids_named, mut images, mut tree, mut force) = (vec![], vec![], false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let Some(option) = a.strip_prefix("//").or_else(|| a.strip_prefix('/')).or_else(|| a.strip_prefix('-')) else {
            return error(1, format!("Invalid argument/option - '{a}'.\nType \"TASKKILL /?\" for usage."));
        };
        match option.to_ascii_uppercase().as_str() {
            "PID" => match it.next() {
                Some(v) => pids_named.push(v.clone()),
                None => return error(1, "Invalid syntax. Value expected for '/PID'.".into()),
            },
            "IM" => match it.next() {
                Some(v) => images.push(v.clone()),
                None => return error(1, "Invalid syntax. Value expected for '/IM'.".into()),
            },
            "T" => tree = true,
            "F" => force = true,
            "S" | "U" | "P" => return error(1, "Loft's taskkill stops processes only on this computer.".into()),
            "FI" => return error(1, "Loft's taskkill takes /PID and /IM, not /FI.".into()),
            "?" => return Plan::Done(Output { stdout: "TASKKILL { /PID processid | /IM imagename }... [/T] [/F]\n".into(), ..Default::default() }),
            _ => return error(1, format!("Invalid argument/option - '{a}'.\nType \"TASKKILL /?\" for usage.")),
        }
    }
    if pids_named.is_empty() && images.is_empty() {
        return error(1, "Invalid syntax. Neither /FI nor /PID nor /IM were specified.\nType \"TASKKILL /?\" for usage.".into());
    }
    let mut out = Output::default();
    let mut roots = vec![];
    for v in &pids_named {
        match v.parse::<i32>() {
            Ok(pid) if procs.iter().any(|p| p.pid == pid) => roots.push(pid),
            _ => {
                out.stderr += &format!("ERROR: The process \"{v}\" not found.\n");
                out.code = 128;
            }
        }
    }
    for image in &images {
        let found: Vec<i32> = procs.iter().filter(|p| !caller_side.contains(&p.pid) && wildcard(image, p.name)).map(|p| p.pid).collect();
        if found.is_empty() {
            out.stderr += &format!("ERROR: The process \"{image}\" not found.\n");
            out.code = 128;
        }
        roots.extend(found);
    }
    let mut pids: Vec<i32> = vec![];
    for pid in roots {
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    if tree {
        // A child started after its parent: a PID Windows handed out again is no parent of an older process.
        let mut i = 0;
        while i < pids.len() {
            let parent = pids[i];
            let age = procs.iter().find(|p| p.pid == parent).map_or(u64::MAX, |p| p.age_secs);
            let children: Vec<i32> = procs.iter().filter(|p| p.ppid == parent && p.pid != parent && p.age_secs <= age && !caller_side.contains(&p.pid) && !pids.contains(&p.pid)).map(|p| p.pid).collect();
            pids.extend(children);
            i += 1;
        }
    }
    Plan::Signal { signal: if force { SIGKILL } else { SIGTERM }, pids, out }
}

/// Whether a process named `name` (`node.exe`) is the `wanted` one: the same name, or the same without `.exe`.
fn same_name(name: &str, wanted: &str) -> bool {
    let bare = |s: &str| {
        let lower = s.to_lowercase();
        lower.strip_suffix(".exe").map(String::from).unwrap_or(lower)
    };
    bare(name) == bare(wanted)
}

/// taskkill's `/IM`: `*` matches any run of characters, whatever their case; without `.exe` the name matches too.
fn wildcard(pattern: &str, name: &str) -> bool {
    fn glob(p: &[char], s: &[char]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some(('*', rest)) => (0..=s.len()).any(|i| glob(rest, &s[i..])),
            Some((c, rest)) => s.first() == Some(c) && glob(rest, &s[1..]),
        }
    }
    let (p, n): (Vec<char>, Vec<char>) = (pattern.to_lowercase().chars().collect(), name.to_lowercase().chars().collect());
    let bare: Vec<char> = name.to_lowercase().strip_suffix(".exe").unwrap_or(&name.to_lowercase()).chars().collect();
    glob(&p, &n) || glob(&p, &bare)
}

// ---- patterns -------------------------------------------------------------

/// A regular expression as `pgrep` reads one (the `regex` crate's syntax, which covers ERE's), found anywhere in the
/// text (unless anchored), whatever the case.
pub(super) struct Pattern(regex::Regex);

impl Pattern {
    /// The pattern, which with `exact` must match the whole text (`pkill -x`).
    pub(super) fn new(pattern: &str, exact: bool) -> Result<Self, String> {
        let p = if exact { format!("^(?:{pattern})$") } else { pattern.to_string() };
        regex::RegexBuilder::new(&p).case_insensitive(true).build().map(Pattern).map_err(|e| e.to_string())
    }

    fn is_match(&self, text: &str) -> bool {
        self.0.is_match(text)
    }

    /// A process's name, with or without `.exe`.
    pub(super) fn matches_name(&self, name: &str) -> bool {
        self.is_match(name) || name.to_lowercase().strip_suffix(".exe").is_some_and(|bare| self.is_match(bare))
    }

    /// A command line, also with `/` for `\`, so a pattern written the way Git Bash writes paths matches too.
    pub(super) fn matches_command(&self, command: &str) -> bool {
        self.is_match(command) || (command.contains('\\') && self.is_match(&command.replace('\\', "/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procs() -> Vec<Named<'static>> {
        let p = |pid, ppid, age_secs, name, command| Named { pid, ppid, age_secs, name, command };
        vec![
            p(10, 1, 500, "bash.exe", r"C:\Program Files\Git\usr\bin\bash.exe"),
            p(11, 10, 400, "loft.exe", r"C:\Users\me\loft\loft.exe kill-as pkill -f vite"),
            p(20, 10, 300, "node.exe", r#""C:\Program Files\nodejs\node.exe" C:\Users\me\app\node_modules\vite\bin\vite.js --port 20100"#),
            p(21, 20, 200, "esbuild.exe", r"C:\Users\me\app\node_modules\@esbuild\win32-x64\esbuild.exe --service=0.21.5 --ping"),
            p(30, 1, 100, "Node.EXE", r"C:\tools\Node.EXE server.js"),
            p(40, 1, 50, "postgres.exe", r"C:\pg\bin\postgres.exe -D C:\pg\data"),
            // An older process whose parent PID 40 was handed out again: not a child of the postgres above.
            p(41, 40, 9000, "conhost.exe", r"\??\C:\Windows\system32\conhost.exe 0x4"),
        ]
    }

    fn sig(plan: Plan) -> (i32, Vec<i32>, Output) {
        match plan {
            Plan::Signal { signal, pids, out } => (signal, pids, out),
            Plan::Done(out) => panic!("no signal: {out:?}"),
        }
    }

    fn done(plan: Plan) -> Output {
        match plan {
            Plan::Done(out) => out,
            Plan::Signal { pids, .. } => panic!("signals {pids:?}"),
        }
    }

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    /// The caller (`loft kill-as`, 11) and the shell it runs under (10).
    fn caller() -> HashSet<i32> {
        HashSet::from([10, 11])
    }

    #[test]
    fn pkill_matches_the_name_or_with_f_the_command_line_but_never_the_caller() {
        let ps = procs();
        // By name, whatever the case, with or without .exe.
        assert_eq!(sig(pkill(&args(&["node"]), &ps, &caller())).1, vec![20, 30]);
        assert_eq!(sig(pkill(&args(&["-x", "node"]), &ps, &caller())).1, vec![20, 30]);
        assert_eq!(sig(pkill(&args(&["-x", "node.exe"]), &ps, &caller())).1, vec![20, 30]);
        assert_eq!(done(pkill(&args(&["-x", "nod"]), &ps, &caller())).code, 1, "-x needs the whole name");
        // `vite` is in node's command line, and in the caller's, which is left out.
        assert_eq!(done(pkill(&args(&["vite"]), &ps, &caller())).code, 1, "no process is named vite");
        let (signal, pids, _) = sig(pkill(&args(&["-f", "vite"]), &ps, &caller()));
        assert_eq!((signal, pids), (SIGTERM, vec![20]));
        // A regular expression, and a path written the Git Bash way.
        assert_eq!(sig(pkill(&args(&["-9", "-f", "vite\\.js|server\\.js$"]), &ps, &caller())).1, vec![20, 30]);
        assert_eq!(sig(pkill(&args(&["-KILL", "-f", "C:/Users/me/app/node_modules/vite"]), &ps, &caller())).0, SIGKILL);
        // Anchored: the command line starts with the path, the name with postgres.
        assert_eq!(done(pkill(&args(&["-f", "^postgres"]), &ps, &caller())).code, 1);
        assert_eq!(sig(pkill(&args(&["^postgres"]), &ps, &caller())).1, vec![40]);
    }

    #[test]
    fn pkill_picks_by_parent_newest_or_oldest_and_says_what_it_does_not_know() {
        let ps = procs();
        assert_eq!(sig(pkill(&args(&["-P", "20"]), &ps, &caller())).1, vec![21]);
        assert_eq!(sig(pkill(&args(&["-P10", "-f", "."]), &ps, &caller())).1, vec![20], "the caller (11) is left out");
        assert_eq!(sig(pkill(&args(&["-n", "node"]), &ps, &caller())).1, vec![30]);
        assert_eq!(sig(pkill(&args(&["-o", "node"]), &ps, &caller())).1, vec![20]);
        assert_eq!(sig(pkill(&args(&["-if", "VITE"]), &ps, &caller())).1, vec![20]);
        assert_eq!(sig(pkill(&args(&["--signal", "KILL", "-f", "--", "--port"]), &ps, &caller())).1, vec![20]);
        for (bad, why) in [
            (vec![], "no matching criteria"),
            (vec!["a", "b"], "only one pattern"),
            (vec!["-u", "me", "node"], "has no -u"),
            (vec!["--euid", "node"], "has no --euid"),
            (vec!["-n", "-o", "node"], "can't go together"),
            (vec!["-P", "x"], "-P needs"),
        ] {
            let out = done(pkill(&args(&bad), &ps, &caller()));
            assert_eq!(out.code, 2, "{bad:?}");
            assert!(out.stderr.contains(why), "{bad:?}: {}", out.stderr);
        }
        let out = done(pkill(&args(&["(vite"]), &ps, &caller()));
        assert!(out.code == 2 && out.stderr.contains("invalid pattern"), "{out:?}");
    }

    #[test]
    fn killall_stops_each_name_says_the_ones_it_did_not_find_and_lists_signals() {
        let ps = procs();
        let (signal, pids, out) = sig(killall(&args(&["node"]), &ps, &caller()));
        assert_eq!((signal, pids, out.code), (SIGTERM, vec![20, 30], 0));
        // A name is the whole name: `nod` is no process, and `postgres.exe` is postgres.
        let (signal, pids, out) = sig(killall(&args(&["-9", "nod", "POSTGRES.exe"]), &ps, &caller()));
        assert_eq!((signal, pids, out.code), (SIGKILL, vec![40], 1));
        assert_eq!(out.stderr, "nod: no process found\n");
        let (_, pids, out) = sig(killall(&args(&["-q", "nod"]), &ps, &caller()));
        assert!(pids.is_empty() && out.code == 1 && out.stderr.is_empty(), "{out:?}");
        assert_eq!(sig(killall(&args(&["-s", "KILL", "-r", "^(node|esbuild)$"]), &ps, &caller())).1, vec![20, 21, 30]);
        // Never the caller's shell.
        assert!(sig(killall(&args(&["bash"]), &ps, &caller())).1.is_empty());
        assert!(done(killall(&args(&["-l"]), &ps, &caller())).stdout.contains("KILL"));
        assert_eq!(done(killall(&args(&["-m", "node"]), &ps, &caller())).code, 2);
        assert_eq!(done(killall(&args(&[]), &ps, &caller())).code, 2);
    }

    #[test]
    fn taskkill_takes_pids_and_images_written_any_way_and_with_t_their_trees() {
        let ps = procs();
        let (signal, pids, out) = sig(taskkill(&args(&["/PID", "20"]), &ps, &caller()));
        assert_eq!((signal, pids, out.code), (SIGTERM, vec![20], 0));
        let (signal, pids, _) = sig(taskkill(&args(&["//f", "//pid", "20", "//T"]), &ps, &caller()));
        assert_eq!((signal, pids), (SIGKILL, vec![20, 21]));
        assert_eq!(sig(taskkill(&args(&["-IM", "node.exe", "-F"]), &ps, &caller())).1, vec![20, 30]);
        assert_eq!(sig(taskkill(&args(&["/IM", "post*"]), &ps, &caller())).1, vec![40]);
        // The PID 40 older process isn't postgres's child.
        assert_eq!(sig(taskkill(&args(&["/IM", "postgres.exe", "/T"]), &ps, &caller())).1, vec![40]);
        // What it doesn't find, it says the way taskkill does, and stops the rest.
        let (_, pids, out) = sig(taskkill(&args(&["/PID", "99", "/PID", "30", "/IM", "nope.exe"]), &ps, &caller()));
        assert_eq!((pids, out.code), (vec![30], 128));
        assert_eq!(out.stderr, "ERROR: The process \"99\" not found.\nERROR: The process \"nope.exe\" not found.\n");
        for (bad, why) in [
            (vec![], "Neither /FI nor /PID nor /IM"),
            (vec!["/S", "server", "/PID", "20"], "only on this computer"),
            (vec!["/FI", "IMAGENAME eq node.exe"], "not /FI"),
            (vec!["/PID"], "Value expected for '/PID'"),
            (vec!["20"], "Invalid argument/option - '20'"),
        ] {
            let out = done(taskkill(&args(&bad), &ps, &caller()));
            assert_eq!(out.code, 1, "{bad:?}");
            assert!(out.stderr.contains(why), "{bad:?}: {}", out.stderr);
        }
    }

    #[test]
    fn patterns_read_like_pgrep_s() {
        let m = |p: &str, s: &str| Pattern::new(p, false).unwrap().is_match(s);
        assert!(m("vite", "node vite.js") && !m("vite", "node next.js"), "anywhere in the text");
        assert!(m("^(next|vite)( |$)", "vite --port 1") && !m("^(next|vite)( |$)", "vitest"));
        assert!(m("NODE", "node") && m("node", "NODE"), "whatever the case");
        assert!(Pattern::new("^node$", false).unwrap().matches_name("node.exe"));
        assert!(Pattern::new("x", true).unwrap().is_match("X") && !Pattern::new("x", true).unwrap().is_match("xx"));
        assert!(Pattern::new("a|b", true).unwrap().is_match("b") && !Pattern::new("a|b", true).unwrap().is_match("ab"), "-x wraps the whole pattern");
        assert!(Pattern::new("c:/app/vite", false).unwrap().matches_command("node C:\\app\\vite.js"), "Git Bash's slashes");
        for bad in ["(a", "a)", "[a", "*a", "a{3,1}"] {
            assert!(Pattern::new(bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn taskkill_images_take_wildcards_and_names_their_exe() {
        assert!(wildcard("node.exe", "Node.EXE") && wildcard("node", "node.exe") && wildcard("*.exe", "a.exe") && wildcard("no*", "node.exe"));
        assert!(!wildcard("node", "nodemon.exe") && !wildcard("*.dll", "a.exe"));
        assert!(same_name("node.exe", "NODE") && !same_name("node.exe", "nod"));
    }
}
