//! Unfilled board graphics on silk or user layers — cover / mask overlays.
//!
//! These are **outlines**, never copper and never Edge.Cuts. A filled silk
//! blob would plot to JLCPCB; a rectangle on Edge.Cuts would become a
//! cutout and punch the pours. `clear_shapes` / `clear_board` delete only
//! managed layers (F/B.Silkscreen, Dwgs/Cmts/Eco1/Eco2.User).
//! Default layer is Cmts.User (does not plot). Optional `tag` is stored as a
//! KiCad group named `kicad-mcp:<tag>` so one overlay can be cleared without
//! wiping a silk logo. `kind: table` is a grid of lines, one undo.
//! `kind: line` is an open segment. Table `cells` are BoardText on the
//! same layer (need a tag so `clear_shapes` deletes grid + text). On
//! B.Silkscreen, `cells` is still top-row-first as you read the finished
//! back — the tool maps that onto the flipped board coordinates.
//! Plotting silk is punched at same-side pads, holes, and via drills (`silk_dfm`).
//! `kind: rect` + `reference` draws the package body, then clips.

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
const TYPE_GROUP: &str = "type.googleapis.com/kiapi.board.types.Group";

/// Max shapes in one `add_shapes` undo.
pub const SHAPE_MAX: usize = 150;
const POLY_MAX: usize = 400;
const TABLE_MAX: u32 = 40;
const TAG_MAX: usize = 40;
/// KiCad group name prefix so `clear_shapes tag=` never deletes a user group.
pub const TAG_PREFIX: &str = "kicad-mcp:";
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
    let raw = name.unwrap_or("Cmts.User").trim();
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
    n == "edge.cuts" || n == "edgecuts" || (n.contains("edge") && n.contains("cut"))
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

/// Sort key so `get_shapes` lists overlay text in reading order on that layer.
/// Front / user: top-to-bottom (+y first), then left-to-right.
/// B.Silkscreen: as seen on the finished back (low Y first, then high X).
pub fn overlay_text_read_key(layer: &str, x_mm: f64, y_mm: f64) -> (i64, i64) {
    let x = (x_mm * 1_000_000.0).round() as i64;
    let y = (y_mm * 1_000_000.0).round() as i64;
    if layer.eq_ignore_ascii_case("B.Silkscreen") {
        (y, -x)
    } else {
        (-y, x)
    }
}

/// Overlay tag stored on the board as a KiCad group `kicad-mcp:<tag>`.
/// Empty / whitespace is untagged. Letters, digits, `.`, `_`, `-`; max 40.
pub fn parse_overlay_tag(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(s) = raw else {
        return Ok(None);
    };
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let s = s.strip_prefix(TAG_PREFIX).unwrap_or(s);
    if s.is_empty() {
        return Ok(None);
    }
    if s.len() > TAG_MAX {
        return Err(format!("tag max {TAG_MAX} characters (got {})", s.len()));
    }
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return Ok(None);
    };
    if !first.is_ascii_alphanumeric() {
        return Err("tag must start with a letter or digit".into());
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("tag may contain only A–Z, a–z, 0–9, '.', '_' and '-'".into());
    }
    if s.contains("..") {
        return Err("tag must not contain '..'".into());
    }
    Ok(Some(s.to_string()))
}

pub fn group_name_for_tag(tag: &str) -> String {
    format!("{TAG_PREFIX}{tag}")
}

pub fn tag_from_group_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix(TAG_PREFIX)?;
    parse_overlay_tag(Some(rest)).ok().flatten()
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
    pub rows: Option<u32>,
    pub cols: Option<u32>,
    pub cell_width_mm: Option<f64>,
    pub cell_height_mm: Option<f64>,
    pub tag: Option<String>,
    /// Outer table border. Default = `stroke_mm` (inner grid).
    pub border_stroke_mm: Option<f64>,
    /// Line endpoints (kind line).
    pub a_x_mm: Option<f64>,
    pub a_y_mm: Option<f64>,
    pub b_x_mm: Option<f64>,
    pub b_y_mm: Option<f64>,
    /// Table cell strings, row-major, **top row first as you read the finished layer**.
    /// On B.Silkscreen that is the physical back (the tool flips the mapping).
    pub cells: Vec<Vec<String>>,
    /// Cell text height. Default 1.0 mm (silk floor 0.8; user layers 0.5).
    pub size_mm: Option<f64>,
    /// Footprint reference (`"U1"`). `kind: rect` only: size/centre come from
    /// the package body (JLCPCB L/W). Plotting silk is then gapped at pads.
    pub reference: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ShapeMade {
    pub kind: &'static str,
    pub group_kind: &'static str,
    pub layer: GraphicLayer,
    pub stroke_mm: f64,
    pub tag: Option<String>,
    pub item: Any,
}

/// One spec may expand (a table is outer rect + inner grid lines, plus cell text).
pub fn shape_items(spec: &ShapeSpec) -> Result<Vec<ShapeMade>, String> {
    let layer = parse_graphic_layer(spec.layer.as_deref())?;
    let stroke = resolve_stroke(spec.stroke_mm, layer.plots)?;
    let kind = normalize_kind(&spec.kind)?;
    let tag = parse_overlay_tag(spec.tag.as_deref())?;
    let mut items: Vec<ShapeMade> = Vec::new();
    match kind {
        "rect" => items.push(made(
            "rect",
            kind,
            layer,
            stroke,
            tag.clone(),
            graphic_item(layer.id, stroke, rect_geometry(spec)?),
        )),
        "circle" => items.push(made(
            "circle",
            kind,
            layer,
            stroke,
            tag.clone(),
            graphic_item(layer.id, stroke, circle_geometry(spec)?),
        )),
        "polygon" => items.push(made(
            "polygon",
            kind,
            layer,
            stroke,
            tag.clone(),
            graphic_item(layer.id, stroke, polygon_geometry(spec)?),
        )),
        "line" => items.push(made(
            "segment",
            kind,
            layer,
            stroke,
            tag.clone(),
            graphic_item(layer.id, stroke, line_geometry(spec, &spec.kind)?),
        )),
        "table" => {
            let layout = table_layout(spec)?;
            let border = match spec.border_stroke_mm {
                Some(_) => resolve_stroke(spec.border_stroke_mm, layer.plots)?,
                None => stroke,
            };
            items.push(made(
                "rect",
                kind,
                layer,
                border,
                tag.clone(),
                graphic_item(
                    layer.id,
                    border,
                    rect_from_origin(layout.x0, layout.y0, layout.w, layout.h),
                ),
            ));
            for geom in layout.inner_segments() {
                items.push(made(
                    "segment",
                    kind,
                    layer,
                    stroke,
                    tag.clone(),
                    graphic_item(layer.id, stroke, geom),
                ));
            }
            let texts = table_cell_texts(spec, &layout, layer, &tag)?;
            items.extend(texts);
        }
        _ => unreachable!(),
    }
    if items.len() > SHAPE_MAX {
        return Err(format!(
            "overlay max {SHAPE_MAX} items in one undo (got {})",
            items.len()
        ));
    }
    Ok(items)
}

fn made(
    kind: &'static str,
    group_kind: &'static str,
    layer: GraphicLayer,
    stroke_mm: f64,
    tag: Option<String>,
    item: Any,
) -> ShapeMade {
    ShapeMade {
        kind,
        group_kind,
        layer,
        stroke_mm,
        tag,
        item,
    }
}

/// Open stroke used after silk-to-pad clipping (a rect/circle becomes segments).
pub(crate) fn overlay_segment(
    group_kind: &'static str,
    layer: GraphicLayer,
    stroke_mm: f64,
    tag: Option<String>,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
) -> ShapeMade {
    made(
        "segment",
        group_kind,
        layer,
        stroke_mm,
        tag,
        graphic_item(layer.id, stroke_mm, segment(ax, ay, bx, by)),
    )
}

pub fn shape_any(spec: &ShapeSpec) -> Result<ShapeMade, String> {
    let mut items = shape_items(spec)?;
    if items.len() != 1 {
        return Err(format!(
            "kind {} expands to {} items — use shape_items",
            spec.kind,
            items.len()
        ));
    }
    Ok(items.remove(0))
}

fn normalize_kind(kind: &str) -> Result<&'static str, String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "rect" | "rectangle" | "box" => Ok("rect"),
        "circle" | "disc" => Ok("circle"),
        "polygon" | "poly" => Ok("polygon"),
        "table" | "grid" => Ok("table"),
        "line" | "segment" | "hline" | "vline" => Ok("line"),
        "" => Err(
            "kind is required: rect, circle, polygon, line (two points) or table (rows/cols + cell size)"
                .into(),
        ),
        other => Err(format!(
            "kind must be rect, circle, polygon, line or table (got {other})"
        )),
    }
}

pub(crate) fn resolve_stroke(stroke_mm: Option<f64>, plots: bool) -> Result<f64, String> {
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

fn origin_xy(spec: &ShapeSpec, w: f64, h: f64, kind: &str) -> Result<(f64, f64), String> {
    let origin_pair = match (spec.origin_x_mm, spec.origin_y_mm) {
        (Some(x), Some(y)) => Some((
            require_finite("origin_x_mm", x)?,
            require_finite("origin_y_mm", y)?,
        )),
        (None, None) => None,
        _ => {
            return Err(format!(
                "{kind} origin needs both origin_x_mm and origin_y_mm (bottom-left)"
            ))
        }
    };
    let center_pair = match (spec.center_x_mm, spec.center_y_mm) {
        (Some(x), Some(y)) => Some((
            require_finite("center_x_mm", x)?,
            require_finite("center_y_mm", y)?,
        )),
        (None, None) => None,
        _ => {
            return Err(format!(
                "{kind} centre needs both center_x_mm and center_y_mm"
            ))
        }
    };
    match (origin_pair, center_pair) {
        (Some(_), Some(_)) => Err(format!(
            "{kind} takes origin_x_mm/origin_y_mm or center_x_mm/center_y_mm, not both"
        )),
        (Some(o), None) => Ok(o),
        (None, Some((cx, cy))) => Ok((cx - w / 2.0, cy - h / 2.0)),
        (None, None) => Err(format!(
            "{kind} needs origin_x_mm/origin_y_mm (bottom-left) or center_x_mm/center_y_mm"
        )),
    }
}

fn rect_from_origin(x0: f64, y0: f64, w: f64, h: f64) -> graphic_shape::Geometry {
    // KiCad +y up: top-left is min x, max y.
    graphic_shape::Geometry::Rectangle(GraphicRectangleAttributes {
        top_left: Some(Vector2 {
            x_nm: mm_to_nm(x0),
            y_nm: mm_to_nm(y0 + h),
        }),
        bottom_right: Some(Vector2 {
            x_nm: mm_to_nm(x0 + w),
            y_nm: mm_to_nm(y0),
        }),
        corner_radius: None,
    })
}

fn segment(x0: f64, y0: f64, x1: f64, y1: f64) -> graphic_shape::Geometry {
    graphic_shape::Geometry::Segment(GraphicSegmentAttributes {
        start: Some(Vector2 {
            x_nm: mm_to_nm(x0),
            y_nm: mm_to_nm(y0),
        }),
        end: Some(Vector2 {
            x_nm: mm_to_nm(x1),
            y_nm: mm_to_nm(y1),
        }),
    })
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
    let (x0, y0) = origin_xy(spec, w, h, "rect")?;
    Ok(rect_from_origin(x0, y0, w, h))
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
        (Some(_), Some(_)) => return Err("circle takes radius_mm or diameter_mm, not both".into()),
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
    let min_x = spec
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min);
    let max_x = spec
        .points
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = spec
        .points
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min);
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

fn line_geometry(spec: &ShapeSpec, raw_kind: &str) -> Result<graphic_shape::Geometry, String> {
    let (ax, ay) = match (spec.a_x_mm, spec.a_y_mm) {
        (Some(x), Some(y)) => (require_finite("a_x_mm", x)?, require_finite("a_y_mm", y)?),
        _ => return Err("line needs a_x_mm/a_y_mm and b_x_mm/b_y_mm".into()),
    };
    let (bx, by) = match (spec.b_x_mm, spec.b_y_mm) {
        (Some(x), Some(y)) => (require_finite("b_x_mm", x)?, require_finite("b_y_mm", y)?),
        _ => return Err("line needs a_x_mm/a_y_mm and b_x_mm/b_y_mm".into()),
    };
    let alias = raw_kind.trim().to_ascii_lowercase();
    if alias == "hline" && (ay - by).abs() > 0.001 {
        return Err("hline must be horizontal (a_y_mm == b_y_mm)".into());
    }
    if alias == "vline" && (ax - bx).abs() > 0.001 {
        return Err("vline must be vertical (a_x_mm == b_x_mm)".into());
    }
    let len = ((bx - ax).hypot(by - ay)).abs();
    if len < MIN_SIZE_MM {
        return Err(format!("line must be at least {MIN_SIZE_MM} mm long"));
    }
    if len > MAX_SIZE_MM {
        return Err(format!("line max {MAX_SIZE_MM} mm"));
    }
    Ok(segment(ax, ay, bx, by))
}

struct TableLayout {
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
    cell_w: f64,
    cell_h: f64,
    rows: u32,
    cols: u32,
}

impl TableLayout {
    fn inner_segments(&self) -> Vec<graphic_shape::Geometry> {
        let x1 = self.x0 + self.w;
        let y1 = self.y0 + self.h;
        let mut out = Vec::new();
        for i in 1..self.rows {
            let y = self.y0 + self.cell_h * f64::from(i);
            out.push(segment(self.x0, y, x1, y));
        }
        for j in 1..self.cols {
            let x = self.x0 + self.cell_w * f64::from(j);
            out.push(segment(x, self.y0, x, y1));
        }
        out
    }

    fn cell_center(&self, row_from_top: u32, col: u32, from_back: bool) -> (f64, f64) {
        // `cells` is reading order on the finished layer. Front silk / user
        // layers: row 0 is KiCad +y (top). B.Silkscreen is read on the
        // physical back (bottom camera flips X and Y), so row 0 / col 0
        // sit at low Y, high X — do not reverse the matrix in the caller.
        let col = if from_back {
            self.cols.saturating_sub(1).saturating_sub(col)
        } else {
            col
        };
        let row_from_bottom = if from_back {
            row_from_top
        } else {
            self.rows.saturating_sub(1).saturating_sub(row_from_top)
        };
        let x = self.x0 + self.cell_w * (f64::from(col) + 0.5);
        let y = self.y0 + self.cell_h * (f64::from(row_from_bottom) + 0.5);
        (x, y)
    }
}

fn padded_cells(spec: &ShapeSpec, rows: u32, cols: u32) -> Result<Vec<Vec<String>>, String> {
    if spec.cells.is_empty() {
        return Ok(vec![vec![String::new(); cols as usize]; rows as usize]);
    }
    if spec.cells.len() > rows as usize {
        return Err(format!(
            "table cells has {} rows, but rows is {rows}",
            spec.cells.len()
        ));
    }
    for (i, row) in spec.cells.iter().enumerate() {
        if row.len() > cols as usize {
            return Err(format!(
                "table cells[{i}] has {} columns, but cols is {cols}",
                row.len()
            ));
        }
    }
    let mut out = vec![vec![String::new(); cols as usize]; rows as usize];
    for (r, row) in spec.cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            out[r][c] = cell.trim().to_string();
        }
    }
    Ok(out)
}

fn table_layout(spec: &ShapeSpec) -> Result<TableLayout, String> {
    let (rows, cols) = if spec.cells.is_empty() {
        (
            spec.rows
                .ok_or_else(|| "table needs rows and cols".to_string())?,
            spec.cols
                .ok_or_else(|| "table needs rows and cols".to_string())?,
        )
    } else {
        let inferred_rows = spec.cells.len() as u32;
        let inferred_cols = spec
            .cells
            .iter()
            .map(|r| r.len() as u32)
            .max()
            .unwrap_or(0)
            .max(1);
        (
            spec.rows.unwrap_or(inferred_rows),
            spec.cols.unwrap_or(inferred_cols),
        )
    };
    if rows == 0 || cols == 0 {
        return Err("table rows and cols must be at least 1".into());
    }
    if rows > TABLE_MAX || cols > TABLE_MAX {
        return Err(format!("table max {TABLE_MAX} × {TABLE_MAX} cells"));
    }
    let line_count = 1 + (rows as usize - 1) + (cols as usize - 1);
    if line_count > SHAPE_MAX {
        return Err(format!(
            "table {rows}×{cols} is {line_count} lines (max {SHAPE_MAX})"
        ));
    }
    let (cell_w, cell_h) = match (
        spec.cell_width_mm,
        spec.cell_height_mm,
        spec.width_mm,
        spec.height_mm,
    ) {
        (Some(_), Some(_), None, None) => (
            require_finite("cell_width_mm", spec.cell_width_mm.unwrap())?,
            require_finite("cell_height_mm", spec.cell_height_mm.unwrap())?,
        ),
        (None, None, Some(w), Some(h)) => {
            let w = require_finite("width_mm", w)?;
            let h = require_finite("height_mm", h)?;
            (w / f64::from(cols), h / f64::from(rows))
        }
        (None, None, None, None) => {
            return Err(
                "table needs cell_width_mm and cell_height_mm, or width_mm/height_mm as the overall size"
                    .into(),
            )
        }
        _ => {
            return Err(
                "table takes cell_width_mm/cell_height_mm or overall width_mm/height_mm, not both"
                    .into(),
            )
        }
    };
    if cell_w < MIN_SIZE_MM || cell_h < MIN_SIZE_MM {
        return Err(format!(
            "table cell must be at least {MIN_SIZE_MM} × {MIN_SIZE_MM} mm"
        ));
    }
    let w = cell_w * f64::from(cols);
    let h = cell_h * f64::from(rows);
    if w > MAX_SIZE_MM || h > MAX_SIZE_MM {
        return Err(format!(
            "table overall max {MAX_SIZE_MM} × {MAX_SIZE_MM} mm"
        ));
    }
    let (x0, y0) = origin_xy(spec, w, h, "table")?;
    Ok(TableLayout {
        x0,
        y0,
        w,
        h,
        cell_w,
        cell_h,
        rows,
        cols,
    })
}

fn table_cell_texts(
    spec: &ShapeSpec,
    layout: &TableLayout,
    layer: GraphicLayer,
    tag: &Option<String>,
) -> Result<Vec<ShapeMade>, String> {
    if spec
        .cells
        .iter()
        .all(|row| row.iter().all(|c| c.trim().is_empty()))
    {
        return Ok(Vec::new());
    }
    if tag.is_none() {
        return Err(
            "table cells need a tag so clear_shapes can delete the text with the grid".into(),
        );
    }
    let cells = padded_cells(spec, layout.rows, layout.cols)?;
    let min_size = if layer.plots { 0.8 } else { 0.5 };
    let size = spec.size_mm.unwrap_or(1.0);
    if !size.is_finite() {
        return Err("size_mm must be finite millimetres".into());
    }
    if size >= layout.cell_w || size >= layout.cell_h {
        return Err(format!(
            "size_mm ({size}) must be smaller than the cell ({:.3} × {:.3} mm)",
            layout.cell_w, layout.cell_h
        ));
    }
    let from_back = layer.id == BL_B_SILKS;
    let mirrored = from_back;
    let mut out = Vec::new();
    for (r, row) in cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            let (x, y) = layout.cell_center(r as u32, c as u32, from_back);
            let item = crate::silk::text_on_layer(
                cell,
                x,
                y,
                layer.id,
                mirrored,
                Some(size),
                None,
                min_size,
            )?;
            out.push(made(
                "text",
                "table",
                layer,
                size * crate::silk::STROKE_RATIO,
                tag.clone(),
                item,
            ));
        }
    }
    Ok(out)
}

pub fn graphic_id_from_any(any: &Any) -> Option<String> {
    if !any.type_url.contains("BoardGraphicShape") {
        return None;
    }
    BoardGraphicShape::decode(any.value.as_slice())
        .ok()?
        .id
        .map(|k| k.value)
        .filter(|s| !s.is_empty())
}

pub fn overlay_item_id_from_any(any: &Any) -> Option<String> {
    graphic_id_from_any(any).or_else(|| crate::silk::text_id_from_any(any))
}

pub fn group_any(name: &str, member_ids: &[String]) -> Any {
    let item = BoardGroup {
        id: None,
        name: name.to_string(),
        items: member_ids
            .iter()
            .map(|value| Kiid {
                value: value.clone(),
            })
            .collect(),
    };
    Any {
        type_url: TYPE_GROUP.into(),
        value: item.encode_to_vec(),
    }
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
            snap.a_mm = seg
                .start
                .as_ref()
                .map(|p| [nm_to_mm(p.x_nm), nm_to_mm(p.y_nm)]);
            snap.b_mm = seg
                .end
                .as_ref()
                .map(|p| [nm_to_mm(p.x_nm), nm_to_mm(p.y_nm)]);
        }
    }
    Some(snap)
}

/// Outline vertices of the first polygon (closing duplicate omitted).
pub fn polygon_vertices_mm_from_any(any: &Any) -> Option<Vec<(f64, f64)>> {
    if !any.type_url.contains("BoardGraphicShape") {
        return None;
    }
    let proto = BoardGraphicShape::decode(any.value.as_slice()).ok()?;
    let graphic_shape::Geometry::Polygon(set) = proto.shape?.geometry? else {
        return None;
    };
    let outline = set.polygons.first()?.outline.as_ref()?;
    let mut pts: Vec<(f64, f64)> = outline
        .nodes
        .iter()
        .filter_map(|n| match &n.geometry {
            Some(poly_line_node::Geometry::Point(p)) => Some((nm_to_mm(p.x_nm), nm_to_mm(p.y_nm))),
            _ => None,
        })
        .collect();
    if pts.len() >= 2 && pts.first() == pts.last() {
        pts.pop();
    }
    (pts.len() >= 3).then_some(pts)
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

#[derive(Clone, PartialEq, Message)]
struct BoardGroup {
    #[prost(message, optional, tag = "1")]
    id: Option<Kiid>,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(message, repeated, tag = "3")]
    items: Vec<Kiid>,
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
    fn default_layer_is_comments_and_does_not_plot() {
        let layer = parse_graphic_layer(None).unwrap();
        assert_eq!(layer.id, BL_CMTS_USER);
        assert!(!layer.plots);
    }

    #[test]
    fn explicit_silk_still_plots() {
        let layer = parse_graphic_layer(Some("F.Silkscreen")).unwrap();
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
    fn overlay_text_read_key_matches_finished_layer() {
        let mut front_pos = vec![
            ("Cmts.User", 15.0, 4.0),
            ("Cmts.User", 5.0, 4.0),
            ("Cmts.User", 15.0, 12.0),
            ("Cmts.User", 5.0, 12.0),
        ];
        front_pos.sort_by_key(|(l, x, y)| overlay_text_read_key(l, *x, *y));
        assert_eq!(
            front_pos,
            vec![
                ("Cmts.User", 5.0, 12.0),
                ("Cmts.User", 15.0, 12.0),
                ("Cmts.User", 5.0, 4.0),
                ("Cmts.User", 15.0, 4.0),
            ]
        );
        let mut back_pos = vec![
            ("B.Silkscreen", 5.0, 12.0),
            ("B.Silkscreen", 15.0, 12.0),
            ("B.Silkscreen", 5.0, 4.0),
            ("B.Silkscreen", 15.0, 4.0),
        ];
        back_pos.sort_by_key(|(l, x, y)| overlay_text_read_key(l, *x, *y));
        assert_eq!(
            back_pos,
            vec![
                ("B.Silkscreen", 15.0, 4.0),
                ("B.Silkscreen", 5.0, 4.0),
                ("B.Silkscreen", 15.0, 12.0),
                ("B.Silkscreen", 5.0, 12.0),
            ]
        );
    }

    #[test]
    fn rect_uses_native_rectangle_y_up() {
        let made = shape_any(&rect_origin()).unwrap();
        assert_eq!(made.kind, "rect");
        assert!(made.item.type_url.contains("BoardGraphicShape"));
        let proto = BoardGraphicShape::decode(made.item.value.as_slice()).unwrap();
        assert_eq!(proto.layer, BL_CMTS_USER);
        assert!(!made.layer.plots);
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
        spec.layer = Some("F.Silkscreen".into());
        spec.stroke_mm = Some(0.08);
        assert!(shape_any(&spec).unwrap_err().contains("0.15"));
    }

    #[test]
    fn default_user_layer_allows_thinner_stroke() {
        let mut spec = rect_origin();
        spec.stroke_mm = Some(0.08);
        assert!(shape_any(&spec).is_ok());
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

    #[test]
    fn overlay_tag_sanitizer() {
        assert_eq!(parse_overlay_tag(None).unwrap(), None);
        assert_eq!(parse_overlay_tag(Some("")).unwrap(), None);
        assert_eq!(parse_overlay_tag(Some("  ")).unwrap(), None);
        assert_eq!(
            parse_overlay_tag(Some("4x5")).unwrap().as_deref(),
            Some("4x5")
        );
        assert_eq!(
            parse_overlay_tag(Some("kicad-mcp:table"))
                .unwrap()
                .as_deref(),
            Some("table")
        );
        assert_eq!(
            parse_overlay_tag(Some("cover_v2")).unwrap().as_deref(),
            Some("cover_v2")
        );
        assert!(parse_overlay_tag(Some("-nope"))
            .unwrap_err()
            .contains("start"));
        assert!(parse_overlay_tag(Some("a/b")).unwrap_err().contains("only"));
        assert!(parse_overlay_tag(Some("a..b")).unwrap_err().contains(".."));
        assert!(parse_overlay_tag(Some(&"x".repeat(41)))
            .unwrap_err()
            .contains("40"));
        assert_eq!(tag_from_group_name("kicad-mcp:4x5").as_deref(), Some("4x5"));
        assert_eq!(tag_from_group_name("user-group"), None);
        assert_eq!(group_name_for_tag("4x5"), "kicad-mcp:4x5");
    }

    #[test]
    fn group_any_encodes_name_and_members() {
        let any = group_any("kicad-mcp:4x5", &["abc".into(), "def".into()]);
        assert!(any.type_url.contains("Group"));
        let proto = BoardGroup::decode(any.value.as_slice()).unwrap();
        assert_eq!(proto.name, "kicad-mcp:4x5");
        assert_eq!(
            proto
                .items
                .iter()
                .map(|k| k.value.as_str())
                .collect::<Vec<_>>(),
            vec!["abc", "def"]
        );
    }

    fn table_2x3() -> ShapeSpec {
        ShapeSpec {
            kind: "table".into(),
            origin_x_mm: Some(10.0),
            origin_y_mm: Some(20.0),
            rows: Some(2),
            cols: Some(3),
            cell_width_mm: Some(10.0),
            cell_height_mm: Some(5.0),
            ..Default::default()
        }
    }

    #[test]
    fn table_2x3_is_outer_rect_plus_inner_lines() {
        let items = shape_items(&table_2x3()).unwrap();
        assert_eq!(items.len(), 4); // 1 rect + 1 h + 2 v
        assert!(items.iter().all(|s| s.group_kind == "table"));
        assert!(!items[0].layer.plots);
        let kinds: Vec<_> = items.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec!["rect", "segment", "segment", "segment"]);

        let proto = BoardGraphicShape::decode(items[0].item.value.as_slice()).unwrap();
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Rectangle(r) => {
                let tl = r.top_left.unwrap();
                let br = r.bottom_right.unwrap();
                assert_eq!(tl.x_nm, mm_to_nm(10.0));
                assert_eq!(tl.y_nm, mm_to_nm(30.0)); // 20 + 2*5
                assert_eq!(br.x_nm, mm_to_nm(40.0)); // 10 + 3*10
                assert_eq!(br.y_nm, mm_to_nm(20.0));
            }
            other => panic!("expected outer rectangle, got {other:?}"),
        }

        let segs: Vec<_> = items[1..]
            .iter()
            .map(|s| {
                let proto = BoardGraphicShape::decode(s.item.value.as_slice()).unwrap();
                match proto.shape.unwrap().geometry.unwrap() {
                    graphic_shape::Geometry::Segment(seg) => {
                        let a = seg.start.unwrap();
                        let b = seg.end.unwrap();
                        (
                            nm_to_mm(a.x_nm),
                            nm_to_mm(a.y_nm),
                            nm_to_mm(b.x_nm),
                            nm_to_mm(b.y_nm),
                        )
                    }
                    other => panic!("expected segment, got {other:?}"),
                }
            })
            .collect();
        assert_eq!(
            segs,
            vec![
                (10.0, 25.0, 40.0, 25.0), // inner horizontal
                (20.0, 20.0, 20.0, 30.0),
                (30.0, 20.0, 30.0, 30.0),
            ]
        );
    }

    #[test]
    fn table_from_centre_and_overall_size() {
        let spec = ShapeSpec {
            kind: "table".into(),
            center_x_mm: Some(148.5),
            center_y_mm: Some(105.0),
            rows: Some(2),
            cols: Some(2),
            width_mm: Some(20.0),
            height_mm: Some(20.0),
            ..Default::default()
        };
        let items = shape_items(&spec).unwrap();
        assert_eq!(items.len(), 3); // 1 rect + 1 h + 1 v
        let proto = BoardGraphicShape::decode(items[0].item.value.as_slice()).unwrap();
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Rectangle(r) => {
                let tl = r.top_left.unwrap();
                let br = r.bottom_right.unwrap();
                assert_eq!(tl.x_nm, mm_to_nm(138.5));
                assert_eq!(tl.y_nm, mm_to_nm(115.0));
                assert_eq!(br.x_nm, mm_to_nm(158.5));
                assert_eq!(br.y_nm, mm_to_nm(95.0));
            }
            other => panic!("expected rectangle, got {other:?}"),
        }
    }

    #[test]
    fn table_refuses_zero_rows_and_huge_grid() {
        let mut spec = table_2x3();
        spec.rows = Some(0);
        assert!(shape_items(&spec).unwrap_err().contains("at least 1"));
        spec.rows = Some(41);
        spec.cols = Some(1);
        spec.cell_width_mm = Some(1.0);
        spec.cell_height_mm = Some(1.0);
        assert!(shape_items(&spec).unwrap_err().contains("40"));
    }

    #[test]
    fn table_1x1_is_just_the_outer_rect() {
        let spec = ShapeSpec {
            kind: "table".into(),
            origin_x_mm: Some(0.0),
            origin_y_mm: Some(0.0),
            rows: Some(1),
            cols: Some(1),
            cell_width_mm: Some(10.0),
            cell_height_mm: Some(8.0),
            ..Default::default()
        };
        let items = shape_items(&spec).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "rect");
        assert_eq!(items[0].group_kind, "table");
    }

    fn stroke_nm(item: &Any) -> i64 {
        let proto = BoardGraphicShape::decode(item.value.as_slice()).unwrap();
        proto
            .shape
            .unwrap()
            .attributes
            .unwrap()
            .stroke
            .unwrap()
            .width
            .unwrap()
            .value_nm
    }

    #[test]
    fn line_is_an_open_segment() {
        let spec = ShapeSpec {
            kind: "hline".into(),
            a_x_mm: Some(10.0),
            a_y_mm: Some(20.0),
            b_x_mm: Some(40.0),
            b_y_mm: Some(20.0),
            stroke_mm: Some(0.3),
            ..Default::default()
        };
        let made = shape_any(&spec).unwrap();
        assert_eq!(made.kind, "segment");
        assert_eq!(made.group_kind, "line");
        assert_eq!(made.stroke_mm, 0.3);
        let proto = BoardGraphicShape::decode(made.item.value.as_slice()).unwrap();
        match proto.shape.unwrap().geometry.unwrap() {
            graphic_shape::Geometry::Segment(seg) => {
                let a = seg.start.unwrap();
                let b = seg.end.unwrap();
                assert_eq!(nm_to_mm(a.x_nm), 10.0);
                assert_eq!(nm_to_mm(a.y_nm), 20.0);
                assert_eq!(nm_to_mm(b.x_nm), 40.0);
                assert_eq!(nm_to_mm(b.y_nm), 20.0);
            }
            other => panic!("expected segment, got {other:?}"),
        }
        let mut tilted = spec.clone();
        tilted.b_y_mm = Some(21.0);
        assert!(shape_any(&tilted).unwrap_err().contains("horizontal"));
    }

    #[test]
    fn vline_must_be_vertical() {
        let spec = ShapeSpec {
            kind: "vline".into(),
            a_x_mm: Some(5.0),
            a_y_mm: Some(0.0),
            b_x_mm: Some(6.0),
            b_y_mm: Some(10.0),
            ..Default::default()
        };
        assert!(shape_any(&spec).unwrap_err().contains("vertical"));
    }

    #[test]
    fn table_border_stroke_thicker_than_inner() {
        let mut spec = table_2x3();
        spec.stroke_mm = Some(0.15);
        spec.border_stroke_mm = Some(0.4);
        let items = shape_items(&spec).unwrap();
        assert_eq!(stroke_nm(&items[0].item), mm_to_nm(0.4));
        assert_eq!(items[0].stroke_mm, 0.4);
        assert_eq!(stroke_nm(&items[1].item), mm_to_nm(0.15));
        assert_eq!(items[1].stroke_mm, 0.15);
    }

    #[test]
    fn table_cells_are_top_row_first_and_need_a_tag() {
        let mut spec = ShapeSpec {
            kind: "table".into(),
            origin_x_mm: Some(0.0),
            origin_y_mm: Some(0.0),
            rows: Some(2),
            cols: Some(2),
            cell_width_mm: Some(10.0),
            cell_height_mm: Some(8.0),
            cells: vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]],
            ..Default::default()
        };
        assert!(shape_items(&spec).unwrap_err().contains("tag"));
        spec.tag = Some("ds".into());
        let layout = table_layout(&spec).unwrap();
        assert_eq!(layout.cell_center(0, 0, false), (5.0, 12.0)); // top-left
        assert_eq!(layout.cell_center(0, 1, false), (15.0, 12.0));
        assert_eq!(layout.cell_center(1, 0, false), (5.0, 4.0)); // bottom-left
        let items = shape_items(&spec).unwrap();
        assert_eq!(items.len(), 3 + 4); // 1 rect + 1 h + 1 v + 4 texts
        assert_eq!(items.iter().filter(|s| s.kind == "text").count(), 4);
        assert!(items
            .iter()
            .filter(|s| s.kind == "text")
            .all(|s| s.item.type_url.contains("BoardText")));
    }

    #[test]
    fn table_cells_infer_rows_and_skip_empty() {
        let spec = ShapeSpec {
            kind: "table".into(),
            origin_x_mm: Some(0.0),
            origin_y_mm: Some(0.0),
            cell_width_mm: Some(10.0),
            cell_height_mm: Some(8.0),
            tag: Some("ds".into()),
            cells: vec![vec!["A".into(), "".into()], vec!["".into(), "D".into()]],
            ..Default::default()
        };
        let items = shape_items(&spec).unwrap();
        assert_eq!(items.iter().filter(|s| s.kind == "text").count(), 2);
        assert_eq!(items.iter().filter(|s| s.kind == "rect").count(), 1);
        assert_eq!(items.iter().filter(|s| s.kind == "segment").count(), 2); // 1h + 1v
        let mut extra = spec.clone();
        extra.rows = Some(1);
        extra.cols = Some(1);
        assert!(shape_items(&extra).unwrap_err().contains("rows"));
    }

    #[test]
    fn table_size_mm_must_fit_the_cell() {
        let spec = ShapeSpec {
            kind: "table".into(),
            origin_x_mm: Some(0.0),
            origin_y_mm: Some(0.0),
            rows: Some(1),
            cols: Some(1),
            cell_width_mm: Some(10.0),
            cell_height_mm: Some(8.0),
            tag: Some("ds".into()),
            cells: vec![vec!["A".into()]],
            size_mm: Some(10.0),
            ..Default::default()
        };
        assert!(shape_items(&spec)
            .unwrap_err()
            .contains("smaller than the cell"));
    }

    #[test]
    fn table_cells_on_back_silk_read_as_on_the_finished_layer() {
        let spec = ShapeSpec {
            kind: "table".into(),
            layer: Some("B.Silkscreen".into()),
            origin_x_mm: Some(0.0),
            origin_y_mm: Some(0.0),
            rows: Some(2),
            cols: Some(2),
            cell_width_mm: Some(10.0),
            cell_height_mm: Some(8.0),
            tag: Some("ds".into()),
            cells: vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]],
            ..Default::default()
        };
        let layout = table_layout(&spec).unwrap();
        // Reading order from the back: [0][0] is visual top-left = board bottom-right.
        assert_eq!(layout.cell_center(0, 0, true), (15.0, 4.0));
        assert_eq!(layout.cell_center(0, 1, true), (5.0, 4.0));
        assert_eq!(layout.cell_center(1, 0, true), (15.0, 12.0));
        assert_eq!(layout.cell_center(1, 1, true), (5.0, 12.0));
        let items = shape_items(&spec).unwrap();
        let texts: Vec<_> = items
            .iter()
            .filter(|s| s.kind == "text")
            .map(|s| {
                (
                    crate::silk::text_body_from_any(&s.item).unwrap(),
                    crate::silk::text_xy_from_any(&s.item).unwrap(),
                )
            })
            .collect();
        assert_eq!(
            texts,
            vec![
                ("A".into(), (15.0, 4.0)),
                ("B".into(), (5.0, 4.0)),
                ("C".into(), (15.0, 12.0)),
                ("D".into(), (5.0, 12.0)),
            ]
        );
    }
}
