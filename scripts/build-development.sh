#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/dist/development}"
mkdir -p "$out"
cargo build --manifest-path "$root/Cargo.toml" -p niles-withings-component --target wasm32-wasip2 --release
cp "$root/target/wasm32-wasip2/release/niles_withings_component.wasm" "$out/integration.wasm"
cp "$root/component/manifest.toml" "$out/niles-integration.toml"
cp "$root/assets/withings-logo.svg" "$out/logo.svg"
printf '%s\n' 'DEVELOPMENT ARTIFACT ONLY — not host-checked, installable, or live-proven.' > "$out/DEVELOPMENT-ONLY.txt"
printf 'Development artifact: %s\n' "$out"
