import Foundation

/// `GET /api/needs-you` for the fixture server: the backend's own fixture (docs/fixtures/needs-you.json in #91, its grill
/// item as of rounds),
/// with the fixture's agents, a stuck item and longer texts for the summaries.
enum FixtureNeedsYou {
    static let json = #"""
{
 "count": 9,
 "generated_at": "2026-10-23T14:37:36.766739+00:00",
 "items": [
  {
   "actions": [
    {
     "body": {
      "decision": "allow_once"
     },
     "kind": "approve",
     "label": "Allow once",
     "method": "POST",
     "path": "/api/permissions/p1"
    },
    {
     "body": {
      "decision": "allow_always",
      "rule": "Bash(git push *)"
     },
     "kind": "approve_always",
     "label": "Always allow",
     "method": "POST",
     "path": "/api/permissions/p1"
    },
    {
     "body": {
      "decision": "deny"
     },
     "kind": "deny",
     "label": "Deny",
     "method": "POST",
     "path": "/api/permissions/p1"
    },
    {
     "kind": "open",
     "label": "Open chat",
     "target": {
      "agent_id": "be",
      "screen": "chat"
     }
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "be",
    "name": "Backend Agent"
   },
   "created_at": "2026-10-23T14:37:36.763087+00:00",
   "id": "permission:p1",
   "links": [],
   "reason": "Waiting for your approval.",
   "task_id": null,
   "text": "Push the fix\n$ git push https://x:[redacted]@github.com/o/r",
   "title": "Backend Agent wants to use Bash",
   "type": "permission",
   "urgency": 3
  },
  {
   "actions": [
    {
     "body": {
      "send": true,
      "text": ""
     },
     "kind": "answer",
     "label": "Reply on the task",
     "method": "POST",
     "path": "/api/tasks/9768b326-df66-4876-803e-41f8a4317202/comments"
    },
    {
     "kind": "open",
     "label": "Open task",
     "target": {
      "screen": "task",
      "task_id": "9768b326-df66-4876-803e-41f8a4317202"
     }
    },
    {
     "body": {},
     "kind": "dismiss",
     "label": "Dismiss",
     "method": "POST",
     "path": "/api/needs-you/task%3A9768b326-df66-4876-803e-41f8a4317202%3Ablocked/dismiss"
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "qa",
    "name": "QA Agent"
   },
   "created_at": "2026-10-23T14:37:36.763243+00:00",
   "id": "task:9768b326-df66-4876-803e-41f8a4317202:blocked",
   "links": [
    {
     "label": "Task",
     "path": "/tasks/9768b326-df66-4876-803e-41f8a4317202"
    }
   ],
   "reason": "Blocked: Which simulator should I use?",
   "task_id": "9768b326-df66-4876-803e-41f8a4317202",
   "text": "Blocked: Which simulator should I use? The suite needs iOS 26, but the only booted one is iPhone 16 on iOS 18.6. I can boot the iPhone 17 Pro (iOS 26.5) or create a new one.",
   "title": "Run the suite",
   "type": "task_blocked",
   "urgency": 2
  },
  {
   "actions": [
    {
     "body": {
      "text": ""
     },
     "kind": "answer",
     "label": "Reply",
     "method": "POST",
     "path": "/api/agents/be/messages"
    },
    {
     "kind": "open",
     "label": "Open chat",
     "target": {
      "agent_id": "be",
      "screen": "chat"
     }
    },
    {
     "body": {},
     "kind": "dismiss",
     "label": "Dismiss",
     "method": "POST",
     "path": "/api/needs-you/question%3A1/dismiss"
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "be",
    "name": "Backend Agent"
   },
   "created_at": "2026-10-23T14:37:36.763825+00:00",
   "id": "question:1",
   "links": [],
   "reason": "Should I use B2 or S3?",
   "task_id": null,
   "text": "I looked at the backup options for the Mac mini. Backblaze B2 costs about $1 a month for our size and restic supports it directly; S3 is about $3 and we'd need a lifecycle rule for old snapshots. Both encrypt at rest. Should I use B2 or S3?",
   "title": "Backend Agent asked you something",
   "type": "question",
   "urgency": 2
  },
  {
   "actions": [
    {
     "kind": "open",
     "label": "Open chat",
     "target": {
      "agent_id": "orch",
      "screen": "chat"
     }
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "orch",
    "name": "Orchestrator"
   },
   "created_at": "2026-10-23T14:37:36.764637+00:00",
   "id": "grill:e877de29-96a2-4220-be37-8f2b75b4a0e6:1",
   "links": [],
   "reason": "Round 1: 0 of 1 answered",
   "task_id": null,
   "text": "• Who pays?",
   "title": "Orchestrator is interviewing you: Pricing",
   "type": "grill_round",
   "urgency": 2
  },
  {
   "id": "stuck:be:1",
   "type": "stuck",
   "urgency": 2,
   "agent": {
    "id": "be",
    "name": "Backend Agent"
   },
   "task_id": null,
   "title": "Backend Agent may be stuck",
   "reason": "Waiting on a background task for 14 min: cargo test --release",
   "text": "Waiting on: cargo test --release",
   "created_at": "2026-10-23T14:37:36.763087+00:00",
   "age_seconds": 840,
   "links": [],
   "actions": [
    {
     "kind": "restart",
     "label": "Restart (keeps queued messages)",
     "method": "POST",
     "path": "/api/agents/be/restart",
     "body": {}
    },
    {
     "kind": "open",
     "label": "Open chat",
     "target": {
      "screen": "chat",
      "agent_id": "be"
     }
    },
    {
     "kind": "dismiss",
     "label": "Dismiss",
     "method": "POST",
     "path": "/api/needs-you/stuck%3Abe%3A1/dismiss",
     "body": {}
    }
   ]
  },
  {
   "actions": [
    {
     "body": {
      "send": true,
      "text": ""
     },
     "kind": "answer",
     "label": "Reply on the task",
     "method": "POST",
     "path": "/api/tasks/dbc521e2-042c-4485-a15d-956c162919ed/comments"
    },
    {
     "body": {
      "reopen": true,
      "send": true,
      "text": ""
     },
     "kind": "retry",
     "label": "Reopen and send",
     "method": "POST",
     "path": "/api/tasks/dbc521e2-042c-4485-a15d-956c162919ed/comments"
    },
    {
     "kind": "open",
     "label": "Open task",
     "target": {
      "screen": "task",
      "task_id": "dbc521e2-042c-4485-a15d-956c162919ed"
     }
    },
    {
     "body": {},
     "kind": "dismiss",
     "label": "Dismiss",
     "method": "POST",
     "path": "/api/needs-you/task%3Adbc521e2-042c-4485-a15d-956c162919ed%3Afailed/dismiss"
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "qa",
    "name": "QA Agent"
   },
   "created_at": "2026-10-23T14:37:36.763444+00:00",
   "id": "task:dbc521e2-042c-4485-a15d-956c162919ed:failed",
   "links": [
    {
     "label": "Task",
     "path": "/tasks/dbc521e2-042c-4485-a15d-956c162919ed"
    }
   ],
   "reason": "Failed: Xcode 27 missing",
   "task_id": "dbc521e2-042c-4485-a15d-956c162919ed",
   "text": "Xcode 27 missing\nmore lines",
   "title": "Fix the build",
   "type": "task_failed",
   "urgency": 1
  },
  {
   "actions": [
    {
     "body": {
      "method": "squash"
     },
     "kind": "merge",
     "label": "Merge",
     "method": "POST",
     "path": "/api/pull-requests/pr1/merge"
    },
    {
     "kind": "open",
     "label": "Open on GitHub",
     "target": {
      "url": "https://github.com/o/r/pull/7"
     }
    },
    {
     "body": {},
     "kind": "dismiss",
     "label": "Dismiss",
     "method": "POST",
     "path": "/api/needs-you/pr%3Apr1%3Aabc/dismiss"
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "be",
    "name": "Backend Agent"
   },
   "created_at": "2026-10-23T14:37:36.765073+00:00",
   "id": "pr:pr1:abc",
   "links": [
    {
     "label": "Pull request",
     "url": "https://github.com/o/r/pull/7"
    }
   ],
   "reason": "Ready to merge, 3 checks passed.",
   "task_id": null,
   "text": "→  · +0 −0 in 0 files",
   "title": "o/r#7: Fix it",
   "type": "pr_ready",
   "urgency": 1
  },
  {
   "actions": [
    {"body": {}, "kind": "approve", "label": "Approve", "method": "POST", "path": "/api/issues/t1/comments/77/approve"},
    {"body": {"text": "Backend is done: 4 endpoints with tests. iOS screen next."}, "kind": "answer", "label": "Edit and approve", "method": "POST", "path": "/api/issues/t1/comments/77/approve"},
    {"body": {}, "kind": "deny", "label": "Keep in Loft", "method": "POST", "path": "/api/issues/t1/comments/77/keep"},
    {"kind": "open", "label": "Open task", "target": {"screen": "task", "task_id": "t1"}}
   ],
   "age_seconds": 120,
   "agent": {"id": "be", "name": "Backend Agent"},
   "created_at": "2026-10-23T14:35:36.763574+00:00",
   "id": "comment:77",
   "links": [{"label": "Task", "path": "/tasks/t1"}],
   "project": "loft",
   "reason": "Not on GitHub yet: approve to post it on octo/loft#42.",
   "task_id": "t1",
   "text": "Backend is done: 4 endpoints with tests. iOS screen next.",
   "title": "Backend Agent's comment on Implement settings functionality",
   "type": "comment_approval",
   "urgency": 1
  },
  {
   "actions": [
    {
     "body": {
      "reopen": true,
      "send": true,
      "text": ""
     },
     "kind": "answer",
     "label": "Follow up",
     "method": "POST",
     "path": "/api/tasks/c835a7f2-343c-414d-8165-cf09eb16575b/comments"
    },
    {
     "kind": "open",
     "label": "Open task",
     "target": {
      "screen": "task",
      "task_id": "c835a7f2-343c-414d-8165-cf09eb16575b"
     }
    },
    {
     "body": {},
     "kind": "dismiss",
     "label": "Dismiss",
     "method": "POST",
     "path": "/api/needs-you/done%3Ac835a7f2-343c-414d-8165-cf09eb16575b%3A2026-10-23T14%3A37%3A36.763574%2B00%3A00/dismiss"
    }
   ],
   "age_seconds": 0,
   "agent": {
    "id": "qa",
    "name": "QA Agent"
   },
   "created_at": "2026-10-23T14:37:36.763574+00:00",
   "id": "done:c835a7f2-343c-414d-8165-cf09eb16575b:2026-10-23T14:37:36.763574+00:00",
   "links": [
    {
     "label": "Task",
     "path": "/tasks/c835a7f2-343c-414d-8165-cf09eb16575b"
    }
   ],
   "reason": "Done: 15 GB after cleanup.",
   "task_id": "c835a7f2-343c-414d-8165-cf09eb16575b",
   "text": "**15 GB** after cleanup.",
   "title": "Measure sizes",
   "type": "task_done",
   "urgency": 0
  }
 ],
 "urgent": 4,
 "version": 1
}
"""#
}
