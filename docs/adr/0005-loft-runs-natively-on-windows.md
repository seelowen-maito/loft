# Loft runs natively on Windows, with Git Bash, not inside WSL2

Status: accepted (2026-11-18)

## Context
Loft ran on macOS and Linux only: the backend used Unix sessions (`setsid`), signals, `ps`, `statvfs`, `exec` and file modes without any `cfg` gate, so it didn't even compile on Windows. The user wants to run Loft on Windows the way they run it on the Mac: `setup.sh` or `start.sh` builds and starts the backend, and Loft is used in the browser (and from the iPhone over the network). WSL2 would run the Linux build unchanged, but repos must live inside the Linux filesystem (`/mnt/c` is slow), Claude Code needs a second login inside the distro, and WSL2 shuts down when idle. Claude Code itself runs natively on Windows, and its Bash tool uses Git Bash.

## Decision
The backend runs directly on Windows (x64, Windows 10 22H2 and 11), with the agents calling the native `claude`. Git for Windows is required: the same `setup.sh`, `start.sh` and one-line `install.sh` run in Git Bash, with no PowerShell copies, and agents use only the Bash tool (Git Bash); Loft turns Claude Code's PowerShell tool off for them so the kill guard sees every command. Each agent's processes live in a Windows Job Object, which replaces the Unix process session: the agent stops its own processes freely, stopping anything else needs the user's approval, and stopping an agent ends its whole job at once (Windows has no graceful SIGTERM). Mac-only features (Simulator, voice, Mac access, opening Terminal to log in, Herdr setup) are hidden outside the Mac. This first port (V1) has no packaged app or installer. Because Windows has no `exec` and locks a running binary, Loft restarts there by exiting with a restart code instead of replacing itself: `start.sh` (on every OS) and a Task Scheduler logon task (Windows' launchd, offered by `setup.sh`) start it again. Both tell Loft so (`LOFT_RESTART_BY_EXIT=1`), and under them it exits that way on every OS; a Loft started otherwise on the Mac or Linux still replaces itself with `exec`, so an older `start.sh` (which had no restart loop) or the launchd service keeps working across the upgrade. Auto-update renames the running binary aside to a single fixed name before building and deletes it on the next start.

Rejected: "Windows support = run the Linux build in WSL2" (it stays documented as an alternative, not the supported path); a packaged desktop app or `install.ps1` (left for a later grill); PowerShell versions of the scripts.

## Consequences
- The backend carries `cfg(windows)` code for processes, kills, disk space, file permissions and paths, and CI must build and test it on a Windows runner: the team's agents run on a Mac and can't prove it otherwise.
- Git for Windows becomes a hard requirement on Windows.
- Stopping an agent on Windows is a hard kill: its shells and dev servers get no chance to clean up.
- Herdr, voice and the packaged app stay out of V1.
