# Withings V1 goal and acceptance ledger

## Goal
Standalone Niles add-on for Sleep mat in-bed / out-of-bed state and automation events. In-bed is not asleep. History/analysis is V2; Body+ is secondary and uses a distinct path.

## Current status
**Latest live evidence supersedes earlier no-account/no-capture statements below:** temporary harness completed real OAuth, encrypted token persistence, subscription readback for50/51, HEAD capture and one real POST for each direction. User/account binding and same-device identity verified privately. Delays were4.683s/4.486s from provider timestamps, NOT measured physical-to-notification delay. See [live provider evidence](live-provider-evidence.md). Actual Niles add-on remains disconnected pending interface0.3 spec and implementation. Cloudflare Access was removed at Johan's explicit direction; older protected-test-site descriptions below are historical. No household actuation. Next external gate is Mark's spec review and resolution of the proposed occupancy API refetch requirement; overnight/physical latency remains unproven.
Implemented locally: parser/state core (20 retained tests), host-neutral storage/sink delivery adapter (8 tests), file-backed replay, configurable component room and truthful disconnected setup/status UI/tool. No account access, credentials, provider writes, callback route, deployment, or live service test occurred. Pinned WIT has no inbound HTTP listener; provider callback delivery and component occupancy/action wiring remain unimplemented. Main independently reran local Niles 0.2.1 checker/dev-host acceptance; the binary source commit is not independently established. WIT provenance remains pinned separately.

## Acceptance ledger
- [x] Official bed category/payload/transport contract documented.
- [x] Parser and state core compile; replay sequence executed; parser/state tests pass.
- [x] Exact pinned Niles WIT vendored with provenance.
- [x] Local Niles 0.2.1 checker and `plugin dev` accepted the real generated component and synthetic setup; disconnected status, room-scoped sensor and setup UI verified offline.
- [ ] Niles host action proof (requires incoming callback contract/runtime support).
- [ ] Host inbound callback + durable delivery contract; integrate callback → shared parser/state → report-state/report-action.
- [x] Implement provider OAuth/token and notify request-response contracts with injected transport/secret-store seams; fixture-tested only, not connected/account-proven. Host encrypted mutable secret support remains unconfirmed.
- [ ] Wire provider client through host HTTP/callback APIs and encrypted durable secret store; maintain separate subscription health from bed presence.
- [ ] Offline host-level action proof and separately authorized account/callback/physical event validation.

## Safety and state contract
Presence is Unknown/InBed/OutOfBed; startup restores state without action, first fresh valid observation can emit matching bed action, silence never expires occupancy, same-direction repeats and older events do not fire, equal-time opposing events mark health uncertain. Parser defaults are CLI-injected/configurable thresholds (max age and future skew); validate those choices before production. OAuth URL/callback-state, exchange/refresh and both bed subscription contracts are implemented and fixture-tested, but no connected OAuth flow or concrete token store is implemented.

Main provider acceptance: 44 tests passed (11 provider unit, 8 adapter, 18 core contracts, 2 original review, 5 provider review), strict Clippy, formatting, fresh file-backed replay and real local Niles 0.2.1 checker/dev-host initialization. Verified using the existing CLI at `/home/jleth/projects/niles/target/release/niles`. Both categories 50/51 are handled; ambiguous cleanup ownership fails closed; secret-bearing Debug output is redacted. Live completion is gated on an agreed host inbound callback/OAuth-return interface and encrypted mutable token storage, followed by component wiring and separately authorized physical testing.

## Test environment checkpoint
A separate PC-side test Niles is running on 10.42.20.209, Compose project `test-niles`, with fresh database/encryption material, an empty isolated broker and no Internet/household connectivity. Remote UI: `https://test-niles.johanleth.dk`, through the existing PC tunnel, protected by Cloudflare Access OTP restricted to `jleth@me.com`; connector JWT audience enforcement is enabled. Anonymous UI/API redirects verified. Actual user login remains unverified. Loopback origin remains `http://127.0.0.1:18081`. No callback exceptions or outbound Withings access enabled. Deployment and resume instructions: `/home/jleth/services/test-niles/README.md`. Health/UI HTTP checks passed; add-on installation in this server, browser walkthrough and Withings login are still pending. Retired installation and project-server production remain untouched. Mark's revised reply (`message-2.txt`) supersedes his first reply. Proposed interface 0.3: permission-gated `on-http`; signed-in OAuth callback with synchronous code exchange; host-generated full public hook URL and host-handled HEAD 2xx, bounded/rate-limited POST forwarding; encrypted `secrets.set`. Add-on stays in its own repo; no relay, generic host OAuth, WebSockets or Live Sync 98–100. Docs-only upstream spec review precedes code. Requested prerequisites are measured bed-event delay, chosen public hook hostname/path isolation, and portal Callback URI multiplicity/registration rules; optional real HEAD/bed-in captures and add-on id `withings`. Revised reply's exact code lifetime, refresh grace and retry/cancellation statements are maintainer-supplied claims pending independent verification. Critical unresolved contradiction remains: API verification before reporting bed events is requested, but no current-occupancy API has been established for notification-only 50/51; do not substitute historical sleep data. Clarify durable acknowledgement/deferred work semantics rather than promising fast acknowledgement plus an unproven synchronous API fetch. Existing test UI remains Access-protected, not public-anonymous; no callback exceptions, capture endpoint, Withings login or subscriptions have been enabled. Await agreed specification and authorized real-account evidence; do not change host/WIT or ingress on the basis of this reply alone.

## Commands
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`
`cargo run --bin withings-replay -- <new-synthetic-state-file>`
`scripts/build-development.sh <development-output-directory>` (development only; local checker/dev-host tested, not production deployed)

No live external tests are included.
