#!/usr/bin/env python3
"""Probe: does `claude -p` take a user message in the middle of a run, and what hook events does it print?

Spawns the real Claude Code the way Loft runs agents (`-p --output-format stream-json --verbose`), plus
`--input-format stream-json --include-hook-events --replay-user-messages`, keeps stdin open and logs every
stdout line with the time it arrived. It sends a second user message at a chosen moment and closes stdin
once the run has answered it. See #476 for what it found.

Scenarios (--scenario):
  stop-hook  A Stop hook blocks until a user message says the password; the second message (the password)
             is sent after the first block. Capped at --max-blocks so a broken run can't loop forever.
  goal       `/goal` with a condition that needs the password; the second message is sent after the first
             goal evaluation that doesn't pass.
  goal-clear Same goal, but the second message is `/goal clear`.
  tools      The first prompt runs a slow Bash command; the second message is sent while it runs, to see
             whether it enters after the tool or at the end of the turn.

  python3 scripts/probe-midrun-input.py --scenario stop-hook [--priority now] [--no-tools] [--model haiku] [--out log.jsonl]

The log (--out, default stdout summary only) has one JSON object per line: {"t": seconds, "dir": "in"|"out"
|"err", "line": ...}.

What it found with Claude Code 2.1.331 (details and real lines in #476):
- Input: {"type":"user","message":{"role":"user","content":[{"type":"text","text":"..."}]},"priority":"now"}.
  Without `priority` (= "next") a message sent mid-run enters after the running tool's result, in the same
  turn; but a turn that only writes text (a Stop hook or /goal loop) never takes it: it waits until the hook
  gives up (Claude's own cap: 9 blocks in a row) and then starts a new turn. With "now" the turn is aborted
  (a result with terminal_reason "aborted_streaming", or "aborted_tools" after a running tool finishes, and
  result "") and a new turn starts with the message.
- `--replay-user-messages` echoes the message ({..., "isReplay": true}) when it enters, not when it's sent.
- Every block, of a Stop hook or of /goal, is a user line {"isSynthetic": true} whose text starts
  "Stop hook feedback:\n": then the hook's reason, or "[<goal condition>]: <evaluator's reason>" for /goal.
  `--include-hook-events` adds system hook_started / hook_response (with the hook's stdout) for command
  hooks only; /goal prints none.
- /goal answers with a synthetic assistant text "Goal set: <condition>"; `/goal clear` gives a turn of its
  own with result "Goal cleared: <condition>" (num_turns 0). The cap prints system "informational" and
  notification key "stop-hook-block-cap".
- With stdin open claude stays alive after a result; it exits once stdin is closed.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time

PASSWORD = "BANANA-42"
GOAL = f"/goal The user has told you the password {PASSWORD}. Until then, ask for the password in one short line; never guess it."

HOOK = r'''#!/usr/bin/env python3
import json, os, sys
d = json.load(sys.stdin)
count = os.path.join(os.path.dirname(os.path.abspath(__file__)), "blocks")
n = int(open(count).read()) if os.path.exists(count) else 0
said = False
try:
    for line in open(d["transcript_path"]):
        e = json.loads(line)
        if e.get("type") == "user" and "%(pw)s" in json.dumps(e.get("message", {})):
            said = True
except Exception:
    pass
if said or n >= %(max)d:
    sys.exit(0)
open(count, "w").write(str(n + 1))
print(json.dumps({"decision": "block", "reason": "Objective not met: the user hasn't said the password yet. Ask for it in one short line."}))
'''


def user_line(text, priority=None):
    msg = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}
    if priority:
        msg["priority"] = priority
    return json.dumps(msg) + "\n"


def is_block(e):
    """A Stop hook or /goal that kept the turn from ending: both inject a synthetic user message (a /goal
    evaluation prints no hook_started/hook_response events, so this is the one signal they share)."""
    if e.get("type") != "user" or not e.get("isSynthetic"):
        return False
    content = e.get("message", {}).get("content")
    text = content if isinstance(content, str) else "".join(c.get("text", "") for c in content or [] if isinstance(c, dict))
    return text.startswith("Stop hook feedback:")


def main():
    sys.stdout.reconfigure(line_buffering=True)
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", choices=["stop-hook", "goal", "goal-clear", "tools"], required=True)
    ap.add_argument("--claude", default="claude")
    ap.add_argument("--model", default="haiku")
    ap.add_argument("--max-blocks", type=int, default=6)
    ap.add_argument("--timeout", type=float, default=240)
    ap.add_argument("--priority", choices=["now", "next", "later"], help="priority field on the second message")
    ap.add_argument("--no-tools", action="store_true", help="run with no built-in tools (--tools \"\")")
    ap.add_argument("--out")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="probe-midrun-")
    settings = {}
    if args.scenario == "stop-hook":
        hook = os.path.join(work, "stop-hook.py")
        with open(hook, "w") as f:
            f.write(HOOK % {"pw": PASSWORD, "max": args.max_blocks})
        os.chmod(hook, 0o755)
        settings["hooks"] = {"Stop": [{"hooks": [{"type": "command", "command": hook}]}]}

    cmd = [
        args.claude, "-p",
        "--input-format", "stream-json",
        "--output-format", "stream-json", "--verbose",
        "--include-hook-events", "--replay-user-messages",
        "--setting-sources", "",
        "--settings", json.dumps(settings),
        "--model", args.model,
        "--permission-mode", "bypassPermissions",
        # No --no-session-persistence: the Stop hook reads the transcript to see the password.
        "--strict-mcp-config",
    ]
    if args.no_tools:
        cmd += ["--tools", ""]
    first = {
        "stop-hook": "Say hello in one short line.",
        "goal": GOAL,
        "goal-clear": GOAL,
        "tools": "Run the Bash command `sleep 15; echo slept` and then tell me what it printed.",
    }[args.scenario]
    second = {
        "stop-hook": f"The password is {PASSWORD}.",
        "goal": f"The password is {PASSWORD}.",
        "goal-clear": "/goal clear",
        "tools": "Also: what is 2+2? Answer after the command.",
    }[args.scenario]

    log = open(args.out, "w") if args.out else None
    t0 = time.monotonic()
    lock = threading.Lock()

    def record(direction, line):
        entry = {"t": round(time.monotonic() - t0, 2), "dir": direction}
        try:
            entry["line"] = json.loads(line)
        except ValueError:
            entry["line"] = line.rstrip("\n")
        with lock:
            if log:
                log.write(json.dumps(entry) + "\n")
                log.flush()
        return entry

    p = subprocess.Popen(cmd, cwd=work, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    threading.Thread(target=lambda: [record("err", l) for l in p.stderr], daemon=True).start()

    def send(text, priority=None):
        line = user_line(text, priority)
        record("in", line)
        p.stdin.write(line)
        p.stdin.flush()
        print(f"[{time.monotonic() - t0:6.2f}s] >>> {text}" + (f" (priority {priority})" if priority else ""))

    timer = threading.Timer(args.timeout, lambda: (print("timeout: stopping claude", flush=True), p.terminate()))
    timer.daemon = True
    timer.start()
    send(first)
    sent_second = False
    results = 0
    for raw in p.stdout:
        e = record("out", raw)["line"]
        if not isinstance(e, dict):
            continue
        kind = e.get("type"), e.get("subtype")
        summary = ""
        if e.get("type") == "assistant":
            for c in e.get("message", {}).get("content", []):
                if c.get("type") == "text":
                    summary += "text: " + c["text"][:100]
                elif c.get("type") == "tool_use":
                    summary += f"tool_use {c['name']} "
        elif e.get("type") == "user":
            summary = json.dumps(e.get("message", {}).get("content"))[:120]
        elif e.get("type") == "system" and str(e.get("subtype", "")).startswith("hook"):
            summary = json.dumps({k: e.get(k) for k in ("hook_name", "hook_event", "outcome", "exit_code", "output", "stdout")})[:240]
        elif e.get("type") == "result":
            summary = f"is_error={e.get('is_error')} turns={e.get('num_turns')} result={str(e.get('result'))[:80]!r}"
        else:
            summary = json.dumps(e)[:160]
        print(f"[{time.monotonic() - t0:6.2f}s] {kind[0]}/{kind[1]} {summary}")

        if not sent_second:
            if args.scenario == "tools" and e.get("type") == "assistant" and "tool_use" in json.dumps(e):
                time.sleep(3)
                send(second, args.priority)
                sent_second = True
            elif args.scenario != "tools" and is_block(e):
                send(second, args.priority)
                sent_second = True
        if e.get("type") == "result":
            results += 1
            if sent_second or results > 1:
                p.stdin.close()
            else:
                # The run ended before the trigger (no block came): send it now, as a new turn.
                send(second, args.priority)
                sent_second = True
    p.wait()
    print(f"exit {p.returncode}, {results} result(s), second message sent: {sent_second}")


if __name__ == "__main__":
    main()
