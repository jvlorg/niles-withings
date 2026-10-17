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
python3 - "$out" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
check, dev = (p/'check.log').read_text(), (p/'dev.log').read_text()
assert 'It would install.' in check
assert 'plugin@0.3' in check
for expected in ['status: Working', 'Connect Withings', '0 supported sleep mats', 'delivery is not ready']:
    assert expected in dev, expected
print('PASS real Niles 0.3 component load and disconnected startup; NOT live provider proof')
PY
