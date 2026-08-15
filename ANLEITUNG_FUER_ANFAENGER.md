# kicad-mcp — Einstieg für Anfänger

In 15 Minuten von der `.deb` bis zur ersten Platine in KiCad, gesteuert
aus Cursor. Das Nachschlagewerk ist [docs/HANDBUCH.md](docs/HANDBUCH.md).

## 1. KiCad

1. KiCad 9 oder 10 installieren und den **PCB-Editor** öffnen
   (eine leere Platine reicht).
2. **Preferences → Plugins → Enable IPC API** einschalten.
3. KiCad **neu starten**, PCB-Editor wieder öffnen.

Ohne diesen Haken kann kicad-mcp KiCad nicht erreichen.

## 2. Paket installieren

Von der [Releases-Seite](https://github.com/Draganito/kicad-mcp/releases)
`kicad-mcp_<version>_amd64.deb` laden:

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb
```

## 3. Cursor anbinden

Den **Inhalt** von `/usr/share/kicad-mcp/cursor-setup/` in den Ordner
kopieren, den du in Cursor öffnest — also `.cursor/` **und**
`.cursorignore`:

```bash
cp -a /usr/share/kicad-mcp/cursor-setup/.cursor /usr/share/kicad-mcp/cursor-setup/.cursorignore .
```

Cursor neu laden bzw. den MCP-Server `kicad-mcp` einmal aus- und
einschalten. `mcp.json` startet `/usr/bin/kicad-mcp --allow-ai-write`.

## 4. Erster Check

In Cursor die KI bitten: *„board_summary aufrufen“*. Antwort sollte
KiCad-Version, Projektpfad und `has_open_board: true` enthalten.

Danach typischer Ablauf:

1. `set_board_outline` — Rechteck oder Polygon auf Edge.Cuts
2. `download_lcsc_part` — LCSC-C-Nummer (z. B. C14663)
3. `place_footprint` / `place_parts` / `place_matrix`
4. `connect_many` — Netze (Ratsnest)
5. `add_track` / `set_copper_zone` — Kupfer
6. `check_board` — bevor die KI „fertig“ sagt
7. `save_board` — **nur wenn du es willst**

Rückgängig in KiCad: **Ctrl+Z**. `.kicad_pcb` nicht von Hand editieren.

## Koordinaten

Millimeter, KiCad-Ursprung, **+x rechts, +y oben**. Der rosa A4-Rahmen
ist nur das Zeichenblatt. Die Platine ist das gelbe **Edge.Cuts**.
Ohne Origin liegt ein Rechteck-Umriss in der Blattmitte, nicht bei 0,0.

## Wenn etwas hakt

| Symptom | Typische Ursache |
| --- | --- |
| MCP verbindet nicht | IPC API aus, oder PCB-Editor nicht offen |
| Write-Tools lehnen ab | `--allow-ai-write` fehlt in `mcp.json` |
| Teile sitzen in der Blatt-Ecke | Pad-Koordinaten nicht gebacken — Bug, nicht du |
| Netze leer nach `connect_pins` | KiCad 9.0.2 speichert Pad.net nicht über IPC |
| Altes Edge.Cuts bleibt | `replace` muss true sein (Default); MCP neu laden nach einem Update |

Mehr: [docs/HANDBUCH.md](docs/HANDBUCH.md) Kapitel „Probleme lösen“.
