# Neuinstallation auf Debian + Cursor

Schritt für Schritt von einer leeren Debian-Maschine (x86-64, z. B.
Debian 13) bis zur ersten Frage in Cursor. Wenn du das Paket schon
hast: [Einstieg für Maker](../ANLEITUNG_FUER_ANFAENGER.md).
Nachschlagewerk: [HANDBUCH.md](HANDBUCH.md).
English: [INSTALL_DEBIAN.en.md](INSTALL_DEBIAN.en.md).

Die zwei `.deb` liegen auf derselben
[Releases-Seite](https://github.com/Draganito/kicad-mcp/releases).
Sie rufen sich **nicht** gegenseitig auf:

| Paket | Macht |
| --- | --- |
| `kicad-mcp_*.deb` | Cursor steuert KiCad (Teile, Netze, einzelne Bahnen) |
| `kicad-routing-tools_*.deb` | Autorouter **in** Pcbnew (Werkzeuge → Externe Plugins) |

KiCad selbst kommt **nicht** im Deb — offizielles AppImage von kicad.org.

---

## 1. KiCad 10 AppImage

Debian 13 liefert nur KiCad **9**. Netze und MCP brauchen **10**.

1. Download **Lite** (reicht) von
   [kicad.org/download/linux](https://www.kicad.org/download/linux/).
   Manche Browser speichern `kicad-10.0.5-x86_64.AppImage.tar`:

   ```bash
   mkdir -p ~/Programme
   tar -xf kicad-10.0.5-x86_64.AppImage.tar -C ~/Programme
   chmod +x ~/Programme/kicad-10.0.5-x86_64.AppImage
   ```

2. **Nicht** die `.AppImage` doppelklicken und **nicht** `/usr/bin/kicad`
   (System-9). Das AppImage legt `TMPDIR` sonst unter `~/.cache/tmp` —
   MCP sucht `ipc:///tmp/kicad/api.sock`.

---

## 2. Beide Debs

Von der Releases-Seite beide Dateien laden, dann:

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb ./kicad-routing-tools_0.20.4-2_amd64.deb
```

`kicad-mcp` legt u. a. an:

- `/usr/bin/kicad-mcp` — MCP-Server
- `/usr/bin/kicad-10` — Wrapper (`TMPDIR=/tmp` + AppImage)
- `/usr/share/kicad-mcp/cursor-setup/` — Cursor-Vorlage
- `/usr/share/applications/kicad-10.desktop` — Menüeintrag

`kicad-routing-tools` legt die Plugin-Dateien unter
`/usr/share/kicad-routing-tools/` ab. Die muss jeder User **einmal**
in sein KiCad-Verzeichnis kopieren:

```bash
kicad-routing-tools-setup
```

Nicht als root. Das entpackt auch numpy/scipy/shapely (CPython 3.11)
nach `~/.local/share/kicad/10.0/3rdparty/python`. Im AppImage gibt es
kein pip — `/usr/bin/python3 -m pip` aus KiCad heraus nicht starten
(Fehler: `No module named encodings`).

---

## 3. KiCad starten und IPC

```bash
kicad-10
```

Oder den Menüeintrag **KiCad 10 (AppImage)**. Alternativ
`KICAD_10_APPIMAGE=/pfad/zur.AppImage kicad-10`.

1. **PCB-Editor** öffnen (eine leere Platine reicht).
2. **Einstellungen → Plugins → IPC-API aktivieren**.
3. KiCad **komplett beenden**, wieder `kicad-10`, PCB-Editor öffnen.

`board_summary` muss später `10.0.x` und `net_ipc_persists: true` zeigen.
Steht dort 9.x: falsches KiCad — Wrapper nutzen, nicht das Systempaket.

---

## 4. Cursor

[Cursor](https://cursor.com) installieren. Einen **Projektordner**
anlegen (oder ein bestehendes KiCad-Projekt öffnen) und die Vorlage
hinein kopieren:

```bash
cd /pfad/zum/projekt
cp -a /usr/share/kicad-mcp/cursor-setup/.cursor /usr/share/kicad-mcp/cursor-setup/.cursorignore .
```

In Cursor den Ordner öffnen, MCP-Server `kicad-mcp` einmal aus- und
einschalten. `mcp.json` startet `/usr/bin/kicad-mcp --allow-ai-write`.

Ohne `--allow-ai-write` lehnt jedes Write-Tool ab.

---

## 5. Erster Check

KiCad 10 läuft, PCB-Editor ist offen. In Cursor die KI bitten:
*„board_summary aufrufen“*.

Erwartet: Version 10.x, `has_open_board: true`, `net_ipc_persists: true`.

Typischer Ablauf:

1. MCP: Umriss, LCSC-Teile, platzieren, **Netze** (Ratsnest)
2. Kupfer: selbst in KiCad, Plugin-Dialog, oder MCP `autoroute_nets`
   mit **genannten** Netzen (nie GND, nie unbenutzte GPIOs)
3. GND/5V als Fläche (`set_copper_zone`), 5V nur auf Wunsch autorouten
4. `check_board` und nach Kupfer `check_drc`, bei Bedarf `export_manufacturing`
5. `save_board` **nur wenn du es willst**

Rückgängig: **Ctrl+Z** in KiCad. `.kicad_pcb` nicht von Hand editieren.

---

## 6. Was die Pakete nicht tun

- kicad-mcp **drückt nicht** den Route-Knopf. `autoroute_nets` startet
  die Plugin-**CLI** und lädt die Datei neu (kein Ctrl+Z).
- Das Plugin **legt keine** Teile und setzt keine Netze.
- System-`apt install kicad` bleibt 9 — ignorieren.
- Die `.deb`s enthalten kein AppImage (zu groß, fremde Lizenz).

---

## Wenn etwas hakt

| Symptom | Ursache |
| --- | --- |
| MCP: connect / No such file | KiCad zu, PCB-Editor zu, oder IPC-API aus |
| `api.sock` fehlt | `.AppImage` direkt gestartet — `kicad-10` nutzen |
| `net_ipc_persists: false` / Version 9 | System-KiCad statt AppImage 10 |
| Write refused | `--allow-ai-write` fehlt in `mcp.json` |
| Plugin: `No module named encodings` | pip aus KiCad — stattdessen `kicad-routing-tools-setup` |
| Plugin fehlt im Menü | Setup nicht als User ausgeführt, oder KiCad nicht neu gestartet |
| Plugin-pip schlägt fehl | Absicht: AppImage hat kein pip. Setup reicht |

Mehr: [HANDBUCH.md](HANDBUCH.md) Kapitel „Probleme lösen“.
