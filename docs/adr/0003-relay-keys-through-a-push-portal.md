# Pushing through the relay needs a relay key from a push portal

Status: accepted (2026-11-04)

## Context
ADR 0002 gave every install an anonymous relay token on its first push, with no sign-in. A lost token could not be recovered, a second Mac could not share an identity, and the user could not revoke a key themselves. The user now wants a login: always.

## Decision
Pushing through the relay always needs a **relay key**, which the user gets by signing in with Apple on a **push portal**: a small web page on the relay's Vercel project. The portal stores only Apple's stable user id and hashed keys with their names and dates (no email, no name). A user can create several keys, revoke them, and use the same identity again after a reinstall or on a second Mac. Anonymous relay tokens are dropped. An install with its own `.p8` still sends directly to Apple. This supersedes the "no sign-in" and "relay token" parts of ADR 0002; the rest of it (the relay only forwards, nothing stored or logged, the official bundle id only) stands.

## Consequences
- Push through the relay is no longer zero-setup: the user signs in once.
- Reinstalling Loft or adding a Mac no longer loses the identity.
- The relay now stores accounts (Apple ids), so it needs account deletion and a privacy answer in the App Store listing.
