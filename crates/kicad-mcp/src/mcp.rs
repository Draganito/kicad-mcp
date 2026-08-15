use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo};
use rmcp::schemars;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::kicad::Kicad;

#[derive(Clone)]
pub struct KicadMcp {
    allow_ai_write: bool,
    kicad: Arc<Mutex<Option<Arc<Kicad>>>>,
    #[allow(dead_code)]
    tool_router: rmcp::handler::server::tool::ToolRouter<Self>,
}

#[tool_router]
impl KicadMcp {
    pub fn new(allow_ai_write: bool) -> Self {
        Self {
            allow_ai_write,
            kicad: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    fn require_write(&self) -> Option<Result<CallToolResult, McpError>> {
        if self.allow_ai_write {
            None
        } else {
            Some(json_err(
                "write tools are disabled — relaunch kicad-mcp with --allow-ai-write",
            ))
        }
    }

    async fn client(&self) -> Result<Arc<Kicad>, String> {
        let mut guard = self.kicad.lock().await;
        if guard.is_none() {
            *guard = Some(Arc::new(Kicad::connect().await?));
        }
        Ok(guard.as_ref().expect("just inserted").clone())
    }

    #[tool(
        description = "Live KiCad board overview: version, net_ipc_persists, project path, copper layers, footprint/net/track/via/zone counts. Start here. Target is KiCad 10 (net_ipc_persists true). If the version is 9.x, tell the user to start ~/Programme/kicad-10.sh — do not assign nets in the GUI. Needs IPC API enabled."
    )]
    async fn board_summary(&self) -> Result<CallToolResult, McpError> {
        match self.client().await {
            Ok(k) => match k.summary().await {
                Ok(s) => json_ok(&s),
                Err(e) => json_err(&e),
            },
            Err(e) => json_err(&e),
        }
    }

    #[tool(
        description = "Every footprint on the open board: reference, value, x_mm/y_mm (KiCad native origin, +x right, +y up), rotation_deg, layer, pad_count."
    )]
    async fn get_footprints(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move { k.footprints().await }).await
    }

    #[tool(description = "Nets on the open board with the pads on each net as REF.PIN.")]
    async fn get_nets(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move { k.nets().await }).await
    }

    #[tool(
        description = "Tracks and vias currently on the board (id, net, layer, endpoints in mm). Use track/via id with ripup_wire."
    )]
    async fn get_routing_scene(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move {
            let tracks = k.tracks().await?;
            let vias = k.vias().await?;
            Ok(serde_json::json!({ "tracks": tracks, "vias": vias }))
        })
        .await
    }

    #[tool(
        description = "Footprint templates in this project's jlcpcb_parts.pretty — exact names place_footprint wants, plus F.CrtYd size. Also writes builtin WirePad_PTH and MountingHole_M3_NPTH if missing (not LCSC)."
    )]
    async fn list_parts(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move {
            let dir = k.project_dir().await?;
            let pretty = crate::kicad::jlc_pretty_dir(&dir);
            easyeda_kicad::ensure_fp_lib_table(&dir.join("fp-lib-table")).map_err(|e| e.to_string())?;
            let _ = crate::builtins::ensure_builtin_footprints(&pretty)?;
            let names = easyeda_kicad::list_pretty_footprints(&pretty).map_err(|e| e.to_string())?;
            let mut templates = Vec::new();
            for name in names {
                let loaded = crate::place::load_template(&pretty, &name).ok();
                let (cw, ch) = loaded
                    .as_ref()
                    .map(|t| {
                        (
                            t.courtyard.max_x - t.courtyard.min_x,
                            t.courtyard.max_y - t.courtyard.min_y,
                        )
                    })
                    .unwrap_or((0.0, 0.0));
                templates.push(serde_json::json!({
                    "template": name,
                    "courtyard_w_mm": (cw * 1000.0).round() / 1000.0,
                    "courtyard_h_mm": (ch * 1000.0).round() / 1000.0,
                    "pad_count": loaded.map(|t| t.pads.len()).unwrap_or(0),
                }));
            }
            Ok(serde_json::json!({ "library": "jlcpcb_parts", "templates": templates }))
        })
        .await
    }

    #[tool(
        description = "Connectivity snapshot: footprints, nets, and pads whose net_name is empty or 'unconnected'. Not a full KiCad DRC run yet."
    )]
    async fn check_board(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move {
            let footprints = k.footprints().await?;
            let nets = k.nets().await?;
            let pads = k.pad_netlist().await?;
            let unconnected: Vec<String> = pads
                .iter()
                .filter(|p| match p.net_name.as_deref() {
                    None | Some("") | Some("unconnected") => true,
                    _ => false,
                })
                .map(|p| {
                    let r = p.footprint_reference.as_deref().unwrap_or("?");
                    format!("{r}.{}", p.pad_number)
                })
                .collect();
            Ok(serde_json::json!({
                "ok": unconnected.is_empty(),
                "footprint_count": footprints.len(),
                "net_count": nets.len(),
                "unconnected_pads": unconnected,
                "unconnected_pad_count": unconnected.len(),
            }))
        })
        .await
    }

    #[tool(
        description = "Download LCSC C-number from EasyEDA and write a native KiCad footprint + symbol into the open project's jlcpcb_parts library. Returns the template name place_footprint wants."
    )]
    async fn download_lcsc_part(
        &self,
        Parameters(args): Parameters<LcscArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let dir = k.project_dir().await?;
            let part = tokio::task::spawn_blocking(move || easyeda_kicad::fetch_by_lcsc_code(&args.lcsc_code))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            easyeda_kicad::ensure_fp_lib_table(&dir.join("fp-lib-table")).map_err(|e| e.to_string())?;
            easyeda_kicad::ensure_sym_lib_table(&dir.join("sym-lib-table")).map_err(|e| e.to_string())?;
            let name = easyeda_kicad::write_library_files(&part, &crate::kicad::jlc_pretty_dir(&dir), &crate::kicad::jlc_sym_path(&dir))
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "ok": true,
                "lcsc_code": part.lcsc_code,
                "name": part.name,
                "template": name,
                "reference_prefix": part.reference_prefix,
                "pad_count": part.pads.len(),
                "library": "jlcpcb_parts",
                "note": "KiCad may need a library refresh (close/reopen the footprint chooser) before the new part appears in the GUI picker. place_footprint pastes it directly.",
            }))
        })
        .await
    }

    #[tool(
        description = "Place a previously downloaded LCSC footprint on the open board. template is the name list_parts reports. x_mm/y_mm are KiCad native millimetres (+x right, +y up). Optional reference (e.g. R12); otherwise the next free prefix number is used. Refuses courtyard overlap with an already placed part (F.CrtYd from the .kicad_mod). Lands on KiCad's undo stack."
    )]
    async fn place_footprint(
        &self,
        Parameters(args): Parameters<PlaceArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            place_many(
                &k,
                vec![PlacePartSpec {
                    template: args.template,
                    x_mm: args.x_mm,
                    y_mm: args.y_mm,
                    rotation_deg: args.rotation_deg,
                    reference: args.reference,
                }],
            )
            .await
        })
        .await
    }

    #[tool(
        description = "Place many LCSC footprints in one undo (max 150). Each entry: {template, x_mm, y_mm, rotation_deg?, reference?}. All-or-nothing courtyard check against the board and each other. Prefer this (or place_matrix) over N× place_footprint for an LED panel."
    )]
    async fn place_parts(
        &self,
        Parameters(args): Parameters<PlacePartsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move { place_many(&k, args.parts).await }).await
    }

    #[tool(
        description = "Place a rows×cols grid of one LCSC template in one undo (max 150 cells). origin_x_mm/origin_y_mm is cell (0,0); columns go +x, rows go +y. Pitch is centre-to-centre millimetres (Darkroom LEDs: 12.7). Refuses courtyard overlap. Same pad-bake as place_footprint."
    )]
    async fn place_matrix(
        &self,
        Parameters(args): Parameters<PlaceMatrixArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let pts = crate::place::matrix_positions(
                args.rows,
                args.cols,
                args.pitch_x_mm,
                args.pitch_y_mm,
                args.origin_x_mm,
                args.origin_y_mm,
            )?;
            let rot = args.rotation_deg;
            let parts = pts
                .into_iter()
                .map(|(x_mm, y_mm)| PlacePartSpec {
                    template: args.template.clone(),
                    x_mm,
                    y_mm,
                    rotation_deg: rot,
                    reference: None,
                })
                .collect();
            place_many(&k, parts).await
        })
        .await
    }

    #[tool(description = "Delete one footprint by reference. Ctrl+Z undoes.")]
    async fn remove_footprint(
        &self,
        Parameters(args): Parameters<RefArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let Some(id) = k.footprint_id_by_reference(&args.reference).await? else {
                return Err(format!("{} is not on the board", args.reference));
            };
            let session = k.begin_commit().await?;
            match k.delete_ids(vec![id]).await {
                Ok(deleted) => {
                    k.end_commit(session, &format!("kicad-mcp remove {}", args.reference)).await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({ "ok": true, "reference": args.reference, "deleted_ids": deleted }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Rip copper. Pass segment_id (tracks[].id or vias[].id from get_routing_scene) to delete that one item. Ctrl+Z undoes."
    )]
    async fn ripup_wire(
        &self,
        Parameters(args): Parameters<RipupArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let Some(id) = args.segment_id.clone() else {
                return Err("ripup_wire currently needs segment_id from get_routing_scene (one track or via)".into());
            };
            let session = k.begin_commit().await?;
            match k.delete_ids(vec![id.clone()]).await {
                Ok(deleted) => {
                    k.end_commit(session, "kicad-mcp ripup").await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({ "ok": true, "deleted_ids": deleted }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Delete every footprint, track, via and zone on the open board (Edge.Cuts stays unless you set_board_outline with replace). One undo. Use this to start a board from scratch."
    )]
    async fn clear_board(&self) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, |k| async move {
            let mut ids = Vec::new();
            for fp in k.footprints().await? {
                if let Some(id) = fp.id {
                    ids.push(id);
                }
            }
            for t in k.tracks().await? {
                if let Some(id) = t.id {
                    ids.push(id);
                }
            }
            for v in k.vias().await? {
                if let Some(id) = v.id {
                    ids.push(id);
                }
            }
            ids.extend(k.zone_ids().await?);
            if ids.is_empty() {
                return Ok(serde_json::json!({ "ok": true, "deleted": 0 }));
            }
            let session = k.begin_commit().await?;
            let mut deleted = 0usize;
            for chunk in ids.chunks(200) {
                match k.delete_ids(chunk.to_vec()).await {
                    Ok(d) => deleted += d.len(),
                    Err(e) => {
                        let _ = k.drop_commit(session).await;
                        return Err(e);
                    }
                }
            }
            k.end_commit(session, "kicad-mcp clear board").await?;
            let _ = k.refresh().await;
            Ok(serde_json::json!({ "ok": true, "deleted": deleted }))
        })
        .await
    }

    #[tool(
        description = "Draw the board outline on Edge.Cuts (that is KiCad's board size). Rectangle: width_mm/height_mm, origin = bottom-left (+y up); omit origin to centre on the A4 sheet. Polygon: points [{x_mm, y_mm}, ...] already in KiCad millimetres (closed automatically). replace defaults to true."
    )]
    async fn set_board_outline(
        &self,
        Parameters(args): Parameters<OutlineArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let points: Vec<(f64, f64)> = args
                .points
                .as_ref()
                .map(|pts| pts.iter().map(|p| (p.x_mm, p.y_mm)).collect())
                .unwrap_or_default();
            let items = if points.len() >= 3 {
                crate::outline::poly_edge_cuts(&points)?
            } else {
                let w = args.width_mm.ok_or_else(|| {
                    "set_board_outline needs width_mm/height_mm or points".to_string()
                })?;
                let h = args.height_mm.ok_or_else(|| {
                    "set_board_outline needs width_mm/height_mm or points".to_string()
                })?;
                let (ox, oy) = match (args.origin_x_mm, args.origin_y_mm) {
                    (Some(x), Some(y)) => (x, y),
                    _ => outline_origin_for_sheet(w, h),
                };
                crate::outline::rect_edge_cuts(ox, oy, w, h)?
            };
            let n_seg = items.len();
            let replace = args.replace.unwrap_or(true);
            let session = k.begin_commit().await?;
            let replaced = if replace {
                let ids = k.edge_cuts_ids().await?;
                let n = ids.len();
                if !ids.is_empty() {
                    if let Err(e) = k.delete_ids(ids).await {
                        let _ = k.drop_commit(session).await;
                        return Err(e);
                    }
                }
                n
            } else {
                0
            };
            match k.create_items(items).await {
                Ok(n) => {
                    if let Err(e) = k.refill_all_zones().await {
                        let _ = k.drop_commit(session).await;
                        return Err(e);
                    }
                    k.end_commit(session, "kicad-mcp Edge.Cuts outline").await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "origin_x_mm": args.origin_x_mm,
                        "origin_y_mm": args.origin_y_mm,
                        "width_mm": args.width_mm,
                        "height_mm": args.height_mm,
                        "point_count": points.len(),
                        "layer": "Edge.Cuts",
                        "segments": n_seg,
                        "items_created": n,
                        "replaced_segments": replaced,
                    }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Join two pads onto one net (Pad.net via UpdateItems of the parent footprint). ref1/pin1 and ref2/pin2 are like U1 and 2. Optional net names a new one (e.g. \"5V\", \"GND\"). Daisy-chain hops omit net. Persists on KiCad 10. Not copper — use add_track / set_copper_zone after."
    )]
    async fn connect_pins(
        &self,
        Parameters(args): Parameters<ConnectPinsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let pairs = [(
                crate::nets::PinRef {
                    reference: args.ref1,
                    pin: args.pin1,
                },
                crate::nets::PinRef {
                    reference: args.ref2,
                    pin: args.pin2,
                },
                args.net,
            )];
            commit_connect(&k, &pairs).await
        })
        .await
    }

    #[tool(
        description = "Join many pad pairs onto nets in one undo (max 150). Each pair: {ref1, pin1, ref2, pin2, net?}. Same rules as connect_pins. Use this for the SK6812 5V/GND star and DOUT→DIN daisy chain."
    )]
    async fn connect_many(
        &self,
        Parameters(args): Parameters<ConnectManyArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let pairs: Vec<_> = args
                .pairs
                .into_iter()
                .map(|p| {
                    (
                        crate::nets::PinRef {
                            reference: p.ref1,
                            pin: p.pin1,
                        },
                        crate::nets::PinRef {
                            reference: p.ref2,
                            pin: p.pin2,
                        },
                        p.net,
                    )
                })
                .collect();
            commit_connect(&k, &pairs).await
        })
        .await
    }

    #[tool(
        description = "Create one straight copper track (no autorouter). a_x_mm/a_y_mm to b_x_mm/b_y_mm in KiCad millimetres. net is required (from connect_pins / get_nets). layer is F.Cu or B.Cu (default F.Cu). width_mm defaults to 0.25. Ctrl+Z undoes."
    )]
    async fn add_track(
        &self,
        Parameters(args): Parameters<AddTrackArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let layer = crate::copper::parse_copper_layer(args.layer.as_deref())?;
            let item = crate::copper::track_any(
                args.a_x_mm,
                args.a_y_mm,
                args.b_x_mm,
                args.b_y_mm,
                args.width_mm,
                layer,
                &args.net,
            )?;
            let session = k.begin_commit().await?;
            match k.create_items(vec![item]).await {
                Ok(n) => {
                    k.end_commit(session, &format!("kicad-mcp track {}", args.net))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "net": args.net,
                        "layer": crate::copper::layer_name(layer),
                        "items_created": n,
                    }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Create many straight copper tracks in one undo (max 150, no autorouter). Each item is the same as add_track: {a_x_mm, a_y_mm, b_x_mm, b_y_mm, net, layer?, width_mm?}."
    )]
    async fn add_tracks(
        &self,
        Parameters(args): Parameters<AddTracksArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            if args.tracks.is_empty() {
                return Err("add_tracks needs at least one track".into());
            }
            if args.tracks.len() > crate::copper::COPPER_MAX {
                return Err(format!(
                    "add_tracks max {} (got {})",
                    crate::copper::COPPER_MAX,
                    args.tracks.len()
                ));
            }
            let mut items = Vec::with_capacity(args.tracks.len());
            for t in &args.tracks {
                let layer = crate::copper::parse_copper_layer(t.layer.as_deref())?;
                items.push(crate::copper::track_any(
                    t.a_x_mm,
                    t.a_y_mm,
                    t.b_x_mm,
                    t.b_y_mm,
                    t.width_mm,
                    layer,
                    &t.net,
                )?);
            }
            let n_req = items.len();
            let session = k.begin_commit().await?;
            match k.create_items(items).await {
                Ok(n) => {
                    k.end_commit(session, &format!("kicad-mcp {n_req} tracks"))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "count": n_req,
                        "items_created": n,
                    }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Create one through-via at x_mm/y_mm. net is required. drill_mm defaults to 0.3, size_mm (copper diameter) to 0.6. Ctrl+Z undoes."
    )]
    async fn add_via(
        &self,
        Parameters(args): Parameters<AddViaArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let item = crate::copper::via_any(
                args.x_mm,
                args.y_mm,
                &args.net,
                args.drill_mm,
                args.size_mm,
            )?;
            let session = k.begin_commit().await?;
            match k.create_items(vec![item]).await {
                Ok(n) => {
                    k.end_commit(session, &format!("kicad-mcp via {}", args.net))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "net": args.net,
                        "x_mm": args.x_mm,
                        "y_mm": args.y_mm,
                        "items_created": n,
                    }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Create many through-vias in one undo (max 150). Each item is the same as add_via: {x_mm, y_mm, net, drill_mm?, size_mm?}."
    )]
    async fn add_vias(
        &self,
        Parameters(args): Parameters<AddViasArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            if args.vias.is_empty() {
                return Err("add_vias needs at least one via".into());
            }
            if args.vias.len() > crate::copper::COPPER_MAX {
                return Err(format!(
                    "add_vias max {} (got {})",
                    crate::copper::COPPER_MAX,
                    args.vias.len()
                ));
            }
            let mut items = Vec::with_capacity(args.vias.len());
            for v in &args.vias {
                items.push(crate::copper::via_any(
                    v.x_mm,
                    v.y_mm,
                    &v.net,
                    v.drill_mm,
                    v.size_mm,
                )?);
            }
            let n_req = items.len();
            let session = k.begin_commit().await?;
            match k.create_items(items).await {
                Ok(n) => {
                    k.end_commit(session, &format!("kicad-mcp {n_req} vias"))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "count": n_req,
                        "items_created": n,
                    }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(
        description = "Create a copper zone (pour) and refill. net is required (5V or GND). layer is F.Cu or B.Cu (default F.Cu). Rectangle: origin_x_mm/origin_y_mm/width_mm/height_mm (origin = bottom-left). Polygon: points [{x_mm, y_mm}, ...] in KiCad millimetres. Pads should already sit on that net via connect_pins. Ctrl+Z undoes."
    )]
    async fn set_copper_zone(
        &self,
        Parameters(args): Parameters<SetZoneArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let layer = crate::copper::parse_copper_layer(args.layer.as_deref())?;
            let poly: Vec<(f64, f64)> = args
                .points
                .as_ref()
                .map(|pts| pts.iter().map(|p| (p.x_mm, p.y_mm)).collect())
                .unwrap_or_default();
            let item = if poly.len() >= 3 {
                crate::copper::poly_zone_mm(&poly, layer, &args.net, args.name.as_deref())?
            } else {
                let ox = args.origin_x_mm.ok_or_else(|| {
                    "set_copper_zone needs origin+size or points".to_string()
                })?;
                let oy = args.origin_y_mm.ok_or_else(|| {
                    "set_copper_zone needs origin+size or points".to_string()
                })?;
                let w = args.width_mm.ok_or_else(|| {
                    "set_copper_zone needs origin+size or points".to_string()
                })?;
                let h = args.height_mm.ok_or_else(|| {
                    "set_copper_zone needs origin+size or points".to_string()
                })?;
                crate::copper::rect_zone_any(
                    ox,
                    oy,
                    w,
                    h,
                    layer,
                    &args.net,
                    args.name.as_deref(),
                )?
            };
            let session = k.begin_commit().await?;
            match k.create_items(vec![item]).await {
                Ok(n) => {
                    if let Err(e) = k.refill_all_zones().await {
                        let _ = k.drop_commit(session).await;
                        return Err(e);
                    }
                    k.end_commit(session, &format!("kicad-mcp zone {}", args.net))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "net": args.net,
                        "layer": crate::copper::layer_name(layer),
                        "origin_x_mm": args.origin_x_mm,
                        "origin_y_mm": args.origin_y_mm,
                        "width_mm": args.width_mm,
                        "height_mm": args.height_mm,
                        "items_created": n,
                        "refilled": true,
                    }))
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    Err(e)
                }
            }
        })
        .await
    }

    #[tool(description = "Save the open board to its current path.")]
    async fn save_board(&self) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, |k| async move {
            k.save().await?;
            Ok(serde_json::json!({ "ok": true }))
        })
        .await
    }

    #[tool(
        description = "JLCPCB manufacturing bundle like Alladin: refill zones, save, then write <stem>_gerbers.zip (Gerber + Excellon drill via kicad-cli; silkscreen has no refdes/value text — avoids JLCPCB silk-to-pad DFM), <stem>_cpl.csv (pick & place), <stem>_bom.csv (LCSC). Upload the zip as Gerbers and the two CSVs as BOM/CPL on jlcpcb.com. Optional out_dir (default: project folder). Needs kicad-cli on PATH."
    )]
    async fn export_manufacturing(
        &self,
        Parameters(args): Parameters<ExportManufacturingArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let _ = k.refill_all_zones().await;
            k.save().await?;
            let board = k.board_file_path().await?;
            let footprints = k.footprints().await?;
            let out_dir = match args.out_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(dir) => std::path::PathBuf::from(dir),
                None => k.project_dir().await?,
            };
            let files = tokio::task::spawn_blocking(move || {
                crate::fab::export_manufacturing(&board, &out_dir, &footprints)
            })
            .await
            .map_err(|e| e.to_string())??;
            Ok(serde_json::json!({
                "ok": true,
                "gerber_zip": files.gerber_zip,
                "cpl_csv": files.cpl_csv,
                "bom_csv": files.bom_csv,
                "bom_rows": files.bom_rows,
                "cpl_rows": files.cpl_rows,
                "gerber_files": files.gerber_files,
                "note": "JLCPCB: upload gerber_zip as PCB Gerbers, bom_csv as BOM, cpl_csv as CPL / centroid.",
            }))
        })
        .await
    }
}

async fn with_kicad<F, Fut, T>(mcp: &KicadMcp, f: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce(Arc<Kicad>) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
    T: serde::Serialize,
{
    match mcp.client().await {
        Ok(k) => match f(k).await {
            Ok(v) => json_ok(&v),
            Err(e) => json_err(&e),
        },
        Err(e) => json_err(&e),
    }
}

fn json_ok(value: &impl serde::Serialize) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn json_err(msg: &str) -> Result<CallToolResult, McpError> {
    let text = serde_json::json!({ "ok": false, "error": msg }).to_string();
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

async fn place_many(
    k: &crate::kicad::Kicad,
    specs: Vec<PlacePartSpec>,
) -> Result<serde_json::Value, String> {
    if specs.is_empty() {
        return Err("place_parts needs at least one part".into());
    }
    if specs.len() > crate::place::PLACE_MAX {
        return Err(format!(
            "place_parts max {} (got {})",
            crate::place::PLACE_MAX,
            specs.len()
        ));
    }
    let dir = k.project_dir().await?;
    let pretty = crate::kicad::jlc_pretty_dir(&dir);
    let existing = k.footprints().await?;
    let mut used: Vec<String> = existing.iter().filter_map(|f| f.reference.clone()).collect();
    let mut occupied: Vec<(String, crate::place::Aabb)> = Vec::new();
    for fp in &existing {
        let Some(tmpl) = fp.value.as_deref() else {
            continue;
        };
        let Some(local) = courtyard_of_template(&pretty, tmpl) else {
            continue;
        };
        occupied.push((
            fp.reference.clone().unwrap_or_else(|| tmpl.to_string()),
            crate::place::aabb_at(
                &local,
                fp.x_mm.unwrap_or(0.0),
                fp.y_mm.unwrap_or(0.0),
                fp.rotation_deg.unwrap_or(0.0),
            ),
        ));
    }
    let mut items = Vec::new();
    let mut placed = Vec::new();
    for spec in &specs {
        let loaded = crate::place::load_template(&pretty, &spec.template)?;
        let rot = spec.rotation_deg.unwrap_or(0.0);
        let prefix = infer_prefix(&spec.template, spec.reference.as_deref());
        let reference = spec
            .reference
            .clone()
            .unwrap_or_else(|| next_free_reference(&used, &prefix));
        if used.iter().any(|u| u == &reference) {
            return Err(format!("{reference} is already on the board"));
        }
        let new_box = crate::place::aabb_at(&loaded.courtyard, spec.x_mm, spec.y_mm, rot);
        for (r, other) in &occupied {
            if new_box.overlaps(other, 0.0) {
                return Err(format!(
                    "courtyard of {reference} at ({:.2}, {:.2}) overlaps {r} — pick free space (F.CrtYd)",
                    spec.x_mm, spec.y_mm
                ));
            }
        }
        let item = crate::place::footprint_instance_any(&crate::place::PlaceSpec {
            template: &spec.template,
            reference: &reference,
            x_mm: spec.x_mm,
            y_mm: spec.y_mm,
            rotation_deg: rot,
            pads: &loaded.pads,
        })?;
        used.push(reference.clone());
        occupied.push((reference.clone(), new_box));
        items.push(item);
        placed.push(serde_json::json!({
            "reference": reference,
            "template": spec.template,
            "x_mm": spec.x_mm,
            "y_mm": spec.y_mm,
            "pad_count": loaded.pads.len(),
        }));
    }
    let n_parts = placed.len();
    let session = k.begin_commit().await?;
    match k.create_items(items).await {
        Ok(n) => {
            let msg = if n_parts == 1 {
                format!(
                    "kicad-mcp place {}",
                    placed[0]["reference"].as_str().unwrap_or("?")
                )
            } else {
                format!("kicad-mcp place {n_parts} parts")
            };
            k.end_commit(session, &msg).await?;
            let _ = k.refresh().await;
            let mut out = serde_json::json!({
                "ok": true,
                "placed": placed,
                "items_created": n,
                "count": n_parts,
            });
            if let Some(one) = out["placed"].as_array().and_then(|a| a.first()).cloned() {
                out["reference"] = one["reference"].clone();
                out["template"] = one["template"].clone();
                out["x_mm"] = one["x_mm"].clone();
                out["y_mm"] = one["y_mm"].clone();
                out["pad_count"] = one["pad_count"].clone();
            }
            Ok(out)
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            Err(e)
        }
    }
}

fn infer_prefix(template: &str, explicit: Option<&str>) -> String {
    if let Some(r) = explicit {
        r.chars().take_while(|c| c.is_ascii_alphabetic()).collect()
    } else if template.contains("WirePad") {
        "W".into()
    } else if template.contains("MountingHole") {
        "H".into()
    } else if template.contains("_R") || template.starts_with('R') {
        "R".into()
    } else if template.contains("_C") {
        "C".into()
    } else {
        "U".into()
    }
}

fn next_free_reference(used: &[String], prefix: &str) -> String {
    let mut max = 0u32;
    for r in used {
        if let Some(rest) = r.strip_prefix(prefix) {
            if let Ok(n) = rest.parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    format!("{}{}", prefix, max + 1)
}

/// KiCad's default drawing sheet is A4 landscape (297 × 210 mm).
/// New board outlines sit in the middle of that visible work area.
const SHEET_W_MM: f64 = 297.0;
const SHEET_H_MM: f64 = 210.0;

fn outline_origin_for_sheet(width_mm: f64, height_mm: f64) -> (f64, f64) {
    (
        ((SHEET_W_MM - width_mm) / 2.0).max(0.0),
        ((SHEET_H_MM - height_mm) / 2.0).max(0.0),
    )
}

fn courtyard_of_template(pretty_dir: &std::path::Path, template: &str) -> Option<crate::place::Aabb> {
    let path = pretty_dir.join(format!("{template}.kicad_mod"));
    let text = std::fs::read_to_string(path).ok()?;
    crate::place::parse_kicad_mod_courtyard(&text)
        .or_else(|| crate::place::parse_kicad_mod_pads(&text).ok().map(|p| crate::place::pads_aabb(&p)))
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ExportManufacturingArgs {
    /// Destination folder. Default: the open KiCad project directory.
    pub out_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LcscArgs {
    /// e.g. `"C25804"` or `"C2980298"`.
    pub lcsc_code: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PlaceArgs {
    pub template: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub rotation_deg: Option<f64>,
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PlacePartSpec {
    pub template: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub rotation_deg: Option<f64>,
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PlacePartsArgs {
    pub parts: Vec<PlacePartSpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PlaceMatrixArgs {
    pub template: String,
    pub rows: u32,
    pub cols: u32,
    pub pitch_x_mm: f64,
    pub pitch_y_mm: f64,
    /// Cell (0,0) in KiCad millimetres.
    pub origin_x_mm: f64,
    pub origin_y_mm: f64,
    pub rotation_deg: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RefArgs {
    pub reference: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RipupArgs {
    pub segment_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutlinePoint {
    pub x_mm: f64,
    pub y_mm: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutlineArgs {
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
    pub origin_x_mm: Option<f64>,
    pub origin_y_mm: Option<f64>,
    /// Closed polygon in KiCad millimetres. When set, width/height are ignored.
    pub points: Option<Vec<OutlinePoint>>,
    /// Delete existing Edge.Cuts first (default true).
    pub replace: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectPinsArgs {
    pub ref1: String,
    pub pin1: String,
    pub ref2: String,
    pub pin2: String,
    /// Optional net name, e.g. `"5V"` or `"GND"`.
    pub net: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectPairSpec {
    pub ref1: String,
    pub pin1: String,
    pub ref2: String,
    pub pin2: String,
    pub net: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectManyArgs {
    pub pairs: Vec<ConnectPairSpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTrackArgs {
    pub a_x_mm: f64,
    pub a_y_mm: f64,
    pub b_x_mm: f64,
    pub b_y_mm: f64,
    pub net: String,
    pub layer: Option<String>,
    pub width_mm: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTracksArgs {
    pub tracks: Vec<AddTrackArgs>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddViaArgs {
    pub x_mm: f64,
    pub y_mm: f64,
    pub net: String,
    pub drill_mm: Option<f64>,
    pub size_mm: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddViasArgs {
    pub vias: Vec<AddViaArgs>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetZoneArgs {
    pub origin_x_mm: Option<f64>,
    pub origin_y_mm: Option<f64>,
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
    pub net: String,
    pub layer: Option<String>,
    pub name: Option<String>,
    pub points: Option<Vec<OutlinePoint>>,
}

async fn commit_connect(
    k: &Kicad,
    pairs: &[(crate::nets::PinRef, crate::nets::PinRef, Option<String>)],
) -> Result<serde_json::Value, String> {
    let session = k.begin_commit().await?;
    match crate::nets::connect_pairs(k, pairs).await {
        Ok(connected) => {
            let n = connected.len();
            k.end_commit(session, &format!("kicad-mcp connect {n} pairs"))
                .await?;
            let _ = k.refresh().await;
            let net = connected
                .first()
                .and_then(|r| r.get("net").cloned())
                .unwrap_or(serde_json::Value::Null);
            Ok(serde_json::json!({
                "ok": true,
                "count": n,
                "connected": connected,
                "net": net,
            }))
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            Err(e)
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for KicadMcp {
    fn get_info(&self) -> ServerInfo {
        let write_note = if self.allow_ai_write {
            "Write tools are ENABLED (--allow-ai-write): download_lcsc_part, place_footprint, place_parts, place_matrix, remove_footprint, clear_board, set_board_outline, connect_pins, connect_many, add_track, add_tracks, add_via, add_vias, set_copper_zone, ripup_wire, save_board, export_manufacturing."
        } else {
            "Write tools are DISABLED. Relaunch with --allow-ai-write."
        };
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            format!(
                "Mini MCP for a running KiCad 10 PCB editor (IPC API). \
             Start KiCad 10 via ~/Programme/kicad-10.sh (not the AppImage, not system KiCad 9). \
             KiCad must be open with Preferences → Plugins → Enable IPC API. \
             Coordinates are KiCad native millimetres (origin = board origin, +x right, +y up). \
             LCSC parts come from EasyEDA so JLCPCB footprints match. WirePad_PTH and MountingHole_M3_NPTH \
             are builtins (list_parts writes them). \
             Start with board_summary. Prefer download_lcsc_part then place_matrix/place_parts for grids. \
             The pink A4 frame is the drawing sheet, not the PCB. Board size is an Edge.Cuts rectangle \
             (set_board_outline); default origin is the sheet centre, not 0,0. Outline replace defaults to true. \
             Place on free F.CrtYd space inside the board; placement refuses courtyard overlap. \
             Darkroom panel: darkroom_led_panel_4x5_slim.json (~153 mm round, 109 LEDs). \
             clear_board, set_board_outline with points, place_parts, connect_many, add_tracks/add_vias, set_copper_zone. \
             No move_footprint (remove then place). Copper: get_routing_scene then ripup_wire with segment_id. \
             No autorouter. Do not edit .kicad_pcb by hand. \
             export_manufacturing writes JLCPCB files: <stem>_gerbers.zip + _bom.csv + _cpl.csv (needs kicad-cli). \
             {write_note}"
            ),
        )
    }
}
