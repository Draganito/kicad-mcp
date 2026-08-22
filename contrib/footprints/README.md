# Shared footprints

Hand-verified `.kicad_mod` files from real boards built with kicad-mcp.
Drop the file into your project's `jlcpcb_parts.pretty/` folder and it
shows up in `list_parts`.

## C165948_USB-C_SMD-TYPE-C-31-M-12_1

USB-C 2.0 receptacle **TYPE-C-31-M-12** (HRO / Korean Hroparts Elec),
LCSC **C165948** — the part JLCPCB stocks for basic assembly.

Checked dimension by dimension against the HRO "RECOMMEND P.C.B LAYOUT
(COMPONENT SIDE)" drawing (tolerance ±0.05 mm):

- 12 signal pads: 8×0.30 mm + 4×0.60 mm wide, 1.64 mm tall, 0.50 mm
  pitch, spans 2.50 / 3.50 / 4.80 / 6.40 mm
- 4 shield mounts: 0.90 mm pads with **oblong slots** — 0.60×1.70 mm
  (top pair), 0.60×1.40 mm (bottom pair), 8.65 mm apart, 4.18 mm
  between rows
- 2 positioning pegs: Ø0.60 mm **NPTH**, 5.78 mm apart, 0.50 mm below
  the pad row

Needs kicad-mcp ≥ 0.1.0-11: older versions dropped the NPTH pegs and
collapsed the oblong slots to 0.6 mm round holes when baking pads onto
the board.
