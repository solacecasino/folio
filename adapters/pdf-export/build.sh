#!/bin/sh
# Builds the adapter and puts the binary where adapter.toml's `run` expects it.
set -e
cd "$(dirname "$0")"
cargo build --release
cp target/release/pdf-export ./pdf-export
echo "Built $(pwd)/pdf-export"
