# Withings V1 goal and acceptance ledger

## Goal
Standalone Niles add-on for Sleep mat in-bed / out-of-bed state and automation events. In-bed is not asleep. History/analysis is V2; Body+ is secondary and uses a distinct path.

## Current status
Implemented locally: parser/state core (20 retained tests), host-neutral storage/sink delivery adapter (8 tests), file-backed replay, configurable component room and truthful disconnected setup/status UI/tool. No account access, credentials, provider writes, callback route, deployment, or live service test occurred. Pinned WIT has no inbound HTTP listener; provider callback delivery and component occupancy/action wiring remain unimplemented. Main independently reran local Niles 0.2.1 checker/dev-host acceptance; the binary source commit is not independently established. WIT provenance remains pinned separately.

## Acceptance ledger
- [x] Official bed category/payload/transport contract documented.
- [x] Parser and state core compile; replay sequence executed; parser/state tests pass.
- [x] Exact pinned Niles WIT vendored with provenance.
- [x] Local Niles 0.2.1 checker and `plugin dev` accepted the real generated component and synthetic setup; disconnected status, room-scoped sensor and setup UI verified offline.
- [ ] Niles host action proof (requires incoming callback contract/runtime support).
- [ ] Host inbound callback + durable delivery contract; integrate callback → shared parser/state → report-state/report-action.
- [ ] Confirm secure mutable OAuth secret support before token flow.
- [ ] Offline host-level action proof and separately authorized account/callback/physical event validation.

## Safety and state contract
Presence is Unknown/InBed/OutOfBed; startup restores state without action, first fresh valid observation can emit matching bed action, silence never expires occupancy, same-direction repeats and older events do not fire, equal-time opposing events mark health uncertain. Parser defaults are CLI-injected/configurable thresholds (max age and future skew); validate those choices before production. No OAuth flow or insecure token storage is implemented.

## Commands
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`
`cargo run --bin withings-replay -- <new-synthetic-state-file>`
`scripts/build-development.sh <development-output-directory>` (development only; local checker/dev-host tested, not production deployed)

No live external tests are included.
