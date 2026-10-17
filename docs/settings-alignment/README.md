# Settings alignment — local candidate

Implements the owner's annotated reference: remove the prominent metadata table, put Save alongside each ordinary input, and add a native Show <page> page switch with label/help on the left and toggle on the right. Non-running/setup failures remain visible without the metadata table. OAuth, secrets, removal and integration enablement retain their existing owners.

## Patch and boundary

`niles-settings.patch` is a frontend-only patch against the inspected Niles host source (baseline used for the pinned 0.3 host proof, upstream revision `861a86448319e8cc14c8fd7a681768a3158dc8c4`). Local isolated candidate commit: `8d16b7523942bf7a71e11980ded8251660637c8a`. It is not applied to a live host, published, or sent upstream. Rebase against Mark's current source before integration.

The previous static-menu limitation was overstated: the manifest declares pages statically, but existing `menu.hidden` configuration already controls their navigation visibility. The switch uses that existing API/configuration path; no backend schema, WIT or Withings runtime changes are required. It preserves current visibility rather than silently hiding an existing installation's page. Hidden pages remain directly addressable, matching existing Niles behavior. A fresh read preserves unrelated hidden entries, and a post-write read verifies persistence; concurrent writes between those requests still have the existing API's last-writer-wins limitation.

Each inline Save validates and writes only its associated ordinary field. Other pending edits survive refresh; untouched values reconcile with refreshed configuration. Secret fields retain their existing write-only save implementation.

## Verification

- Production TypeScript/Vite build passed.
- Full frontend suite: 98 files, 1,233 tests passed.
- Browser checks at 900×1100 and 390×844: no metadata table; Save geometrically beside its input; field save and reload; page hide and reload; keyboard re-show; actual TabBar entry removal/restoration; fixture plugin remains enabled; no browser page errors.
- `desktop.png` and `phone.png` were visually reviewed after the dialog transition settled. Phone contents scroll inside the existing native dialog.
- Browser fixture imports the actual modified PluginPanel, PluginSettings, Switch, dialog and TabBar. Configuration is explicitly synthetic, persisted in localStorage through substituted API methods. This proves frontend behavior, not installed-server persistence or live automation continuity. Live services and provider APIs were not contacted.
- Existing dependency audit reported 10 vulnerabilities (1 moderate, 7 high, 2 critical); no unrelated dependency upgrade was attempted. Existing bundle-size and inbound-button semantics warnings remain outside this narrow patch.

## Reproduce

Apply the patch to the matching host checkout, install its frontend dependencies, run `npm run build` and `npm test` in `web/`. The isolated working copy and browser fixture for this run are at `/home/jleth/.hermes/profiles/main/cache/scratch/niles-settings-alignment`; fixture files are proof infrastructure, not a production route.

## For Mark — not sent

Please review the frontend-only alignment patch. It reuses the existing menu visibility mechanism rather than requesting a new renderer or integrating Withings logic into core. Before a release, apply/rebase in the maintained Niles tree and verify configuration persistence and menu updates against an actual test host. No production deployment is authorized by this document.
