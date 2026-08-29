# kicad-mcp — Einstieg für Maker

Ich habe kicad-mcp gebaut, weil ich für meinen **Besseler 4×5**
(MILUKA Aristo D2) ein Ersatz-LED-Panel brauchte — ohne PCB-Erfahrung
und ohne Lust, KiCad von Hand zu klicken.

Du lässt **KiCad** offen. In **Cursor** sagst du, was auf die Platine
soll. Die KI setzt Teile, vergibt Netze, gießt Kupfer und schreibt den
JLCPCB-Zip — auf der Platine, die du schon siehst.

Kein zweiter Editor. Kein „Chat, erfinde eine `.kicad_pcb`“.
Rückgängig ist immer **Ctrl+Z** in KiCad. Speichern nur, wenn du es
willst.

Das Panel liegt unter
[contrib/aristo-d2-led-panel](contrib/aristo-d2-led-panel)
(109 SK6812, 4 Lagen, bei JLCPCB bestellt).

Englisch: [README](README.md) · [Getting started](docs/GETTING_STARTED.md)

Komplette Neuinstallation (AppImage, Debs, Cursor):
[docs/INSTALL_DEBIAN.md](docs/INSTALL_DEBIAN.md).
Nachschlagewerk: [docs/HANDBUCH.md](docs/HANDBUCH.md).

---

## Für wen

Hobby, Studium, eine LED-Matrix, Drahtpads, eine GND-Fläche, Dateien
die JLCPCB schluckt.

Die Schaltung bleibt deine. Pinnamen kommen von LCSC/EasyEDA, nicht
aus dem Gedächtnis der KI.

Nicht gedacht für Altium, nur-Windows, oder wenn du keine KI im
Editor willst.

---

## 1. KiCad 10

Auf Debian immer **`kicad-10`** starten (liegt nach der `.deb` in
`/usr/bin/kicad-10`). Nicht das System-KiCad 9, nicht die `.AppImage`
doppelklicken.

1. **PCB-Editor** öffnen (eine leere Platine reicht).
2. **Einstellungen → Plugins → IPC-API aktivieren**.
3. KiCad **ganz zu**, wieder `kicad-10`, PCB-Editor erneut öffnen.

Ohne den Haken findet die KI KiCad nicht.

---

## 2. Paket

Von der [Releases-Seite](https://github.com/Draganito/kicad-mcp/releases)
`kicad-mcp_*.deb` laden:

```bash
sudo apt install ./kicad-mcp_*_amd64.deb
```

Der Autorouter (`kicad-routing-tools_*.deb`) ist **optional**. Nur wenn
du ihn willst: mitinstallieren, dann einmal `kicad-routing-tools-setup`
als normaler User. MCP und Plugin rufen sich nicht auf.

---

## 3. Cursor

Inhalt der Vorlage in den Ordner kopieren, den du in Cursor öffnest
(`.cursor/` **und** `.cursorignore`):

```bash
cp -a /usr/share/kicad-mcp/cursor-setup/.cursor \
      /usr/share/kicad-mcp/cursor-setup/.cursorignore .
```

Cursor: MCP-Server `kicad-mcp` einmal aus- und einschalten.

---

## 4. Erster Satz

KiCad läuft, Platine offen, Cursor auf dem Ordner:

> Rufe `board_summary` auf.

Du willst **10.x**, `has_open_board: true` und
`net_ipc_persists: true`. Steht dort 9.x: falsches KiCad — `kicad-10`.

Dann zum Beispiel:

> Mach eine Platine 40 × 30 mm. Lade C14663. Setz einen Elko in die
> Mitte. Noch nicht speichern.

---

## So entsteht eine Platine

1. **Umriss** — das gelbe Edge.Cuts *ist* die Platine. Der rosa
   A4-Rahmen ist nur das Zeichenblatt.
2. **Teile** — LCSC-C-Nummer laden, dann setzen oder ein Raster.
3. **Netze** — erst nur Ratsnest (Luftlinien). Kupfer kommt danach.
4. **Kupfer** — Bahnen, Vias, bei Bedarf 4 Lagen, GND/5V als Fläche.
5. **Silk** — `5V` / `GND` / `DATA` neben die Drahtpads, nicht aufs
   Kupfer.
6. **Prüfen** — leere Pads, DRC, kurzer Physik-Report
   (`review_board`).
7. **Bestellen** — Gerber-Zip + BOM + Bestückung für JLCPCB.
   Silk ohne U1/C3 (sonst meckert JLCPCB).

Millimeter, **+x rechts, +y oben**. `.kicad_pcb` nicht von Hand
editieren.

Die Werkzeugnamen stehen im [Handbuch](docs/HANDBUCH.md), nicht in
jedem Chat.

---

## Wenn etwas hakt

| Symptom | Typische Ursache |
| --- | --- |
| MCP verbindet nicht | IPC-API aus, oder PCB-Editor nicht offen |
| Version 9 / Netze leer | System-KiCad 9 statt `kicad-10` |
| Write-Tools lehnen ab | Vorlage nicht kopiert (`--allow-ai-write`) |
| Teile in der Blatt-Ecke | Bug in den Pad-Koordinaten — nicht du |
| AppImage, kein Socket | `.AppImage` direkt gestartet statt `kicad-10` |

Mehr: [Handbuch](docs/HANDBUCH.md) → „Probleme lösen“.
