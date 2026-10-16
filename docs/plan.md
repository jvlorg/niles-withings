# Withings V1 goal and acceptance ledger

## Goal
Standalone Niles add-on for Sleep mat in-bed / out-of-bed state and automation events. In-bed is not asleep. History/analysis is V2; Body+ is secondary and uses a distinct path.

## Current status
Latest live evidence supersedes earlier no-account/no-capture statements below: a previous temporary harness completed real OAuth, encrypted token persistence, subscription readback for 50/51, HEAD capture and one real POST for each direction. See [live provider evidence](live-provider-evidence.md). This package made no live provider calls, changed no live services and does not wire lifecycle into Niles. The actual add-on remains disconnected pending interface 0.3 specification and implementation. Cloudflare Access was removed at Johan's explicit direction; older protected-test-site descriptions are historical. No household actuation. The external gate remains Mark's host contract review and resolution of proposed occupancy API refetch requirements; physical latency remains unproven.

Implemented offline: parser/state core, persistence/sink adapter, synthetic replay, provider OAuth/notification contracts, and host-neutral lifecycle using injected transport/clock and atomic encrypted-store trait. Lifecycle tracks acquisition/expiry, refreshes near or after expiry only with coherent metadata, fails closed for invalid/future restart metadata and foreign account binding, latches token-rotation storage failure, reconciles categories 50/51, refuses conflicting ownership tuples, and requires readback before current status may report ready. The status query takes caller-supplied current time and is no longer stale-ready after expiry. Provider errors are distinct from explicit refresh account mismatch. Core `bed_in`/`bed_out` events map at a named host-neutral boundary to proposed `in_bed`/`out_of_bed`; this is not host wiring. Reusable presentation model separates connection from occupancy confidence and exposes last event time. The pinned component continues to report disconnected and does not claim event/actions or expose OAuth controls.

## Acceptance ledger
- [x] Official bed category/payload/transport contract documented.
- [x] Parser and state core compile; replay sequence executed; parser/state tests pass.
- [x] Exact pinned Niles WIT vendored with provenance; interface remains 0.2.1.
- [x] Local Niles 0.2.1 checker and `plugin dev` accepted the generated component and synthetic setup; disconnected status and UI verified.
- [x] Host-neutral core event-to-proposed-action mapping, unknown/known readings, presentation model and lifecycle expiry/account-error semantics are covered by synthetic tests.
- [ ] Niles host action proof; requires incoming callback contract/runtime support.
- [ ] Host inbound callback + durable delivery contract; integrate callback → shared parser/state → report-state/report-action.
- [x] Implement provider OAuth/token and notify request-response contracts with injected transport/secret-store seams; fixture-tested only, not connected/account-proven. Host encrypted mutable secret support remains unconfirmed.
- [ ] Wire lifecycle to host OAuth return, encrypted durable secret storage, callback and truthful live status UI.
- [ ] Offline host-level action proof and separately authorized provider/physical validation.

## Safety and state contract
Presence and lifecycle readiness remain separate. Token metadata records Unix-second acquisition and expiry; refresh begins 300 seconds before expiry. If access expiry has passed, refresh may proceed only with known, positive, coherent, bounded acquisition/expiry metadata and a valid bound refresh credential; the expired access token is never used. A long-expired token beyond the bounded refresh window fails closed. Missing/invalid restart metadata and future or excessive lifetime fail closed. The secure-store trait requires atomic encrypted replacement of token secrets plus metadata. A store failure after refresh latches the lifecycle unusable for that process because the provider may have rotated the refresh token; process restart loses that in-memory latch, so the host must persist recovery state in its own durable contract. Do not imply exactly-once or durable crash recovery. Adapter persistence precedes publication; if a committed event's reading/action publication fails, duplicate delivery does not retry the missing publication. This is at-most-once attempt behavior, not reliable delivery or exactly-once. `Ready` means a recent-enough, time-checked offline lifecycle reconciliation only, not Niles-host connectivity, callback delivery, event receipt, or bed presence.

Main verification for this package: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` (56 passing tests; 0 failed). Fresh file-backed synthetic replay passed, including duplicate/restart/foreign-profile assertions. The real Niles 0.2.1 CLI checker and dev-host run passed; rendered output explicitly showed `Connection: disconnected`, `Occupancy: unavailable while disconnected`, and `Last event: none`. It also reported `Setup incomplete: callback transport is not available in this Niles interface; no Withings account is connected.` Host run confirms only the disconnected component, not lifecycle wiring. Both categories 50/51 are handled; ambiguous cleanup ownership fails closed; secret-bearing Debug output is redacted. No live external tests are included.

## Test environment checkpoint
The isolated test Niles checkpoint and upstream interface 0.3 proposal below are historical planning/evidence, not implementation authorization or proof of current runtime state. No callback exceptions, capture endpoint, Withings login or subscriptions were enabled by this package. Await an agreed host specification before wiring APIs; no current-occupancy fetch is established for notification-only 50/51, and historical sleep data must not substitute. Do not change host/WIT or ingress based on the proposal alone.

## Commands
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`
`cargo run --bin withings-replay -- <new-synthetic-state-file>`
`scripts/build-development.sh <development-output-directory>` (development only; local checker/dev-host tested, not production deployed)

No live external tests are included.
