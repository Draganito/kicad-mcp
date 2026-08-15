# kicad-mcp — Das Handbuch

Vollständige Anleitung von A bis Z, Stand v0.1.0.
English: [MANUAL.md](MANUAL.md).
Einstieg mit Checkliste: [ANLEITUNG_FUER_ANFAENGER.md](../ANLEITUNG_FUER_ANFAENGER.md).

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

kicad-mcp ist ein **stdio-MCP-Server** für Cursor. Er steuert einen
**laufenden KiCad-PCB-Editor** über die offizielle IPC-API. KiCad bleibt
der Editor; dieses Programm ist kein zweiter Layout-Editor und kein
Autorouter.

Footprints für JLCPCB kommen von **EasyEDA / LCSC**
(`download_lcsc_part`). Builtin-Teile: `WirePad_PTH`,
`MountingHole_M3_NPTH`.

Lizenz: AGPL-3.0-only. KiCad selbst ist ein separates GPL-3.0-Programm
und liegt nicht in diesem Paket.

## 2. Installation

### Fertiges Paket

Von [GitHub Releases](https://github.com/Draganito/kicad-mcp/releases):

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb
```

Debian/Ubuntu x86-64, glibc 2.39+. **KiCad 10** muss laufen
(`recommends: kicad` ist nur das Systempaket — auf Debian 13 ist das
9.0.2 und reicht nicht für Netze).

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
install -m 755 scripts/kicad-10.sh ~/Programme/kicad-10.sh
~/Programme/kicad-10.sh
```

`scripts/kicad-10.sh` setzt `TMPDIR=/tmp` und startet das AppImage aus
`~/Programme/` (oder `KICAD_10_APPIMAGE`). Menüeintrag:
`Exec=$HOME/Programme/kicad-10.sh`.

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
   EasyEDA-Geometrie nach `jlcpcb_parts.pretty` neben dem offenen Board.
2. `list_parts` nennt die Template-Namen für `place_footprint`.
3. `place_footprint` / `place_parts` (max. 150, ein Undo) /
   `place_matrix` (Raster: Origin = Zelle 0,0, +x Spalten, +y Zeilen,
   Pitch Mitte-zu-Mitte).

Platzierung prüft F.CrtYd-Überlappung untereinander. Kein
`move_footprint`: entfernen und neu setzen. LCSC-C-Nummern nicht durch
generische KiCad-Library-Footprints ersetzen.

Nested Pads werden nicht mit dem Parent transformiert. `place.rs` backt
Board-Millimeter in jedes Pad. Ohne diesen Bake liegt das Kupfer in der
Blatt-Ecke (0,0), während die API behauptet, das Teil sitze in der Mitte.

## 7. Netze

`connect_pins` / `connect_many` setzen **Pad.net** (Ratsnest), kein
Kupfer. Daisy-Chain: `net` weglassen. Das Netz wird in die
Parent-FootprintInstance gespliced — ein freies Pad-UpdateItems lehnt
KiCad ab. Auf **KiCad 10** bleibt der Name nach Speichern erhalten;
`get_nets` / `check_board` müssen ihn zeigen.

## 8. Kupfer

- `add_track` / `add_tracks` (max. 150, ein Undo)
- `add_via` / `add_vias` (max. 150)
- `set_copper_zone` — Rechteck oder Polygon; Netz z. B. `5V` / `GND`;
  Lage `F.Cu` oder `B.Cu`; danach Refill
- `ripup_wire` — `segment_id` aus `get_routing_scene`

Kein Autorouter. Keine parallelen Copper-Writes: KiCad
`BeginCommit` verträgt das nicht.

## 9. Werkzeugkatalog

| Tool | Art |
| --- | --- |
| `board_summary` | Lesen — Version, Zähler |
| `get_footprints` | Lesen — Ref, Lage, Drehung |
| `get_nets` | Lesen — Netze und Pads |
| `get_routing_scene` | Lesen — Tracks/Vias + IDs |
| `list_parts` | Lesen — Templates inkl. Builtins |
| `check_board` | Lesen — Pads ohne Netz |
| `download_lcsc_part` | Schreiben — EasyEDA → Library |
| `place_footprint` | Schreiben |
| `place_parts` | Schreiben — Batch |
| `place_matrix` | Schreiben — Raster |
| `remove_footprint` | Schreiben |
| `clear_board` | Schreiben — Teile + Kupfer, Umriss bleibt |
| `set_board_outline` | Schreiben — Edge.Cuts |
| `connect_pins` / `connect_many` | Schreiben — Ratsnest |
| `add_track` / `add_tracks` | Schreiben — Leiterbahn |
| `add_via` / `add_vias` | Schreiben — Via |
| `set_copper_zone` | Schreiben — Pour + Refill |
| `ripup_wire` | Schreiben |
| `save_board` | Schreiben — nur auf Wunsch |
| `export_manufacturing` | Schreiben — JLCPCB: `*_gerbers.zip` + `*_bom.csv` + `*_cpl.csv` (`kicad-cli`) |

## 10. Speichern und Undo

Jeder Write liegt auf KiCads Undo-Stack (**Ctrl+Z**). `save_board` nur
wenn der Mensch es verlangt.

`export_manufacturing` speichert die offene Platine, füllt Zonen neu
und schreibt drei Dateien (wie Alladin) ins Projektverzeichnis oder
nach `out_dir`:

| Datei | JLCPCB-Slot |
| --- | --- |
| `<name>_gerbers.zip` | PCB / Gerber (Kupfer, Maske, Silk, Paste, Edge.Cuts, Bohrung). **Kein** Bestückungsdruck der Bauteilnummern (U1, C3, …) und kein Value-Text — die liegen auf dichtem Raster zu nah an Pads/Bohrungen und scheitern bei JLCPCB am Silk-to-pad / Silk-to-hole-DFM. Referenzen stehen in BOM und CPL. |
| `<name>_bom.csv` | BOM (Comment, Designator, Footprint, LCSC Part #) |
| `<name>_cpl.csv` | CPL / Centroid (Designator, Mid X, Mid Y, Layer, Rotation) |

Builtin-Drahtpads und M3-Löcher stehen nicht in BOM/CPL. Braucht
`kicad-cli` (Paket `kicad`).

## 11. Debian-Paket bauen

```bash
cargo install cargo-deb --locked
dist/make_beta_package.sh
```

Erzeugt `dist/kicad-mcp_<version>_amd64.deb` (Binary + Doku +
Cursor-Setup). Das `.deb` nicht ins Git legen — als GitHub-Release-
Asset hochladen.

## 12. Probleme lösen

| Symptom | Ursache / Fix |
| --- | --- |
| connect / No such file | KiCad zu, oder IPC API aus |
| `api.sock` fehlt (AppImage 10) | Direkt die `.AppImage` gestartet — `~/Programme/kicad-10.sh` nutzen (`TMPDIR=/tmp`) |
| Write refused | `--allow-ai-write` fehlt |
| Kupfer in der Blatt-Ecke | Pad-Bake fehlt — Regression in `place.rs` |
| `replaced_segments: 0` | Altes MCP-Binary; nach Rust-Fix MCP neu laden |
| Loch in der Kupferzone | Inneres Edge.Cuts war ein Ausschnitt; Zonen neu füllen (B in KiCad) oder `set_board_outline` erneut |
| `net_count: 1`, viele unconnected / `net_ipc_persists: false` | Falsches KiCad (9) — `~/Programme/kicad-10.sh` |
| Commit already in progress | Copper-Batches nicht parallel senden |

## 13. Bewusste Grenzen

- Kein Autorouter, kein Schema-Editor, kein `move_footprint`.
- Kein Edit von `.kicad_pcb` über diesen Server.
- Maximal 150 Teile / Tracks / Vias pro Undo-Batch.
- Polygon-Umriss max. 400 Punkte.
- Zielplattform: **KiCad 10**. System-9 nur Geometrie, keine Netze.
