# kicad-mcp — getting started

You keep **KiCad** open. In **Cursor** you say what belongs on the
board. The assistant places parts, assigns nets, pours copper, and
writes the JLCPCB zip — on the board you already see.

Not a second editor. Not “chat, invent a `.kicad_pcb`”. Undo is always
**Ctrl+Z** in KiCad. Save only when you ask.

I built kicad-mcp to make a replacement LED panel for a Besseler 4×5
enlarger. The board is in
[contrib/aristo-d2-led-panel](../contrib/aristo-d2-led-panel).

Deutsch: [ANLEITUNG_FUER_ANFAENGER.md](../ANLEITUNG_FUER_ANFAENGER.md).
Full Debian install: [INSTALL_DEBIAN.en.md](INSTALL_DEBIAN.en.md).
Reference: [MANUAL.md](MANUAL.md). Landing page: [README](../README.md).

---

## Who

Hobby, class project, a LED matrix, wire pads, a GND pour, files
JLCPCB will take.

You own the schematic idea. Pin names come from LCSC/EasyEDA, not from
the model’s memory.

Not aimed at Altium, Windows-only setups, or people who do not want an
assistant in the editor.

---

## 1. KiCad 10

On Debian always start **`kicad-10`** (the `.deb` installs
`/usr/bin/kicad-10`). Not system KiCad 9, not a double-clicked
`.AppImage`.

1. Open the **PCB editor** (an empty board is enough).
2. **Preferences → Plugins → Enable IPC API**.
3. Quit KiCad fully, start `kicad-10` again, open the PCB editor.

Without that checkbox the assistant cannot reach KiCad.

---

## 2. Package

From [Releases](https://github.com/Draganito/kicad-mcp/releases)
download `kicad-mcp_*.deb`:

```bash
sudo apt install ./kicad-mcp_*_amd64.deb
```

The autorouter (`kicad-routing-tools_*.deb`) is **optional**. If you
want it: install both, then run `kicad-routing-tools-setup` once as
your user. The two packages do not call each other.

---

## 3. Cursor

Copy the template into the folder you open in Cursor (`.cursor/`
**and** `.cursorignore`):

```bash
cp -a /usr/share/kicad-mcp/cursor-setup/.cursor \
      /usr/share/kicad-mcp/cursor-setup/.cursorignore .
```

In Cursor, toggle the `kicad-mcp` server off and on.

---

## 4. First sentence

KiCad running, a board open, Cursor on that folder:

> Call `board_summary`.

You want **10.x**, `has_open_board: true`, and
`net_ipc_persists: true`. If it shows 9.x you started the wrong KiCad
— use `kicad-10`.

Then, for example:

> Make a 40 × 30 mm board. Download C14663. Place one cap in the
> middle. Don’t save yet.

---

## How a board is usually built

1. **Outline** — the yellow Edge.Cuts *is* the PCB. The pink A4 frame
   is only the drawing sheet.
2. **Parts** — fetch an LCSC C-number, then place or a grid.
3. **Nets** — ratsnest first. Copper later.
4. **Copper** — tracks, vias, 4 layers if you need them, GND/5V pours.
5. **Silk** — `5V` / `GND` / `DATA` next to wire pads, not on copper.
6. **Check** — empty pads, DRC, short physics review (`review_board`).
7. **Order** — Gerber zip + BOM + pick-and-place for JLCPCB. Silk has
   no U1/C3 (otherwise JLCPCB DFM complains).

Millimetres, **+x right, +y up**. Do not edit `.kicad_pcb` by hand.

Tool names live in the [manual](MANUAL.md), not in every chat.

---

## If something fails

| What you see | Usual cause |
| --- | --- |
| MCP cannot connect | IPC API off, or PCB editor closed |
| Version 9 / nets empty | System KiCad 9 instead of `kicad-10` |
| Writes refused | Template not copied (`--allow-ai-write`) |
| Parts in the sheet corner | Pad-coordinate bug — not you |
| AppImage, no socket | `.AppImage` started directly instead of `kicad-10` |

More: [MANUAL.md](MANUAL.md) → “Troubleshooting”.
