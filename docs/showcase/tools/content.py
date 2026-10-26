"""The showcase copy, shared by the web page and the PDF. Plain words, no internal jargon."""

TAGLINE = "Your own crew of AI developers, working from your Mac."
INTRO = ("Loft runs a small team of Claude agents on your Mac. Each one has a job, a face and a memory. "
         "They write code, open pull requests, plan work and hand it to each other. "
         "You follow along, answer their questions and approve what matters, from the browser or your iPhone.")

# key, eyebrow, title, lead, bullets, web shots, phone shots
SECTIONS = [
    ("crew", "The crew", "Agents with a job, a face and a memory",
     "Give an agent a role and a project, and it gets to work. It can wake up on a schedule, remember what it learned, and draw its own sticker face.",
     ["Each agent works in its own folder and copies your projects instead of touching the originals.",
      "Schedules with quiet hours, so nothing wakes up while you sleep.",
      "Long-term memory that carries over between conversations.",
      "See at a glance who is working, who is waiting on you, and what each one costs.",
      "If a run goes quiet or gets stuck, Loft says so and offers a one-tap restart that keeps your messages."],
     ["web-repos-agents"], ["digest"]),
    ("chat", "Chat", "A conversation that shows the work",
     "Talk to any agent like a teammate. Every file it reads, every command it runs and every result shows up in the chat, so you always know what it did.",
     ["Paste, drag in or pick images.",
      "@-mention another agent to pull it into the conversation.",
      "When an agent asks you something, tap a suggested reply.",
      "Plan cards show delegated work and its subtasks, live.",
      "Attach a task to a message, and tap a task chip to open it."],
     ["web-chat-orchestrator", "web-chat-webdev"], ["chat", "mention"]),
    ("tasks", "Tasks", "One shared to-do list for you and the crew",
     "Tasks are where work gets tracked. You create them, agents pick them up, report progress and hand pieces to each other.",
     ["Big jobs split into subtasks, each with its own agent.",
      "Every task has a thread: progress notes, status changes and comments.",
      "Comment on a task and the agent working on it answers right there.",
      "The agent that handed work out hears back when it's done, failed or blocked.",
      "The same list on the web and on your phone."],
     ["web-task-detail", "web-tasks"], ["tasks", "task-detail"]),
    ("needs-you", "Needs you", "Everything waiting on you, in one place",
     "Agents ask before doing anything risky. Approvals, questions, failed work and pull requests ready to merge land in one inbox, sorted by what's most urgent.",
     ["Allow once, always allow, or deny, with the exact command shown.",
      "Risk labels and a plain-language “What does this do?” explanation.",
      "Pull requests grouped by project, with a merge button.",
      "Get a heads-up on your iPhone, or a text by iMessage."],
     ["web-chat-approval"], ["needs-you"]),
    ("voice", "Voice", "Talk to your crew, hands-free",
     "Tap Talk and just speak. Loft keeps listening, sends when you're done and reads the answer back to you.",
     ["Long dictations get tidied up on the Mac before they're sent.",
      "“Make it a brief” turns a ramble into a clear request.",
      "Long answers are read as a short version first. Say “more” for the rest.",
      "Say stop, skip or repeat, or use the big buttons.",
      "Pick the voice that reads to you."],
     [], ["talk", "brief"]),
    ("interview", "Guided interviews", "“Grill me”: decide things one question at a time",
     "Not sure what you want yet? Ask an agent to interview you. It asks one question at a time, with quick answers to tap, and sums up the decisions at the end.",
     ["Question cards right inside the chat.",
      "Skip, go back, pause, or say “decide for me”.",
      "Works by voice too, in English and Portuguese.",
      "A cheaper, faster model tidies up the questions, so interviews stay quick."],
     [], ["grill"]),
    ("on-device", "On-device smarts", "Your iPhone helps, privately",
     "Apple Intelligence on the phone handles the small things instantly, without sending anything anywhere.",
     ["“While you were away”: a short summary of what happened since you last looked.",
      "“What does this do?” explains an approval in plain words before you decide.",
      "Short summaries and suggested replies in Needs you.",
      "A quick first answer, and “Send to …?” when a message fits another agent better.",
      "Replies turned into a few plain sentences for reading aloud."],
     [], ["approval"]),
    ("mac", "Your Mac", "Mission control for your projects",
     "Loft runs quietly in the background on your Mac and you reach it from anywhere on your private network.",
     ["All your repositories at a glance: commits, pull requests, issues and unsaved work.",
      "14 days of activity and spending for every agent.",
      "Your Claude plan usage and the Mac's temperature, live in the header.",
      "Watch and tap the iPhone Simulator from the browser while agents build apps.",
      "Updates itself and starts at login."],
     ["web-repos", "web-phone-chat"], ["overview"]),
]

COMING_SOON = [
    ("A native Mac app", "Loft in your menu bar, starting and stopping by itself."),
    ("Backups", "Agents, tasks, chats and memories, saved and restorable in one step."),
]

PHONE_CAPTIONS = {
    "agents": "Agents on iPhone", "chat": "Plan card in a chat", "mention": "@-mention picker",
    "tasks": "Tasks", "task-detail": "A task's thread", "needs-you": "Needs you", "approval": "What does this do?",
    "talk": "Talk", "brief": "Make it a brief", "grill": "An interview question", "digest": "While you were away",
    "overview": "Plan usage and the Mac's temperature", "settings": "Settings",
}
