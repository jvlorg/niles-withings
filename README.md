# Niles Withings

Rust add-on for Withings Sleep bed-presence, onboarding and events. Scope is in-bed / out-of-bed only; in-bed does not mean asleep. Historical sleep is V2; Body+ remains secondary and not yet implemented.

## Implemented locally

- Bounded Rust form parser for official categories 50 (bed in) / 51 (bed out), required user/device/date isolation, strict duplicate required-key rejection and form percent decoding.
- Deterministic-age-filtered presence reducer, Unknown/InBed/OutOfBed state, health distinct from observation, restart serialization, bounded dedup/cursor and conservative transition policy.
- Offline synthetic replay CLI exercising parser/state and restart round trips.
- Host-neutral delivery adapter around the same parser and state, with injected durable store and sink. It commits changed state before publishing readings/actions. Failed reading/action publication is not retried after commit; this is at-most-once attempt behavior, not reliable delivery or exactly-once.
- Niles component supports host Connect OAuth; authenticated account identity is derived from a bounded validated encrypted record and restored on restart. No numeric Withings user ID is needed. Optional `user_id` restricts the accepted account; invalid values fail closed.
- Sleep mats are discovered and selected explicitly through the component UI. Device selection remains ordinary non-secret storage and is account-bound; credentials remain only in the host encrypted secret store.
- Offline-only `user.getdevice` discovery and account-bound Sleep Monitor candidate/selection logic are also transport-fixture tested. The current official catalogue lists Aura Dock/Aura Sensor models, not Withings Sleep Analyzer; discovery deliberately does not guess that model mapping. Existing component configuration remains manual until host UI wiring.

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

All fixtures are synthetic; provider client logic is fixture-tested, not connected or account-proven. No listener, account, provider write, deployment or live service interaction is included. The development artifact passes the local host checker and dev-host initialization; this is not production installation or Withings connectivity proof. See [architecture and evidence](docs/architecture.md) and [implementation plan](docs/plan.md).

`withings-runtime-check` exercises the public `runtime::Owner` callback, persistence readback, discovery, selection, event, owner drop/restore, activation, duplicate suppression, and refresh final-save failure path using only an injected synthetic transport and shared in-memory stores. Output is explicitly `OFFLINE SYNTHETIC NOT HOST0.3`. The restart stage is simulated by dropping/recreating the owner over shared in-memory persistence; it is not process or disk durability proof. `--break-refresh-response` deliberately supplies a malformed refresh response instead of reaching the expected final-save failure and must exit nonzero; it must not report a PASS for the failed scenario. No tokens, capability URLs, or request data are printed.

Official provider references: [User v2 Getdevice](https://developer.withings.com/api-reference/#operation/userv2-getdevice), [notification content](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-content), [subscription](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-subscribe), [overview](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-overview/).
