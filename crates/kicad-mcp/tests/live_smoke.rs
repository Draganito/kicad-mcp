//! Live smoke against a running KiCad PCB editor. `cargo test -p kicad-mcp -- --ignored --nocapture`

use kicad_mcp::builtins::{self, WIRE_PAD};
use kicad_mcp::copper::{self, parse_copper_layer};
use kicad_mcp::kicad::{self, Kicad};
use kicad_mcp::nets::{self, PinRef};
use kicad_mcp::place::{self, PlaceSpec};

#[tokio::test]
#[ignore = "needs a running KiCad PCB editor with IPC API"]
async fn smoke_new_tools() {
    let k = Kicad::connect().await.expect("KiCad IPC");
    let summary = k.summary().await.expect("summary");
    assert!(summary.has_open_board, "open a board in KiCad first");
    eprintln!(
        "board: {} footprints, {} tracks, {} vias, {} nets (KiCad {})",
        summary.footprint_count,
        summary.track_count,
        summary.via_count,
        summary.net_count,
        summary.kicad_version
    );

    let dir = k.project_dir().await.expect("project dir");
    let pretty = kicad::jlc_pretty_dir(&dir);
    let written = builtins::ensure_builtin_footprints(&pretty).expect("builtins");
    eprintln!("builtins written: {written:?}");
    let loaded = place::load_template(&pretty, WIRE_PAD).expect("WirePad_PTH template");
    assert_eq!(loaded.pads.len(), 1);

    // A free spot on the open board (adjust if this board is smaller).
    let x = 112.0;
    let y = 105.0;
    let item = place::footprint_instance_any(&PlaceSpec {
        template: WIRE_PAD,
        reference: "W99",
        x_mm: x,
        y_mm: y,
        rotation_deg: 0.0,
        pads: &loaded.pads,
    })
    .expect("encode WirePad");

    let session = k.begin_commit().await.expect("commit");
    let mut report: Vec<String> = Vec::new();

    match k.create_items(vec![item]).await {
        Ok(n) => {
            k.end_commit(session, "kicad-mcp smoke place W99")
                .await
                .expect("end place");
            let _ = k.refresh().await;
            report.push(format!(
                "place WirePad_PTH as W99 at ({x}, {y}): {n} item(s)"
            ));
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            panic!("place W99 failed: {e}");
        }
    }

    let fps = k.footprints().await.expect("footprints");
    assert!(
        fps.iter().any(|f| f.reference.as_deref() == Some("W99")),
        "W99 not on board after place"
    );

    let layer = parse_copper_layer(Some("F.Cu")).unwrap();
    let track = copper::track_any(114.0, 105.0, 118.0, 105.0, Some(0.25), layer, "5V")
        .expect("encode track");
    let via = copper::via_any(116.0, 107.0, "5V", None, None).expect("encode via");
    let session = k.begin_commit().await.expect("commit copper");
    match k.create_items(vec![track, via]).await {
        Ok(n) => {
            k.end_commit(session, "kicad-mcp smoke track+via")
                .await
                .expect("end copper");
            let _ = k.refresh().await;
            report.push(format!("add_track + add_via: {n} item(s)"));
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            panic!("copper create failed: {e}");
        }
    }

    let tracks = k.tracks().await.expect("tracks");
    let vias = k.vias().await.expect("vias");
    report.push(format!(
        "routing scene: {} track(s), {} via(s)",
        tracks.len(),
        vias.len()
    ));
    assert!(!tracks.is_empty(), "expected at least one track");
    assert!(!vias.is_empty(), "expected at least one via");

    let session = k.begin_commit().await.expect("commit nets");
    let connected = nets::connect_pairs(
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
    match connected {
        Ok(rows) => {
            k.end_commit(session, "kicad-mcp smoke connect")
                .await
                .expect("end connect");
            let _ = k.refresh().await;
            let pads = k.pad_netlist().await.expect("netlist");
            let u1 = pads
                .iter()
                .find(|p| p.footprint_reference.as_deref() == Some("U1") && p.pad_number == "2");
            let persisted = u1.and_then(|p| p.net_name.as_deref()) == Some("5V");
            report.push(format!(
                "connect_pins U1.2–C1.2 5V: ipc_ok rows={rows:?} persisted={persisted}"
            ));
            if !persisted {
                report.push(
                    "NOTE: Pad.net did not persist — start KiCad 10 via ~/Programme/kicad-10.sh."
                        .into(),
                );
            }
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            report.push(format!("connect_pins failed: {e}"));
        }
    }

    let session = k.begin_commit().await.expect("commit smoke W99 net");
    let w99_net = nets::connect_pairs(
        &k,
        &[(
            PinRef {
                reference: "W99".into(),
                pin: "1".into(),
            },
            PinRef {
                reference: "W99".into(),
                pin: "1".into(),
            },
            Some("SMOKE_NET".into()),
        )],
    )
    .await;
    match w99_net {
        Ok(_) => {
            k.end_commit(session, "kicad-mcp smoke connect W99")
                .await
                .expect("end W99 connect");
            let _ = k.refresh().await;
            let session = k.begin_commit().await.expect("commit smoke disconnect");
            match nets::disconnect_pins(
                &k,
                &[PinRef {
                    reference: "W99".into(),
                    pin: "1".into(),
                }],
            )
            .await
            {
                Ok(rows) => {
                    k.end_commit(session, "kicad-mcp smoke disconnect W99")
                        .await
                        .expect("end disconnect");
                    let _ = k.refresh().await;
                    let pads = k.pad_netlist().await.expect("netlist after disconnect");
                    let w99 = pads.iter().find(|p| {
                        p.footprint_reference.as_deref() == Some("W99") && p.pad_number == "1"
                    });
                    let cleared = match w99.and_then(|p| p.net_name.as_deref()) {
                        None | Some("") | Some("unconnected") => true,
                        Some(_) => false,
                    };
                    report.push(format!(
                        "disconnect_pin W99.1: ipc_ok rows={rows:?} cleared={cleared}"
                    ));
                    if !cleared {
                        report.push(
                            "NOTE: disconnect_pin did not persist — start KiCad 10 via ~/Programme/kicad-10.sh."
                                .into(),
                        );
                    }
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    report.push(format!("disconnect_pin failed: {e}"));
                }
            }
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            report.push(format!("connect W99 for disconnect smoke failed: {e}"));
        }
    }

    // Remove the smoke parts (Ctrl+Z also works).
    let mut drop_ids: Vec<String> = Vec::new();
    if let Some(id) = k.footprint_id_by_reference("W99").await.ok().flatten() {
        drop_ids.push(id);
    }
    drop_ids.extend(tracks.into_iter().filter_map(|t| t.id));
    drop_ids.extend(vias.into_iter().filter_map(|v| v.id));
    if !drop_ids.is_empty() {
        let session = k.begin_commit().await.expect("commit cleanup");
        match k.delete_ids(drop_ids).await {
            Ok(deleted) => {
                k.end_commit(session, "kicad-mcp smoke cleanup").await.ok();
                let _ = k.refresh().await;
                report.push(format!("cleanup deleted {} item(s)", deleted.len()));
            }
            Err(e) => {
                let _ = k.drop_commit(session).await;
                report.push(format!("cleanup failed: {e}"));
            }
        }
    }

    eprintln!("--- smoke report ---");
    for line in &report {
        eprintln!("  {line}");
    }
}

#[tokio::test]
#[ignore = "needs a running KiCad PCB editor with IPC API"]
async fn export_manufacturing_to_project() {
    let k = Kicad::connect().await.expect("KiCad IPC");
    let _ = k.refill_all_zones().await;
    k.save().await.expect("save");
    let board = k.board_file_path().await.expect("board path");
    let dir = k.project_dir().await.expect("project dir");
    let fps = k.footprints().await.expect("footprints");
    let files = kicad_mcp::fab::export_manufacturing(&board, &dir, &fps).expect("export");
    eprintln!("gerber_zip={}", files.gerber_zip.display());
    eprintln!(
        "bom_csv={} rows={}",
        files.bom_csv.display(),
        files.bom_rows
    );
    eprintln!(
        "cpl_csv={} rows={}",
        files.cpl_csv.display(),
        files.cpl_rows
    );
    assert!(files.gerber_zip.is_file());
    assert!(files.bom_rows >= 1);
    assert!(files.cpl_rows >= 1);
}

#[tokio::test]
#[ignore = "needs a running KiCad PCB editor with IPC API"]
async fn assign_vias_gnd_and_refill() {
    use kicad_ipc_rs::PcbObjectTypeCode;
    use kicad_mcp::proto_wire::{encode_net, set_len_field};
    use prost_types::Any;

    let k = Kicad::connect().await.expect("KiCad IPC");
    let raw = k
        .raw_items(vec![PcbObjectTypeCode::new_via().code])
        .await
        .expect("raw vias");
    assert!(!raw.is_empty(), "no vias on the board");
    let payload = encode_net("GND", 1);
    let patched: Vec<Any> = raw
        .into_iter()
        .map(|item| {
            let value = set_len_field(&item.value, 5, &payload).expect("splice Via.net");
            Any {
                type_url: item.type_url,
                value,
            }
        })
        .collect();
    let n = patched.len();
    let session = k.begin_commit().await.expect("commit");
    match k.update_items(patched).await {
        Ok(updated) => {
            k.end_commit(session, "kicad-mcp vias → GND")
                .await
                .expect("end");
            eprintln!("updated {updated} vias (sent {n})");
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            panic!("UpdateItems vias: {e}");
        }
    }
    k.refill_all_zones().await.expect("refill (B)");
    let _ = k.refresh().await;
    let vias = k.vias().await.expect("vias after");
    let gnd = vias
        .iter()
        .filter(|v| v.net.as_deref() == Some("GND"))
        .count();
    let nets = k.nets().await.expect("nets");
    eprintln!(
        "after: via_net_gnd={gnd}/{} nets={:?}",
        vias.len(),
        nets.iter().map(|n| &n.name).collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "needs KiCad 10, Routing Tools setup, and an open board"]
async fn live_autoroute_named_net_reloads() {
    let k = Kicad::connect().await.expect("KiCad IPC");
    let before = k.summary().await.expect("summary");
    assert!(before.has_open_board, "open a board in KiCad first");
    let result = kicad_mcp::autoroute::autoroute_nets(
        &k,
        &["5V".into()],
        &kicad_mcp::autoroute::AutorouteOpts::default(),
    )
    .await
    .expect("autoroute_nets");
    eprintln!("{}", serde_json::to_string_pretty(&result).unwrap());
    assert!(
        result.reloaded,
        "KiCad must show the CLI copper after reload"
    );
    assert!(
        result.track_count > before.track_count || result.via_count > before.via_count,
        "expected new 5V copper after reload"
    );
}
