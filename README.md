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
- **Memory that lasts.** Each agent keeps long-term memory across conversations and session resets. It remembers your preferences, past decisions and what it did and why.
- **You stay in control.** Approve permission requests, review plans and pull requests, and follow every agent's activity from one dashboard, on your desktop or your phone. **Needs you** (the inbox icon in the top bar, with a live count of what you haven't seen yet on every page) lists everything waiting on you: questions, permissions, stalled Deliveries with the question the agent stopped on, tickets published into a spec whose Delivery is done (with Reopen delivery: the spec PR goes back to draft, the new tickets are built and reviewed, and the final review covers only the new commits), pull requests to merge, a nearly full disk, and push being off. Each item opens the agent's chat or the Delivery card with Resume. Close it with the icon, its X or Esc to get back to the page you were on. Opening it marks what it shows as seen, on the web and the iPhone alike: the counter only comes back for something new, and seen items stay listed, dimmed, until you tick Done. Items also leave by themselves once nothing waits on you: a finished issue's result when you open it (or after a day), an agent's question once you or its Lead answer it, a blocked issue once its GitHub issue closes, and a Delivery stalled on its Lead once the Lead hands the ticket out. A blocked or failed issue shows why it stopped, with Send to… to hand it to another agent, or the same one once the block is cleared: the agent gets the reason with the issue. A Delivery's card shows the GitHub Actions minutes its branch has spent so far (each job rounded up to the minute, as GitHub bills it), and its Delivery done item gives the total. When a ticket starts while its spec PR is ready for review (tickets published after the ready and started without Reopen delivery), Loft puts the PR back to draft, as Reopen does, so workflows that skip drafts stop running the whole CI on every push, and the card says so (or that GitHub refused, and the Delivery carries on); the PR is marked ready again the usual way once the tickets close.
- **Lean by design.** Every agent works in its own clone of your repos, never your checkout. Your setup is left alone, and all of loft's data lives in one folder, `.loft/`.

loft aims to support any and every coding agent: Claude Code, Codex, and whatever comes next.

## Requirements

- [Rust](https://rustup.rs) (`cargo`)
- Node 22 or newer. The web app builds with [pnpm](https://pnpm.io), which the scripts get through corepack (shipped with Node) when it isn't installed, or through `npm exec`.
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
- **Auto-update**: checks the upstream branch every minute and, once no agent is running, pulls, rebuilds and restarts. Needs `cargo` and `node` (and `npm` when pnpm and corepack are missing) on `PATH`.

Each project also has its own **Project settings**, opened with **Settings** in its repo panel (Repos, then click the repo). Each one starts at Loft's default and shows who changed it last and when: you, or the project's Lead, which can change one with its `set_project_setting` tool once you said yes. They're kept in loft's database:

- **CI mode**: `Per ticket` (the default) closes each Delivery ticket only once its CI is green; `At the end` closes tickets on their reviewers and checks the CI once the spec PR is ready, where a red CI is a final round for the Dev. It only changes what Loft waits for, never which workflows GitHub runs.
- **Base branch**: a picker with the repo's branches on GitHub. The next spec PRs and standalone tickets start from it and merge into it; it starts at the repo's default branch, and Deliveries already under way keep the base they have.
- **Ticket reviewers**: one row per reviewer role (QA, Security, Code reviewer) with the team's agent in it, its sticker and a switch for "reviews each ticket". QA and Security are on by default. The Code reviewer always does the final review of the whole spec PR; switched on, it also reviews each ticket. A Delivery waits only for the reviewers switched on. A role with no agent shows dimmed with **Create**, which asks Loft's own agent to set one up.
- **Rounds of changes**: 1 to 5 (3 by default), the rounds of changes a Delivery ticket, or the final review, gets before it waits for you. A change counts from the next round on.
- **CI grace**: how long a commit with no check at all waits for one when the repo has workflows (10 minutes by default, up to 120), before Loft takes it as no CI ran and carries on.

### Use it from other devices

loft is a web app on port `7420`, so it works with whatever network setup you already have: a reverse proxy, a VPN, or your local network. Set **SSH host** to this machine's name so `/code` opens VS Code over Remote-SSH.

An easy option is [Tailscale](https://tailscale.com). Turn on HTTPS certificates in the Tailscale admin console, then run:

```sh
tailscale serve --bg 7420
```

With Tailscale, dev servers that agents start are linked by this machine's tailnet name instead of `localhost`.

The `ios/` folder has a native iPhone app for agent status, approvals, tasks and chat, over the same address. See [ios/README.md](ios/README.md). Its push alerts need a relay key (Settings → Push notifications → Sign in) or your own APNs key: with neither, the web app's Needs you and Settings say push is off, and the backend logs once that it's dropping alerts.

## How agents run

Each agent runs headless Claude Code in its own folder under `~/.local/share/loft/<checkout>-<hash>/agent-local-storage/` (`$XDG_DATA_HOME` when set), outside any repo; an older install moves its workspaces there from the checkout on start ([ADR 0005](docs/adr/0005-workspaces-outside-the-checkout.md)). Agents clone your repos instead of editing them in place, and your personal Claude Code settings, hooks and MCP servers aren't passed to them. loft gives each agent a small set of tools of its own (memory, status, permission requests, messaging other agents, sharing pull requests) through a local MCP server.

A message you send while the agent is running waits in the queue until the run ends, and you can still cancel it there. **Send now** on one of your queued messages delivers it into the run in progress instead, like Ctrl+Enter in the terminal: the agent answers it in that run, even while a `/goal` or Stop hook keeps the run going, and it can't be cancelled any more. When a `/goal` or Stop hook sends the agent back to work, Loft does this by itself for your queued messages, since an answer from you is usually what the run waits for. Messages from other agents and Delivery steps always wait in the queue.

Each time a `/goal` or Stop hook sends the agent back to work, the chat says why in a system line ("Goal not met: …"). While the agent's Session has a goal, its header shows **Goal active** (hover for the condition) and the agents list marks it; the chip's ✕ sends `/goal clear`, into the run in progress if there is one.

A run that takes 5 turns in a row without calling a tool, as when a `/goal` or Stop hook keeps sending the agent back with the same answer, is stopped, the chat says why, and the next queued message runs.

## Development

Run `./dev.sh` for hot reload and open http://localhost:7421. `LOFT_PORT` changes the backend port (default `7420`).

## License

MIT — see [LICENSE](LICENSE). Free to use, modify, and sell; just keep the copyright notice.
