#!/usr/bin/env python3
"""Rebuild the 109-LED slim panel.

EasyEDA C5348912: 1=GND (via), 2=DIN, 3=5V (pour), 4=DOUT.
Alladin board is +y down — slim_panel_kicad.json already Y-flips into KiCad.
"""
from __future__ import annotations

import json
import math
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path("/home/dragan/Dokumente/Projekte/PlatformIO/kicad-mcp")
BIN = ROOT / "target/debug/kicad-mcp"
JSON = ROOT / "kicad_projekte/test1/slim_panel_kicad.json"
PRETTY = ROOT / "kicad_projekte/led_panel_4x5_test2/jlcpcb_parts.pretty"


class Mcp:
    def __init__(self) -> None:
        self.proc = subprocess.Popen(
            [str(BIN), "--allow-ai-write"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=0,
        )
        self._id = 0
        self.call(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "rebuild-slim", "version": "0.2"},
            },
        )
        self.notify("notifications/initialized", {})

    def _send(self, obj: dict) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(obj, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()

    def _read(self) -> dict:
        assert self.proc.stdout is not None
        while True:
            line = self.proc.stdout.readline()
            if line == "":
                raise RuntimeError(self.proc.stderr.read() if self.proc.stderr else "EOF")
            line = line.strip()
            if line:
                return json.loads(line)

    def notify(self, method: str, params: dict) -> None:
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def call(self, method: str, params: dict | None = None) -> dict:
        self._id += 1
        msg: dict = {"jsonrpc": "2.0", "id": self._id, "method": method}
        if params is not None:
            msg["params"] = params
        self._send(msg)
        while True:
            resp = self._read()
            if resp.get("id") == self._id:
                if "error" in resp:
                    raise RuntimeError(json.dumps(resp["error"], ensure_ascii=False))
                return resp["result"]

    def tool(self, name: str, arguments: dict | None = None) -> object:
        result = self.call("tools/call", {"name": name, "arguments": arguments or {}})
        if result.get("isError"):
            raise RuntimeError(json.dumps(result, ensure_ascii=False)[:2000])
        texts = [b.get("text") or "" for b in (result.get("content") or []) if b.get("type") == "text"]
        if not texts:
            return result
        try:
            return json.loads(texts[0])
        except json.JSONDecodeError:
            return texts[0]

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()


def pads_from_mod(text: str) -> dict[str, tuple[float, float]]:
    pads: dict[str, tuple[float, float]] = {}
    for m in re.finditer(r'\(pad "(\d+)" [^()]*\(at ([-\d.]+) ([-\d.]+)', text):
        pads[m.group(1)] = (float(m.group(2)), float(m.group(3)))
    return pads


def rot(x: float, y: float, deg: float) -> tuple[float, float]:
    r = math.radians(deg)
    c, s = math.cos(r), math.sin(r)
    return c * x - s * y, s * x + c * y


def chunks(xs, n):
    for i in range(0, len(xs), n):
        yield xs[i : i + n]


# EasyEDA C5348912 pin names (not the generic SK6812 datasheet):
# 1=GND (via stub), 2=DIN, 3=VDD/5V (front pour), 4=DOUT.
LED_POWER = {"1": "GND", "3": "5V"}
LED_DATA = {"2", "4"}


def assign_pads(src: dict) -> dict[tuple[str, str], str]:
    """Map (ref, pin) → net. Cap GND = pad closer to companion LED pin 1.

    JSON via coordinates are not used — stitch_via places those later.
    """
    locals_: dict[str, dict[str, tuple[float, float]]] = {}
    for p in PRETTY.glob("*.kicad_mod"):
        locals_[p.stem] = pads_from_mod(p.read_text())

    pad_pos: list[tuple[str, str, float, float, str]] = []
    by_ref: dict[str, list[str]] = defaultdict(list)
    world: dict[tuple[str, str], tuple[float, float]] = {}
    for part in src["parts"]:
        loc = locals_.get(part["template"]) or {}
        if not loc and "WirePad" in part["template"]:
            loc = {"1": (0.0, 0.0)}
        if not loc and "MountingHole" in part["template"]:
            loc = {"1": (0.0, 0.0)}
        for num, (lx, ly) in loc.items():
            wx, wy = rot(lx, ly, part["rotation_deg"])
            xy = (part["x_mm"] + wx, part["y_mm"] + wy)
            pad_pos.append((part["reference"], num, xy[0], xy[1], part["template"]))
            by_ref[part["reference"]].append(num)
            world[(part["reference"], num)] = xy

    def nearest(
        x: float, y: float, maxd: float, allow: set[tuple[str, str]] | None = None
    ) -> tuple[str, str, float] | None:
        best: tuple[float, str, str] | None = None
        for ref, pin, px, py, _tmpl in pad_pos:
            if allow is not None and (ref, pin) not in allow:
                continue
            dist = math.hypot(px - x, py - y)
            if dist <= maxd and (best is None or dist < best[0]):
                best = (dist, ref, pin)
        if best is None:
            return None
        return best[1], best[2], best[0]

    nets: dict[tuple[str, str], str] = {}

    def set_net(ref: str, pin: str, net: str) -> None:
        key = (ref, pin)
        prev = nets.get(key)
        if prev and prev != net:
            return
        nets[key] = net

    leds = [p for p in src["parts"] if "C5348912" in p["template"]]
    for part in leds:
        for pin, net in LED_POWER.items():
            nets[(part["reference"], pin)] = net

    # Cap pad nearer the companion LED's GND (pin 1) is GND; the other is 5V.
    for cap in (p for p in src["parts"] if "C14663" in p["template"]):
        led = min(
            leds,
            key=lambda u: math.hypot(u["x_mm"] - cap["x_mm"], u["y_mm"] - cap["y_mm"]),
        )
        gnd = world[(led["reference"], "1")]
        pins = by_ref[cap["reference"]]
        closer = min(
            pins,
            key=lambda n: math.hypot(
                world[(cap["reference"], n)][0] - gnd[0],
                world[(cap["reference"], n)][1] - gnd[1],
            ),
        )
        for pin in pins:
            nets[(cap["reference"], pin)] = "GND" if pin == closer else "5V"

    for part in src["parts"]:
        if "C502527" in part["template"]:
            nets.setdefault((part["reference"], "1"), "GND")
            nets.setdefault((part["reference"], "2"), "Net1")

    nets[("W226", "1")] = "5V"
    nets[("W227", "1")] = "GND"
    nets[("W228", "1")] = "Net1"

    data_ok: set[tuple[str, str]] = set()
    for part in src["parts"]:
        ref, tmpl = part["reference"], part["template"]
        if "C5348912" in tmpl:
            for pin in LED_DATA:
                data_ok.add((ref, pin))
        elif "WirePad" in tmpl or "C502527" in tmpl:
            for pin in by_ref.get(ref, []):
                if nets.get((ref, pin)) != "GND":
                    data_ok.add((ref, pin))
    for t in src["tracks"]:
        if t["net"] in ("GND", "5V"):
            continue
        for x, y in ((t["a_x_mm"], t["a_y_mm"]), (t["b_x_mm"], t["b_y_mm"])):
            hit = nearest(x, y, 0.9, data_ok)
            if hit and nets.get((hit[0], hit[1])) not in ("GND", "5V"):
                set_net(hit[0], hit[1], t["net"])

    return nets


def pairs_from_nets(nets: dict[tuple[str, str], str]) -> list[dict]:
    groups: dict[str, list[tuple[str, str]]] = defaultdict(list)
    for (ref, pin), net in nets.items():
        if net and net != "NetNone":
            groups[net].append((ref, pin))
    pairs = []
    for net, pads in groups.items():
        if len(pads) < 2:
            continue
        a_ref, a_pin = pads[0]
        for b_ref, b_pin in pads[1:]:
            pairs.append(
                {"ref1": a_ref, "pin1": a_pin, "ref2": b_ref, "pin2": b_pin, "net": net}
            )
    return pairs


def main() -> None:
    src = json.loads(JSON.read_text())
    nets = assign_pads(src)
    print("assigned", len(nets), Counter(nets.values()).most_common(8), flush=True)
    led_gnd = sum(1 for (r, p), n in nets.items() if r.startswith("U") and n == "GND")
    led_5v = sum(1 for (r, p), n in nets.items() if r.startswith("U") and n == "5V")
    gnd_pins = Counter(p for (r, p), n in nets.items() if r.startswith("U") and n == "GND")
    v5_pins = Counter(p for (r, p), n in nets.items() if r.startswith("U") and n == "5V")
    print(f"LED pads GND={led_gnd} 5V={led_5v} (expect 109 each)", flush=True)
    print(f"LED GND pins {dict(gnd_pins)}  5V pins {dict(v5_pins)}", flush=True)
    if gnd_pins.keys() != {"1"} or v5_pins.keys() != {"3"}:
        raise SystemExit("LED power pins must be 1=GND and 3=5V (EasyEDA C5348912)")

    pts = [{"x_mm": x, "y_mm": y} for x, y in src["outline"]]
    place = [
        {
            "template": p["template"],
            "x_mm": p["x_mm"],
            "y_mm": p["y_mm"],
            "rotation_deg": p["rotation_deg"],
            "reference": p["reference"],
        }
        for p in src["parts"]
    ]
    pairs = pairs_from_nets(nets)
    # Data hops only. GND stubs come from stitch_via, not JSON vias/tracks.
    tracks = [
        {
            "a_x_mm": t["a_x_mm"],
            "a_y_mm": t["a_y_mm"],
            "b_x_mm": t["b_x_mm"],
            "b_y_mm": t["b_y_mm"],
            "net": t["net"],
            "layer": t["layer"],
            "width_mm": t["width_mm"],
        }
        for t in src["tracks"]
        if t["net"] not in ("GND", "5V")
    ]

    mcp = Mcp()
    try:
        print("clear_board", mcp.tool("clear_board"), flush=True)
        print("outline", mcp.tool("set_board_outline", {"points": pts}), flush=True)
        for i, batch in enumerate(chunks(place, 150), 1):
            print(f"place {i}", mcp.tool("place_parts", {"parts": batch}), flush=True)
        for i, batch in enumerate(chunks(pairs, 150), 1):
            print(f"connect {i}", mcp.tool("connect_many", {"pairs": batch}), flush=True)
        for i, batch in enumerate(chunks(tracks, 150), 1):
            print(f"tracks {i}", mcp.tool("add_tracks", {"tracks": batch}), flush=True)

        scene = mcp.tool("get_routing_scene")
        print("before stitch vias", len(scene.get("vias") or []), flush=True)
        stitch = mcp.tool("stitch_via", {"net": "GND"})
        print(
            "stitch_via",
            {
                "ok": stitch.get("ok"),
                "placed": stitch.get("placed_count"),
                "skipped": len(stitch.get("skipped") or []),
                "failed": len(stitch.get("failed") or []),
            },
            flush=True,
        )
        for item in (stitch.get("failed") or [])[:20]:
            print("  fail", item, flush=True)
        for item in (stitch.get("skipped") or [])[:10]:
            print("  skip", item, flush=True)

        scene = mcp.tool("get_routing_scene")
        via_nets = Counter(v.get("net") for v in scene["vias"])
        track_nets = Counter(t.get("net") for t in scene["tracks"])
        print("via nets", dict(via_nets), flush=True)
        print("track GND/5V", track_nets.get("GND"), "GND", track_nets.get("5V"), "5V", flush=True)
        if via_nets.get("GND", 0) < 150:
            raise SystemExit(f"stitch_via did not place enough GND vias: {dict(via_nets)}")

        print("zone 5V", mcp.tool("set_copper_zone", {"net": "5V", "layer": "F.Cu", "name": "5V", "points": pts}), flush=True)
        print("zone GND", mcp.tool("set_copper_zone", {"net": "GND", "layer": "B.Cu", "name": "GND", "points": pts}), flush=True)

        scene = mcp.tool("get_routing_scene")
        via_nets = Counter(v.get("net") for v in scene["vias"])
        track_nets = Counter(t.get("net") for t in scene["tracks"])
        print("after pour via", dict(via_nets), flush=True)
        print("after pour tracks GND/5V", track_nets.get("GND"), "GND", track_nets.get("5V"), "5V", flush=True)
        print("summary", mcp.tool("board_summary"), flush=True)
        print("check", mcp.tool("check_board"), flush=True)
        if via_nets.get("GND", 0) < 200:
            raise SystemExit(f"vias not GND after pour: {dict(via_nets)}")
    finally:
        mcp.close()


if __name__ == "__main__":
    main()
