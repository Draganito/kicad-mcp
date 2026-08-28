//! Short layout-physics review. Reads the open board; does not place copper.
//!
//! Checks return path (ground pour), power pour, plane adjacency, and a via
//! next to each decoupling-cap GND pad. Does **not** nag about 90° corners,
//! silk overlap, or DRC clearance — those are myths or `check_drc`.

use serde::Serialize;

use crate::copper::{self, ZoneSnap};
use crate::kicad::Kicad;
use crate::pads::PadRow;

const CAP_VIA_MM: f64 = 3.0;
const CAP_VIA_WARN_MISSING: usize = 1;

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
    pub net: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub kind: String,
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
    PadFact {
        reference: p.reference,
        net: p.net,
        x_mm: p.x_mm,
        y_mm: p.y_mm,
        kind: p.kind,
    }
}

fn zone_fact(z: ZoneSnap) -> ZoneFact {
    ZoneFact {
        net: z.net,
        layer_ids: z.layer_ids,
    }
}

fn net_kind(name: &str) -> NetKind {
    let n = name.trim();
    if n.is_empty() || n.eq_ignore_ascii_case("unconnected") {
        return NetKind::Empty;
    }
    let u = n.to_ascii_uppercase().replace([' ', '-'], "");
    let u = u.trim_start_matches('+');
    if u == "0V"
        || u == "VSS"
        || u == "AGND"
        || u == "DGND"
        || u == "PGND"
        || u.starts_with("GND")
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
    matches!(chars.next(), Some('C') | Some('c')) && chars.next().is_some_and(|c| c.is_ascii_digit())
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
        gnd_layers.iter().any(|g| {
            match (stack_index(*r, n), stack_index(*g, n)) {
                (Some(a), Some(b)) => a.abs_diff(b) == 1,
                _ => false,
            }
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
        let near = gnd_vias.iter().any(|v| dist(v.x_mm, v.y_mm, *x, *y) <= CAP_VIA_MM);
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
        PadFact {
            reference: r.into(),
            net: net.into(),
            x_mm: x,
            y_mm: y,
            kind: "smd".into(),
        }
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
        }
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
            zones: vec![
                zone("5V", &[4]),
                zone("GND", &[5]),
                zone("GND", &[34]),
            ],
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
}
