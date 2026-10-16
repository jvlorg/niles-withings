# Request for Mark: generic add-on callbacks and OAuth secret persistence

Draft for Johan to review/send. Not posted upstream. Based on Niles `457821d5b210f8005b42868ad6786687bbcf248c`.

## Why

Our first Withings add-on is focused on Sleep mat bed-in/out automation events. Official Withings documentation confirms `appli=50/51`, scope `user.sleepevents`, with form-urlencoded POST payloads containing user/device identity and event timestamp. These are notification-only events, not historical sleep data pulls. The callback must answer a reachability HEAD with 2xx and process POST notifications promptly.

Niles already has the outbound HTTP, devices/actions, timers and persistent state needed for much of the add-on. Its inspected WIT exports have no inbound HTTP callback. `config.secret` reads encrypted secrets but offers no mutation API for the add-on to safely persist refreshed OAuth tokens. Ordinary private plugin KV is not established as encrypted secret storage.

## Minimum generic host facilities to agree

### 1. Plugin-scoped callback routes

- Opt-in manifest permission, visible during installation approval.
- Host-owned stable route allocation, scoped to an installed add-on; no arbitrary listener sockets or plugin-selected root paths.
- Receive bounded method/query/headers/body plus a route identifier. Permit explicit bounded status/headers/body responses.
- Support HEAD reachability checks and form POST notifications. Browser OAuth GET redirects need query parameters and safe completion/redirect responses; they are a distinct route purpose even if implemented by one generic HTTP facility.
- Endpoint lifecycle for restart/update/disable/uninstall, capability-secret rotation and revocation. Preserve callback identity across routine restart where possible.
- Bound execution time/body/queue/rate. Define when a 2xx means acceptance. Do not silently acknowledge and drop events while the plugin is unavailable. A durable queue is one design option, not an unbounded requirement; explicit retryable failure is another.
- Retain normal Niles API authentication. External-provider routes must not demand a Niles browser login/Bearer token, but must be narrowly scoped and protected by the agreed endpoint capability. Avoid callback body/query/route-secret logging.

### 2. Mutable encrypted plugin secrets

Provide plugin-scoped read/write/delete for runtime-owned OAuth tokens, or an equivalent host-managed OAuth token facility. Preserve isolation, atomic persistence/error reporting, redaction and uninstall cleanup. Do not require manual settings editing/restart for each token rotation. The add-on can own provider-specific refresh HTTP logic if secure storage is generic.

### 3. Public URL plumbing

Provide an explicit trusted public base URL/issued callback URL rather than deriving URLs from untrusted Host headers. We already run Niles through Cloudflare Tunnel at `home.johanleth.dk`; this is deployment plumbing, not a request for core to manage Cloudflare. Actual callback route reachability remains to be tested.

## What stays in the Withings add-on

OAuth scopes and authorization URL; state validation and short-lived code exchange; refresh handling; subscription create/list/reconciliation/revoke only for owned subscriptions; user/device validation; timestamp/dedup/order policy; unknown/in/out state; `bed_in`/`bed_out` action reporting; setup and diagnostics. Historical sleep and Body+ reads are later/secondary scope.

No Withings-specific core routes, built-in Withings integration or custom fork is requested. Existing report-action appears suitable for automation triggers; we will prove that against the real Niles runtime rather than request a new generic presence class prematurely.

## Acceptance evidence requested

1. Old 0.1/0.2 plugin compatibility remains intact; interface/version strategy explicit.
2. Callback isolation, permission denial, HEAD/POST/GET routing and bounded responses tested.
3. Disabled/removed/update/restart behavior and overload acceptance policy tested; no acknowledged silent loss.
4. Token mutation stays plugin-scoped and encrypted; persistence failures surfaced; no secret leakage in logs or ordinary config reads.
5. Synthetic callback → add-on state/action → Niles automation proof; no household device commands needed for the test.

## Provider references and limits

- [Categories, scope and exact example payloads](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-content)
- [HEAD check and callback URL requirements](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-subscribe)
- [Retries and latency](https://developer.withings.com/developer-guide/v3/data-api/notifications/notification-overview/)
- [Pinned Niles WIT](https://github.com/MarkNygaard/niles/blob/457821d5b210f8005b42868ad6786687bbcf248c/schemas/niles-integration.wit)

Withings's general notification guidance says typical delay is under two minutes with no standard-plan real-time SLA. We have not proved a signed-callback scheme or real account delivery. An unguessable endpoint is a bearer capability, not proof of sender identity. We will not present bed events as instantaneous or suitable for security-critical actions.
