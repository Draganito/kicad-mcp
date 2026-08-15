#!/usr/bin/env bash
# Builds the binary release package for kicad-mcp.
#
# Result (always under dist/):
#   dist/kicad-mcp_<version>_amd64.deb   THE release file: binary +
#                                        docs + cursor-setup (see
#                                        crates/kicad-mcp/Cargo.toml
#                                        [package.metadata.deb])
#
# Usage:  dist/make_beta_package.sh [--skip-build]
#
# One-time prerequisite:  cargo install cargo-deb --locked

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}"
BIN="$TARGET_DIR/release/kicad-mcp"

if [[ "${1:-}" != "--skip-build" ]]; then
    echo "==> cargo build --release -p kicad-mcp"
    echo "    CARGO_TARGET_DIR=$TARGET_DIR"
    (cd "$PROJECT_DIR" && cargo build --release -p kicad-mcp)
fi
[[ -x "$BIN" ]] || { echo "ERROR: $BIN missing"; exit 1; }
echo "==> binary: $BIN ($(date -r "$BIN" '+%Y-%m-%d %H:%M'), $(du -h "$BIN" | cut -f1))"

HELP=$("$BIN" --help)
echo "$HELP" | grep -qi 'allow-ai-write' \
    || { echo "ERROR: binary missing --allow-ai-write"; exit 1; }

echo "==> cargo deb"
command -v cargo-deb >/dev/null \
    || { echo "ERROR: cargo-deb not installed (fix: cargo install cargo-deb --locked)"; exit 1; }
rm -f "$PROJECT_DIR"/dist/kicad-mcp_*.deb
DEB=$(cd "$PROJECT_DIR" && cargo deb -p kicad-mcp --no-build -o "$PROJECT_DIR/dist/" | tail -1)
[[ -f "$DEB" ]] || { echo "ERROR: cargo deb produced no package"; exit 1; }

DEB_LISTING=$(dpkg-deb -c "$DEB")
for f in \
    ./usr/bin/kicad-mcp \
    ./usr/share/kicad-mcp/cursor-setup/.cursor/mcp.json \
    ./usr/share/kicad-mcp/cursor-setup/.cursor/rules/kicad-mcp.mdc \
    ./usr/share/kicad-mcp/cursor-setup/.cursorignore \
    ./usr/share/doc/kicad-mcp/README.md \
    ./usr/share/doc/kicad-mcp/LIESMICH.txt \
    ./usr/share/doc/kicad-mcp/LICENSE.txt \
    ./usr/share/doc/kicad-mcp/NOTICE \
    ./usr/share/doc/kicad-mcp/ANLEITUNG_FUER_ANFAENGER.md \
    ./usr/share/doc/kicad-mcp/docs/HANDBUCH.md \
    ./usr/share/doc/kicad-mcp/docs/MANUAL.md \
    ./usr/share/doc/kicad-mcp/docs/architecture.md
do
    grep -q " $f\$" <<<"$DEB_LISTING" \
        || { echo "ERROR: $f missing in deb"; exit 1; }
done
if grep -qE '\.git/|kicad_projekte/|/target/' <<<"$DEB_LISTING"; then
    echo "ERROR: source tree or live boards leaked into deb"; exit 1
fi
DEB_SIZE=$(stat -c%s "$DEB")
[[ "$DEB_SIZE" -lt 99000000 ]] \
    || { echo "ERROR: deb exceeds GitHub's 100 MB release-asset comfort zone"; exit 1; }

echo
echo "OK: $DEB  ($(du -h "$DEB" | cut -f1))"
