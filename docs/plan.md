# Withings V1 goal and acceptance ledger

## Goal

Standalone Niles add-on for Sleep mat **in-bed / out-of-bed** state and automation events. In-bed is not asleep. History/analysis is V2; Body+ measurements are secondary and use a separate notification/data path.

## Current status

Research/design reviewed by Main; no production implementation or deployment. Worker drafts were corrected against first-party notification categories and subscription documentation. Core request is prepared but not sent. Nothing is currently running.

## Acceptance

- [x] Official bed event categories, scope, example payloads, POST transport, HEAD validation, general retry and latency documented.
- [x] Pinned HA reference confirms direct bed events with initially unknown state and no polling fallback.
- [x] Inspected Niles host gap identified: incoming callbacks and secure runtime token persistence.
- [ ] Mark agrees/provides the generic host contract; compatibility tested.
- [ ] Withings developer app, actual account eligibility and least-scope OAuth consent proved.
- [ ] Synthetic parser/state-machine tests: profile/device isolation, malformed fields, duplicate/reordered/conflicting/old events, restart, unknown never out-of-bed.
- [ ] Callback → state/action → actual Niles automation verified offline without household writes.
- [ ] Separately approved public route/registration and observed bed-in/out proof, including actual latency and restart behavior.

## Decisions and safety

Add-on owns Withings logic. Core only supplies generic facilities. No custom Niles fork or sidecar assumed. Do not persist tokens in ordinary KV without confirming encrypted storage. No login, account/provider writes, service changes or publishing performed. No guarantee of signed callback authentication or instantaneous delivery established. Do not apply arbitrary occupied-state expiration: silence during sleep is normal.

## Next gate

Review/send `niles-core-request.md` to Mark and agree the generic callback/token-storage contract. Meanwhile a provider-independent parser/state-machine package is possible, but its Niles adapter must not target an invented WIT API. Actual user/device/transport proof remains separately gated. OAuth scope for V1 bed events is `user.sleepevents`; optional scale needs `user.metrics`. V2 sleep-history scope spelling must be checked before use.
