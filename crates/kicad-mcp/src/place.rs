//! Build a KiCad `FootprintInstance` protobuf for `CreateItems`.
//!
//! KiCad 9.0.2 returns `IRS_UNKNOWN` for `ParseAndCreateItemsFromString`
//! (clipboard paste). Typed `CreateItems` works — same path as kicad-python
//! `BoardText` / footprint create.
//!
//! Nested pads are **not** moved with `FootprintInstance.position`. Always
//! bake board millimetres via [`world_xy`] or copper appears at 0,0.

use prost::Message;
use prost_types::Any;

const NM_PER_MM: f64 = 1_000_000.0;
/// Max cells for `place_matrix` / parts for `place_parts` / pairs for `connect_many`.
pub const PLACE_MAX: usize = 150;

const BL_F_CU: i32 = 3;
const BL_B_CU: i32 = 34;
const BL_B_PASTE: i32 = 37;
const BL_F_PASTE: i32 = 38;
const BL_B_MASK: i32 = 41;
const BL_F_MASK: i32 = 42;
const BL_F_SILKS: i32 = 40;
const BL_F_FAB: i32 = 52;

const LS_UNLOCKED: i32 = 1;
const PST_NORMAL: i32 = 1;
const PSS_CIRCLE: i32 = 1;
const PSS_RECTANGLE: i32 = 2;
const PSS_OVAL: i32 = 3;
const PT_PTH: i32 = 1;
const PT_SMD: i32 = 2;
const PT_NPTH: i32 = 4;
// kiapi.board.types.DrillShape
const DRILL_CIRCLE: i32 = 1;
const DRILL_OBLONG: i32 = 2;
const FMS_THROUGH_HOLE: i32 = 1;
const FMS_SMD: i32 = 2;
const HA_CENTER: i32 = 2;
const VA_CENTER: i32 = 2;

const TYPE_FOOTPRINT_INSTANCE: &str = "type.googleapis.com/kiapi.board.types.FootprintInstance";
const TYPE_PAD: &str = "type.googleapis.com/kiapi.board.types.Pad";

pub const JLC_LIBRARY: &str = "jlcpcb_parts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModPadKind {
    SmdFront,
    SmdBack,
    ThruHole,
    Npth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModPadShape {
    Rect,
    Oval,
    Circle,
}

#[derive(Debug, Clone)]
pub struct ModPad {
    pub number: String,
    pub kind: ModPadKind,
    pub shape: ModPadShape,
    pub x_mm: f64,
    pub y_mm: f64,
    pub rot_deg: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Drill diameter (x). For oblong drills this is the slot width.
    pub drill_mm: Option<f64>,
    /// Slot length (drill y) for oblong drills, e.g. USB shield slots.
    /// None means a round hole.
    pub drill_h_mm: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct PlaceSpec<'a> {
    pub template: &'a str,
    pub reference: &'a str,
    pub x_mm: f64,
    pub y_mm: f64,
    pub rotation_deg: f64,
    pub pads: &'a [ModPad],
}

pub fn mm_to_nm(mm: f64) -> i64 {
    (mm * NM_PER_MM).round() as i64
}

/// KiCad 9 `CreateItems` stores `FootprintInstance.position` (the anchor
/// `get_footprints` reports) but draws nested pads at their raw coordinates —
/// it does not parent-transform them. Bake board millimetres into each pad
/// or the copper sits on the page origin (0,0) while the anchor is elsewhere.
/// KiCad angles are positive **counterclockwise as displayed**, but the
/// internal frame is y-down. In that frame a visually-CCW rotation is
/// `(x·cos + y·sin, −x·sin + y·cos)` — the inverse of the y-up textbook
/// matrix. Using the textbook matrix here bakes pads clockwise while the
/// orientation field (and the CPL export JLCPCB assembles from) claims
/// counterclockwise: every ±90° part would be soldered 180° off.
pub(crate) fn world_xy(local_x: f64, local_y: f64, spec: &PlaceSpec<'_>) -> (f64, f64) {
    let rad = spec.rotation_deg.to_radians();
    let (s, c) = (rad.sin(), rad.cos());
    (
        spec.x_mm + c * local_x + s * local_y,
        spec.y_mm - s * local_x + c * local_y,
    )
}

pub fn footprint_instance_any(spec: &PlaceSpec<'_>) -> Result<Any, String> {
    if spec.pads.is_empty() {
        return Err("footprint has no pads — the .kicad_mod is empty or unreadable".into());
    }
    let smd = spec
        .pads
        .iter()
        .any(|p| matches!(p.kind, ModPadKind::SmdFront | ModPadKind::SmdBack));
    let pads_world: Vec<ModPad> = spec
        .pads
        .iter()
        .map(|p| {
            let (x_mm, y_mm) = world_xy(p.x_mm, p.y_mm, spec);
            ModPad {
                x_mm,
                y_mm,
                rot_deg: p.rot_deg + spec.rotation_deg,
                ..p.clone()
            }
        })
        .collect();
    let (ref_x, ref_y) = world_xy(0.0, -2.0, spec);
    let (val_x, val_y) = world_xy(0.0, 2.0, spec);
    let inst = FootprintInstance {
        position: Some(Vector2 {
            x_nm: mm_to_nm(spec.x_mm),
            y_nm: mm_to_nm(spec.y_mm),
        }),
        orientation: Some(Angle {
            value_degrees: spec.rotation_deg,
        }),
        layer: BL_F_CU,
        locked: LS_UNLOCKED,
        definition: Some(Footprint {
            id: Some(LibraryIdentifier {
                library_nickname: JLC_LIBRARY.into(),
                entry_name: spec.template.into(),
            }),
            attributes: Some(FootprintAttributes {
                mounting_style: if smd { FMS_SMD } else { FMS_THROUGH_HOLE },
                ..Default::default()
            }),
            reference_field: Some(field(
                "Reference",
                spec.reference,
                ref_x,
                ref_y,
                BL_F_SILKS,
                true,
            )),
            value_field: Some(field("Value", spec.template, val_x, val_y, BL_F_FAB, true)),
            items: pads_world.iter().map(pad_any).collect(),
            ..Default::default()
        }),
        reference_field: Some(field(
            "Reference",
            spec.reference,
            ref_x,
            ref_y,
            BL_F_SILKS,
            true,
        )),
        value_field: Some(field("Value", spec.template, val_x, val_y, BL_F_FAB, true)),
        ..Default::default()
    };
    Ok(pack(&inst, TYPE_FOOTPRINT_INSTANCE))
}

pub fn parse_kicad_mod_pads(mod_text: &str) -> Result<Vec<ModPad>, String> {
    let mut pads = Vec::new();
    let mut rest = mod_text;
    while let Some(idx) = rest.find("(pad ") {
        let chunk = &rest[idx..];
        let end = matching_paren(chunk)
            .ok_or_else(|| "unterminated (pad …) in .kicad_mod".to_string())?;
        let pad_sexpr = &chunk[..=end];
        pads.push(parse_one_pad(pad_sexpr)?);
        rest = &chunk[end + 1..];
    }
    if pads.is_empty() {
        return Err("no (pad …) entries in .kicad_mod".into());
    }
    Ok(pads)
}

/// Axis-aligned box in footprint-local millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Aabb {
    pub fn overlaps(&self, other: &Aabb, gap_mm: f64) -> bool {
        self.min_x < other.max_x + gap_mm
            && self.max_x + gap_mm > other.min_x
            && self.min_y < other.max_y + gap_mm
            && self.max_y + gap_mm > other.min_y
    }
}

/// F.CrtYd rectangle from `(fp_line … (layer "F.CrtYd"))`. None if the mod has no courtyard.
pub fn parse_kicad_mod_courtyard(mod_text: &str) -> Option<Aabb> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut rest = mod_text;
    while let Some(idx) = rest.find("(fp_line ") {
        let chunk = &rest[idx..];
        let end = matching_paren(chunk)?;
        let line = &chunk[..=end];
        rest = &chunk[end + 1..];
        if !line.contains("CrtYd") {
            continue;
        }
        if let Some(s) = tuple_after(line, "(start ") {
            if s.len() >= 2 {
                xs.push(s[0]);
                ys.push(s[1]);
            }
        }
        if let Some(e) = tuple_after(line, "(end ") {
            if e.len() >= 2 {
                xs.push(e[0]);
                ys.push(e[1]);
            }
        }
    }
    if xs.is_empty() {
        return None;
    }
    Some(Aabb {
        min_x: xs.iter().copied().fold(f64::INFINITY, f64::min),
        min_y: ys.iter().copied().fold(f64::INFINITY, f64::min),
        max_x: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        max_y: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    })
}

pub fn pads_aabb(pads: &[ModPad]) -> Aabb {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in pads {
        let hx = p.width_mm.abs() / 2.0;
        let hy = p.height_mm.abs() / 2.0;
        min_x = min_x.min(p.x_mm - hx);
        max_x = max_x.max(p.x_mm + hx);
        min_y = min_y.min(p.y_mm - hy);
        max_y = max_y.max(p.y_mm + hy);
    }
    Aabb {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

/// Translate + rotate a local AABB to board millimetres.
/// Same visually-CCW rotation sense as [`world_xy`].
pub fn aabb_at(local: &Aabb, x_mm: f64, y_mm: f64, rot_deg: f64) -> Aabb {
    let rad = rot_deg.to_radians();
    let (s, c) = (rad.sin(), rad.cos());
    let corners = [
        (local.min_x, local.min_y),
        (local.max_x, local.min_y),
        (local.max_x, local.max_y),
        (local.min_x, local.max_y),
    ];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (lx, ly) in corners {
        let wx = x_mm + c * lx + s * ly;
        let wy = y_mm - s * lx + c * ly;
        min_x = min_x.min(wx);
        max_x = max_x.max(wx);
        min_y = min_y.min(wy);
        max_y = max_y.max(wy);
    }
    Aabb {
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

/// Cell centres for a rows×cols grid. Cell (0,0) is `origin`; columns
/// go +x, rows go +y (KiCad native). Used by `place_matrix`.
pub fn matrix_positions(
    rows: u32,
    cols: u32,
    pitch_x_mm: f64,
    pitch_y_mm: f64,
    origin_x_mm: f64,
    origin_y_mm: f64,
) -> Result<Vec<(f64, f64)>, String> {
    if rows == 0 || cols == 0 {
        return Err("place_matrix needs rows ≥ 1 and cols ≥ 1".into());
    }
    let n = rows.saturating_mul(cols);
    if n as usize > PLACE_MAX {
        return Err(format!(
            "place_matrix max {PLACE_MAX} cells (got {rows}×{cols} = {n})"
        ));
    }
    if pitch_x_mm < 1.0 || pitch_y_mm < 1.0 {
        return Err("place_matrix pitch must be at least 1 mm".into());
    }
    let mut out = Vec::with_capacity(n as usize);
    for r in 0..rows {
        for c in 0..cols {
            out.push((
                origin_x_mm + f64::from(c) * pitch_x_mm,
                origin_y_mm + f64::from(r) * pitch_y_mm,
            ));
        }
    }
    Ok(out)
}

pub struct LoadedTemplate {
    pub pads: Vec<ModPad>,
    pub courtyard: Aabb,
}

/// F.CrtYd box of a template on disk; falls back to the pad AABB.
/// None if the template file is missing or unreadable.
pub fn courtyard_of_template(pretty_dir: &std::path::Path, template: &str) -> Option<Aabb> {
    let path = pretty_dir.join(format!("{template}.kicad_mod"));
    let text = std::fs::read_to_string(path).ok()?;
    parse_kicad_mod_courtyard(&text)
        .or_else(|| parse_kicad_mod_pads(&text).ok().map(|p| pads_aabb(&p)))
}

pub fn load_template(
    pretty_dir: &std::path::Path,
    template: &str,
) -> Result<LoadedTemplate, String> {
    let path = pretty_dir.join(format!("{template}.kicad_mod"));
    if !path.exists() {
        return Err(format!(
            "no template named {template} in jlcpcb_parts — call list_parts or download_lcsc_part first"
        ));
    }
    let lib = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let pads = parse_kicad_mod_pads(&lib)?;
    let courtyard = parse_kicad_mod_courtyard(&lib).unwrap_or_else(|| pads_aabb(&pads));
    Ok(LoadedTemplate { pads, courtyard })
}

fn parse_one_pad(sexpr: &str) -> Result<ModPad, String> {
    let number = quoted_after(sexpr, "(pad ").ok_or_else(|| "pad without number".to_string())?;
    // `np_thru_hole` is KiCad's own token (EasyEDA-converted footprints);
    // `npth` is the shorthand the builtin generators write. Check the long
    // form first — matching ` thru_hole ` alone would misread neither, but
    // falling through to SmdFront silently deleted the hole (USB-C
    // positioning pegs baked as paste-covered SMD circles).
    let kind = if sexpr.contains(" np_thru_hole ") || sexpr.contains(" npth ") {
        ModPadKind::Npth
    } else if sexpr.contains(" thru_hole ") {
        ModPadKind::ThruHole
    } else if sexpr.contains("\"B.Cu\"") {
        ModPadKind::SmdBack
    } else {
        ModPadKind::SmdFront
    };
    let shape = if sexpr.contains(" circle ") {
        ModPadShape::Circle
    } else if sexpr.contains(" oval ") {
        ModPadShape::Oval
    } else {
        ModPadShape::Rect
    };
    let at = tuple_after(sexpr, "(at ").ok_or_else(|| format!("pad {number} missing (at)"))?;
    let size =
        tuple_after(sexpr, "(size ").ok_or_else(|| format!("pad {number} missing (size)"))?;
    // `(drill 0.6)` → round; `(drill oval 0.6 1.7)` → slot (the word `oval`
    // is skipped by the numeric parser, leaving [width, length]).
    let drill = tuple_after(sexpr, "(drill ");
    Ok(ModPad {
        number,
        kind,
        shape,
        x_mm: at[0],
        y_mm: at[1],
        rot_deg: if at.len() >= 3 { at[2] } else { 0.0 },
        width_mm: size[0],
        height_mm: size.get(1).copied().unwrap_or(size[0]),
        drill_mm: drill.as_ref().map(|v| v[0]),
        drill_h_mm: drill.as_ref().and_then(|v| v.get(1).copied()),
    })
}

fn quoted_after(src: &str, prefix: &str) -> Option<String> {
    let from = src.find(prefix)? + prefix.len();
    let tail = src[from..].trim_start();
    if !tail.starts_with('"') {
        return None;
    }
    let inner = &tail[1..];
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

fn tuple_after(src: &str, prefix: &str) -> Option<Vec<f64>> {
    let from = src.find(prefix)? + prefix.len();
    let tail = src[from..].trim_start();
    let end = tail.find(')')?;
    let nums = tail[..end]
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .collect::<Vec<_>>();
    if nums.is_empty() {
        None
    } else {
        Some(nums)
    }
}

fn matching_paren(src: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in src.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn field(name: &str, text: &str, x_mm: f64, y_mm: f64, layer: i32, visible: bool) -> Field {
    Field {
        name: name.into(),
        visible,
        text: Some(BoardText {
            text: Some(Text {
                position: Some(Vector2 {
                    x_nm: mm_to_nm(x_mm),
                    y_nm: mm_to_nm(y_mm),
                }),
                attributes: Some(TextAttributes {
                    horizontal_alignment: HA_CENTER,
                    vertical_alignment: VA_CENTER,
                    stroke_width: Some(Distance { value_nm: 150_000 }),
                    size: Some(Vector2 {
                        x_nm: 1_000_000,
                        y_nm: 1_000_000,
                    }),
                    ..Default::default()
                }),
                text: text.into(),
                ..Default::default()
            }),
            layer,
            locked: LS_UNLOCKED,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn pad_any(pad: &ModPad) -> Any {
    let (pad_type, layers) = match pad.kind {
        ModPadKind::SmdFront => (PT_SMD, vec![BL_F_CU, BL_F_PASTE, BL_F_MASK]),
        ModPadKind::SmdBack => (PT_SMD, vec![BL_B_CU, BL_B_PASTE, BL_B_MASK]),
        ModPadKind::ThruHole => (PT_PTH, vec![BL_F_CU, BL_B_CU, BL_F_MASK, BL_B_MASK]),
        ModPadKind::Npth => (PT_NPTH, vec![BL_F_CU, BL_B_CU, BL_F_MASK, BL_B_MASK]),
    };
    let shape = match pad.shape {
        ModPadShape::Circle => PSS_CIRCLE,
        ModPadShape::Oval => PSS_OVAL,
        ModPadShape::Rect => PSS_RECTANGLE,
    };
    let copper_layer = match pad.kind {
        ModPadKind::SmdBack => BL_B_CU,
        _ => BL_F_CU,
    };
    let proto = Pad {
        locked: LS_UNLOCKED,
        number: pad.number.clone(),
        r#type: pad_type,
        position: Some(Vector2 {
            x_nm: mm_to_nm(pad.x_mm),
            y_nm: mm_to_nm(pad.y_mm),
        }),
        pad_stack: Some(PadStack {
            r#type: PST_NORMAL,
            layers,
            drill: pad.drill_mm.map(|d| {
                let h = pad.drill_h_mm.unwrap_or(d);
                DrillProperties {
                    diameter: Some(Vector2 {
                        x_nm: mm_to_nm(d),
                        y_nm: mm_to_nm(h),
                    }),
                    // Without the explicit shape KiCad drills a round hole
                    // even when x≠y — the USB-C shield slots came out as
                    // 0.6 mm circles in the Excellon file.
                    shape: if (h - d).abs() > 1e-9 {
                        DRILL_OBLONG
                    } else {
                        DRILL_CIRCLE
                    },
                    ..Default::default()
                }
            }),
            copper_layers: vec![PadStackLayer {
                layer: copper_layer,
                shape,
                size: Some(Vector2 {
                    x_nm: mm_to_nm(pad.width_mm),
                    y_nm: mm_to_nm(pad.height_mm),
                }),
                ..Default::default()
            }],
            angle: (pad.rot_deg.abs() > 0.01).then_some(Angle {
                value_degrees: pad.rot_deg,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    pack(&proto, TYPE_PAD)
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
struct LibraryIdentifier {
    #[prost(string, tag = "1")]
    library_nickname: String,
    #[prost(string, tag = "2")]
    entry_name: String,
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

#[derive(Clone, PartialEq, Message)]
struct Field {
    #[prost(string, tag = "2")]
    name: String,
    #[prost(message, optional, tag = "3")]
    text: Option<BoardText>,
    #[prost(bool, tag = "4")]
    visible: bool,
}

#[derive(Clone, PartialEq, Message)]
struct FootprintAttributes {
    #[prost(string, tag = "1")]
    description: String,
    #[prost(string, tag = "2")]
    keywords: String,
    #[prost(bool, tag = "3")]
    not_in_schematic: bool,
    #[prost(bool, tag = "4")]
    exclude_from_position_files: bool,
    #[prost(bool, tag = "5")]
    exclude_from_bill_of_materials: bool,
    #[prost(bool, tag = "6")]
    exempt_from_courtyard_requirement: bool,
    #[prost(bool, tag = "7")]
    do_not_populate: bool,
    #[prost(int32, tag = "8")]
    mounting_style: i32,
}

#[derive(Clone, PartialEq, Message)]
struct Footprint {
    #[prost(message, optional, tag = "1")]
    id: Option<LibraryIdentifier>,
    #[prost(message, optional, tag = "3")]
    attributes: Option<FootprintAttributes>,
    #[prost(message, optional, tag = "7")]
    reference_field: Option<Field>,
    #[prost(message, optional, tag = "8")]
    value_field: Option<Field>,
    #[prost(message, repeated, tag = "11")]
    items: Vec<Any>,
}

#[derive(Clone, PartialEq, Message)]
struct FootprintInstance {
    #[prost(message, optional, tag = "2")]
    position: Option<Vector2>,
    #[prost(message, optional, tag = "3")]
    orientation: Option<Angle>,
    #[prost(int32, tag = "4")]
    layer: i32,
    #[prost(int32, tag = "5")]
    locked: i32,
    #[prost(message, optional, tag = "6")]
    definition: Option<Footprint>,
    #[prost(message, optional, tag = "7")]
    reference_field: Option<Field>,
    #[prost(message, optional, tag = "8")]
    value_field: Option<Field>,
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
    #[prost(int32, tag = "4")]
    unconnected_layer_removal: i32,
    #[prost(message, repeated, tag = "5")]
    copper_layers: Vec<PadStackLayer>,
    #[prost(message, optional, tag = "6")]
    angle: Option<Angle>,
}

#[derive(Clone, PartialEq, Message)]
struct Pad {
    #[prost(int32, tag = "2")]
    locked: i32,
    #[prost(string, tag = "3")]
    number: String,
    #[prost(int32, tag = "5")]
    r#type: i32,
    #[prost(message, optional, tag = "6")]
    pad_stack: Option<PadStack>,
    #[prost(message, optional, tag = "7")]
    position: Option<Vector2>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_0603_pads() {
        let src = r#"(footprint "C25804_R0603"
  (pad "2" smd rect (at 0.7534 -0) (size 0.8065 0.864) (layers "F.Cu" "F.Paste" "F.Mask"))
  (pad "1" smd rect (at -0.7534 -0) (size 0.8065 0.864) (layers "F.Cu" "F.Paste" "F.Mask"))
)"#;
        let pads = parse_kicad_mod_pads(src).unwrap();
        assert_eq!(pads.len(), 2);
        assert_eq!(pads[0].number, "2");
        assert!((pads[0].x_mm - 0.7534).abs() < 1e-6);
        assert_eq!(pads[1].number, "1");
        assert_eq!(pads[1].kind, ModPadKind::SmdFront);
    }

    /// The exact pad lines of the C165948 USB-C footprint. `np_thru_hole`
    /// must become NPTH (not a paste-covered SMD circle) and the oval drill
    /// must keep its slot length — both were silently dropped once, leaving
    /// the connector without positioning holes and with 0.6 mm round holes
    /// where 0.6×1.7 shield slots belong.
    #[test]
    fn parses_npth_pegs_and_oval_drill_slots() {
        let src = r#"(footprint "C165948_USB-C"
  (pad "1" thru_hole oval (at 4.325 -1.7057) (size 0.9 2) (drill oval 0.6 1.7) (layers "*.Cu" "*.Mask"))
  (pad "" np_thru_hole circle (at 2.89 -1.1541) (size 0.6 0.6) (drill 0.6) (layers "*.Cu" "*.Mask"))
)"#;
        let pads = parse_kicad_mod_pads(src).unwrap();
        assert_eq!(pads.len(), 2);
        let slot = &pads[0];
        assert_eq!(slot.kind, ModPadKind::ThruHole);
        assert_eq!(slot.drill_mm, Some(0.6));
        assert_eq!(slot.drill_h_mm, Some(1.7));
        let peg = &pads[1];
        assert_eq!(peg.kind, ModPadKind::Npth);
        assert_eq!(peg.drill_mm, Some(0.6));
        assert_eq!(peg.drill_h_mm, None);
    }

    /// The builtin generators write the short `npth` token — must still work.
    #[test]
    fn parses_builtin_npth_token() {
        let src = r#"(footprint "MountingHole_M3_NPTH"
  (pad "" npth circle (at 0 0) (size 3.2 3.2) (drill 3.2) (layers "*.Cu" "*.Mask"))
)"#;
        let pads = parse_kicad_mod_pads(src).unwrap();
        assert_eq!(pads[0].kind, ModPadKind::Npth);
        assert_eq!(pads[0].drill_mm, Some(3.2));
    }

    /// Baked proto keeps the oblong drill: diameter x≠y and shape OBLONG.
    #[test]
    fn bakes_oval_drill_as_oblong() {
        let pad = ModPad {
            number: "1".into(),
            kind: ModPadKind::ThruHole,
            shape: ModPadShape::Oval,
            x_mm: 0.0,
            y_mm: 0.0,
            rot_deg: 0.0,
            width_mm: 0.9,
            height_mm: 2.0,
            drill_mm: Some(0.6),
            drill_h_mm: Some(1.7),
        };
        let any = pad_any(&pad);
        let decoded = Pad::decode(any.value.as_slice()).unwrap();
        let drill = decoded.pad_stack.unwrap().drill.unwrap();
        assert_eq!(drill.shape, DRILL_OBLONG);
        let d = drill.diameter.unwrap();
        assert_eq!(d.x_nm, 600_000);
        assert_eq!(d.y_nm, 1_700_000);
        // Round hole stays a circle.
        let round = ModPad {
            drill_h_mm: None,
            ..pad
        };
        let decoded = Pad::decode(pad_any(&round).value.as_slice()).unwrap();
        let drill = decoded.pad_stack.unwrap().drill.unwrap();
        assert_eq!(drill.shape, DRILL_CIRCLE);
        assert_eq!(drill.diameter.unwrap().y_nm, 600_000);
    }

    #[test]
    fn parses_courtyard_and_overlap() {
        let src = r#"(footprint "C25804_R0603"
  (fp_line (start -1.3851 -0.6606) (end 1.3851 -0.6606) (stroke (width 0.05) (type solid)) (layer "F.CrtYd"))
  (fp_line (start 1.3851 -0.6606) (end 1.3851 0.6606) (stroke (width 0.05) (type solid)) (layer "F.CrtYd"))
  (fp_line (start 1.3851 0.6606) (end -1.3851 0.6606) (stroke (width 0.05) (type solid)) (layer "F.CrtYd"))
  (fp_line (start -1.3851 0.6606) (end -1.3851 -0.6606) (stroke (width 0.05) (type solid)) (layer "F.CrtYd"))
)"#;
        let cy = parse_kicad_mod_courtyard(src).unwrap();
        assert!((cy.max_x - 1.3851).abs() < 1e-6);
        let a = aabb_at(&cy, 0.0, 0.0, 0.0);
        let b = aabb_at(&cy, 10.0, 0.0, 0.0);
        assert!(!a.overlaps(&b, 0.5));
        let stacked = aabb_at(&cy, 0.2, 0.0, 0.0);
        assert!(a.overlaps(&stacked, 0.5));
    }

    #[test]
    fn bakes_local_pad_to_board_xy() {
        let spec = PlaceSpec {
            template: "t",
            reference: "R1",
            x_mm: 123.0,
            y_mm: 100.0,
            rotation_deg: 0.0,
            pads: &[],
        };
        let (x, y) = world_xy(0.75, 0.0, &spec);
        assert!((x - 123.75).abs() < 1e-9);
        assert!((y - 100.0).abs() < 1e-9);
    }

    /// Rotation-sense guard: KiCad's +90° is counterclockwise on screen.
    /// In the y-down internal frame an east pad (+x) must land **north**
    /// (smaller y) after +90°, matching the orientation field that the
    /// CPL export hands to JLCPCB assembly. If this asserts, pads are
    /// baked clockwise and every rotated asymmetric part solders 180° off.
    #[test]
    fn plus_90_rotates_counterclockwise_on_screen() {
        let spec = PlaceSpec {
            template: "t",
            reference: "U1",
            x_mm: 100.0,
            y_mm: 100.0,
            rotation_deg: 90.0,
            pads: &[],
        };
        let (x, y) = world_xy(3.0, 0.0, &spec);
        assert!((x - 100.0).abs() < 1e-9, "east pad must move to centre x, got {x}");
        assert!(
            (y - 97.0).abs() < 1e-9,
            "east pad must land north (y-down frame: smaller y), got {y}"
        );
    }

    #[test]
    fn matrix_positions_row_major_plus_y() {
        let pts = matrix_positions(2, 3, 12.7, 12.7, 100.0, 80.0).unwrap();
        assert_eq!(pts.len(), 6);
        assert!((pts[0].0 - 100.0).abs() < 1e-9);
        assert!((pts[0].1 - 80.0).abs() < 1e-9);
        assert!((pts[1].0 - 112.7).abs() < 1e-9);
        assert!((pts[3].0 - 100.0).abs() < 1e-9);
        assert!((pts[3].1 - 92.7).abs() < 1e-9);
        assert!(matrix_positions(20, 10, 12.7, 12.7, 0.0, 0.0).is_err());
    }
}
