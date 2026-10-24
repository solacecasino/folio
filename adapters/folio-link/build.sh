#!/bin/sh
# Builds Folio Link and puts the binary next to adapter.toml.
set -e
cd "$(dirname "$0")"
cargo build --release
cp target/release/folio-link ./folio-link
echo "Built adapters/folio-link/folio-link"
