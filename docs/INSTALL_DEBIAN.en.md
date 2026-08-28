# Fresh install on Debian + Cursor

A clean Debian machine (x86-64, e.g. Debian 13) through to the first
question in Cursor. If the package is already installed:
[Getting started](GETTING_STARTED.md).
Reference: [MANUAL.md](MANUAL.md).
German: [INSTALL_DEBIAN.md](INSTALL_DEBIAN.md).

Both `.deb` files sit on the same
[Releases page](https://github.com/Draganito/kicad-mcp/releases).
They do **not** call each other:

| Package | Role |
| --- | --- |
| `kicad-mcp_*.deb` | Cursor drives KiCad (parts, nets, individual tracks) |
| `kicad-routing-tools_*.deb` | Autorouter **inside** Pcbnew (Tools → External Plugins) |

KiCad itself is **not** in either deb — official AppImage from kicad.org.

---

## 1. KiCad 10 AppImage

Debian 13 ships KiCad **9**. Nets and MCP need **10**.

1. Download **Lite** (enough) from
   [kicad.org/download/linux](https://www.kicad.org/download/linux/).
   Some browsers save `kicad-10.0.5-x86_64.AppImage.tar`:

   ```bash
   mkdir -p ~/Programme
   tar -xf kicad-10.0.5-x86_64.AppImage.tar -C ~/Programme
   chmod +x ~/Programme/kicad-10.0.5-x86_64.AppImage
   ```

2. Do **not** double-click the `.AppImage` and do **not** run
   `/usr/bin/kicad` (system 9). The AppImage remaps `TMPDIR` under
   `~/.cache/tmp`; MCP looks for `ipc:///tmp/kicad/api.sock`.

---

## 2. Both debs

Download both files from the Releases page, then:

```bash
sudo apt install ./kicad-mcp_<version>_amd64.deb ./kicad-routing-tools_0.20.4-2_amd64.deb
```

`kicad-mcp` installs:

- `/usr/bin/kicad-mcp` — MCP server
- `/usr/bin/kicad-10` — wrapper (`TMPDIR=/tmp` + AppImage)
- `/usr/share/kicad-mcp/cursor-setup/` — Cursor template
- `/usr/share/applications/kicad-10.desktop` — menu entry

`kicad-routing-tools` stores the plugin under
`/usr/share/kicad-routing-tools/`. Each user must copy it **once**:

```bash
kicad-routing-tools-setup
```

Not as root. That also unpacks numpy/scipy/shapely (CPython 3.11) into
`~/.local/share/kicad/10.0/3rdparty/python`. The AppImage has no pip —
do not run `/usr/bin/python3 -m pip` from inside KiCad
(`No module named encodings`).

---

## 3. Start KiCad and enable IPC

```bash
kicad-10
```

Or the menu entry **KiCad 10 (AppImage)**. Alternative:
`KICAD_10_APPIMAGE=/path/to.AppImage kicad-10`.

1. Open the **PCB editor** (an empty board is enough).
2. **Preferences → Plugins → Enable IPC API**.
3. Quit KiCad completely, run `kicad-10` again, open the PCB editor.

`board_summary` must later show `10.0.x` and `net_ipc_persists: true`.
If it shows 9.x: wrong KiCad — use the wrapper, not the distro package.

---

## 4. Cursor

Install [Cursor](https://cursor.com). Create a **project folder**
(or open an existing KiCad project) and copy the template in:

```bash
cd /path/to/project
cp -a /usr/share/kicad-mcp/cursor-setup/.cursor /usr/share/kicad-mcp/cursor-setup/.cursorignore .
```

Open that folder in Cursor and toggle the `kicad-mcp` MCP server off/on.
`mcp.json` launches `/usr/bin/kicad-mcp --allow-ai-write`.

Without `--allow-ai-write` every write tool refuses.

---

## 5. First check

KiCad 10 is running and the PCB editor is open. In Cursor ask:
*“call board_summary”*.

Expect: version 10.x, `has_open_board: true`, `net_ipc_persists: true`.

Typical flow:

1. MCP: outline, LCSC parts, place, **nets** (ratsnest)
2. Copper: by hand in KiCad, the plugin dialog, or MCP `autoroute_nets`
   with **named** nets (never GND, never unused GPIOs)
3. GND/5V as pours (`set_copper_zone`); autoroute 5V only if asked
4. `check_board` and after copper `check_drc`, then `export_manufacturing` if needed
5. `save_board` **only when you ask**

Undo: **Ctrl+Z** in KiCad. Do not edit `.kicad_pcb` by hand.

---

## 6. What the packages do not do

- kicad-mcp does **not** press Route. `autoroute_nets` runs the plugin
  **CLI** and reloads the file (no Ctrl+Z).
- The plugin does **not** place parts or assign nets.
- Distro `apt install kicad` stays 9 — ignore it.
- The debs do not ship the AppImage (too large, third-party license).

---

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| MCP: connect / No such file | KiCad closed, PCB editor closed, or IPC API off |
| `api.sock` missing | Started the `.AppImage` directly — use `kicad-10` |
| `net_ipc_persists: false` / version 9 | System KiCad instead of AppImage 10 |
| Write refused | `--allow-ai-write` missing from `mcp.json` |
| Plugin: `No module named encodings` | pip from KiCad — run `kicad-routing-tools-setup` instead |
| Plugin missing from the menu | Setup not run as the user, or KiCad not restarted |
| Plugin pip fails | Expected: AppImage has no pip. Setup is enough |

More: [MANUAL.md](MANUAL.md) “Troubleshooting”.
