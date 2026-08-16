# kicad-mcp — Einstieg für Anfänger

In 15 Minuten von der `.deb` bis zur ersten Platine in KiCad, gesteuert
aus Cursor. Komplette Neuinstallation (AppImage + beide Debs + Cursor):
[docs/INSTALL_DEBIAN.md](docs/INSTALL_DEBIAN.md).
Nachschlagewerk: [docs/HANDBUCH.md](docs/HANDBUCH.md).

## 1. KiCad

1. **KiCad 10** starten — auf Debian immer `kicad-10` (liegt nach der
   `.deb` in `/usr/bin/kicad-10`), nicht das System-KiCad 9 und nicht
   die `.AppImage` direkt. Download: [docs/INSTALL_DEBIAN.md](docs/INSTALL_DEBIAN.md).
2. **PCB-Editor** öffnen (eine leere Platine reicht).
3. **Preferences → Plugins → Enable IPC API** einschalten.
4. KiCad **neu starten**, PCB-Editor wieder öffnen.

Ohne diesen Haken kann kicad-mcp KiCad nicht erreichen.
`board_summary` muss `10.0.x` und `net_ipc_persists: true` zeigen.

## 2. Pakete installieren

Von der [Releases-Seite](https://github.com/Draganito/kicad-mcp/releases)
beide Dateien laden:

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb ./kicad-routing-tools_0.20.4-2_amd64.deb
kicad-routing-tools-setup
```

`kicad-routing-tools` ist optional (Autorouter in Pcbnew). Setup als
normaler User, nicht als root. MCP und Plugin rufen sich nicht auf.

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
2. `download_lcsc_part` — LCSC-C-Nummer (z. B. C14663); Antwort enthält
   EasyEDA-Pins (`number` + `pin_name`). Schon geladen:
   `get_part_pins`. Netze danach, Datenblatt nur wenn EasyEDA logisch
   nicht stimmen kann.
3. `place_footprint` / `place_parts` / `place_matrix`
4. `connect_many` — Netze (Ratsnest). Falsch verdrahtet:
   `disconnect_pin` (Pad wieder unconnected).
5. `add_track` / `set_copper_zone` — Kupfer; oder `autoroute_nets`
   mit genannten Signalnetzen (nicht GND)
6. `check_board` — bevor die KI „fertig“ sagt; nach Kupfer auch `check_drc`
7. `export_manufacturing` — Gerber-Zip + BOM + CPL für JLCPCB
   (Silk ohne U1/C3-Beschriftung — sonst DFM „Silkscreen to pad“)
8. `save_board` — **nur wenn du es willst**

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
| Netze leer / `net_ipc_persists: false` | System-KiCad 9 statt `kicad-10` |
| AppImage 10, kein Socket | `.AppImage` direkt gestartet statt `kicad-10` |
| Plugin: encodings / pip fail | `kicad-routing-tools-setup` als User, nicht pip in KiCad |
| Altes Edge.Cuts bleibt | `replace` muss true sein (Default); MCP neu laden nach einem Update |

Mehr: [docs/HANDBUCH.md](docs/HANDBUCH.md) Kapitel „Probleme lösen“.
