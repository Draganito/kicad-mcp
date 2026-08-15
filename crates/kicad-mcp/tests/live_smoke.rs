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

    // Left of the 4×5 grid, inside the 80×50 mm board (origin ~108.5, 80).
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
            report.push(format!("place WirePad_PTH as W99 at ({x}, {y}): {n} item(s)"));
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
                    "NOTE: KiCad 9.0.2 IPC does not persist Pad.net — assign nets in the GUI or use KiCad 10."
                        .into(),
                );
            }
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            report.push(format!("connect_pins failed: {e}"));
        }
    }

    // Cleanup so the playground stays a 4×5 LED grid (Ctrl+Z also works).
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
                k.end_commit(session, "kicad-mcp smoke cleanup")
                    .await
                    .ok();
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
