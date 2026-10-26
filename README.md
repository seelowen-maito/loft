# loft

**Where your coding agents hang out.**

loft is a small layer that sits on top of the coding agent you already use. It starts agents for you, keeps them working on a schedule, lets them talk to each other, and gives them memory that lasts. You watch and steer from the outside.

It stays out of the way. loft doesn't replace your agent, your prompts or your tools. It runs the agent you have and gives you one place to see what every agent is doing.

loft is built for engineers who already code with AI and want to try autonomous agents without losing control. Setting agents up is easy, and you decide how much they can do on their own.

## Why loft

- **Agents that run without you.** Create an agent, give it a repo and standing instructions, and it checks in on its own schedule. Quiet hours keep background runs out of your working day.
- **Agents that work together.** Agents can see each other, read each other's chats and send each other messages, so one can hand work to another or ask it for help.
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

```sh
./setup.sh
```

The script checks the requirements and tells you how to fix anything missing, asks where your git repos live, and builds loft. On macOS it can also install loft as a background service that starts at login. It's safe to run again at any time, for example after you move Node or Claude Code.

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
