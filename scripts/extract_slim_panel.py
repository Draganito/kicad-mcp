#!/usr/bin/env python3
"""Translate darkroom_led_panel_4x5_slim.json into KiCad millimetres (A4 centre).

Alladin is +x right, +y down. KiCad is +x right, +y up — this flips Y and
negates rotation so EasyEDA pad 1 (GND) lands on the GND via.
"""
from __future__ import annotations

import json
from collections import Counter
from pathlib import Path

SRC = Path("/home/dragan/Dokumente/Projekte/PlatformIO/alladin/examples/darkroom_led_panel_4x5_slim.json")
OUT = Path("/home/dragan/Dokumente/Projekte/PlatformIO/kicad-mcp/kicad_projekte/test1/slim_panel_kicad.json")

CX = 148.5  # A4 centre X mm
CY = 105.0  # A4 centre Y mm

TEMPLATES = {
    "SKC6812RGBW-NW": "C5348912_LED-SMD_4P-L5.0-W4.9-BR",
    "CC0603KRX7R9BB104": "C14663_C0603",
    "Mounting hole (M3, NPTH)": "MountingHole_M3_NPTH",
    "Wire pad (solder, 2mm)": "WirePad_PTH",
    "SD05_C502527": "C502527_SOD-323_L1.8-W1.3-LS2.5-FD",
}

LAYER = {"FCu": "F.Cu", "BCu": "B.Cu", "F.Cu": "F.Cu", "B.Cu": "B.Cu"}


def mm(nm: int | float | None) -> float:
    return 0.0 if nm is None else nm / 1e6


def xy(pos: dict) -> tuple[float, float]:
    # Alladin / EasyEDA: +x right, +y down. KiCad: +x right, +y up.
    return round(mm(pos["x"]) + CX, 4), round(CY - mm(pos["y"]), 4)


def net_name(nid: int, nets: dict[int, str]) -> str:
    name = nets.get(nid, f"Net{nid}")
    if name in ("GND", "5V"):
        return name
    if name.startswith("Net"):
        return name
    return name


def main() -> None:
    d = json.loads(SRC.read_text())
    nets = {n["id"]: n["name"] for n in d.get("nets", [])}

    outline = [list(xy(p)) for p in d["outline"][0]["points"]]

    parts = []
    unknown = Counter()
    for f in d["footprints"]:
        tmpl = TEMPLATES.get(f["template_name"])
        if tmpl is None:
            unknown[f["template_name"]] += 1
            continue
        x, y = xy(f["position"])
        pad_nets = [net_name(n, nets) for n in (f.get("pad_nets") or [])]
        parts.append(
            {
                "template": tmpl,
                "reference": f["reference"],
                "x_mm": x,
                "y_mm": y,
                "rotation_deg": -(f.get("rotation_deg") or 0.0),
                "pad_nets": pad_nets,
            }
        )

    tracks = []
    for t in d.get("tracks") or []:
        ax, ay = xy(t["from"])
        bx, by = xy(t["to"])
        tracks.append(
            {
                "a_x_mm": ax,
                "a_y_mm": ay,
                "b_x_mm": bx,
                "b_y_mm": by,
                "net": net_name(t["net"], nets),
                "layer": LAYER.get(t.get("layer") or "FCu", "F.Cu"),
                "width_mm": round(mm(t.get("width") or 250000), 3),
            }
        )

    vias = []
    for v in d.get("vias") or []:
        x, y = xy(v["center"])
        vias.append(
            {
                "x_mm": x,
                "y_mm": y,
                "net": net_name(v["net"], nets),
                "drill_mm": round(mm(v.get("drill") or 300000), 3),
                "size_mm": round(mm(v.get("diameter") or 600000), 3),
            }
        )

    zones = []
    for z in d.get("zones") or []:
        keys = sorted(z.keys())
        pts_src = (z.get("outline") or {}).get("points") or []
        # Pre-filled pours can have thousands of thermal spokes — use the board outline instead.
        use_pts = outline if len(pts_src) > 400 else [list(xy(p)) for p in pts_src]
        zones.append(
            {
                "keys": keys,
                "net": net_name(z.get("net") or 0, nets) if z.get("net") else None,
                "layer": LAYER.get(z.get("layer") or "", None),
                "point_count_src": len(pts_src),
                "points": use_pts,
                "raw": {k: z[k] for k in z if k != "outline"},
            }
        )

    xs = [p[0] for p in outline]
    ys = [p[1] for p in outline]
    out = {
        "cx": CX,
        "cy": CY,
        "outline_bbox": {
            "min_x": round(min(xs), 3),
            "max_x": round(max(xs), 3),
            "min_y": round(min(ys), 3),
            "max_y": round(max(ys), 3),
            "w": round(max(xs) - min(xs), 3),
            "h": round(max(ys) - min(ys), 3),
        },
        "unknown_templates": dict(unknown),
        "template_counts": dict(Counter(p["template"] for p in parts)),
        "outline": outline,
        "parts": parts,
        "tracks": tracks,
        "vias": vias,
        "zones": zones,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(out))
    print(
        f"parts={len(parts)} tracks={len(tracks)} vias={len(vias)} "
        f"outline={len(outline)} zones={len(zones)} bbox={out['outline_bbox']}"
    )
    print("templates", out["template_counts"])
    print("unknown", unknown)
    for i, z in enumerate(zones):
        print(f"zone{i} keys={z['keys']} net={z['net']} layer={z['layer']} src_pts={z['point_count_src']}")
        print("  raw", z["raw"])


if __name__ == "__main__":
    main()
