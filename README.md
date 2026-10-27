<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="site/readme/loft-dark.svg">
    <img src="site/readme/loft-light.svg" alt="loft" width="280">
  </picture>
</p>

<h3 align="center">Where your coding agents hang out.</h3>

<p align="center">
  <img src="site/avatars/reviewer.svg" alt="" width="44">
  <img src="site/avatars/builder.svg" alt="" width="44">
  <img src="site/avatars/triage.svg" alt="" width="44">
  <img src="site/avatars/docs.svg" alt="" width="44">
</p>

<p align="center">
  <a href="https://seelowen-maito.github.io/loft/">Website</a> ·
  <a href="#install">Install</a> ·
  <a href="#why-loft">Why loft</a>
</p>

**One-line installer**

```sh
curl -fsSL https://seelowen-maito.github.io/loft/install.sh | bash
```

---

loft is a small layer that sits on top of the coding agent you already use. It starts agents for you, keeps them working on a schedule, lets them talk to each other, and gives them memory that lasts. You watch and steer from the outside.

It stays out of the way. loft doesn't replace your agent, your prompts or your tools. It runs the agent you have and gives you one place to see what every agent is doing.

loft is built for engineers who already code with AI and want to try autonomous agents without losing control. Setting agents up is easy, and you decide how much they can do on their own.

## Why loft

- **Agents that run without you.** Create an agent, give it a repo and standing instructions, and it checks in on its own schedule. Quiet hours keep background runs out of your working day.
- **Agents that work together.** Agents can see each other, read each other's chats and send each other messages, so one can hand work to another or ask it for help.
- **Plans made with you.** Switch on Planning for an agent and start a grill with `/grill` or "grill me". It reads the project's existing documents and looks the other facts up itself, then asks in rounds: every question it can ask now, each with a recommendation you accept with "yes". On each question you can also answer differently, leave it to the agent ("Decide for me"), or add a comment to discuss instead of deciding. It runs again only when you send the round; what you leave unanswered comes back in the next round. Pause, resume or end it from the round bar or by typing "pause", "resume" or "stop"; an idle grill pauses after 30 minutes and ends with a summary after a day. An open round shows in Needs you and is pushed to your phone. The iPhone app shows rounds as the same cards, and text you type in the composer is a chat message, not an answer, as on the web; by voice it reads one question at a time with its recommendation, in the language you started the grill in (English or Portuguese), "yes" accepts it, "skip", "back", "repeat", "pause" and "I don't know" work as you go, and the round is sent after the last question. While it grills a project with a repo, it writes the terms you settle into `GLOSSARY.md` and offers an ADR in `docs/adr/` only for decisions that are hard to reverse, surprising and a real trade-off. It does this in its own clone on a `docs/<topic>` branch and opens one PR when the grill ends, shown as a PR card in the chat (adding a short `CLAUDE.md` that points at both if the repo has none). A grill that settled nothing opens no PR. When every branch is settled, its last round offers to turn the grill into a spec: it asks you how the work will be tested, then Loft publishes the spec as a GitHub issue on the project's repo, labelled `spec` and assigned to you, creating the `spec` and `ready-for-agent` labels if they're missing. It then splits the spec into tickets, thin slices that each work end to end, and asks "Publish these N tickets?" with each ticket's title, what it delivers and what blocks it. After your yes, Loft creates them blockers first as sub-issues of the spec, labelled `ready-for-agent`, assigned to you, with blockers linked as GitHub issue dependencies. The grill then ends with one card, on the web and on the iPhone: links to the spec and tickets (they open GitHub), a **Clear context** button that starts a fresh session and leaves "Implement spec #N: hand the ready tickets to the team" in your composer to edit and send yourself, and, when no agent in the project has the Dev or Reviewer team role, a **Set up the team with loft** button that sends `loft` the project and the spec link. It sends once, whether you tap it on the web or the iPhone: the card then shows "Sent to loft" on both, also after a reload or an app restart. An agent's team role comes from the template it was made from (Dev; Reviewer for code review, QA and security) and can be changed under Team role in its Config tab; agents from before team roles get one from their name and title once. Planning agents never create agents themselves. If the session's context nears the end of the smart zone before the tickets are published, the agent suggests `/compact`. On a project folder with no git repo or GitHub remote yet, the first round also offers to create the GitHub repo and publish the folder, listing the files and total size of the first commit, with your account and the organizations you can create repos in as owners and private recommended. The repo goes under the owner your answer names, or else the one in the question, and stays private unless you write in your own words that you want it public. Only after your yes does Loft itself add a `.gitignore` for `.DS_Store` and `.remember/`, run `git init`, commit the rest, create the repo and push; agents never write into the project folder. Say no and the grill goes on, asking again before the spec is published. Planning's rules for how specs and tickets are written, labelled, linked and published take precedence over an agent's own instructions; those still set its role, team and how work gets built. Without a grill, ask a planning agent to "create an issue for X": one small change becomes a single ticket straight away, and anything bigger becomes a spec and tickets published only after your yes. It never starts a grill on its own, but suggests one when the request has many open decisions. Publishing again never creates a second spec or ticket, and a missing GitHub remote or token, or a GitHub refusal, is explained in the chat. Issues any agent creates some other way are assigned to you too, unless you tell it to assign no one or someone else.
- **Your issues in one place.** The Issues tab lists each project's GitHub issues, grouped by project like the agents and read live from the repo in the project folder's `origin`: state, labels in their colors, assignees and the Markdown body, a spec's sub-issues, what blocks a ticket and what it blocks, and a link to each issue on GitHub. It tells you when GitHub isn't connected or a project has no GitHub remote, and refreshes only that project when Loft publishes a spec or tickets on it. Pull requests never count against it, so a PR-heavy repo loads as fast as any other; a repo with more than 1,000 issues shows the first 1,000, open ones first, with a link to the rest on GitHub. Click a project's header to fold its issues away, the same as a project's agents; a folded project still shows its name and issue count, and the browser remembers it. **Send to agent** on an issue lists the agents whose project is on that issue's repo; picking one posts the issue's number and link in its chat and wakes it. The tab never changes an issue: labels, assignees and everything else change on GitHub.
- **Memory that lasts.** Each agent keeps long-term memory across conversations and session resets. It remembers your preferences, past decisions and what it did and why.
- **You stay in control.** Approve permission requests, review plans and pull requests, and follow every agent's activity from one dashboard, on your desktop or your phone. While an agent works, the chat on the web and on iOS shows how long its run has taken, counting every second, and what it is doing, like `✻ 2m 23s · Reading CI logs`; when the run ends the line reads `✻ Done in 2m 23s · Thu 16:23` until the next one. On the web, the Agents sidebar shows the same ticking timer next to each busy agent, and the Agents tab opens on the last agent you viewed in that browser, even after a reload (or the first agent if that one is gone). The Repos page shows one row of agent cards, most recently active first, as many as fit the screen; **See more (N)** unfolds the rest. Its Open issues list has a **See all →** link that opens the Issues tab.
- **Your Mac at a glance.** The web header shows the CPU temperature and memory used as a share of the total, like `RAM 66%` (active, wired and compressed memory, as Activity Monitor's Memory Used counts it; hover for the GB), sampled every 30 seconds. Memory turns amber at 80 % and red at 90 %.
- **Lean by design.** Every agent works in its own clone of your repos, never your checkout. Your setup is left alone, and all of loft's data lives in one folder, `.loft/`.

loft aims to support any and every coding agent: Claude Code, Codex, and whatever comes next.

## Requirements

- [Rust](https://rustup.rs) (`cargo`)
- Node 22 or newer (`npm`)
- git
- [Claude Code](https://claude.com/claude-code) (`claude`), installed and logged in. Agents use your login.
- Optional: [`gh`](https://cli.github.com), logged in, for GitHub pull requests and issues.

## Install

**One-line installer**

```sh
curl -fsSL https://seelowen-maito.github.io/loft/install.sh | bash
```

The installer downloads loft to `~/loft` (set `LOFT_DIR` to put it elsewhere), then runs `setup.sh`. Setup checks the requirements and tells you how to fix anything missing, asks where your git repos live, and builds loft. On macOS it can also install loft as a background service that starts at login. Both are safe to run again at any time, for example after you move Node or Claude Code.

Already have a clone? Run `./setup.sh` from it instead.

To run loft in the foreground instead, use `./start.sh`.

Then open http://localhost:7420 and check **Settings** (gear icon, top right). Settings are saved to `.loft/settings.json`:

- **Projects folder**: where your git repos live. Takes effect after a restart, which loft offers once no agent is running.
- **Claude command**: the command agents run.
- **GitHub token**: needed for private repos and merging pull requests. Without one, loft uses `gh auth token`.
- **SSH host**: when loft runs on another machine, `/code` opens an agent's workspace in your local VS Code over Remote-SSH to this host.
- **Auto-update**: checks the upstream branch every minute and, once no agent is running, pulls, rebuilds and restarts. Needs `npm` and `cargo` on `PATH`.

### Use it from other devices

loft is a web app on port `7420`, so it works with whatever network setup you already have: a reverse proxy, a VPN, or your local network. Set **SSH host** to this machine's name so `/code` opens VS Code over Remote-SSH.

An easy option is [Tailscale](https://tailscale.com). Turn on HTTPS certificates in the Tailscale admin console, then run:

```sh
tailscale serve --bg 7420
```

With Tailscale, dev servers that agents start are linked by this machine's tailnet name instead of `localhost`.

The `ios/` folder has a native iPhone app for agent status, approvals, tasks and chat, over the same address. See [ios/README.md](ios/README.md).

## How agents run

Each agent runs headless Claude Code in its own folder under `agent-local-storage/`. Agents clone your repos instead of editing them in place, and your personal Claude Code settings, hooks and MCP servers aren't passed to them. loft gives each agent a small set of tools of its own (memory, status, permission requests, messaging other agents, sharing pull requests) through a local MCP server.

## Development

Run `./dev.sh` for hot reload and open http://localhost:7421. `LOFT_PORT` changes the backend port (default `7420`).

## License

MIT — see [LICENSE](LICENSE). Free to use, modify, and sell; just keep the copyright notice.
