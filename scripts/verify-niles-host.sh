#!/usr/bin/env bash
set -euo pipefail
if [[ $# -ne 2 ]]; then
  printf 'usage: %s <niles-0.2.1-cli> <absolute-output-directory>\n' "$0" >&2
  exit 2
fi
cli=$1
out=$2
if [[ "$out" != /* ]]; then
  printf 'output directory must be absolute and disposable\n' >&2
  exit 2
fi
if [[ ! -x "$cli" ]]; then
  printf 'Niles CLI is not executable: %s\n' "$cli" >&2
  exit 2
fi
version=$("$cli" --version)
if [[ "$version" != 'niles 0.2.1' ]]; then
  printf 'requires exact pinned host CLI niles 0.2.1; found: %s\n' "$version" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$out"
"$root/scripts/build-development.sh" "$out"
"$cli" plugin check "$out"
"$cli" plugin dev "$out" --dry-run \
  --settings user_id=synthetic-user \
  --settings device_id=synthetic-mat \
  --settings room=bedroom \
  --call status '{}' --render withings --wait 0
