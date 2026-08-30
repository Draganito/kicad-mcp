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
        description = "Every pad as hard data straight from KiCad's baked protos: reference, pin, net, absolute x_mm/y_mm, size, rotation, smd/pth/npth, shape, layer, layers (every copper layer the pad exists on — PTH wire pads must list In1/In2 or a 5V pour cannot attach), drill. Optional reference and/or net filter. Use this to verify placement and orientation against reality (a mirrored or mis-rotated part shows pads on the wrong side of the anchor) — never guess from templates or renders."
    )]
    async fn get_pads(
        &self,
        Parameters(args): Parameters<GetPadsArgs>,
    ) -> Result<CallToolResult, McpError> {
        with_kicad(self, move |k| async move {
            let reference = args
                .reference
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let net = args.net.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let pads = crate::pads::board_pads(&k, reference, net).await?;
            Ok(serde_json::json!({
                "pad_count": pads.len(),
                "pads": pads,
            }))
        })
        .await
    }

    #[tool(
        description = "Tracks and vias currently on the board (id, net, layer, endpoints in mm). Optional net filter (e.g. `\"DATA_IN\"`) so you are not dumped every segment. Use track/via id with ripup_wire."
    )]
    async fn get_routing_scene(
        &self,
        Parameters(args): Parameters<GetRoutingSceneArgs>,
    ) -> Result<CallToolResult, McpError> {
        with_kicad(self, move |k| async move {
            let want = args
                .net
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let mut tracks = k.tracks().await?;
            let mut vias = k.vias().await?;
            if let Some(net) = want {
                tracks.retain(|t| t.net.as_deref() == Some(net));
                vias.retain(|v| v.net.as_deref() == Some(net));
            }
            Ok(serde_json::json!({ "tracks": tracks, "vias": vias }))
        })
        .await
    }

    #[tool(
        description = "Footprint templates in this project's jlcpcb_parts.pretty — exact names place_footprint wants, plus F.CrtYd size. has_easyeda_pins true means get_part_pins has EasyEDA pin_name/function. Also writes the default WirePad_PTH and MountingHole_M3_NPTH if missing; other sizes come from make_wire_pad / make_mounting_hole."
    )]
    async fn list_parts(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move {
            let dir = k.project_dir().await?;
            let pretty = crate::kicad::jlc_pretty_dir(&dir);
            easyeda_kicad::ensure_fp_lib_table(&dir.join("fp-lib-table"))
                .map_err(|e| e.to_string())?;
            let _ = crate::builtins::ensure_builtin_footprints(&pretty)?;
            let names =
                easyeda_kicad::list_pretty_footprints(&pretty).map_err(|e| e.to_string())?;
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
                    "has_easyeda_pins": pretty.join(format!("{name}.pins.json")).is_file(),
                }));
            }
            Ok(serde_json::json!({ "library": "jlcpcb_parts", "templates": templates }))
        })
        .await
    }

    #[tool(
        description = "Hard OK/fail placement audit. Recomputes every pad from its jlcpcb_parts template at the footprint's anchor + rotation and compares against the baked pads KiCad actually draws. A mirrored, mis-rotated or stale-baked part fails with per-pad deltas in mm (pin, expected vs actual position, plus size/angle/type/drill mismatches — a lost NPTH hole or a slot baked as a round hole fails too). Thermal clusters with shared pin numbers are matched by nearest position. Optional reference filter; tolerance_mm default 0.01. Footprints without a template are listed as skipped, not failed. Run after placing or moving parts and on boards built with older kicad-mcp versions — trust this over any render."
    )]
    async fn check_placement(
        &self,
        Parameters(args): Parameters<CheckPlacementArgs>,
    ) -> Result<CallToolResult, McpError> {
        with_kicad(self, move |k| async move {
            let reference = args
                .reference
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            crate::pads::check_placement(&k, reference, args.tolerance_mm.unwrap_or(0.01)).await
        })
        .await
    }

    #[tool(
        description = "Connectivity snapshot: footprints, nets, and pads whose net_name is empty or 'unconnected'. For copper clearance / silk / hole DRC use check_drc."
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
        description = "Pin-coverage audit (reads only) — the ERC substitute for the schematic-less workflow. Groups every electrical pad into REF.PIN pins (NPTH and unnumbered pads skipped), reports open_pins (no net) annotated with the EasyEDA pin_name when the template has one, and single_pad_nets (a net reaching exactly one pin — netted but connecting nothing). allow: [\"U1.5\", …] declares intentionally open pins; stale allow entries fail the report. ok only when every pin is netted or explicitly allowed. Run after connect_many, before copper — every open pin must be justified (NC per pin_name or an allow entry), never silently accepted."
    )]
    async fn check_pins(
        &self,
        Parameters(args): Parameters<CheckPinsArgs>,
    ) -> Result<CallToolResult, McpError> {
        with_kicad(self, move |k| async move {
            let pads = crate::pads::board_pads(&k, None, None).await?;
            // Best-effort EasyEDA pin names: footprint value = template name.
            let mut names: std::collections::HashMap<String, std::collections::HashMap<String, String>> =
                std::collections::HashMap::new();
            if let (Ok(fps), Ok(dir)) = (k.footprints().await, k.project_dir().await) {
                let pretty = crate::kicad::jlc_pretty_dir(&dir);
                let sym = crate::kicad::jlc_sym_path(&dir);
                let mut template_of: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for fp in &fps {
                    if let (Some(r), Some(v)) = (fp.reference.as_deref(), fp.value.as_deref()) {
                        template_of.insert(r.to_string(), v.to_string());
                    }
                }
                let mut by_template: std::collections::HashMap<String, std::collections::HashMap<String, String>> =
                    std::collections::HashMap::new();
                for template in template_of.values() {
                    if by_template.contains_key(template) {
                        continue;
                    }
                    let map = easyeda_kicad::load_part_pins(&pretty, &sym, template)
                        .map(|p| {
                            p.pins
                                .into_iter()
                                .filter_map(|pin| pin.pin_name.map(|n| (pin.number, n)))
                                .collect()
                        })
                        .unwrap_or_default();
                    by_template.insert(template.clone(), map);
                }
                for (reference, template) in template_of {
                    if let Some(map) = by_template.get(&template) {
                        names.insert(reference, map.clone());
                    }
                }
            }
            let rows: Vec<crate::coverage::PinRow> = pads
                .iter()
                .map(|p| crate::coverage::PinRow {
                    reference: p.reference.clone(),
                    pin: p.pin.clone(),
                    net: p.net.clone(),
                    pin_name: names
                        .get(&p.reference)
                        .and_then(|m| m.get(&p.pin))
                        .cloned(),
                    x_mm: p.x_mm,
                    y_mm: p.y_mm,
                    kind: p.kind.clone(),
                })
                .collect();
            let allow = args.allow.unwrap_or_default();
            Ok(crate::coverage::coverage(&rows, &allow))
        })
        .await
    }

    #[tool(
        description = "Short layout-physics review of the open board (reads only). Return path / GND pour, power pour, whether 5V and GND sit on adjacent layers, a GND via within 3 mm of each decoupling-cap GND pad, every PTH against those pours, SK6812 daisy (DOUT→DIN, one start, end open), and whether each 0603 GND pad sits next to the companion LED pin 1. Not DRC and not connectivity — those are check_drc and check_board. Does not flag 90° corners or silk overlap. Call after copper, before claiming the board is done. Report: ok, verdict, summary, findings[], not_checked[]."
    )]
    async fn review_board(&self) -> Result<CallToolResult, McpError> {
        with_kicad(self, |k| async move {
            crate::review::review_open_board(&k).await
        })
        .await
    }

    #[tool(
        description = "EasyEDA pin numbers and pin_name (electrical function) for a downloaded template. template may be the full list_parts name or just the LCSC C-number (C5348912). Source of truth for connect_many nets. Do not use a manufacturer datasheet unless a logic check shows the EasyEDA names cannot be right. Call after download_lcsc_part, or for an existing list_parts template. Builtins have pad numbers only."
    )]
    async fn get_part_pins(
        &self,
        Parameters(args): Parameters<GetPartPinsArgs>,
    ) -> Result<CallToolResult, McpError> {
        with_kicad(self, move |k| async move {
            let dir = k.project_dir().await?;
            let pretty = crate::kicad::jlc_pretty_dir(&dir);
            let _ = crate::builtins::ensure_builtin_footprints(&pretty)?;
            let pins = easyeda_kicad::load_part_pins(
                &pretty,
                &crate::kicad::jlc_sym_path(&dir),
                &args.template,
            )?;
            serde_json::to_value(&pins).map_err(|e| e.to_string())
        })
        .await
    }

    #[tool(
        description = "Download LCSC C-number from EasyEDA and write a native KiCad footprint + symbol + pins.json into the open project's jlcpcb_parts library. Returns template (for place_footprint) and pins: [{number, pin_name}]. pin_name is the EasyEDA function — use it for nets. Datasheet only if a logic check proves EasyEDA cannot be right."
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
            let pins = part.part_pins();
            Ok(serde_json::json!({
                "ok": true,
                "lcsc_code": part.lcsc_code,
                "name": part.name,
                "template": name,
                "reference_prefix": part.reference_prefix,
                "pad_count": part.pads.len(),
                "datasheet_url": part.datasheet_url,
                "pins": pins.pins,
                "source": "easyeda",
                "library": "jlcpcb_parts",
                "note": "pins[].pin_name is the EasyEDA function. Use it for connect_many. Manufacturer datasheet only after a logic check that EasyEDA cannot be right. KiCad may need a library refresh before the part appears in the GUI picker. place_footprint pastes it directly.",
            }))
        })
        .await
    }

    #[tool(
        description = "Generate a PTH wire pad template with any copper/drill diameter (mm) and write it into jlcpcb_parts.pretty. Returns the template name for place_footprint (default 2.5/1.5 keeps the name WirePad_PTH, e.g. 3.2/2.0 becomes WirePad_PTH_3.2_2). Regenerating an existing size overwrites the file. Enforces JLCPCB drill limits and 0.25 mm annular ring."
    )]
    async fn make_wire_pad(
        &self,
        Parameters(args): Parameters<MakeWirePadArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let dir = k.project_dir().await?;
            let pretty = crate::kicad::jlc_pretty_dir(&dir);
            easyeda_kicad::ensure_fp_lib_table(&dir.join("fp-lib-table"))
                .map_err(|e| e.to_string())?;
            let pad_mm = args.pad_mm.unwrap_or(crate::builtins::WIRE_PAD_DEFAULT_PAD_MM);
            let drill_mm = args
                .drill_mm
                .unwrap_or(crate::builtins::WIRE_PAD_DEFAULT_DRILL_MM);
            let template = crate::builtins::make_wire_pad(&pretty, pad_mm, drill_mm)?;
            Ok(serde_json::json!({
                "ok": true,
                "template": template,
                "pad_mm": pad_mm,
                "drill_mm": drill_mm,
                "library": "jlcpcb_parts",
                "note": "Use the template name with place_footprint. Already placed footprints keep their old geometry — remove and place again to pick up a new size.",
            }))
        })
        .await
    }

    #[tool(
        description = "Generate an NPTH mounting hole template with any hole diameter (mm) and write it into jlcpcb_parts.pretty. Returns the template name for place_footprint (3.2 keeps the name MountingHole_M3_NPTH, e.g. 4.5 becomes MountingHole_4.5_NPTH). Includes a 7.5 mm copper keepout on M3 (hole + 4.3 mm) so a typical screw head and tightening pressure sit on laminate, not on a pour. Courtyard matches the keepout. Regenerating an existing size overwrites the file."
    )]
    async fn make_mounting_hole(
        &self,
        Parameters(args): Parameters<MakeMountingHoleArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let dir = k.project_dir().await?;
            let pretty = crate::kicad::jlc_pretty_dir(&dir);
            easyeda_kicad::ensure_fp_lib_table(&dir.join("fp-lib-table"))
                .map_err(|e| e.to_string())?;
            let hole_mm = args
                .hole_mm
                .unwrap_or(crate::builtins::MOUNTING_HOLE_DEFAULT_MM);
            let template = crate::builtins::make_mounting_hole(&pretty, hole_mm)?;
            Ok(serde_json::json!({
                "ok": true,
                "template": template,
                "hole_mm": hole_mm,
                "library": "jlcpcb_parts",
                "note": "Use the template name with place_footprint. Already placed footprints keep their old geometry — remove and place again to pick up a new size.",
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
        description = "Place many LCSC footprints in one undo (max 150). Each entry: {template, x_mm, y_mm, rotation_deg?, reference?}. All-or-nothing courtyard check against the board and each other. Prefer this (or place_matrix) over N× place_footprint for grids."
    )]
    async fn place_parts(
        &self,
        Parameters(args): Parameters<PlacePartsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(
            self,
            move |k| async move { place_many(&k, args.parts).await },
        )
        .await
    }

    #[tool(
        description = "Place a rows×cols grid of one LCSC template in one undo (max 150 cells). origin_x_mm/origin_y_mm is cell (0,0); columns go +x, rows go +y. Pitch is centre-to-centre millimetres. Refuses courtyard overlap. Same pad-bake as place_footprint."
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

    #[tool(
        description = "Move and/or rotate one placed footprint by reference to x_mm/y_mm (KiCad native millimetres), optional rotation_deg (omit = keep current). Rigid transform of the anchor and every baked pad in one UpdateItems — nets, reference and padstack geometry survive (better than remove+place). Refuses courtyard overlap at the target. Copper does NOT move: re-route tracks that reached this part. Ctrl+Z undoes."
    )]
    async fn move_footprint(
        &self,
        Parameters(args): Parameters<MoveFootprintArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            crate::pads::move_footprint(
                &k,
                &args.reference,
                args.x_mm,
                args.y_mm,
                args.rotation_deg,
            )
            .await
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
        description = "Rip copper. Pass segment_id (one track/via id from get_routing_scene) or segment_ids (max 500). Ctrl+Z undoes."
    )]
    async fn ripup_wire(
        &self,
        Parameters(args): Parameters<RipupArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let mut ids: Vec<String> = args.segment_ids.unwrap_or_default();
            if let Some(id) = args.segment_id {
                if !id.trim().is_empty() {
                    ids.push(id);
                }
            }
            ids.retain(|id| !id.trim().is_empty());
            if ids.is_empty() {
                return Err(
                    "ripup_wire needs segment_id or segment_ids from get_routing_scene".into(),
                );
            }
            if ids.len() > 500 {
                return Err(format!("ripup_wire max 500 ids (got {})", ids.len()));
            }
            let session = k.begin_commit().await?;
            match k.delete_ids(ids).await {
                Ok(deleted) => {
                    k.end_commit(session, "kicad-mcp ripup").await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "deleted": deleted.len(),
                        "deleted_ids": deleted
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
        description = "Delete every footprint, track, via, zone and board silk text on the open board (Edge.Cuts stays unless you set_board_outline with replace). One undo. Use this to start a board from scratch."
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
            ids.extend(k.board_text_ids().await?);
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
        description = "Place one silkscreen label (board text, not a footprint Value). text + x_mm/y_mm in KiCad millimetres. layer is F.Silkscreen (default) or B.Silkscreen — never F.Cu. size_mm defaults to 1.0 (min 0.8). rotation_deg optional. Use for connector names (5V, GND, DATA). Not U1/C3 refdes — export_manufacturing already strips those. Ctrl+Z undoes."
    )]
    async fn add_text(
        &self,
        Parameters(args): Parameters<AddTextArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let layer = crate::silk::parse_silk_layer(args.layer.as_deref())?;
            let item = crate::silk::text_any(
                &args.text,
                args.x_mm,
                args.y_mm,
                args.layer.as_deref(),
                args.size_mm,
                args.rotation_deg,
            )?;
            let session = k.begin_commit().await?;
            match k.create_items(vec![item]).await {
                Ok(n) => {
                    k.end_commit(session, &format!("kicad-mcp silk {}", args.text.trim()))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "text": args.text.trim(),
                        "x_mm": args.x_mm,
                        "y_mm": args.y_mm,
                        "layer": layer.name,
                        "size_mm": args.size_mm.unwrap_or(1.0),
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
        description = "Place many silkscreen labels in one undo (max 150). Each item is the same as add_text: {text, x_mm, y_mm, layer?, size_mm?, rotation_deg?}."
    )]
    async fn add_texts(
        &self,
        Parameters(args): Parameters<AddTextsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            if args.texts.is_empty() {
                return Err("add_texts needs at least one label".into());
            }
            if args.texts.len() > crate::silk::SILK_MAX {
                return Err(format!(
                    "add_texts max {} (got {})",
                    crate::silk::SILK_MAX,
                    args.texts.len()
                ));
            }
            let mut items = Vec::with_capacity(args.texts.len());
            let mut placed = Vec::with_capacity(args.texts.len());
            for t in &args.texts {
                let layer = crate::silk::parse_silk_layer(t.layer.as_deref())?;
                items.push(crate::silk::text_any(
                    &t.text,
                    t.x_mm,
                    t.y_mm,
                    t.layer.as_deref(),
                    t.size_mm,
                    t.rotation_deg,
                )?);
                placed.push(serde_json::json!({
                    "text": t.text.trim(),
                    "x_mm": t.x_mm,
                    "y_mm": t.y_mm,
                    "layer": layer.name,
                }));
            }
            let n_req = items.len();
            let session = k.begin_commit().await?;
            match k.create_items(items).await {
                Ok(n) => {
                    k.end_commit(session, &format!("kicad-mcp {n_req} silk labels"))
                        .await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "count": n_req,
                        "items_created": n,
                        "placed": placed,
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
        description = "Delete every copper zone on the open board (tracks, vias and footprints stay). One undo. Use this before re-pouring 5V/GND so new GND vias are not swallowed by an old 5V fill."
    )]
    async fn clear_zones(&self) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, |k| async move {
            let ids = k.zone_ids().await?;
            if ids.is_empty() {
                return Ok(serde_json::json!({ "ok": true, "deleted": 0 }));
            }
            let session = k.begin_commit().await?;
            match k.delete_ids(ids).await {
                Ok(deleted) => {
                    k.end_commit(session, "kicad-mcp clear zones").await?;
                    let _ = k.refresh().await;
                    Ok(serde_json::json!({ "ok": true, "deleted": deleted.len() }))
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
        description = "Join two pads onto one net (Pad.net via UpdateItems of the parent footprint). ref1/pin1 and ref2/pin2 are like U1 and 2. Every pad that shares that pin number is assigned (thermal clusters such as U1.41). Optional net names a new one (e.g. \"5V\", \"GND\"). Daisy-chain hops omit net. Persists on KiCad 10. Not copper — use add_track / set_copper_zone after."
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
        description = "Join many pad pairs onto nets in one undo (max 150). Each pair: {ref1, pin1, ref2, pin2, net?}. Same rules as connect_pins (every pad that shares a pin number is assigned). Use this for power rails and daisy-chained signal hops."
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
        description = "Take one pin off its net (Pad.net → unconnected). reference+pin like U16 and 7. connect_pins' inverse, for fixing a mis-wire after a bad EasyEDA name. Every pad that shares that pin number is cleared (thermal clusters). Idempotent if already unconnected. Does not rip copper. Ctrl+Z undoes."
    )]
    async fn disconnect_pin(
        &self,
        Parameters(args): Parameters<DisconnectPinArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let pins = [crate::nets::PinRef {
                reference: args.reference,
                pin: args.pin,
            }];
            commit_disconnect(&k, &pins).await
        })
        .await
    }

    #[tool(
        description = "Take many pins off their nets in one undo (max 150). Each pin: {reference, pin}. Same rules as disconnect_pin (every pad that shares a pin number is cleared)."
    )]
    async fn disconnect_many(
        &self,
        Parameters(args): Parameters<DisconnectManyArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let pins: Vec<_> = args
                .pins
                .into_iter()
                .map(|p| crate::nets::PinRef {
                    reference: p.reference,
                    pin: p.pin,
                })
                .collect();
            commit_disconnect(&k, &pins).await
        })
        .await
    }

    #[tool(
        description = "Create one straight copper track (no autorouter). a_x_mm/a_y_mm to b_x_mm/b_y_mm in KiCad millimetres. net is required (from connect_pins / get_nets). layer is F.Cu, In1.Cu, In2.Cu or B.Cu (default F.Cu). width_mm defaults to 0.25. Ctrl+Z undoes."
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
            let mut codes = board_net_codes(&k).await;
            let item = crate::copper::track_any_coded(
                args.a_x_mm,
                args.a_y_mm,
                args.b_x_mm,
                args.b_y_mm,
                args.width_mm,
                layer,
                &args.net,
                codes.code_for(&args.net),
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
            let mut codes = board_net_codes(&k).await;
            let mut items = Vec::with_capacity(args.tracks.len());
            for t in &args.tracks {
                let layer = crate::copper::parse_copper_layer(t.layer.as_deref())?;
                let code = codes.code_for(&t.net);
                items.push(crate::copper::track_any_coded(
                    t.a_x_mm, t.a_y_mm, t.b_x_mm, t.b_y_mm, t.width_mm, layer, &t.net, code,
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
            let mut codes = board_net_codes(&k).await;
            let item = crate::copper::via_any_coded(
                args.x_mm,
                args.y_mm,
                &args.net,
                args.drill_mm,
                args.size_mm,
                codes.code_for(&args.net),
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
            let mut codes = board_net_codes(&k).await;
            let mut items = Vec::with_capacity(args.vias.len());
            for v in &args.vias {
                let code = codes.code_for(&v.net);
                items.push(crate::copper::via_any_coded(
                    v.x_mm, v.y_mm, &v.net, v.drill_mm, v.size_mm, code,
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
        description = "Place a stitching via + short F.Cu stub next to a pin, radially away from the part. Through-via spans every copper layer on the current stack (2- or 4-layer). Single pin: reference+pin (net optional). Batch: net=\"GND\" or net=\"5V\" stitches every SMD pad on that net that does not already have a same-net via nearby (max 250). Sweeps ±15°…±90° if the natural spot is blocked. Via and stub both refuse pads and tracks. Skip PTH/NPTH. Use GND always; use 5V when the 5V pour is an inner layer (In1.Cu), not F.Cu. drill_mm 0.3, size_mm 0.6, stub 0.25. One undo. Ctrl+Z undoes."
    )]
    async fn stitch_via(
        &self,
        Parameters(args): Parameters<StitchViaArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            crate::stitch::stitch_vias(
                &k,
                crate::stitch::StitchArgs {
                    reference: args.reference,
                    pin: args.pin,
                    net: args.net,
                    drill_mm: args.drill_mm,
                    size_mm: args.size_mm,
                    stub_width_mm: args.stub_width_mm,
                },
            )
            .await
        })
        .await
    }

    #[tool(
        description = "Create a copper zone (pour) and refill. net is required (5V or GND). layer is F.Cu, In1.Cu, In2.Cu or B.Cu (default F.Cu). Pads connect solid by default. thermal=true: PTH pads get 1.2 mm spokes (vias and SMD stay solid). thermal_smd=true: SMD and PTH get 0.4 mm spokes (LED/cap GND on an F.Cu pour; vias stay solid). remove_islands=true: drop disconnected slivers (isolated_copper between LED pads). Default keeps islands. Rectangle: origin+size. Polygon: points. Pads should already sit on that net via connect_pins. Ctrl+Z undoes."
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
            let pads = crate::copper::zone_pad_connect_from_flags(
                args.thermal.unwrap_or(false),
                args.thermal_smd.unwrap_or(false),
            );
            let poly: Vec<(f64, f64)> = args
                .points
                .as_ref()
                .map(|pts| pts.iter().map(|p| (p.x_mm, p.y_mm)).collect())
                .unwrap_or_default();
            let mut codes = board_net_codes(&k).await;
            let net_code = codes.code_for(&args.net);
            let item = if poly.len() >= 3 {
                crate::copper::poly_zone_mm_ex(
                    &poly,
                    layer,
                    &args.net,
                    args.name.as_deref(),
                    net_code,
                    pads,
                    args.remove_islands.unwrap_or(false),
                )?
            } else {
                let ox = args
                    .origin_x_mm
                    .ok_or_else(|| "set_copper_zone needs origin+size or points".to_string())?;
                let oy = args
                    .origin_y_mm
                    .ok_or_else(|| "set_copper_zone needs origin+size or points".to_string())?;
                let w = args
                    .width_mm
                    .ok_or_else(|| "set_copper_zone needs origin+size or points".to_string())?;
                let h = args
                    .height_mm
                    .ok_or_else(|| "set_copper_zone needs origin+size or points".to_string())?;
                crate::copper::rect_zone_any_coded(
                    ox,
                    oy,
                    w,
                    h,
                    layer,
                    &args.net,
                    args.name.as_deref(),
                    net_code,
                    pads,
                    args.remove_islands.unwrap_or(false),
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
                        "thermal": args.thermal.unwrap_or(false),
                        "thermal_smd": args.thermal_smd.unwrap_or(false),
                        "remove_islands": args.remove_islands.unwrap_or(false),
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
        description = "Set the board copper layer count (2, 4, 6 or 8). Copper layers are assigned from the count (F.Cu, In1.Cu…, B.Cu); existing non-copper layers stay. Removing layers deletes their copper and is not undoable. Typical 4-layer power stack: F.Cu data, In1.Cu 5V, In2.Cu + B.Cu GND. Call clear_zones first if old F.Cu/B.Cu pours should not stay."
    )]
    async fn set_copper_layers(
        &self,
        Parameters(args): Parameters<SetCopperLayersArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let count = k.set_copper_layer_count(args.copper_layer_count).await?;
            Ok(serde_json::json!({
                "ok": true,
                "copper_layer_count": count,
            }))
        })
        .await
    }

    #[tool(
        description = "Autoroute named nets via the KiCad Routing Tools CLI (not the wx dialog). nets is required — never all nets. GND/VSS are refused (pour a zone). USB_DN and USB_DP must be passed together (two singles + length-match, not route_diff.py). Optional track_width_mm / via_size_mm / via_drill_mm / clearance_mm; defaults pin JLCPCB-safe floors (0.2 mm clearance, 0.6/0.3 via) so the CLI cannot drop to 0.127. After reload, copper zones are refilled. Saves and reloads — no Ctrl+Z. Needs kicad-routing-tools-setup and KiCad 10 via kicad-10. Only when the human wants autorouting."
    )]
    async fn autoroute_nets(
        &self,
        Parameters(args): Parameters<AutorouteNetsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let opts = crate::autoroute::AutorouteOpts {
                track_width_mm: args.track_width_mm,
                via_size_mm: args.via_size_mm,
                via_drill_mm: args.via_drill_mm,
                clearance_mm: args.clearance_mm,
            };
            crate::autoroute::autoroute_nets(&k, &args.nets, &opts).await
        })
        .await
    }

    #[tool(
        description = "KiCad DRC via kicad-cli: refill zones, save, then report clearance / silk / hole / unconnected-copper violations. Not the same as check_board (that is only empty pad nets). Needs kicad-cli (AppImage 10 preferred). Saves the open board. After a copper batch, call this before claiming the board is done."
    )]
    async fn check_drc(&self) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, |k| async move {
            let _ = k.refill_all_zones().await;
            k.save().await?;
            let board = k.board_file_path().await?;
            crate::fab::run_pcb_drc(&board)
        })
        .await
    }

    #[tool(
        description = "Raytrace the board to a PNG via kicad-cli pcb render (refills zones and saves the open board first, like check_drc). side: top|bottom|left|right|front|back (default top). Optional zoom, rotate [x,y,z] degrees (e.g. [-45,0,45] + perspective for an isometric view), floor, width/height px (default 1600). Writes <stem>_render_<side>.png next to the board (or output path). Read the returned PNG to inspect copper, silkscreen, mask and holes. EasyEDA parts carry no 3D bodies — package orientation still needs get_pads or an external render."
    )]
    async fn render_board(
        &self,
        Parameters(args): Parameters<RenderBoardArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.require_write() {
            return refusal;
        }
        with_kicad(self, move |k| async move {
            let _ = k.refill_all_zones().await;
            k.save().await?;
            let board = k.board_file_path().await?;
            let mut opts = crate::fab::RenderOpts::default();
            if let Some(side) = args.side.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                opts.side = side.to_ascii_lowercase();
            }
            if let Some(z) = args.zoom {
                opts.zoom = z;
            }
            if let Some(w) = args.width {
                opts.width = w;
            }
            if let Some(h) = args.height {
                opts.height = h;
            }
            if let Some(rot) = &args.rotate {
                if rot.len() != 3 {
                    return Err("rotate needs exactly [x, y, z] degrees".into());
                }
                opts.rotate = Some((rot[0], rot[1], rot[2]));
            }
            opts.perspective = args.perspective.unwrap_or(false);
            opts.floor = args.floor.unwrap_or(false);
            let out_file = args
                .output
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from);
            let side = opts.side.clone();
            let png = tokio::task::spawn_blocking(move || {
                crate::fab::render_board_png(&board, out_file, &opts)
            })
            .await
            .map_err(|e| e.to_string())??;
            Ok(serde_json::json!({
                "ok": true,
                "png": png,
                "side": side,
                "note": "Read the PNG to inspect it. Copper, silk, mask and holes are real; EasyEDA footprints have no 3D bodies, so packages are invisible — verify orientation with get_pads.",
            }))
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
        description = "JLCPCB manufacturing bundle: refill zones, save, then write <stem>_gerbers.zip (Gerber + Excellon drill via kicad-cli; silkscreen has no refdes/value text — avoids JLCPCB silk-to-pad DFM), <stem>_cpl.csv (pick & place), <stem>_bom.csv (LCSC). Upload the zip as Gerbers and the two CSVs as BOM/CPL on jlcpcb.com. Optional out_dir (default: project folder). Needs kicad-cli on PATH."
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
            let copper = k.copper_layer_count().await.unwrap_or(2);
            let files = tokio::task::spawn_blocking(move || {
                crate::fab::export_manufacturing(&board, &out_dir, &footprints, copper)
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
    let mut used: Vec<String> = existing
        .iter()
        .filter_map(|f| f.reference.clone())
        .collect();
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

async fn board_net_codes(k: &Kicad) -> crate::nets::NetCodes {
    crate::nets::NetCodes::from_board(&k.board_nets().await.unwrap_or_default())
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

use crate::place::courtyard_of_template;

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ExportManufacturingArgs {
    /// Destination folder. Default: the open KiCad project directory.
    pub out_dir: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AutorouteNetsArgs {
    /// Net names to route. Required. Never `*` or an empty list. GND is refused.
    pub nets: Vec<String>,
    /// Track width in mm. Omit to keep each net's netclass width (floor 0.2).
    pub track_width_mm: Option<f64>,
    /// Via outer diameter in mm. Default 0.6.
    pub via_size_mm: Option<f64>,
    /// Via drill in mm. Default 0.3. Must be smaller than via_size_mm.
    pub via_drill_mm: Option<f64>,
    /// Copper clearance ceiling in mm. Default 0.2 (JLCPCB-safe; pins the fab floor).
    pub clearance_mm: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LcscArgs {
    /// e.g. `"C25804"` or `"C2980298"`.
    pub lcsc_code: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MakeWirePadArgs {
    /// Copper pad diameter in mm (default 2.5). Must leave a 0.25 mm annular ring around the drill.
    pub pad_mm: Option<f64>,
    /// Drill diameter in mm (default 1.5). JLCPCB range 0.3–6.3 mm.
    pub drill_mm: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MakeMountingHoleArgs {
    /// NPTH hole diameter in mm (default 3.2 = M3 clearance). JLCPCB range 0.3–6.3 mm.
    pub hole_mm: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetPartPinsArgs {
    /// Template from `list_parts` / `download_lcsc_part`, or just the LCSC C-number (`C5348912`).
    pub template: String,
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

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct GetRoutingSceneArgs {
    /// Only tracks and vias on this net, e.g. `"DATA_IN"`. Omit for the whole board.
    pub net: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct GetPadsArgs {
    /// Only pads of this footprint, e.g. `"U6"`.
    pub reference: Option<String>,
    /// Only pads on this net, e.g. `"GND"`.
    pub net: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct CheckPinsArgs {
    /// Pins that are intentionally open, as `REF.PIN` (e.g. `["U226.5"]`).
    /// Each entry must match an open pin, otherwise the report fails.
    pub allow: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct CheckPlacementArgs {
    /// Audit only this footprint, e.g. `"U6"`. Omit for the whole board.
    pub reference: Option<String>,
    /// Position/size tolerance in mm (default 0.01).
    pub tolerance_mm: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveFootprintArgs {
    /// Footprint reference, e.g. `"U14"`.
    pub reference: String,
    pub x_mm: f64,
    pub y_mm: f64,
    /// New absolute rotation in degrees (KiCad counterclockwise). Omit to keep.
    pub rotation_deg: Option<f64>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct RenderBoardArgs {
    /// top | bottom | left | right | front | back. Default top.
    pub side: Option<String>,
    /// Camera zoom, default 1.0.
    pub zoom: Option<f64>,
    /// Board rotation `[x, y, z]` in degrees, e.g. `[-45, 0, 45]` for isometric.
    pub rotate: Option<Vec<f64>>,
    /// Perspective projection instead of orthogonal.
    pub perspective: Option<bool>,
    /// Floor, shadows and post-processing.
    pub floor: Option<bool>,
    /// Image width in px (64–4096, default 1600).
    pub width: Option<u32>,
    /// Image height in px (64–4096, default 1600).
    pub height: Option<u32>,
    /// Output PNG path. Default: `<stem>_render_<side>.png` next to the board.
    pub output: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RipupArgs {
    pub segment_id: Option<String>,
    pub segment_ids: Option<Vec<String>>,
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
pub struct DisconnectPinArgs {
    /// Footprint reference, e.g. `"U16"`.
    pub reference: String,
    /// Pad number, e.g. `"7"`.
    pub pin: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DisconnectPinSpec {
    pub reference: String,
    pub pin: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DisconnectManyArgs {
    pub pins: Vec<DisconnectPinSpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTextArgs {
    /// Label, e.g. `"5V"`, `"GND"`, `"DATA"`. Single line, max 80 characters.
    pub text: String,
    pub x_mm: f64,
    pub y_mm: f64,
    /// F.Silkscreen (default) or B.Silkscreen. Copper is refused.
    pub layer: Option<String>,
    /// Height in mm (default 1.0, min 0.8).
    pub size_mm: Option<f64>,
    pub rotation_deg: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTextsArgs {
    pub texts: Vec<AddTextArgs>,
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
pub struct StitchViaArgs {
    /// Footprint reference, e.g. `"U73"`. Required with pin for a single pad.
    pub reference: Option<String>,
    /// Pad number, e.g. `"1"`. Required with reference for a single pad.
    pub pin: Option<String>,
    /// Stitch every SMD pad on this net (e.g. `"GND"`). Or confirm the pin's net.
    pub net: Option<String>,
    pub drill_mm: Option<f64>,
    pub size_mm: Option<f64>,
    pub stub_width_mm: Option<f64>,
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
    /// PTH thermal spokes (1.2 mm). Vias and SMD stay solid. Default false.
    pub thermal: Option<bool>,
    /// SMD + PTH thermal spokes (0.4 mm / 0.3 mm gap). For LED/cap GND on an F.Cu pour. Vias stay solid. Wins over `thermal`.
    pub thermal_smd: Option<bool>,
    /// Drop disconnected copper slivers (`isolated_copper`). Default false (keep islands).
    pub remove_islands: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetCopperLayersArgs {
    /// Even count 2–8. 4 enables F.Cu, In1.Cu, In2.Cu, B.Cu.
    pub copper_layer_count: u32,
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
            let pads_assigned: usize = connected
                .iter()
                .filter_map(|r| r.get("pads").and_then(|v| v.as_u64()))
                .sum::<u64>() as usize;
            Ok(serde_json::json!({
                "ok": true,
                "count": n,
                "pads_assigned": pads_assigned,
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

async fn commit_disconnect(
    k: &Kicad,
    pins: &[crate::nets::PinRef],
) -> Result<serde_json::Value, String> {
    let session = k.begin_commit().await?;
    match crate::nets::disconnect_pins(k, pins).await {
        Ok(disconnected) => {
            let n = disconnected.len();
            k.end_commit(session, &format!("kicad-mcp disconnect {n} pins"))
                .await?;
            let _ = k.refresh().await;
            let pads_cleared: usize = disconnected
                .iter()
                .filter_map(|r| r.get("pads").and_then(|v| v.as_u64()))
                .sum::<u64>() as usize;
            Ok(serde_json::json!({
                "ok": true,
                "count": n,
                "pads_cleared": pads_cleared,
                "disconnected": disconnected,
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
            "Write tools are ENABLED (--allow-ai-write): download_lcsc_part, make_wire_pad, make_mounting_hole, place_footprint, place_parts, place_matrix, move_footprint, remove_footprint, clear_board, clear_zones, set_board_outline, add_text, add_texts, connect_pins, connect_many, disconnect_pin, disconnect_many, add_track, add_tracks, add_via, add_vias, stitch_via, set_copper_zone, set_copper_layers, autoroute_nets, ripup_wire, check_drc, render_board, save_board, export_manufacturing."
        } else {
            "Write tools are DISABLED. Relaunch with --allow-ai-write."
        };
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            format!(
                "Mini MCP for a running KiCad 10 PCB editor (IPC API). \
             Start KiCad 10 via ~/Programme/kicad-10.sh (not the AppImage, not system KiCad 9). \
             KiCad must be open with Preferences → Plugins → Enable IPC API. \
             Coordinates are KiCad native millimetres (origin = board origin, +x right, +y up). \
             LCSC parts come from EasyEDA so JLCPCB footprints match. Wire pads and mounting holes are \
             generated parametrically: list_parts writes the defaults (WirePad_PTH 2.5/1.5 mm, \
             MountingHole_M3_NPTH 3.2 mm); make_wire_pad / make_mounting_hole write any other size. \
             Pin names and functions come from EasyEDA (`download_lcsc_part` returns pins; `get_part_pins` \
             for already downloaded templates — C-number is enough). Use those pin_name values for connect_many. \
             Manufacturer datasheets only after a logic check that EasyEDA cannot be right. \
             Start with board_summary. Prefer download_lcsc_part then place_matrix/place_parts for grids. \
             The pink A4 frame is the drawing sheet, not the PCB. Board size is an Edge.Cuts rectangle \
             (set_board_outline); default origin is the sheet centre, not 0,0. Outline replace defaults to true. \
             Place on free F.CrtYd space inside the board; placement refuses courtyard overlap. \
             add_text / add_texts place F.Silkscreen labels (5V/GND/DATA next to wire pads) — never F.Cu, never footprint Value. \
             Typical write path: clear_board, set_board_outline, place_parts or place_matrix, connect_many \
             (assigns every pad that shares a pin number, e.g. thermal pad 41), \
             disconnect_pin to put a pad back on unconnected after a mis-wire, \
             then check_pins (the ERC substitute: every pin netted or explicitly allowed open via allow: [\"REF.PIN\"], plus nets reaching only one pin) before copper, \
             autoroute_nets for named signal nets (not GND), set_copper_layers then set_copper_zone for 5V/GND. \
             move_footprint relocates/rotates a placed part (rigid transform, nets stay, copper does not move). \
             get_pads reports every pad with absolute position, rotation, net and layers (all copper \
             layers on the padstack) — verify placement with it instead of guessing. check_placement is the hard audit: template pads recomputed at \
             anchor+rotation vs the baked board pads; mirrored/mis-rotated/stale parts fail with mm deltas. \
             Run it after placing or moving. render_board writes a PNG (kicad-cli pcb render) for visual checks \
             (no 3D bodies on EasyEDA parts). Copper: get_routing_scene (optional net) then ripup_wire with segment_id. \
             autoroute_nets calls the Routing Tools CLI, reloads, and refills zones (no Ctrl+Z for that step). \
             After copper, check_drc (kicad-cli), check_board, then review_board (return path / pours / cap vias / PTH thermals / daisy / cap polarity — not 90° corners). \
             Do not edit .kicad_pcb by hand. \
             export_manufacturing writes JLCPCB files: <stem>_gerbers.zip + _bom.csv + _cpl.csv (needs kicad-cli). \
             {write_note}"
            ),
        )
    }
}
