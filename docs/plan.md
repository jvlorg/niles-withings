# Withings V1 goal and acceptance ledger

## Goal
Standalone Niles add-on for Sleep mat in-bed / out-of-bed state and automation events. In-bed is not asleep. History/analysis is V2; Body+ is secondary.

## Current status

### Executable add-on application boundary (offline only)

The approved host contract remains the docs-only upstream Niles PR #439 at `03c4b55c09861d8ac34b50df1cd5305752cc1063`; it is not interface implementation. This package retains the host-neutral callback/webhook/lifecycle boundaries and does not add WIT 0.3 wiring. Callback storage errors retain previous records; webhook storage/read/action errors map to 503. Persist-before-publication retains a post-commit retry-loss window: duplicate retries do not repeat the reading/action. No delivery semantics were changed.

Withings device discovery is now implemented as an offline transport seam: `POST https://wbsapi.withings.net/v2/user`, form `action=getdevice`, bearer token; bounded JSON `body.devices` parse of `deviceid`, `type`, `model`, `model_id`. Response limits reuse the 64 KiB response bound; rows limited to 256 and fields bounded. Device Debug redacts opaque identifiers. Host-neutral setup helper APIs compose the secure callback record, discovery, an explicit candidate/choice acceptance, bounded versioned non-secret account/device selection, and construction of the existing event Adapter only after selection save succeeds. The secret and selection stores are independent, so this deliberately makes no cross-store atomicity claim. A selection from a new OAuth account is not eligible; corrupt/foreign selection fails closed, while a missing or conflicting selected ID is stale and never switches. Existing manual component config remains unchanged.

Official API reference: https://developer.withings.com/api-reference/#operation/userv2-getdevice . Its current schema specifies `type` values including `Sleep Monitor` and `Scale`, with `model`, `model_id`, and `deviceid`; its published Sleep Monitor models are `Aura Dock` (60), `Aura Sensor` (61), and `Aura Sensor V2` (63). It does not list the Withings Sleep Analyzer. We therefore use only the documented type field to identify the eligible sleep-monitor category, without asserting those Aura models are the Sleep Analyzer. Exact Sleep Analyzer model/type compatibility remains unresolved pending an authoritative listing. This avoids fabricated model matches.

Latest live evidence is historical and documented in [live provider evidence](live-provider-evidence.md). This package made no live provider calls, changed no live services, and did not wire discovery into the component. No device/user credentials or private files were accessed.

## Acceptance ledger
- [x] Official bed category/payload/transport contract documented.
- [x] Parser and state core compile; replay sequence executed; parser/state tests pass.
- [x] Exact pinned Niles WIT vendored with provenance; interface remains 0.2.1.
- [x] Local Niles 0.2.1 checker and `plugin dev` accepted generated component and synthetic setup; disconnected status/UI verified.
- [x] Host-neutral event/action mapping, lifecycle and composed callback → secure record restore → subscription readback → bed events → restart covered offline.
- [x] Documented `user.getdevice` transport parser and bounded account/device selection tested with clearly synthetic fixtures; explicit persisted acceptance gates Adapter construction.
- [x] Webhook store load/save, reading publish, and action publish failure seams exercised; 503 and committed retry-loss behavior confirmed.
- [x] Stateful setup coordinator owns the active adapter and setup/discovery generation, invalidates delivery on account change, device refresh/switch, and discovery errors; selection must be durable before adapter activation. Account/device isolation and stale discovery acceptance are represented by generation-bound API. Coordinator activation now requires a key-value backend and internally derives account/device-scoped storage; unscoped activation helpers are private. Main exercised account switching, stale generation rejection, save failure, stale-discovery activation refusal, and old-account webhook rejection. Main also verified mat-switch/back persistence isolation.
- [x] Account/device-scoped presence storage and coordinator mat-A → mat-B → mat-A restart proof; restored readings unknown, duplicate actions suppressed, new exit delivered, other scopes unchanged. Injected load/save failures fail closed and recovery executes.
- [ ] Host WIT 0.3 callback / mutable encrypted storage / device-selection UI integration.
- [ ] Verify official device model/category for the exact Withings Sleep Analyzer; published API schema currently lists Aura models only under Sleep Monitor.
- [ ] Account/device provider validation and separately authorized physical validation.

## Safety and state contract
Presence and lifecycle readiness remain separate. Refresh now atomically persists a version-2 pending marker in the same encrypted record before contacting the provider. Restore rejects pending records with RecoveryRequired; old readers reject version 2. Successful replacement returns to a complete version-1 record. Failed intent persistence sends no HTTP; ambiguous refresh outcomes stay blocked. A crash before the request may conservatively require reconnect. Five offline regressions cover pending restore, completed refresh restore, final-save failure, intent failure and foreign-account response. Actual host-neutral callback recovery now restores usable credentials and reaches synthetic subscription Ready; failed callback persistence preserves pending bytes, and ambiguous transport failure blocks retry. Existing five refresh regressions are retained alongside four new tests. Browser/host wiring and crash injection across every boundary remain unproven. Test Niles remains publicly accessible without Cloudflare Access at Johan’s explicit direction. The secure-store trait requires atomic encrypted replacement of tokens plus metadata. Adapter persistence precedes publication; if committed-event publication fails, duplicate delivery does not retry missing publication. This is at-most-once attempt behavior, not reliable delivery or exactly-once. `Ready` refers only to offline lifecycle reconciliation, not host connectivity, callback delivery, event receipt, or bed presence.

## Verification
Main independently verified formatting, strict workspace Clippy, 79 passing workspace tests, fresh synthetic replay and the actual Niles 0.2.1 checker/dev-host. Coordinator-specific account-switch and failure-path tests now execute. Mat-switch/back namespace restoration, separate-account history, and load/save failure recovery now pass through the coordinator. No live external tests.

## Commands
`cargo fmt --all -- --check`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`
`cargo run --bin withings-replay -- <new-synthetic-state-file>`
`scripts/verify-niles-host.sh <niles-0.2.1-cli> <absolute-scratch-output>`
