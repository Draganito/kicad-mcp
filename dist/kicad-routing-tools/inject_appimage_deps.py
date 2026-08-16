#!/usr/bin/env python3
"""Insert the AppImage Python-deps hook into KiCadRoutingTools __init__.py.

The official KiCad 10 AppImage has no pip. Its PYTHONHOME also makes
/usr/bin/python3 die with "No module named encodings". The plugin must
import numpy/scipy/shapely from ~/.local/share/kicad/<ver>/3rdparty/python
before deps_check.py offers pip.
"""
from __future__ import annotations

import pathlib
import sys

HOOK = '''
def _add_bundled_python_deps():
    """KiCad 10 AppImage has no pip. PYTHONHOME also breaks /usr/bin/python3
    (encodings missing). Pre-unpacked cp311 wheels live next to the plugin:
    ~/.local/share/kicad/<ver>/3rdparty/python
    """
    home = os.path.expanduser("~")
    candidates = []
    parts = os.path.normpath(_plugin_dir).split(os.sep)
    try:
        i = parts.index("kicad")
        ver = parts[i + 1]
        candidates.append(os.path.join(home, ".local", "share", "kicad", ver, "3rdparty", "python"))
    except (ValueError, IndexError):
        pass
    candidates.append(os.path.join(home, ".local", "share", "kicad", "10.0", "3rdparty", "python"))
    for path in candidates:
        if os.path.isdir(os.path.join(path, "numpy")) and path not in sys.path:
            sys.path.insert(0, path)
            return


_add_bundled_python_deps()
'''

MARKER = "def _add_bundled_python_deps():"
ANCHOR = "if _plugin_dir not in sys.path:\n    sys.path.insert(0, _plugin_dir)\n"


def main() -> int:
    path = pathlib.Path(sys.argv[1])
    text = path.read_text(encoding="utf-8")
    if MARKER in text:
        return 0
    if ANCHOR not in text:
        print(f"ERROR: unexpected {path} — cannot find sys.path insert", file=sys.stderr)
        return 1
    path.write_text(text.replace(ANCHOR, ANCHOR + "\n" + HOOK, 1), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
