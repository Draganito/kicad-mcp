//! Edge.Cuts rectangle via typed `CreateItems` (KiCad 9 has no paste-from-sexpr).
//!
//! That rectangle **is** the PCB size. The pink A4 frame in the GUI is the
//! drawing sheet, not the board. Default origin (when the MCP omits it) is
//! the centre of that sheet (297 × 210 mm), not 0,0. `set_board_outline`
//! deletes existing Edge.Cuts when `replace` is true (the default).

use prost::Message;
use prost_types::Any;

use crate::place::mm_to_nm;

pub(crate) const BL_EDGE_CUTS: i32 = 47;
const LS_UNLOCKED: i32 = 1;
const SLS_SOLID: i32 = 2;
const GFT_UNFILLED: i32 = 1;
/// KiCad default Edge.Cuts stroke.
const STROKE_NM: i64 = 50_000;

const TYPE_BOARD_GRAPHIC: &str = "type.googleapis.com/kiapi.board.types.BoardGraphicShape";

/// Four Edge.Cuts segments: closed rectangle, origin = bottom-left, +y up.
pub fn rect_edge_cuts(
    origin_x_mm: f64,
    origin_y_mm: f64,
    width_mm: f64,
    height_mm: f64,
) -> Result<Vec<Any>, String> {
    if width_mm < 5.0 || height_mm < 5.0 {
        return Err("board outline must be at least 5 × 5 mm".into());
    }
    if width_mm > 400.0 || height_mm > 400.0 {
        return Err("board outline max 400 × 400 mm (JLCPCB sheet limit)".into());
    }
    let x0 = mm_to_nm(origin_x_mm);
    let y0 = mm_to_nm(origin_y_mm);
    let x1 = mm_to_nm(origin_x_mm + width_mm);
    let y1 = mm_to_nm(origin_y_mm + height_mm);
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)];
    Ok(corners
        .windows(2)
        .map(|w| segment(w[0].0, w[0].1, w[1].0, w[1].1))
        .collect())
}

/// Closed Edge.Cuts polygon in KiCad millimetres (already translated to the sheet).
pub fn poly_edge_cuts(points: &[(f64, f64)]) -> Result<Vec<Any>, String> {
    if points.len() < 3 {
        return Err("board polygon needs at least 3 points".into());
    }
    if points.len() > 400 {
        return Err(format!(
            "board polygon max 400 points (got {})",
            points.len()
        ));
    }
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let width = max_x - min_x;
    let height = max_y - min_y;
    if width < 5.0 || height < 5.0 {
        return Err("board outline must be at least 5 × 5 mm".into());
    }
    if width > 400.0 || height > 400.0 {
        return Err("board outline max 400 × 400 mm (JLCPCB sheet limit)".into());
    }
    let mut ring: Vec<(i64, i64)> = points
        .iter()
        .map(|(x, y)| (mm_to_nm(*x), mm_to_nm(*y)))
        .collect();
    if ring.first() != ring.last() {
        ring.push(ring[0]);
    }
    Ok(ring
        .windows(2)
        .map(|w| segment(w[0].0, w[0].1, w[1].0, w[1].1))
        .collect())
}

fn segment(x0: i64, y0: i64, x1: i64, y1: i64) -> Any {
    let shape = GraphicShape {
        attributes: Some(GraphicAttributes {
            stroke: Some(StrokeAttributes {
                width: Some(Distance { value_nm: STROKE_NM }),
                style: SLS_SOLID,
            }),
            fill: Some(GraphicFillAttributes {
                fill_type: GFT_UNFILLED,
            }),
        }),
        geometry: Some(graphic_shape::Geometry::Segment(GraphicSegmentAttributes {
            start: Some(Vector2 { x_nm: x0, y_nm: y0 }),
            end: Some(Vector2 { x_nm: x1, y_nm: y1 }),
        })),
    };
    let item = BoardGraphicShape {
        shape: Some(shape),
        layer: BL_EDGE_CUTS,
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
struct GraphicSegmentAttributes {
    #[prost(message, optional, tag = "1")]
    start: Option<Vector2>,
    #[prost(message, optional, tag = "2")]
    end: Option<Vector2>,
}

#[derive(Clone, PartialEq, Message)]
struct GraphicShape {
    #[prost(message, optional, tag = "3")]
    attributes: Option<GraphicAttributes>,
    #[prost(oneof = "graphic_shape::Geometry", tags = "4")]
    geometry: Option<graphic_shape::Geometry>,
}

mod graphic_shape {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Geometry {
        #[prost(message, tag = "4")]
        Segment(super::GraphicSegmentAttributes),
    }
}

#[derive(Clone, PartialEq, Message)]
struct BoardGraphicShape {
    #[prost(message, optional, tag = "1")]
    shape: Option<GraphicShape>,
    #[prost(int32, tag = "2")]
    layer: i32,
    #[prost(int32, tag = "5")]
    locked: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_segments() {
        let items = rect_edge_cuts(0.0, 0.0, 40.0, 30.0).unwrap();
        assert_eq!(items.len(), 4);
        assert!(items[0].type_url.contains("BoardGraphicShape"));
    }

    #[test]
    fn polygon_closes() {
        let items = poly_edge_cuts(&[(0.0, 0.0), (40.0, 0.0), (20.0, 30.0)]).unwrap();
        assert_eq!(items.len(), 3);
    }
}
