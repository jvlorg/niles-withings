# Persistent Withings diagnostics

## Delivery and evidence boundary

Implemented in the standalone add-on, not Niles core. The journal uses the existing host-managed persistent key/value store. Historical alpha.12 test-host installation and physical-event acceptance are recorded on test Niles; see [historical test-host acceptance](plan.md#historical-test-host-acceptance). A separate later recorded audit observed enabled alpha.12 on production; see [recorded state and evidence provenance](plan.md#recorded-state-and-evidence-provenance) and the [production automation-discovery audit](plan.md#production-automation-discovery-audit). Subsequent whole-host Access restrictions block fresh unauthenticated test-host provider callbacks. Detailed journal restart/update and installed export proof belongs to test Niles; a matching installed version does not transfer that acceptance to production. Approved reauthorization recovered the test-host connection at that acceptance stage; actual bed-entry/exit processing and publication were recorded.

The canonical delivery adapter emits a typed observation after applying its existing reducer and after each actual persistence/publication result. The component buffers at most two observations, then writes receipt/decision/result records **after essential state and actions**. There is no second diagnostic parser or reducer and no inference of publication from HTTP 200.

Each valid bed event carries its receipt timestamp, provider timestamp, `bed_in`/`bed_out`, an account-scoped hashed mat reference and a durably allocated request correlation. Duplicate/stale and unchanged observations are distinguished from transitions. `action_published` means the host action import returned success; `no_action` means no action was attempted. Reading, action and presence-persistence failures are separate. This preserves the existing at-most-once-attempt contract: a post-persistence publication failure can permanently lose an action. It is not an outbox.

Malformed or blocked input gets an unscoped `blocked_or_rejected` record; no raw payload, URL, token, account/device ID or arbitrary error text is retained. Local references are pseudonyms, not a promise of anonymity. Exports contain sensitive household timing data.

Health summaries record startup and readiness/failure changes immediately, and unchanged health at most hourly. Fixed categories distinguish authorization, refresh, refresh persistence, subscription and discovery failures. Callback receipt, denial/failure and completed activation are recorded. These categories describe observed boundaries, not inferred provider root causes.

## Bounds and recovery

- Shared Niles quota: **1,048,576 bytes**, including ordinary storage keys and values. Encrypted secrets have separate host ownership.
- Journal hard bound: **540,672 bytes (528 KiB)**, including metadata and keys; 64 fixed slots, at most 24 records per slot, 8 KiB per serialized chunk, 1 KiB metadata, 320 bytes per serialized record.
- Up to **1,536 records**, before age, capacity, corruption or runtime-pressure eviction. Thirty days is a target, not an unlimited guarantee.
- The maintained 1,080-record / 30-day real-epoch fixture occupies **246,570 bytes**, including all keys and metadata. This fixture uses bounded mat references and duplicate-decision records rather than assuming the earlier 256-byte estimate is a proven encoding limit.
- Normal append: two reads and three writes; correlation reservation is separate. Recovery, daily cleanup and export scan only the fixed slots. Partial chunks are pruned without resetting their append cursor. Export always filters records older than 30 days; physical cleanup happens on the next daily maintenance append while running.
- A durable pending marker precedes chunk mutation. Lost/corrupt metadata reconstructs the cursor from retained chunks and marks a gap; partial commits, corrupt/missing chunks and failed writes cannot masquerade as complete history. Recovery counters can be uncertain: inspect `counters_exact` and `gap_count`, not just totals.
- No implicit merging of real events. Malformed/blocked request storms retain one detailed attempt per minute and aggregate subsequent attempts into bounded count/first/last summaries. The next window or lifecycle timer flushes them. Tests exercise 10,000 requests with and without failed diagnostic writes, asserting no more than 12 storage-write attempts during each same-window flood.
- Pending suppression counts and write failures are exposed separately. **Unflushed summaries/failure notices can be lost on a crash**, especially if storage itself is unavailable; the export says so. No journal can prove that an absent Withings callback was ever sent.

The 528 KiB cap leaves 496 KiB below the shared quota, but no fixed reserve can promise every historical mat scope will fit: the existing host-ID registry alone permits 64 KiB and individual restored presence records permit 64 KiB. On an essential ordinary-storage write failure, the adapter checks the host's actual key-plus-value accounting. Only confirmed quota pressure reclaims diagnostic chunks, preserving a loss marker and unrelated state, then retries the essential write once. Generic storage errors do not erase the journal. This cannot fix a runtime-state-only dataset exceeding the entire quota.

## Inspect/export

Use the existing add-on tool `withings_diagnostics_export`. The pinned host exposes it as `withings_withings_diagnostics_export` in its global tool catalogue. Alpha.12 also provides an opt-in **Read diagnostic journal** button on the Withings surface. It uses this canonical export function, renders 25 records per JSON page, provides Next and Hide controls, and rejects stale snapshots. Copy the JSON pages for export; there is no browser-file download API.

First request:

```json
{"page":0,"per_page":100}
```

Continue using the returned `snapshot` token and unchanged `per_page`:

```json
{"page":1,"per_page":100,"snapshot":"<token from page 0>"}
```

Continue until `has_more` is false. If records change or expire between pages, the tool explicitly rejects the old token; restart at page zero rather than joining inconsistent pages. Page size is 1–100, arguments are strictly typed/bounded, and extraction makes no writes or provider requests. Maximum record payload is bounded by 100 × 320 bytes plus fixed metadata. Export is append-ordered, with timestamps available for analysis.

Read `total`, `oldest`, `newest`, `used_bytes`, `truncation_count`, `suppression_count`, `write_failure_count`, `gap_count`, `counters_exact`, pending counters and `complete`. `complete` is only the absence of **known journal loss**, never proof of complete provider delivery or automation execution. Counts may be uncertain after a marked partial write/recovery.

## Verified / not yet verified

Recorded historical local gates: **133 workspace tests**, formatting, strict all-target Clippy, release Wasm build, fresh replay, positive and deliberately failing runtime checks. Actual component tests included callback, two mats, same-second correlations, duplicate/restart silence, failed reading/action publication, diagnostic-write independence, hourly health/failure recovery, malformed floods, read-only token-guarded export and real delivery under a host-rule quota-enforcing test store. These are prior candidate results, not checks run for this documentation reconciliation.

The maintained `scripts/verify-niles03-host.sh` runs the real pinned Niles checker, loads/renders the disconnected component, then starts a second dev-host process and exports both retained startup records. This is actual local host/file-backed process-restart proof, not connected production proof.

Initial alpha.11 test-host installation used the normal hash-verified update flow with no added permissions. All four prior ordinary-storage entries and encrypted-secret rows survived unchanged. A full test-host restart retained its PostgreSQL journal record and added a second startup record. At that stage, the pre-update account record already had an unresolved refresh-pending marker, so account restoration was blocked; no credentials or markers were changed. Temporary outbound was closed, dry-run preserved and health verified.

Subsequent approved reauthorization and ongoing provider egress restored test-host readiness; real bed-entry/exit publications were recorded. Alpha.11→12 retained journal and ordinary state, and the actual installed UI exported all 12 records without storage writes. Final automatic-refresh/restart/export acceptance retained all 18 earlier records and exported 19 including startup. These are historical test-host records; live assistant orchestration and the household test remain distinct from installed UI export proof. Counts describe their acceptance stages, not current journal totals.

Niles automation execution remains a separate evidence gap: the inspected host's recorded limitation was retaining only its last 20 runs per automation in memory, and the add-on interface exposed no attributable started/completed/failed automation hook. Successful `action_published` does **not** prove an automation ran. Adaptive wake-up is also unproven: brief-exit session bedtime retention, once-per-session cancellation and calculated scheduling are separate requirements from trigger support. See the [production audit](plan.md#production-automation-discovery-audit) for recorded action-discovery and native wake-up-ramp findings.
