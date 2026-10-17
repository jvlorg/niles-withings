# Niles Withings

Rust add-on for Withings Sleep bed-presence, onboarding and events. Scope is in-bed / out-of-bed only; in-bed does not mean asleep. Historical sleep is V2; Body+ remains secondary and not yet implemented.

## Implemented locally

- Bounded Rust form parser for official categories 50 (bed in) / 51 (bed out), required user/device/date isolation, strict duplicate required-key rejection and form percent decoding.
- Deterministic-age-filtered presence reducer, Unknown/InBed/OutOfBed state, health distinct from observation, restart serialization, bounded dedup/cursor and conservative transition policy.
- Offline synthetic replay CLI exercising parser/state and restart round trips.
- Host-neutral delivery adapter around the same parser and state, with injected durable store and sink. It commits changed state before publishing readings/actions. Failed reading/action publication is not retried after commit; this is at-most-once attempt behavior, not reliable delivery or exactly-once.
- Niles component supports host Connect OAuth and automatic activation of every eligible Sleep Monitor mat. No account/device ID or mat-selection step is required in normal setup. Account identity comes exclusively from the authenticated Withings account. The unused manual account-restriction setting and its runtime checks have been removed at the owner’s request.
- One shared account/refresh/subscription lifecycle feeds independent per-mat state, deduplication and `bed_in` / `bed_out` actions. Host IDs are deterministic per account/device. Alpha.7's persisted selected mat retains its `withings_sleep` alias and scoped history; another account cannot inherit them.
- Startup restores unknown occupancy without replaying actions. Failed inventory, registration or storage blocks delivery; removed devices become unavailable. Durable host-ID ownership allows cold-start revocation even if credentials cannot restore.
- The Withings page is diagnostics plus a discovery refresh control, not a setup selection page. A show/hide switch is implemented in the separate local Niles frontend candidate using existing menu visibility configuration; it is not installed on test Niles.
- Supported eligibility uses the provider's `Sleep Monitor` type, not guessed model mappings. The installed alpha.7 discovered Aura Sensor V2 Sleep Monitor and received real bed-in/bed-out events. Alpha.8 previously ran on isolated test Niles and automatically restored that mat with its existing identity; both physical state updates and saved-account recovery through a full test-host restart have been observed; strict live replay-action counting remains outstanding.

Alpha.11 includes a bounded persistent diagnostic journal: canonical receipt/decision/actual publication records, health summaries, quota-priority recovery and read-only snapshot-guarded JSON export. The 1,080-record sizing fixture uses 246,570 bytes under a 528 KiB journal cap. See [diagnostics and export](docs/diagnostics.md). Alpha.12 is installed on test Niles with approved ongoing outbound access and dry-run preserved. Account recovery, real entry/exit publication, PostgreSQL journal restart/update retention and installed read-only JSON viewing/export have passed. Automatic refresh acceptance is scheduled. Production is unchanged.

The real pinned Niles 0.3 checker/dev host verifies disconnected loading and journal retention across two separate local processes. This is not live provider proof, an installed update test or a production installation.

## Settings presentation — direction and discussion with Mark

The intended Withings settings experience should visually match Niles' built-in add-ons: icon/title and concise description, consistent spacing and dividers, familiar inputs and action buttons, clear account status, and only necessary setup controls. Automatic mat discovery stays automatic; manual account/device identifiers and diagnostic clutter must not return.

The requested change is alignment of the existing settings page, not an independent HTML/CSS page or a new rendering architecture:

- Remove the prominent Status / From / Version / By / Installed metadata block from the main settings view.
- Present **Show Withings page** as a native switch row: label and short explanation on the left, switch on the right. Persist page visibility independently from add-on enablement; hiding the page must not stop bed-event automations. Verify actual navigation visibility, not merely empty page content.
- Place Save immediately to the right of the corresponding input, rather than below it; preserve validation, saving and error feedback.

Keep Withings account/device/event logic standalone. Preserve host-owned OAuth validation, encrypted secrets and add-on removal. The earlier proposal for independent settings markup was a misunderstanding and is withdrawn.

**Local candidate implemented, not deployed:** a frontend-only Niles patch now removes the metadata table, adds the native visibility switch and moves Save beside each ordinary input. It uses the existing `menu.hidden` configuration; the earlier claim that a static manifest prevented menu hiding was too broad. No new renderer, backend schema, WIT or Withings runtime changes are needed. Existing page visibility is preserved until explicitly changed.

See [the patch, screenshots and verification record](docs/settings-alignment/README.md). Production frontend build and all 1,233 frontend tests passed; desktop/phone browser checks used the actual components with explicitly synthetic configuration. Installed-host persistence and live automation continuity have not been re-tested for this candidate. The patch remains local and has not been sent to Mark or deployed.

## Local commands

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --bin withings-replay -- /absolute/path/to/new-synthetic-state.json
cargo run --bin withings-runtime-check
cargo run --bin withings-runtime-check -- --break-refresh-response
scripts/build-development.sh /absolute/path/to/development-output
```

These commands use synthetic fixtures and make no live provider writes. Actual Plugin entrypoint tests cover automatic multi-mat activation, migration, account isolation and recovery; the pinned host checker/dev host additionally proves disconnected loading and rendering. Neither proves live automatic multi-mat operation or full installed-host restart durability. Alpha.7's separately obtained live single-mat evidence must not be attributed to this candidate. See [architecture](docs/architecture.md) and the current [acceptance ledger](docs/plan.md).

`withings-runtime-check` exercises the public `runtime::Owner` callback, persistence readback, discovery, selection, event, owner drop/restore, activation, duplicate suppression, and refresh final-save failure path using only an injected synthetic transport and shared in-memory stores. Output is explicitly `OFFLINE SYNTHETIC NOT HOST0.3`. The restart stage is simulated by dropping/recreating the owner over shared in-memory persistence; it is not process or disk durability proof. `--break-refresh-response` deliberately supplies a malformed refresh response instead of reaching the expected final-save failure and must exit nonzero; it must not report a PASS for the failed scenario. No tokens, capability URLs, or request data are printed.

Official provider references: [User v2 Getdevice](https://developer.withings.com/api-reference/#operation/userv2-getdevice), [notification content](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-content), [subscription](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-subscribe), [overview](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-overview/).
