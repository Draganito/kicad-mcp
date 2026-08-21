//! Pad truth and footprint moves, straight from the board.
//!
//! `board_pads` decodes the raw `FootprintInstance` protos KiCad returns
//! over IPC — the **baked** pad positions, not template math. This is the
//! verification tool: a mirrored or mis-rotated part shows up here as pads
//! on the wrong side of the anchor, no 3D render needed.
//!
//! `move_footprint` applies a rigid transform (translate + rotate about the
//! anchor) to the instance and every nested pad, then `UpdateItems` the
//! whole footprint — the same splice path as `connect_pins`, so nets,
//! padstack geometry and the reference survive. No remove+place.

use std::collections::HashMap;

use prost::Message;
use prost_types::Any;

use crate::kicad::Kicad;
use crate::proto_wire::{map_len_fields, set_len_field};

const NM_PER_MM: f64 = 1_000_000.0;
const TYPE_PAD: &str = "type.googleapis.com/kiapi.board.types.Pad";

// Layer / enum codes as in place.rs (kiapi.board.types).
const BL_F_CU: i32 = 3;
const BL_B_CU: i32 = 34;
const PT_PTH: i32 = 1;
const PT_SMD: i32 = 2;
const PT_NPTH: i32 = 4;
const PSS_CIRCLE: i32 = 1;
const PSS_RECTANGLE: i32 = 2;
const PSS_OVAL: i32 = 3;

fn nm_to_mm(nm: i64) -> f64 {
    nm as f64 / NM_PER_MM
}

fn mm_to_nm(mm: f64) -> i64 {
    (mm * NM_PER_MM).round() as i64
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PadRow {
    pub reference: String,
    pub pin: String,
    pub net: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotation_deg: f64,
    /// `smd`, `pth`, or `npth`.
    pub kind: String,
    /// `rect`, `oval`, `circle`.
    pub shape: String,
    pub layer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drill_mm: Option<f64>,
}

/// Baked pad rows for every footprint on the board (IPC raw protos, the
/// positions KiCad actually draws). Optional reference / net filters.
pub async fn board_pads(
    k: &Kicad,
    reference: Option<&str>,
    net: Option<&str>,
) -> Result<Vec<PadRow>, String> {
    let fps = k.footprints().await?;
    let mut ref_of: HashMap<String, String> = HashMap::new();
    for fp in &fps {
        if let (Some(id), Some(r)) = (fp.id.as_deref(), fp.reference.as_deref()) {
            ref_of.insert(id.to_string(), r.to_string());
        }
    }
    let mut net_of: HashMap<String, String> = HashMap::new();
    for entry in k.pad_netlist().await.unwrap_or_default() {
        if let Some(id) = entry.pad_id {
            net_of.insert(id, entry.net_name.unwrap_or_default());
        }
    }
    let raws = k
        .raw_items(vec![kicad_ipc_rs::PcbObjectTypeCode::new_footprint().code])
        .await?;
    let mut rows = Vec::new();
    for raw in &raws {
        let inst = FpInstDec::decode(raw.value.as_slice())
            .map_err(|e| format!("footprint proto decode: {e}"))?;
        let fp_ref = inst
            .id
            .as_ref()
            .and_then(|id| ref_of.get(&id.value))
            .cloned()
            .unwrap_or_default();
        if let Some(want) = reference {
            if fp_ref != want {
                continue;
            }
        }
        let Some(def) = inst.definition else {
            continue;
        };
        for item in &def.items {
            let Some(pad) = decode_pad(item) else {
                continue;
            };
            let pad_net = pad
                .id
                .as_ref()
                .and_then(|id| net_of.get(&id.value))
                .cloned()
                .unwrap_or_default();
            if let Some(want) = net {
                if pad_net != want {
                    continue;
                }
            }
            let pos = pad.position.unwrap_or_default();
            let stack = pad.pad_stack.unwrap_or_default();
            let copper = stack.copper_layers.first();
            let size = copper.and_then(|c| c.size.clone()).unwrap_or_default();
            rows.push(PadRow {
                reference: fp_ref.clone(),
                pin: pad.number.clone(),
                net: pad_net,
                x_mm: round4(nm_to_mm(pos.x_nm)),
                y_mm: round4(nm_to_mm(pos.y_nm)),
                width_mm: round4(nm_to_mm(size.x_nm)),
                height_mm: round4(nm_to_mm(size.y_nm)),
                rotation_deg: stack.angle.map(|a| a.value_degrees).unwrap_or(0.0),
                kind: match pad.r#type {
                    PT_PTH => "pth",
                    PT_NPTH => "npth",
                    PT_SMD => "smd",
                    _ => "?",
                }
                .into(),
                shape: match copper.map(|c| c.shape) {
                    Some(PSS_CIRCLE) => "circle",
                    Some(PSS_OVAL) => "oval",
                    Some(PSS_RECTANGLE) => "rect",
                    _ => "?",
                }
                .into(),
                layer: match copper.map(|c| c.layer) {
                    Some(BL_B_CU) => "B.Cu".into(),
                    Some(BL_F_CU) => "F.Cu".into(),
                    Some(other) => format!("L{other}"),
                    None => "?".into(),
                },
                drill_mm: stack
                    .drill
                    .and_then(|d| d.diameter)
                    .map(|v| round4(nm_to_mm(v.x_nm)))
                    .filter(|d| *d > 0.0),
            });
        }
    }
    rows.sort_by(|a, b| {
        (natural_ref(&a.reference), a.pin.parse::<u32>().ok(), &a.pin).cmp(&(
            natural_ref(&b.reference),
            b.pin.parse::<u32>().ok(),
            &b.pin,
        ))
    });
    Ok(rows)
}

/// `U14` → (`U`, 14) so U2 sorts before U14.
fn natural_ref(r: &str) -> (String, u32) {
    let prefix: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let num = r[prefix.len()..].parse::<u32>().unwrap_or(0);
    (prefix, num)
}

/// Rigid transform in KiCad's y-down frame with visually-CCW positive
/// angles — the same rotation sense as `place::world_xy`. A point at
/// `(px, py)` rides along when the anchor moves from `old` to `new` and
/// the part turns by `delta_deg`.
pub(crate) fn rigid_xy(
    px: f64,
    py: f64,
    old_anchor: (f64, f64),
    new_anchor: (f64, f64),
    delta_deg: f64,
) -> (f64, f64) {
    let rad = delta_deg.to_radians();
    let (s, c) = (rad.sin(), rad.cos());
    let dx = px - old_anchor.0;
    let dy = py - old_anchor.1;
    (new_anchor.0 + c * dx + s * dy, new_anchor.1 - s * dx + c * dy)
}

/// Patch a raw `FootprintInstance` Any: anchor, orientation, every nested
/// pad (position + padstack angle) and the reference/value text positions.
/// Pure splice — everything not touched (nets, padstack geometry, KIIDs)
/// passes through byte-identical. Returns the patched Any and how many
/// pads were transformed.
pub fn transform_footprint_any(
    fp: &Any,
    old: (f64, f64, f64),
    new: (f64, f64, f64),
) -> Result<(Any, usize), String> {
    let old_anchor = (old.0, old.1);
    let new_anchor = (new.0, new.1);
    let delta_deg = new.2 - old.2;
    let pads_moved = std::cell::Cell::new(0usize);

    // Instance anchor + orientation.
    let mut value = set_len_field(&fp.value, 2, &encode_vec2(new.0, new.1))?;
    value = set_len_field(&value, 3, &encode_angle(new.2))?;

    // Reference/value text on the instance (fields 7/8).
    for field in [7u32, 8u32] {
        value = map_len_fields(&value, field, |f| {
            patch_field_text_pos(f, old_anchor, new_anchor, delta_deg)
        })?;
    }

    // Definition: nested pads + its own reference/value fields.
    value = map_len_fields(&value, 6, |def| {
        let mut def = map_len_fields(def, 11, |item_bytes| {
            let item = match Any::decode(item_bytes) {
                Ok(item) => item,
                Err(_) => return Ok(item_bytes.to_vec()),
            };
            let Some(pad) = decode_pad(&item) else {
                return Ok(item_bytes.to_vec());
            };
            let pos = pad.position.unwrap_or_default();
            let (nx, ny) = rigid_xy(
                nm_to_mm(pos.x_nm),
                nm_to_mm(pos.y_nm),
                old_anchor,
                new_anchor,
                delta_deg,
            );
            let mut pad_value = set_len_field(&item.value, 7, &encode_vec2(nx, ny))?;
            if delta_deg.abs() > 1e-9 {
                let new_angle = pad
                    .pad_stack
                    .as_ref()
                    .and_then(|s| s.angle.as_ref())
                    .map(|a| a.value_degrees)
                    .unwrap_or(0.0)
                    + delta_deg;
                pad_value = map_len_fields(&pad_value, 6, |stack| {
                    set_len_field(stack, 6, &encode_angle(new_angle))
                })?;
            }
            pads_moved.set(pads_moved.get() + 1);
            let patched = Any {
                type_url: if item.type_url.is_empty() {
                    TYPE_PAD.into()
                } else {
                    item.type_url
                },
                value: pad_value,
            };
            Ok(patched.encode_to_vec())
        })?;
        for field in [7u32, 8u32] {
            def = map_len_fields(&def, field, |f| {
                patch_field_text_pos(f, old_anchor, new_anchor, delta_deg)
            })?;
        }
        Ok(def)
    })?;

    if pads_moved.get() == 0 {
        return Err("footprint proto has no pads — refusing a move that would strand copper".into());
    }
    Ok((
        Any {
            type_url: fp.type_url.clone(),
            value,
        },
        pads_moved.get(),
    ))
}

/// Field (tag 3 BoardText → tag 2 Text → tag 2 position): ride the text
/// along with the part so the silk refdes does not stay behind.
fn patch_field_text_pos(
    field_bytes: &[u8],
    old_anchor: (f64, f64),
    new_anchor: (f64, f64),
    delta_deg: f64,
) -> Result<Vec<u8>, String> {
    map_len_fields(field_bytes, 3, |board_text| {
        map_len_fields(board_text, 2, |text| {
            let decoded = TextDec::decode(text).unwrap_or_default();
            let Some(pos) = decoded.position else {
                return Ok(text.to_vec());
            };
            let (nx, ny) = rigid_xy(
                nm_to_mm(pos.x_nm),
                nm_to_mm(pos.y_nm),
                old_anchor,
                new_anchor,
                delta_deg,
            );
            set_len_field(text, 2, &encode_vec2(nx, ny))
        })
    })
}

/// Move / rotate one placed footprint. Courtyard-checked at the target,
/// nets untouched, one undo. Copper does not move.
pub async fn move_footprint(
    k: &Kicad,
    reference: &str,
    x_mm: f64,
    y_mm: f64,
    rotation_deg: Option<f64>,
) -> Result<serde_json::Value, String> {
    let fps = k.footprints().await?;
    let me = fps
        .iter()
        .find(|f| f.reference.as_deref() == Some(reference))
        .ok_or_else(|| format!("{reference} is not on the board"))?;
    let id = me
        .id
        .clone()
        .ok_or_else(|| format!("{reference} has no KIID over IPC"))?;
    let old = (
        me.x_mm.unwrap_or(0.0),
        me.y_mm.unwrap_or(0.0),
        me.rotation_deg.unwrap_or(0.0),
    );
    let new_rot = rotation_deg.unwrap_or(old.2);

    // Courtyard check at the target, against everything except this part.
    let dir = k.project_dir().await?;
    let pretty = crate::kicad::jlc_pretty_dir(&dir);
    let mut note = String::new();
    match me
        .value
        .as_deref()
        .and_then(|t| crate::place::courtyard_of_template(&pretty, t))
    {
        Some(local) => {
            let new_box = crate::place::aabb_at(&local, x_mm, y_mm, new_rot);
            for other in &fps {
                if other.reference.as_deref() == Some(reference) {
                    continue;
                }
                let Some(tmpl) = other.value.as_deref() else {
                    continue;
                };
                let Some(other_local) = crate::place::courtyard_of_template(&pretty, tmpl) else {
                    continue;
                };
                let other_box = crate::place::aabb_at(
                    &other_local,
                    other.x_mm.unwrap_or(0.0),
                    other.y_mm.unwrap_or(0.0),
                    other.rotation_deg.unwrap_or(0.0),
                );
                if new_box.overlaps(&other_box, 0.0) {
                    return Err(format!(
                        "courtyard of {reference} at ({x_mm:.2}, {y_mm:.2}) overlaps {} — pick free space (F.CrtYd)",
                        other.reference.as_deref().unwrap_or(tmpl)
                    ));
                }
            }
        }
        None => {
            note.push_str("no courtyard template found — overlap not checked. ");
        }
    }

    let raws = k
        .raw_items(vec![kicad_ipc_rs::PcbObjectTypeCode::new_footprint().code])
        .await?;
    let raw = raws
        .iter()
        .find(|r| {
            FpHead::decode(r.value.as_slice())
                .ok()
                .and_then(|h| h.id)
                .map(|k| k.value == id)
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("{reference}: raw footprint proto not found over IPC"))?;

    let (patched, pads_moved) = transform_footprint_any(raw, old, (x_mm, y_mm, new_rot))?;

    let session = k.begin_commit().await?;
    match k.update_items(vec![patched]).await {
        Ok(_) => {
            k.end_commit(session, &format!("kicad-mcp move {reference}"))
                .await?;
            let _ = k.refresh().await;
            note.push_str("Copper does not move — re-route tracks that reached this part.");
            Ok(serde_json::json!({
                "ok": true,
                "reference": reference,
                "from": { "x_mm": old.0, "y_mm": old.1, "rotation_deg": old.2 },
                "to": { "x_mm": x_mm, "y_mm": y_mm, "rotation_deg": new_rot },
                "pads_moved": pads_moved,
                "note": note,
            }))
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            Err(e)
        }
    }
}

fn decode_pad(item: &Any) -> Option<PadDec> {
    if !item.type_url.is_empty() && !item.type_url.contains("Pad") {
        return None;
    }
    PadDec::decode(item.value.as_slice()).ok()
}

fn encode_vec2(x_mm: f64, y_mm: f64) -> Vec<u8> {
    Vec2Dec {
        x_nm: mm_to_nm(x_mm),
        y_nm: mm_to_nm(y_mm),
    }
    .encode_to_vec()
}

fn encode_angle(deg: f64) -> Vec<u8> {
    AngleDec { value_degrees: deg }.encode_to_vec()
}

// --- decode mirrors of the kiapi.board.types wire format (see place.rs) ---

#[derive(Clone, PartialEq, Message)]
struct KiidDec {
    #[prost(string, tag = "1")]
    value: String,
}

#[derive(Clone, PartialEq, Message)]
struct Vec2Dec {
    #[prost(int64, tag = "1")]
    x_nm: i64,
    #[prost(int64, tag = "2")]
    y_nm: i64,
}

#[derive(Clone, PartialEq, Message)]
struct AngleDec {
    #[prost(double, tag = "1")]
    value_degrees: f64,
}

#[derive(Clone, PartialEq, Message)]
struct DrillDec {
    #[prost(message, optional, tag = "3")]
    diameter: Option<Vec2Dec>,
}

#[derive(Clone, PartialEq, Message)]
struct PadStackLayerDec {
    #[prost(int32, tag = "1")]
    layer: i32,
    #[prost(int32, tag = "2")]
    shape: i32,
    #[prost(message, optional, tag = "3")]
    size: Option<Vec2Dec>,
}

#[derive(Clone, PartialEq, Message)]
struct PadStackDec {
    #[prost(message, optional, tag = "3")]
    drill: Option<DrillDec>,
    #[prost(message, repeated, tag = "5")]
    copper_layers: Vec<PadStackLayerDec>,
    #[prost(message, optional, tag = "6")]
    angle: Option<AngleDec>,
}

/// `net` (field 4) is deliberately not decoded — a shape mismatch there
/// would fail the whole pad (see nets.rs `PadHead`). Nets come from the
/// official pad netlist, joined by KIID.
#[derive(Clone, PartialEq, Message)]
struct PadDec {
    #[prost(message, optional, tag = "1")]
    id: Option<KiidDec>,
    #[prost(string, tag = "3")]
    number: String,
    #[prost(int32, tag = "5")]
    r#type: i32,
    #[prost(message, optional, tag = "6")]
    pad_stack: Option<PadStackDec>,
    #[prost(message, optional, tag = "7")]
    position: Option<Vec2Dec>,
}

#[derive(Clone, PartialEq, Message)]
struct FpDefDec {
    #[prost(message, repeated, tag = "11")]
    items: Vec<Any>,
}

#[derive(Clone, PartialEq, Message)]
struct FpInstDec {
    #[prost(message, optional, tag = "1")]
    id: Option<KiidDec>,
    #[prost(message, optional, tag = "2")]
    position: Option<Vec2Dec>,
    #[prost(message, optional, tag = "3")]
    orientation: Option<AngleDec>,
    #[prost(message, optional, tag = "6")]
    definition: Option<FpDefDec>,
}

#[derive(Clone, PartialEq, Message)]
struct FpHead {
    #[prost(message, optional, tag = "1")]
    id: Option<KiidDec>,
}

#[derive(Clone, PartialEq, Message)]
struct TextDec {
    #[prost(message, optional, tag = "2")]
    position: Option<Vec2Dec>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::{footprint_instance_any, ModPad, ModPadKind, ModPadShape, PlaceSpec};

    fn two_pads() -> Vec<ModPad> {
        let pad = |n: &str, px: f64| ModPad {
            number: n.into(),
            kind: ModPadKind::SmdFront,
            shape: ModPadShape::Rect,
            x_mm: px,
            y_mm: 0.0,
            rot_deg: 0.0,
            width_mm: 0.8,
            height_mm: 0.9,
            drill_mm: None,
        };
        vec![pad("1", -0.75), pad("2", 0.75)]
    }

    fn instance(x: f64, y: f64, rot: f64, pads: &[ModPad]) -> Any {
        footprint_instance_any(&PlaceSpec {
            template: "T",
            reference: "R1",
            x_mm: x,
            y_mm: y,
            rotation_deg: rot,
            pads,
        })
        .unwrap()
    }

    fn decoded_pads(any: &Any) -> Vec<(String, f64, f64)> {
        let inst = FpInstDec::decode(any.value.as_slice()).unwrap();
        inst.definition
            .unwrap()
            .items
            .iter()
            .filter_map(decode_pad)
            .map(|p| {
                let pos = p.position.unwrap();
                (p.number, nm_to_mm(pos.x_nm), nm_to_mm(pos.y_nm))
            })
            .collect()
    }

    #[test]
    fn decodes_baked_pad_positions() {
        let pads = two_pads();
        let any = instance(100.0, 80.0, 0.0, &pads);
        let rows = decoded_pads(&any);
        assert_eq!(rows.len(), 2);
        let p1 = rows.iter().find(|r| r.0 == "1").unwrap();
        assert!((p1.1 - 99.25).abs() < 1e-6);
        assert!((p1.2 - 80.0).abs() < 1e-6);
    }

    #[test]
    fn rigid_translation_keeps_offsets() {
        let (x, y) = rigid_xy(101.0, 80.5, (100.0, 80.0), (140.0, 60.0), 0.0);
        assert!((x - 141.0).abs() < 1e-9);
        assert!((y - 60.5).abs() < 1e-9);
    }

    /// Same rotation-sense guard as place::world_xy: +90° is visually CCW,
    /// so a pad east of the anchor must land north (smaller y, y-down frame).
    #[test]
    fn rigid_plus_90_is_counterclockwise_on_screen() {
        let (x, y) = rigid_xy(103.0, 80.0, (100.0, 80.0), (100.0, 80.0), 90.0);
        assert!((x - 100.0).abs() < 1e-9);
        assert!((y - 77.0).abs() < 1e-9, "east pad must land north, got y={y}");
    }

    #[test]
    fn transform_moves_anchor_and_pads_together() {
        let pads = two_pads();
        let any = instance(100.0, 80.0, 0.0, &pads);
        let (patched, moved) =
            transform_footprint_any(&any, (100.0, 80.0, 0.0), (150.0, 90.0, 0.0)).unwrap();
        assert_eq!(moved, 2);
        let inst = FpInstDec::decode(patched.value.as_slice()).unwrap();
        let anchor = inst.position.unwrap();
        assert!((nm_to_mm(anchor.x_nm) - 150.0).abs() < 1e-6);
        assert!((nm_to_mm(anchor.y_nm) - 90.0).abs() < 1e-6);
        let rows = decoded_pads(&patched);
        let p2 = rows.iter().find(|r| r.0 == "2").unwrap();
        assert!((p2.1 - 150.75).abs() < 1e-6);
        assert!((p2.2 - 90.0).abs() < 1e-6);
    }

    #[test]
    fn transform_rotates_pads_ccw_and_sets_orientation() {
        let pads = two_pads();
        let any = instance(100.0, 80.0, 0.0, &pads);
        let (patched, _) =
            transform_footprint_any(&any, (100.0, 80.0, 0.0), (100.0, 80.0, 90.0)).unwrap();
        let inst = FpInstDec::decode(patched.value.as_slice()).unwrap();
        assert!((inst.orientation.unwrap().value_degrees - 90.0).abs() < 1e-9);
        let rows = decoded_pads(&patched);
        // Pad 2 sat east of the anchor; after +90° (visually CCW) it is north.
        let p2 = rows.iter().find(|r| r.0 == "2").unwrap();
        assert!((p2.1 - 100.0).abs() < 1e-6);
        assert!((p2.2 - 79.25).abs() < 1e-6, "expected north of anchor, got y={}", p2.2);
        // Pad angle followed the delta.
        let def = FpInstDec::decode(patched.value.as_slice())
            .unwrap()
            .definition
            .unwrap();
        let angles: Vec<f64> = def
            .items
            .iter()
            .filter_map(decode_pad)
            .filter_map(|p| p.pad_stack.and_then(|s| s.angle).map(|a| a.value_degrees))
            .collect();
        assert!(angles.iter().all(|a| (a - 90.0).abs() < 1e-9));
    }

    #[test]
    fn transform_matches_fresh_placement() {
        // Moving 0°→90° must land pads exactly where placing at 90° would.
        let pads = two_pads();
        let placed_at_0 = instance(100.0, 80.0, 0.0, &pads);
        let placed_at_90 = instance(120.0, 70.0, 90.0, &pads);
        let (moved, _) =
            transform_footprint_any(&placed_at_0, (100.0, 80.0, 0.0), (120.0, 70.0, 90.0))
                .unwrap();
        let a = decoded_pads(&moved);
        let b = decoded_pads(&placed_at_90);
        for (pa, pb) in a.iter().zip(b.iter()) {
            assert_eq!(pa.0, pb.0);
            assert!((pa.1 - pb.1).abs() < 1e-6, "{} x {} vs {}", pa.0, pa.1, pb.1);
            assert!((pa.2 - pb.2).abs() < 1e-6, "{} y {} vs {}", pa.0, pa.2, pb.2);
        }
    }

    #[test]
    fn transform_preserves_pad_numbers_and_sizes() {
        let pads = two_pads();
        let any = instance(100.0, 80.0, 0.0, &pads);
        let (patched, _) =
            transform_footprint_any(&any, (100.0, 80.0, 0.0), (150.0, 90.0, 90.0)).unwrap();
        let inst = FpInstDec::decode(patched.value.as_slice()).unwrap();
        for item in &inst.definition.unwrap().items {
            let pad = decode_pad(item).unwrap();
            assert!(pad.number == "1" || pad.number == "2");
            let size = pad.pad_stack.unwrap().copper_layers[0].size.clone().unwrap();
            assert!((nm_to_mm(size.x_nm) - 0.8).abs() < 1e-6);
            assert!((nm_to_mm(size.y_nm) - 0.9).abs() < 1e-6);
        }
    }
}
