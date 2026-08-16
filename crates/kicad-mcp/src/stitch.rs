//! Stitching via next to a pin.
//!
//! Places a through-via radially away from the footprint centre, plus a
//! short stub on F.Cu. Sweeps ±15°…±90° if the natural spot is blocked.
//! Never lands on a pad or through a track. The stub Pad→Via is checked
//! the same way (no crossing a foreign track/pad/via). Thru-hole / NPTH
//! pads are skipped. Pads that already have a same-net via nearby are skipped.

use std::collections::HashMap;
use std::path::Path;

use crate::copper::{track_any_coded, via_any_coded};
use crate::kicad::{jlc_pretty_dir, FootprintInfo, Kicad, TrackInfo, ViaInfo};
use crate::nets::NetCodes;
use crate::place::{load_template, world_xy, ModPad, ModPadKind, PlaceSpec};

const DEFAULT_VIA_DRILL_MM: f64 = 0.3;
const DEFAULT_VIA_SIZE_MM: f64 = 0.6;
const DEFAULT_STUB_MM: f64 = 0.25;
/// Pad-edge ↔ via-copper. 0.35 mm meets JLCPCB tented/plug and assembly via-to-SMD.
const CLEARANCE_MM: f64 = 0.35;
const STEP_DEG: f64 = 15.0;
const MAX_DEG: f64 = 90.0;
/// Same-net via this close to *this* pad already counts as stitched.
/// Keep it pad-local: a neighbour LED via (~2–3 mm) must not skip a cap.
const ALREADY_MM: f64 = 0.5;

#[derive(Debug, Clone)]
struct PadGeom {
    reference: String,
    pin: String,
    net: String,
    x_mm: f64,
    y_mm: f64,
    radius_mm: f64,
    fp_x: f64,
    fp_y: f64,
    smd: bool,
}

#[derive(Debug, Clone)]
pub struct StitchPlan {
    pub reference: String,
    pub pin: String,
    pub net: String,
    pub via_x_mm: f64,
    pub via_y_mm: f64,
    pub pad_x_mm: f64,
    pub pad_y_mm: f64,
}

pub struct StitchArgs {
    pub reference: Option<String>,
    pub pin: Option<String>,
    pub net: Option<String>,
    pub drill_mm: Option<f64>,
    pub size_mm: Option<f64>,
    pub stub_width_mm: Option<f64>,
}

pub async fn stitch_vias(k: &Kicad, args: StitchArgs) -> Result<serde_json::Value, String> {
    let ref_s = args.reference.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let pin_s = args.pin.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let net_s = args.net.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match (ref_s, pin_s, net_s) {
        (Some(_), Some(_), _) | (_, _, Some(_)) => {}
        _ => {
            return Err(
                "stitch_via needs reference+pin (one pad) or net (every SMD pad on that net)"
                    .into(),
            )
        }
    }

    let size = args.size_mm.unwrap_or(DEFAULT_VIA_SIZE_MM);
    let drill = args.drill_mm.unwrap_or(DEFAULT_VIA_DRILL_MM);
    let stub_w = args.stub_width_mm.unwrap_or(DEFAULT_STUB_MM);
    if !(0.4..=2.0).contains(&size) {
        return Err("via size_mm must be between 0.4 and 2.0".into());
    }
    if !(0.2..=1.5).contains(&drill) || drill >= size {
        return Err("via drill_mm must be between 0.2 and 1.5 and smaller than size_mm".into());
    }
    if !(0.1..=1.0).contains(&stub_w) {
        return Err("stub_width_mm must be between 0.1 and 1.0".into());
    }

    let dir = k.project_dir().await?;
    let pretty = jlc_pretty_dir(&dir);
    let fps = k.footprints().await?;
    let netlist = k.pad_netlist().await?;
    let tracks = k.tracks().await.unwrap_or_default();
    let vias = k.vias().await.unwrap_or_default();
    let pads = board_pads(&pretty, &fps, &netlist)?;

    let targets: Vec<&PadGeom> = if let (Some(r), Some(p)) = (ref_s, pin_s) {
        let hits: Vec<_> = pads
            .iter()
            .filter(|pad| pad.reference == r && pad.pin == p)
            .collect();
        if hits.is_empty() {
            return Err(format!("no such pin: {r}.{p}"));
        }
        if let Some(want) = net_s {
            if hits.iter().any(|h| h.net != want) {
                let got = hits
                    .iter()
                    .map(|h| h.net.as_str())
                    .find(|n| *n != want)
                    .unwrap_or("");
                return Err(format!(
                    "{r}.{p} is on net {got} — connect_pins first if you wanted {want}"
                ));
            }
        }
        hits
    } else {
        let want = net_s.expect("net checked above");
        pads.iter().filter(|pad| pad.net == want).collect()
    };
    if targets.is_empty() {
        return Err(format!(
            "no pads on net {} — connect_pins first",
            net_s.unwrap_or("?")
        ));
    }
    const STITCH_MAX: usize = 250;
    if targets.len() > STITCH_MAX {
        return Err(format!(
            "stitch_via max {STITCH_MAX} pads (got {}) — call again for the rest",
            targets.len()
        ));
    }

    let via_r = size / 2.0;
    let mut extra_vias: Vec<(f64, f64)> = Vec::new();
    let mut extra_tracks: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut placed = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for pad in &targets {
        if !pad.smd {
            skipped.push(json_skip(pad, "not an SMD pad (PTH/NPTH)"));
            continue;
        }
        if pad.net.is_empty() || pad.net == "unconnected" {
            skipped.push(json_skip(pad, "pad has no net — connect_pins first"));
            continue;
        }
        if already_stitched(pad, &vias, &extra_vias) {
            skipped.push(json_skip(pad, "same-net via already next to this pad"));
            continue;
        }
        match pick_spot(
            pad,
            via_r,
            stub_w,
            &pads,
            &vias,
            &tracks,
            &extra_vias,
            &extra_tracks,
        ) {
            Some((vx, vy)) => {
                extra_vias.push((vx, vy));
                extra_tracks.push((pad.x_mm, pad.y_mm, vx, vy));
                placed.push(StitchPlan {
                    reference: pad.reference.clone(),
                    pin: pad.pin.clone(),
                    net: pad.net.clone(),
                    via_x_mm: round4(vx),
                    via_y_mm: round4(vy),
                    pad_x_mm: round4(pad.x_mm),
                    pad_y_mm: round4(pad.y_mm),
                });
            }
            None => failed.push(json_skip(
                pad,
                "no free spot within ±90° (via or stub would hit a pad/track)",
            )),
        }
    }

    if placed.is_empty() {
        return Ok(serde_json::json!({
            "ok": failed.is_empty(),
            "placed": [],
            "placed_count": 0,
            "skipped": skipped,
            "failed": failed,
        }));
    }

    let mut codes = NetCodes::from_board(&k.board_nets().await.unwrap_or_default());
    let mut items = Vec::with_capacity(placed.len() * 2);
    for p in &placed {
        let code = codes.code_for(&p.net);
        items.push(via_any_coded(
            p.via_x_mm,
            p.via_y_mm,
            &p.net,
            Some(drill),
            Some(size),
            code,
        )?);
        items.push(track_any_coded(
            p.pad_x_mm,
            p.pad_y_mm,
            p.via_x_mm,
            p.via_y_mm,
            Some(stub_w),
            crate::copper::parse_copper_layer(Some("F.Cu"))?,
            &p.net,
            code,
        )?);
    }

    let session = k.begin_commit().await?;
    match k.create_items(items).await {
        Ok(n) => {
            k.end_commit(session, &format!("kicad-mcp stitch {} vias", placed.len()))
                .await?;
            let _ = k.refresh().await;
            Ok(serde_json::json!({
                "ok": failed.is_empty(),
                "placed": placed.iter().map(|p| serde_json::json!({
                    "ref": p.reference,
                    "pin": p.pin,
                    "net": p.net,
                    "x_mm": p.via_x_mm,
                    "y_mm": p.via_y_mm,
                })).collect::<Vec<_>>(),
                "placed_count": placed.len(),
                "items_created": n,
                "skipped": skipped,
                "failed": failed,
            }))
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            Err(e)
        }
    }
}

fn board_pads(
    pretty: &Path,
    fps: &[FootprintInfo],
    netlist: &[kicad_ipc_rs::model::board::PadNetEntry],
) -> Result<Vec<PadGeom>, String> {
    let mut net_of: HashMap<(String, String), String> = HashMap::new();
    for p in netlist {
        let Some(r) = p.footprint_reference.as_deref() else {
            continue;
        };
        let name = p.net_name.as_deref().unwrap_or("").to_string();
        net_of.insert((r.to_string(), p.pad_number.clone()), name);
    }

    let mut templates: HashMap<String, Vec<ModPad>> = HashMap::new();
    let mut out = Vec::new();
    for fp in fps {
        let Some(reference) = fp.reference.as_deref() else {
            continue;
        };
        let Some(value) = fp.value.as_deref() else {
            continue;
        };
        let Some(fx) = fp.x_mm else { continue };
        let Some(fy) = fp.y_mm else { continue };
        let rot = fp.rotation_deg.unwrap_or(0.0);
        if !templates.contains_key(value) {
            match load_template(pretty, value) {
                Ok(t) => {
                    templates.insert(value.to_string(), t.pads);
                }
                Err(_) => continue,
            }
        }
        let pads = templates.get(value).expect("just inserted");
        let spec = PlaceSpec {
            template: value,
            reference,
            x_mm: fx,
            y_mm: fy,
            rotation_deg: rot,
            pads,
        };
        for pad in pads {
            let (x, y) = world_xy(pad.x_mm, pad.y_mm, &spec);
            let net = net_of
                .get(&(reference.to_string(), pad.number.clone()))
                .cloned()
                .unwrap_or_default();
            out.push(PadGeom {
                reference: reference.to_string(),
                pin: pad.number.clone(),
                net,
                x_mm: x,
                y_mm: y,
                radius_mm: pad.width_mm.min(pad.height_mm) / 2.0,
                fp_x: fx,
                fp_y: fy,
                smd: matches!(pad.kind, ModPadKind::SmdFront | ModPadKind::SmdBack),
            });
        }
    }
    Ok(out)
}

fn already_stitched(pad: &PadGeom, vias: &[ViaInfo], extra: &[(f64, f64)]) -> bool {
    let limit = pad.radius_mm + DEFAULT_VIA_SIZE_MM / 2.0 + ALREADY_MM;
    for v in vias {
        if v.net.as_deref() != Some(pad.net.as_str()) {
            continue;
        }
        let (Some(x), Some(y)) = (v.x_mm, v.y_mm) else {
            continue;
        };
        if hypot(x - pad.x_mm, y - pad.y_mm) <= limit {
            return true;
        }
    }
    extra
        .iter()
        .any(|(x, y)| hypot(x - pad.x_mm, y - pad.y_mm) <= limit)
}

fn pick_spot(
    pad: &PadGeom,
    via_r: f64,
    stub_w: f64,
    pads: &[PadGeom],
    vias: &[ViaInfo],
    tracks: &[TrackInfo],
    extra_vias: &[(f64, f64)],
    extra_tracks: &[(f64, f64, f64, f64)],
) -> Option<(f64, f64)> {
    for (vx, vy) in candidates(pad, via_r) {
        if spot_clear(
            pad, vx, vy, via_r, stub_w, pads, vias, tracks, extra_vias, extra_tracks,
        ) && stub_clear(
            pad, vx, vy, stub_w, pads, vias, tracks, extra_vias, extra_tracks,
        ) {
            return Some((vx, vy));
        }
    }
    None
}

pub(crate) fn candidate_points(
    pad_x: f64,
    pad_y: f64,
    fp_x: f64,
    fp_y: f64,
    gap: f64,
) -> Vec<(f64, f64)> {
    let dx = pad_x - fp_x;
    let dy = pad_y - fp_y;
    let len = hypot(dx, dy);
    let (ux, uy) = if len < 1e-6 {
        (1.0, 0.0)
    } else {
        (dx / len, dy / len)
    };
    let mut degs = vec![0.0];
    let mut deg = STEP_DEG;
    while deg <= MAX_DEG + f64::EPSILON {
        degs.push(deg);
        degs.push(-deg);
        deg += STEP_DEG;
    }
    degs.into_iter()
        .map(|deg| {
            let (s, c) = deg.to_radians().sin_cos();
            let (rx, ry) = (ux * c - uy * s, ux * s + uy * c);
            (pad_x + rx * gap, pad_y + ry * gap)
        })
        .collect()
}

fn candidates(pad: &PadGeom, via_r: f64) -> Vec<(f64, f64)> {
    let gap = pad.radius_mm + CLEARANCE_MM + via_r;
    candidate_points(pad.x_mm, pad.y_mm, pad.fp_x, pad.fp_y, gap)
}

fn spot_clear(
    owner: &PadGeom,
    vx: f64,
    vy: f64,
    via_r: f64,
    stub_w: f64,
    pads: &[PadGeom],
    vias: &[ViaInfo],
    tracks: &[TrackInfo],
    extra_vias: &[(f64, f64)],
    extra_tracks: &[(f64, f64, f64, f64)],
) -> bool {
    let need_pad = via_r + CLEARANCE_MM;
    for p in pads {
        if hypot(vx - p.x_mm, vy - p.y_mm) < p.radius_mm + need_pad {
            return false;
        }
    }
    for v in vias {
        let (Some(x), Some(y)) = (v.x_mm, v.y_mm) else {
            continue;
        };
        if hypot(vx - x, vy - y) < DEFAULT_VIA_SIZE_MM + CLEARANCE_MM {
            return false;
        }
    }
    for (x, y) in extra_vias {
        if hypot(vx - x, vy - y) < DEFAULT_VIA_SIZE_MM + CLEARANCE_MM {
            return false;
        }
    }
    let stub_half = stub_w / 2.0;
    for t in tracks {
        let (Some(a), Some(b)) = (t.a_mm, t.b_mm) else {
            continue;
        };
        let half = t.width_mm.unwrap_or(stub_w) / 2.0;
        if dist_point_seg(vx, vy, a[0], a[1], b[0], b[1]) < via_r + half + CLEARANCE_MM {
            return false;
        }
    }
    for (ax, ay, bx, by) in extra_tracks {
        if *ax == owner.x_mm && *ay == owner.y_mm {
            continue;
        }
        if dist_point_seg(vx, vy, *ax, *ay, *bx, *by) < via_r + stub_half + CLEARANCE_MM {
            return false;
        }
    }
    true
}

/// Stub from the owner's pad centre to the candidate via. The owner pad
/// and the via itself are allowed to touch the stub; everything else is not.
fn stub_clear(
    owner: &PadGeom,
    vx: f64,
    vy: f64,
    stub_w: f64,
    pads: &[PadGeom],
    vias: &[ViaInfo],
    tracks: &[TrackInfo],
    extra_vias: &[(f64, f64)],
    extra_tracks: &[(f64, f64, f64, f64)],
) -> bool {
    let half = stub_w / 2.0;
    let need = half + CLEARANCE_MM;
    for p in pads {
        if p.reference == owner.reference && p.pin == owner.pin {
            continue;
        }
        if dist_point_seg(p.x_mm, p.y_mm, owner.x_mm, owner.y_mm, vx, vy) < p.radius_mm + need {
            return false;
        }
    }
    for v in vias {
        let (Some(x), Some(y)) = (v.x_mm, v.y_mm) else {
            continue;
        };
        if dist_point_seg(x, y, owner.x_mm, owner.y_mm, vx, vy) < DEFAULT_VIA_SIZE_MM / 2.0 + need {
            return false;
        }
    }
    for (x, y) in extra_vias {
        if dist_point_seg(*x, *y, owner.x_mm, owner.y_mm, vx, vy) < DEFAULT_VIA_SIZE_MM / 2.0 + need {
            return false;
        }
    }
    for t in tracks {
        let (Some(a), Some(b)) = (t.a_mm, t.b_mm) else {
            continue;
        };
        let th = t.width_mm.unwrap_or(stub_w) / 2.0;
        if dist_seg_seg(owner.x_mm, owner.y_mm, vx, vy, a[0], a[1], b[0], b[1]) < half + th + CLEARANCE_MM
        {
            return false;
        }
    }
    for (ax, ay, bx, by) in extra_tracks {
        if *ax == owner.x_mm && *ay == owner.y_mm {
            continue;
        }
        if dist_seg_seg(owner.x_mm, owner.y_mm, vx, vy, *ax, *ay, *bx, *by) < stub_w + CLEARANCE_MM {
            return false;
        }
    }
    true
}

fn dist_point_seg(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        ((px - ax) * dx + (py - ay) * dy) / len2
    }
    .clamp(0.0, 1.0);
    hypot(px - (ax + t * dx), py - (ay + t * dy))
}

fn orient(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> f64 {
    (by - ay) * (cx - bx) - (bx - ax) * (cy - by)
}

fn on_seg(ax: f64, ay: f64, bx: f64, by: f64, px: f64, py: f64) -> bool {
    px >= ax.min(bx) - 1e-9
        && px <= ax.max(bx) + 1e-9
        && py >= ay.min(by) - 1e-9
        && py <= ay.max(by) + 1e-9
}

fn segments_cross(
    a0x: f64,
    a0y: f64,
    a1x: f64,
    a1y: f64,
    b0x: f64,
    b0y: f64,
    b1x: f64,
    b1y: f64,
) -> bool {
    let o1 = orient(a0x, a0y, a1x, a1y, b0x, b0y);
    let o2 = orient(a0x, a0y, a1x, a1y, b1x, b1y);
    let o3 = orient(b0x, b0y, b1x, b1y, a0x, a0y);
    let o4 = orient(b0x, b0y, b1x, b1y, a1x, a1y);
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return true;
    }
    (o1.abs() < 1e-9 && on_seg(a0x, a0y, a1x, a1y, b0x, b0y))
        || (o2.abs() < 1e-9 && on_seg(a0x, a0y, a1x, a1y, b1x, b1y))
        || (o3.abs() < 1e-9 && on_seg(b0x, b0y, b1x, b1y, a0x, a0y))
        || (o4.abs() < 1e-9 && on_seg(b0x, b0y, b1x, b1y, a1x, a1y))
}

fn dist_seg_seg(
    a0x: f64,
    a0y: f64,
    a1x: f64,
    a1y: f64,
    b0x: f64,
    b0y: f64,
    b1x: f64,
    b1y: f64,
) -> f64 {
    if segments_cross(a0x, a0y, a1x, a1y, b0x, b0y, b1x, b1y) {
        return 0.0;
    }
    dist_point_seg(a0x, a0y, b0x, b0y, b1x, b1y)
        .min(dist_point_seg(a1x, a1y, b0x, b0y, b1x, b1y))
        .min(dist_point_seg(b0x, b0y, a0x, a0y, a1x, a1y))
        .min(dist_point_seg(b1x, b1y, a0x, a0y, a1x, a1y))
}

fn hypot(x: f64, y: f64) -> f64 {
    x.hypot(y)
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn json_skip(pad: &PadGeom, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "ref": pad.reference,
        "pin": pad.pin,
        "reason": reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_candidate_is_radially_outward() {
        // Footprint at origin, pad at +x. Via should go further +x.
        let pts = candidate_points(2.0, 0.0, 0.0, 0.0, 1.5);
        assert!((pts[0].0 - 3.5).abs() < 1e-9);
        assert!(pts[0].1.abs() < 1e-9);
        assert!(pts.len() > 4);
    }

    #[test]
    fn point_to_segment_hits_the_middle() {
        assert!((dist_point_seg(0.0, 1.0, -2.0, 0.0, 2.0, 0.0) - 1.0).abs() < 1e-9);
        assert!(dist_point_seg(5.0, 0.0, 0.0, 0.0, 1.0, 0.0) > 3.9);
    }

    #[test]
    fn crossing_tracks_have_zero_distance() {
        assert!(dist_seg_seg(0.0, 0.0, 2.0, 0.0, 1.0, -1.0, 1.0, 1.0) < 1e-9);
        assert!((dist_seg_seg(0.0, 0.0, 10.0, 0.0, 3.0, 1.0, 7.0, 1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn stub_rejected_when_a_track_crosses_pad_to_via() {
        let owner = PadGeom {
            reference: "U1".into(),
            pin: "1".into(),
            net: "GND".into(),
            x_mm: 0.0,
            y_mm: 0.0,
            radius_mm: 0.6,
            fp_x: -2.0,
            fp_y: 0.0,
            smd: true,
        };
        let tracks = [TrackInfo {
            id: None,
            net: Some("Net1".into()),
            layer: "F.Cu".into(),
            width_mm: Some(0.25),
            a_mm: Some([1.0, -2.0]),
            b_mm: Some([1.0, 2.0]),
        }];
        assert!(
            !stub_clear(&owner, 2.0, 0.0, 0.25, &[], &[], &tracks, &[], &[]),
            "data track between pad and via must block the stub"
        );
        let clear = [TrackInfo {
            id: None,
            net: Some("Net1".into()),
            layer: "F.Cu".into(),
            width_mm: Some(0.25),
            a_mm: Some([1.0, 3.0]),
            b_mm: Some([2.0, 3.0]),
        }];
        assert!(stub_clear(
            &owner, 2.0, 0.0, 0.25, &[], &[], &clear, &[], &[]
        ));
    }
}
