# kicad-mcp

Mini MCP server that drives a **running KiCad PCB editor** from Cursor
over KiCad's official IPC API. Parts come from **EasyEDA / LCSC** so
JLCPCB footprints match. Not a second PCB editor. Named-net autoroute
is optional (`autoroute_nets`, companion Routing Tools deb).

**License: [AGPL-3.0-only](LICENSE)** — Copyright © 2026 Dragan Bojovic.
See [NOTICE](NOTICE).

Deutsche Einstiegsanleitung: [ANLEITUNG_FUER_ANFAENGER.md](ANLEITUNG_FUER_ANFAENGER.md).
Neuinstallation Debian + Cursor: [docs/INSTALL_DEBIAN.md](docs/INSTALL_DEBIAN.md)
/ [docs/INSTALL_DEBIAN.en.md](docs/INSTALL_DEBIAN.en.md).
Handbuch A–Z: [docs/HANDBUCH.md](docs/HANDBUCH.md) (Deutsch) /
[docs/MANUAL.md](docs/MANUAL.md) (English).
Architecture: [docs/architecture.md](docs/architecture.md).

## Download (precompiled, nothing to build)

Grab the packages from the
**[Releases page](https://github.com/Draganito/kicad-mcp/releases)**:

- `kicad-mcp_<version>_amd64.deb` — MCP binary, `kicad-10` wrapper,
  Cursor setup, docs
- `kicad-routing-tools_0.20.4-2_amd64.deb` — optional autorouter plugin
  for the KiCad 10 AppImage (MIT, [drandyhaas](https://github.com/drandyhaas/KiCadRoutingTools)).
  Does not call kicad-mcp; kicad-mcp does not call it.

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb ./kicad-routing-tools_0.20.4-2_amd64.deb
kicad-routing-tools-setup   # as your user, once
```

Debian/Ubuntu x86-64, glibc 2.39+. You need **KiCad 10** with
Preferences → Plugins → **Enable IPC API**. On Debian 13, system KiCad
is 9.0.2 — do not use it. After the mcp deb, start the official AppImage
with `kicad-10` (`TMPDIR=/tmp`, socket `/tmp/kicad/api.sock`). See
[docs/INSTALL_DEBIAN.md](docs/INSTALL_DEBIAN.md).
Copy the contents of `/usr/share/kicad-mcp/cursor-setup/` (`.cursor/`
**and** `.cursorignore`) into the folder you open in Cursor. Everything
below is only needed if you want to build from source.

## What you get

- **KiCad stays the editor** — this process only talks IPC. Do not
  edit `.kicad_pcb` by hand.
- **LCSC parts** — `download_lcsc_part` writes EasyEDA geometry and
  pin names (`pins.json`) into `jlcpcb_parts.pretty` next to the open
  board. `get_part_pins` rereads them. Nets follow EasyEDA `pin_name`;
  a datasheet only after a logic check that EasyEDA cannot be right.
- **Placement, nets, copper** — footprints (including grids), ratsnest
  nets, tracks, vias, copper zones. Every write is one KiCad undo
  (Ctrl+Z).
- **Cursor MCP** — stdio server; Cursor starts `/usr/bin/kicad-mcp`
  (or your built binary). Write tools need `--allow-ai-write`.

## Build

Requirements: recent Rust (stable), Linux, KiCad 10 with IPC API.

```bash
cargo test --workspace
cargo build --release -p kicad-mcp
```

Debian package (needs [`cargo-deb`](https://crates.io/crates/cargo-deb)):

```bash
cargo install cargo-deb --locked
dist/make_beta_package.sh
# → dist/kicad-mcp_<version>_amd64.deb
# → dist/kicad-routing-tools_0.20.4-2_amd64.deb
```

From a source tree, point Cursor at the **built binary**, not
`cargo run`:

```json
{
  "mcpServers": {
    "kicad-mcp": {
      "command": "/path/to/kicad-mcp/target/release/kicad-mcp",
      "args": ["--allow-ai-write"]
    }
  }
}
```

A packaged copy lives in [`contrib/cursor-setup/`](contrib/cursor-setup)
(command = `/usr/bin/kicad-mcp`). After Rust changes: rebuild, then
toggle the MCP server off/on.

Without `--allow-ai-write`, every write tool refuses.

## Tools

Read: `board_summary`, `get_footprints`, `get_nets`, `get_pads`,
`check_placement`, `get_routing_scene`, `list_parts`, `get_part_pins`,
`check_board`.

Write: `download_lcsc_part`, `make_wire_pad`, `make_mounting_hole`,
`place_footprint`, `place_parts`, `place_matrix`, `move_footprint`,
`remove_footprint`, `clear_board`, `clear_zones`, `set_board_outline`,
`connect_pins`, `connect_many`, `disconnect_pin`, `disconnect_many`,
`add_track`, `add_tracks`, `add_via`,
`add_vias`, `stitch_via`, `set_copper_zone`, `autoroute_nets`,
`ripup_wire` (by `segment_id`), `check_drc`, `render_board`,
`save_board`, `export_manufacturing`.

Coordinates are **KiCad native millimetres** (board origin, +x right,
+y up). Start with `board_summary`.

The pink A4 frame in the editor is the **drawing sheet**, not the PCB.
Board size is **Edge.Cuts** (`set_board_outline`). If origin is omitted
on a rectangle, it is centred on that sheet. Existing Edge.Cuts are
**replaced** unless `replace` is false.

**KiCad 10** persists `Pad.net` / `Track.net` after `connect_many`.
System 9 does not — `board_summary.net_ipc_persists` must be true.
Nested pads are not parent-transformed — this crate bakes board
millimetres into every pad.

`export_manufacturing` needs **kicad-cli** (same KiCad install). It
saves the open board, then writes JLCPCB files next to
the project: `<name>_gerbers.zip`, `<name>_bom.csv`, `<name>_cpl.csv`.
Silkscreen in the zip has **no** reference/value text (JLCPCB DFM
flags those on dense boards); names stay in the BOM and CPL.

## Repository layout

```text
crates/kicad-mcp      MCP stdio server (KiCad IPC)
crates/easyeda-kicad  LCSC/EasyEDA → .kicad_mod / .kicad_sym
contrib/              Cursor MCP config for the .deb
docs/                 Manuals + architecture notes
dist/                 Deb build script + LIESMICH.txt
scripts/              KiCad 10 launcher (also /usr/bin/kicad-10 in the .deb)
dist/kicad-routing-tools/  companion autorouter .deb (built, not committed)
```

## Credits

KiCad is a separate GPL-3.0 program. This repo only talks to it over
the published IPC API.
