#!/bin/bash
# Start the official KiCad 10 AppImage so kicad-mcp can reach the IPC socket.
#
# The AppImage remaps TMPDIR into ~/.cache/tmp. kicad-mcp looks at
# /tmp/kicad/api.sock. Always start KiCad 10 via this wrapper (or a copy
# at ~/Programme/kicad-10.sh), never the .AppImage directly.
#
# Download: https://www.kicad.org/download/linux/  (Lite is enough)
# Docs: docs/HANDBUCH.md §3 / docs/MANUAL.md §3
set -euo pipefail
export TMPDIR=/tmp

if [ -n "${KICAD_10_APPIMAGE:-}" ]; then
  APP="$KICAD_10_APPIMAGE"
else
  APP=""
  for cand in \
    "$HOME/Programme/kicad-10.0.5-x86_64.AppImage" \
    "$HOME/Programme/kicad-10.AppImage" \
    "$HOME/Downloads/kicad-10.0.5-x86_64.AppImage"
  do
    if [ -x "$cand" ]; then
      APP="$cand"
      break
    fi
  done
fi

if [ -z "${APP:-}" ] || [ ! -x "$APP" ]; then
  echo "KiCad 10 AppImage not found." >&2
  echo "Download Lite/Full from https://www.kicad.org/download/linux/" >&2
  echo "chmod +x, put it in ~/Programme/, or set KICAD_10_APPIMAGE." >&2
  exit 1
fi

exec "$APP" "$@"
