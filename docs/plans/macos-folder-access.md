# Plan: macOS folder access on a screenless Mac

Status: proposal, for discussion. Nothing here is built except what "Where we are" says.

## Where we are

Loft runs on a Mac mini with no screen, started by launchd (`dev.loft`): `loft` starts `claude`, which starts the tools an agent uses. When an agent touches a protected folder (Desktop, Documents, Downloads, iCloud Drive, removable or network volumes), macOS shows a permission dialog **on the mini's screen**. Nobody is there to click it, and the agent waits. Loft's only signal today is the generic "may be stuck" item in Needs you, which says nothing about why.

Checked on the mini (by Loft Dev, nothing was triggered):

- The permission belongs to the **`loft` binary**, not to `claude` or the shell. The dialog reads "**loft** would like to access files in your Desktop folder".
- `loft` is **ad-hoc, linker-signed** (no team). macOS can only remember such a binary by its code hash, and the updater rebuilds it on every update (`cargo build --release`), so **every grant, Full Disk Access included, is lost on every update** and the prompts come back.
- There is no Full Disk Access today.
- Loft can't time out the reads: they happen inside `claude`.
- Not decided yet: the call waits on the dialog. Denied: it fails at once with "Operation not permitted". Allowed: it works.

## The problem

Two separate things: (1) the first grant needs a click on the Mac, which is awkward on a screenless one; (2) the grant doesn't survive an update, so (1) repeats. Fixing only (1) buys days; fixing (2) is what makes it stick.

## What cannot be done

- **No programmatic grant.** No API lets a process, or the web app, give itself folder access.
- **No configuration profile without MDM.** Pre-approving access (PPPC) needs the Mac to be managed.
- Only a click (in person or over Screen Sharing) or a change in System Settings grants it.

## Options

| | What | Fixes | Size | Needs |
|---|---|---|---|---|
| A | Stable signing identity for the release build, then one Full Disk Access grant | the grant surviving updates | M | a signing identity, updater change |
| B | Settings → "Mac access" check | seeing what's allowed, getting the dialogs while you're connected | S | nothing |
| C | Needs you item from the macOS permission log | knowing *why* an agent is stuck | M | a check on screen first |
| D | Keep agents out of protected folders | most prompts, for free | S | a convention |
| E | MDM profile | pre-approval | L | a managed Mac |

**A. Stable signing identity.** After each build the updater signs the binary with the same identity and identifier (`codesign -s <identity> --identifier dev.loft`). macOS then recognises it by that identity, not its hash, so one grant lasts.
- *Apple Developer certificate (Development / Developer ID):* already exists for the iOS app, trusted by macOS, no keychain tricks. Cost: ties the Mac's updater to an Apple account, the cert expires and must be renewed, and the key must be available to the launchd session (keychain access on a screenless Mac).
- *One-time self-signed local certificate:* free, no account, no expiry worry for years. Cost: only valid on that Mac, someone must create and trust it once, and it isn't a real identity for anyone else running Loft.
- *With TestFlight signing:* they are separate: the iOS app is signed by Xcode with the team profile. This signs only the macOS `loft` binary, so they don't interact, but whichever Apple cert is used shouldn't be the one used to ship the app unless that is the intent.
- *With the updater:* one new step after `cargo build --release` and before the restart; if signing fails the updater should keep the old binary and report it, never start an unsigned one silently.
- Then grant Full Disk Access to the Loft binary once over Screen Sharing.

**B. "Mac access" check in Settings.** A button, a fixed list of folders (Desktop, Documents, Downloads, iCloud Drive, plus Full Disk Access), listing each in a child process with a 3 s timeout. Each shows *allowed*, *blocked*, *waiting* (a dialog is up on the Mac now) or *not there*, with plain steps and links: Screen Sharing to the mini, the Privacy pane (`x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`), and the path of the running binary to add. Pressing it while on Screen Sharing makes the dialogs appear while you're there to click them. Loft Dev has a draft PR for this.

**C. Needs you from the permission log.** `log stream --predicate 'subsystem == "com.apple.TCC"'` shows who is asking, with no root. A request for a files-and-folders service with no result after about 10 s means a dialog is waiting. That becomes a Needs you item: "macOS is asking for access to <folder> on <mac>", with the Screen Sharing link and *Check again* (re-runs B). It can't unblock the stuck call; only the click or Stop can. Needs a small on-screen verification that request and result lines pair up for folder services.

**D. Keep agents out of protected folders.** `~/dev` is not protected and needs nothing. Desktop, Documents, Downloads, iCloud Drive and external or network volumes prompt. Cheapest option: put work there and say so in agent instructions.

**E. MDM.** Would pre-approve everything, but only if the Mac is managed. Rejected unless someone already runs MDM.

Rejected: granting Full Disk Access to `/bin/sh` or Terminal so launchd-started tools inherit it. Far too broad.

## Recommendation

Do D now (free), build B (already underway), then A with the self-signed local certificate unless someone prefers the Apple one, and C last. A is what ends the repeat prompts; B and C make the remaining first-time cases visible and quick.

## Phases

1. **B, Mac access check (S).** Backend `GET /api/system/mac-access?check=1` and a Settings card; API tests with a fake slow folder. In progress as a draft PR by Loft Dev.
2. **D, convention (S).** A line in the agent instructions / README about where to work.
3. **A, stable signing in the updater (M).** After the signing identity is chosen. Then one Full Disk Access grant.
4. **C, Needs you item from the log (M).** After the on-screen verification.

## Security notes

- The check reads **no contents**: it lists a folder from a child process whose output goes to `/dev/null`, and only the exit status is used. No names or contents are returned.
- The folder list is **fixed** in the code; the request can't name a path.
- It runs only when the user presses Check, because listing a folder is what triggers the dialog (and an unanswered or denied dialog records a "deny" for `loft`).
- Full Disk Access to `loft` is broad: it is the one grant the signing step makes durable, so the identity's private key matters.
- Signing must fail closed (keep the old binary) in the updater.

## Open questions

1. **Signing identity (A):** Apple Developer cert or a one-time local self-signed cert? Who holds the key?
2. **Updater (A):** OK to add a signing step, and to refuse to start an unsigned build?
3. **Managed Mac:** does anyone run Loft on a Mac under MDM? If so, E becomes the simplest answer for that Mac.
4. **Folders agents really need:** do any of your agents work outside `~/dev` (Desktop, Documents, Downloads, iCloud, external drives)? If not, D alone removes most prompts.
5. **Other hosts:** does anything here matter for Linux or other hosts (the check and the log reader are macOS-only and should simply be absent there)?
