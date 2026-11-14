#!/usr/bin/env bash
# Builds (if needed) and runs the fidelity harness, tools/compare. See tools/README.md.
#   tools/compare.sh one data/charts --native target/release/gallery
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6} nice cargo build --quiet --release \
  --manifest-path "$here/compare/Cargo.toml" >&2
exec "$here/compare/target/release/compare" "$@"
