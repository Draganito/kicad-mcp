# kicad-mcp — Manual

Complete A-to-Z reference, v0.1.0.
German: [HANDBUCH.md](HANDBUCH.md).
Fresh Debian + Cursor: [INSTALL_DEBIAN.en.md](INSTALL_DEBIAN.en.md).
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
this is not a second layout program. `autoroute_nets` can run the
companion CLI for named nets.

JLCPCB footprints come from **EasyEDA / LCSC** (`download_lcsc_part`).
Wire pads and mounting holes are generated parametrically:
`list_parts` writes the defaults (`WirePad_PTH` 2.5/1.5 mm,
`MountingHole_M3_NPTH` 3.2 mm); `make_wire_pad` / `make_mounting_hole`
write any other size (e.g. `WirePad_PTH_3.2_2`, `MountingHole_4.5_NPTH`).

License: AGPL-3.0-only. KiCad itself is a separate GPL-3.0 program and
is not shipped in this package.

## 2. Install

### Prebuilt package

From [GitHub Releases](https://github.com/Draganito/kicad-mcp/releases)
both files (MCP + optional autorouter):

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb ./kicad-routing-tools_0.20.4-2_amd64.deb
kicad-routing-tools-setup
```

Step by step: [INSTALL_DEBIAN.en.md](INSTALL_DEBIAN.en.md).
Debian/Ubuntu x86-64, glibc 2.39+. **KiCad 10** must be running
(`recommends: kicad` is only the distro package — on Debian 13 that is
9.0.2 and is not enough for nets). After the MCP `.deb`, `kicad-10`
starts the AppImage with `TMPDIR=/tmp`.

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
kicad-10
```

`/usr/bin/kicad-10` (from the `.deb`, source `scripts/kicad-10.sh`) sets
`TMPDIR=/tmp` and launches the AppImage from `~/Programme/` (or
`KICAD_10_APPIMAGE`). Menu entry: **KiCad 10 (AppImage)**.

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
   geometry into `jlcpcb_parts.pretty` next to the open board and
   returns `pins: [{number, pin_name}]` (EasyEDA function).
2. `list_parts` names the templates `place_footprint` wants.
   `get_part_pins` rereads those EasyEDA names for an already downloaded
   template (`{template}.pins.json`).
3. `place_footprint` / `place_parts` (max 150, one undo) /
   `place_matrix` (grid: origin = cell 0,0, +x columns, +y rows,
   pitch centre-to-centre).

Placement checks F.CrtYd overlap between footprints. `move_footprint`
relocates and/or rotates a placed part in one undo: a rigid transform
of the anchor and every baked pad, so nets, reference and padstack
geometry survive (no remove+place). The target is courtyard-checked.
Copper does **not** move — re-route tracks that reached the part.
Do not substitute a generic KiCad library footprint for an LCSC
C-number.

`get_pads` reports every pad as hard data straight from KiCad's baked
protos: reference, pin, net, absolute x/y, size, rotation, smd/pth/npth,
shape, layer, drill; filter by `reference` and/or `net`. Use it to
verify placement and orientation (a mirrored or mis-rotated part shows
pads on the wrong side of the anchor) instead of guessing from
templates or renders.

`check_placement` turns that into a hard OK/fail audit: every pad is
recomputed from its `jlcpcb_parts` template at the footprint's anchor +
rotation and compared against the baked board pads. A mirrored,
mis-rotated or stale-baked part (placed by an older kicad-mcp) fails
with per-pad deltas in mm — pin, expected vs actual position, plus
size/angle/type/drill mismatches (a lost NPTH hole or a slot baked as
a round hole fails too). Thermal clusters that share a pin number
are matched by nearest position. Optional `reference` filter,
`tolerance_mm` default 0.01. Footprints without a template on disk are
listed as `skipped`, not failed. Run it after placing or moving parts;
trust it over any render.

Nested pads are not parent-transformed. `place.rs` bakes board
millimetres into every pad. Without that bake, copper piles up at the
sheet corner (0,0) while the API still claims the part is in the middle.

## 7. Nets

Pin names and functions come from **EasyEDA** (`download_lcsc_part` /
`get_part_pins`), not from datasheet memory and not from Alladin
`pad_nets`. `connect_pins` / `connect_many` set **Pad.net** (ratsnest),
not copper. Every pad that shares the pin number is assigned (thermal
clusters, e.g. ESP32 pad 41). Daisy-chain: omit `net`. The net is
spliced into the parent FootprintInstance — a free-pad UpdateItems is
rejected. On **KiCad 10** the name persists after save; `get_nets` /
`check_board` must show it. `disconnect_pin` / `disconnect_many` clear
that assignment back to unconnected (same splice, code 0). Idempotent
if the pin is already open. Does not rip copper.

A manufacturer PDF is allowed only after a **logic check** shows the
EasyEDA names cannot be right (example: WROOM pad 1 named `IO20` while
pin 1 is the module GND corner). Then fetch hard facts (`datasheet_url`
if EasyEDA sent one), name the contradiction, and net from that. 0603
parts whose EasyEDA names are only `1`/`2` have no polarity — GND vs
rail from the companion pad.

## 8. Copper

- `add_track` / `add_tracks` (max 150, one undo)
- `add_via` / `add_vias` (max 150)
- `stitch_via` — GND via + F.Cu stub next to a pin or every SMD pad on
  a net (`net: "GND"`)
- `set_copper_layers` — copper count 2/4/6/8 (not undoable)
- `set_copper_zone` — rectangle or polygon; net e.g. `5V` / `GND`;
  layer `F.Cu` / `In1.Cu` / `In2.Cu` / `B.Cu`; pads solid unless `thermal=true` (PTH) or `thermal_smd=true` (SMD+PTH); `remove_islands=true` drops isolated slivers; then refill
- `clear_zones` — delete copper zones (tracks stay)
- `ripup_wire` — `segment_id` from `get_routing_scene`
- `autoroute_nets` — named nets via the plugin CLI, then reload and
  refill zones
- `check_drc` — `kicad-cli pcb drc` (clearance, silk, holes); saves
- `review_board` — read only: GND/power pour, adjacent layers, via
  at each cap GND (3 mm). Not DRC, not 90° corners. Before “done”.

### Silk text

`add_text` / `add_texts` (max 150, one undo) places board text on
**F.Silkscreen** (default) or **B.Silkscreen**. Use it for connector
names (`5V`, `GND`, `DATA`) next to wire pads. Never F.Cu, never the
footprint Value (export already strips U1/C3). Size default 1.0 mm
(min 0.8). `clear_board` deletes these labels.

`autoroute_nets` runs the optional KiCad Routing Tools **CLI** (not the
wx dialog). Needs `kicad-routing-tools-setup` and KiCad 10 via `kicad-10`.
`nets` is required — never `*` / every net. GND/VSS are refused (pour a
zone). USB_DN and USB_DP must be passed together (two singles plus
length-match, not `route_diff.py`). Optional `track_width_mm`,
`via_size_mm`, `via_drill_mm`, `clearance_mm` — defaults pin JLCPCB-safe
floors (0.2 mm clearance, 0.6/0.3 via) so the CLI cannot silently drop
to 0.127 mm. The step saves, writes the file, reloads, and refills
zones — **no Ctrl+Z**. Use it only when the human wants autorouting or
a full A–Z board including copper. Do not send parallel copper writes:
KiCad `BeginCommit` races.

## 9. Tool catalogue

| Tool | Kind |
| --- | --- |
| `board_summary` | Read — version, counts |
| `get_footprints` | Read — ref, position, rotation |
| `get_nets` | Read — nets and pads |
| `get_pads` | Read — baked pad truth (position, net, rotation) |
| `check_placement` | Read — hard OK/fail: template pads vs baked pads |
| `get_routing_scene` | Read — tracks/vias + ids |
| `list_parts` | Read — templates including builtins |
| `get_part_pins` | Read — EasyEDA `number` + `pin_name` |
| `check_board` | Read — pads with empty net |
| `review_board` | Read — short physics report (pour, return path, cap via) |
| `download_lcsc_part` | Write — EasyEDA → library + pins |
| `make_wire_pad` | Write — parametric PTH wire pad template |
| `make_mounting_hole` | Write — parametric NPTH hole template |
| `place_footprint` | Write |
| `place_parts` | Write — batch |
| `place_matrix` | Write — grid |
| `move_footprint` | Write — rigid move/rotate, nets stay, copper stays |
| `remove_footprint` | Write |
| `clear_board` | Write — parts + copper + silk text, outline stays |
| `clear_zones` | Write — zones only |
| `set_board_outline` | Write — Edge.Cuts |
| `add_text` / `add_texts` | Write — silk label (F/B.Silkscreen) |
| `connect_pins` / `connect_many` | Write — ratsnest (every same-number pad) |
| `disconnect_pin` / `disconnect_many` | Write — pin back to unconnected |
| `add_track` / `add_tracks` | Write — track |
| `add_via` / `add_vias` | Write — via |
| `stitch_via` | Write — GND via + stub |
| `set_copper_layers` | Write — copper count 2/4/6/8 |
| `set_copper_zone` | Write — pour + refill |
| `autoroute_nets` | Write — CLI autorouter, named nets |
| `ripup_wire` | Write |
| `check_drc` | Write — `kicad-cli` DRC (saves) |
| `render_board` | Write — 3D PNG via `kicad-cli pcb render` (saves) |
| `save_board` | Write — only when asked |
| `export_manufacturing` | Write — JLCPCB: `*_gerbers.zip` + `*_bom.csv` + `*_cpl.csv` (`kicad-cli`) |

`render_board` refills zones, saves, and raytraces the board to
`<stem>_render_<side>.png` via `kicad-cli pcb render` (side
top/bottom/left/right/front/back; optional `zoom`, `rotate [x,y,z]`,
`perspective`, `floor`, `width`/`height`). Copper, silkscreen, mask and
holes are real; EasyEDA footprints carry no 3D bodies, so package
orientation is verified with `get_pads`, not the render.

## 10. Save and undo

Every write lands on KiCad's undo stack (**Ctrl+Z**), except
`autoroute_nets` (file reload). Call `save_board` only when the human
asks; `autoroute_nets` and `check_drc` save internally.

`export_manufacturing` saves the open board, refills zones, and writes
three JLCPCB files into the project folder (or `out_dir`):

| File | JLCPCB slot |
| --- | --- |
| `<name>_gerbers.zip` | PCB / Gerbers (copper, mask, silk, paste, Edge.Cuts, drill). **No** silkscreen reference designators (U1, C3, …) and no value text — on a dense grid they sit on pads/holes and fail JLCPCB silk-to-pad / silk-to-hole DFM. References live in the BOM and CPL. |
| `<name>_bom.csv` | BOM (Comment, Designator, Footprint, LCSC Part #) |
| `<name>_cpl.csv` | CPL / centroid (Designator, Mid X, Mid Y, Layer, Rotation) |

Generated wire pads and mounting holes (any size) are omitted from
BOM/CPL. Needs `kicad-cli` (the `kicad` package).

## 11. Building the Debian package

```bash
cargo install cargo-deb --locked
dist/make_beta_package.sh
```

Produces `dist/kicad-mcp_<version>_amd64.deb` (binary + `kicad-10` +
docs + Cursor setup) and `dist/kicad-routing-tools_*.deb`. Do not
commit the `.deb` files — attach them as GitHub Release assets.
MCP only: `SKIP_ROUTING_TOOLS=1 dist/make_beta_package.sh`.

## 12. Troubleshooting

| Symptom | Cause / fix |
| --- | --- |
| connect / No such file | KiCad closed, or IPC API off |
| `api.sock` missing (AppImage 10) | Started the `.AppImage` directly — use `kicad-10` (`TMPDIR=/tmp`) |
| Write refused | `--allow-ai-write` missing |
| Copper at the sheet corner | Pad-bake missing — regression in `place.rs` |
| `replaced_segments: 0` | Stale MCP binary; reload MCP after a Rust fix |
| Hole in the copper zone | Inner Edge.Cuts was a cutout; refill zones (B in KiCad) or `set_board_outline` again |
| `net_count: 1`, many unconnected / `net_ipc_persists: false` | Wrong KiCad (9) — use `kicad-10` |
| Commit already in progress | Do not send copper batches in parallel |
| `check_drc` cannot find kicad-cli | Start KiCad 10 via `kicad-10` (AppImage CLI under `/tmp/.mount_kicad*`) |

## 13. Deliberate limits

- No schematic editor. Autorouter only via `autoroute_nets`
  (named nets, companion deb).
- No editing `.kicad_pcb` through this server.
- Max 150 parts / tracks / vias per undo batch.
- Polygon outline max 400 points.
- Target is **KiCad 10**. System 9 is geometry-only; nets do not persist.
