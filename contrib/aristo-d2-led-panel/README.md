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

**The complete splitgrade system is under $150** — panel, holder,
SenseCAP controller, TSL2591 meter, XIAO receiver, and a 5 V / 5 A
PSU. That is the point of this build.

![JLCPCB parcel](jlcpcb_box.jpg)

![Assembled panel, DATA input and level shifter](panel_smt_data.jpg)

![Panel in the printed holder](panel_in_holder.jpg)

![Back of the holder](holder_back.jpg)

![All 109 LEDs on (blue)](panel_lit_blue.jpg)

On the Beseler, with the XIAO on the back of the holder:

![Head on the enlarger](head_on_enlarger.jpg)

https://github.com/user-attachments/assets/cf3984b5-ae27-4c1a-a5dd-19e20b47ff94

Factory SMT view from the JLCPCB order:

![MILUKA Aristo D2 replacement LED panel, JLCPCB SMT top](jlcpcb_smt_top.jpg)

## Under $150 — complete splitgrade system

List prices, one of each, no shipping or VAT. Panel is the JLCPCB
assembled board at **$60 / piece**.

| Part | USD | Link |
| --- | ---: | --- |
| LED panel, 109× SK6812, JLCPCB SMT | 60.00 | [Gerbers in this folder](led_panel_4x5_gerbers.zip) |
| [SenseCAP Indicator D1](https://www.seeedstudio.com/SenseCAP-Indicator-D1-p-5643.html) (controller) | 49.00 | [seeedstudio.com](https://www.seeedstudio.com/SenseCAP-Indicator-D1-p-5643.html) |
| [Seeed XIAO ESP32-S3](https://www.seeedstudio.com/XIAO-ESP32S3-p-5627.html) (head) | 7.49 | [seeedstudio.com](https://www.seeedstudio.com/XIAO-ESP32S3-p-5627.html) |
| [Adafruit TSL2591](https://www.adafruit.com/product/1980) (meter) | 6.95 | [adafruit.com/product/1980](https://www.adafruit.com/product/1980) |
| [Waveshare 5 V / 5 A](https://www.waveshare.com/psu-5v5a-5.5-2.1.htm), 5.5×2.1 mm | 6.49 | [waveshare.com](https://www.waveshare.com/psu-5v5a-5.5-2.1.htm) |
| Cable, screws, solder, PLA+ holder | 20.00 | [holder CAD](../panel-holder-freecad/panel-holder.FCStd) |
| **Total** | **149.93** | |

Head firmware (free): [darkroom-enlarger-head](https://github.com/Draganito/darkroom-enlarger-head).
SenseCAP wiki: [hands-on demo](https://wiki.seeedstudio.com/SenseCAP_Indicator_Application_LoRaWAN/#hands-on-demo).

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
