//! Tracks, vias and copper zones via typed `CreateItems` (no autorouter).

use prost::Message;
use prost_types::Any;

use crate::place::mm_to_nm;

const BL_F_CU: i32 = 3;
const BL_B_CU: i32 = 34;
const LS_UNLOCKED: i32 = 1;
const PSS_CIRCLE: i32 = 1;
const VT_THROUGH: i32 = 1;
const PST_FRONT_INNER_BACK: i32 = 2;
const ZT_COPPER: i32 = 1;
const ZFM_SOLID: i32 = 1;
const IRM_NEVER: i32 = 2;
const ZCS_THERMAL: i32 = 3;
const ZCS_PTH_THERMAL: i32 = 5;
/// 5V SMD thermals (JLCPCB HASL / no tombstone).
const THERMAL_SPOKE_NM: i64 = 300_000;
const THERMAL_GAP_NM: i64 = 500_000;
/// GND wire-pad PTH: wider spokes, vias stay solid (`ZCS_PTH_THERMAL`).
const GND_PTH_SPOKE_NM: i64 = 500_000;

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
    match name.unwrap_or("F.Cu") {
        "F.Cu" | "F_Cu" | "f.cu" => Ok(BL_F_CU),
        "B.Cu" | "B_Cu" | "b.cu" => Ok(BL_B_CU),
        other => Err(format!("copper layer must be F.Cu or B.Cu (got {other})")),
    }
}

pub fn layer_name(id: i32) -> &'static str {
    match id {
        BL_B_CU => "B.Cu",
        _ => "F.Cu",
    }
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
    let item = Via {
        position: Some(Vector2 {
            x_nm: mm_to_nm(x_mm),
            y_nm: mm_to_nm(y_mm),
        }),
        pad_stack: Some(PadStack {
            r#type: PST_FRONT_INNER_BACK,
            layers: vec![BL_F_CU, BL_B_CU],
            drill: Some(DrillProperties {
                start_layer: BL_F_CU,
                end_layer: BL_B_CU,
                diameter: Some(Vector2 {
                    x_nm: mm_to_nm(drill),
                    y_nm: mm_to_nm(drill),
                }),
                shape: PSS_CIRCLE,
            }),
            copper_layers: vec![
                via_copper_layer(BL_F_CU, size),
                via_copper_layer(BL_B_CU, size),
            ],
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
    poly_zone_any(&nm, layer, net, name, 0)
}

pub fn poly_zone_mm_coded(
    points_mm: &[(f64, f64)],
    layer: i32,
    net: &str,
    name: Option<&str>,
    net_code: i32,
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
    poly_zone_any(&nm, layer, net, name, net_code)
}

fn poly_zone_any(
    corners_nm: &[(i64, i64)],
    layer: i32,
    net: &str,
    name: Option<&str>,
    net_code: i32,
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
        locked: LS_UNLOCKED,
        settings: Some(zone::Settings::CopperSettings(CopperZoneSettings {
            connection: zone_pad_connection(net),
            clearance: Some(Distance {
                value_nm: ZONE_CLEARANCE_NM,
            }),
            min_thickness: Some(Distance {
                value_nm: ZONE_MIN_THICKNESS_NM,
            }),
            island_mode: IRM_NEVER,
            fill_mode: ZFM_SOLID,
            net: Some(net_msg(net, net_code)),
        })),
    };
    Ok(pack(&zone, TYPE_ZONE))
}

fn zone_pad_connection(net: &str) -> Option<ZoneConnectionSettings> {
    let (style, spoke_nm) = match net {
        "5V" => (ZCS_THERMAL, THERMAL_SPOKE_NM),
        "GND" => (ZCS_PTH_THERMAL, GND_PTH_SPOKE_NM),
        _ => return None,
    };
    Some(ZoneConnectionSettings {
        zone_connection: style,
        thermal_spokes: Some(ThermalSpokeSettings {
            width: Some(Distance { value_nm: spoke_nm }),
            gap: Some(Distance {
                value_nm: THERMAL_GAP_NM,
            }),
        }),
    })
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
struct Zone {
    #[prost(int32, tag = "2")]
    r#type: i32,
    #[prost(int32, repeated, tag = "3")]
    layers: Vec<i32>,
    #[prost(message, optional, tag = "4")]
    outline: Option<PolySet>,
    #[prost(string, tag = "5")]
    name: String,
    #[prost(int32, tag = "12")]
    locked: i32,
    #[prost(oneof = "zone::Settings", tags = "6")]
    settings: Option<zone::Settings>,
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
    fn fivev_zone_uses_thermal_relief() {
        let any = rect_zone_any(0.0, 0.0, 40.0, 30.0, BL_F_CU, "5V", None).unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        let conn = s.connection.expect("5V zone must set pad connection");
        assert_eq!(conn.zone_connection, ZCS_THERMAL);
        let spokes = conn.thermal_spokes.expect("thermal spokes");
        assert_eq!(spokes.width.unwrap().value_nm, THERMAL_SPOKE_NM);
        assert_eq!(spokes.gap.unwrap().value_nm, THERMAL_GAP_NM);
    }

    #[test]
    fn gnd_zone_uses_pth_thermal() {
        let any = rect_zone_any(0.0, 0.0, 40.0, 30.0, BL_B_CU, "GND", None).unwrap();
        let z = Zone::decode(any.value.as_slice()).unwrap();
        let Some(zone::Settings::CopperSettings(s)) = z.settings else {
            panic!("expected copper settings");
        };
        let conn = s.connection.expect("GND zone must set pad connection");
        assert_eq!(conn.zone_connection, ZCS_PTH_THERMAL);
        let spokes = conn.thermal_spokes.expect("thermal spokes");
        assert_eq!(spokes.width.unwrap().value_nm, GND_PTH_SPOKE_NM);
        assert_eq!(spokes.gap.unwrap().value_nm, THERMAL_GAP_NM);
    }
}
