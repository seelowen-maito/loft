"""Fill a fresh demo Loft (LOFT_ROOT=demoenv, port 7440) with made-up agents, chats and tasks.

Everything here is fiction: no real names, repos, chats or keys.
"""
import json
import sqlite3
import sys
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone

BASE = "http://localhost:7440/api"
DB = sys.argv[1]
NOW = datetime.now(timezone.utc)


def ts(minutes_ago):
    return (NOW - timedelta(minutes=minutes_ago)).isoformat().replace("+00:00", "Z")


def api(method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read() or b"null")


AGENTS = [
    ("Orchestrator", "Plans the work and hands it out", "storefront", "Calm, organised, plans before acting.", "running", "Planning the checkout redesign"),
    ("Web Dev", "Builds the bakery shop website", "storefront", "Practical and quick.", "running", "Running the storefront tests"),
    ("iOS Dev", "Builds the weather app", "weather-app", "Careful about details.", "waiting", "Waiting for your approval"),
    ("Writer", "Writes recipes and release notes", "recipe-blog", "Warm and friendly.", "idle", ""),
    ("Reviewer", "Reviews every pull request", "", "Thorough, kind, direct.", "idle", ""),
    ("Trail Guide", "Keeps the trail API healthy", "trail-tracker", "Curious.", "idle", ""),
]

ids = {}
for name, title, project, pers, *_ in AGENTS:
    a = api("POST", "/agents", {"name": name, "title": title, "project": project, "personality": pers, "interval_minutes": 0})
    ids[name] = a["id"]

db = sqlite3.connect(DB)
for name, _, _, _, status, status_text in AGENTS:
    row = db.execute("select data from agents where id=?", (ids[name],)).fetchone()
    d = json.loads(row[0])
    d.update(status=status, status_text=status_text, last_run_at=ts(12), total_cost_usd=0)
    d["context_tokens"] = {"Orchestrator": 64000, "Web Dev": 41000, "iOS Dev": 23000}.get(name)
    db.execute("update agents set data=? where id=?", (json.dumps(d), ids[name]))

msg_ts = [300]


def msg(agent, role, kind, content, meta=None, run=None):
    msg_ts[0] -= 1
    db.execute("insert into messages(agent_id,run_id,role,kind,content,meta,created_at) values(?,?,?,?,?,?,?)",
               (ids[agent], run, role, kind, content, json.dumps(meta or {}), ts(msg_ts[0])))


def tool(agent, run, name, summary, inp, result):
    tid = "toolu_" + uuid.uuid4().hex[:12]
    msg(agent, "assistant", "tool_use", summary, {"id": tid, "name": name, "input": inp}, run)
    msg(agent, "tool", "tool_result", result, {"tool_use_id": tid, "is_error": False}, run)


def run(agent, user_text, steps, reply, trigger="message", end=True):
    r = str(uuid.uuid4())
    if user_text:
        msg(agent, "user", "text", user_text)
    msg(agent, "system", "run_start", trigger, {"trigger": trigger}, r)
    for s in steps:
        tool(agent, r, *s)
    if reply:
        msg(agent, "assistant", "text", reply, {}, r)
    if end:
        msg(agent, "system", "run_end", "", {"cost_usd": 0.04, "duration_ms": 48000, "num_turns": 6}, r)


# Orchestrator: plans a feature and delegates.
run("Orchestrator", "Let's redesign the checkout for the bakery shop. Keep it simple: one page, Apple Pay, and a pickup time.",
    [("Read", "Reading storefront/src/checkout/Checkout.tsx", {"file_path": "storefront/src/checkout/Checkout.tsx"}, "export function Checkout() { … }"),
     ("Grep", "Searching for “payment” in storefront", {"pattern": "payment"}, "src/checkout/Payment.tsx\nsrc/api/orders.ts")],
    "Here's the plan. I split it in three and handed it out:\n\n1. **Web Dev**: one-page checkout with a pickup time picker.\n2. **Web Dev**: Apple Pay through the existing payment provider.\n3. **Reviewer**: review both pull requests when they're ready.\n\nI'll keep the task list up to date and tell you when something needs you.")
run("Orchestrator", "Sounds good. What's left after that?", [],
    "Only the order confirmation email. I'd wait until the checkout is merged, so the email can link to the new page. Want me to queue it?")
run("Orchestrator", None, [("mcp__loft__list_tasks", "Checking the task list", {}, "3 open tasks")],
    "Morning update: the one-page checkout is ready for review, Apple Pay is half done, and nothing is blocked.", trigger="schedule")

# Web Dev: working, with a PR.
run("Web Dev", "Can you make the product photos load faster on phones?",
    [("Bash", "Running npm run build", {"command": "npm run build"}, "✓ built in 2.1s"),
     ("Edit", "Editing src/components/ProductGrid.tsx", {"file_path": "src/components/ProductGrid.tsx"}, "ok"),
     ("Bash", "Running npm test", {"command": "npm test"}, "Tests: 42 passed, 42 total")],
    "Done. Photos now load in the right size for each screen and only when they scroll into view. The home page went from 4.1 MB to 900 KB on a phone. Tests pass, and the pull request is open for review.")
r = str(uuid.uuid4())
msg("Web Dev", "user", "text", "Great. Now the one-page checkout from the Orchestrator's plan, please.")
msg("Web Dev", "system", "run_start", "message", {"trigger": "message"}, r)
tool("Web Dev", r, "Read", "Reading src/checkout/Checkout.tsx", {"file_path": "src/checkout/Checkout.tsx"}, "…")
tool("Web Dev", r, "Edit", "Editing src/checkout/PickupTime.tsx", {"file_path": "src/checkout/PickupTime.tsx"}, "ok")
msg("Web Dev", "assistant", "tool_use", "Running npm test", {"id": "toolu_live", "name": "Bash", "input": {"command": "npm test"}}, r)

# iOS Dev: waiting for a permission.
run("iOS Dev", "Add a 10-day forecast screen to the weather app.",
    [("Read", "Reading WeatherApp/ForecastView.swift", {"file_path": "WeatherApp/ForecastView.swift"}, "struct ForecastView: View { … }"),
     ("Write", "Writing WeatherApp/TenDayView.swift", {"file_path": "WeatherApp/TenDayView.swift"}, "ok")],
    "The 10-day screen is built. I need to run the app in the Simulator to take screenshots for the pull request, so I asked for permission.", end=False)
perm_id = str(uuid.uuid4())
perm = {"id": perm_id, "agent_id": ids["iOS Dev"], "source": "prompt", "tool_name": "Bash",
        "input": {"command": "xcodebuild -scheme WeatherApp -destination 'platform=iOS Simulator' build", "description": "Build the weather app for the Simulator"},
        "reason": "Build the app to take screenshots of the new forecast screen", "rule": "Bash(xcodebuild *)", "status": "pending", "created_at": ts(3)}
db.execute("insert into permissions(id,agent_id,data,created_at) values(?,?,?,?)", (perm_id, ids["iOS Dev"], json.dumps(perm), ts(3)))
msg("iOS Dev", "system", "permission", "Bash", {"request_id": perm_id})

# Writer: a finished run.
run("Writer", "Write a short post for the blog about our sourdough recipe.",
    [("Write", "Writing posts/sourdough.md", {"file_path": "posts/sourdough.md"}, "ok")],
    "The post is ready: “Sourdough in three days, without the stress.” About 600 words, with a shopping list and a timeline. Want a photo caption too?")

# Reviewer: an @-mention.
msg("Reviewer", "user", "text", "@Reviewer can you look at the image loading change before I merge it?",
    {"from_agent": {"id": ids["Web Dev"], "name": "Web Dev"}})
run("Reviewer", None, [("Bash", "Reading the pull request diff", {"command": "gh pr diff 12"}, "+ loading=\"lazy\"")],
    "Looks good. One small thing: add a width and height to each photo so the page doesn't jump while they load. Otherwise ready to merge.")
db.commit()

# Tasks.
def task(title, status, agent=None, parent=None, priority="normal", note="", summary="", desc="", project=""):
    t = api("POST", "/tasks", {"title": title, "description": desc, "priority": priority, "project": project,
                                 "agent_id": ids.get(agent), "parent_id": parent, "kind": "work" if agent else "todo"})
    row = db.execute("select data from tasks where id=?", (t["id"],)).fetchone()
    d = json.loads(row[0])
    d.update(status=status, note=note, summary=summary)
    if status in ("in_progress", "done"):
        d["started_at"] = ts(90)
    if status == "done":
        d["completed_at"] = ts(20)
    db.execute("update tasks set data=? where id=?", (json.dumps(d), t["id"]))
    db.commit()
    return t["id"]


epic = task("Checkout redesign", "in_progress", "Orchestrator", priority="high", project="storefront",
            note="2 of 3 subtasks under way", desc="One page, Apple Pay, pickup time.")
c1 = task("One-page checkout with a pickup time", "in_progress", "Web Dev", epic, "high", project="storefront", note="Running the tests")
task("Apple Pay at checkout", "ready", "Web Dev", epic, project="storefront")
task("Review the checkout pull requests", "inbox", "Reviewer", epic, project="storefront")
task("10-day forecast screen", "waiting_approval", "iOS Dev", priority="normal", project="weather-app", note="Waiting for permission to build")
task("Faster product photos on phones", "done", "Web Dev", project="storefront", summary="Home page from 4.1 MB to 900 KB on a phone.")
task("Sourdough blog post", "done", "Writer", project="recipe-blog", summary="Post ready, about 600 words.")
task("Order more flour", "inbox", priority="low")
task("Trail API: rate limits for the public map", "blocked", "Trail Guide", project="trail-tracker", note="Needs a decision: 60 or 120 requests per minute?")

for author, kind, text in [("user", "comment", "Pickup slots every 15 minutes, please, from 7 to 14."),
                           (ids["Web Dev"], "comment", "Will do. I'll grey out slots that are already full.")]:
    db.execute("insert into task_comments(task_id,author,kind,text,meta,created_at) values(?,?,?,?,?,?)", (c1, author, kind, text, "{}", ts(30)))
db.commit()
print(json.dumps(ids))
