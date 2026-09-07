# MILUKA Aristo D2 replacement LED panel

A 4-layer, Ø 157 mm LED head for a **Beseler 4×5** enlarger — the
board that started [kicad-mcp](../../README.md). I needed this panel,
had never designed a PCB, and did not want to operate KiCad by hand.

109 × SK6812 (C5348912), local 0603 caps, two 220 µF bulk caps, TVS,
level shifter, 5V / GND / DATA wire pads (2.8 mm pad / 1.4 mm drill),
four M3 holes. The gerbers here are the set sent to JLCPCB
(31 Aug 2026), including the load table on B.Silkscreen.

The first board is back from JLCPCB, soldered, in the printed
[holder](../panel-holder-freecad/panel-holder.FCStd), and tested —
all 109 LEDs run.

![JLCPCB parcel](jlcpcb_box.jpg)

![Assembled panel, DATA input and level shifter](panel_smt_data.jpg)

![Panel in the printed holder](panel_in_holder.jpg)

![Back of the holder](holder_back.jpg)

![All 109 LEDs on (blue)](panel_lit_blue.jpg)

Factory SMT view from the JLCPCB order:

![MILUKA Aristo D2 replacement LED panel, JLCPCB SMT top](jlcpcb_smt_top.jpg)

## Open it

KiCad **10**, PCB editor. Open `led_panel_4x5.kicad_pro`. Footprints
live in `jlcpcb_parts.pretty/` (already in the project library table).

Do not edit the `.kicad_pcb` in a text editor.

## Order from JLCPCB

Use the files next to the project:

- `led_panel_4x5_gerbers.zip` — Gerbers + drill
- `led_panel_4x5_bom.csv` — BOM (LCSC)
- `led_panel_4x5_cpl.csv` — pick & place

How to click through the site: [JLCPCB_Order_Guide.pdf](JLCPCB_Order_Guide.pdf).
