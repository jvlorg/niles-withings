# Withings V1 architecture and evidence

Research/design only; no add-on implementation or live account proof. Main reviewed and corrected worker research on 2026-10-16.

## Product boundary

V1 is Sleep mat **in-bed / out-of-bed**, with explicit state and automation events. In-bed does not mean asleep. Historical sleep is V2. Body+ is secondary and is not part of the same payload.

## Verified official provider contract

First-party sources:
- [Notification categories and payloads](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-content)
- [Subscription and URL requirements](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-subscribe)
- [Transport, retry and latency](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-overview/)

| Purpose | `appli` | OAuth scope | Delivery |
|---|---|---|---|
| Bed in | 50 | `user.sleepevents` | Notification-only: user lay down on bed |
| Bed out | 51 | `user.sleepevents` | Notification-only: user got out of bed |
| Scale weight/body composition | 1 | `user.metrics` | Notification of a date range, followed by Measure Getmeas |
| Sleep history | 44 | Docs label `users.activity` | Date range, followed by Sleep Get/Getsummary; V2, verify exact OAuth scope spelling before using |

Official bed payload examples have `userid`, `appli`, Unix-epoch `date` and `deviceid`. They are form-urlencoded POST bodies. No follow-up data fetch is needed for categories 50/51. Validate linked user and selected device, not merely the category. Example documentation is not a complete schema proving every field is always present; missing required identity/time fields must fail closed pending actual-account proof.

Callback URL: public HTTPS domain, not IP/localhost; production supported ports 80 or 443 (choose HTTPS 443); max 255 characters. Withings verifies registration with HTTP HEAD expecting 2xx. Normal event handling should return 2xx within seconds. The overview says responses not below 400 trigger retries: five cycles (initial, +10s, +1m, +1h, +4h), two attempts per cycle, later jitter. Repeated failures escalate to emails and eventual cancellation after 20 days. Generic historical backfill advice is not proof of a bed-presence reconciliation API.

Typical notification delay is under two minutes, occasionally longer; the standard plan has no real-time SLA. This is general notification guidance, not a measured bed-event latency or a device-specific guarantee. Do not promise instant lights-on on getting out of bed. Categories 98–100 provide 10-minute occupancy status but require a Withings Live Sync contract and device activation; they are not a normal free polling fallback.

No callback signing scheme was established from the reviewed sources. HEAD is reachability validation, not sender authentication. A callback route's unguessable secret can be a capability URL, but is not cryptographic proof of Withings origin. Never put OAuth access/refresh tokens in that URL. Keep route secrets out of logs. Validate payload identity and bounds; document residual forgery risk and avoid security-critical automation claims.

## Existing implementation corroboration

Home Assistant pinned to [3b6baa0bc3f1a1c10889bf84dc81fc1be14a7c62](https://github.com/home-assistant/core/tree/3b6baa0bc3f1a1c10889bf84dc81fc1be14a7c62/homeassistant/components/withings). `__init__.py` subscribes separately to weight, sleep and bed events, parses form POST `appli`; `coordinator.py` starts bed presence as `None`, sets it directly from IN_BED/OUT_BED and has no polling fallback. Manifest pins aiowithings 3.1.6, source [f055ccc7fed0868f52e7f7720b8fe0fa4b4e5642](https://github.com/joostlek/python-withings/tree/f055ccc7fed0868f52e7f7720b8fe0fa4b4e5642/src/aiowithings). These are reference implementation choices, not additional provider guarantees. Do not copy broad unsubscribe-all lifecycle handling: manage only this installation's owned subscriptions.

[Bambu Lab reference 329d1a8](https://github.com/MarkNygaard/niles-bambulab/tree/329d1a8d78e39ce1614d412ffca9c29cb598f15e) is a packaging/device/action/UI reference, not the transport design.

## Niles host compatibility

Pinned Niles [457821d5b210f8005b42868ad6786687bbcf248c](https://github.com/MarkNygaard/niles/tree/457821d5b210f8005b42868ad6786687bbcf248c).

| Requirement | Finding |
|---|---|
| Outbound subscription/token requests | Existing allowlisted HTTP |
| Read configured client secret | Existing `config.secret` reads encrypted secret settings |
| Persist dedup/state | Existing per-plugin database key/value storage, total 1 MiB |
| Securely persist rotated OAuth tokens | Not established: config exposes secret read, not write; ordinary private KV must not be called encrypted secret storage |
| Receive POST and answer HEAD | Missing from inspected plugin interface |
| Receive browser OAuth redirect | No generic inbound callback export found; needs separate GET/query/state response semantics |
| Report readings/actions | Existing device report and `report-action`; exact automation behavior still needs executable proof |
| Timers | Existing, re-establish after restart |

Sources: [WIT interface](https://github.com/MarkNygaard/niles/blob/457821d5b210f8005b42868ad6786687bbcf248c/schemas/niles-integration.wit), [host API](https://github.com/MarkNygaard/niles/blob/457821d5b210f8005b42868ad6786687bbcf248c/docs/integrations/host-api.md), [HTTP host](https://github.com/MarkNygaard/niles/blob/457821d5b210f8005b42868ad6786687bbcf248c/crates/niles-plugins/src/http.rs), [host imports](https://github.com/MarkNygaard/niles/blob/457821d5b210f8005b42868ad6786687bbcf248c/crates/niles-plugins/src/host.rs).

The inspected WIT exports lifecycle, tools, device set, timer, events, UI actions/render and MQTT callbacks, but no incoming HTTP callback. This blocks a self-contained push add-on on that revision. A website already being public does not create a plugin callback handler.

## Recommended ownership

Niles provides generic bounded callback routing, endpoint lifecycle, secure mutable secret persistence and public-base-URL plumbing. The add-on owns Withings OAuth flow/state, token refresh, subscriptions, payload validation, normalized occupancy, dedup/order policy and device/action reporting. No Withings-specific core logic, custom Niles fork or speculative sidecar.

Deployment context: existing Niles `home.johanleth.dk` is already behind the project-server Cloudflare Tunnel. Reuse that ingress if the host can expose narrowly scoped callback routes without weakening normal application authentication. Route behavior/reachability needs separate approved proof; this research has not changed the tunnel or server.

## State/event contract to implement and prove

- Identify the actual profile/device. Use a stable plugin-owned Niles ID, not last reporter wins across mats.
- Model unknown, in-bed and out-of-bed. Keep observed event time and transport/subscription status separate. Unknown is never an out-of-bed event.
- Proposed automation actions: `bed_in` and `bed_out` through existing `devices.report-action`, emitted only on accepted fresh transitions. State readings provide current known status; exact numeric reading/unknown mapping must be proved against Niles automation/UI contracts before acceptance.
- Persist bounded event cursor/dedup metadata. Same-direction repeats do not retrigger. Older events cannot undo newer state. Equal-time conflicting events are ambiguous, not resolved by arbitrary arrival order.
- Provider events can be delayed/retried. Reject delayed action replay using an explicitly chosen age policy while retaining diagnostics; timestamps alone cannot detect every missed event or establish present occupancy. The age threshold needs product approval and measured delivery, not an invented guarantee.
- Startup never emits a transition from restored state. A historical last-known state is not newly confirmed current presence. Without an authoritative reconciliation path, expose uncertainty until a fresh acceptable event.
- Silence during a long sleep is not an outage heartbeat. Do not expire normal occupied state into absent on an arbitrary timer. Known auth/subscription failure may mark status uncertain without emitting bed_out.
- No body-weight announcements, raw health/token logs, access tokens in callback URLs or unsecured refresh-token persistence.

## Remaining gates

1. Mark's generic callback contract plus secure rotating-secret solution, reviewed against existing interface compatibility.
2. OAuth app setup, least-scope consent and account eligibility; no account access performed. Official portal currently states new free-plan accounts from October 12, 2026 need Withings+, older accounts unaffected; verify actual account.
3. Synthetic handler/state-machine tests and real Niles action-trigger proof after the host contract is known.
4. Separately authorized callback registration, readback and user-observed bed entry/exit test to measure latency, duplicates, timestamps, actual payload and recovery behavior.

No complete callback authentication guarantee, physical event proof or instantaneous automation claim is made.
