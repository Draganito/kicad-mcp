kicad-mcp — Debian package
==========================

Prebuilt Linux build of kicad-mcp (AGPL-3.0) as a .deb package.
Lay out a PCB in a running KiCad PCB editor, from Cursor.
Footprints come from EasyEDA/LCSC so they match JLCPCB.
Start here: README.md (same doc folder).
Source: https://github.com/Draganito/kicad-mcp
License notices: LICENSE.txt

Install
-------
  sudo apt install ./kicad-mcp_<version>_amd64.deb

What is where
-------------
  /usr/bin/kicad-mcp                          MCP server (stdio, Linux x86-64)
  /usr/bin/kicad-10                           Wrapper: AppImage with TMPDIR=/tmp
  /usr/share/applications/kicad-10.desktop    Menu entry
  /usr/share/kicad-mcp/cursor-setup/          Cursor setup: MCP config,
                                              AI rules, .cursorignore
  /usr/share/doc/kicad-mcp/                   This file, README.md,
                                              LICENSE.txt, docs — including
                                              fresh install:
                                              docs/INSTALL_DEBIAN.md
                                              A-Z reference:
                                              docs/MANUAL.md

Requirements
------------
- Debian/Ubuntu x86-64, glibc 2.39+ (ldd --version)
- KiCad 10 AppImage with the PCB editor open (not the Debian 9 package)
- In KiCad: Preferences -> Plugins -> Enable IPC API, then restart KiCad
- Always start via kicad-10 (TMPDIR=/tmp, otherwise MCP cannot find the
  socket). Fresh install: docs/INSTALL_DEBIAN.md
- Optional: kicad-routing-tools_*.deb from the same release, then
  kicad-routing-tools-setup (as your user). MCP and plugin do not call
  each other.

Start
-----
  Cursor launches the binary itself (stdio). Copy the contents of
  /usr/share/kicad-mcp/cursor-setup/ (.cursor/ AND .cursorignore)
  into your Cursor project folder.

  Write access: mcp.json passes --allow-ai-write (preconfigured).
  Without that flag every write tool refuses.

Product
-------
- Not a second PCB editor, not an autorouter
- The board is Edge.Cuts in KiCad; never edit .kicad_pcb by hand
- LCSC parts via download_lcsc_part (EasyEDA geometry)
- Place, nets, tracks, vias, copper zones via MCP
- check_pins: every pin netted or explicitly allowed open
- export_manufacturing: Gerber zip + BOM + CPL for JLCPCB
  (silk without reference designators — avoids DFM silkscreen-to-pad)
- Every write lands on KiCad's undo stack (Ctrl+Z)
