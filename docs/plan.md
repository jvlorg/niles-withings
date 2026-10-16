# Withings V1 goal and acceptance ledger

## Goal
Standalone Niles add-on for Sleep mat in-bed / out-of-bed state and automation events. In-bed is not asleep. History/analysis is V2; Body+ is secondary and uses a distinct path.

## Current status

### Executable add-on application boundary (offline only)

The approved host contract is the docs-only upstream Niles PR #439 at `03c4b55c09861d8ac34b50df1cd5305752cc1063`, verified by Main: callback HEAD returns empty 200 before auth/state consumption/add-on invocation; secret `set` atomically commits each value before success and failure retains the previous value; no plaintext-secret guidance; callback URL uses only configured `public_url` with no Host fallback. These are resolved SPEC requirements, not open design questions. The interface remains proposal-only: no WIT 0.3 wiring or implementation tests exist here. The host-neutral boundary in `src/application.rs` accepts optional person, stores tokens+timing in one bounded record, and redacts sensitive Debug data. Callback storage errors retain prior records; webhook delivery errors map to 503. Persist-before-publication still has a post-commit retry-loss window, documented rather than claimed solved. Host encryption remains host responsibility. Discovery remains intentionally deferred; manual device id is preserved.

Latest live evidence supersedes earlier no-account/no-capture statements below: a previous temporary harness completed real OAuth, encrypted token persistence, subscription readback for 50/51, HEAD capture and one real POST for each direction. See [live provider evidence](live-provider-evidence.md). This package made no live provider calls, changed no live services and does not wire lifecycle into Niles. The actual add-on remains disconnected pending interface 0.3 implementation. Cloudflare Access was removed at Johan's explicit direction; older protected-test-site descriptions are historical. No household actuation. The external gate remains host implementation validation; physical latency is unproven.

Implemented offline: parser/state core, persistence/sink adapter, synthetic replay, provider OAuth/notification contracts, and host-neutral lifecycle using injected transport/clock and atomic encrypted-store trait. Lifecycle tracks acquisition/expiry, refreshes near or after expiry only with coherent metadata, fails closed for invalid/future restart metadata and foreign account binding, latches token-rotation storage failure, reconciles categories 50/51, refuses conflicting ownership tuples, and requires readback before current status may report ready. The status query takes caller-supplied current time and is no longer stale-ready after expiry. Provider errors are distinct from explicit refresh account mismatch. Core and host boundary preserve `bed_in`/`bed_out`; this is not host wiring. Reusable presentation model separates connection from occupancy confidence and exposes last event time. The pinned component continues to report disconnected and does not claim event/actions or expose OAuth controls.

## Acceptance ledger
- [x] Official bed category/payload/transport contract documented.
- [x] Parser and state core compile; replay sequence executed; parser/state tests pass.
- [x] Exact pinned Niles WIT vendored with provenance; interface remains 0.2.1.
- [x] Local Niles 0.2.1 checker and `plugin dev` accepted the generated component and synthetic setup; disconnected status and UI verified.
- [x] Host-neutral core event-to-proposed-action mapping, unknown/known readings, presentation model and lifecycle expiry/account-error semantics are covered by synthetic tests.
- [ ] Niles host action proof; requires incoming callback contract/runtime support.
- [ ] Host inbound callback + durable delivery contract; integrate callback → shared parser/state → report-state/report-action.
- [x] Implement provider OAuth/token and notify request-response contracts with injected transport/secret-store seams; fixture-tested only, not connected/account-proven. Host encrypted mutable secret support remains unconfirmed.
- [ ] Wire lifecycle to host OAuth return, encrypted durable secret storage, callback and truthful live status UI. PR #439 is a docs-only spec; validate its implementation at current upstream HEAD before claiming host support. Confirm atomic per-value `secrets.set` guarantee and how required `public_url` becomes a trusted callback.
- [ ] Offline host-level action proof and separately authorized provider/physical validation.

## Safety and state contract
Presence and lifecycle readiness remain separate. Token metadata records Unix-second acquisition and expiry; refresh begins 300 seconds before expiry. If access expiry has passed, refresh may proceed only with known, positive, coherent, bounded acquisition/expiry metadata and a valid bound refresh credential; the expired access token is never used. Access-token age alone does not establish refresh-token expiry; a coherent expired record may attempt refresh, and provider rejection fails closed. Missing/invalid restart metadata and future or excessive lifetime fail closed. The secure-store trait requires atomic encrypted replacement of token secrets plus metadata. A store failure after refresh latches the lifecycle unusable for that process because the provider may have rotated the refresh token; process restart loses that in-memory latch, so the host must persist recovery state in its own durable contract. Do not imply exactly-once or durable crash recovery. Adapter persistence precedes publication; if a committed event's reading/action publication fails, duplicate delivery does not retry the missing publication. This is at-most-once attempt behavior, not reliable delivery or exactly-once. `Ready` means a recent-enough, time-checked offline lifecycle reconciliation only, not Niles-host connectivity, callback delivery, event receipt, or bed presence.

Main verification for this package: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` (61 passing tests; 0 failed). Fresh file-backed synthetic replay passed, including duplicate/restart/foreign-profile assertions. The real Niles 0.2.1 CLI checker and dev-host run passed; rendered output explicitly showed `Connection: disconnected`, `Occupancy: unavailable while disconnected`, and `Last event: none`. It also reported `Setup incomplete: callback transport is not available in this Niles interface; no Withings account is connected.` Host run confirms only the disconnected component, not lifecycle wiring. Both categories 50/51 are handled; ambiguous cleanup ownership fails closed; secret-bearing Debug output is redacted. No live external tests are included.

## Test environment checkpoint
The upstream interface 0.3 proposal is documentation only, not an installed WIT/component contract. This package does not enable callback exceptions, capture endpoints, Withings login or subscriptions. No occupancy refetch is required for event identity/date/dedup plus host capability authentication under the accepted contract. Validate upstream HEAD implementation and the atomic-store/public-URL details before host wiring. Historical sleep data must not substitute for bed occupancy. The configured device id remains manual until component discovery UI is implemented; no device-discovery schema was verified for this slice.

## Commands
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`
`cargo run --bin withings-replay -- <new-synthetic-state-file>`
`scripts/build-development.sh <development-output-directory>` (development only; local checker/dev-host tested, not production deployed)

No live external tests are included.
