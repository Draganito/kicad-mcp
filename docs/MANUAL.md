# kicad-mcp — Manual

Complete A-to-Z reference, v0.1.0.
German: [HANDBUCH.md](HANDBUCH.md).
Short German starter: [ANLEITUNG_FUER_ANFAENGER.md](../ANLEITUNG_FUER_ANFAENGER.md).

---

## Contents

1. [What it is](#1-what-it-is)
2. [Install](#2-install)
3. [Prepare KiCad](#3-prepare-kicad)
4. [Cursor / MCP](#4-cursor--mcp)
5. [Coordinates and outline](#5-coordinates-and-outline)
6. [Parts](#6-parts)
7. [Nets](#7-nets)
8. [Copper](#8-copper)
9. [Tool catalogue](#9-tool-catalogue)
10. [Save and undo](#10-save-and-undo)
11. [Building the Debian package](#11-building-the-debian-package)
12. [Troubleshooting](#12-troubleshooting)
13. [Deliberate limits](#13-deliberate-limits)

---

## 1. What it is

kicad-mcp is a **stdio MCP server** for Cursor. It drives a **running
KiCad PCB editor** over the official IPC API. KiCad remains the editor;
this is not a second layout program and not an autorouter.

JLCPCB footprints come from **EasyEDA / LCSC** (`download_lcsc_part`).
Builtins: `WirePad_PTH`, `MountingHole_M3_NPTH`.

License: AGPL-3.0-only. KiCad itself is a separate GPL-3.0 program and
is not shipped in this package.

## 2. Install

### Prebuilt package

From [GitHub Releases](https://github.com/Draganito/kicad-mcp/releases):

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb
```

Debian/Ubuntu x86-64, glibc 2.39+. **KiCad 10** must be running
(`recommends: kicad` is only the distro package — on Debian 13 that is
9.0.2 and is not enough for nets).

### From source

```bash
cargo test --workspace
cargo build --release -p kicad-mcp
```

Binary: `target/release/kicad-mcp`.

## 3. Prepare KiCad

kicad-mcp targets **KiCad 10**. Nets from `connect_many` /
`set_copper_zone` persist there. Do not use Debian 13's system KiCad 9.

1. Official AppImage: [kicad.org/download/linux](https://www.kicad.org/download/linux/)
   — **Lite** is enough. Some browsers save
   `kicad-10.0.5-x86_64.AppImage.tar`: `tar -xf … -C ~/Programme`.
2. `chmod +x ~/Programme/kicad-10.0.5-x86_64.AppImage`
3. **Always** start through the wrapper, not the `.AppImage` and not
   `/usr/bin/kicad`. The AppImage remaps `TMPDIR` under `~/.cache/tmp`;
   MCP looks for `ipc:///tmp/kicad/api.sock`.

```bash
install -m 755 scripts/kicad-10.sh ~/Programme/kicad-10.sh
~/Programme/kicad-10.sh
```

`scripts/kicad-10.sh` sets `TMPDIR=/tmp` and launches the AppImage from
`~/Programme/` (or `KICAD_10_APPIMAGE`). Desktop entry:
`Exec=$HOME/Programme/kicad-10.sh`.

4. Open the PCB editor (a board must be loaded).
5. **Preferences → Plugins → Enable IPC API**, restart KiCad.
6. `board_summary` must show `10.0.x` and `net_ipc_persists: true`.
   Toggle the Cursor MCP server if 9 was running before.

Without the IPC API every tool fails (socket error).

`export_manufacturing` needs `kicad-cli` **from KiCad 10**
(`/tmp/.mount_kicad*/bin/kicad-cli`), not the system 9 binary.

## 4. Cursor / MCP

After the `.deb` install, copy the contents of
`/usr/share/kicad-mcp/cursor-setup/` into the Cursor project folder
(`.cursor/` and `.cursorignore`).

`mcp.json` launches `/usr/bin/kicad-mcp --allow-ai-write`. From a
source tree, point at `target/release/kicad-mcp` or
`target/debug/kicad-mcp` — **not** `cargo run`. After Rust changes:
rebuild, then toggle the MCP server off/on.

Without `--allow-ai-write` every write tool refuses.

`.cursorignore` hides `.kicad_pcb` / `.kicad_sch` so the model cannot
edit them by hand.

## 5. Coordinates and outline

- Units: **millimetres**, KiCad origin, **+x right, +y up**.
- The pink A4 frame is the **drawing sheet** (297 × 210 mm), not the PCB.
- The PCB is **Edge.Cuts** (`set_board_outline`).
- Rectangle: `width_mm` / `height_mm`. Origin = bottom-left. If origin
  is omitted, the rectangle is centred on the sheet, not at 0,0.
- Polygon: `points: [{x_mm, y_mm}, ...]` (max 400, closed automatically).
- Default `replace=true` deletes existing Edge.Cuts (including when
  KiCad reports the layer as `BL_Edge_Cuts`) and refills copper zones.

`clear_board` deletes footprints, tracks, vias, zones — Edge.Cuts stays.

## 6. Parts

1. `download_lcsc_part` with a C-number (e.g. `C5348912`) writes EasyEDA
   geometry into `jlcpcb_parts.pretty` next to the open board.
2. `list_parts` names the templates `place_footprint` wants.
3. `place_footprint` / `place_parts` (max 150, one undo) /
   `place_matrix` (grid: origin = cell 0,0, +x columns, +y rows,
   pitch centre-to-centre).

Placement checks F.CrtYd overlap between footprints. There is no
`move_footprint`: remove then place. Do not substitute a generic KiCad
library footprint for an LCSC C-number.

Nested pads are not parent-transformed. `place.rs` bakes board
millimetres into every pad. Without that bake, copper piles up at the
sheet corner (0,0) while the API still claims the part is in the middle.

## 7. Nets

`connect_pins` / `connect_many` set **Pad.net** (ratsnest), not copper.
Daisy-chain: omit `net`. The net is spliced into the parent
FootprintInstance — a free-pad UpdateItems is rejected. On **KiCad 10**
the name persists after save; `get_nets` / `check_board` must show it.

## 8. Copper

- `add_track` / `add_tracks` (max 150, one undo)
- `add_via` / `add_vias` (max 150)
- `set_copper_zone` — rectangle or polygon; net e.g. `5V` / `GND`;
  layer `F.Cu` or `B.Cu`; then refill
- `ripup_wire` — `segment_id` from `get_routing_scene`

No autorouter. Do not send parallel copper writes: KiCad `BeginCommit`
races.

## 9. Tool catalogue

| Tool | Kind |
| --- | --- |
| `board_summary` | Read — version, counts |
| `get_footprints` | Read — ref, position, rotation |
| `get_nets` | Read — nets and pads |
| `get_routing_scene` | Read — tracks/vias + ids |
| `list_parts` | Read — templates including builtins |
| `check_board` | Read — pads with empty net |
| `download_lcsc_part` | Write — EasyEDA → library |
| `place_footprint` | Write |
| `place_parts` | Write — batch |
| `place_matrix` | Write — grid |
| `remove_footprint` | Write |
| `clear_board` | Write — parts + copper, outline stays |
| `set_board_outline` | Write — Edge.Cuts |
| `connect_pins` / `connect_many` | Write — ratsnest |
| `add_track` / `add_tracks` | Write — track |
| `add_via` / `add_vias` | Write — via |
| `set_copper_zone` | Write — pour + refill |
| `ripup_wire` | Write |
| `save_board` | Write — only when asked |
| `export_manufacturing` | Write — JLCPCB: `*_gerbers.zip` + `*_bom.csv` + `*_cpl.csv` (`kicad-cli`) |

## 10. Save and undo

Every write lands on KiCad's undo stack (**Ctrl+Z**). Call `save_board`
only when the human asks.

`export_manufacturing` saves the open board, refills zones, and writes
three JLCPCB files into the project folder (or `out_dir`):

| File | JLCPCB slot |
| --- | --- |
| `<name>_gerbers.zip` | PCB / Gerbers (copper, mask, silk, paste, Edge.Cuts, drill). **No** silkscreen reference designators (U1, C3, …) and no value text — on a dense grid they sit on pads/holes and fail JLCPCB silk-to-pad / silk-to-hole DFM. References live in the BOM and CPL. |
| `<name>_bom.csv` | BOM (Comment, Designator, Footprint, LCSC Part #) |
| `<name>_cpl.csv` | CPL / centroid (Designator, Mid X, Mid Y, Layer, Rotation) |

Builtin wire pads and M3 holes are omitted from BOM/CPL. Needs
`kicad-cli` (the `kicad` package).

## 11. Building the Debian package

```bash
cargo install cargo-deb --locked
dist/make_beta_package.sh
```

Produces `dist/kicad-mcp_<version>_amd64.deb` (binary + docs + Cursor
setup). Do not commit the `.deb` — attach it as a GitHub Release asset.

## 12. Troubleshooting

| Symptom | Cause / fix |
| --- | --- |
| connect / No such file | KiCad closed, or IPC API off |
| `api.sock` missing (AppImage 10) | Started the `.AppImage` directly — use `~/Programme/kicad-10.sh` (`TMPDIR=/tmp`) |
| Write refused | `--allow-ai-write` missing |
| Copper at the sheet corner | Pad-bake missing — regression in `place.rs` |
| `replaced_segments: 0` | Stale MCP binary; reload MCP after a Rust fix |
| Hole in the copper zone | Inner Edge.Cuts was a cutout; refill zones (B in KiCad) or `set_board_outline` again |
| `net_count: 1`, many unconnected / `net_ipc_persists: false` | Wrong KiCad (9) — use `~/Programme/kicad-10.sh` |
| Commit already in progress | Do not send copper batches in parallel |

## 13. Deliberate limits

- No autorouter, no schematic editor, no `move_footprint`.
- No editing `.kicad_pcb` through this server.
- Max 150 parts / tracks / vias per undo batch.
- Polygon outline max 400 points.
- Target is **KiCad 10**. System 9 is geometry-only; nets do not persist.
