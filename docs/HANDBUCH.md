# kicad-mcp — Das Handbuch

Nachschlagewerk von A bis Z. Zum Ankommen zuerst den
[Einstieg für Maker](../ANLEITUNG_FUER_ANFAENGER.md).
English: [MANUAL.md](MANUAL.md) · [Getting started](GETTING_STARTED.md).
Neuinstallation: [INSTALL_DEBIAN.md](INSTALL_DEBIAN.md).

---

## Inhaltsverzeichnis

1. [Was ist kicad-mcp](#1-was-ist-kicad-mcp)
2. [Installation](#2-installation)
3. [KiCad vorbereiten](#3-kicad-vorbereiten)
4. [Cursor / MCP](#4-cursor--mcp)
5. [Koordinaten und Umriss](#5-koordinaten-und-umriss)
6. [Bauteile](#6-bauteile)
7. [Netze](#7-netze)
8. [Kupfer](#8-kupfer)
9. [Werkzeugkatalog](#9-werkzeugkatalog)
10. [Speichern und Undo](#10-speichern-und-undo)
11. [Debian-Paket bauen](#11-debian-paket-bauen)
12. [Probleme lösen](#12-probleme-lösen)
13. [Bewusste Grenzen](#13-bewusste-grenzen)

---

## 1. Was ist kicad-mcp

Du lässt KiCad offen. Aus Cursor heraus legt ein Assistent die Platine
in dem Editor, den du schon siehst. KiCad bleibt die Wahrheit; dieses
Programm ist kein zweiter Layout-Editor.

Technisch: ein **stdio-MCP-Server**, der den **PCB-Editor** über die
offizielle IPC-API steuert. `autoroute_nets` kann die Companion-CLI
für genannte Netze starten.

Footprints für JLCPCB kommen von **EasyEDA / LCSC**
(`download_lcsc_part`). Drahtpads und Montagelöcher werden parametrisch
erzeugt: `list_parts` schreibt die Defaults (`WirePad_PTH` 2,5/1,5 mm,
`MountingHole_M3_NPTH` 3,2 mm); `make_wire_pad` / `make_mounting_hole`
erzeugen jede andere Größe (z. B. `WirePad_PTH_3.2_2`,
`MountingHole_4.5_NPTH`).

Lizenz: AGPL-3.0-only. KiCad selbst ist ein separates GPL-3.0-Programm
und liegt nicht in diesem Paket.

## 2. Installation

### Fertiges Paket

Von [GitHub Releases](https://github.com/Draganito/kicad-mcp/releases)
beide Dateien (MCP + optionaler Autorouter):

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb ./kicad-routing-tools_0.20.4-2_amd64.deb
kicad-routing-tools-setup
```

Schritt für Schritt: [INSTALL_DEBIAN.md](INSTALL_DEBIAN.md).
Debian/Ubuntu x86-64, glibc 2.39+. **KiCad 10** muss laufen
(`recommends: kicad` ist nur das Systempaket — auf Debian 13 ist das
9.0.2 und reicht nicht für Netze). Nach der MCP-`.deb` startet
`kicad-10` das AppImage mit `TMPDIR=/tmp`.

### Aus dem Quellcode

```bash
cargo test --workspace
cargo build --release -p kicad-mcp
```

Binary: `target/release/kicad-mcp`.

## 3. KiCad vorbereiten

kicad-mcp zielt auf **KiCad 10**. Dort bleiben Netze nach
`connect_many` / `set_copper_zone` erhalten. System-KiCad 9 auf Debian 13
nicht verwenden.

1. Offizielles AppImage: [kicad.org/download/linux](https://www.kicad.org/download/linux/)
   — **Lite** reicht. Manche Browser speichern
   `kicad-10.0.5-x86_64.AppImage.tar`: `tar -xf … -C ~/Programme`.
2. `chmod +x ~/Programme/kicad-10.0.5-x86_64.AppImage`
3. **Immer** über den Wrapper starten, nicht die `.AppImage` und nicht
   `/usr/bin/kicad`. Das AppImage legt `TMPDIR` sonst unter
   `~/.cache/tmp` — MCP sucht `ipc:///tmp/kicad/api.sock`.

```bash
kicad-10
```

`/usr/bin/kicad-10` (aus der `.deb`, Quelle `scripts/kicad-10.sh`) setzt
`TMPDIR=/tmp` und startet das AppImage aus `~/Programme/` (oder
`KICAD_10_APPIMAGE`). Menüeintrag: **KiCad 10 (AppImage)**.

4. PCB-Editor öffnen (eine Platine muss geladen sein).
5. **Einstellungen → Plugins → IPC-API aktivieren**, KiCad neu starten.
6. `board_summary` muss `10.0.x` und `net_ipc_persists: true` zeigen.
   MCP in Cursor einmal aus/an, wenn vorher 9 lief.

Ohne IPC-API schlagen alle Tools fehl (Socket-Fehler).

`export_manufacturing` braucht `kicad-cli` **aus KiCad 10** (Mount
`/tmp/.mount_kicad*/bin/kicad-cli`), nicht das System-9-Binary.

## 4. Cursor / MCP

Nach der `.deb`-Installation den Inhalt von
`/usr/share/kicad-mcp/cursor-setup/` in den Cursor-Projektordner
kopieren (`.cursor/` und `.cursorignore`).

`mcp.json` startet `/usr/bin/kicad-mcp --allow-ai-write`. Aus einem
Source-Tree den Pfad auf `target/release/kicad-mcp` oder
`target/debug/kicad-mcp` setzen — **nicht** `cargo run`. Nach Rust-
Änderungen: neu bauen, MCP-Server in Cursor aus/an.

Ohne `--allow-ai-write` lehnt jedes Write-Tool ab.

`.cursorignore` blendet `.kicad_pcb` / `.kicad_sch` aus, damit die KI
die Dateien nicht von Hand anfasst.

## 5. Koordinaten und Umriss

- Einheit: **Millimeter**, KiCad-Ursprung, **+x rechts, +y oben**.
- Der rosa A4-Rahmen ist das **Zeichenblatt** (297 × 210 mm), nicht die
  Platine.
- Die Platine ist **Edge.Cuts** (`set_board_outline`).
- Rechteck: `width_mm` / `height_mm`. Origin = unten links. Fehlt der
  Origin, liegt das Rechteck in der Blattmitte, nicht bei 0,0.
- Polygon: `points: [{x_mm, y_mm}, ...]` (max. 400, wird geschlossen).
- Default `replace=true` löscht vorhandenes Edge.Cuts (auch wenn KiCad
  den Layer intern `BL_Edge_Cuts` nennt) und füllt Kupferzonen neu.

`clear_board` löscht Footprints, Tracks, Vias, Zonen — Edge.Cuts bleibt.

## 6. Bauteile

1. `download_lcsc_part` mit C-Nummer (z. B. `C5348912`) schreibt
   EasyEDA-Geometrie nach `jlcpcb_parts.pretty` neben dem offenen Board
   und liefert `pins: [{number, pin_name}]` (EasyEDA-Funktion).
2. `list_parts` nennt die Template-Namen für `place_footprint`.
   `get_part_pins` liest dieselben EasyEDA-Namen für ein schon
   geladenes Template (`{template}.pins.json`).
3. `place_footprint` / `place_parts` (max. 150, ein Undo) /
   `place_matrix` (Raster: Origin = Zelle 0,0, +x Spalten, +y Zeilen,
   Pitch Mitte-zu-Mitte).

Platzierung prüft F.CrtYd-Überlappung untereinander. `move_footprint`
verschiebt und/oder dreht ein gesetztes Teil in einem Undo: starre
Transformation von Anker und jedem gebackenen Pad — Netze, Referenz und
Padstack-Geometrie bleiben erhalten (kein remove+place). Der Zielort
wird Courtyard-geprüft. Kupfer zieht **nicht** mit — Leiterbahnen zum
Teil neu ziehen. LCSC-C-Nummern nicht durch generische
KiCad-Library-Footprints ersetzen.

`get_pads` liefert jeden Pad als harte Daten direkt aus KiCads
gebackenen Protos: Referenz, Pin, Netz, absolute x/y, Größe, Drehung,
smd/pth/npth, Form, Lage, `layers` (jede Kupferlage des Padstacks —
ein 5V-PTH ohne `In1.Cu` bekommt keine Thermals aus der Power-Fläche),
Bohrung; filterbar nach `reference` und/oder `net`. Damit Platzierung
und Orientierung verifizieren (ein gespiegeltes oder falsch gedrehtes
Teil zeigt Pads auf der falschen Seite des Ankers) statt aus Templates
oder Renderings zu raten.

`check_placement` macht daraus ein hartes OK/Fail-Audit: jeder Pad wird
aus seinem `jlcpcb_parts`-Template am Anker + Rotation des Footprints
neu berechnet und gegen die gebackenen Board-Pads verglichen. Ein
gespiegeltes, falsch gedrehtes oder mit einer älteren kicad-mcp-Version
gebackenes Teil fällt durch — mit Delta pro Pad in mm (Pin, erwartete
vs. tatsächliche Position, dazu Größe/Winkel/Typ/Bohrungs-Abweichungen;
ein verlorenes NPTH-Loch oder ein als Rundloch gebackener Slot fällt
ebenfalls durch).
Thermal-Cluster mit gemeinsamer Pin-Nummer werden über die nächste
Position gematcht. Optionaler `reference`-Filter, `tolerance_mm`
Standard 0.01. Footprints ohne Template auf der Platte landen unter
`skipped`, nicht unter failed. Nach Platzieren oder Verschieben laufen
lassen; das Ergebnis zählt mehr als jedes Rendering.

Nested Pads werden nicht mit dem Parent transformiert. `place.rs` backt
Board-Millimeter in jedes Pad. Ohne diesen Bake liegt das Kupfer in der
Blatt-Ecke (0,0), während die API behauptet, das Teil sitze in der Mitte.

## 7. Netze

Pin-Namen und Funktionen kommen von **EasyEDA** (`download_lcsc_part` /
`get_part_pins`), nicht aus dem Datenblatt-Gedächtnis und nicht aus
Alladin-`pad_nets`. `connect_pins` / `connect_many` setzen **Pad.net**
(Ratsnest), kein Kupfer. Alle Pads mit derselben Pin-Nummer bekommen
das Netz (Thermal-Cluster, z. B. ESP32-Pad 41). Daisy-Chain: `net`
weglassen. Das Netz wird in die Parent-FootprintInstance gespliced —
ein freies Pad-UpdateItems lehnt KiCad ab. Auf **KiCad 10** bleibt der
Name nach Speichern erhalten; `get_nets` / `check_board` müssen ihn
zeigen. `disconnect_pin` / `disconnect_many` setzen die Zuweisung
zurück auf unconnected (gleicher Splice, Code 0). Idempotent, wenn der
Pin schon offen ist. Reißt kein Kupfer auf.

Ein Hersteller-PDF ist nur erlaubt, wenn eine **Logikprüfung** die
EasyEDA-Namen unmöglich macht (Beispiel: WROOM-Pad 1 heißt `IO20`,
obwohl Pin 1 der GND-Eckpin des Moduls ist). Dann harte Fakten holen
(`datasheet_url`, wenn EasyEDA eine mitliefert), den Widerspruch
nennen und danach vernetzen. 0603 mit EasyEDA-Namen `1`/`2` haben keine
Polarität — GND vs. Rail vom Nachbarpad.

## 8. Kupfer

- `add_track` / `add_tracks` (max. 150, ein Undo)
- `add_via` / `add_vias` (max. 150)
- `stitch_via` — GND-Via + F.Cu-Stub neben einem Pin oder allen SMD-Pads
  eines Netzes (`net: "GND"`)
- `set_copper_layers` — Kupferlagen 2/4/6/8 (nicht undo-bar)
- `set_copper_zone` — Rechteck oder Polygon; Netz z. B. `5V` / `GND`;
  Lage `F.Cu` / `In1.Cu` / `In2.Cu` / `B.Cu`; Pads solid; `thermal=true` PTH-Speichen; `thermal_smd=true` auch SMD (LED/Elko); `remove_islands=true` tote Kupferinseln weg; danach Refill
- `clear_zones` — alle Kupferzonen löschen (Bahnen bleiben)
- `ripup_wire` — `segment_id` aus `get_routing_scene`
- `autoroute_nets` — genannte Netze über die Plugin-CLI, dann Reload
  und Zonen-Refill
- `check_drc` — `kicad-cli pcb drc` (Clearance, Silk, Löcher); speichert
- `review_board` — liest nur: GND/Power-Pour, benachbarte Lagen, Via
  am Elko-GND (3 mm), PTH-Drahtpads gegen die Flächen (Thermals vs.
  Abstand). Kein DRC, keine 90°-Ecken. Vor „fertig“.

### Silk-Text

`add_text` / `add_texts` (max. 150, ein Undo) setzt Board-Text auf
**F.Silkscreen** (Default) oder **B.Silkscreen**. Für Anschlussnamen
(`5V`, `GND`, `DATA`) neben Drahtpads. Nie F.Cu, nie Footprint-Value
(Export streicht U1/C3 bereits). Größe Default 1,0 mm (min. 0,8).
`clear_board` löscht diese Labels mit.

`autoroute_nets` startet die **CLI** des optionalen Plugins KiCad
Routing Tools (nicht den wx-Dialog). Voraussetzung:
`kicad-routing-tools-setup` und KiCad 10 über `kicad-10`.
`nets` ist Pflicht — nie `*` / alle Netze. GND/VSS werden abgelehnt
(Fläche). USB_DN und USB_DP nur zusammen (zwei Einzelnetze plus
Längenabgleich, kein `route_diff.py`). Optional `track_width_mm`,
`via_size_mm`, `via_drill_mm`, `clearance_mm` — Default pinnt
JLCPCB-sichere Floors (0,2 mm Abstand, Via 0,6/0,3), damit die CLI
nicht still auf 0,127 mm fällt. Der Schritt speichert, schreibt die
Datei, lädt neu und gießt Zonen nach — **kein Ctrl+Z**.
Nur wenn du Autorouting oder eine Platine A–Z inkl. Kupfer willst.
Keine parallelen Copper-Writes: KiCad `BeginCommit` verträgt das nicht.

## 9. Werkzeugkatalog

| Tool | Art |
| --- | --- |
| `board_summary` | Lesen — Version, Zähler |
| `get_footprints` | Lesen — Ref, Lage, Drehung |
| `get_nets` | Lesen — Netze und Pads |
| `get_pads` | Lesen — gebackene Pad-Wahrheit (Position, Netz, Drehung, Lagen) |
| `check_placement` | Lesen — hartes OK/Fail: Template-Pads vs. gebackene Pads |
| `get_routing_scene` | Lesen — Tracks/Vias + IDs |
| `list_parts` | Lesen — Templates inkl. Builtins |
| `get_part_pins` | Lesen — EasyEDA `number` + `pin_name` |
| `check_board` | Lesen — Pads ohne Netz |
| `review_board` | Lesen — kurzer Physik-Report (Pour, Rückweg, Elko-Via, PTH-Thermals) |
| `download_lcsc_part` | Schreiben — EasyEDA → Library + Pins |
| `make_wire_pad` | Schreiben — parametrisches PTH-Drahtpad-Template |
| `make_mounting_hole` | Schreiben — parametrisches NPTH-Loch-Template |
| `place_footprint` | Schreiben |
| `place_parts` | Schreiben — Batch |
| `place_matrix` | Schreiben — Raster |
| `move_footprint` | Schreiben — starres Verschieben/Drehen, Netze bleiben, Kupfer bleibt |
| `remove_footprint` | Schreiben |
| `clear_board` | Schreiben — Teile + Kupfer + Silk-Text, Umriss bleibt |
| `clear_zones` | Schreiben — nur Zonen |
| `set_board_outline` | Schreiben — Edge.Cuts |
| `add_text` / `add_texts` | Schreiben — Silk-Label (F/B.Silkscreen) |
| `connect_pins` / `connect_many` | Schreiben — Ratsnest (alle gleichnamigen Pads) |
| `disconnect_pin` / `disconnect_many` | Schreiben — Pin wieder unconnected |
| `add_track` / `add_tracks` | Schreiben — Leiterbahn |
| `add_via` / `add_vias` | Schreiben — Via |
| `stitch_via` | Schreiben — GND-Via + Stub |
| `set_copper_layers` | Schreiben — Kupferlagen 2/4/6/8 |
| `set_copper_zone` | Schreiben — Pour + Refill |
| `autoroute_nets` | Schreiben — CLI-Autorouter, genannte Netze |
| `ripup_wire` | Schreiben |
| `check_drc` | Schreiben — `kicad-cli` DRC (speichert) |
| `render_board` | Schreiben — 3D-PNG über `kicad-cli pcb render` (speichert) |
| `save_board` | Schreiben — nur auf Wunsch |
| `export_manufacturing` | Schreiben — JLCPCB: `*_gerbers.zip` + `*_bom.csv` + `*_cpl.csv` (`kicad-cli`) |

`render_board` füllt Zonen, speichert und raytraced die Platine nach
`<stem>_render_<side>.png` über `kicad-cli pcb render` (side
top/bottom/left/right/front/back; optional `zoom`, `rotate [x,y,z]`,
`perspective`, `floor`, `width`/`height`). Kupfer, Silk, Maske und
Löcher sind echt; EasyEDA-Footprints haben keine 3D-Körper — die
Orientierung der Gehäuse prüft man mit `get_pads`, nicht am Rendering.

## 10. Speichern und Undo

Jeder Write liegt auf KiCads Undo-Stack (**Ctrl+Z**), außer
`autoroute_nets` (Datei-Reload). `save_board` nur wenn der Mensch es
verlangt; `autoroute_nets` und `check_drc` speichern intern.

`export_manufacturing` speichert die offene Platine, füllt Zonen neu
und schreibt drei JLCPCB-Dateien ins Projektverzeichnis oder
nach `out_dir`:

| Datei | JLCPCB-Slot |
| --- | --- |
| `<name>_gerbers.zip` | PCB / Gerber (Kupfer, Maske, Silk, Paste, Edge.Cuts, Bohrung). **Kein** Bestückungsdruck der Bauteilnummern (U1, C3, …) und kein Value-Text — die liegen auf dichtem Raster zu nah an Pads/Bohrungen und scheitern bei JLCPCB am Silk-to-pad / Silk-to-hole-DFM. Referenzen stehen in BOM und CPL. |
| `<name>_bom.csv` | BOM (Comment, Designator, Footprint, LCSC Part #) |
| `<name>_cpl.csv` | CPL / Centroid (Designator, Mid X, Mid Y, Layer, Rotation) |

Generierte Drahtpads und Montagelöcher (jede Größe) stehen nicht in
BOM/CPL. Braucht `kicad-cli` (Paket `kicad`).

## 11. Debian-Paket bauen

```bash
cargo install cargo-deb --locked
dist/make_beta_package.sh
```

Erzeugt `dist/kicad-mcp_<version>_amd64.deb` (Binary + `kicad-10` +
Doku + Cursor-Setup) und `dist/kicad-routing-tools_*.deb`. Die `.deb`s
nicht ins Git legen — als GitHub-Release-Assets hochladen.
Nur MCP: `SKIP_ROUTING_TOOLS=1 dist/make_beta_package.sh`.

## 12. Probleme lösen

| Symptom | Ursache / Fix |
| --- | --- |
| connect / No such file | KiCad zu, oder IPC API aus |
| `api.sock` fehlt (AppImage 10) | Direkt die `.AppImage` gestartet — `kicad-10` nutzen (`TMPDIR=/tmp`) |
| Write refused | `--allow-ai-write` fehlt |
| Kupfer in der Blatt-Ecke | Pad-Bake fehlt — Regression in `place.rs` |
| `replaced_segments: 0` | Altes MCP-Binary; nach Rust-Fix MCP neu laden |
| Loch in der Kupferzone | Inneres Edge.Cuts war ein Ausschnitt; Zonen neu füllen (B in KiCad) oder `set_board_outline` erneut |
| `net_count: 1`, viele unconnected / `net_ipc_persists: false` | Falsches KiCad (9) — `kicad-10` |
| Commit already in progress | Copper-Batches nicht parallel senden |
| `check_drc` findet kein kicad-cli | KiCad 10 über `kicad-10` starten (AppImage-CLI unter `/tmp/.mount_kicad*`) |

## 13. Bewusste Grenzen

- Kein Schema-Editor. Autorouter nur über `autoroute_nets`
  (genannte Netze, Companion-Deb).
- Kein Edit von `.kicad_pcb` über diesen Server.
- Maximal 150 Teile / Tracks / Vias pro Undo-Batch.
- Polygon-Umriss max. 400 Punkte.
- Zielplattform: **KiCad 10**. System-9 nur Geometrie, keine Netze.
