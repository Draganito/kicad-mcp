//! JLCPCB silk-to-pad / silk-to-hole for plotting overlay outlines.
//!
//! Plotting silk (`F.Silkscreen` / `B.Silkscreen`) must not *print* on copper
//! pads or holes. The outline stays; the stroke is **gapped** where it would
//! collide (JLCPCB 0.15 mm + half stroke). User layers are not checked.
//! Same-side only: front silk vs `F.Cu`, back silk vs `B.Cu`, PTH/NPTH
//! holes on both. `kind: rect` + `reference` is the **package body**
//! (JLCPCB `L…-W…` / EIA size at the footprint origin), not the pad
//! envelope; clipping opens the pads. Cell text on a pad is omitted.
//! Refused only when nothing remains.

use crate::graphics::{
    overlay_segment, parse_graphic_layer, polygon_vertices_mm_from_any, shape_snap_from_any,
    GraphicLayer, ShapeMade, ShapeSpec,
};
use crate::pads::PadRow;

/// JLCPCB silk-to-pad / silk-to-hole, same number as the silk min stroke.
pub const SILK_TO_PAD_MM: f64 = 0.15;
/// Drop clipped remnants shorter than a drawable line.
const MIN_PIECE_MM: f64 = 0.5;
/// Extra millimetres so remaining ink sits strictly outside the keepout
/// (gerber rounding / closed AABB edges).
const CLIP_SLACK_MM: f64 = 0.01;
const HIT_EPS_MM: f64 = 1e-6;
const CIRCLE_SIDES: usize = 48;

#[derive(Clone, Copy, Debug)]
struct BoxMm {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl BoxMm {
    fn from_center(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            min_x: x - w / 2.0,
            min_y: y - h / 2.0,
            max_x: x + w / 2.0,
            max_y: y + h / 2.0,
        }
    }

    fn expand(self, d: f64) -> Self {
        Self {
            min_x: self.min_x - d,
            min_y: self.min_y - d,
            max_x: self.max_x + d,
            max_y: self.max_y + d,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    fn w(self) -> f64 {
        self.max_x - self.min_x
    }

    fn h(self) -> f64 {
        self.max_y - self.min_y
    }

    fn cx(self) -> f64 {
        (self.min_x + self.max_x) / 2.0
    }

    fn cy(self) -> f64 {
        (self.min_y + self.max_y) / 2.0
    }

    fn overlaps(self, other: Self) -> bool {
        self.min_x < other.max_x - HIT_EPS_MM
            && self.max_x > other.min_x + HIT_EPS_MM
            && self.min_y < other.max_y - HIT_EPS_MM
            && self.max_y > other.min_y + HIT_EPS_MM
    }
}

#[derive(Clone, Debug)]
struct Keepout {
    copper: Option<BoxMm>,
    drill: Option<(f64, f64, f64)>,
}

/// Four board-mm corners of the package body (already rotated).
pub type BodyCorners = [(f64, f64); 4];

fn corners_aabb(corners: &BodyCorners) -> BoxMm {
    let mut acc = BoxMm {
        min_x: corners[0].0,
        min_y: corners[0].1,
        max_x: corners[0].0,
        max_y: corners[0].1,
    };
    for &(x, y) in &corners[1..] {
        acc.min_x = acc.min_x.min(x);
        acc.min_y = acc.min_y.min(y);
        acc.max_x = acc.max_x.max(x);
        acc.max_y = acc.max_y.max(y);
    }
    acc
}

fn corners_axis_aligned(corners: &BodyCorners) -> bool {
    let closed = [
        corners[0],
        corners[1],
        corners[2],
        corners[3],
        corners[0],
    ];
    closed.windows(2).all(|w| {
        (w[0].0 - w[1].0).abs() < 0.02 || (w[0].1 - w[1].1).abs() < 0.02
    })
}

/// Fill `kind: rect` from the package body (JLCPCB L/W at the footprint
/// origin). `body` is four board corners; omit it only when the template
/// has no package size (then the pad AABB is the fallback).
pub fn apply_reference(
    spec: &mut ShapeSpec,
    pads: &[PadRow],
    body: Option<BodyCorners>,
) -> Result<(), String> {
    let raw = spec
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "reference is empty".to_string())?;
    match spec.kind.trim().to_ascii_lowercase().as_str() {
        "rect" | "rectangle" | "box" => {}
        other => {
            return Err(format!(
                "reference is only for kind rect (package body), not {other}"
            ));
        }
    }
    if spec.origin_x_mm.is_some()
        || spec.origin_y_mm.is_some()
        || spec.center_x_mm.is_some()
        || spec.center_y_mm.is_some()
        || spec.width_mm.is_some()
        || spec.height_mm.is_some()
        || spec.x_mm.is_some()
        || spec.y_mm.is_some()
        || spec.radius_mm.is_some()
        || spec.diameter_mm.is_some()
        || spec.a_x_mm.is_some()
        || spec.a_y_mm.is_some()
        || spec.b_x_mm.is_some()
        || spec.b_y_mm.is_some()
        || !spec.points.is_empty()
        || spec.rows.is_some()
        || spec.cols.is_some()
        || spec.cell_width_mm.is_some()
        || spec.cell_height_mm.is_some()
        || spec
            .cells
            .iter()
            .any(|row| row.iter().any(|c| !c.is_empty()))
    {
        return Err("reference draws the package body — omit origin/centre/size/points".into());
    }
    if pads.is_empty() {
        return Err(format!("no pads for {raw} on the board"));
    }
    let layer = parse_graphic_layer(spec.layer.as_deref())?;
    let _stroke = crate::graphics::resolve_stroke(spec.stroke_mm, layer.plots)?;
    if let Some(corners) = body {
        if corners_axis_aligned(&corners) {
            let aabb = corners_aabb(&corners);
            if aabb.w() < 0.5 || aabb.h() < 0.5 {
                return Err(format!(
                    "package body would be {:.3} × {:.3} mm (min 0.5 mm)",
                    aabb.w(),
                    aabb.h()
                ));
            }
            if aabb.w() > 400.0 || aabb.h() > 400.0 {
                return Err("package body max 400 × 400 mm".into());
            }
            spec.center_x_mm = Some(round4(aabb.cx()));
            spec.center_y_mm = Some(round4(aabb.cy()));
            spec.width_mm = Some(round4(aabb.w()));
            spec.height_mm = Some(round4(aabb.h()));
        } else {
            spec.kind = "polygon".into();
            spec.points = corners.iter().copied().collect();
            let aabb = corners_aabb(&corners);
            spec.center_x_mm = Some(round4(aabb.cx()));
            spec.center_y_mm = Some(round4(aabb.cy()));
            spec.width_mm = Some(round4(aabb.w()));
            spec.height_mm = Some(round4(aabb.h()));
        }
        return Ok(());
    }
    let aabb = part_outline_aabb(pads, layer)?;
    spec.center_x_mm = Some(round4(aabb.cx()));
    spec.center_y_mm = Some(round4(aabb.cy()));
    spec.width_mm = Some(round4(aabb.w()));
    spec.height_mm = Some(round4(aabb.h()));
    Ok(())
}

fn part_outline_aabb(pads: &[PadRow], layer: GraphicLayer) -> Result<BoxMm, String> {
    let mut acc: Option<BoxMm> = None;
    for pad in pads {
        if !pad_in_outline(pad, layer) {
            continue;
        }
        if let Some(copper) = pad_copper_aabb(pad) {
            acc = Some(match acc {
                Some(a) => a.union(copper),
                None => copper,
            });
        }
        if let Some(hole) = pad_hole_aabb(pad) {
            acc = Some(match acc {
                Some(a) => a.union(hole),
                None => hole,
            });
        }
    }
    let Some(union) = acc else {
        return Err(format!(
            "{} has no copper or holes on {} — cannot draw a part outline",
            pads.first().map(|p| p.reference.as_str()).unwrap_or("?"),
            layer.name
        ));
    };
    if union.w() < 0.5 || union.h() < 0.5 {
        return Err(format!(
            "part outline would be {:.3} × {:.3} mm (min 0.5 mm)",
            union.w(),
            union.h()
        ));
    }
    if union.w() > 400.0 || union.h() > 400.0 {
        return Err("part outline max 400 × 400 mm".into());
    }
    Ok(union)
}

/// Gap plotting silk where the stroke or cell text sits on a same-side
/// pad or hole. User layers are copied through. Refused only if every
/// plotting item disappears.
pub fn clip_plotting_silk(made: Vec<ShapeMade>, pads: &[PadRow]) -> Result<Vec<ShapeMade>, String> {
    if !made.iter().any(|m| m.layer.plots) {
        return Ok(made);
    }
    let front = keepouts_for_silk(pads, false);
    let back = keepouts_for_silk(pads, true);
    let had_plotting = made.iter().any(|m| m.layer.plots);
    let mut out = Vec::with_capacity(made.len());
    let mut kept_plotting = 0usize;
    for m in made {
        if !m.layer.plots {
            out.push(m);
            continue;
        }
        let kos = if m.layer.name == "B.Silkscreen" {
            &back
        } else {
            &front
        };
        let clipped = clip_one(m, kos)?;
        kept_plotting += clipped.len();
        out.extend(clipped);
    }
    if had_plotting && kept_plotting == 0 {
        return Err(format!(
            "F/B.Silkscreen overlay is entirely on pads/holes after {SILK_TO_PAD_MM} mm gaps — nothing left to draw"
        ));
    }
    Ok(out)
}

fn clip_one(m: ShapeMade, keepouts: &[Keepout]) -> Result<Vec<ShapeMade>, String> {
    if m.kind == "text" {
        let Some((x, y, size, body)) = crate::silk::text_metrics_from_any(&m.item) else {
            return Err("cannot DFM-check overlay text".into());
        };
        let n = body.chars().count().max(1) as f64;
        let w = (n * size * 0.9).max(size);
        let h = size;
        let ink = BoxMm::from_center(x, y, w, h);
        if keepouts.iter().any(|k| filled_hits_keepout(ink, 0.0, k)) {
            return Ok(Vec::new());
        }
        return Ok(vec![m]);
    }
    let stroke = if m.stroke_mm.is_finite() {
        m.stroke_mm.max(0.0)
    } else {
        0.0
    };
    if m.kind == "circle" {
        let snap = shape_snap_from_any(&m.item)
            .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
        let x = snap
            .x_mm
            .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
        let y = snap
            .y_mm
            .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
        let r = snap
            .radius_mm
            .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
        if !keepouts
            .iter()
            .any(|k| ring_hits_keepout(x, y, r, stroke, k))
        {
            return Ok(vec![m]);
        }
    }
    let edges = item_centerlines(&m)?;
    if edges.is_empty() {
        return Ok(vec![m]);
    }
    let mut pieces = Vec::new();
    let mut gapped = false;
    for edge in &edges {
        let clipped = clip_segment(*edge, stroke, keepouts);
        let orig_len = hypot(edge[1][0] - edge[0][0], edge[1][1] - edge[0][1]);
        let kept_len: f64 = clipped
            .iter()
            .map(|p| hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]))
            .sum();
        if clipped.len() != 1 || (orig_len - kept_len).abs() > 0.02 {
            gapped = true;
        }
        pieces.extend(clipped);
    }
    if !gapped {
        return Ok(vec![m]);
    }
    Ok(pieces
        .into_iter()
        .map(|p| {
            overlay_segment(
                m.group_kind,
                m.layer,
                stroke,
                m.tag.clone(),
                p[0][0],
                p[0][1],
                p[1][0],
                p[1][1],
            )
        })
        .collect())
}

fn item_centerlines(m: &ShapeMade) -> Result<Vec<[[f64; 2]; 2]>, String> {
    match m.kind {
        "rect" => {
            let snap = shape_snap_from_any(&m.item)
                .ok_or_else(|| "cannot DFM-check overlay rect".to_string())?;
            let x0 = snap
                .origin_x_mm
                .ok_or_else(|| "cannot DFM-check overlay rect".to_string())?;
            let y0 = snap
                .origin_y_mm
                .ok_or_else(|| "cannot DFM-check overlay rect".to_string())?;
            let w = snap
                .width_mm
                .ok_or_else(|| "cannot DFM-check overlay rect".to_string())?;
            let h = snap
                .height_mm
                .ok_or_else(|| "cannot DFM-check overlay rect".to_string())?;
            Ok(rect_edges(x0, y0, w, h).to_vec())
        }
        "segment" => {
            let snap = shape_snap_from_any(&m.item)
                .ok_or_else(|| "cannot DFM-check overlay line".to_string())?;
            let a = snap
                .a_mm
                .ok_or_else(|| "cannot DFM-check overlay line".to_string())?;
            let b = snap
                .b_mm
                .ok_or_else(|| "cannot DFM-check overlay line".to_string())?;
            Ok(vec![[a, b]])
        }
        "circle" => {
            let snap = shape_snap_from_any(&m.item)
                .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
            let x = snap
                .x_mm
                .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
            let y = snap
                .y_mm
                .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
            let r = snap
                .radius_mm
                .ok_or_else(|| "cannot DFM-check overlay circle".to_string())?;
            Ok(circle_edges(x, y, r, CIRCLE_SIDES))
        }
        "polygon" => {
            let pts = polygon_vertices_mm_from_any(&m.item)
                .ok_or_else(|| "cannot DFM-check overlay polygon".to_string())?;
            Ok(closed_edges(&pts))
        }
        other => Err(format!("cannot DFM-check overlay kind {other}")),
    }
}

fn circle_edges(x: f64, y: f64, r: f64, n: usize) -> Vec<[[f64; 2]; 2]> {
    let n = n.max(8);
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let a = (i as f64) * std::f64::consts::TAU / n as f64;
        pts.push((x + r * a.cos(), y + r * a.sin()));
    }
    closed_edges(&pts)
}

fn clip_segment(seg: [[f64; 2]; 2], stroke: f64, keepouts: &[Keepout]) -> Vec<[[f64; 2]; 2]> {
    let a = seg[0];
    let b = seg[1];
    let len = hypot(b[0] - a[0], b[1] - a[1]);
    if len < HIT_EPS_MM {
        return Vec::new();
    }
    let grow = SILK_TO_PAD_MM + stroke / 2.0 + CLIP_SLACK_MM;
    let mut blocked: Vec<(f64, f64)> = Vec::new();
    for k in keepouts {
        if let Some(copper) = k.copper {
            if let Some(iv) = segment_aabb_t(a, b, copper.expand(grow)) {
                blocked.push(iv);
            }
        }
        if let Some((x, y, r)) = k.drill {
            if let Some(iv) = segment_circle_t(a, b, x, y, r + grow) {
                blocked.push(iv);
            }
        }
    }
    merge_intervals(&mut blocked);
    let mut out = Vec::new();
    for (t0, t1) in invert_intervals(&blocked) {
        let p0 = lerp(a, b, t0);
        let p1 = lerp(a, b, t1);
        let plen = hypot(p1[0] - p0[0], p1[1] - p0[1]);
        if plen + HIT_EPS_MM >= MIN_PIECE_MM {
            out.push([p0, p1]);
        }
    }
    out
}

fn segment_aabb_t(a: [f64; 2], b: [f64; 2], aabb: BoxMm) -> Option<(f64, f64)> {
    let mut t0 = 0.0;
    let mut t1 = 1.0;
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    if clip_t(-dx, a[0] - aabb.min_x, &mut t0, &mut t1)
        && clip_t(dx, aabb.max_x - a[0], &mut t0, &mut t1)
        && clip_t(-dy, a[1] - aabb.min_y, &mut t0, &mut t1)
        && clip_t(dy, aabb.max_y - a[1], &mut t0, &mut t1)
    {
        Some((t0, t1))
    } else {
        None
    }
}

fn segment_circle_t(a: [f64; 2], b: [f64; 2], cx: f64, cy: f64, r: f64) -> Option<(f64, f64)> {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let fx = a[0] - cx;
    let fy = a[1] - cy;
    let aa = dx * dx + dy * dy;
    let bb = 2.0 * (fx * dx + fy * dy);
    let cc = fx * fx + fy * fy - r * r;
    if aa < 1e-18 {
        return if cc <= 0.0 { Some((0.0, 1.0)) } else { None };
    }
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return if cc <= 0.0 { Some((0.0, 1.0)) } else { None };
    }
    let s = disc.sqrt();
    let mut u0 = (-bb - s) / (2.0 * aa);
    let mut u1 = (-bb + s) / (2.0 * aa);
    if u0 > u1 {
        std::mem::swap(&mut u0, &mut u1);
    }
    let lo = u0.max(0.0);
    let hi = u1.min(1.0);
    if hi - lo > HIT_EPS_MM {
        Some((lo, hi))
    } else {
        None
    }
}

fn merge_intervals(iv: &mut Vec<(f64, f64)>) {
    if iv.is_empty() {
        return;
    }
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(iv.len());
    let mut cur = iv[0];
    for &(s, e) in iv.iter().skip(1) {
        if s <= cur.1 + HIT_EPS_MM {
            cur.1 = cur.1.max(e);
        } else {
            out.push(cur);
            cur = (s, e);
        }
    }
    out.push(cur);
    *iv = out;
}

fn invert_intervals(blocked: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut free = Vec::new();
    let mut t = 0.0;
    for &(s, e) in blocked {
        let s = s.clamp(0.0, 1.0);
        let e = e.clamp(0.0, 1.0);
        if s > t + HIT_EPS_MM {
            free.push((t, s));
        }
        t = t.max(e);
    }
    if t < 1.0 - HIT_EPS_MM {
        free.push((t, 1.0));
    }
    free
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn rect_edges(x0: f64, y0: f64, w: f64, h: f64) -> [[[f64; 2]; 2]; 4] {
    let x1 = x0 + w;
    let y1 = y0 + h;
    [
        [[x0, y0], [x1, y0]],
        [[x1, y0], [x1, y1]],
        [[x1, y1], [x0, y1]],
        [[x0, y1], [x0, y0]],
    ]
}

fn closed_edges(pts: &[(f64, f64)]) -> Vec<[[f64; 2]; 2]> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(pts.len());
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        out.push([[a.0, a.1], [b.0, b.1]]);
    }
    out
}

fn keepouts_for_silk(pads: &[PadRow], silk_is_back: bool) -> Vec<Keepout> {
    pads.iter()
        .filter(|p| pad_faces_silk(p, silk_is_back))
        .map(keepout_from_pad)
        .collect()
}

fn pad_faces_silk(pad: &PadRow, silk_is_back: bool) -> bool {
    if pad.kind == "pth" || pad.kind == "npth" || pad.drill_mm.is_some() {
        return true;
    }
    copper_on_side(pad, silk_is_back)
}

fn pad_in_outline(pad: &PadRow, layer: GraphicLayer) -> bool {
    if !layer.plots {
        return true;
    }
    pad_faces_silk(pad, layer.name == "B.Silkscreen")
}

fn copper_on_side(pad: &PadRow, back: bool) -> bool {
    pad.layers
        .iter()
        .chain(std::iter::once(&pad.layer))
        .any(|n| if back { is_b_cu(n) } else { is_f_cu(n) })
}

fn is_f_cu(n: &str) -> bool {
    let n = n.replace('_', ".").to_ascii_lowercase();
    n == "f.cu" || n.ends_with(".f.cu")
}

fn is_b_cu(n: &str) -> bool {
    let n = n.replace('_', ".").to_ascii_lowercase();
    n == "b.cu" || n.ends_with(".b.cu")
}

fn keepout_from_pad(pad: &PadRow) -> Keepout {
    Keepout {
        copper: pad_copper_aabb(pad),
        drill: pad_drill_circle(pad),
    }
}

fn pad_copper_aabb(pad: &PadRow) -> Option<BoxMm> {
    if pad.width_mm <= 0.0 || pad.height_mm <= 0.0 {
        return None;
    }
    Some(rotated_rect_aabb(
        pad.x_mm,
        pad.y_mm,
        pad.width_mm,
        pad.height_mm,
        pad.rotation_deg,
    ))
}

fn pad_hole_aabb(pad: &PadRow) -> Option<BoxMm> {
    let d = pad.drill_mm?;
    if d <= 0.0 {
        return None;
    }
    let h = pad.drill_h_mm.unwrap_or(d);
    Some(rotated_rect_aabb(
        pad.x_mm,
        pad.y_mm,
        d,
        h,
        pad.rotation_deg,
    ))
}

fn pad_drill_circle(pad: &PadRow) -> Option<(f64, f64, f64)> {
    let d = pad.drill_mm?;
    if d <= 0.0 {
        return None;
    }
    if pad.drill_h_mm.is_some() {
        return None;
    }
    Some((pad.x_mm, pad.y_mm, d / 2.0))
}

fn rotated_rect_aabb(x: f64, y: f64, w: f64, h: f64, rot_deg: f64) -> BoxMm {
    let hw = w / 2.0;
    let hh = h / 2.0;
    let (s, c) = rot_deg.to_radians().sin_cos();
    let corners = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (lx, ly) in corners {
        let px = x + lx * c - ly * s;
        let py = y + lx * s + ly * c;
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    BoxMm {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

fn filled_hits_keepout(ink: BoxMm, extra: f64, k: &Keepout) -> bool {
    let grow = SILK_TO_PAD_MM + extra;
    if let Some(copper) = k.copper {
        if ink.overlaps(copper.expand(grow)) {
            return true;
        }
    }
    if let Some((x, y, r)) = k.drill {
        if dist_aabb_point(ink, x, y) < r + grow - HIT_EPS_MM {
            return true;
        }
    }
    false
}

fn ring_hits_keepout(cx: f64, cy: f64, r: f64, stroke: f64, k: &Keepout) -> bool {
    let hs = stroke / 2.0;
    let inner = (r - hs - SILK_TO_PAD_MM).max(0.0);
    let outer = r + hs + SILK_TO_PAD_MM;
    if let Some(copper) = k.copper {
        if radial_range_overlaps(copper, cx, cy, inner, outer) {
            return true;
        }
    }
    if let Some((x, y, hr)) = k.drill {
        let d = hypot(cx - x, cy - y);
        let dmin = (d - hr).max(0.0);
        let dmax = d + hr;
        if dmin <= outer + HIT_EPS_MM && dmax >= inner - HIT_EPS_MM {
            return true;
        }
    }
    false
}

fn radial_range_overlaps(aabb: BoxMm, cx: f64, cy: f64, inner: f64, outer: f64) -> bool {
    let dmin = dist_aabb_point(aabb, cx, cy);
    let dmax = [
        hypot(aabb.min_x - cx, aabb.min_y - cy),
        hypot(aabb.min_x - cx, aabb.max_y - cy),
        hypot(aabb.max_x - cx, aabb.min_y - cy),
        hypot(aabb.max_x - cx, aabb.max_y - cy),
    ]
    .into_iter()
    .fold(0.0_f64, f64::max);
    dmin <= outer + HIT_EPS_MM && dmax >= inner - HIT_EPS_MM
}

fn dist_aabb_point(aabb: BoxMm, x: f64, y: f64) -> f64 {
    let dx = if x < aabb.min_x {
        aabb.min_x - x
    } else if x > aabb.max_x {
        x - aabb.max_x
    } else {
        0.0
    };
    let dy = if y < aabb.min_y {
        aabb.min_y - y
    } else if y > aabb.max_y {
        y - aabb.max_y
    } else {
        0.0
    };
    hypot(dx, dy)
}

fn clip_t(p: f64, q: f64, t0: &mut f64, t1: &mut f64) -> bool {
    if p.abs() < 1e-18 {
        return q >= 0.0;
    }
    let r = q / p;
    if p < 0.0 {
        if r > *t1 {
            return false;
        }
        if r > *t0 {
            *t0 = r;
        }
    } else {
        if r < *t0 {
            return false;
        }
        if r < *t1 {
            *t1 = r;
        }
    }
    true
}

fn hypot(x: f64, y: f64) -> f64 {
    x.hypot(y)
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Build items then clip plotting silk (unit tests; no KiCad).
#[cfg(test)]
fn clip_spec(spec: &ShapeSpec, pads: &[PadRow]) -> Result<Vec<ShapeMade>, String> {
    let mut spec = spec.clone();
    if let Some(r) = spec
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let of_ref: Vec<_> = pads.iter().filter(|p| p.reference == r).cloned().collect();
        apply_reference(&mut spec, &of_ref, None)?;
    }
    let made = crate::graphics::shape_items(&spec)?;
    clip_plotting_silk(made, pads)
}

#[cfg(test)]
fn u1_body() -> BodyCorners {
    let cx = 129.45;
    let cy = 35.15;
    let h = 1.75;
    [
        (cx - h, cy - h),
        (cx + h, cy - h),
        (cx + h, cy + h),
        (cx - h, cy + h),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smd(
        reference: &str,
        pin: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        rot: f64,
        layer: &str,
    ) -> PadRow {
        PadRow {
            reference: reference.into(),
            pin: pin.into(),
            net: String::new(),
            x_mm: x,
            y_mm: y,
            width_mm: w,
            height_mm: h,
            rotation_deg: rot,
            kind: "smd".into(),
            shape: "rect".into(),
            layer: layer.into(),
            layers: vec![layer.into()],
            drill_mm: None,
            drill_h_mm: None,
        }
    }

    fn u1_pads() -> Vec<PadRow> {
        // WS2812B-MINI-JT01 on the Aristo D2 panel (U1).
        vec![
            smd("U1", "1", 127.8125, 34.26, 1.545, 0.997, 180.0, "F.Cu"),
            smd("U1", "2", 131.0875, 34.26, 1.545, 0.997, 180.0, "F.Cu"),
            smd("U1", "3", 127.8125, 36.04, 1.545, 0.997, 180.0, "F.Cu"),
            smd("U1", "4", 131.0875, 36.04, 1.545, 0.997, 180.0, "F.Cu"),
        ]
    }

    fn neighbor() -> PadRow {
        // ~12.7 mm pitch to the right of U1.
        smd(
            "U2",
            "1",
            127.8125 + 12.7,
            34.26,
            1.545,
            0.997,
            180.0,
            "F.Cu",
        )
    }

    fn assert_silk_clear(made: &[ShapeMade], pads: &[PadRow]) {
        let front = keepouts_for_silk(pads, false);
        let back = keepouts_for_silk(pads, true);
        for m in made {
            if !m.layer.plots {
                continue;
            }
            let kos = if m.layer.name == "B.Silkscreen" {
                &back
            } else {
                &front
            };
            if m.kind == "text" {
                let (x, y, size, body) = crate::silk::text_metrics_from_any(&m.item).unwrap();
                let n = body.chars().count().max(1) as f64;
                let ink = BoxMm::from_center(x, y, (n * size * 0.9).max(size), size);
                assert!(
                    !kos.iter().any(|k| filled_hits_keepout(ink, 0.0, k)),
                    "text {:?} still on a pad",
                    body
                );
                continue;
            }
            let edges = item_centerlines(m).unwrap();
            for edge in edges {
                assert!(
                    !kos.iter().any(|k| {
                        let grow = SILK_TO_PAD_MM + m.stroke_mm / 2.0;
                        k.copper.is_some_and(|c| {
                            segment_aabb_t(edge[0], edge[1], c.expand(grow)).is_some()
                        }) || k.drill.is_some_and(|(x, y, r)| {
                            segment_circle_t(edge[0], edge[1], x, y, r + grow).is_some()
                        })
                    }),
                    "clipped stroke still hits a pad/hole"
                );
            }
        }
    }

    fn guessed_u1_box() -> ShapeSpec {
        ShapeSpec {
            kind: "rect".into(),
            layer: Some("F.Silkscreen".into()),
            origin_x_mm: Some(127.2),
            origin_y_mm: Some(32.9),
            width_mm: Some(4.5),
            height_mm: Some(4.5),
            ..Default::default()
        }
    }

    #[test]
    fn guessed_4mm5_silk_around_u1_is_gapped_not_refused() {
        let made = clip_spec(&guessed_u1_box(), &u1_pads()).unwrap();
        assert!(made.iter().any(|m| m.kind == "segment"), "expected gaps");
        assert!(
            made.iter().all(|m| m.kind != "rect"),
            "{:?}",
            made.iter().map(|m| m.kind).collect::<Vec<_>>()
        );
        assert_silk_clear(&made, &u1_pads());
    }

    #[test]
    fn comments_layer_skips_dfm() {
        let mut spec = guessed_u1_box();
        spec.layer = Some("Cmts.User".into());
        let made = clip_spec(&spec, &u1_pads()).unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].kind, "rect");
    }

    #[test]
    fn back_silk_does_not_see_front_smd() {
        let mut spec = guessed_u1_box();
        spec.layer = Some("B.Silkscreen".into());
        let made = clip_spec(&spec, &u1_pads()).unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].kind, "rect");
    }

    #[test]
    fn reference_outline_is_package_body_gapped_at_pads() {
        let mut pads = u1_pads();
        pads.push(neighbor());
        let spec = ShapeSpec {
            kind: "rect".into(),
            layer: Some("F.Silkscreen".into()),
            reference: Some("U1".into()),
            tag: Some("u1".into()),
            ..Default::default()
        };
        let mut filled = spec.clone();
        apply_reference(&mut filled, &u1_pads(), Some(u1_body())).unwrap();
        let w = filled.width_mm.unwrap();
        let h = filled.height_mm.unwrap();
        assert!((w - 3.5).abs() < 0.02, "width {w}");
        assert!((h - 3.5).abs() < 0.02, "height {h}");
        let cx = filled.center_x_mm.unwrap();
        let cy = filled.center_y_mm.unwrap();
        assert!((cx - 129.45).abs() < 0.05, "cx {cx}");
        assert!((cy - 35.15).abs() < 0.05, "cy {cy}");
        let made = crate::graphics::shape_items(&filled).unwrap();
        let made = clip_plotting_silk(made, &pads).unwrap();
        assert!(made.iter().any(|m| m.kind == "segment"), "expected pad gaps");
        assert_silk_clear(&made, &pads);
        let mut ys: Vec<f64> = made
            .iter()
            .flat_map(|m| item_centerlines(m).unwrap())
            .map(|e| (e[0][1] + e[1][1]) / 2.0)
            .collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            ys.iter().any(|y| (y - 33.40).abs() < 0.05),
            "bottom of 3.5 mm body missing: {ys:?}"
        );
        assert!(
            ys.iter().any(|y| (y - 36.90).abs() < 0.05),
            "top of 3.5 mm body missing: {ys:?}"
        );
    }

    #[test]
    fn reference_refuses_explicit_size() {
        let spec = ShapeSpec {
            kind: "rect".into(),
            layer: Some("F.Silkscreen".into()),
            reference: Some("U1".into()),
            width_mm: Some(4.5),
            height_mm: Some(4.5),
            origin_x_mm: Some(127.2),
            origin_y_mm: Some(32.9),
            ..Default::default()
        };
        let err = clip_spec(&spec, &u1_pads()).unwrap_err();
        assert!(err.contains("omit origin"), "{err}");
    }

    #[test]
    fn hole_gaps_both_silk_sides() {
        let hole = PadRow {
            reference: "H1".into(),
            pin: "1".into(),
            net: String::new(),
            x_mm: 10.0,
            y_mm: 10.0,
            width_mm: 3.2,
            height_mm: 3.2,
            rotation_deg: 0.0,
            kind: "npth".into(),
            shape: "circle".into(),
            layer: "?".into(),
            layers: vec![],
            drill_mm: Some(3.2),
            drill_h_mm: None,
        };
        let spec = ShapeSpec {
            kind: "rect".into(),
            layer: Some("B.Silkscreen".into()),
            origin_x_mm: Some(9.0),
            origin_y_mm: Some(8.0),
            width_mm: Some(2.0),
            height_mm: Some(4.0),
            ..Default::default()
        };
        let made = clip_spec(&spec, &[hole.clone()]).unwrap();
        assert!(made.iter().any(|m| m.kind == "segment"));
        assert_silk_clear(&made, &[hole]);
    }

    #[test]
    fn silk_entirely_on_a_pad_is_refused() {
        let spec = ShapeSpec {
            kind: "rect".into(),
            layer: Some("F.Silkscreen".into()),
            center_x_mm: Some(131.0875),
            center_y_mm: Some(36.04),
            width_mm: Some(0.6),
            height_mm: Some(0.6),
            ..Default::default()
        };
        let err = clip_spec(&spec, &u1_pads()).unwrap_err();
        assert!(err.contains("nothing left"), "{err}");
    }

    #[test]
    fn back_table_ignores_front_led_pads() {
        let spec = ShapeSpec {
            kind: "table".into(),
            layer: Some("B.Silkscreen".into()),
            origin_x_mm: Some(20.0),
            origin_y_mm: Some(20.0),
            rows: Some(2),
            cols: Some(2),
            cell_width_mm: Some(8.0),
            cell_height_mm: Some(6.0),
            tag: Some("led-spec".into()),
            cells: vec![
                vec!["WS2812B".into(), "R".into()],
                vec!["5V".into(), "12mA".into()],
            ],
            size_mm: Some(1.0),
            ..Default::default()
        };
        let made = clip_spec(&spec, &u1_pads()).unwrap();
        assert!(made.iter().any(|m| m.kind == "text"));
        assert!(made.iter().any(|m| m.kind == "rect"));
    }

    #[test]
    fn silk_text_on_own_pad_is_omitted() {
        let spec = ShapeSpec {
            kind: "table".into(),
            layer: Some("F.Silkscreen".into()),
            origin_x_mm: Some(126.0),
            origin_y_mm: Some(33.0),
            rows: Some(1),
            cols: Some(1),
            cell_width_mm: Some(8.0),
            cell_height_mm: Some(5.0),
            tag: Some("onpad".into()),
            cells: vec![vec!["U1".into()]],
            size_mm: Some(1.0),
            ..Default::default()
        };
        let made = clip_spec(&spec, &u1_pads()).unwrap();
        assert!(
            made.iter().all(|m| m.kind != "text"),
            "cell on a pad should be omitted"
        );
        assert!(!made.is_empty());
        assert_silk_clear(&made, &u1_pads());
    }
}
