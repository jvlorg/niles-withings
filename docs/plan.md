# Withings V1 goal and acceptance ledger

## Goal and current boundary
Standalone, automation-first Niles add-on: Connect → automatically discover and activate all eligible Sleep Monitor mats, each with independent identity, presence, `bed_in` / `bed_out` actions. In-bed is not asleep. Scale/history remain V2. This is development work, not a stable release.

Alpha.8 is now published and installed on test Niles through the normal preview/update API, with both artifact digests verified and no added permissions. Installed readback is running: the saved account loaded, one mat activated automatically, subscriptions are ready, and legacy `x_withings:bedroom/withings_sleep` identity is retained. Occupancy is correctly unknown pending a fresh event. Production remains unchanged. Alpha.7 has real evidence for OAuth (`user.info,user.sleepevents`), Aura Sensor V2 Sleep Monitor discovery, manual selection, subscription readiness and both event directions. Physical detection is delayed (>1–2 minutes reported); latency attribution is not measured. Fresh physical-event acceptance on alpha.8, live multi-mat validation and a full installed-process restart/dedup test remain outstanding.

## Implemented candidate
- Shared account OAuth, encrypted refresh and subscription lifecycle remains in `runtime::Owner`; `runtime::devices::DeviceRegistry` owns independent account/device-scoped adapters.
- The actual component callback/startup/refresh control/timer uses automatic discovery. No user selection or device-ID input is needed. Each eligible mat gets a deterministic account/device host ID, not an inventory-order or display-name ID.
- Inventory validity and subscription readiness are separate gates. Failed discovery/storage/registration revokes delivery; healthy lifecycle alone cannot override invalid inventory. Removed devices become unavailable. Unchanged successful discovery retains fresh occupancy.
- Presence restoration is unknown and replayed events are silent. Setup emits no bed action. Revocation clears cached occupancy.
- Per-device host registration, publication and HTTP/storage imports are isolated in `component/src/host_adapter.rs`; fleet/legacy compatibility is in `component/src/automatic.rs`. Obsolete singleton sink and manual selection/switch UI are removed. Legacy neutral coordinator APIs remain for migration regression coverage, not the normal component path.
- Durable `host_devices` records all owned host IDs before registration, including retired IDs, so cold startup can revoke stale host availability even when credentials cannot restore. This registry is bounded to 128 historical host IDs and 64 KiB; capacity/storage failure blocks activation rather than silently losing ownership.

## Alpha.7 migration contract
A valid persisted `selection` remains the authority for the old `room/withings_sleep` alias, only for its exact authenticated account/device pair. That mat retains its host identity and the unchanged account/device-scoped presence key. Additional mats use deterministic IDs. A replacement account cannot inherit the alias or history. Selection bytes are preserved, not rewritten into new ownership. Corrupt selection blocks delivery rather than silently renaming the mat.

The component migration test constructs the legacy Owner selection/accept/event flow using the same storage, then starts the automatic Plugin and proves legacy identity, unknown restoration, duplicate suppression, a second independent mat and new-account isolation. This is synthetic in-memory process-recreation evidence, not live database durability.

## Verification and acceptance
- [x] Bounded parser/reducer, persistent cursor/dedup and scoped adapter tests.
- [x] Automatic registry and composed Owner tests, including two mats and expiry refusal before HTTP.
- [x] Actual Plugin entrypoint tests: automatic zero/one/two-mat setup, isolated actions, entry/duplicate/exit, restart unknown, callback denial, corrupt account/selection, stale control, account restriction/replacement, refresh final-save failure, transient subscriptions, removed devices, registration/storage failures with successful retry, and cold-start stale-host revocation.
- [x] Legacy migration identity and scoped-history test.
- [x] Removed normal device-ID setting and selection controls; no numeric account ID required.
- [x] Main final verification: 111 workspace tests, formatting, strict all-target Clippy, release component Wasm, diff checks, fresh replay, positive runtime-check and intentionally failing negative runtime-check all behaved as required. Logs are in managed scratch `withings-automatic-final-checks-2`; final host proof has no undeclared-setting warning.
- [x] Real pinned Niles 0.3 checker and disconnected dev-host render accept the automatic candidate. Maintained command: `scripts/verify-niles03-host.sh <absolute-host> <fresh-absolute-scratch-output>`. Fresh setup renders Connect, zero mats, delivery not ready; it no longer invents a phantom sensor.
- [ ] Optional diagnostics menu visibility. Pinned host builds menu from static `manifest.ui.menu`; no supported per-install runtime setting found. The existing menu remains visible. No fake toggle or core/WIT patch.
- [ ] Remove legacy account restriction from settings without bypassing it. Niles rejects `config.get` for undeclared settings; retain optional `user_id`, labelled legacy and normally blank, until a deliberate host/migration solution. Removing the declaration while retaining Rust checks is not compatibility.
- [x] Published/installed `v0.1.0-alpha.8`, release commit `e590a45543761b15d28099ec9f7bbd19ef481b58`. Private pre-update PostgreSQL backup completed. Readback proves automatic single-mat migration and subscription readiness; real fresh entry/exit and full host restart remain pending.
- [ ] Full installed-host restart, encrypted storage durability and no-replayed automation action acceptance.

## Safety and evidence limits
- Offline implementation made no provider/service changes. The subsequent explicitly approved alpha.8 release/test updated only test Niles; its normal startup performed automatic discovery and subscription reconciliation. Dry-run remains enabled; production and household actuation are untouched.
- Real alpha.7 callbacks prove event receipt, not current physical posture or measured physical latency. Live test is dry-run.
- Credential refresh persists pending intent before rotation and fails closed across restart; restore validates account, format/version and metadata first.
- Presence persists before publication. A post-commit reading/action publication failure can permanently lose that publication; no outbox/exactly-once claim.
- Account secrets, host-ID registry and presence storage are independent; no cross-store transaction claim.
- Temporary outbound was closed after offline work, then reopened with approval for the alpha.8 release/test. Readback now confirms `bridge` plus `test-niles_isolated`; keep this window explicitly temporary and close it when physical testing finishes or pauses. No Cloudflare Access policy changes.
- Harness retirement and subscription ownership cleanup remain separate; never revoke installed add-on subscriptions as incidental cleanup.

## Repeatable commands
```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p niles-withings-component --target wasm32-wasip2 --release
cargo run --bin withings-replay -- <new-synthetic-state-file>
cargo run --bin withings-runtime-check
cargo run --bin withings-runtime-check -- --break-refresh-response
scripts/verify-niles03-host.sh <absolute-niles03-cli> <fresh-absolute-scratch-directory>
```
The runtime-check executable retains the legacy single-selection composition as a compatibility proof; the new automatic path is covered by `tests/automatic_owner.rs` and actual component tests. The deliberately broken refresh fixture must exit nonzero. The old 0.2 verifier is historical, not a gate for the current 0.3 component. Exact WIT remains pinned and unmodified; merged host source pin: `861a86448319e8cc14c8fd7a681768a3158dc8c4`.
