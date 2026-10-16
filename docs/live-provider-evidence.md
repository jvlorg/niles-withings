# Withings V1: real-account evidence for interface 0.3 spec

## Scope and outcome
Temporary validation/OAuth/capture harness on Johan's PC test infrastructure, not a deployed Niles add-on. Real OAuth code exchange and encrypted token storage succeeded; notification subscriptions for appli 50 and 51 were independently read back from Withings. Existing subscriptions were not revoked. No household device commands or Niles automation actions were issued.

## Deployment and URL evidence
- Add-on id: `withings`; V1 categories only 50/51. History V2; Live Sync 98–100 excluded.
- Johan owns deployment and manages Cloudflare Tunnel ingress.
- Registered OAuth callback shown saved in portal: `https://test-niles.johanleth.dk/plugins/withings/callback`.
- A distinct capability-token hook path on the SAME hostname was successfully subscribed for both categories. Its token is intentionally omitted.
- Therefore exact equality between notification URL and the displayed registered OAuth callback was not required in this account test. Multiple registered OAuth URLs and cross-host notification matching remain untested.
- Callback validation initially failed on Access302 then nonexistent-handler404; after implementing the temporary receiver, portal accepted it. Actual no-query HEAD callback requests captured at 19:58:50Z and19:58:54Z; hook HEAD requests captured during subscription at20:10:09Z. Receiver replies200 to HEAD. No HEAD body.
- Current test site is publicly reachable without Cloudflare Access at Johan's explicit request. This is NOT evidence of a final hook-only deployment. Tunnel supports exact-path routing; final normal UI/callback authentication and public hook policy remain to be agreed.

## Observed notification requests
Both real requests: POST, Content-Type `application/x-www-form-urlencoded`, body89 bytes, exactly fields `userid`, `deviceid`, `appli`, `date`. User IDs matched the authorized account; both device IDs matched each other. Raw bodies and selected headers remain in owner-only local captures, not this repository.

| Category | Provider date (Unix seconds) | Receiver arrival UTC | Arrival minus provider timestamp |
|---|---:|---|---:|
|50 bed-in|1792181539|2026-10-16T20:12:23.683391Z|4.683 s|
|51 bed-out|1792181608|2026-10-16T20:13:32.486211Z|4.486 s|

Redacted shape only, NOT a verbatim wire payload:
`userid=<redacted>&deviceid=<redacted>&appli=50&date=1792181539`

Johan reported lying down during the test. Exact physical movement timestamps were not recorded; the exit was not separately timestamp-confirmed. These two observations establish real event receipt, not guaranteed physical-to-event latency, an overnight sample, SLA, or immediate night-light suitability. Timestamp subtraction depends on clock alignment.

## Remaining questions for the spec (historical notes; PR #439 resolved relevant contract items)
1. Identity/date/dedup plus host capability authentication is the accepted contract; no occupancy refetch for notification-only 50/51. This resolves the earlier question as a SPEC requirement, not evidence of deployed host behavior or cryptographic provider-origin authentication.
2. HEAD reachability validation of the registered OAuth callback must work without interactive login, while actual OAuth completion remains bound to the initiating browser/state. PR #439 specifies empty 200 before state consumption/add-on invocation.
3. Post-commit retry-loss remains a delivery limitation: acknowledge after durable state/action processing; current adapter is persist-before-publish and at-most-once attempt.
4. PR #439 specifies per-value atomic encrypted secret persistence and callback URL from configured public_url without Host fallback. Host implementation/wiring and tests remain pending.
5. V1 sensor/actions suffice; unknown must remain distinct from out-of-bed. The approved add-on contract retains the exact core action names `bed_in`/`bed_out`; no translation to `in_bed`/`out_of_bed`.

## State
Evidence ready for Johan to share; not posted upstream by agent. Niles host extension remains docs-spec-first, then review, then implementation. Temporary harness is only for evidence gathering and is not a permanent relay architecture.
