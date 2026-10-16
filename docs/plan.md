# Withings V1 goal and acceptance ledger

## Goal
Standalone Niles add-on for Sleep mat in-bed / out-of-bed state and automation events. In-bed is not asleep. History/analysis is V2; Body+ is secondary and uses a distinct path.

## Current status
**Latest live evidence supersedes earlier no-account/no-capture statements below:** temporary harness completed real OAuth, encrypted token persistence, subscription readback for50/51, HEAD capture and one real POST for each direction. User/account binding and same-device identity verified privately. Delays were4.683s/4.486s from provider timestamps, NOT measured physical-to-notification delay. See [live provider evidence](live-provider-evidence.md). Actual Niles add-on remains disconnected pending interface0.3 spec and implementation. Cloudflare Access was removed at Johan's explicit direction; older protected-test-site descriptions below are historical. No household actuation. Next external gate is Mark's spec review and resolution of the proposed occupancy API refetch requirement; overnight/physical latency remains unproven.
Implemented offline: parser/state core, persistence/sink adapter, synthetic replay, provider OAuth/notification contracts, and host-neutral lifecycle using injected transport/clock and atomic encrypted-store trait. Lifecycle tracks acquisition/expiry, fails closed on missing/expired metadata and foreign account binding, latches token-rotation storage failure, reconciles only category 50/51, refuses conflicting ownership tuples, and requires readback before status Ready. Ready means offline provider-client lifecycle readiness only; host integration and event receipt are not implied. No live provider requests or deployment occurred in this package.

## Acceptance ledger
- [x] Official bed category/payload/transport contract documented.
- [x] Parser and state core compile; replay sequence executed; parser/state tests pass.
- [x] Exact pinned Niles WIT vendored with provenance.
- [x] Local Niles 0.2.1 checker and `plugin dev` accepted the real generated component and synthetic setup; disconnected status, room-scoped sensor and setup UI verified offline.
- [ ] Niles host action proof (requires incoming callback contract/runtime support).
- [ ] Host inbound callback + durable delivery contract; integrate callback → shared parser/state → report-state/report-action.
- [x] Implement provider OAuth/token and notify request-response contracts with injected transport/secret-store seams; fixture-tested only, not connected/account-proven. Host encrypted mutable secret support remains unconfirmed.
- [ ] Wire lifecycle to host OAuth return, encrypted durable secret storage, callback and truthful status UI.
- [ ] Offline host-level action proof and separately authorized provider/physical validation.

## Safety and state contract
Presence and lifecycle readiness remain separate. Token metadata records Unix-second acquisition and expiry; refresh begins 300 seconds before expiry. Time arithmetic is checked and token lifetimes are bounded to 31 days. The secure-store trait requires atomic encrypted replacement of token secrets plus metadata. Missing/invalid restart metadata and expired access tokens fail closed. A store failure after refresh latches the lifecycle unusable because the provider may have rotated the refresh token; recovery must reauthorize, never retry the old token blindly. Subscription readiness requires readback of both exact callback/category tuples; unrelated entries are never revoked. Ready means offline lifecycle reconciliation only, not Niles-host connectivity, callback delivery, event receipt, or bed presence.

Main verification: 52 tests passed before lifecycle addition (provider, adapter, core, review); current workspace total is 52 passing tests, strict Clippy, formatting, fresh file-backed replay and real local Niles 0.2.1 checker/dev-host initialization. Lifecycle tests are included in this current count; the Niles host run confirms only the unchanged disconnected component, not lifecycle wiring. Verified using the existing CLI at `/home/jleth/projects/niles/target/release/niles`. Both categories 50/51 are handled; ambiguous cleanup ownership fails closed; secret-bearing Debug output is redacted. Live completion is gated on an agreed host inbound callback/OAuth-return interface and encrypted mutable token storage, followed by component wiring and separately authorized physical testing.

## Test environment checkpoint
The isolated test Niles checkpoint and upstream interface0.3 proposal below are historical planning/evidence, not implementation authorization or proof of current runtime state. No callback exceptions, capture endpoint, Withings login or subscriptions were enabled by this package. Await an agreed host specification before wiring APIs; no current-occupancy fetch is established for notification-only 50/51, and historical sleep data must not substitute. Do not change host/WIT or ingress based on the proposal alone.

## Commands
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`
`cargo run --bin withings-replay -- <new-synthetic-state-file>`
`scripts/build-development.sh <development-output-directory>` (development only; local checker/dev-host tested, not production deployed)

No live external tests are included.
