//! Unfilled board graphics on silk or user layers — cover / mask overlays.
//!
//! These are **outlines**, never copper and never Edge.Cuts. A filled silk
//! blob would plot to JLCPCB; a rectangle on Edge.Cuts would become a
//! cutout and punch the pours. `clear_shapes` / `clear_board` delete only
//! managed layers (F/B.Silkscreen, Dwgs/Cmts/Eco1/Eco2.User).

use prost::Message;
use prost_types::Any;

use crate::place::mm_to_nm;

const BL_B_SILKS: i32 = 39;
const BL_F_SILKS: i32 = 40;
const BL_DWGS_USER: i32 = 43;
const BL_CMTS_USER: i32 = 44;
const BL_ECO1_USER: i32 = 45;
const BL_ECO2_USER: i32 = 46;
const LS_UNLOCKED: i32 = 1;
const SLS_SOLID: i32 = 2;
const GFT_UNFILLED: i32 = 1;

const TYPE_BOARD_GRAPHIC: &str = "type.googleapis.com/kiapi.board.types.BoardGraphicShape";

/// Max shapes in one `add_shapes` undo.
pub const SHAPE_MAX: usize = 150;
const POLY_MAX: usize = 400;
const DEFAULT_STROKE_MM: f64 = 0.15;
const MIN_SILK_STROKE_MM: f64 = 0.15;
const MIN_USER_STROKE_MM: f64 = 0.05;
const MAX_STROKE_MM: f64 = 2.0;
const MIN_SIZE_MM: f64 = 0.5;
const MAX_SIZE_MM: f64 = 400.0;
const MIN_RADIUS_MM: f64 = 0.25;
const MAX_RADIUS_MM: f64 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicLayer {
    pub id: i32,
    pub name: &'static str,
    /// `true` = F/B.Silkscreen (plotted in gerbers). User layers do not plot.
    pub plots: bool,
}

pub fn parse_graphic_layer(name: Option<&str>) -> Result<GraphicLayer, String> {
    let raw = name.unwrap_or("F.Silkscreen").trim();
    let n = raw.replace('_', ".").to_ascii_lowercase();
    let n = n.strip_prefix("bl.").unwrap_or(&n);
    match n {
        "f.silkscreen" | "f.silks" | "f.silk" => Ok(GraphicLayer {
            id: BL_F_SILKS,
            name: "F.Silkscreen",
            plots: true,
        }),
        "b.silkscreen" | "b.silks" | "b.silk" => Ok(GraphicLayer {
            id: BL_B_SILKS,
            name: "B.Silkscreen",
            plots: true,
        }),
        "cmts.user" | "comments" | "user.comments" | "comment" => Ok(GraphicLayer {
            id: BL_CMTS_USER,
            name: "Cmts.User",
            plots: false,
        }),
        "dwgs.user" | "drawings" | "user.drawings" | "drawing" => Ok(GraphicLayer {
            id: BL_DWGS_USER,
            name: "Dwgs.User",
            plots: false,
        }),
        "eco1.user" | "eco1" => Ok(GraphicLayer {
            id: BL_ECO1_USER,
            name: "Eco1.User",
            plots: false,
        }),
        "eco2.user" | "eco2" => Ok(GraphicLayer {
            id: BL_ECO2_USER,
            name: "Eco2.User",
            plots: false,
        }),
        // Edge.Cuts before copper: "edge.cuts" contains the substring ".cu".
        other if looks_like_edge_cuts(other) => Err(format!(
            "add_shape must not draw Edge.Cuts — that is the board outline / a cutout (got {raw})"
        )),
        other if looks_like_copper(other) => Err(format!(
            "add_shape is outline-only — use F.Silkscreen or Cmts.User, not copper (got {raw})"
        )),
        other if other.contains("crtyd") || other.contains("courtyard") => Err(format!(
            "add_shape must not draw courtyards (got {raw})"
        )),
        other if other.contains("mask") || other.contains("paste") || other.contains("fab") => {
            Err(format!(
                "add_shape layer must be F/B.Silkscreen or Cmts/Dwgs/Eco.User (got {raw})"
            ))
        }
        _ => Err(format!(
            "add_shape layer must be F.Silkscreen, B.Silkscreen, Cmts.User, Dwgs.User, Eco1.User or Eco2.User (got {raw})"
        )),
    }
}

/// `edge.cuts` contains the letters `.cu`; never treat that as copper.
fn looks_like_edge_cuts(n: &str) -> bool {
    n == "edge.cuts"
        || n == "edgecuts"
        || (n.contains("edge") && n.contains("cut"))
}

fn looks_like_copper(n: &str) -> bool {
    n == "cu" || n == "fcu" || n == "bcu" || n.ends_with(".cu")
}

pub fn graphic_layer_from_id(id: i32) -> Option<GraphicLayer> {
    match id {
        BL_F_SILKS => Some(GraphicLayer {
            id: BL_F_SILKS,
            name: "F.Silkscreen",
            plots: true,
        }),
        BL_B_SILKS => Some(GraphicLayer {
            id: BL_B_SILKS,
            name: "B.Silkscreen",
            plots: true,
        }),
        BL_CMTS_USER => Some(GraphicLayer {
            id: BL_CMTS_USER,
            name: "Cmts.User",
            plots: false,
        }),
        BL_DWGS_USER => Some(GraphicLayer {
            id: BL_DWGS_USER,
            name: "Dwgs.User",
            plots: false,
        }),
        BL_ECO1_USER => Some(GraphicLayer {
            id: BL_ECO1_USER,
            name: "Eco1.User",
            plots: false,
        }),
        BL_ECO2_USER => Some(GraphicLayer {
            id: BL_ECO2_USER,
            name: "Eco2.User",
            plots: false,
        }),
        _ => None,
    }
}

/// Warning for `export_manufacturing` when overlay outlines sit on silk.
pub fn silk_export_warning(plotting_count: usize) -> Option<String> {
    if plotting_count == 0 {
        None
    } else {
        Some(format!(
            "{plotting_count} silk overlay outline(s) will plot to gerbers. clear_shapes if that was only a cover check. Cmts.User does not plot."
        ))
    }
}

/// Layers `clear_shapes` / `clear_board` may delete. Never Edge.Cuts.
pub fn is_managed_graphic_layer(id: i32, name: &str) -> bool {
    if kicad_layer::is_edge_cuts(id, name) {
        return false;
    }
    matches!(
        id,
        BL_F_SILKS | BL_B_SILKS | BL_DWGS_USER | BL_CMTS_USER | BL_ECO1_USER | BL_ECO2_USER
    ) || parse_graphic_layer(Some(name)).is_ok()
}

#[derive(Clone, Debug, Default)]
pub struct ShapeSpec {
    pub kind: String,
    pub layer: Option<String>,
    pub stroke_mm: Option<f64>,
    pub origin_x_mm: Option<f64>,
    pub origin_y_mm: Option<f64>,
    pub center_x_mm: Option<f64>,
    pub center_y_mm: Option<f64>,
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub radius_mm: Option<f64>,
    pub diameter_mm: Option<f64>,
    pub points: Vec<(f64, f64)>,
}

#[derive(Clone, Debug)]
pub struct ShapeMade {
    pub kind: &'static str,
    pub layer: GraphicLayer,
    pub stroke_mm: f64,
    pub item: Any,
}

pub fn shape_any(spec: &ShapeSpec) -> Result<ShapeMade, String> {
    let layer = parse_graphic_layer(spec.layer.as_deref())?;
    let stroke = resolve_stroke(spec.stroke_mm, layer.plots)?;
    let kind = normalize_kind(&spec.kind)?;
    let geometry = match kind {
        "rect" => rect_geometry(spec)?,
        "circle" => circle_geometry(spec)?,
        "polygon" => polygon_geometry(spec)?,
        _ => unreachable!(),
    };
    Ok(ShapeMade {
        kind,
        layer,
        stroke_mm: stroke,
        item: graphic_item(layer.id, stroke, geometry),
    })
}

fn normalize_kind(kind: &str) -> Result<&'static str, String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "rect" | "rectangle" | "box" => Ok("rect"),
        "circle" | "disc" => Ok("circle"),
        "polygon" | "poly" => Ok("polygon"),
        "" => Err(
            "kind is required: rect (origin or centre + width/height), circle (x/y + radius_mm), polygon (points)"
                .into(),
        ),
        other => Err(format!(
            "kind must be rect, circle or polygon (got {other})"
        )),
    }
}

fn resolve_stroke(stroke_mm: Option<f64>, plots: bool) -> Result<f64, String> {
    let stroke = stroke_mm.unwrap_or(DEFAULT_STROKE_MM);
    if !stroke.is_finite() {
        return Err("stroke_mm must be finite millimetres".into());
    }
    let min = if plots {
        MIN_SILK_STROKE_MM
    } else {
        MIN_USER_STROKE_MM
    };
    if stroke < min || stroke > MAX_STROKE_MM {
        return Err(format!(
            "stroke_mm must be {min}–{MAX_STROKE_MM} mm (JLCPCB silk floor {MIN_SILK_STROKE_MM} mm, got {stroke})"
        ));
    }
    Ok(stroke)
}

fn require_finite(name: &str, v: f64) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{name} must be finite millimetres"))
    }
}

fn rect_geometry(spec: &ShapeSpec) -> Result<graphic_shape::Geometry, String> {
    let w = spec
        .width_mm
        .ok_or_else(|| "rect needs width_mm and height_mm".to_string())?;
    let h = spec
        .height_mm
        .ok_or_else(|| "rect needs width_mm and height_mm".to_string())?;
    let w = require_finite("width_mm", w)?;
    let h = require_finite("height_mm", h)?;
    if w < MIN_SIZE_MM || h < MIN_SIZE_MM {
        return Err(format!(
            "rect must be at least {MIN_SIZE_MM} × {MIN_SIZE_MM} mm"
        ));
    }
    if w > MAX_SIZE_MM || h > MAX_SIZE_MM {
        return Err(format!("rect max {MAX_SIZE_MM} × {MAX_SIZE_MM} mm"));
    }
    let origin_pair = match (spec.origin_x_mm, spec.origin_y_mm) {
        (Some(x), Some(y)) => Some((require_finite("origin_x_mm", x)?, require_finite("origin_y_mm", y)?)),
        (None, None) => None,
        _ => {
            return Err("rect origin needs both origin_x_mm and origin_y_mm (bottom-left)".into())
        }
    };
    let center_pair = match (spec.center_x_mm, spec.center_y_mm) {
        (Some(x), Some(y)) => Some((require_finite("center_x_mm", x)?, require_finite("center_y_mm", y)?)),
        (None, None) => None,
        _ => return Err("rect centre needs both center_x_mm and center_y_mm".into()),
    };
    let (x0, y0) = match (origin_pair, center_pair) {
        (Some(_), Some(_)) => {
            return Err("rect takes origin_x_mm/origin_y_mm or center_x_mm/center_y_mm, not both".into())
        }
        (Some(o), None) => o,
        (None, Some((cx, cy))) => (cx - w / 2.0, cy - h / 2.0),
        (None, None) => {
            return Err(
                "rect needs origin_x_mm/origin_y_mm (bottom-left) or center_x_mm/center_y_mm"
                    .into(),
            )
        }
    };
    // KiCad +y up: top-left is min x, max y.
    let x1 = x0 + w;
    let y1 = y0 + h;
    Ok(graphic_shape::Geometry::Rectangle(
        GraphicRectangleAttributes {
            top_left: Some(Vector2 {
                x_nm: mm_to_nm(x0),
                y_nm: mm_to_nm(y1),
            }),
            bottom_right: Some(Vector2 {
                x_nm: mm_to_nm(x1),
                y_nm: mm_to_nm(y0),
            }),
            corner_radius: None,
        },
    ))
}

fn circle_geometry(spec: &ShapeSpec) -> Result<graphic_shape::Geometry, String> {
    let x = spec
        .x_mm
        .or(spec.center_x_mm)
        .ok_or_else(|| "circle needs x_mm/y_mm (centre)".to_string())?;
    let y = spec
        .y_mm
        .or(spec.center_y_mm)
        .ok_or_else(|| "circle needs x_mm/y_mm (centre)".to_string())?;
    let x = require_finite("x_mm", x)?;
    let y = require_finite("y_mm", y)?;
    let radius = match (spec.radius_mm, spec.diameter_mm) {
        (Some(_), Some(_)) => {
            return Err("circle takes radius_mm or diameter_mm, not both".into())
        }
        (Some(r), None) => require_finite("radius_mm", r)?,
        (None, Some(d)) => require_finite("diameter_mm", d)? / 2.0,
        (None, None) => return Err("circle needs radius_mm or diameter_mm".into()),
    };
    if radius < MIN_RADIUS_MM || radius > MAX_RADIUS_MM {
        return Err(format!(
            "circle radius must be {MIN_RADIUS_MM}–{MAX_RADIUS_MM} mm (got {radius})"
        ));
    }
    Ok(graphic_shape::Geometry::Circle(GraphicCircleAttributes {
        center: Some(Vector2 {
            x_nm: mm_to_nm(x),
            y_nm: mm_to_nm(y),
        }),
        radius_point: Some(Vector2 {
            x_nm: mm_to_nm(x + radius),
            y_nm: mm_to_nm(y),
        }),
    }))
}

fn polygon_geometry(spec: &ShapeSpec) -> Result<graphic_shape::Geometry, String> {
    if spec.points.len() < 3 {
        return Err("polygon needs at least 3 points".into());
    }
    if spec.points.len() > POLY_MAX {
        return Err(format!(
            "polygon max {POLY_MAX} points (got {})",
            spec.points.len()
        ));
    }
    for (i, (x, y)) in spec.points.iter().enumerate() {
        require_finite(&format!("points[{i}].x_mm"), *x)?;
        require_finite(&format!("points[{i}].y_mm"), *y)?;
    }
    let min_x = spec.points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = spec
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = spec.points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = spec
        .points
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max);
    if (max_x - min_x) > MAX_SIZE_MM || (max_y - min_y) > MAX_SIZE_MM {
        return Err(format!("polygon bbox max {MAX_SIZE_MM} × {MAX_SIZE_MM} mm"));
    }
    let mut nodes: Vec<PolyLineNode> = spec
        .points
        .iter()
        .map(|(x, y)| PolyLineNode {
            geometry: Some(poly_line_node::Geometry::Point(Vector2 {
                x_nm: mm_to_nm(*x),
                y_nm: mm_to_nm(*y),
            })),
        })
        .collect();
    if spec.points.first() != spec.points.last() {
        let (x, y) = spec.points[0];
        nodes.push(PolyLineNode {
            geometry: Some(poly_line_node::Geometry::Point(Vector2 {
                x_nm: mm_to_nm(x),
                y_nm: mm_to_nm(y),
            })),
        });
    }
    Ok(graphic_shape::Geometry::Polygon(PolySet {
        polygons: vec![PolygonWithHoles {
            outline: Some(PolyLine {
                nodes,
                closed: true,
            }),
            holes: vec![],
        }],
    }))
}

fn nm_to_mm(nm: i64) -> f64 {
    nm as f64 / 1_000_000.0
}

/// Overlay outline as KiCad stored it (vertex count from the PolyLine, not PolySet length).
#[derive(Clone, Debug)]
pub struct GraphicSnap {
    pub id: Option<String>,
    pub kind: &'static str,
    pub layer: GraphicLayer,
    pub stroke_mm: Option<f64>,
    pub origin_x_mm: Option<f64>,
    pub origin_y_mm: Option<f64>,
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub radius_mm: Option<f64>,
    pub a_mm: Option<[f64; 2]>,
    pub b_mm: Option<[f64; 2]>,
    /// Outline vertices of the first polygon (closing duplicate omitted).
    pub polygon_points: Option<usize>,
}

pub fn shape_snap_from_any(any: &Any) -> Option<GraphicSnap> {
    if !any.type_url.contains("BoardGraphicShape") {
        return None;
    }
    let proto = BoardGraphicShape::decode(any.value.as_slice()).ok()?;
    let layer = graphic_layer_from_id(proto.layer)?;
    let shape = proto.shape.as_ref()?;
    let stroke_mm = shape
        .attributes
        .as_ref()
        .and_then(|a| a.stroke.as_ref())
        .and_then(|s| s.width.as_ref())
        .map(|d| nm_to_mm(d.value_nm));
    let mut snap = GraphicSnap {
        id: proto.id.map(|k| k.value).filter(|s| !s.is_empty()),
        kind: "unknown",
        layer,
        stroke_mm,
        origin_x_mm: None,
        origin_y_mm: None,
        width_mm: None,
        height_mm: None,
        x_mm: None,
        y_mm: None,
        radius_mm: None,
        a_mm: None,
        b_mm: None,
        polygon_points: None,
    };
    match shape.geometry.as_ref()? {
        graphic_shape::Geometry::Rectangle(r) => {
            snap.kind = "rect";
            if let (Some(tl), Some(br)) = (r.top_left.as_ref(), r.bottom_right.as_ref()) {
                let min_x = nm_to_mm(tl.x_nm.min(br.x_nm));
                let max_x = nm_to_mm(tl.x_nm.max(br.x_nm));
                let min_y = nm_to_mm(tl.y_nm.min(br.y_nm));
                let max_y = nm_to_mm(tl.y_nm.max(br.y_nm));
                snap.origin_x_mm = Some(min_x);
                snap.origin_y_mm = Some(min_y);
                snap.width_mm = Some(max_x - min_x);
                snap.height_mm = Some(max_y - min_y);
            }
        }
        graphic_shape::Geometry::Circle(c) => {
            snap.kind = "circle";
            if let Some(c0) = c.center.as_ref() {
                snap.x_mm = Some(nm_to_mm(c0.x_nm));
                snap.y_mm = Some(nm_to_mm(c0.y_nm));
                if let Some(rp) = c.radius_point.as_ref() {
                    let dx = nm_to_mm(rp.x_nm) - nm_to_mm(c0.x_nm);
                    let dy = nm_to_mm(rp.y_nm) - nm_to_mm(c0.y_nm);
                    snap.radius_mm = Some((dx * dx + dy * dy).sqrt());
                }
            }
        }
        graphic_shape::Geometry::Polygon(set) => {
            snap.kind = "polygon";
            snap.polygon_points = Some(polygon_vertex_count(set));
        }
        graphic_shape::Geometry::Segment(seg) => {
            snap.kind = "segment";
            snap.a_mm = seg.start.as_ref().map(|p| [nm_to_mm(p.x_nm), nm_to_mm(p.y_nm)]);
            snap.b_mm = seg.end.as_ref().map(|p| [nm_to_mm(p.x_nm), nm_to_mm(p.y_nm)]);
        }
    }
    Some(snap)
}

fn polygon_vertex_count(set: &PolySet) -> usize {
    let Some(outline) = set.polygons.first().and_then(|p| p.outline.as_ref()) else {
        return 0;
    };
    let points: Vec<(i64, i64)> = outline
        .nodes
        .iter()
        .filter_map(|n| match &n.geometry {
            Some(poly_line_node::Geometry::Point(p)) => Some((p.x_nm, p.y_nm)),
            _ => None,
        })
        .collect();
    if points.len() >= 2 && points.first() == points.last() {
        points.len() - 1
    } else {
        points.len()
    }
}

fn graphic_item(layer: i32, stroke_mm: f64, geometry: graphic_shape::Geometry) -> Any {
    let item = BoardGraphicShape {
        shape: Some(GraphicShape {
            attributes: Some(GraphicAttributes {
                stroke: Some(StrokeAttributes {
                    width: Some(Distance {
                        value_nm: mm_to_nm(stroke_mm),
                    }),
                    style: SLS_SOLID,
                }),
                fill: Some(GraphicFillAttributes {
                    fill_type: GFT_UNFILLED,
                }),
            }),
            geometry: Some(geometry),
        }),
        layer,
        id: None,
        locked: LS_UNLOCKED,
    };
    Any {
        type_url: TYPE_BOARD_GRAPHIC.into(),
        value: item.encode_to_vec(),
    }
}

#[derive(Clone, PartialEq, Message)]
struct Vector2 {
    #[prost(int64, tag = "1")]
    x_nm: i64,
    #[prost(int64, tag = "2")]
    y_nm: i64,
}

#[derive(Clone, PartialEq, Message)]
struct Distance {
    #[prost(int64, tag = "1")]
    value_nm: i64,
}

#[derive(Clone, PartialEq, Message)]
struct StrokeAttributes {
    #[prost(message, optional, tag = "1")]
    width: Option<Distance>,
    #[prost(int32, tag = "2")]
    style: i32,
}

#[derive(Clone, PartialEq, Message)]
struct GraphicFillAttributes {
    #[prost(int32, tag = "1")]
    fill_type: i32,
}

#[derive(Clone, PartialEq, Message)]
struct GraphicAttributes {
    #[prost(message, optional, tag = "1")]
    stroke: Option<StrokeAttributes>,
    #[prost(message, optional, tag = "2")]
    fill: Option<GraphicFillAttributes>,
}

#[derive(Clone, PartialEq, Message)]
struct GraphicRectangleAttributes {
    #[prost(message, optional, tag = "1")]
    top_left: Option<Vector2>,
    #[prost(message, optional, tag = "2")]
    bottom_right: Option<Vector2>,
    #[prost(message, optional, tag = "3")]
    corner_radius: Option<Distance>,
}

#[derive(Clone, PartialEq, Message)]
struct GraphicCircleAttributes {
    #[prost(message, optional, tag = "1")]
    center: Option<Vector2>,
    #[prost(message, optional, tag = "2")]
    radius_point: Option<Vector2>,
}

#[derive(Clone, PartialEq, Message)]
struct PolyLineNode {
    #[prost(oneof = "poly_line_node::Geometry", tags = "1")]
    geometry: Option<poly_line_node::Geometry>,
}

mod poly_line_node {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Geometry {
        #[prost(message, tag = "1")]
        Point(super::Vector2),
    }
}

#[derive(Clone, PartialEq, Message)]
struct PolyLine {
    #[prost(message, repeated, tag = "1")]
    nodes: Vec<PolyLineNode>,
    #[prost(bool, tag = "2")]
    closed: bool,
}

#[derive(Clone, PartialEq, Message)]
struct PolygonWithHoles {
    #[prost(message, optional, tag = "1")]
    outline: Option<PolyLine>,
    #[prost(message, repeated, tag = "2")]
    holes: Vec<PolyLine>,
}

#[derive(Clone, PartialEq, Message)]
struct PolySet {
    #[prost(message, repeated, tag = "1")]
    polygons: Vec<PolygonWithHoles>,
}

#[derive(Clone, PartialEq, Message)]
struct GraphicShape {
    #[prost(message, optional, tag = "3")]
    attributes: Option<GraphicAttributes>,
    #[prost(oneof = "graphic_shape::Geometry", tags = "4, 5, 7, 8")]
    geometry: Option<graphic_shape::Geometry>,
}

mod graphic_shape {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Geometry {
        #[prost(message, tag = "4")]
        Segment(super::GraphicSegmentAttributes),
        #[prost(message, tag = "5")]
        Rectangle(super::GraphicRectangleAttributes),
        #[prost(message, tag = "7")]
        Circle(super::GraphicCircleAttributes),
        #[prost(message, tag = "8")]
        Polygon(super::PolySet),
    }
}

#[derive(Clone, PartialEq, Message)]
struct GraphicSegmentAttributes {
    #[prost(message, optional, tag = "1")]
    start: Option<Vector2>,
    #[prost(message, optional, tag = "2")]
    end: Option<Vector2>,
}

#[derive(Clone, PartialEq, Message)]
struct Kiid {
    #[prost(string, tag = "1")]
    value: String,
}

#[derive(Clone, PartialEq, Message)]
struct BoardGraphicShape {
    #[prost(message, optional, tag = "1")]
    shape: Option<GraphicShape>,
    #[prost(int32, tag = "2")]
    layer: i32,
    #[prost(message, optional, tag = "4")]
    id: Option<Kiid>,
    #[prost(int32, tag = "5")]
    locked: i32,
}

/// Tiny helper so `graphics` can refuse Edge.Cuts without importing `kicad.rs`
/// (that module talks to IPC).
pub(crate) mod kicad_layer {
    use crate::outline::BL_EDGE_CUTS;

    pub fn is_edge_cuts(id: i32, name: &str) -> bool {
        id == BL_EDGE_CUTS
            || name.eq_ignore_ascii_case("Edge.Cuts")
            || name.eq_ignore_ascii_case("BL_Edge_Cuts")
            || name.eq_ignore_ascii_case("Edge_Cuts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_origin() -> ShapeSpec {
        ShapeSpec {
            kind: "rect".into(),
            origin_x_mm: Some(10.0),
            origin_y_mm: Some(20.0),
            width_mm: Some(30.0),
            height_mm: Some(12.0),
            ..Default::default()
        }
    }

    #[test]
    fn default_layer_is_front_silk_and_plots() {
        let layer = parse_graphic_layer(None).unwrap();
        assert_eq!(layer.id, BL_F_SILKS);
        assert!(layer.plots);
    }

    #[test]
    fn comments_layer_does_not_plot() {
        let layer = parse_graphic_layer(Some("Cmts.User")).unwrap();
        assert_eq!(layer.id, BL_CMTS_USER);
        assert!(!layer.plots);
    }

    #[test]
    fn refuses_copper_and_edge_cuts() {
        let copper = parse_graphic_layer(Some("F.Cu")).unwrap_err();
        assert!(copper.contains("copper"), "{copper}");
        assert!(!copper.contains("cutout"), "{copper}");

        let edge = parse_graphic_layer(Some("Edge.Cuts")).unwrap_err();
        assert!(edge.contains("must not draw Edge.Cuts"), "{edge}");
        assert!(edge.contains("cutout"), "{edge}");
        assert!(!edge.contains("copper"), "{edge}");

        let edge_alias = parse_graphic_layer(Some("BL_Edge_Cuts")).unwrap_err();
        assert!(edge_alias.contains("cutout"), "{edge_alias}");
        assert!(!edge_alias.contains("copper"), "{edge_alias}");

        let inner = parse_graphic_layer(Some("In1.Cu")).unwrap_err();
        assert!(inner.contains("copper"), "{inner}");

        assert!(parse_graphic_layer(Some("F.CrtYd"))
            .unwrap_err()
            .contains("courtyard"));
    }

    #[test]
    fn polygon_get_shapes_reports_vertices_not_polyset_count() {
        let spec = ShapeSpec {
            kind: "polygon".into(),
            points: vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)],
            layer: Some("Cmts.User".into()),
            ..Default::default()
        };
        let made = shape_any(&spec).unwrap();
        let snap = shape_snap_from_any(&made.item).expect("decode overlay");
        assert_eq!(snap.kind, "polygon");
        assert_eq!(snap.polygon_points, Some(3));
        assert!(!snap.layer.plots);
    }

    #[test]
    fn silk_export_warns_only_when_silk_plots() {
        assert!(silk_export_warning(0).is_none());
        let msg = silk_export_warning(2).unwrap();
        assert!(msg.contains("2 silk overlay"));
        assert!(msg.contains("clear_shapes"));
    }

    #[test]
    fn rect_uses_native_rectangle_y_up() {
        let made = shape_any(&rect_origin()).unwrap();
        assert_eq!(made.kind, "rect");
        assert!(made.item.type_url.contains("BoardGraphicShape"));
        let proto = BoardGraphicShape::decode(made.item.value.as_slice()).unwrap();
        assert_eq!(proto.layer, BL_F_SILKS);
        let fill = proto
            .shape
            .as_ref()
            .unwrap()
            .attributes
            .as_ref()
            .unwrap()
            .fill
            .as_ref()
            .unwrap()
            .fill_type;
        assert_eq!(fill, GFT_UNFILLED);
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Rectangle(r) => {
                let tl = r.top_left.unwrap();
                let br = r.bottom_right.unwrap();
                assert_eq!(tl.x_nm, 10_000_000);
                assert_eq!(tl.y_nm, 32_000_000); // 20 + 12
                assert_eq!(br.x_nm, 40_000_000);
                assert_eq!(br.y_nm, 20_000_000);
            }
            other => panic!("expected rectangle, got {other:?}"),
        }
    }

    #[test]
    fn rect_from_centre() {
        let spec = ShapeSpec {
            kind: "rect".into(),
            center_x_mm: Some(148.5),
            center_y_mm: Some(105.0),
            width_mm: Some(40.0),
            height_mm: Some(20.0),
            layer: Some("Cmts.User".into()),
            ..Default::default()
        };
        let made = shape_any(&spec).unwrap();
        assert!(!made.layer.plots);
        let proto = BoardGraphicShape::decode(made.item.value.as_slice()).unwrap();
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Rectangle(r) => {
                let tl = r.top_left.unwrap();
                let br = r.bottom_right.unwrap();
                assert_eq!(tl.x_nm, mm_to_nm(128.5));
                assert_eq!(tl.y_nm, mm_to_nm(115.0));
                assert_eq!(br.x_nm, mm_to_nm(168.5));
                assert_eq!(br.y_nm, mm_to_nm(95.0));
            }
            other => panic!("expected rectangle, got {other:?}"),
        }
    }

    #[test]
    fn circle_from_diameter() {
        let spec = ShapeSpec {
            kind: "circle".into(),
            x_mm: Some(148.5),
            y_mm: Some(105.0),
            diameter_mm: Some(157.0),
            ..Default::default()
        };
        let made = shape_any(&spec).unwrap();
        let proto = BoardGraphicShape::decode(made.item.value.as_slice()).unwrap();
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Circle(c) => {
                let c0 = c.center.unwrap();
                let rp = c.radius_point.unwrap();
                assert_eq!(c0.x_nm, mm_to_nm(148.5));
                assert_eq!(c0.y_nm, mm_to_nm(105.0));
                assert_eq!(rp.x_nm, mm_to_nm(148.5 + 78.5));
                assert_eq!(rp.y_nm, mm_to_nm(105.0));
            }
            other => panic!("expected circle, got {other:?}"),
        }
    }

    #[test]
    fn polygon_closes() {
        let spec = ShapeSpec {
            kind: "polygon".into(),
            points: vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)],
            ..Default::default()
        };
        let made = shape_any(&spec).unwrap();
        let proto = BoardGraphicShape::decode(made.item.value.as_slice()).unwrap();
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Polygon(set) => {
                let nodes = set.polygons[0].outline.as_ref().unwrap();
                assert!(nodes.closed);
                assert_eq!(nodes.nodes.len(), 4);
            }
            other => panic!("expected polygon, got {other:?}"),
        }
    }

    #[test]
    fn silk_stroke_floor() {
        let mut spec = rect_origin();
        spec.stroke_mm = Some(0.08);
        assert!(shape_any(&spec).unwrap_err().contains("0.15"));
    }

    #[test]
    fn user_layer_allows_thinner_stroke() {
        let spec = ShapeSpec {
            kind: "circle".into(),
            x_mm: Some(0.0),
            y_mm: Some(0.0),
            radius_mm: Some(10.0),
            layer: Some("Cmts.User".into()),
            stroke_mm: Some(0.05),
            ..Default::default()
        };
        assert!(shape_any(&spec).is_ok());
    }

    #[test]
    fn managed_layers_exclude_edge_cuts() {
        assert!(is_managed_graphic_layer(BL_F_SILKS, "BL_F_SilkS"));
        assert!(is_managed_graphic_layer(BL_CMTS_USER, "Cmts.User"));
        assert!(!is_managed_graphic_layer(47, "BL_Edge_Cuts"));
        assert!(!is_managed_graphic_layer(3, "F.Cu"));
    }
}
