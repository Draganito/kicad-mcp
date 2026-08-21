//! Fetch an LCSC/EasyEDA C-number and emit a native KiCad footprint
//! (`.kicad_mod`) plus a matching schematic symbol (`.kicad_sym`).
//!
//! Geometry comes from EasyEDA's undocumented product API — the same
//! `PAD~shape~x~y~width~height~layer~net~number~holeDia~...` field order
//! KiCad's own EasyEDA importer documents. Coordinates in EasyEDA PCB
//! space are +x right, +y down — the **same** convention `.kicad_mod`
//! files use, so geometry passes through without any Y flip. (An earlier
//! version negated Y here, which vertically mirrored every footprint.)
//!
//! Pin **numbers** come from the footprint pads. Pin **names/functions**
//! come from the EasyEDA schematic SVG (`/svgs`). Both are written to
//! `{template}.pins.json` so MCP can net from EasyEDA without a datasheet.
//! A manufacturer PDF is only for a logic check that EasyEDA cannot be right.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

const API_VERSION: &str = "6.4.19.5";
const REFERER: &str = "https://easyeda.com/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

/// 1 EasyEDA PCB unit = 10 mil = 0.254 mm.
const EASYEDA_UNIT_MM: f64 = 0.254;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("couldn't reach LCSC/EasyEDA: {0}")]
    Network(String),
    #[error("{0} wasn't found on LCSC/EasyEDA")]
    NotFound(String),
    #[error("{0} has no PCB footprint to import")]
    NoFootprint(String),
    #[error("couldn't understand LCSC/EasyEDA's response: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadLayer {
    Front,
    Back,
    Thru,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PadShape {
    Rect,
    Oval,
    Circle,
}

#[derive(Debug, Clone)]
pub struct Pad {
    pub number: String,
    pub pin_name: Option<String>,
    /// EasyEDA millimetres: +x right, +y down, origin = footprint head.
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotation_deg: f64,
    pub hole_dia_mm: Option<f64>,
    pub layer: PadLayer,
    pub shape: PadShape,
}

#[derive(Debug, Clone)]
pub struct Courtyard {
    pub center_x_mm: f64,
    pub center_y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, Clone)]
pub struct FetchedPart {
    pub lcsc_code: String,
    pub name: String,
    pub reference_prefix: String,
    pub description: String,
    pub category: Option<String>,
    pub package: String,
    pub datasheet_url: Option<String>,
    pub pads: Vec<Pad>,
    pub courtyard: Option<Courtyard>,
}

/// One EasyEDA pad number and its symbol pin name (the electrical function).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PinInfo {
    pub number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_name: Option<String>,
}

/// EasyEDA pin list persisted next to the `.kicad_mod` as `{template}.pins.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PartPins {
    pub lcsc_code: String,
    pub name: String,
    pub template: String,
    /// Always `"easyeda"` when written by `download_lcsc_part`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datasheet_url: Option<String>,
    pub pins: Vec<PinInfo>,
}

impl FetchedPart {
    /// KiCad footprint name, e.g. `C25804_R0603`.
    pub fn footprint_name(&self) -> String {
        let pkg = sanitize_ident(&self.package);
        if pkg.is_empty() {
            self.lcsc_code.clone()
        } else {
            format!("{}_{pkg}", self.lcsc_code)
        }
    }

    /// Unique pad numbers in footprint order, with EasyEDA `pin_name` when known.
    pub fn unique_pins(&self) -> Vec<PinInfo> {
        let mut seen = HashSet::new();
        let mut pins = Vec::new();
        for pad in &self.pads {
            if !seen.insert(pad.number.clone()) {
                continue;
            }
            pins.push(PinInfo {
                number: pad.number.clone(),
                pin_name: normalize_pin_name(pad.pin_name.as_deref()),
            });
        }
        pins
    }

    pub fn part_pins(&self) -> PartPins {
        PartPins {
            lcsc_code: self.lcsc_code.clone(),
            name: self.name.clone(),
            template: self.footprint_name(),
            source: "easyeda".into(),
            datasheet_url: self.datasheet_url.clone(),
            pins: self.unique_pins(),
        }
    }
}

fn normalize_pin_name(name: Option<&str>) -> Option<String> {
    let s = name.map(str::trim).filter(|s| !s.is_empty() && *s != "~")?;
    Some(s.to_string())
}

pub fn fetch_by_lcsc_code(code: &str) -> Result<FetchedPart, FetchError> {
    let code = code.trim();
    if code.is_empty() {
        return Err(FetchError::NotFound("(empty)".into()));
    }
    let url = format!("https://easyeda.com/api/products/{code}/components?version={API_VERSION}");
    let response = ureq::get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Referer", REFERER)
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| FetchError::Network(e.to_string()))?;
    let body: Value = response
        .into_json()
        .map_err(|e| FetchError::Parse(e.to_string()))?;
    let mut part = parse_response(code, &body)?;
    let pin_names = fetch_pin_names(code);
    for pad in &mut part.pads {
        if let Some(name) = pin_names.get(&pad.number) {
            pad.pin_name = Some(name.clone());
        }
    }
    Ok(part)
}

pub fn parse_response(code: &str, body: &Value) -> Result<FetchedPart, FetchError> {
    if body.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(FetchError::NotFound(code.to_string()));
    }
    let result = body
        .get("result")
        .ok_or_else(|| FetchError::Parse("missing 'result'".into()))?;
    let name = result
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(code)
        .to_string();
    let description = result
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let category = result
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|tags| tags.first())
        .and_then(Value::as_str)
        .map(str::to_string);

    let package_data_str = result
        .get("packageDetail")
        .and_then(|p| p.get("dataStr"))
        .ok_or_else(|| FetchError::NoFootprint(code.to_string()))?;
    let data: Value = match package_data_str {
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| FetchError::Parse(e.to_string()))?
        }
        other => other.clone(),
    };

    let head = data
        .get("head")
        .ok_or_else(|| FetchError::Parse("footprint has no 'head'".into()))?;
    let origin_x = head.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let origin_y = head.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    let c_para = head.get("c_para");
    let reference_prefix = c_para
        .and_then(|c| c.get("pre"))
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches('?').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "U".to_string());
    let package = c_para
        .and_then(|c| c.get("package"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let shapes = data
        .get("shape")
        .and_then(Value::as_array)
        .ok_or_else(|| FetchError::Parse("footprint has no 'shape' array".into()))?;
    let shape_lines: Vec<&str> = shapes.iter().filter_map(Value::as_str).collect();
    let pads: Vec<Pad> = shape_lines
        .iter()
        .filter_map(|line| parse_pad_line(line, origin_x, origin_y))
        .collect();
    if pads.is_empty() {
        return Err(FetchError::NoFootprint(code.to_string()));
    }
    let courtyard = parse_silk_courtyard(&shape_lines, origin_x, origin_y);

    let description = match (package.is_empty(), description.is_empty()) {
        (false, false) => format!("{package} — {description}"),
        (false, true) => package.clone(),
        (true, _) => description,
    };

    Ok(FetchedPart {
        lcsc_code: code.to_string(),
        name,
        reference_prefix,
        description,
        category,
        package,
        datasheet_url: extract_datasheet_url(result),
        pads,
        courtyard,
    })
}

fn extract_datasheet_url(result: &Value) -> Option<String> {
    const KEYS: &[&str] = &["datasheet", "dataManualUrl", "dataManual", "url"];
    for key in KEYS {
        if let Some(s) = result
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| s.starts_with("http"))
        {
            return Some(s.to_string());
        }
    }
    let attrs = result
        .get("attributes")
        .or_else(|| result.get("szAttr"))
        .and_then(Value::as_object)?;
    for (key, value) in attrs {
        let lower = key.to_ascii_lowercase();
        if !(lower.contains("datasheet") || lower.contains("manual")) {
            continue;
        }
        if let Some(s) = value
            .as_str()
            .map(str::trim)
            .filter(|s| s.starts_with("http"))
        {
            return Some(s.to_string());
        }
    }
    None
}

fn fetch_pin_names(code: &str) -> HashMap<String, String> {
    let url = format!("https://easyeda.com/api/products/{code}/svgs");
    let Ok(response) = ureq::get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Referer", REFERER)
        .set("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
    else {
        return HashMap::new();
    };
    let Ok(body): Result<Value, _> = response.into_json() else {
        return HashMap::new();
    };
    parse_pin_names(&body)
}

pub fn parse_pin_names(body: &Value) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Some(entries) = body.get("result").and_then(Value::as_array) else {
        return names;
    };
    for entry in entries
        .iter()
        .filter(|e| e.get("docType").and_then(Value::as_i64) == Some(2))
    {
        if let Some(svg) = entry.get("svg").and_then(Value::as_str) {
            for (number, name) in extract_pin_names_from_symbol_svg(svg) {
                names.insert(number, name);
            }
        }
    }
    names
}

fn extract_pin_names_from_symbol_svg(svg: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for group in svg.split("c_partid=\"part_pin\"").skip(1) {
        let Some(number) = extract_attr(group, "c_spicepin=\"") else {
            continue;
        };
        let mut texts = Vec::new();
        let mut rest = group;
        while texts.len() < 2 {
            let Some(start) = rest.find("<text") else {
                break;
            };
            let Some(gt) = rest[start..].find('>') else {
                break;
            };
            let after_gt = &rest[start + gt + 1..];
            let Some(close) = after_gt.find("</text>") else {
                break;
            };
            texts.push(after_gt[..close].to_string());
            rest = &after_gt[close + "</text>".len()..];
        }
        if let Some(name) = texts.first().filter(|s| !s.is_empty()) {
            pairs.push((number, name.clone()));
        }
    }
    pairs
}

fn extract_attr(s: &str, marker: &str) -> Option<String> {
    let start = s.find(marker)? + marker.len();
    let rest = &s[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

const TOP_SILK_LAYER: i32 = 3;

fn parse_pad_line(line: &str, origin_x: f64, origin_y: f64) -> Option<Pad> {
    let f: Vec<&str> = line.split('~').collect();
    if f.len() < 12 || f[0] != "PAD" {
        return None;
    }
    let shape_name = f[1];
    let x: f64 = f[2].parse().ok()?;
    let y: f64 = f[3].parse().ok()?;
    let width: f64 = f[4].parse().ok()?;
    let height: f64 = f[5].parse().ok()?;
    let layer: i32 = f[6].parse().ok()?;
    let number = f[8].to_string();
    let hole_dia: f64 = f[9].parse().unwrap_or(0.0);
    let rotation_deg: f64 = f[11].parse().unwrap_or(0.0);

    let mut w = width * EASYEDA_UNIT_MM;
    let mut h = height * EASYEDA_UNIT_MM;
    let is_tht = hole_dia > 0.0;
    let pad_layer = if is_tht || layer == 11 {
        PadLayer::Thru
    } else if layer == 2 {
        PadLayer::Back
    } else {
        PadLayer::Front
    };

    let shape = match shape_name {
        "RECT" => PadShape::Rect,
        "OVAL" | "ELLIPSE" if (w - h).abs() > 1e-6 => PadShape::Oval,
        "OVAL" | "ELLIPSE" => PadShape::Circle,
        "POLYGON" => {
            if let Some((bw, bh)) = poly_bbox_mm(f.get(10).copied().unwrap_or("")) {
                w = bw;
                h = bh;
            }
            PadShape::Rect
        }
        _ => PadShape::Circle,
    };

    Some(Pad {
        number,
        pin_name: None,
        x_mm: (x - origin_x) * EASYEDA_UNIT_MM,
        y_mm: (y - origin_y) * EASYEDA_UNIT_MM,
        width_mm: w,
        height_mm: h,
        rotation_deg,
        hole_dia_mm: is_tht.then_some((hole_dia * EASYEDA_UNIT_MM).max(0.001)),
        layer: pad_layer,
        shape,
    })
}

fn poly_bbox_mm(points_str: &str) -> Option<(f64, f64)> {
    let nums: Vec<f64> = points_str
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() < 4 {
        return None;
    }
    let xs: Vec<f64> = nums.iter().step_by(2).copied().collect();
    let ys: Vec<f64> = nums.iter().skip(1).step_by(2).copied().collect();
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((
        (max_x - min_x) * EASYEDA_UNIT_MM,
        (max_y - min_y) * EASYEDA_UNIT_MM,
    ))
}

fn parse_coordinate_pairs(points_or_path: &str) -> Vec<(f64, f64)> {
    let nums: Vec<f64> = points_or_path
        .split_whitespace()
        .filter_map(|tok| tok.parse().ok())
        .collect();
    nums.chunks_exact(2).map(|p| (p[0], p[1])).collect()
}

fn extend_bbox(bbox: &mut Option<(f64, f64, f64, f64)>, x: f64, y: f64) {
    *bbox = Some(match bbox.take() {
        Some((min_x, min_y, max_x, max_y)) => {
            (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
        }
        None => (x, y, x, y),
    });
}

fn extend_arc_bbox(bbox: &mut Option<(f64, f64, f64, f64)>, path_data: &str) {
    let tokens: Vec<&str> = path_data.split_whitespace().collect();
    let mut last_point: Option<(f64, f64)> = None;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "M" | "L" if i + 2 < tokens.len() => {
                if let (Ok(x), Ok(y)) = (tokens[i + 1].parse::<f64>(), tokens[i + 2].parse::<f64>())
                {
                    extend_bbox(bbox, x, y);
                    last_point = Some((x, y));
                }
                i += 3;
            }
            "A" if i + 7 < tokens.len() => {
                let parsed: Option<(f64, f64, f64, f64)> = (|| {
                    Some((
                        tokens[i + 1].parse().ok()?,
                        tokens[i + 2].parse().ok()?,
                        tokens[i + 6].parse().ok()?,
                        tokens[i + 7].parse().ok()?,
                    ))
                })();
                if let Some((rx, ry, x, y)) = parsed {
                    if let Some((sx, sy)) = last_point {
                        extend_bbox(bbox, sx - rx, sy - ry);
                        extend_bbox(bbox, sx + rx, sy + ry);
                    }
                    extend_bbox(bbox, x - rx, y - ry);
                    extend_bbox(bbox, x + rx, y + ry);
                    last_point = Some((x, y));
                }
                i += 8;
            }
            _ => i += 1,
        }
    }
}

fn parse_silk_courtyard(shapes: &[&str], origin_x: f64, origin_y: f64) -> Option<Courtyard> {
    let mut bbox: Option<(f64, f64, f64, f64)> = None;
    for line in shapes {
        let f: Vec<&str> = line.split('~').collect();
        match f.first().copied() {
            Some("TRACK") if f.len() >= 5 => {
                if f[2].parse::<i32>() != Ok(TOP_SILK_LAYER) {
                    continue;
                }
                for (x, y) in parse_coordinate_pairs(f[4]) {
                    extend_bbox(&mut bbox, x, y);
                }
            }
            Some("RECT") if f.len() >= 6 => {
                if f[5].parse::<i32>() != Ok(TOP_SILK_LAYER) {
                    continue;
                }
                let parsed: Option<[f64; 4]> = (|| {
                    Some([
                        f[1].parse().ok()?,
                        f[2].parse().ok()?,
                        f[3].parse().ok()?,
                        f[4].parse().ok()?,
                    ])
                })();
                let Some([x, y, width, height]) = parsed else {
                    continue;
                };
                extend_bbox(&mut bbox, x, y);
                extend_bbox(&mut bbox, x + width, y + height);
            }
            Some("ARC") if f.len() >= 5 => {
                if f[2].parse::<i32>() != Ok(TOP_SILK_LAYER) {
                    continue;
                }
                extend_arc_bbox(&mut bbox, f[4]);
            }
            _ => continue,
        }
    }
    let (min_x, min_y, max_x, max_y) = bbox?;
    Some(Courtyard {
        center_x_mm: ((min_x + max_x) / 2.0 - origin_x) * EASYEDA_UNIT_MM,
        center_y_mm: ((min_y + max_y) / 2.0 - origin_y) * EASYEDA_UNIT_MM,
        width_mm: (max_x - min_x) * EASYEDA_UNIT_MM,
        height_mm: (max_y - min_y) * EASYEDA_UNIT_MM,
    })
}

fn sanitize_ident(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    out.trim_matches('_').to_string()
}

fn sexpr_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn fmt_mm(v: f64) -> String {
    format!("{v:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// KiCad `.kicad_mod` files use +x right, **+y down** — the same axis
/// convention as EasyEDA. Coordinates pass through unchanged; negating y
/// here would mirror every footprint vertically (swapping e.g. VIN/GND on
/// a SOT-223) even though the pad pattern still looks plausible.
fn kicad_xy(pad: &Pad) -> (f64, f64) {
    (pad.x_mm, pad.y_mm)
}

/// Rotation passes through unchanged for the same reason: both canvases
/// share the y-down frame, so the rotation sense is already identical.
fn kicad_rot(pad: &Pad) -> f64 {
    pad.rotation_deg
}

pub fn emit_kicad_mod(part: &FetchedPart) -> String {
    emit_kicad_mod_placed(part, None)
}

/// If `place` is `Some((x_mm, y_mm, rot_deg, reference))` the footprint is
/// board-ready (has a root `(at …)` and a real Reference) for KiCad's
/// `ParseAndCreateItemsFromString`.
pub fn emit_kicad_mod_placed(part: &FetchedPart, place: Option<(f64, f64, f64, &str)>) -> String {
    let name = part.footprint_name();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "(footprint {} (version 20240108) (generator \"kicad-mcp\")",
        sexpr_str(&name)
    );
    let _ = writeln!(out, "  (layer \"F.Cu\")");
    if let Some((x, y, rot, _)) = place {
        let _ = writeln!(out, "  (at {} {} {})", fmt_mm(x), fmt_mm(y), fmt_mm(rot));
    }
    let descr = format!("{} — LCSC {}", part.description, part.lcsc_code);
    let _ = writeln!(out, "  (descr {})", sexpr_str(&descr));
    let _ = writeln!(
        out,
        "  (tags {})",
        sexpr_str(&format!("LCSC {}", part.lcsc_code))
    );
    let attr = if part.pads.iter().any(|p| p.layer == PadLayer::Thru) {
        "through_hole"
    } else {
        "smd"
    };
    let _ = writeln!(out, "  (attr {attr})");
    let reference = place.map(|p| p.3).unwrap_or("REF**");
    let _ = writeln!(
        out,
        "  (property \"Reference\" {} (at 0 -2) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))",
        sexpr_str(reference)
    );
    let _ = writeln!(
        out,
        "  (property \"Value\" {} (at 0 2) (layer \"F.Fab\") (effects (font (size 1 1) (thickness 0.15))))",
        sexpr_str(&name)
    );
    let _ = writeln!(
        out,
        "  (property \"LCSC\" {} (at 0 0) (layer \"F.Fab\") (hide yes) (effects (font (size 1 1) (thickness 0.15))))",
        sexpr_str(&part.lcsc_code)
    );
    let _ = writeln!(
        out,
        "  (property \"ki_description\" {} (at 0 0) (layer \"F.Fab\") (hide yes))",
        sexpr_str(&part.name)
    );

    if let Some(c) = &part.courtyard {
        let hw = c.width_mm / 2.0;
        let hh = c.height_mm / 2.0;
        let cx = c.center_x_mm;
        let cy = c.center_y_mm;
        courtyard_rect(&mut out, cx - hw, cy - hh, cx + hw, cy + hh);
    } else {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for pad in &part.pads {
            let (x, y) = kicad_xy(pad);
            min_x = min_x.min(x - pad.width_mm / 2.0);
            max_x = max_x.max(x + pad.width_mm / 2.0);
            min_y = min_y.min(y - pad.height_mm / 2.0);
            max_y = max_y.max(y + pad.height_mm / 2.0);
        }
        courtyard_rect(
            &mut out,
            min_x - 0.25,
            min_y - 0.25,
            max_x + 0.25,
            max_y + 0.25,
        );
    }

    for pad in &part.pads {
        out.push_str(&emit_pad(pad));
    }
    out.push_str(")\n");
    out
}

fn courtyard_rect(out: &mut String, x1: f64, y1: f64, x2: f64, y2: f64) {
    let pts = [(x1, y1), (x2, y1), (x2, y2), (x1, y2), (x1, y1)];
    for w in pts.windows(2) {
        let _ = writeln!(
            out,
            "  (fp_line (start {} {}) (end {} {}) (stroke (width 0.05) (type solid)) (layer \"F.CrtYd\"))",
            fmt_mm(w[0].0),
            fmt_mm(w[0].1),
            fmt_mm(w[1].0),
            fmt_mm(w[1].1)
        );
    }
}

fn emit_pad(pad: &Pad) -> String {
    let (x, y) = kicad_xy(pad);
    let rot = kicad_rot(pad);
    let shape = match pad.shape {
        PadShape::Rect => "rect",
        PadShape::Oval => "oval",
        PadShape::Circle => "circle",
    };
    let (kind, layers) = match pad.layer {
        PadLayer::Thru => ("thru_hole", "\"*.Cu\" \"*.Mask\""),
        PadLayer::Front => ("smd", "\"F.Cu\" \"F.Paste\" \"F.Mask\""),
        PadLayer::Back => ("smd", "\"B.Cu\" \"B.Paste\" \"B.Mask\""),
    };
    let at = if rot.abs() > 0.01 {
        format!("(at {} {} {})", fmt_mm(x), fmt_mm(y), fmt_mm(rot))
    } else {
        format!("(at {} {})", fmt_mm(x), fmt_mm(y))
    };
    let size = if pad.shape == PadShape::Circle {
        let d = pad.width_mm.max(pad.height_mm);
        format!("(size {} {})", fmt_mm(d), fmt_mm(d))
    } else {
        format!("(size {} {})", fmt_mm(pad.width_mm), fmt_mm(pad.height_mm))
    };
    let drill = pad
        .hole_dia_mm
        .map(|d| format!(" (drill {})", fmt_mm(d)))
        .unwrap_or_default();
    format!(
        "  (pad {} {kind} {shape} {at} {size}{drill} (layers {layers}))\n",
        sexpr_str(&pad.number)
    )
}

pub fn emit_kicad_sym(part: &FetchedPart) -> String {
    let name = part.footprint_name();
    let mut pins: Vec<&Pad> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for pad in &part.pads {
        if seen.insert(&pad.number) {
            pins.push(pad);
        }
    }
    pins.sort_by_key(|p| pin_sort_key(&p.number));
    let mut out = String::new();
    let _ = writeln!(
        out,
        "(kicad_symbol_lib (version 20231120) (generator \"kicad-mcp\")"
    );
    let _ = writeln!(
        out,
        "  (symbol {} (pin_names (offset 1.016)) (in_bom yes) (on_board yes)",
        sexpr_str(&name)
    );
    let _ = writeln!(
        out,
        "    (property \"Reference\" {} (at 0 5.08 0)",
        sexpr_str(&part.reference_prefix)
    );
    let _ = writeln!(out, "      (effects (font (size 1.27 1.27))))");
    let _ = writeln!(
        out,
        "    (property \"Value\" {} (at 0 -5.08 0)",
        sexpr_str(&part.name)
    );
    let _ = writeln!(out, "      (effects (font (size 1.27 1.27))))");
    let _ = writeln!(
        out,
        "    (property \"Footprint\" {} (at 0 -7.62 0)",
        sexpr_str(&format!("jlcpcb_parts:{name}"))
    );
    let _ = writeln!(out, "      (effects (font (size 1.27 1.27)) (hide yes)))");
    let _ = writeln!(
        out,
        "    (property \"LCSC\" {} (at 0 -10.16 0)",
        sexpr_str(&part.lcsc_code)
    );
    let _ = writeln!(out, "      (effects (font (size 1.27 1.27)) (hide yes)))");
    let _ = writeln!(out, "    (symbol {}_0_1", sexpr_str(&name));
    let h = (pins.len() as f64 * 2.54).max(5.08) / 2.0;
    let _ = writeln!(
        out,
        "      (rectangle (start -5.08 {0}) (end 5.08 -{0}) (stroke (width 0.254) (type default)) (fill (type background)))",
        fmt_mm(h)
    );
    let _ = writeln!(out, "    )");
    let _ = writeln!(out, "    (symbol {}_1_1", sexpr_str(&name));
    for (i, pad) in pins.iter().enumerate() {
        let y = h - 1.27 - i as f64 * 2.54;
        let pin_name = pad.pin_name.as_deref().unwrap_or("~");
        let _ = writeln!(
            out,
            "      (pin unspecified line (at -7.62 {} 0) (length 2.54) (name {} (effects (font (size 1.27 1.27)))) (number {} (effects (font (size 1.27 1.27)))))",
            fmt_mm(y),
            sexpr_str(pin_name),
            sexpr_str(&pad.number)
        );
    }
    let _ = writeln!(out, "    )");
    let _ = writeln!(out, "  )");
    let _ = writeln!(out, ")");
    out
}

fn pin_sort_key(n: &str) -> (u32, String) {
    if let Ok(v) = n.parse::<u32>() {
        (v, String::new())
    } else {
        (u32::MAX, n.to_string())
    }
}

/// Write `{pretty}/{name}.kicad_mod`, `{name}.pins.json`, and merge the symbol.
pub fn write_library_files(
    part: &FetchedPart,
    pretty_dir: &Path,
    sym_path: &Path,
) -> Result<String, std::io::Error> {
    std::fs::create_dir_all(pretty_dir)?;
    let name = part.footprint_name();
    let mod_path = pretty_dir.join(format!("{name}.kicad_mod"));
    std::fs::write(&mod_path, emit_kicad_mod(part))?;
    let pins_path = pretty_dir.join(format!("{name}.pins.json"));
    let pins_json = serde_json::to_string_pretty(&part.part_pins())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&pins_path, pins_json)?;
    merge_symbol_lib(sym_path, part)?;
    Ok(name)
}

/// Load EasyEDA pin names for a template. Prefers `{template}.pins.json`
/// written by `download_lcsc_part`; falls back to the `.kicad_sym` names
/// plus pad numbers from the `.kicad_mod`.
pub fn load_part_pins(
    pretty_dir: &Path,
    sym_path: &Path,
    template: &str,
) -> Result<PartPins, String> {
    let template = template.trim();
    if template.is_empty() {
        return Err("template is empty".into());
    }
    let pins_path = pretty_dir.join(format!("{template}.pins.json"));
    if pins_path.is_file() {
        let text = std::fs::read_to_string(&pins_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&text).map_err(|e| format!("{template}.pins.json: {e}"));
    }
    let mod_path = pretty_dir.join(format!("{template}.kicad_mod"));
    if !mod_path.is_file() {
        return Err(format!(
            "no template named {template} in jlcpcb_parts — call download_lcsc_part or list_parts first"
        ));
    }
    let mod_text = std::fs::read_to_string(&mod_path).map_err(|e| e.to_string())?;
    let numbers = pad_numbers_from_mod(&mod_text);
    if numbers.is_empty() {
        return Err(format!("{template}.kicad_mod has no pads"));
    }
    let names = if sym_path.is_file() {
        let sym = std::fs::read_to_string(sym_path).map_err(|e| e.to_string())?;
        parse_symbol_pin_names(&sym, template)
    } else {
        HashMap::new()
    };
    let pins = numbers
        .into_iter()
        .map(|number| {
            let pin_name = names
                .get(&number)
                .cloned()
                .and_then(|s| normalize_pin_name(Some(&s)));
            PinInfo { number, pin_name }
        })
        .collect();
    let source = if names.is_empty() {
        "kicad_mod"
    } else {
        "kicad_sym"
    };
    Ok(PartPins {
        lcsc_code: lcsc_code_from_template(template),
        name: template.to_string(),
        template: template.to_string(),
        source: source.into(),
        datasheet_url: None,
        pins,
    })
}

fn lcsc_code_from_template(template: &str) -> String {
    template
        .split_once('_')
        .map(|(head, _)| head)
        .filter(|head| head.starts_with('C') && head[1..].chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("")
        .to_string()
}

fn pad_numbers_from_mod(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut numbers = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("(pad \"") {
        rest = &rest[i + 6..];
        let Some(end) = rest.find('"') else {
            break;
        };
        let number = rest[..end].to_string();
        rest = &rest[end + 1..];
        if seen.insert(number.clone()) {
            numbers.push(number);
        }
    }
    numbers
}

fn parse_symbol_pin_names(sym: &str, template: &str) -> HashMap<String, String> {
    let marker = format!("(symbol \"{template}\"");
    let Some(start) = sym.find(&marker) else {
        return HashMap::new();
    };
    let rest = &sym[start..];
    let end = rest[marker.len()..]
        .find("\n  (symbol ")
        .map(|i| marker.len() + i)
        .unwrap_or(rest.len());
    let body = &rest[..end];
    let mut names = HashMap::new();
    for line in body.lines() {
        if !line.contains("(pin ") {
            continue;
        }
        let Some(name) = extract_sexpr_str(line, "(name ") else {
            continue;
        };
        let Some(number) = extract_sexpr_str(line, "(number ") else {
            continue;
        };
        if let Some(name) = normalize_pin_name(Some(&name)) {
            names.insert(number, name);
        }
    }
    names
}

fn extract_sexpr_str(line: &str, marker: &str) -> Option<String> {
    let i = line.find(marker)? + marker.len();
    let rest = line[i..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn merge_symbol_lib(sym_path: &Path, part: &FetchedPart) -> Result<(), std::io::Error> {
    let name = part.footprint_name();
    let new_sym = emit_kicad_sym(part);
    if !sym_path.exists() {
        std::fs::write(sym_path, new_sym)?;
        return Ok(());
    }
    let existing = std::fs::read_to_string(sym_path)?;
    let marker = format!("(symbol \"{name}\"");
    if existing.contains(&marker) {
        // Replace is v2 work; leave the old symbol if the name already exists.
        return Ok(());
    }
    let inner = new_sym
        .lines()
        .filter(|l| !l.starts_with("(kicad_symbol_lib") && *l != ")")
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = existing.trim_end();
    let without_close = trimmed.strip_suffix(')').unwrap_or(trimmed);
    let merged = format!("{without_close}\n{inner}\n)\n");
    std::fs::write(sym_path, merged)?;
    Ok(())
}

pub fn ensure_fp_lib_table(table_path: &Path) -> Result<(), std::io::Error> {
    ensure_lib_table(
        table_path,
        "fp_lib_table",
        "jlcpcb_parts",
        "${KIPRJMOD}/jlcpcb_parts.pretty",
        "LCSC/EasyEDA imports via kicad-mcp",
    )
}

pub fn ensure_sym_lib_table(table_path: &Path) -> Result<(), std::io::Error> {
    ensure_lib_table(
        table_path,
        "sym_lib_table",
        "jlcpcb_parts",
        "${KIPRJMOD}/jlcpcb_parts.kicad_sym",
        "LCSC/EasyEDA imports via kicad-mcp",
    )
}

fn ensure_lib_table(
    path: &Path,
    root: &str,
    name: &str,
    uri: &str,
    descr: &str,
) -> Result<(), std::io::Error> {
    let entry = format!(
        "  (lib (name \"{name}\")(type \"KiCad\")(uri \"{uri}\")(options \"\")(descr \"{descr}\"))\n"
    );
    if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        if existing.contains(&format!("(name \"{name}\")")) {
            return Ok(());
        }
        let trimmed = existing.trim_end();
        let without_close = trimmed.strip_suffix(')').unwrap_or(trimmed);
        std::fs::write(path, format!("{without_close}\n{entry})\n"))?;
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, format!("({root}\n  (version 7)\n{entry})\n"))?;
    }
    Ok(())
}

pub fn list_pretty_footprints(pretty_dir: &Path) -> Result<Vec<String>, std::io::Error> {
    if !pretty_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(pretty_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("kicad_mod") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path} isn't JSON: {e}"))
    }

    #[test]
    fn parses_real_0603_resistor() {
        let part = parse_response("C25804", &fixture("lcsc_c25804_resistor.json"))
            .expect("fixture must parse");
        assert_eq!(part.name, "0603WAF1002T5E");
        assert_eq!(part.reference_prefix, "R");
        assert_eq!(part.pads.len(), 2);
        assert_eq!(part.footprint_name(), "C25804_R0603");
        let mut xs: Vec<f64> = part.pads.iter().map(|p| p.x_mm).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            (xs[0] + xs[1]).abs() < 0.02,
            "pads should be origin-symmetric, got {xs:?}"
        );
        let spacing = xs[1] - xs[0];
        assert!(
            (spacing - 1.506).abs() < 0.02,
            "expected ~1.51mm 0603 spacing, got {spacing}"
        );
        let sexpr = emit_kicad_mod(&part);
        assert!(sexpr.contains("(pad \"1\" smd rect"));
        assert!(sexpr.contains("(pad \"2\" smd rect"));
        assert!(sexpr.contains("(layer \"F.CrtYd\")"));
        assert!(sexpr.contains("C25804"));
        for pad in &part.pads {
            assert!(pad.y_mm.abs() < 0.05);
        }
    }

    /// Chirality guard: EasyEDA and KiCad both use +y down, so pad
    /// coordinates must pass through **without** y negation. The USBLC6
    /// (SOT-23-6) fixture is real LCSC data; per the ST datasheet pin 1
    /// sits diagonally opposite pin 4. A vertical mirror would swap
    /// VBUS (pin 5) and GND (pin 2) rows and short the board.
    #[test]
    fn sot23_6_is_not_mirrored() {
        let part = parse_response("C7519", &fixture("lcsc_c7519_usblc6.json"))
            .expect("fixture must parse");
        assert_eq!(part.pads.len(), 6);
        let pad = |n: &str| part.pads.iter().find(|p| p.number == n).unwrap();
        // Raw EasyEDA values (y-down frame): 1,2,3 on the +y row; 4,5,6 on −y.
        assert!((pad("1").x_mm - -0.95).abs() < 0.01);
        assert!((pad("1").y_mm - 1.149).abs() < 0.01);
        assert!((pad("4").x_mm - 0.95).abs() < 0.01);
        assert!((pad("4").y_mm - -1.149).abs() < 0.01);
        // Emitted .kicad_mod keeps the same signs (no mirror).
        let sexpr = emit_kicad_mod(&part);
        assert!(
            sexpr.contains("(pad \"1\" smd rect (at -0.95 1.149"),
            "pad 1 must keep EasyEDA's y sign, got:\n{sexpr}"
        );
        assert!(
            sexpr.contains("(pad \"6\" smd rect (at -0.95 -1.149"),
            "pad 6 must keep EasyEDA's y sign, got:\n{sexpr}"
        );
        // Same chirality as KiCad's official SOT-23-6 rotated 90°:
        // walking 1→2→3 and 4→5→6 both run +x (left to right).
        assert!(pad("2").x_mm > pad("1").x_mm && pad("3").x_mm > pad("2").x_mm);
        assert!(pad("5").x_mm < pad("4").x_mm && pad("6").x_mm < pad("5").x_mm);
    }

    #[test]
    fn library_table_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("kicad-mcp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let table = dir.join("fp-lib-table");
        ensure_fp_lib_table(&table).unwrap();
        ensure_fp_lib_table(&table).unwrap();
        let text = std::fs::read_to_string(&table).unwrap();
        assert_eq!(text.matches("(name \"jlcpcb_parts\")").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resistor_unique_pins_are_pad_numbers() {
        let part = parse_response("C25804", &fixture("lcsc_c25804_resistor.json"))
            .expect("fixture must parse");
        let pins = part.unique_pins();
        assert_eq!(pins.len(), 2);
        let numbers: Vec<_> = pins.iter().map(|p| p.number.as_str()).collect();
        assert!(numbers.contains(&"1"));
        assert!(numbers.contains(&"2"));
        assert!(pins.iter().all(|p| p.pin_name.is_none()));
        assert!(part.datasheet_url.is_none());
    }

    #[test]
    fn writes_and_reloads_pins_json() {
        let mut part = parse_response("C25804", &fixture("lcsc_c25804_resistor.json"))
            .expect("fixture must parse");
        part.pads[0].pin_name = Some("GND".into());
        part.pads[1].pin_name = Some("3V3".into());
        let dir = std::env::temp_dir().join(format!("kicad-mcp-pins-{}", std::process::id()));
        let pretty = dir.join("jlcpcb_parts.pretty");
        let sym = dir.join("jlcpcb_parts.kicad_sym");
        let _ = std::fs::remove_dir_all(&dir);
        let name = write_library_files(&part, &pretty, &sym).unwrap();
        let loaded = load_part_pins(&pretty, &sym, &name).unwrap();
        assert_eq!(loaded.source, "easyeda");
        assert_eq!(loaded.lcsc_code, "C25804");
        assert_eq!(loaded.pins.len(), 2);
        let gnd = loaded.pins.iter().find(|p| p.pin_name.as_deref() == Some("GND"));
        let vcc = loaded.pins.iter().find(|p| p.pin_name.as_deref() == Some("3V3"));
        assert!(gnd.is_some() && vcc.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_pin_names_from_symbol_svg() {
        let svg = r#"<g c_partid="part_pin" c_spicepin="1"><text>GND</text><text>1</text></g>
<g c_partid="part_pin" c_spicepin="2"><text>VCC</text><text>2</text></g>"#;
        let body = serde_json::json!({
            "result": [{ "docType": 2, "svg": svg }]
        });
        let names = parse_pin_names(&body);
        assert_eq!(names.get("1").map(String::as_str), Some("GND"));
        assert_eq!(names.get("2").map(String::as_str), Some("VCC"));
    }

    #[test]
    fn extract_datasheet_from_attributes() {
        let mut body = fixture("lcsc_c25804_resistor.json");
        body["result"]["attributes"] = serde_json::json!({
            "Datasheet": "https://example.com/c25804.pdf"
        });
        let part = parse_response("C25804", &body).unwrap();
        assert_eq!(
            part.datasheet_url.as_deref(),
            Some("https://example.com/c25804.pdf")
        );
    }
}
