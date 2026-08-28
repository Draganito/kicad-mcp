//! Board silkscreen text via typed `CreateItems` (same path as kicad-python `BoardText`).
//!
//! Only F.Silkscreen / B.Silkscreen. Copper and Fab are refused — connector
//! labels must not become nets, and footprint Value is not this tool.

use prost::Message;
use prost_types::Any;

use crate::place::mm_to_nm;

const BL_B_SILKS: i32 = 39;
const BL_F_SILKS: i32 = 40;
const LS_UNLOCKED: i32 = 1;
const HA_CENTER: i32 = 2;
const VA_CENTER: i32 = 2;

const TYPE_BOARD_TEXT: &str = "type.googleapis.com/kiapi.board.types.BoardText";

/// Default KiCad silk height (JLCPCB DFM floor is ~0.8 mm).
const DEFAULT_SIZE_MM: f64 = 1.0;
const MIN_SIZE_MM: f64 = 0.8;
const MAX_SIZE_MM: f64 = 8.0;
const MAX_CHARS: usize = 80;
/// Max labels in one `add_texts` undo.
pub const SILK_MAX: usize = 150;
/// Stroke = 15 % of height (1.0 mm → 0.15 mm, KiCad default).
const STROKE_RATIO: f64 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SilkLayer {
    pub id: i32,
    pub name: &'static str,
    pub mirrored: bool,
}

pub fn parse_silk_layer(name: Option<&str>) -> Result<SilkLayer, String> {
    let raw = name.unwrap_or("F.Silkscreen").trim();
    let n = raw.replace('_', ".").to_ascii_lowercase();
    let n = n.strip_prefix("bl.").unwrap_or(&n);
    match n {
        "f.silkscreen" | "f.silks" | "f.silk" => Ok(SilkLayer {
            id: BL_F_SILKS,
            name: "F.Silkscreen",
            mirrored: false,
        }),
        "b.silkscreen" | "b.silks" | "b.silk" => Ok(SilkLayer {
            id: BL_B_SILKS,
            name: "B.Silkscreen",
            mirrored: true,
        }),
        other if other.contains(".cu") || other.ends_with("cu") => Err(format!(
            "add_text is silkscreen only — use F.Silkscreen or B.Silkscreen, not copper (got {raw})"
        )),
        _ => Err(format!(
            "silk layer must be F.Silkscreen or B.Silkscreen (got {raw})"
        )),
    }
}

pub fn text_any(
    text: &str,
    x_mm: f64,
    y_mm: f64,
    layer: Option<&str>,
    size_mm: Option<f64>,
    rotation_deg: Option<f64>,
) -> Result<Any, String> {
    let body = sanitize_text(text)?;
    if !x_mm.is_finite() || !y_mm.is_finite() {
        return Err("x_mm and y_mm must be finite millimetres".into());
    }
    let size = size_mm.unwrap_or(DEFAULT_SIZE_MM);
    if !size.is_finite() || size < MIN_SIZE_MM || size > MAX_SIZE_MM {
        return Err(format!(
            "size_mm must be {MIN_SIZE_MM}–{MAX_SIZE_MM} (JLCPCB silk floor {MIN_SIZE_MM} mm, got {size})"
        ));
    }
    let rot = rotation_deg.unwrap_or(0.0);
    if !rot.is_finite() {
        return Err("rotation_deg must be finite".into());
    }
    let silk = parse_silk_layer(layer)?;
    let size_nm = mm_to_nm(size);
    let stroke_nm = mm_to_nm(size * STROKE_RATIO);
    let proto = BoardText {
        text: Some(Text {
            position: Some(Vector2 {
                x_nm: mm_to_nm(x_mm),
                y_nm: mm_to_nm(y_mm),
            }),
            attributes: Some(TextAttributes {
                horizontal_alignment: HA_CENTER,
                vertical_alignment: VA_CENTER,
                angle: (rot.abs() > 0.01).then_some(Angle {
                    value_degrees: rot,
                }),
                line_spacing: 1.0,
                stroke_width: Some(Distance { value_nm: stroke_nm }),
                visible: true,
                mirrored: silk.mirrored,
                size: Some(Vector2 {
                    x_nm: size_nm,
                    y_nm: size_nm,
                }),
                ..Default::default()
            }),
            text: body,
            ..Default::default()
        }),
        layer: silk.id,
        knockout: false,
        locked: LS_UNLOCKED,
    };
    Ok(Any {
        type_url: TYPE_BOARD_TEXT.into(),
        value: proto.encode_to_vec(),
    })
}

fn sanitize_text(text: &str) -> Result<String, String> {
    let body = text.trim();
    if body.is_empty() {
        return Err("text is empty".into());
    }
    if body.contains('\n') || body.contains('\r') {
        return Err("text must be a single line (no newline)".into());
    }
    if body.chars().count() > MAX_CHARS {
        return Err(format!("text max {MAX_CHARS} characters"));
    }
    Ok(body.to_string())
}

#[derive(Clone, PartialEq, Message)]
struct Vector2 {
    #[prost(int64, tag = "1")]
    x_nm: i64,
    #[prost(int64, tag = "2")]
    y_nm: i64,
}

#[derive(Clone, PartialEq, Message)]
struct Angle {
    #[prost(double, tag = "1")]
    value_degrees: f64,
}

#[derive(Clone, PartialEq, Message)]
struct Distance {
    #[prost(int64, tag = "1")]
    value_nm: i64,
}

#[derive(Clone, PartialEq, Message)]
struct TextAttributes {
    #[prost(string, tag = "1")]
    font_name: String,
    #[prost(int32, tag = "2")]
    horizontal_alignment: i32,
    #[prost(int32, tag = "3")]
    vertical_alignment: i32,
    #[prost(message, optional, tag = "4")]
    angle: Option<Angle>,
    #[prost(double, tag = "5")]
    line_spacing: f64,
    #[prost(message, optional, tag = "6")]
    stroke_width: Option<Distance>,
    #[prost(bool, tag = "7")]
    italic: bool,
    #[prost(bool, tag = "8")]
    bold: bool,
    #[prost(bool, tag = "9")]
    underlined: bool,
    #[prost(bool, tag = "10")]
    visible: bool,
    #[prost(bool, tag = "11")]
    mirrored: bool,
    #[prost(bool, tag = "12")]
    multiline: bool,
    #[prost(bool, tag = "13")]
    keep_upright: bool,
    #[prost(message, optional, tag = "14")]
    size: Option<Vector2>,
}

#[derive(Clone, PartialEq, Message)]
struct Text {
    #[prost(message, optional, tag = "2")]
    position: Option<Vector2>,
    #[prost(message, optional, tag = "3")]
    attributes: Option<TextAttributes>,
    #[prost(string, tag = "5")]
    text: String,
    #[prost(string, tag = "6")]
    hyperlink: String,
}

#[derive(Clone, PartialEq, Message)]
struct BoardText {
    #[prost(message, optional, tag = "2")]
    text: Option<Text>,
    #[prost(int32, tag = "3")]
    layer: i32,
    #[prost(bool, tag = "4")]
    knockout: bool,
    #[prost(int32, tag = "5")]
    locked: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layer_is_front_silk() {
        let layer = parse_silk_layer(None).unwrap();
        assert_eq!(layer.id, BL_F_SILKS);
        assert!(!layer.mirrored);
    }

    #[test]
    fn accepts_kicad_silk_aliases() {
        assert_eq!(parse_silk_layer(Some("F.SilkS")).unwrap().id, BL_F_SILKS);
        assert_eq!(parse_silk_layer(Some("BL_F_SilkS")).unwrap().id, BL_F_SILKS);
        let back = parse_silk_layer(Some("B.Silkscreen")).unwrap();
        assert_eq!(back.id, BL_B_SILKS);
        assert!(back.mirrored);
    }

    #[test]
    fn refuses_copper() {
        let err = parse_silk_layer(Some("F.Cu")).unwrap_err();
        assert!(err.contains("silkscreen only"), "{err}");
    }

    #[test]
    fn encodes_visible_centered_label() {
        let any = text_any("5V", 10.0, 20.0, None, None, None).unwrap();
        assert!(any.type_url.contains("BoardText"));
        let proto = BoardText::decode(any.value.as_slice()).unwrap();
        assert_eq!(proto.layer, BL_F_SILKS);
        assert!(!proto.knockout);
        let text = proto.text.unwrap();
        assert_eq!(text.text, "5V");
        assert_eq!(
            text.position.map(|p| (p.x_nm, p.y_nm)),
            Some((10_000_000, 20_000_000))
        );
        let attr = text.attributes.unwrap();
        assert!(attr.visible);
        assert!(!attr.mirrored);
        assert_eq!(attr.horizontal_alignment, HA_CENTER);
        assert_eq!(attr.size.unwrap().x_nm, 1_000_000);
        assert_eq!(attr.stroke_width.unwrap().value_nm, 150_000);
        assert!(attr.angle.is_none());
    }

    #[test]
    fn back_silk_is_mirrored() {
        let any = text_any("GND", 0.0, 0.0, Some("B.Silkscreen"), Some(1.5), Some(90.0)).unwrap();
        let proto = BoardText::decode(any.value.as_slice()).unwrap();
        assert_eq!(proto.layer, BL_B_SILKS);
        let attr = proto.text.unwrap().attributes.unwrap();
        assert!(attr.mirrored);
        assert_eq!(attr.size.unwrap().x_nm, 1_500_000);
        assert_eq!(attr.angle.unwrap().value_degrees, 90.0);
    }

    #[test]
    fn rejects_empty_and_tiny_and_copper() {
        assert!(text_any("  ", 0.0, 0.0, None, None, None)
            .unwrap_err()
            .contains("empty"));
        assert!(text_any("5V", 0.0, 0.0, None, Some(0.3), None)
            .unwrap_err()
            .contains("0.8"));
        assert!(text_any("5V", 0.0, 0.0, Some("F.Cu"), None, None)
            .unwrap_err()
            .contains("silkscreen"));
    }
}
