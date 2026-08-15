# Architecture

kicad-mcp is a small Rust workspace that talks to a **running KiCad PCB
editor** over the official IPC API (protobuf / NNG). Cursor launches the
binary as a **stdio MCP** server.

```text
Cursor  --stdio MCP-->  kicad-mcp  --KiCad IPC-->  pcbnew (KiCad 9/10)
                              |
                              +-- easyeda-kicad --> LCSC/EasyEDA HTTP
```

## Crates

- `kicad-mcp` — MCP tools, IPC session, placement, nets, copper, outline.
- `easyeda-kicad` — fetch an LCSC C-number and emit `.kicad_mod` /
  `.kicad_sym` (Y-flip EasyEDA → KiCad).

There is no in-process board model. KiCad owns the file. Writes go
through `CreateItems` / `UpdateItems` / `DeleteItems` inside a KiCad
commit (one Ctrl+Z).

## Coordinates

KiCad native millimetres, origin = board origin, +x right, +y up. The
A4 frame is the drawing sheet; the PCB is Edge.Cuts.

## Placement (KiCad 9)

`CreateItems` stores `FootprintInstance.position` but draws nested pads
at the raw coordinates in the protobuf. `place.rs` bakes board
millimetres into every pad (and silk/fab text). Dropping that bake
puts copper at the sheet corner.

## Nets (KiCad 9.0.2)

`connect_pins` splices `Pad.net` into the parent footprint and
`UpdateItems` it. A free-pad update is rejected. 9.0.2 does not persist
`Pad.net` / `Track.net` after a successful IPC update. Geometry does
persist. KiCad 10 is the intended target for nets.

## Outline replace

GetItems reports Edge.Cuts as proto name `BL_Edge_Cuts` (layer id 47),
not the UI name `Edge.Cuts`. `set_board_outline` matches both, deletes,
recreates, then refills zones. An inner closed Edge.Cuts path is a
**cutout**; leftover rectangles punch holes in copper pours.

## Packaging

`dist/make_beta_package.sh` builds `target/release/kicad-mcp` and runs
`cargo deb -p kicad-mcp`. The `.deb` ships `/usr/bin/kicad-mcp`, docs,
and `contrib/cursor-setup`. Live boards under `kicad_projekte/` and
`.cursor/` (local debug `mcp.json`) stay out of git.
