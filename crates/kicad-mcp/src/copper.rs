//! Tracks, vias and copper zones via typed `CreateItems` (no autorouter).

use prost::Message;
use prost_types::Any;

use crate::place::mm_to_nm;

const BL_F_CU: i32 = 3;
const BL_IN1_CU: i32 = 4;
const BL_B_CU: i32 = 34;
const LS_UNLOCKED: i32 = 1;
const PSS_CIRCLE: i32 = 1;
const VT_THROUGH: i32 = 1;
const PST_FRONT_INNER_BACK: i32 = 2;
const ZT_COPPER: i32 = 1;
const ZFM_SOLID: i32 = 1;
/// KiCad `IslandRemovalMode`: drop disconnected slivers (F.Cu pour between LED pads).
const IRM_ALWAYS: i32 = 1;
const IRM_NEVER: i32 = 2;
/// KiCad `ZoneConnectionStyle`: thermal relief on SMD and PTH pads.
const ZCS_THERMAL: i32 = 3;
const ZCS_FULL: i32 = 4;
const ZCS_PTH_THERMAL: i32 = 5;
/// PTH wire-pad thermals (hand-solder). 4 × 1.2 mm on inner 1 oz ≈ 7 A at 20 °C.
/// Vias and SMD stay solid (`ZCS_PTH_THERMAL`).
const PTH_SPOKE_NM: i64 = 1_200_000;
const THERMAL_GAP_NM: i64 = 500_000;
/// SMD thermals on a pour (SK6812 / 0603). Narrower than PTH so four spokes fit.
const SMD_SPOKE_NM: i64 = 400_000;
const SMD_GAP_NM: i64 = 300_000;

/// How pads attach to a copper zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZonePadConnect {
    Solid,
    /// PTH only (`ZCS_PTH_THERMAL`). SMD and vias stay solid.
    PthThermal,
    /// SMD and PTH (`ZCS_THERMAL`). Vias stay solid.
    AllThermal,
}

const TYPE_TRACK: &str = "type.googleapis.com/kiapi.board.types.Track";
const TYPE_VIA: &str = "type.googleapis.com/kiapi.board.types.Via";
const TYPE_ZONE: &str = "type.googleapis.com/kiapi.board.types.Zone";

const DEFAULT_TRACK_MM: f64 = 0.25;
const DEFAULT_VIA_DRILL_MM: f64 = 0.3;
const DEFAULT_VIA_SIZE_MM: f64 = 0.6;
/// Max tracks or vias in one `add_tracks` / `add_vias` undo.
pub const COPPER_MAX: usize = 150;
const ZONE_CLEARANCE_NM: i64 = 200_000;
const ZONE_MIN_THICKNESS_NM: i64 = 250_000;

pub fn parse_copper_layer(name: Option<&str>) -> Result<i32, String> {
    let raw = name.unwrap_or("F.Cu").trim();
    let n = raw.replace('_', ".");
    match n.as_str() {
        "F.Cu" | "f.cu" => return Ok(BL_F_CU),
        "B.Cu" | "b.cu" => return Ok(BL_B_CU),
        _ => {}
    }
    let rest = n
        .strip_prefix("In")
        .or_else(|| n.strip_prefix("in"))
        .ok_or_else(|| copper_layer_err(raw))?;
    let num = rest
        .strip_suffix(".Cu")
        .or_else(|| rest.strip_suffix(".cu"))
        .ok_or_else(|| copper_layer_err(raw))?;
    let i: i32 = num.parse().map_err(|_| copper_layer_err(raw))?;
    if (1..=30).contains(&i) {
        Ok(BL_IN1_CU + i - 1)
    } else {
        Err(copper_layer_err(raw))
    }
}

fn copper_layer_err(got: &str) -> String {
    format!("copper layer must be F.Cu, In1.Cu…In30.Cu or B.Cu (got {got})")
}

pub fn layer_name(id: i32) -> String {
    match id {
        BL_F_CU => "F.Cu".into(),
        BL_B_CU => "B.Cu".into(),
        n if (BL_IN1_CU..=33).contains(&n) => format!("In{}.Cu", n - BL_IN1_CU + 1),
        _ => format!("layer_{id}"),
    }
}

/// KiCad copper ids: F.Cu=3, In1.Cu=4 … In30.Cu=33, B.Cu=34.
pub fn is_copper_layer_id(id: i32) -> bool {
    id == BL_F_CU || id == BL_B_CU || (BL_IN1_CU..=33).contains(&id)
}

/// Copper ids a PTH actually has: `copper_layers` plus copper entries in
/// the padstack `layers` list. KiCad often omits an inner from
/// `copper_layers` while still listing it on `layers` (`*.Cu` expands to
/// In1…In30 — keep only layers on this board's stack).
pub fn copper_layer_ids_from_stack(
    stack_layers: &[i32],
    copper_layers: &[i32],
    copper_layer_count: u32,
) -> Vec<i32> {
    let allowed = through_via_layers(copper_layer_count);
    let mut ids: Vec<i32> = copper_layers
        .iter()
        .copied()
        .filter(|id| allowed.contains(id))
        .collect();
    for id in stack_layers {
        if allowed.contains(id) && !ids.contains(id) {
            ids.push(*id);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Through-via copper: F.Cu, inner layers for this stack, B.Cu.
pub fn through_via_layers(copper_layer_count: u32) -> Vec<i32> {
    let count = copper_layer_count.clamp(2, 32);
    let mut layers = vec![BL_F_CU];
    for i in 0..count.saturating_sub(2) {
        layers.push(BL_IN1_CU + i as i32);
    }
    layers.push(BL_B_CU);
    layers
}

pub fn track_any(
    ax_mm: f64,
    ay_mm: f64,
    bx_mm: f64,
    by_mm: f64,
    width_mm: Option<f64>,
    layer: i32,
    net: &str,
) -> Result<Any, String> {
    track_any_coded(ax_mm, ay_mm, bx_mm, by_mm, width_mm, layer, net, 0)
}

pub fn track_any_coded(
    ax_mm: f64,
    ay_mm: f64,
    bx_mm: f64,
    by_mm: f64,
    width_mm: Option<f64>,
    layer: i32,
    net: &str,
    net_code: i32,
) -> Result<Any, String> {
    let width = width_mm.unwrap_or(DEFAULT_TRACK_MM);
    if width < 0.1 || width > 5.0 {
        return Err("track width must be between 0.1 mm and 5 mm".into());
    }
    if net.trim().is_empty() {
        return Err("add_track needs a net name (connect_pins first)".into());
    }
    let item = Track {
        start: Some(Vector2 {
            x_nm: mm_to_nm(ax_mm),
            y_nm: mm_to_nm(ay_mm),
        }),
        end: Some(Vector2 {
            x_nm: mm_to_nm(bx_mm),
            y_nm: mm_to_nm(by_mm),
        }),
        width: Some(Distance {
            value_nm: mm_to_nm(width),
        }),
        locked: LS_UNLOCKED,
        layer,
        net: Some(net_msg(net, net_code)),
    };
    Ok(pack(&item, TYPE_TRACK))
}

pub fn via_any(
    x_mm: f64,
    y_mm: f64,
    net: &str,
    drill_mm: Option<f64>,
    size_mm: Option<f64>,
) -> Result<Any, String> {
    via_any_coded(x_mm, y_mm, net, drill_mm, size_mm, 0)
}

pub fn via_any_coded(
    x_mm: f64,
    y_mm: f64,
    net: &str,
    drill_mm: Option<f64>,
    size_mm: Option<f64>,
    net_code: i32,
) -> Result<Any, String> {
    via_any_on_layers(
        x_mm,
        y_mm,
        net,
        drill_mm,
        size_mm,
        net_code,
        &through_via_layers(2),
    )
}

pub fn via_any_on_layers(
    x_mm: f64,
    y_mm: f64,
    net: &str,
    drill_mm: Option<f64>,
    size_mm: Option<f64>,
    net_code: i32,
    copper_layers: &[i32],
) -> Result<Any, String> {
    if net.trim().is_empty() {
        return Err("add_via needs a net name (connect_pins first)".into());
    }
    let drill = drill_mm.unwrap_or(DEFAULT_VIA_DRILL_MM);
    let size = size_mm.unwrap_or(DEFAULT_VIA_SIZE_MM);
    if drill < 0.2 || drill > 3.0 {
        return Err("via drill must be between 0.2 mm and 3 mm".into());
    }
    if size <= drill {
        return Err("via copper diameter must be larger than the drill".into());
    }
    let layers = if copper_layers.len() < 2 {
        through_via_layers(2)
    } else {
        copper_layers.to_vec()
    };
    let start = *layers.first().unwrap_or(&BL_F_CU);
    let end = *layers.last().unwrap_or(&BL_B_CU);
    let item = Via {
        position: Some(Vector2 {
            x_nm: mm_to_nm(x_mm),
            y_nm: mm_to_nm(y_mm),
        }),
        pad_stack: Some(PadStack {
            r#type: PST_FRONT_INNER_BACK,
            layers: layers.clone(),
            drill: Some(DrillProperties {
                start_layer: start,
                end_layer: end,
                diameter: Some(Vector2 {
                    x_nm: mm_to_nm(drill),
                    y_nm: mm_to_nm(drill),
                }),
                shape: PSS_CIRCLE,
            }),
            copper_layers: layers
                .iter()
                .map(|layer| via_copper_layer(*layer, size))
                .collect(),
        }),
        locked: LS_UNLOCKED,
        net: Some(net_msg(net, net_code)),
        r#type: VT_THROUGH,
    };
    Ok(pack(&item, TYPE_VIA))
}

fn via_copper_layer(layer: i32, size_mm: f64) -> PadStackLayer {
    PadStackLayer {
        layer,
        shape: PSS_CIRCLE,
        size: Some(Vector2 {
            x_nm: mm_to_nm(size_mm),
            y_nm: mm_to_nm(size_mm),
        }),
    }
}

pub fn rect_zone_any(
    origin_x_mm: f64,
    origin_y_mm: f64,
    width_mm: f64,
    height_mm: f64,
    layer: i32,
    net: &str,
    name: Option<&str>,
) -> Result<Any, String> {
    if net.trim().is_empty() {
        return Err("set_copper_zone needs a net name (connect_pins first)".into());
    }
    if width_mm < 2.0 || height_mm < 2.0 {
        return Err("copper zone must be at least 2 × 2 mm".into());
    }
    let x0 = mm_to_nm(origin_x_mm);
    let y0 = mm_to_nm(origin_y_mm);
    let x1 = mm_to_nm(origin_x_mm + width_mm);
    let y1 = mm_to_nm(origin_y_mm + height_mm);
    poly_zone_any(
        &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
        layer,
        net,
        name,
        0,
        ZonePadConnect::Solid,
        false,
    )
}

pub fn rect_zone_any_coded(
    origin_x_mm: f64,
    origin_y_mm: f64,
    width_mm: f64,
    height_mm: f64,
    layer: i32,
    net: &str,
    name: Option<&str>,
    net_code: i32,
    pads: ZonePadConnect,
    remove_islands: bool,
) -> Result<Any, String> {
    if net.trim().is_empty() {
        return Err("set_copper_zone needs a net name (connect_pins first)".into());
    }
    if width_mm < 2.0 || height_mm < 2.0 {
        return Err("copper zone must be at least 2 × 2 mm".into());
    }
    let x0 = mm_to_nm(origin_x_mm);
    let y0 = mm_to_nm(origin_y_mm);
    let x1 = mm_to_nm(origin_x_mm + width_mm);
    let y1 = mm_to_nm(origin_y_mm + height_mm);
    poly_zone_any(
        &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
        layer,
        net,
        name,
        net_code,
        pads,
        remove_islands,
    )
}

/// Copper zone from already-nanometre polygon corners (not closed).
pub fn poly_zone_mm(
    points_mm: &[(f64, f64)],
    layer: i32,
    net: &str,
    name: Option<&str>,
) -> Result<Any, String> {
    if net.trim().is_empty() {
        return Err("set_copper_zone needs a net name (connect_pins first)".into());
    }
    if points_mm.len() < 3 {
        return Err("copper zone polygon needs at least 3 points".into());
    }
    if points_mm.len() > 400 {
        return Err(format!(
            "copper zone polygon max 400 points (got {})",
            points_mm.len()
        ));
    }
    let nm: Vec<(i64, i64)> = points_mm
        .iter()
        .map(|(x, y)| (mm_to_nm(*x), mm_to_nm(*y)))
        .collect();
    poly_zone_any(&nm, layer, net, name, 0, ZonePadConnect::Solid, false)
}

pub fn poly_zone_mm_coded(
    points_mm: &[(f64, f64)],
    layer: i32,
    net: &str,
    name: Option<&str>,
    net_code: i32,
) -> Result<Any, String> {
    poly_zone_mm_ex(
        points_mm,
        layer,
        net,
        name,
        net_code,
        ZonePadConnect::Solid,
        false,
    )
}

pub fn poly_zone_mm_ex(
    points_mm: &[(f64, f64)],
    layer: i32,
    net: &str,
    name: Option<&str>,
    net_code: i32,
    pads: ZonePadConnect,
    remove_islands: bool,
) -> Result<Any, String> {
    if net.trim().is_empty() {
        return Err("set_copper_zone needs a net name (connect_pins first)".into());
    }
    if points_mm.len() < 3 {
        return Err("copper zone polygon needs at least 3 points".into());
    }
    if points_mm.len() > 400 {
        return Err(format!(
            "copper zone polygon max 400 points (got {})",
            points_mm.len()
        ));
    }
    let nm: Vec<(i64, i64)> = points_mm
        .iter()
        .map(|(x, y)| (mm_to_nm(*x), mm_to_nm(*y)))
        .collect();
    poly_zone_any(&nm, layer, net, name, net_code, pads, remove_islands)
}

fn poly_zone_any(
    corners_nm: &[(i64, i64)],
    layer: i32,
    net: &str,
    name: Option<&str>,
    net_code: i32,
    pads: ZonePadConnect,
    remove_islands: bool,
) -> Result<Any, String> {
    let nodes = corners_nm
        .iter()
        .map(|(x_nm, y_nm)| PolyLineNode {
            geometry: Some(poly_line_node::Geometry::Point(Vector2 {
                x_nm: *x_nm,
                y_nm: *y_nm,
            })),
        })
        .collect();
    let zone = Zone {
        r#type: ZT_COPPER,
        layers: vec![layer],
        outline: Some(PolySet {
            polygons: vec![PolygonWithHoles {
                outline: Some(PolyLine {
                    nodes,
                    closed: true,
                }),
                holes: vec![],
            }],
        }),
        name: name.unwrap_or(net).to_string(),
        filled: false,
        filled_polygons: vec![],
        locked: LS_UNLOCKED,
        settings: Some(zone::Settings::CopperSettings(CopperZoneSettings {
            connection: zone_pad_connection(net, pads),
            clearance: Some(Distance {
                value_nm: ZONE_CLEARANCE_NM,
            }),
            min_thickness: Some(Distance {
                value_nm: ZONE_MIN_THICKNESS_NM,
            }),
            island_mode: zone_island_mode(remove_islands),
            fill_mode: ZFM_SOLID,
            net: Some(net_msg(net, net_code)),
        })),
    };
    Ok(pack(&zone, TYPE_ZONE))
}

fn zone_pad_connection(net: &str, pads: ZonePadConnect) -> Option<ZoneConnectionSettings> {
    let power = matches!(net, "5V" | "GND");
    let mode = if power { pads } else { ZonePadConnect::Solid };
    match mode {
        ZonePadConnect::Solid => Some(ZoneConnectionSettings {
            zone_connection: ZCS_FULL,
            thermal_spokes: None,
        }),
        ZonePadConnect::PthThermal => Some(ZoneConnectionSettings {
            zone_connection: ZCS_PTH_THERMAL,
            thermal_spokes: Some(ThermalSpokeSettings {
                width: Some(Distance {
                    value_nm: PTH_SPOKE_NM,
                }),
                gap: Some(Distance {
                    value_nm: THERMAL_GAP_NM,
                }),
            }),
        }),
        ZonePadConnect::AllThermal => Some(ZoneConnectionSettings {
            zone_connection: ZCS_THERMAL,
            thermal_spokes: Some(ThermalSpokeSettings {
                width: Some(Distance {
                    value_nm: SMD_SPOKE_NM,
                }),
                gap: Some(Distance {
                    value_nm: SMD_GAP_NM,
                }),
            }),
        }),
    }
}

/// KiCad island policy. Default keeps slivers (`IRM_NEVER`); `true` drops them.
pub fn zone_island_mode(remove_islands: bool) -> i32 {
    if remove_islands {
        IRM_ALWAYS
    } else {
        IRM_NEVER
    }
}

/// Map MCP flags onto pad-connect mode. `thermal_smd` wins over PTH-only.
pub fn zone_pad_connect_from_flags(thermal: bool, thermal_smd: bool) -> ZonePadConnect {
    if thermal_smd {
        ZonePadConnect::AllThermal
    } else if thermal {
        ZonePadConnect::PthThermal
    } else {
        ZonePadConnect::Solid
    }
}

fn net_msg(name: &str, code: i32) -> Net {
    Net {
        code: Some(NetCode { value: code }),
        name: name.to_string(),
    }
}

fn pack(msg: &impl Message, type_url: &str) -> Any {
    Any {
        type_url: type_url.into(),
        value: msg.encode_to_vec(),
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
struct Net {
    #[prost(message, optional, tag = "1")]
    code: Option<NetCode>,
    #[prost(string, tag = "2")]
    name: String,
}

#[derive(Clone, PartialEq, Message)]
struct NetCode {
    #[prost(int32, tag = "1")]
    value: i32,
}

#[derive(Clone, PartialEq, Message)]
struct Track {
    #[prost(message, optional, tag = "2")]
    start: Option<Vector2>,
    #[prost(message, optional, tag = "3")]
    end: Option<Vector2>,
    #[prost(message, optional, tag = "4")]
    width: Option<Distance>,
    #[prost(int32, tag = "5")]
    locked: i32,
    #[prost(int32, tag = "6")]
    layer: i32,
    #[prost(message, optional, tag = "7")]
    net: Option<Net>,
}

#[derive(Clone, PartialEq, Message)]
struct DrillProperties {
    #[prost(int32, tag = "1")]
    start_layer: i32,
    #[prost(int32, tag = "2")]
    end_layer: i32,
    #[prost(message, optional, tag = "3")]
    diameter: Option<Vector2>,
    #[prost(int32, tag = "4")]
    shape: i32,
}

#[derive(Clone, PartialEq, Message)]
struct PadStackLayer {
    #[prost(int32, tag = "1")]
    layer: i32,
    #[prost(int32, tag = "2")]
    shape: i32,
    #[prost(message, optional, tag = "3")]
    size: Option<Vector2>,
}

#[derive(Clone, PartialEq, Message)]
struct PadStack {
    #[prost(int32, tag = "1")]
    r#type: i32,
    #[prost(int32, repeated, tag = "2")]
    layers: Vec<i32>,
    #[prost(message, optional, tag = "3")]
    drill: Option<DrillProperties>,
    #[prost(message, repeated, tag = "5")]
    copper_layers: Vec<PadStackLayer>,
}

#[derive(Clone, PartialEq, Message)]
struct Via {
    #[prost(message, optional, tag = "2")]
    position: Option<Vector2>,
    #[prost(message, optional, tag = "3")]
    pad_stack: Option<PadStack>,
    #[prost(int32, tag = "4")]
    locked: i32,
    #[prost(message, optional, tag = "5")]
    net: Option<Net>,
    #[prost(int32, tag = "6")]
    r#type: i32,
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
struct ThermalSpokeSettings {
    #[prost(message, optional, tag = "1")]
    width: Option<Distance>,
    #[prost(message, optional, tag = "3")]
    gap: Option<Distance>,
}

#[derive(Clone, PartialEq, Message)]
struct ZoneConnectionSettings {
    #[prost(int32, tag = "1")]
    zone_connection: i32,
    #[prost(message, optional, tag = "2")]
    thermal_spokes: Option<ThermalSpokeSettings>,
}

#[derive(Clone, PartialEq, Message)]
struct CopperZoneSettings {
    #[prost(message, optional, tag = "1")]
    connection: Option<ZoneConnectionSettings>,
    #[prost(message, optional, tag = "2")]
    clearance: Option<Distance>,
    #[prost(message, optional, tag = "3")]
    min_thickness: Option<Distance>,
    #[prost(int32, tag = "4")]
    island_mode: i32,
    #[prost(int32, tag = "6")]
    fill_mode: i32,
    #[prost(message, optional, tag = "8")]
    net: Option<Net>,
}

#[derive(Clone, PartialEq, Message)]
struct ZoneFilledPolygons {
    #[prost(int32, tag = "1")]
    layer: i32,
    #[prost(message, optional, tag = "2")]
    shapes: Option<PolySet>,
}

#[derive(Clone, PartialEq, Message)]
struct Zone {
    #[prost(int32, tag = "2")]
    r#type: i32,
    #[prost(int32, repeated, tag = "3")]
    layers: Vec<i32>,
    #[prost(message, optional, tag = "4")]
    outline: Option<PolySet>,
    #[prost(string, tag = "5")]
    name: String,
    #[prost(bool, tag = "9")]
    filled: bool,
    #[prost(message, repeated, tag = "10")]
    filled_polygons: Vec<ZoneFilledPolygons>,
    #[prost(int32, tag = "12")]
    locked: i32,
    #[prost(oneof = "zone::Settings", tags = "6")]
    settings: Option<zone::Settings>,
}

/// How a copper zone attaches pads (KiCad `ZoneConnectionStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneConnectStyle {
    Solid,
    PthThermal,
    Thermal,
    Other,
}

/// Filled pour on one copper layer (board millimetres, even-odd contours).
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneFill {
    pub layer_id: i32,
    pub contours: Vec<Vec<(f64, f64)>>,
}

/// Copper pour as read back from GetItems (net + layers). Keepouts are skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneSnap {
    pub name: String,
    pub net: String,
    pub layer_ids: Vec<i32>,
    pub connection: ZoneConnectStyle,
    pub filled: bool,
    pub fills: Vec<ZoneFill>,
}

pub fn zone_connect_style(code: i32) -> ZoneConnectStyle {
    match code {
        ZCS_FULL => ZoneConnectStyle::Solid,
        ZCS_PTH_THERMAL => ZoneConnectStyle::PthThermal,
        ZCS_THERMAL => ZoneConnectStyle::Thermal,
        _ => ZoneConnectStyle::Other,
    }
}

fn polyline_mm(line: &PolyLine) -> Option<Vec<(f64, f64)>> {
    let mut pts = Vec::new();
    for node in &line.nodes {
        let Some(poly_line_node::Geometry::Point(p)) = &node.geometry else {
            continue;
        };
        pts.push((p.x_nm as f64 / 1_000_000.0, p.y_nm as f64 / 1_000_000.0));
    }
    (pts.len() >= 3).then_some(pts)
}

fn polyset_contours(set: &PolySet) -> Vec<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    for poly in &set.polygons {
        if let Some(outline) = &poly.outline {
            if let Some(pts) = polyline_mm(outline) {
                out.push(pts);
            }
        }
        for hole in &poly.holes {
            if let Some(pts) = polyline_mm(hole) {
                out.push(pts);
            }
        }
    }
    out
}

pub fn zone_snap_from_any(any: &Any) -> Option<ZoneSnap> {
    if !any.type_url.contains("Zone") {
        return None;
    }
    let z = Zone::decode(any.value.as_slice()).ok()?;
    let (net, connection) = match &z.settings {
        Some(zone::Settings::CopperSettings(s)) => {
            let net = s
                .net
                .as_ref()
                .map(|n| n.name.as_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(z.name.as_str())
                .to_string();
            let connection = s
                .connection
                .as_ref()
                .map(|c| zone_connect_style(c.zone_connection))
                .unwrap_or(ZoneConnectStyle::Other);
            (net, connection)
        }
        _ => return None,
    };
    if net.is_empty() {
        return None;
    }
    let fills = z
        .filled_polygons
        .iter()
        .filter_map(|fp| {
            let contours = fp.shapes.as_ref().map(polyset_contours).unwrap_or_default();
            (!contours.is_empty()).then_some(ZoneFill {
                layer_id: fp.layer,
                contours,
            })
        })
        .collect();
    Some(ZoneSnap {
        name: z.name,
        net,
        layer_ids: z.layers,
        connection,
        filled: z.filled,
        fills,
    })
}

mod zone {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Settings {
        #[prost(message, tag = "6")]
        CopperSettings(super::CopperZoneSettings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_encodes() {
        let any = track_any(10.0, 20.0, 11.0, 20.0, Some(0.25), BL_F_CU, "5V").unwrap();
        assert!(any.type_url.contains("Track"));
        assert!(!any.value.is_empty());
    }

    #[test]
    fn via_rejects_tiny_annular_ring() {
        let err = via_any(0.0, 0.0, "GND", Some(0.6), Some(0.6)).unwrap_err();
        assert!(err.contains("larger"));
    }

    #[test]
    fn via_has_front_and_back_copper() {
        let any = via_any(1.0, 2.0, "GND", Some(0.3), Some(0.6)).unwrap();
        let v = Via::decode(any.value.as_slice()).unwrap();
        let stack = v.pad_stack.unwrap();
        let layers: Vec<i32> = stack.copper_layers.iter().map(|l| l.layer).collect();
        assert_eq!(layers, vec![BL_F_CU, BL_B_CU]);
        assert_eq!(stack.r#type, PST_FRONT_INNER_BACK);
    }

    #[test]
    fn zone_has_four_corners() {
        let any = rect_zone_any(0.0, 0.0, 40.0, 30.0, BL_B_CU, "GND", None).unwrap();
        assert!(any.type_url.contains("Zone"));
        let z = Zone::decode(any.value.as_slice()).unwrap();
        assert_eq!(z.layers, vec![BL_B_CU]);
        let n = z.outline.unwrap().polygons[0]
            .outline
            .as_ref()
            .unwrap()
            .nodes
            .len();
        assert_eq!(n, 4);
    }

    #[test]
    fn zone_defaults_to_solid_connection() {
        let any = rect_zone_any(0.0, 0.0, 40.0, 30.0, BL_F_CU, "5V", None).unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        let conn = s.connection.expect("5V zone must set pad connection");
        assert_eq!(conn.zone_connection, ZCS_FULL);
        assert!(conn.thermal_spokes.is_none());
        assert_eq!(s.island_mode, IRM_NEVER);
    }

    #[test]
    fn fivev_zone_can_request_pth_thermal() {
        let any = rect_zone_any_coded(
            0.0,
            0.0,
            40.0,
            30.0,
            BL_F_CU,
            "5V",
            None,
            0,
            ZonePadConnect::PthThermal,
            false,
        )
        .unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        let conn = s.connection.expect("5V zone must set pad connection");
        assert_eq!(conn.zone_connection, ZCS_PTH_THERMAL);
        let spokes = conn.thermal_spokes.expect("thermal spokes");
        assert_eq!(spokes.width.unwrap().value_nm, PTH_SPOKE_NM);
        assert_eq!(spokes.gap.unwrap().value_nm, THERMAL_GAP_NM);
    }

    #[test]
    fn gnd_zone_can_request_pth_thermal() {
        let any = rect_zone_any_coded(
            0.0,
            0.0,
            40.0,
            30.0,
            BL_B_CU,
            "GND",
            None,
            0,
            ZonePadConnect::PthThermal,
            false,
        )
        .unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        let conn = s.connection.expect("GND zone must set pad connection");
        assert_eq!(conn.zone_connection, ZCS_PTH_THERMAL);
        let spokes = conn.thermal_spokes.expect("thermal spokes");
        assert_eq!(spokes.width.unwrap().value_nm, PTH_SPOKE_NM);
        assert_eq!(spokes.gap.unwrap().value_nm, THERMAL_GAP_NM);
    }

    #[test]
    fn gnd_zone_can_request_smd_thermal() {
        let any = rect_zone_any_coded(
            0.0,
            0.0,
            40.0,
            30.0,
            BL_F_CU,
            "GND",
            None,
            0,
            ZonePadConnect::AllThermal,
            false,
        )
        .unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        let conn = s.connection.expect("GND zone must set pad connection");
        assert_eq!(conn.zone_connection, ZCS_THERMAL);
        let spokes = conn.thermal_spokes.expect("thermal spokes");
        assert_eq!(spokes.width.unwrap().value_nm, SMD_SPOKE_NM);
        assert_eq!(spokes.gap.unwrap().value_nm, SMD_GAP_NM);
    }

    #[test]
    fn thermal_smd_flag_wins_over_pth() {
        assert_eq!(
            zone_pad_connect_from_flags(true, true),
            ZonePadConnect::AllThermal
        );
        assert_eq!(
            zone_pad_connect_from_flags(true, false),
            ZonePadConnect::PthThermal
        );
        assert_eq!(
            zone_pad_connect_from_flags(false, true),
            ZonePadConnect::AllThermal
        );
        assert_eq!(
            zone_pad_connect_from_flags(false, false),
            ZonePadConnect::Solid
        );
    }

    #[test]
    fn remove_islands_sets_irm_always() {
        assert_eq!(zone_island_mode(false), IRM_NEVER);
        assert_eq!(zone_island_mode(true), IRM_ALWAYS);
        let any = rect_zone_any_coded(
            0.0,
            0.0,
            40.0,
            30.0,
            BL_F_CU,
            "GND",
            None,
            0,
            ZonePadConnect::AllThermal,
            true,
        )
        .unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        assert_eq!(s.island_mode, IRM_ALWAYS);
    }

    #[test]
    fn parses_inner_copper_layers() {
        assert_eq!(parse_copper_layer(Some("In1.Cu")).unwrap(), BL_IN1_CU);
        assert_eq!(parse_copper_layer(Some("In2.Cu")).unwrap(), BL_IN1_CU + 1);
        assert_eq!(layer_name(BL_IN1_CU + 1), "In2.Cu");
        assert_eq!(through_via_layers(4), vec![BL_F_CU, 4, 5, BL_B_CU]);
    }

    #[test]
    fn four_layer_via_lists_inner_copper() {
        let layers = through_via_layers(4);
        let any = via_any_on_layers(1.0, 2.0, "5V", Some(0.3), Some(0.6), 0, &layers).unwrap();
        let v = Via::decode(any.value.as_slice()).unwrap();
        let stack = v.pad_stack.unwrap();
        let got: Vec<i32> = stack.copper_layers.iter().map(|l| l.layer).collect();
        assert_eq!(got, vec![BL_F_CU, 4, 5, BL_B_CU]);
    }

    #[test]
    fn zone_snap_reads_net_and_layer() {
        let any = rect_zone_any(0.0, 0.0, 40.0, 30.0, BL_B_CU, "GND", None).unwrap();
        let snap = zone_snap_from_any(&any).expect("copper zone");
        assert_eq!(snap.net, "GND");
        assert_eq!(snap.layer_ids, vec![BL_B_CU]);
        assert_eq!(snap.connection, ZoneConnectStyle::Solid);
    }

    #[test]
    fn zone_snap_reads_pth_thermal() {
        let any = rect_zone_any_coded(
            0.0,
            0.0,
            40.0,
            30.0,
            BL_IN1_CU,
            "5V",
            Some("5V_IN1"),
            0,
            ZonePadConnect::PthThermal,
            false,
        )
        .unwrap();
        let snap = zone_snap_from_any(&any).expect("copper zone");
        assert_eq!(snap.connection, ZoneConnectStyle::PthThermal);
    }

    #[test]
    fn padstack_layers_union_picks_up_inner_from_layers_list() {
        // IPC often lists In2 on `layers` but drops it from `copper_layers`.
        let wildcard: Vec<i32> = (BL_F_CU..=33).chain(std::iter::once(BL_B_CU)).collect();
        let ids = copper_layer_ids_from_stack(&wildcard, &[BL_F_CU, BL_IN1_CU, BL_B_CU], 4);
        assert_eq!(ids, vec![BL_F_CU, BL_IN1_CU, BL_IN1_CU + 1, BL_B_CU]);
    }
}
