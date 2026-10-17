# Niles Withings

Rust add-on for Withings Sleep bed-presence, onboarding and events. Scope is in-bed / out-of-bed only; in-bed does not mean asleep. Historical sleep is V2; Body+ remains secondary and not yet implemented.

## Implemented locally

- Bounded Rust form parser for official categories 50 (bed in) / 51 (bed out), required user/device/date isolation, strict duplicate required-key rejection and form percent decoding.
- Deterministic-age-filtered presence reducer, Unknown/InBed/OutOfBed state, health distinct from observation, restart serialization, bounded dedup/cursor and conservative transition policy.
- Offline synthetic replay CLI exercising parser/state and restart round trips.
- Host-neutral delivery adapter around the same parser and state, with injected durable store and sink. It commits changed state before publishing readings/actions. Failed reading/action publication is not retried after commit; this is at-most-once attempt behavior, not reliable delivery or exactly-once.
- Niles component supports host Connect OAuth and automatic activation of every eligible Sleep Monitor mat. No account/device ID or mat-selection step is required in normal setup. An optional legacy account restriction remains declared for compatibility because Niles blocks reads of undeclared settings.
- One shared account/refresh/subscription lifecycle feeds independent per-mat state, deduplication and `bed_in` / `bed_out` actions. Host IDs are deterministic per account/device. Alpha.7's persisted selected mat retains its `withings_sleep` alias and scoped history; another account cannot inherit them.
- Startup restores unknown occupancy without replaying actions. Failed inventory, registration or storage blocks delivery; removed devices become unavailable. Durable host-ID ownership allows cold-start revocation even if credentials cannot restore.
- The Withings page is diagnostics plus a discovery refresh control, not a setup selection page. A per-install show/hide menu toggle remains blocked by the pinned host's static menu contract; no working toggle is claimed.
- Supported eligibility uses the provider's `Sleep Monitor` type, not guessed model mappings. The installed alpha.7 discovered Aura Sensor V2 Sleep Monitor and received real bed-in/bed-out events. Alpha.8 is installed on isolated test Niles and has automatically restored that mat with its existing identity; fresh physical-event and full host-restart acceptance are still pending.

The pinned Niles 0.3 host checker and stand-in dev host can validate disconnected loading only; this is not live provider proof or a production installation.

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
