//! Pin coverage — the ERC substitute for a schematic-less workflow.
//!
//! `connect_many` builds nets straight on the board, so nothing forces an
//! agent to account for every pin the way a schematic ERC would. This module
//! closes that gap: every electrical pin must either carry a net or be
//! explicitly allowed open (`allow: ["U1.5", …]`). It also flags nets that
//! reach exactly one pin — netted, but connecting nothing.
//!
//! Pure logic over simple rows; the MCP glue feeds it baked board pads plus
//! EasyEDA pin names (best effort, for the report only — a floating pin
//! named `1OE#` reads very differently from a floating `NC`).

use serde::Serialize;

use std::collections::{BTreeMap, BTreeSet};

/// One board pad, as assembled by the MCP glue from `board_pads`.
#[derive(Debug, Clone)]
pub struct PinRow {
    pub reference: String,
    pub pin: String,
    /// Net name; empty / `unconnected` / `unconnected-…` counts as open.
    pub net: String,
    /// EasyEDA pin name when the template provides one (`GND`, `DIN`, `NC`).
    pub pin_name: Option<String>,
    pub x_mm: f64,
    pub y_mm: f64,
    /// `smd`, `pth`, or `npth`. NPTH pads have no electrical function.
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OpenPin {
    pub reference: String,
    pub pin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_name: Option<String>,
    pub x_mm: f64,
    pub y_mm: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SinglePadNet {
    pub net: String,
    /// The one pin the net reaches, as `REF.PIN`.
    pub pad: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageReport {
    pub ok: bool,
    pub verdict: String,
    /// Unique electrical pins (`REF.PIN`; NPTH and unnumbered pads excluded).
    pub pin_count: usize,
    pub connected_count: usize,
    /// Open pins with no `allow` entry — each one needs a justification.
    pub open_pins: Vec<OpenPin>,
    /// Open pins covered by the `allow` list (intentionally unconnected).
    pub allowed_open: Vec<OpenPin>,
    /// `allow` entries that matched no open pin — typo or stale entry.
    pub allow_unmatched: Vec<String>,
    /// Nets that reach exactly one pin: netted, but connecting nothing.
    pub single_pad_nets: Vec<SinglePadNet>,
    /// NPTH / unnumbered mechanical pads that were skipped.
    pub skipped_mechanical: usize,
    pub note: String,
}

fn is_open(net: &str) -> bool {
    net.is_empty() || net == "unconnected" || net.starts_with("unconnected-")
}

/// Natural sort key: `U10` after `U2`, pins numerically when possible.
fn nat_key(s: &str) -> (String, u64, String) {
    let alpha: String = s.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let rest = &s[alpha.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let tail = rest[digits.len()..].to_string();
    (alpha, digits.parse().unwrap_or(0), tail)
}

/// Coverage over every electrical pin. `allow` holds `REF.PIN` entries
/// (case-insensitive) for pins that are intentionally open.
pub fn coverage(rows: &[PinRow], allow: &[String]) -> CoverageReport {
    let allow_norm: Vec<(String, String)> = allow
        .iter()
        .map(|a| (a.trim().to_ascii_uppercase(), a.trim().to_string()))
        .filter(|(k, _)| !k.is_empty())
        .collect();

    // Group pads by REF.PIN — thermal clusters share a pin number and are
    // spliced together by connect_pins, so one netted pad covers the pin.
    let mut groups: BTreeMap<(String, String), Vec<&PinRow>> = BTreeMap::new();
    let mut skipped_mechanical = 0usize;
    for row in rows {
        if row.kind == "npth" || row.pin.trim().is_empty() {
            skipped_mechanical += 1;
            continue;
        }
        groups
            .entry((row.reference.clone(), row.pin.clone()))
            .or_default()
            .push(row);
    }

    let pin_count = groups.len();
    let mut connected_count = 0usize;
    let mut open: Vec<OpenPin> = Vec::new();
    // Unique pins per real net, to spot nets that connect exactly one pin.
    let mut net_pins: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();

    for ((reference, pin), pads) in &groups {
        let net = pads.iter().map(|p| p.net.as_str()).find(|n| !is_open(n));
        match net {
            Some(net) => {
                connected_count += 1;
                net_pins
                    .entry(net.to_string())
                    .or_default()
                    .insert((reference.clone(), pin.clone()));
            }
            None => {
                let first = pads[0];
                open.push(OpenPin {
                    reference: reference.clone(),
                    pin: pin.clone(),
                    pin_name: first.pin_name.clone(),
                    x_mm: first.x_mm,
                    y_mm: first.y_mm,
                });
            }
        }
    }

    open.sort_by_key(|p| (nat_key(&p.reference), nat_key(&p.pin)));

    let mut matched_allow: BTreeSet<String> = BTreeSet::new();
    let (allowed_open, open_pins): (Vec<OpenPin>, Vec<OpenPin>) =
        open.into_iter().partition(|p| {
            let key = format!("{}.{}", p.reference, p.pin).to_ascii_uppercase();
            let hit = allow_norm.iter().find(|(k, _)| *k == key);
            if let Some((k, _)) = hit {
                matched_allow.insert(k.clone());
                true
            } else {
                false
            }
        });

    let allow_unmatched: Vec<String> = allow_norm
        .iter()
        .filter(|(k, _)| !matched_allow.contains(k))
        .map(|(_, orig)| orig.clone())
        .collect();

    let mut single_pad_nets: Vec<SinglePadNet> = net_pins
        .iter()
        .filter(|(_, pins)| pins.len() == 1)
        .map(|(net, pins)| {
            let (reference, pin) = pins.iter().next().expect("len checked").clone();
            let pin_name = groups
                .get(&(reference.clone(), pin.clone()))
                .and_then(|pads| pads[0].pin_name.clone());
            SinglePadNet {
                net: net.clone(),
                pad: format!("{reference}.{pin}"),
                pin_name,
            }
        })
        .collect();
    single_pad_nets.sort_by_key(|s| nat_key(&s.pad));

    let ok = open_pins.is_empty() && single_pad_nets.is_empty() && allow_unmatched.is_empty();
    let verdict = if ok {
        format!(
            "every one of {pin_count} pins is netted or explicitly allowed open ({} allowed)",
            allowed_open.len()
        )
    } else {
        let mut parts = Vec::new();
        if !open_pins.is_empty() {
            parts.push(format!("{} open pin(s) without justification", open_pins.len()));
        }
        if !single_pad_nets.is_empty() {
            parts.push(format!(
                "{} net(s) reaching only one pin",
                single_pad_nets.len()
            ));
        }
        if !allow_unmatched.is_empty() {
            parts.push(format!(
                "{} allow entr(y/ies) matching nothing",
                allow_unmatched.len()
            ));
        }
        parts.join("; ")
    };

    CoverageReport {
        ok,
        verdict,
        pin_count,
        connected_count,
        open_pins,
        allowed_open,
        allow_unmatched,
        single_pad_nets,
        skipped_mechanical,
        note: "Every open pin needs a reason: NC per its pin_name, or an allow entry after \
               checking the part's function. Do not silently accept floating enables, \
               inputs, or pad-1 corners."
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(reference: &str, pin: &str, net: &str, pin_name: Option<&str>, kind: &str) -> PinRow {
        PinRow {
            reference: reference.into(),
            pin: pin.into(),
            net: net.into(),
            pin_name: pin_name.map(Into::into),
            x_mm: 0.0,
            y_mm: 0.0,
            kind: kind.into(),
        }
    }

    #[test]
    fn all_connected_is_ok() {
        let rows = vec![
            row("U1", "1", "GND", Some("GND"), "smd"),
            row("U1", "2", "DATA", Some("DIN"), "smd"),
            row("C1", "1", "GND", None, "smd"),
            row("C1", "2", "DATA", None, "smd"),
        ];
        let rep = coverage(&rows, &[]);
        assert!(rep.ok, "{}", rep.verdict);
        assert_eq!(rep.pin_count, 4);
        assert_eq!(rep.connected_count, 4);
        assert!(rep.open_pins.is_empty());
        assert!(rep.single_pad_nets.is_empty());
    }

    #[test]
    fn open_pin_is_reported_with_pin_name() {
        let rows = vec![
            row("U1", "1", "GND", Some("GND"), "smd"),
            row("U1", "2", "", Some("1OE#"), "smd"),
            row("C1", "1", "GND", None, "smd"),
        ];
        let rep = coverage(&rows, &[]);
        assert!(!rep.ok);
        assert_eq!(rep.open_pins.len(), 1);
        assert_eq!(rep.open_pins[0].reference, "U1");
        assert_eq!(rep.open_pins[0].pin, "2");
        assert_eq!(rep.open_pins[0].pin_name.as_deref(), Some("1OE#"));
    }

    #[test]
    fn allowed_open_pin_is_ok_and_listed() {
        let rows = vec![
            row("U1", "1", "GND", Some("GND"), "smd"),
            row("U1", "2", "", Some("NC"), "smd"),
            row("U2", "1", "GND", None, "smd"),
        ];
        let rep = coverage(&rows, &["u1.2".into()]);
        assert!(rep.ok, "{}", rep.verdict);
        assert_eq!(rep.allowed_open.len(), 1);
        assert_eq!(rep.allowed_open[0].pin_name.as_deref(), Some("NC"));
        assert!(rep.open_pins.is_empty());
        assert!(rep.allow_unmatched.is_empty());
    }

    #[test]
    fn stale_allow_entry_fails() {
        let rows = vec![row("U1", "1", "GND", None, "smd")];
        let rep = coverage(&rows, &["U9.4".into()]);
        assert!(!rep.ok);
        assert_eq!(rep.allow_unmatched, vec!["U9.4".to_string()]);
    }

    #[test]
    fn single_pad_net_is_flagged() {
        let rows = vec![
            row("U1", "1", "GND", None, "smd"),
            row("U1", "2", "DATA", Some("DOUT"), "smd"),
            row("U2", "1", "GND", None, "smd"),
        ];
        let rep = coverage(&rows, &[]);
        assert!(!rep.ok);
        assert_eq!(rep.single_pad_nets.len(), 1);
        assert_eq!(rep.single_pad_nets[0].net, "DATA");
        assert_eq!(rep.single_pad_nets[0].pad, "U1.2");
        assert_eq!(rep.single_pad_nets[0].pin_name.as_deref(), Some("DOUT"));
    }

    #[test]
    fn thermal_cluster_counts_once_and_covers_by_any_pad() {
        // Pin 41 exists as three pads; only one carries the net (splice
        // assigns all, but the report must not depend on that).
        let rows = vec![
            row("U1", "41", "GND", Some("EP"), "smd"),
            row("U1", "41", "", Some("EP"), "smd"),
            row("U1", "41", "", Some("EP"), "smd"),
            row("U2", "41", "GND", Some("EP"), "smd"),
        ];
        let rep = coverage(&rows, &[]);
        assert_eq!(rep.pin_count, 2);
        assert_eq!(rep.connected_count, 2);
        assert!(rep.single_pad_nets.is_empty(), "GND reaches two pins");
        assert!(rep.ok, "{}", rep.verdict);
    }

    #[test]
    fn npth_and_unnumbered_pads_are_skipped() {
        let rows = vec![
            row("H1", "", "", None, "npth"),
            row("H2", "1", "", None, "npth"),
            row("U1", "", "", None, "smd"),
            row("U1", "1", "GND", None, "smd"),
            row("U2", "1", "GND", None, "smd"),
        ];
        let rep = coverage(&rows, &[]);
        assert_eq!(rep.skipped_mechanical, 3);
        assert_eq!(rep.pin_count, 2);
        assert!(rep.ok, "{}", rep.verdict);
    }

    #[test]
    fn kicad_unconnected_net_names_count_as_open() {
        let rows = vec![
            row("U1", "1", "unconnected-(U1-Pad1)", None, "smd"),
            row("U2", "1", "GND", None, "smd"),
            row("U3", "1", "GND", None, "smd"),
        ];
        let rep = coverage(&rows, &[]);
        assert_eq!(rep.open_pins.len(), 1);
        assert_eq!(rep.open_pins[0].reference, "U1");
    }

    #[test]
    fn natural_ordering_of_open_pins() {
        let rows = vec![
            row("U10", "1", "", None, "smd"),
            row("U2", "1", "", None, "smd"),
            row("U2", "10", "", None, "smd"),
            row("U2", "9", "", None, "smd"),
        ];
        let rep = coverage(&rows, &[]);
        let order: Vec<String> = rep
            .open_pins
            .iter()
            .map(|p| format!("{}.{}", p.reference, p.pin))
            .collect();
        assert_eq!(order, vec!["U2.1", "U2.9", "U2.10", "U10.1"]);
    }
}
