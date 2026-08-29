//! Short layout-physics review. Reads the open board; does not place copper.
//!
//! Checks return path (ground pour), power pour, plane adjacency, a via
//! next to each decoupling-cap GND pad, SK6812 daisy (DOUT→DIN), and
//! whether a 0603 GND pad sits next to the companion LED pin 1. Does
//! **not** nag about 90° corners, silk overlap, or DRC clearance — those
//! are myths or `check_drc`.

use serde::Serialize;

use crate::copper::{self, ZoneConnectStyle, ZoneSnap};
use crate::kicad::Kicad;
use crate::pads::PadRow;

const CAP_VIA_MM: f64 = 3.0;
const CAP_VIA_WARN_MISSING: usize = 1;
/// Max centre distance from a decoupling cap to its companion LED pin 1.
/// Between a 12.7 mm LED pitch and the bulk-cap cluster at the connector.
const CAP_COMPANION_MM: f64 = 8.0;
/// Half the long pad side. 0603 is ~0.45 mm; 0805 still fits. Bulk polymer
/// (C79111 ~1.2 mm) is above this so C211/C212 at the connector are skipped
/// even when they sit inside `CAP_COMPANION_MM` of an outer LED.
const CAP_DECOUPLE_PAD_RADIUS_MM: f64 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub id: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewReport {
    pub ok: bool,
    pub verdict: String,
    pub summary: String,
    pub findings: Vec<Finding>,
    /// Things this tool will not mention so an agent does not invent them.
    pub not_checked: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ReviewInput {
    pub copper_layers: u32,
    pub pads: Vec<PadFact>,
    pub vias: Vec<ViaFact>,
    pub zones: Vec<ZoneFact>,
}

#[derive(Debug, Clone)]
pub struct PadFact {
    pub reference: String,
    pub pin: String,
    pub net: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub kind: String,
    pub layer_ids: Vec<i32>,
    pub radius_mm: f64,
}

#[derive(Debug, Clone)]
pub struct ViaFact {
    pub net: String,
    pub x_mm: f64,
    pub y_mm: f64,
}

#[derive(Debug, Clone)]
pub struct ZoneFact {
    pub net: String,
    pub layer_ids: Vec<i32>,
    pub connection: ZoneConnectStyle,
    pub fills: Vec<copper::ZoneFill>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetKind {
    Empty,
    Ground,
    Rail,
    Signal,
}

pub fn not_checked() -> Vec<String> {
    vec![
        "90° track corners (not a problem at digital / LED speeds)".into(),
        "silk overlap on EasyEDA artwork (cosmetic; export already drops U1/C3)".into(),
        "DRC clearance / holes — use check_drc".into(),
        "empty pad nets — use check_board".into(),
    ]
}

pub fn review(input: &ReviewInput) -> ReviewReport {
    let mut findings = Vec::new();
    findings.push(stack_finding(input.copper_layers));
    findings.extend(gnd_pour_finding(input));
    findings.extend(rail_pour_finding(input));
    findings.extend(adjacent_planes_finding(input));
    findings.extend(cap_via_finding(input));
    findings.extend(pth_pour_finding(input));
    findings.extend(daisy_finding(input));
    findings.extend(cap_polarity_finding(input));
    findings.extend(signal_pour_finding(input));
    findings.extend(split_gnd_finding(input));

    let fails = findings
        .iter()
        .filter(|f| f.severity == Severity::Fail)
        .count();
    let warns = findings
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    let ok = fails == 0;
    let verdict = if fails > 0 {
        "needs work"
    } else if warns > 0 {
        "ok with warnings"
    } else {
        "ok"
    };
    ReviewReport {
        ok,
        verdict: verdict.into(),
        summary: summary_line(input, &findings),
        findings,
        not_checked: not_checked(),
    }
}

pub async fn review_open_board(k: &Kicad) -> Result<ReviewReport, String> {
    let summary = k.summary().await?;
    let pads = crate::pads::board_pads(k, None, None).await?;
    let vias = k.vias().await?;
    let zones = k.copper_zones().await?;
    Ok(review(&ReviewInput {
        copper_layers: summary.copper_layer_count.unwrap_or(2),
        pads: pads.into_iter().map(pad_fact).collect(),
        vias: vias
            .into_iter()
            .filter_map(|v| {
                Some(ViaFact {
                    net: v.net.unwrap_or_default(),
                    x_mm: v.x_mm?,
                    y_mm: v.y_mm?,
                })
            })
            .collect(),
        zones: zones.into_iter().map(zone_fact).collect(),
    }))
}

fn pad_fact(p: PadRow) -> PadFact {
    let layer_ids: Vec<i32> = p
        .layers
        .iter()
        .filter_map(|n| copper::parse_copper_layer(Some(n)).ok())
        .collect();
    PadFact {
        reference: p.reference,
        pin: p.pin,
        net: p.net,
        x_mm: p.x_mm,
        y_mm: p.y_mm,
        kind: p.kind,
        layer_ids,
        radius_mm: (p.width_mm.max(p.height_mm) / 2.0).max(0.2),
    }
}

fn zone_fact(z: ZoneSnap) -> ZoneFact {
    ZoneFact {
        net: z.net,
        layer_ids: z.layer_ids,
        connection: z.connection,
        fills: z.fills,
    }
}

fn net_kind(name: &str) -> NetKind {
    let n = name.trim();
    if n.is_empty() || n.eq_ignore_ascii_case("unconnected") {
        return NetKind::Empty;
    }
    let u = n.to_ascii_uppercase().replace([' ', '-'], "");
    let u = u.trim_start_matches('+');
    if u == "0V" || u == "VSS" || u == "AGND" || u == "DGND" || u == "PGND" || u.starts_with("GND")
    {
        return NetKind::Ground;
    }
    if matches!(
        u,
        "5V" | "VCC" | "VDD" | "VBAT" | "VBUS" | "VIN" | "3V3" | "3.3V" | "1V8" | "1.8V"
    ) || u.starts_with("VCC")
        || u.starts_with("VDD")
    {
        return NetKind::Rail;
    }
    NetKind::Signal
}

fn is_cap(reference: &str) -> bool {
    let mut chars = reference.chars();
    matches!(chars.next(), Some('C') | Some('c'))
        && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn stack_finding(layers: u32) -> Finding {
    if layers <= 2 {
        Finding {
            severity: Severity::Warn,
            id: "stack".into(),
            detail: "2-layer stack: return current lives on the other copper. A split or slot in GND forces a long loop.".into(),
        }
    } else {
        Finding {
            severity: Severity::Ok,
            id: "stack".into(),
            detail: format!(
                "{layers}-layer stack: inner planes can carry return next to a power pour."
            ),
        }
    }
}

fn gnd_pour_finding(input: &ReviewInput) -> Vec<Finding> {
    let gnd_pads = input
        .pads
        .iter()
        .filter(|p| net_kind(&p.net) == NetKind::Ground && p.kind != "npth")
        .count();
    if gnd_pads < 4 {
        return vec![];
    }
    let zones: Vec<_> = input
        .zones
        .iter()
        .filter(|z| net_kind(&z.net) == NetKind::Ground)
        .collect();
    if zones.is_empty() {
        return vec![Finding {
            severity: Severity::Fail,
            id: "gnd_pour".into(),
            detail: format!(
                "GND has {gnd_pads} pads but no copper pour. Return current is tracks only — pour GND (Hartley: the plane is the return)."
            ),
        }];
    }
    let layers = format_zone_layers(&zones);
    vec![Finding {
        severity: Severity::Ok,
        id: "gnd_pour".into(),
        detail: format!("GND pour on {layers}."),
    }]
}

fn rail_pour_finding(input: &ReviewInput) -> Vec<Finding> {
    let rail_pads = input
        .pads
        .iter()
        .filter(|p| net_kind(&p.net) == NetKind::Rail && p.kind != "npth")
        .count();
    if rail_pads < 8 {
        return vec![];
    }
    let zones: Vec<_> = input
        .zones
        .iter()
        .filter(|z| net_kind(&z.net) == NetKind::Rail)
        .collect();
    if zones.is_empty() {
        return vec![Finding {
            severity: Severity::Warn,
            id: "rail_pour".into(),
            detail: format!(
                "Power rail has {rail_pads} pads and no pour — IR drop and inductance stay on skinny tracks."
            ),
        }];
    }
    let only_front = zones.iter().all(|z| z.layer_ids.iter().all(|id| *id == 3));
    if only_front && input.copper_layers >= 4 {
        return vec![Finding {
            severity: Severity::Warn,
            id: "rail_pour".into(),
            detail: format!(
                "Power pour is F.Cu only ({rail_pads} pads). An inner plane is quieter and leaves F.Cu for data."
            ),
        }];
    }
    let layers = format_zone_layers(&zones);
    vec![Finding {
        severity: Severity::Ok,
        id: "rail_pour".into(),
        detail: format!("Power pour on {layers}."),
    }]
}

fn adjacent_planes_finding(input: &ReviewInput) -> Vec<Finding> {
    if input.copper_layers < 4 {
        return vec![];
    }
    let gnd_layers: Vec<i32> = input
        .zones
        .iter()
        .filter(|z| net_kind(&z.net) == NetKind::Ground)
        .flat_map(|z| z.layer_ids.iter().copied())
        .collect();
    let rail_layers: Vec<i32> = input
        .zones
        .iter()
        .filter(|z| net_kind(&z.net) == NetKind::Rail)
        .flat_map(|z| z.layer_ids.iter().copied())
        .collect();
    if gnd_layers.is_empty() || rail_layers.is_empty() {
        return vec![];
    }
    let n = input.copper_layers;
    let adjacent = rail_layers.iter().any(|r| {
        gnd_layers
            .iter()
            .any(|g| match (stack_index(*r, n), stack_index(*g, n)) {
                (Some(a), Some(b)) => a.abs_diff(b) == 1,
                _ => false,
            })
    });
    if adjacent {
        vec![Finding {
            severity: Severity::Ok,
            id: "return_loop".into(),
            detail: "5V and GND pours sit on adjacent layers — the return loop is a thin sandwich."
                .into(),
        }]
    } else {
        vec![Finding {
            severity: Severity::Warn,
            id: "return_loop".into(),
            detail: "5V and GND pours are not on adjacent layers. The displacement current takes a longer path."
                .into(),
        }]
    }
}

fn cap_via_finding(input: &ReviewInput) -> Vec<Finding> {
    let gnd_vias: Vec<&ViaFact> = input
        .vias
        .iter()
        .filter(|v| net_kind(&v.net) == NetKind::Ground)
        .collect();
    let mut caps: Vec<(&str, f64, f64)> = Vec::new();
    let mut by_ref: std::collections::BTreeMap<&str, Vec<&PadFact>> =
        std::collections::BTreeMap::new();
    for p in &input.pads {
        if is_cap(&p.reference) && p.kind != "npth" {
            by_ref.entry(p.reference.as_str()).or_default().push(p);
        }
    }
    for (r, pads) in &by_ref {
        if pads.len() != 2 {
            continue;
        }
        let kinds: Vec<NetKind> = pads.iter().map(|p| net_kind(&p.net)).collect();
        if !(kinds.contains(&NetKind::Ground) && kinds.contains(&NetKind::Rail)) {
            continue;
        }
        let gnd = pads
            .iter()
            .find(|p| net_kind(&p.net) == NetKind::Ground)
            .unwrap();
        caps.push((r, gnd.x_mm, gnd.y_mm));
    }
    if caps.is_empty() {
        return vec![];
    }
    let mut missing = Vec::new();
    let mut ok_n = 0usize;
    for (r, x, y) in &caps {
        let near = gnd_vias
            .iter()
            .any(|v| dist(v.x_mm, v.y_mm, *x, *y) <= CAP_VIA_MM);
        if near {
            ok_n += 1;
        } else {
            missing.push(*r);
        }
    }
    let n = caps.len();
    if missing.is_empty() {
        return vec![Finding {
            severity: Severity::Ok,
            id: "cap_via".into(),
            detail: format!(
                "All {n} decoupling-cap GND pads have a GND via within {CAP_VIA_MM} mm."
            ),
        }];
    }
    let show: Vec<&str> = missing.iter().copied().take(8).collect();
    let extra = if missing.len() > 8 {
        format!(" +{} more", missing.len() - 8)
    } else {
        String::new()
    };
    let severity = if missing.len() * 2 >= n && n >= 4 {
        Severity::Fail
    } else if missing.len() >= CAP_VIA_WARN_MISSING {
        Severity::Warn
    } else {
        Severity::Ok
    };
    vec![Finding {
        severity,
        id: "cap_via".into(),
        detail: format!(
            "{ok_n}/{n} cap GND pads have a via within {CAP_VIA_MM} mm. Missing: {}{extra}. The via belongs at the cap, not only at a nearby LED.",
            show.join(", ")
        ),
    }]
}

fn signal_pour_finding(input: &ReviewInput) -> Vec<Finding> {
    let signals: Vec<&str> = input
        .zones
        .iter()
        .filter(|z| net_kind(&z.net) == NetKind::Signal)
        .map(|z| z.net.as_str())
        .collect();
    if signals.is_empty() {
        return vec![];
    }
    let mut names = signals;
    names.sort_unstable();
    names.dedup();
    vec![Finding {
        severity: Severity::Warn,
        id: "signal_pour".into(),
        detail: format!(
            "Copper pour on signal net(s) {} — usually a mistake (DATA is a track, not a plane).",
            names.join(", ")
        ),
    }]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PourAttach {
    Clearance,
    Thermal,
    Solid,
    Unknown,
}

fn point_in_fill(x: f64, y: f64, contours: &[Vec<(f64, f64)>]) -> bool {
    let mut inside = false;
    for pts in contours {
        let n = pts.len();
        if n < 3 {
            continue;
        }
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = pts[i];
            let (xj, yj) = pts[j];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi + 1e-30) + xi {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

fn pour_attach(x: f64, y: f64, radius_mm: f64, contours: &[Vec<(f64, f64)>]) -> PourAttach {
    if contours.is_empty() {
        return PourAttach::Unknown;
    }
    let n = 16usize;
    let hits = |r: f64| -> usize {
        (0..n)
            .filter(|i| {
                let a = *i as f64 * std::f64::consts::TAU / n as f64;
                point_in_fill(x + r * a.cos(), y + r * a.sin(), contours)
            })
            .count()
    };
    let inner = hits(radius_mm + 0.15);
    let outer = hits(radius_mm + 0.55);
    if (2..=12).contains(&inner) {
        return PourAttach::Thermal;
    }
    if inner == 0 && outer >= n / 2 {
        return PourAttach::Clearance;
    }
    if inner >= 13 {
        return PourAttach::Solid;
    }
    if inner == 0 && outer == 0 {
        return PourAttach::Clearance;
    }
    PourAttach::Unknown
}

fn fill_for_layer<'a>(zone: &'a ZoneFact, layer: i32) -> &'a [Vec<(f64, f64)>] {
    zone.fills
        .iter()
        .find(|f| f.layer_id == layer)
        .map(|f| f.contours.as_slice())
        .unwrap_or(&[])
}

fn pth_pour_finding(input: &ReviewInput) -> Vec<Finding> {
    let power_zones: Vec<&ZoneFact> = input
        .zones
        .iter()
        .filter(|z| matches!(net_kind(&z.net), NetKind::Ground | NetKind::Rail))
        .collect();
    if power_zones.is_empty() {
        return vec![];
    }
    let pths: Vec<&PadFact> = input
        .pads
        .iter()
        .filter(|p| p.kind == "pth" && net_kind(&p.net) != NetKind::Empty)
        .collect();
    if pths.is_empty() {
        return vec![];
    }
    let have_fill = power_zones.iter().any(|z| !z.fills.is_empty());
    let mut missing = Vec::new();
    let mut unexpected = Vec::new();
    let mut ok_n = 0usize;
    for pad in &pths {
        let pad_kind = net_kind(&pad.net);
        let mut pad_ok = true;
        for zone in &power_zones {
            for &layer in &zone.layer_ids {
                let layer_name = copper::layer_name(layer);
                let same_net = pad.net == zone.net;
                let on_layer = pad.layer_ids.contains(&layer);
                if same_net && !on_layer {
                    missing.push(format!(
                        "{}.{} missing {} — {} pour cannot attach",
                        pad.reference, pad.net, layer_name, zone.net
                    ));
                    pad_ok = false;
                    continue;
                }
                if !have_fill {
                    continue;
                }
                match pour_attach(
                    pad.x_mm,
                    pad.y_mm,
                    pad.radius_mm,
                    fill_for_layer(zone, layer),
                ) {
                    PourAttach::Unknown => {}
                    PourAttach::Thermal | PourAttach::Solid if same_net => {}
                    PourAttach::Clearance if same_net => {
                        unexpected.push(format!(
                            "{}.{} clearance on {} ({}) — expected thermals",
                            pad.reference, pad.net, layer_name, zone.net
                        ));
                        pad_ok = false;
                    }
                    PourAttach::Clearance => {}
                    PourAttach::Thermal | PourAttach::Solid => {
                        unexpected.push(format!(
                            "{}.{} connected to {} on {} — should be clearance",
                            pad.reference, pad.net, zone.net, layer_name
                        ));
                        pad_ok = false;
                    }
                }
            }
        }
        if pad_ok && matches!(pad_kind, NetKind::Ground | NetKind::Rail | NetKind::Signal) {
            ok_n += 1;
        }
    }
    if missing.is_empty() && unexpected.is_empty() {
        let fill_note = if have_fill {
            "thermals on matching pours, clearance on the others"
        } else {
            "copper on matching pour layers (fill not in IPC — spokes vs clearance not sampled)"
        };
        return vec![Finding {
            severity: Severity::Ok,
            id: "pth_pour".into(),
            detail: format!("{ok_n} PTH pad(s) vs power pours: {fill_note}."),
        }];
    }
    let mut problems = missing;
    problems.extend(unexpected);
    let show: Vec<&str> = problems.iter().map(String::as_str).take(8).collect();
    let extra = if problems.len() > 8 {
        format!(" +{} more", problems.len() - 8)
    } else {
        String::new()
    };
    vec![Finding {
        severity: Severity::Fail,
        id: "pth_pour".into(),
        detail: format!("{}{extra}.", show.join("; ")),
    }]
}

#[derive(Debug, Clone)]
struct LedCell {
    reference: String,
    pin1_x: f64,
    pin1_y: f64,
    pin2_net: String,
    pin4_net: String,
}

fn pad_of<'a>(pads: &[&'a PadFact], pin: &str) -> Option<&'a PadFact> {
    pads.iter().copied().find(|p| p.pin == pin)
}

fn led_cells(input: &ReviewInput) -> Vec<LedCell> {
    let mut by_ref: std::collections::BTreeMap<&str, Vec<&PadFact>> =
        std::collections::BTreeMap::new();
    for p in &input.pads {
        if p.kind != "npth" {
            by_ref.entry(p.reference.as_str()).or_default().push(p);
        }
    }
    let mut out = Vec::new();
    for (r, pads) in by_ref {
        if pads.len() != 4 {
            continue;
        }
        let Some(p1) = pad_of(&pads, "1") else { continue };
        let Some(p2) = pad_of(&pads, "2") else { continue };
        let Some(p3) = pad_of(&pads, "3") else { continue };
        if pad_of(&pads, "4").is_none() {
            continue;
        }
        if net_kind(&p1.net) != NetKind::Ground || net_kind(&p3.net) != NetKind::Rail {
            continue;
        }
        out.push(LedCell {
            reference: r.to_string(),
            pin1_x: p1.x_mm,
            pin1_y: p1.y_mm,
            pin2_net: p2.net.clone(),
            pin4_net: pad_of(&pads, "4").unwrap().net.clone(),
        });
    }
    out
}

fn net_others<'a>(
    input: &'a ReviewInput,
    net: &str,
    self_ref: &str,
    self_pin: &str,
) -> Vec<&'a PadFact> {
    if net_kind(net) == NetKind::Empty {
        return vec![];
    }
    input
        .pads
        .iter()
        .filter(|p| p.net == net && !(p.reference == self_ref && p.pin == self_pin))
        .collect()
}

fn is_led_pin(leds: &std::collections::HashSet<&str>, pad: &PadFact, pin: &str) -> bool {
    leds.contains(pad.reference.as_str()) && pad.pin == pin
}

fn daisy_finding(input: &ReviewInput) -> Vec<Finding> {
    let cells = led_cells(input);
    if cells.is_empty() {
        return vec![];
    }
    let led_refs: std::collections::HashSet<&str> =
        cells.iter().map(|c| c.reference.as_str()).collect();
    let mut starts = Vec::new();
    for cell in &cells {
        let others = net_others(input, &cell.pin2_net, &cell.reference, "2");
        let from_led_out = others.iter().any(|p| is_led_pin(&led_refs, p, "4"));
        if !from_led_out {
            starts.push(cell.reference.as_str());
        }
    }
    if starts.len() != 1 {
        return vec![Finding {
            severity: Severity::Fail,
            id: "daisy".into(),
            detail: format!(
                "{} LED cells but {} daisy start(s) (DIN not fed by another LED DOUT): {}. Need exactly one.",
                cells.len(),
                starts.len(),
                starts.join(", ")
            ),
        }];
    }
    let start = starts[0];
    let by_ref: std::collections::HashMap<&str, &LedCell> =
        cells.iter().map(|c| (c.reference.as_str(), c)).collect();
    let mut chain = vec![start.to_string()];
    let mut seen = std::collections::HashSet::new();
    seen.insert(start);
    let mut cur = start;
    loop {
        let cell = by_ref[cur];
        let others = net_others(input, &cell.pin4_net, cur, "4");
        let nexts: Vec<&&PadFact> = others
            .iter()
            .filter(|p| is_led_pin(&led_refs, p, "2"))
            .collect();
        if nexts.is_empty() {
            break;
        }
        if nexts.len() != 1 || others.len() != 1 {
            return vec![Finding {
                severity: Severity::Fail,
                id: "daisy".into(),
                detail: format!(
                    "{cur}.4 hop is not a single DOUT→DIN (net {}).",
                    cell.pin4_net
                ),
            }];
        }
        let next = nexts[0].reference.as_str();
        if !seen.insert(next) {
            return vec![Finding {
                severity: Severity::Fail,
                id: "daisy".into(),
                detail: format!("Daisy cycles at {next}."),
            }];
        }
        chain.push(next.to_string());
        cur = next;
    }
    let leftover: Vec<&str> = cells
        .iter()
        .map(|c| c.reference.as_str())
        .filter(|r| !seen.contains(r))
        .collect();
    if !leftover.is_empty() {
        let show: Vec<&str> = leftover.iter().copied().take(8).collect();
        let extra = if leftover.len() > 8 {
            format!(" +{} more", leftover.len() - 8)
        } else {
            String::new()
        };
        return vec![Finding {
            severity: Severity::Fail,
            id: "daisy".into(),
            detail: format!(
                "{}/{} LEDs in the daisy from {}. Left out: {}{extra}.",
                chain.len(),
                cells.len(),
                start,
                show.join(", ")
            ),
        }];
    }
    let last = chain.last().unwrap();
    vec![Finding {
        severity: Severity::Ok,
        id: "daisy".into(),
        detail: format!(
            "{} LEDs in one daisy: {start}.2 ← … → {last}.4 (open).",
            chain.len()
        ),
    }]
}

fn cap_polarity_finding(input: &ReviewInput) -> Vec<Finding> {
    let leds = led_cells(input);
    if leds.is_empty() {
        return vec![];
    }
    let mut by_ref: std::collections::BTreeMap<&str, Vec<&PadFact>> =
        std::collections::BTreeMap::new();
    for p in &input.pads {
        if is_cap(&p.reference) && p.kind != "npth" {
            by_ref.entry(p.reference.as_str()).or_default().push(p);
        }
    }
    let mut checked = 0usize;
    let mut swapped = Vec::new();
    for (r, pads) in &by_ref {
        if pads.len() != 2 {
            continue;
        }
        let kinds: Vec<NetKind> = pads.iter().map(|p| net_kind(&p.net)).collect();
        if !(kinds.contains(&NetKind::Ground) && kinds.contains(&NetKind::Rail)) {
            continue;
        }
        if !pads
            .iter()
            .all(|p| p.kind == "smd" && p.radius_mm <= CAP_DECOUPLE_PAD_RADIUS_MM)
        {
            continue;
        }
        let Some((near_led, d_led)) = leds
            .iter()
            .map(|led| {
                let d = pads
                    .iter()
                    .map(|p| dist(p.x_mm, p.y_mm, led.pin1_x, led.pin1_y))
                    .fold(f64::INFINITY, f64::min);
                (led, d)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
        else {
            continue;
        };
        if d_led > CAP_COMPANION_MM {
            continue;
        }
        checked += 1;
        let closer = pads.iter().min_by(|a, b| {
            dist(a.x_mm, a.y_mm, near_led.pin1_x, near_led.pin1_y)
                .total_cmp(&dist(b.x_mm, b.y_mm, near_led.pin1_x, near_led.pin1_y))
        });
        if closer.is_some_and(|p| net_kind(&p.net) != NetKind::Ground) {
            swapped.push(*r);
        }
    }
    if checked == 0 {
        return vec![];
    }
    if swapped.is_empty() {
        return vec![Finding {
            severity: Severity::Ok,
            id: "cap_polarity".into(),
            detail: format!(
                "All {checked} decoupling caps have the GND pad next to the companion LED pin 1."
            ),
        }];
    }
    let show: Vec<&str> = swapped.iter().copied().take(8).collect();
    let extra = if swapped.len() > 8 {
        format!(" +{} more", swapped.len() - 8)
    } else {
        String::new()
    };
    vec![Finding {
        severity: Severity::Fail,
        id: "cap_polarity".into(),
        detail: format!(
            "{}/{} caps have GND next to LED pin 1. Swapped (5V pad closer): {}{extra}. EasyEDA 1/2 has no polarity — GND is the pad beside the LED GND.",
            checked - swapped.len(),
            checked,
            show.join(", ")
        ),
    }]
}

fn split_gnd_finding(input: &ReviewInput) -> Vec<Finding> {
    let mut names: Vec<String> = input
        .pads
        .iter()
        .filter(|p| net_kind(&p.net) == NetKind::Ground)
        .map(|p| p.net.clone())
        .collect();
    names.sort();
    names.dedup();
    if names.len() < 2 {
        return vec![];
    }
    vec![Finding {
        severity: Severity::Warn,
        id: "split_gnd".into(),
        detail: format!(
            "Split grounds: {}. Don't split unless you can name how return current crosses the gap.",
            names.join(", ")
        ),
    }]
}

fn format_zone_layers(zones: &[&ZoneFact]) -> String {
    let mut names: Vec<String> = zones
        .iter()
        .flat_map(|z| z.layer_ids.iter().copied())
        .map(copper::layer_name)
        .collect();
    names.sort();
    names.dedup();
    if names.is_empty() {
        "?".into()
    } else {
        names.join(", ")
    }
}

/// F.Cu=0 … B.Cu=n-1 in the physical stack (In2 and B.Cu are adjacent on 4 layers).
fn stack_index(layer_id: i32, copper_count: u32) -> Option<u32> {
    const BL_F_CU: i32 = 3;
    const BL_B_CU: i32 = 34;
    let n = copper_count.max(2);
    if layer_id == BL_F_CU {
        return Some(0);
    }
    if layer_id == BL_B_CU {
        return Some(n - 1);
    }
    if (4..=33).contains(&layer_id) {
        let idx = (layer_id - 3) as u32;
        if idx < n - 1 {
            return Some(idx);
        }
    }
    None
}

fn dist(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let dx = ax - bx;
    let dy = ay - by;
    (dx * dx + dy * dy).sqrt()
}

fn summary_line(input: &ReviewInput, findings: &[Finding]) -> String {
    let gnd_vias = input
        .vias
        .iter()
        .filter(|v| net_kind(&v.net) == NetKind::Ground)
        .count();
    let rail_vias = input
        .vias
        .iter()
        .filter(|v| net_kind(&v.net) == NetKind::Rail)
        .count();
    let fails = findings
        .iter()
        .filter(|f| f.severity == Severity::Fail)
        .count();
    let warns = findings
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    format!(
        "{}-layer, {} pads, {} GND vias, {} rail vias, {} pours. {} fail / {} warn.",
        input.copper_layers,
        input.pads.len(),
        gnd_vias,
        rail_vias,
        input.zones.len(),
        fails,
        warns
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(r: &str, net: &str, x: f64, y: f64) -> PadFact {
        pad_at(r, "1", net, x, y)
    }

    fn pad_at(r: &str, pin: &str, net: &str, x: f64, y: f64) -> PadFact {
        PadFact {
            reference: r.into(),
            pin: pin.into(),
            net: net.into(),
            x_mm: x,
            y_mm: y,
            kind: "smd".into(),
            layer_ids: vec![3],
            radius_mm: 0.4,
        }
    }

    fn pth(r: &str, net: &str, x: f64, y: f64, layers: &[i32]) -> PadFact {
        PadFact {
            reference: r.into(),
            pin: "1".into(),
            net: net.into(),
            x_mm: x,
            y_mm: y,
            kind: "pth".into(),
            layer_ids: layers.to_vec(),
            radius_mm: 1.4,
        }
    }

    /// SK6812 cell: 1=GND, 2=DIN, 3=5V, 4=DOUT.
    fn led(r: &str, x: f64, y: f64, din: &str, dout: &str) -> Vec<PadFact> {
        vec![
            pad_at(r, "1", "GND", x - 1.0, y - 1.0),
            pad_at(r, "2", din, x + 1.0, y - 1.0),
            pad_at(r, "3", "5V", x + 1.0, y + 1.0),
            pad_at(r, "4", dout, x - 1.0, y + 1.0),
        ]
    }

    fn via(net: &str, x: f64, y: f64) -> ViaFact {
        ViaFact {
            net: net.into(),
            x_mm: x,
            y_mm: y,
        }
    }

    fn zone(net: &str, layers: &[i32]) -> ZoneFact {
        ZoneFact {
            net: net.into(),
            layer_ids: layers.to_vec(),
            connection: ZoneConnectStyle::PthThermal,
            fills: vec![],
        }
    }

    fn square(cx: f64, cy: f64, half: f64) -> Vec<(f64, f64)> {
        vec![
            (cx - half, cy - half),
            (cx + half, cy - half),
            (cx + half, cy + half),
            (cx - half, cy + half),
        ]
    }

    /// Typical 4-layer LED cell: In1 5V, In2+B GND, via at the cap.
    fn good_panel() -> ReviewInput {
        let mut pads = vec![
            pad("U1", "GND", 0.0, 0.0),
            pad("U1", "5V", 1.0, 0.0),
            pad("C1", "5V", 2.0, 0.5),
            pad("C1", "GND", 2.8, 0.5),
        ];
        for i in 2..12 {
            pads.push(pad(&format!("U{i}"), "GND", i as f64 * 3.0, 0.0));
            pads.push(pad(&format!("U{i}"), "5V", i as f64 * 3.0 + 1.0, 0.0));
        }
        ReviewInput {
            copper_layers: 4,
            pads,
            vias: vec![via("GND", 2.9, 0.6), via("5V", 2.1, 0.5)],
            zones: vec![zone("5V", &[4]), zone("GND", &[5]), zone("GND", &[34])],
        }
    }

    #[test]
    fn good_four_layer_is_ok() {
        let r = review(&good_panel());
        assert!(r.ok, "{r:?}");
        assert_eq!(r.verdict, "ok");
        assert!(r.findings.iter().all(|f| f.severity == Severity::Ok));
        assert!(r.not_checked.iter().any(|s| s.contains("90°")));
    }

    #[test]
    fn two_layer_warns_about_return() {
        let mut i = good_panel();
        i.copper_layers = 2;
        i.zones = vec![zone("GND", &[34]), zone("5V", &[3])];
        let r = review(&i);
        assert!(r.ok);
        assert_eq!(r.verdict, "ok with warnings");
        assert!(r
            .findings
            .iter()
            .any(|f| f.id == "stack" && f.severity == Severity::Warn));
    }

    #[test]
    fn missing_gnd_pour_fails() {
        let mut i = good_panel();
        i.zones.retain(|z| net_kind(&z.net) != NetKind::Ground);
        let r = review(&i);
        assert!(!r.ok);
        assert_eq!(r.verdict, "needs work");
        assert!(r
            .findings
            .iter()
            .any(|f| f.id == "gnd_pour" && f.severity == Severity::Fail));
    }

    #[test]
    fn cap_without_nearby_via_warns() {
        let mut i = good_panel();
        i.vias.clear();
        let r = review(&i);
        let cap = r.findings.iter().find(|f| f.id == "cap_via").unwrap();
        assert_eq!(cap.severity, Severity::Warn);
        assert!(cap.detail.contains("C1"));
    }

    #[test]
    fn data_pour_is_a_warning() {
        let mut i = good_panel();
        i.zones.push(zone("DATA_IN", &[3]));
        let r = review(&i);
        assert!(r
            .findings
            .iter()
            .any(|f| f.id == "signal_pour" && f.detail.contains("DATA_IN")));
    }

    #[test]
    fn split_grounds_warn() {
        let mut i = good_panel();
        i.pads.push(pad("U20", "AGND", 50.0, 0.0));
        let r = review(&i);
        assert!(r.findings.iter().any(|f| f.id == "split_gnd"));
    }

    #[test]
    fn net_kinds() {
        assert_eq!(net_kind("GND"), NetKind::Ground);
        assert_eq!(net_kind("gnd"), NetKind::Ground);
        assert_eq!(net_kind("5V"), NetKind::Rail);
        assert_eq!(net_kind("DIN"), NetKind::Signal);
        assert_eq!(net_kind(""), NetKind::Empty);
    }

    #[tokio::test]
    #[ignore = "needs a running KiCad PCB editor with IPC API"]
    async fn live_open_board() {
        let k = crate::kicad::Kicad::connect().await.expect("KiCad IPC");
        let r = crate::review::review_open_board(&k).await.expect("review");
        eprintln!("{}", serde_json::to_string_pretty(&r).unwrap());
        assert!(!r.findings.is_empty());
    }

    #[test]
    fn in2_and_back_are_adjacent_on_four_layer() {
        assert_eq!(stack_index(5, 4), Some(2));
        assert_eq!(stack_index(34, 4), Some(3));
        assert_eq!(stack_index(3, 4), Some(0));
        assert_eq!(stack_index(4, 4), Some(1));
    }

    #[test]
    fn fivev_pth_missing_in1_fails() {
        let mut i = good_panel();
        i.pads.push(pth("W1", "5V", 0.0, 10.0, &[3, 34]));
        let r = review(&i);
        let f = r.findings.iter().find(|f| f.id == "pth_pour").unwrap();
        assert_eq!(f.severity, Severity::Fail);
        assert!(f.detail.contains("W1"));
        assert!(f.detail.contains("In1.Cu"));
    }

    #[test]
    fn power_pth_on_pour_layers_is_ok_without_fill() {
        let mut i = good_panel();
        i.pads.push(pth("W1", "5V", 0.0, 10.0, &[3, 4, 5, 34]));
        i.pads.push(pth("W2", "GND", 10.0, 10.0, &[3, 4, 5, 34]));
        i.pads
            .push(pth("W3", "DATA_IN", 20.0, 10.0, &[3, 4, 5, 34]));
        let r = review(&i);
        let f = r.findings.iter().find(|f| f.id == "pth_pour").unwrap();
        assert_eq!(f.severity, Severity::Ok, "{f:?}");
        assert!(f.detail.contains("fill not in IPC"));
    }

    #[test]
    fn fill_classifies_clearance_and_thermal() {
        // Donut: 20 mm square with a 3.2 mm hole — clearance around a 1.4 mm pad.
        let donut = vec![square(0.0, 0.0, 10.0), square(0.0, 0.0, 1.6)];
        assert_eq!(pour_attach(0.0, 0.0, 1.4, &donut), PourAttach::Clearance);

        // Cross of spokes through the gap (1.4+0.15 ring hits 4 of 16).
        let mut thermal = donut.clone();
        thermal.push(vec![(-0.2, -3.0), (0.2, -3.0), (0.2, 3.0), (-0.2, 3.0)]);
        thermal.push(vec![(-3.0, -0.2), (3.0, -0.2), (3.0, 0.2), (-3.0, 0.2)]);
        assert_eq!(pour_attach(0.0, 0.0, 1.4, &thermal), PourAttach::Thermal);
    }

    #[test]
    fn signal_pth_thermal_into_gnd_fails() {
        let mut i = good_panel();
        i.pads.push(pth("W3", "DATA_IN", 0.0, 0.0, &[3, 4, 5, 34]));
        i.zones
            .iter_mut()
            .find(|z| z.net == "GND" && z.layer_ids == vec![5])
            .unwrap()
            .fills = vec![copper::ZoneFill {
            layer_id: 5,
            contours: vec![
                square(0.0, 0.0, 10.0),
                square(0.0, 0.0, 1.6),
                vec![(-0.2, -3.0), (0.2, -3.0), (0.2, 3.0), (-0.2, 3.0)],
                vec![(-3.0, -0.2), (3.0, -0.2), (3.0, 0.2), (-3.0, 0.2)],
            ],
        }];
        let r = review(&i);
        let f = r.findings.iter().find(|f| f.id == "pth_pour").unwrap();
        assert_eq!(f.severity, Severity::Fail);
        assert!(f.detail.contains("DATA_IN"));
    }

    fn daisy_panel(extra: Vec<PadFact>) -> ReviewInput {
        let mut i = good_panel();
        i.pads.extend(extra);
        i
    }

    #[test]
    fn three_led_daisy_is_ok() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "H1");
        pads.extend(led("U31", 90.0, 0.0, "H1", "H2"));
        pads.extend(led("U32", 100.0, 0.0, "H2", ""));
        let r = review(&daisy_panel(pads));
        let f = r.findings.iter().find(|f| f.id == "daisy").unwrap();
        assert_eq!(f.severity, Severity::Ok, "{f:?}");
        assert!(f.detail.contains("3 LEDs"));
        assert!(f.detail.contains("U30.2"));
        assert!(f.detail.contains("U32.4"));
    }

    #[test]
    fn branched_daisy_hop_fails() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "H1");
        pads.extend(led("U31", 90.0, 0.0, "H1", "H2"));
        pads.extend(led("U32", 100.0, 0.0, "H1", ""));
        let r = review(&daisy_panel(pads));
        let f = r.findings.iter().find(|f| f.id == "daisy").unwrap();
        assert_eq!(f.severity, Severity::Fail, "{f:?}");
        assert!(f.detail.contains("U30.4") || f.detail.contains("start"));
    }

    #[test]
    fn leftover_led_cycle_fails() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "H1");
        pads.extend(led("U31", 90.0, 0.0, "H1", ""));
        pads.extend(led("U32", 110.0, 0.0, "CY", "CY2"));
        pads.extend(led("U33", 120.0, 0.0, "CY2", "CY"));
        let r = review(&daisy_panel(pads));
        let f = r.findings.iter().find(|f| f.id == "daisy").unwrap();
        assert_eq!(f.severity, Severity::Fail, "{f:?}");
        assert!(f.detail.contains("Left out") || f.detail.contains("U32"));
    }

    #[test]
    fn cap_gnd_beside_led_pin1_is_ok() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "");
        // LED pin 1 at (79, -1). GND closer than 5V.
        pads.push(pad_at("C20", "1", "5V", 81.0, 0.0));
        pads.push(pad_at("C20", "2", "GND", 78.8, -1.0));
        let r = review(&daisy_panel(pads));
        let f = r.findings.iter().find(|f| f.id == "cap_polarity").unwrap();
        assert_eq!(f.severity, Severity::Ok, "{f:?}");
    }

    #[test]
    fn cap_5v_beside_led_pin1_fails() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "");
        pads.push(pad_at("C20", "1", "5V", 78.8, -1.0));
        pads.push(pad_at("C20", "2", "GND", 83.0, 0.0));
        let r = review(&daisy_panel(pads));
        let f = r.findings.iter().find(|f| f.id == "cap_polarity").unwrap();
        assert_eq!(f.severity, Severity::Fail, "{f:?}");
        assert!(f.detail.contains("C20"));
    }

    #[test]
    fn bulk_cap_far_from_led_is_skipped() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "");
        pads.push(pad_at("C99", "1", "5V", 200.0, 200.0));
        pads.push(pad_at("C99", "2", "GND", 201.0, 200.0));
        let r = review(&daisy_panel(pads));
        assert!(r.findings.iter().all(|f| f.id != "cap_polarity"));
    }

    #[test]
    fn bulk_cap_beside_led_is_skipped() {
        let mut pads = led("U30", 80.0, 0.0, "IN", "");
        // Same pad nets as the fail case, but polymer-sized pads (C211).
        let mut a = pad_at("C211", "1", "5V", 78.8, -1.0);
        let mut b = pad_at("C211", "2", "GND", 83.0, 0.0);
        a.radius_mm = 1.2;
        b.radius_mm = 1.2;
        pads.push(a);
        pads.push(b);
        let r = review(&daisy_panel(pads));
        assert!(r.findings.iter().all(|f| f.id != "cap_polarity"));
    }
}
