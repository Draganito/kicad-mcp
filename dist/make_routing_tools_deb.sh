#!/usr/bin/env bash
# Build kicad-routing-tools_<upstream>-<rev>_amd64.deb for GitHub Releases.
#
# Companion to kicad-mcp: same release page, separate license (upstream MIT).
# Downloads the official PCM zip + CPython 3.11 manylinux wheels so the
# KiCad 10 AppImage can import numpy/scipy/shapely without pip.
#
# Usage:  dist/make_routing_tools_deb.sh
# Output: dist/kicad-routing-tools_0.20.4-2_amd64.deb
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERE="$PROJECT_DIR/dist/kicad-routing-tools"

UPSTREAM_VER="${KRT_UPSTREAM_VER:-0.20.4}"
DEB_REV="${KRT_DEB_REV:-2}"
DEB_VER="${UPSTREAM_VER}-${DEB_REV}"
ZIP_URL="${KRT_ZIP_URL:-https://github.com/drandyhaas/KiCadRoutingTools/releases/download/v${UPSTREAM_VER}/KiCadRoutingTools-${UPSTREAM_VER}.zip}"

# Pinned cp311 manylinux wheels (KiCad 10 AppImage = CPython 3.11).
NUMPY_VER="${KRT_NUMPY_VER:-2.2.6}"
SCIPY_VER="${KRT_SCIPY_VER:-1.16.3}"
SHAPELY_VER="${KRT_SHAPELY_VER:-2.1.2}"

WORK=/tmp/krt-deb-build
ROOT="$WORK/root"
SRC="$WORK/src"
WHEEL="$WORK/wheels"
OUT="$PROJECT_DIR/dist/kicad-routing-tools_${DEB_VER}_amd64.deb"

rm -rf "$WORK"
mkdir -p "$ROOT/DEBIAN" \
  "$ROOT/usr/bin" \
  "$ROOT/usr/share/kicad-routing-tools/python-wheels" \
  "$ROOT/usr/share/doc/kicad-routing-tools" \
  "$SRC" "$WHEEL"

echo "==> download PCM zip $ZIP_URL"
curl -fsSL -o "$WORK/plugin.zip" "$ZIP_URL"
unzip -q -o "$WORK/plugin.zip" -d "$SRC"

# Zip layout: plugins/ + resources/ + metadata.json at the root.
if [[ ! -d "$SRC/plugins" ]]; then
  echo "ERROR: zip has no plugins/ directory" >&2
  exit 1
fi

cp -a "$SRC/plugins" "$ROOT/usr/share/kicad-routing-tools/"
if [[ -d "$SRC/resources" ]]; then
  cp -a "$SRC/resources" "$ROOT/usr/share/kicad-routing-tools/"
fi
if [[ -f "$SRC/metadata.json" ]]; then
  cp -a "$SRC/metadata.json" "$ROOT/usr/share/kicad-routing-tools/"
fi

# Pre-resolve the Linux Rust extension so a read-only copy still imports.
SO_SRC="$ROOT/usr/share/kicad-routing-tools/plugins/rust_router/grid_router-linux-x86_64.so"
SO_DST="$ROOT/usr/share/kicad-routing-tools/plugins/rust_router/grid_router.so"
if [[ -f "$SO_SRC" && ! -f "$SO_DST" ]]; then
  cp -a "$SO_SRC" "$SO_DST"
fi

/usr/bin/python3 "$HERE/inject_appimage_deps.py" \
  "$ROOT/usr/share/kicad-routing-tools/plugins/__init__.py"

echo "==> download cp311 wheels"
/usr/bin/python3 - "$WHEEL" "$NUMPY_VER" "$SCIPY_VER" "$SHAPELY_VER" << 'PY'
import json, pathlib, sys, urllib.request

dest = pathlib.Path(sys.argv[1])
wanted = {
    "numpy": sys.argv[2],
    "scipy": sys.argv[3],
    "shapely": sys.argv[4],
}

def pick(files):
    scored = []
    for f in files:
        name = f["filename"]
        if not name.endswith(".whl"):
            continue
        if "cp311" not in name:
            continue
        if "linux" not in name or "x86_64" not in name:
            continue
        if "manylinux" not in name:
            continue
        score = 10
        if "manylinux_2_17" in name or "manylinux2014" in name:
            score += 5
        scored.append((score, f))
    if not scored:
        return None
    scored.sort(key=lambda x: x[0], reverse=True)
    return scored[0][1]

for pkg, ver in wanted.items():
    data = json.load(urllib.request.urlopen(f"https://pypi.org/pypi/{pkg}/{ver}/json", timeout=60))
    whl = pick(data["urls"])
    if not whl:
        print("NO WHEEL", pkg, ver, file=sys.stderr)
        sys.exit(1)
    out = dest / whl["filename"]
    print("GET", pkg, ver, "->", whl["filename"])
    urllib.request.urlretrieve(whl["url"], out)
PY

cp -a "$WHEEL"/*.whl "$ROOT/usr/share/kicad-routing-tools/python-wheels/"

install -m 755 "$HERE/kicad-routing-tools-setup" "$ROOT/usr/bin/kicad-routing-tools-setup"

cat > "$ROOT/usr/share/doc/kicad-routing-tools/copyright" << EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: KiCadRoutingTools
Source: https://github.com/drandyhaas/KiCadRoutingTools
Hackaday: https://hackaday.io/project/204891-kicad-automated-routing-tools

Files: *
Copyright: 2026 drandyhaas
License: MIT

Files: usr/share/kicad-routing-tools/python-wheels/*
Copyright: NumPy, SciPy, and Shapely authors
License: BSD-3-Clause
Comment: CPython 3.11 manylinux wheels vendored so the KiCad 10 AppImage
 can import them. The AppImage has no pip and PYTHONHOME breaks host python3.

This .deb is a companion to kicad-mcp (AGPL-3.0-only) and is not part of
that program. Packaging scripts live in the kicad-mcp repository.
EOF

cat > "$ROOT/usr/share/doc/kicad-routing-tools/README.Debian" << EOF
KiCad Routing Tools ${UPSTREAM_VER} for KiCad 10 (AppImage).

After apt install, run once as your normal user (not root):

  kicad-routing-tools-setup

That copies the plugin into:

  ~/.local/share/kicad/10.0/3rdparty/plugins/com.github.drandyhaas.kicadroutingtools

and unpacks numpy / scipy / shapely (CPython 3.11) into:

  ~/.local/share/kicad/10.0/3rdparty/python

The official KiCad 10 AppImage has no pip. Do not run
"/usr/bin/python3 -m pip" from inside KiCad — PYTHONHOME then makes
system Python die with "No module named encodings".

Start KiCad via kicad-10 (from the kicad-mcp package). In Pcbnew:
Tools → External Plugins → KiCad Routing Tools

kicad-mcp does not call this plugin. MCP assigns nets; you (or this
plugin) route copper in KiCad.

Upstream: https://github.com/drandyhaas/KiCadRoutingTools
EOF

cat > "$ROOT/DEBIAN/control" << EOF
Package: kicad-routing-tools
Version: ${DEB_VER}
Section: electronics
Priority: optional
Architecture: amd64
Maintainer: Dragan Bojovic <draganito@users.noreply.github.com>
Depends: python3
Description: KiCad 10 autorouter plugin (AppImage-friendly)
 Fast Rust-accelerated A* autorouter for KiCad 9/10 PCB editor.
 .
 Ships the upstream PCM payload (MIT, drandyhaas), CPython 3.11 wheels
 for numpy/scipy/shapely, and a setup helper that copies them into
 ~/.local/share/kicad/10.0/ (the folder the official AppImage reads).
 .
 After install run: kicad-routing-tools-setup
 Do not pip-install from inside the AppImage.
 Companion to kicad-mcp — they do not call each other.
 Upstream: https://github.com/drandyhaas/KiCadRoutingTools
EOF

cat > "$ROOT/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e
echo "kicad-routing-tools: run 'kicad-routing-tools-setup' as your user (not root)."
EOF
chmod 755 "$ROOT/DEBIAN/postinst"

# Rootless build: files stay owned by the builder; dpkg warns, apt still installs.
dpkg-deb --build "$ROOT" "$OUT"
ls -lh "$OUT"
echo "OK: $OUT"
