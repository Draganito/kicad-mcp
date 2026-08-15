# kicad-mcp

Mini MCP server that drives a **running KiCad PCB editor** from Cursor
over KiCad's official IPC API. Parts come from **EasyEDA / LCSC** so
JLCPCB footprints match. Not an autorouter, not a second PCB editor.

**License: [AGPL-3.0-only](LICENSE)** — Copyright © 2026 Dragan Bojovic.
See [NOTICE](NOTICE).

Deutsche Einstiegsanleitung: [ANLEITUNG_FUER_ANFAENGER.md](ANLEITUNG_FUER_ANFAENGER.md).
Handbuch A–Z: [docs/HANDBUCH.md](docs/HANDBUCH.md) (Deutsch) /
[docs/MANUAL.md](docs/MANUAL.md) (English).
Architecture: [docs/architecture.md](docs/architecture.md).

## Download (precompiled, nothing to build)

Grab the ready-to-run package from the
**[Releases page](https://github.com/Draganito/kicad-mcp/releases)**:
`kicad-mcp_<version>_amd64.deb` is the one release file — the binary,
Cursor MCP setup, and docs:

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb
```

Debian/Ubuntu x86-64, glibc 2.39+. You still need **KiCad 9 or 10**
installed, with Preferences → Plugins → **Enable IPC API**. Copy the
contents of `/usr/share/kicad-mcp/cursor-setup/` (`.cursor/` **and**
`.cursorignore`) into the folder you open in Cursor. Everything below
is only needed if you want to build from source.

## What you get

- **KiCad stays the editor** — this process only talks IPC. Do not
  edit `.kicad_pcb` by hand.
- **LCSC parts** — `download_lcsc_part` writes EasyEDA geometry into
  `jlcpcb_parts.pretty` next to the open board.
- **Placement, nets, copper** — footprints (including grids), ratsnest
  nets, tracks, vias, copper zones. Every write is one KiCad undo
  (Ctrl+Z).
- **Cursor MCP** — stdio server; Cursor starts `/usr/bin/kicad-mcp`
  (or your built binary). Write tools need `--allow-ai-write`.

## Build

Requirements: recent Rust (stable), Linux, KiCad 9 or 10 with IPC API.

```bash
cargo test --workspace
cargo build --release -p kicad-mcp
```

Debian package (needs [`cargo-deb`](https://crates.io/crates/cargo-deb)):

```bash
cargo install cargo-deb --locked
dist/make_beta_package.sh
# → dist/kicad-mcp_<version>_amd64.deb
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

Read: `board_summary`, `get_footprints`, `get_nets`, `get_routing_scene`,
`list_parts`, `check_board`.

Write: `download_lcsc_part`, `place_footprint`, `place_parts`,
`place_matrix`, `remove_footprint`, `clear_board`, `set_board_outline`,
`connect_pins`, `connect_many`, `add_track`, `add_tracks`, `add_via`,
`add_vias`, `set_copper_zone`, `ripup_wire` (by `segment_id`),
`save_board`.

Coordinates are **KiCad native millimetres** (board origin, +x right,
+y up). Start with `board_summary`.

The pink A4 frame in the editor is the **drawing sheet**, not the PCB.
Board size is **Edge.Cuts** (`set_board_outline`). If origin is omitted
on a rectangle, it is centred on that sheet. Existing Edge.Cuts are
**replaced** unless `replace` is false.

**KiCad 9.0.2** accepts net updates but does not persist `Pad.net` /
`Track.net`. Geometry does persist. Assign nets in the GUI on 9, or
use KiCad 10. Nested pads are not parent-transformed — this crate
bakes board millimetres into every pad.

## Repository layout

```text
crates/kicad-mcp      MCP stdio server (KiCad IPC)
crates/easyeda-kicad  LCSC/EasyEDA → .kicad_mod / .kicad_sym
contrib/              Cursor MCP config for the .deb
docs/                 Manuals + architecture notes
dist/                 Deb build script + LIESMICH.txt
scripts/              Optional helpers (not shipped in the .deb)
```

## Credits

KiCad is a separate GPL-3.0 program. This repo only talks to it over
the published IPC API.
