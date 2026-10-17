#!/usr/bin/env bash
# Real host loading proof, deliberately without account credentials or provider calls.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
host="${1:?absolute path to Niles 0.3 CLI}"
out="${2:?fresh absolute scratch output directory}"
[[ "$host" = /* && "$out" = /* && ! -e "$out" ]] || { printf 'Require absolute host and fresh absolute output path\n' >&2; exit 1; }
bash "$root/scripts/build-development.sh" "$out"
"$host" plugin check "$out" | tee "$out/check.log"
"$host" plugin dev "$out" --dry-run --settings room=bedroom --settings client_id=fixture-client --settings client_secret=synthetic-not-a-credential --call withings_status '{}' --render withings | tee "$out/dev.log"
# A second real host process must restore the first process's startup journal.
"$host" plugin dev "$out" --dry-run --settings room=bedroom --settings client_id=fixture-client --settings client_secret=synthetic-not-a-credential --call withings_diagnostics_export '{"page":0,"per_page":100}' | tee "$out/export.log"
python3 - "$out" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
check, dev = (p/'check.log').read_text(), (p/'dev.log').read_text()
assert 'It would install.' in check
assert 'plugin@0.3' in check
assert 'logo.svg passes the sanitiser' in check
assert (p/'logo.svg').read_bytes()
for expected in ['status: Working', 'Connect Withings', '0 supported sleep mats', 'delivery is not ready']:
    assert expected in dev, expected
import json
exports = [json.loads(line) for line in (p/'export.log').read_text().splitlines() if line.startswith('{')]
assert len(exports) == 1
export = exports[0]
assert export['total'] == 2 and len(export['records']) == 2
assert all(r['event'] == 'lifecycle' and r['stage'] == 'startup' and r['outcome'] == 'disconnected' for r in export['records'])
assert len({r['correlation'] for r in export['records']}) == 2
assert export['gap_count'] == 0 and export['used_bytes'] > 0
print('PASS real Niles 0.3 component load, disconnected startup and journal export across two dev-host processes; NOT live provider or installed update proof')
PY
