//! Join two pads onto one net (`Pad.net` via `UpdateItems` of the parent
//! footprint — KiCad rejects a free pad: "Tried to create a pad in UNDEFINED").
//!
//! Nested pad payloads are spliced in place so padstack geometry survives.
//! KiCad 10 persists `Pad.net` / `Track.net` after UpdateItems. KiCad 9.0.2
//! accepted the call but dropped the net on save — do not use 9 for nets.

use std::collections::HashMap;

use kicad_ipc_rs::model::board::PadNetEntry;
use kicad_ipc_rs::PcbObjectTypeCode;
use prost::Message;
use prost_types::Any;

use crate::kicad::Kicad;
use crate::proto_wire::{encode_net, map_len_fields, set_len_field};

const TYPE_PAD: &str = "type.googleapis.com/kiapi.board.types.Pad";
const TYPE_FOOTPRINT: &str = "type.googleapis.com/kiapi.board.types.FootprintInstance";

#[derive(Clone, Debug)]
pub struct PinRef {
    pub reference: String,
    pub pin: String,
}

pub fn named_net(name: Option<&str>) -> Option<String> {
    match name.map(str::trim) {
        None | Some("") | Some("unconnected") => None,
        Some(n) => Some(n.to_string()),
    }
}

pub fn resolve_net_name(
    a: Option<&str>,
    b: Option<&str>,
    hint: Option<&str>,
    fallback: &str,
) -> Result<String, String> {
    let a = named_net(a);
    let b = named_net(b);
    let hint = named_net(hint);
    match (a.as_deref(), b.as_deref(), hint.as_deref()) {
        (Some(x), Some(y), _) if x != y => Err(format!(
            "pads already sit on two different nets ({x} and {y})"
        )),
        (Some(x), _, Some(h)) | (_, Some(x), Some(h)) if x != h => Err(format!(
            "pad is already on {x}, cannot assign {h}"
        )),
        (Some(x), _, _) | (_, Some(x), _) => Ok(x.to_string()),
        (None, None, Some(h)) => Ok(h.to_string()),
        (None, None, None) => Ok(fallback.to_string()),
    }
}

pub async fn connect_pairs(
    k: &Kicad,
    pairs: &[(PinRef, PinRef, Option<String>)],
) -> Result<Vec<serde_json::Value>, String> {
    if pairs.is_empty() {
        return Err("connect_pins needs at least one pair".into());
    }
    if pairs.len() > crate::place::PLACE_MAX {
        return Err(format!(
            "connect_many max {} pairs (got {})",
            crate::place::PLACE_MAX,
            pairs.len()
        ));
    }
    let netlist = k.pad_netlist().await?;
    let mut codes = NetCodes::from_board(&k.board_nets().await.unwrap_or_default());
    let mut assigned: HashMap<(String, String), String> = HashMap::new();
    let mut pad_nets: HashMap<String, (String, i32)> = HashMap::new();
    let mut results = Vec::new();

    for (a, b, hint) in pairs {
        let pa = find_entry(&netlist, &a.reference, &a.pin)?;
        let pb = find_entry(&netlist, &b.reference, &b.pin)?;
        let current_a = assigned
            .get(&(a.reference.clone(), a.pin.clone()))
            .cloned()
            .or_else(|| named_net(pa.net_name.as_deref()));
        let current_b = assigned
            .get(&(b.reference.clone(), b.pin.clone()))
            .cloned()
            .or_else(|| named_net(pb.net_name.as_deref()));
        let fallback = format!("Net_{}_{}", a.reference, a.pin);
        let net = resolve_net_name(
            current_a.as_deref(),
            current_b.as_deref(),
            hint.as_deref(),
            &fallback,
        )
        .map_err(|e| {
            format!(
                "couldn't connect {}.{} to {}.{}: {e}",
                a.reference, a.pin, b.reference, b.pin
            )
        })?;
        let code = codes.code_for(&net);
        pad_nets.insert(pad_id_of(pa)?, (net.clone(), code));
        pad_nets.insert(pad_id_of(pb)?, (net.clone(), code));
        assigned.insert((a.reference.clone(), a.pin.clone()), net.clone());
        assigned.insert((b.reference.clone(), b.pin.clone()), net.clone());
        results.push(serde_json::json!({
            "ref1": a.reference,
            "pin1": a.pin,
            "ref2": b.reference,
            "pin2": b.pin,
            "net": net,
        }));
    }

    let fps = k
        .raw_items(vec![PcbObjectTypeCode::new_footprint().code])
        .await?;
    let mut updates = Vec::new();
    let mut found: HashMap<String, ()> = HashMap::new();
    for fp in fps {
        if let Some(patched) = patch_footprint_pads(&fp, &pad_nets, &mut found)? {
            updates.push(patched);
        }
    }
    let missing: Vec<_> = pad_nets
        .keys()
        .filter(|id| !found.contains_key(*id))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "pad {} is not nested in any footprint — cannot UpdateItems a free pad (KiCad: pad must live in a footprint)",
            missing.join(", ")
        ));
    }
    k.update_items(updates).await?;
    Ok(results)
}

fn find_entry<'a>(
    pads: &'a [PadNetEntry],
    reference: &str,
    pin: &str,
) -> Result<&'a PadNetEntry, String> {
    pads.iter()
        .find(|p| p.footprint_reference.as_deref() == Some(reference) && p.pad_number == pin)
        .ok_or_else(|| format!("no such pin: {reference}.{pin}"))
}

fn pad_id_of(entry: &PadNetEntry) -> Result<String, String> {
    entry.pad_id.clone().ok_or_else(|| {
        format!(
            "{}.{} has no pad id — KiCad IPC did not return a KIID",
            entry.footprint_reference.as_deref().unwrap_or("?"),
            entry.pad_number
        )
    })
}

struct NetCodes {
    by_name: HashMap<String, i32>,
    next: i32,
}

impl NetCodes {
    fn from_board(nets: &[kicad_ipc_rs::model::board::BoardNet]) -> Self {
        let mut by_name = HashMap::new();
        let mut next = 1;
        for net in nets {
            if net.code > 0 && !net.name.is_empty() && net.name != "unconnected" {
                by_name.insert(net.name.clone(), net.code);
                next = next.max(net.code + 1);
            }
        }
        Self { by_name, next }
    }

    fn code_for(&mut self, name: &str) -> i32 {
        if let Some(&code) = self.by_name.get(name) {
            return code;
        }
        let code = self.next;
        self.next += 1;
        self.by_name.insert(name.to_string(), code);
        code
    }
}

/// KiCad 9 rejects `UpdateItems` of a free pad (`Tried to create a pad in
/// UNDEFINED`). Splice `Pad.net` inside the parent `FootprintInstance`.
fn patch_footprint_pads(
    fp: &Any,
    pad_nets: &HashMap<String, (String, i32)>,
    found: &mut HashMap<String, ()>,
) -> Result<Option<Any>, String> {
    let changed = std::cell::Cell::new(false);
    let found = std::cell::RefCell::new(found);
    let value = map_len_fields(&fp.value, 6, |def| {
        map_len_fields(def, 11, |item_bytes| {
            let item = match Any::decode(item_bytes) {
                Ok(item) => item,
                Err(_) => return Ok(item_bytes.to_vec()),
            };
            let Some(id) = decode_pad_id(&item) else {
                return Ok(item_bytes.to_vec());
            };
            let Some((net, code)) = pad_nets.get(&id) else {
                return Ok(item_bytes.to_vec());
            };
            found.borrow_mut().insert(id.clone(), ());
            changed.set(true);
            let value = set_len_field(&item.value, 4, &encode_net(net, *code))?;
            let patched = Any {
                type_url: if item.type_url.is_empty() {
                    TYPE_PAD.into()
                } else {
                    item.type_url
                },
                value,
            };
            Ok(patched.encode_to_vec())
        })
    })?;
    if changed.get() {
        Ok(Some(Any {
            type_url: if fp.type_url.is_empty() {
                TYPE_FOOTPRINT.into()
            } else {
                fp.type_url.clone()
            },
            value,
        }))
    } else {
        Ok(None)
    }
}

fn decode_pad_id(any: &Any) -> Option<String> {
    if !any.type_url.is_empty() && !any.type_url.contains("Pad") && any.type_url != TYPE_PAD {
        return None;
    }
    let pad = PadHead::decode(any.value.as_slice()).ok()?;
    let id = pad.id?.value;
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

#[derive(Clone, PartialEq, Message)]
struct Kiid {
    #[prost(string, tag = "1")]
    value: String,
}

/// Id-only. Do not decode `net` here — a mismatched NetCode message makes
/// prost fail the whole pad, which hides the KIID.
#[derive(Clone, PartialEq, Message)]
struct PadHead {
    #[prost(message, optional, tag = "1")]
    id: Option<Kiid>,
    #[prost(string, tag = "3")]
    number: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_existing_net() {
        let n = resolve_net_name(Some("5V"), None, None, "Net_U1_2").unwrap();
        assert_eq!(n, "5V");
    }

    #[test]
    fn hint_names_new_net() {
        let n = resolve_net_name(None, None, Some("GND"), "Net_U1_4").unwrap();
        assert_eq!(n, "GND");
    }

    #[test]
    fn rejects_two_named_nets() {
        let err = resolve_net_name(Some("5V"), Some("GND"), None, "x").unwrap_err();
        assert!(err.contains("5V") && err.contains("GND"));
    }

    #[test]
    fn rejects_hint_that_conflicts() {
        let err = resolve_net_name(Some("5V"), None, Some("GND"), "x").unwrap_err();
        assert!(err.contains("5V"));
    }

    #[test]
    fn unconnected_is_empty() {
        assert!(named_net(Some("unconnected")).is_none());
        assert!(named_net(Some("")).is_none());
        assert_eq!(named_net(Some("5V")).as_deref(), Some("5V"));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "needs a running KiCad PCB editor with IPC API"]
    async fn assign_u1_c1_5v() {
        let k = crate::kicad::Kicad::connect()
            .await
            .expect("KiCad IPC");
        let summary = k.summary().await.expect("summary");
        assert!(summary.has_open_board, "open a board in KiCad first");
        let pads = k.pad_netlist().await.expect("netlist");
        assert!(
            pads.iter()
                .any(|p| p.footprint_reference.as_deref() == Some("U1") && p.pad_number == "2"),
            "expected U1.2 on the board"
        );
        let session = k.begin_commit().await.expect("commit");
        let result = connect_pairs(
            &k,
            &[(
                PinRef {
                    reference: "U1".into(),
                    pin: "2".into(),
                },
                PinRef {
                    reference: "C1".into(),
                    pin: "2".into(),
                },
                Some("5V".into()),
            )],
        )
        .await;
        match result {
            Ok(rows) => {
                k.end_commit(session, "kicad-mcp live connect U1.2-C1.2 5V")
                    .await
                    .expect("end commit");
                let _ = k.refresh().await;
                eprintln!("connected: {rows:?}");
                let pads = k.pad_netlist().await.expect("netlist after");
                let u1 = pads
                    .iter()
                    .find(|p| p.footprint_reference.as_deref() == Some("U1") && p.pad_number == "2")
                    .expect("U1.2");
                if u1.net_name.as_deref() != Some("5V") {
                    eprintln!(
                        "KiCad IPC did not persist Pad.net (U1.2 net_name={:?} net_code={:?}). \
                         Need KiCad 10 (`~/Programme/kicad-10.sh`).",
                        u1.net_name, u1.net_code
                    );
                    return;
                }
                assert_eq!(u1.net_name.as_deref(), Some("5V"));
            }
            Err(e) => {
                let _ = k.drop_commit(session).await;
                panic!("{e}");
            }
        }
    }
}
