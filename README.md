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
- **Plans made with you.** Switch on Planning for an agent and start a grill with `/grill` or "grill me". It looks the facts up itself, then asks in rounds: every question it can ask now, each with a recommendation you accept with "yes". On each question you can also answer differently, leave it to the agent ("Decide for me"), or add a comment to discuss instead of deciding. It runs again only when you send the round; what you leave unanswered comes back in the next round. Pause, resume or end it from the round bar or by typing "pause", "resume" or "stop"; an idle grill pauses after 30 minutes and ends with a summary after a day. An open round shows in Needs you and is pushed to your phone. The iPhone app shows rounds as the same cards; by voice it reads one question at a time with its recommendation, "yes" accepts it, "skip", "back", "repeat", "pause" and "I don't know" work as you go, and the round is sent after the last question. While it grills a project with a repo, it writes the terms you settle into `GLOSSARY.md` and offers an ADR in `docs/adr/` only for decisions that are hard to reverse, surprising and a real trade-off. It does this in its own clone on a `docs/<topic>` branch and opens one PR when the grill ends, shown as a PR card in the chat (adding a short `CLAUDE.md` that points at both if the repo has none). A grill that settled nothing opens no PR. When every branch is settled, its last round offers to turn the grill into a spec: it asks you how the work will be tested, then Loft publishes the spec as a GitHub issue on the project's repo, labelled `spec` and assigned to you, creating the `spec` and `ready-for-agent` labels if they're missing. Publishing again never creates a second spec, and a missing GitHub remote or token, or a GitHub refusal, is explained in the chat.
- **Memory that lasts.** Each agent keeps long-term memory across conversations and session resets. It remembers your preferences, past decisions and what it did and why.
- **You stay in control.** Approve permission requests, review plans and pull requests, and follow every agent's activity from one dashboard, on your desktop or your phone.
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
