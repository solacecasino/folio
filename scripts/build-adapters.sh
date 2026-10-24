#!/bin/sh
# Builds every adapter that has a build.sh, then packs all adapters into
# target/Folio-Adapters-<version>.zip for the release.
# Usage: scripts/build-adapters.sh
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

for build in adapters/*/build.sh; do
  [ -f "$build" ] || continue
  echo "Building $(dirname "$build")"
  sh "$build"
done

# The zip holds each adapter folder without its source build output.
STAGE=target/Folio-Adapters
rm -rf "$STAGE" && mkdir -p "$STAGE"
for dir in adapters/*/; do
  name=$(basename "$dir")
  mkdir -p "$STAGE/$name"
  rsync -a --exclude target --exclude src --exclude tests --exclude 'Cargo.*' --exclude build.sh --exclude .gitignore "$dir" "$STAGE/$name/"
done
cat > "$STAGE/README.txt" <<TXT
Folio adapters $VERSION

Folio ships bare. Each folder here adds one thing; install only what you want.

To install: in Folio choose Folio > Open Adapters Folder, drag adapter
folders into it, then restart Folio. The first time an adapter runs, Folio
shows what it asks for and you decide.

  pdf-export      Export as PDF (File menu, Cmd+E), print-perfect.
  folio-link      Live co-editing with people you invite, peer to peer.
  writing-tools   A word-count goal, today's date, document stats.
TXT
(cd target && rm -f "Folio-Adapters-$VERSION.zip" && ditto -c -k --norsrc --noextattr --keepParent Folio-Adapters "Folio-Adapters-$VERSION.zip")
echo "Built target/Folio-Adapters-$VERSION.zip"
