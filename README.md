# Niles Withings

Research and design package for a standalone Niles Withings add-on.

## Product scope

V1 is Withings Sleep in-bed / out-of-bed state, with events usable by Niles automations. “In bed” is not a claim that someone is asleep. Historical sleep reports and analysis are V2. Body+ is secondary and only in scope if its distinct data/notification path proves straightforward; do not assume it shares the Sleep notification.

## Status

Research/design only. No production code, Withings account access, live API validation, OAuth consent, callback registration, deployment, or release has been performed. Official bed-event categories, scopes, example payloads and callback transport are documented; household end-to-end operation remains unproven. See [the plan](docs/plan.md), [architecture and evidence](docs/architecture.md) and [the draft generic Niles core request](docs/niles-core-request.md). The current inspected host lacks incoming add-on callbacks; secure runtime token persistence also needs agreement.
