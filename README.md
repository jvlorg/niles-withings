# Niles Withings

Rust executable foundation for a standalone Niles Withings Sleep bed-presence add-on. Scope is in-bed / out-of-bed only; in-bed does not mean asleep. Historical sleep is V2; Body+ remains secondary and not yet implemented.

## Implemented locally

- Bounded Rust form parser for official categories 50 (bed in) / 51 (bed out), required user/device/date isolation, strict duplicate required-key rejection and form percent decoding.
- Deterministic-age-filtered presence reducer, Unknown/InBed/OutOfBed state, health distinct from observation, restart serialization, bounded dedup/cursor and conservative transition policy.
- Offline synthetic replay CLI exercising parser/state and restart round trips.
- Host-neutral delivery adapter around the same parser and state, with injected durable store and sink. It commits changed state before publishing readings/actions. Failed reading/action publication is not retried after commit; this is at-most-once attempt behavior, not reliable delivery or exactly-once.
- Niles component setup validates user/device and a configurable room, registers a room-scoped sensor, and exposes a truthful disconnected status surface/tool. WIT is vendored at `component/wit/niles-integration.wit` (package 0.2.1; requires Niles 0.2.1+).
- Offline-only Withings OAuth/notification request contracts and synthetic fixture tests live in `src/withings_client.rs`. They are intentionally disconnected from the component: no callback host API or encrypted mutable secret-store API exists in the pinned WIT.

The local Niles 0.2.1 checker and stand-in dev host (binary source commit not independently established) have now accepted and run this component offline; reproducible command: `scripts/verify-niles-host.sh <niles-0.2.1-cli> <absolute-scratch-output>`. This uses only the CLI checker and `plugin dev` synthetic settings, with no account/API connection or provider writes. The artifact remains development-only, not a release/production install.

## Local commands

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --bin withings-replay -- /absolute/path/to/new-synthetic-state.json
scripts/build-development.sh /absolute/path/to/development-output
```

All fixtures are synthetic; provider client logic is fixture-tested, not connected or account-proven. No listener, account, provider write, deployment or live service interaction is included. The development artifact passes the local host checker and dev-host initialization; this is not production installation or Withings connectivity proof. See [architecture and evidence](docs/architecture.md) and [implementation plan](docs/plan.md).

Official provider references: [notification content](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-content), [subscription](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-subscribe), [overview](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-overview/).
