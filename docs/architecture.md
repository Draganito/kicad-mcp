# Architecture

kicad-mcp is a small Rust workspace that talks to a **running KiCad PCB
editor** over the official IPC API (protobuf / NNG). Cursor launches the
binary as a **stdio MCP** server.

```text
Cursor  --stdio MCP-->  kicad-mcp  --KiCad IPC-->  pcbnew (KiCad 10)
                              |
                              +-- easyeda-kicad --> LCSC/EasyEDA HTTP
```

## Crates

- `kicad-mcp` — MCP tools, IPC session, placement, nets, copper, silk, review, outline.
- `easyeda-kicad` — fetch an LCSC C-number and emit `.kicad_mod` /
  `.kicad_sym` / `{template}.pins.json` (Y-flip EasyEDA → KiCad).
  Pin names/functions are EasyEDA SVG names, not a manufacturer PDF.

There is no in-process board model. KiCad owns the file. Writes go
through `CreateItems` / `UpdateItems` / `DeleteItems` inside a KiCad
commit (one Ctrl+Z).

## Coordinates

KiCad native millimetres, origin = board origin, +x right, +y up. The
A4 frame is the drawing sheet; the PCB is Edge.Cuts.

## Placement

`CreateItems` stores `FootprintInstance.position` but draws nested pads
at the raw coordinates in the protobuf. `place.rs` bakes board
millimetres into every pad (and silk/fab text). Dropping that bake
puts copper at the sheet corner.

## Nets (KiCad 10)

`connect_pins` splices `Pad.net` into the parent footprint and
`UpdateItems` it. `disconnect_pin` splices the same field back to
unconnected (code 0). Every pad that shares the pin number is assigned
or cleared (thermal clusters). A free-pad update is rejected. KiCad 10 persists
`Pad.net` / `Track.net`. `board_summary.net_ipc_persists` is true from
major version 10.

Start 10 with `kicad-10` (`/usr/bin/kicad-10` from the .deb, source
`scripts/kicad-10.sh`). The wrapper forces `TMPDIR=/tmp` so the NNG
socket is `/tmp/kicad/api.sock`. Starting the `.AppImage` or
`/usr/bin/kicad` (9) breaks nets or the socket.

The optional `kicad-routing-tools` .deb is a Pcbnew plugin. `autoroute_nets`
runs its CLI (`py_router/route.py`) with pinned JLCPCB floors, reloads
the open board via `RevertDocument`, and refills copper zones. It does
not press the wx Route button. `check_drc` shells out to
`kicad-cli pcb drc` (same binary as gerber export).

## Outline replace

GetItems reports Edge.Cuts as proto name `BL_Edge_Cuts` (layer id 47),
not the UI name `Edge.Cuts`. `set_board_outline` matches both, deletes,
recreates, then refills zones. An inner closed Edge.Cuts path is a
**cutout**; leftover rectangles punch holes in copper pours.

## Packaging

`dist/make_beta_package.sh` builds `target/release/kicad-mcp` and runs
`cargo deb -p kicad-mcp`. The `.deb` ships `/usr/bin/kicad-mcp`, docs,
and `contrib/cursor-setup`. The Aristo D2 LED panel that started the
tool is `contrib/aristo-d2-led-panel`. Live scratch boards under
`kicad_projekte/` and `.cursor/` stay out of git.

## Manufacturing export

`export_manufacturing` saves the open board, then shells out to
`kicad-cli pcb export gerbers|drill|pos` (KiCad's plotter — this crate
does not parse `.kicad_pcb`). Gerber silk is plotted with
`--exclude-refdes` and `--exclude-value` so JLCPCB DFM does not report
silkscreen-to-pad / silkscreen-to-hole on dense boards; designators
stay in BOM/CPL only. BOM rows are grouped from live footprints
(LCSC C-number from the EasyEDA template name). CPL is the KiCad
position CSV rewritten to JLCPCB columns:
`<stem>_gerbers.zip`, `<stem>_bom.csv`, `<stem>_cpl.csv`.

