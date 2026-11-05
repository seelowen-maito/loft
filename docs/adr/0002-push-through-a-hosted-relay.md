# Other people's installs push through a hosted relay that holds the APNs key

Status: accepted (2026-11-04)

## Context
Loft signs its pushes itself with an APNs auth key (`.p8`) stored on the install (`docs/push.md`). That works for the person who owns the Apple developer account, but the app is now on TestFlight and other people run their own Loft: the key can't be given to them, and it works for every app on the Apple team. TestFlight builds use the production APNs environment, so one key and one host serve all of them.

## Decision
A hosted **push relay** holds the key and forwards pushes to Apple. It only forwards: which alerts a phone wants, quiet hours and the rate limit stay in the install. There is no sign-in: an install gets a **relay token** the first time it uses the relay, and that token is its identity. The phone keeps registering only with its own Loft; Loft sends the phone's token with every push request, so the relay stores no devices, only hashed relay tokens and counters. The relay forwards the same names-and-titles payload as today and never stores or logs it. It serves only the official Loft app's bundle id. An install with its own `.p8` keeps sending directly to Apple, and with none it uses the relay by default. The relay is Vercel functions with a small key-value store, in a `relay/` folder of this repo. Revocation is manual.

Rejected: shipping the key (impossible to keep secret); sign-in with Apple or email (login, recovery and account screens for no v1 benefit); pairing phones to the relay by QR (a second registration path and a device store to protect); an opaque "something happened" push that the app resolves through the user's own Loft (needs a notification service extension and a reachable Loft).

## Consequences
- Push works out of the box on TestFlight, and no one but the operator holds the key.
- Every relay user's push text (agent names and titles) passes through the operator's service, in transit only. Anyone who minds adds their own key.
- The relay is a single point of failure for installs that use it; installs with their own key are unaffected.
- A lost token means a new token: there is no recovery, and no self-service rotation.
- The push contract exists on both sides, in the relay and in the Loft backend, and changes together.
