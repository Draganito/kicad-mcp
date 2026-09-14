//! Talk to a running KiCad PCB editor over the official IPC API.
//! One connection, serialized through a mutex — KiCad handles API
//! events on the UI thread and does not want parallel sockets.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kicad_ipc_rs::client::KiCadClient;
use kicad_ipc_rs::model::board::{
    BoardNet, PadNetEntry, PcbFootprint, PcbItem, PcbTrack, PcbVia, Vector2Nm,
};
use kicad_ipc_rs::model::common::{CommitAction, DocumentType};
use kicad_ipc_rs::{CommitSession, PcbObjectTypeCode};
use prost_types::Any;
use serde::Serialize;

const NM_PER_MM: f64 = 1_000_000.0;

pub fn nm_to_mm(nm: i64) -> f64 {
    nm as f64 / NM_PER_MM
}

#[allow(dead_code)]
pub fn mm_to_nm(mm: f64) -> i64 {
    (mm * NM_PER_MM).round() as i64
}

#[derive(Debug, Serialize)]
pub struct BoardSummary {
    pub kicad_version: String,
    /// Pad/track/via nets persist over IPC from KiCad 10 on.
    pub net_ipc_persists: bool,
    pub project_path: Option<String>,
    pub has_open_board: bool,
    pub copper_layer_count: Option<u32>,
    pub net_count: usize,
    pub footprint_count: usize,
    pub track_count: usize,
    pub via_count: usize,
    pub zone_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn net_ipc_persists(version: &str) -> bool {
    version
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()
        .and_then(|s| s.split('.').next())
        .and_then(|maj| maj.parse::<u32>().ok())
        .is_some_and(|maj| maj >= 10)
}

fn version_note(version: &str) -> Option<String> {
    if net_ipc_persists(version) {
        None
    } else {
        Some(
            "KiCad 9 does not persist Pad.net over IPC. Start KiCad 10 via ~/Programme/kicad-10.sh (scripts/kicad-10.sh)."
                .into(),
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FootprintInfo {
    pub id: Option<String>,
    pub reference: Option<String>,
    pub value: Option<String>,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub rotation_deg: Option<f64>,
    pub layer: String,
    pub pad_count: usize,
}

#[derive(Debug, Serialize)]
pub struct NetInfo {
    pub name: String,
    pub pad_count: usize,
    pub pads: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TrackInfo {
    pub id: Option<String>,
    pub net: Option<String>,
    pub layer: String,
    pub width_mm: Option<f64>,
    pub a_mm: Option<[f64; 2]>,
    pub b_mm: Option<[f64; 2]>,
}

#[derive(Debug, Serialize)]
pub struct ViaInfo {
    pub id: Option<String>,
    pub net: Option<String>,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    /// Drill diameter (mm). Absent when KiCad omits the padstack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drill_mm: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ShapeInfo {
    pub id: Option<String>,
    pub kind: String,
    pub layer: String,
    pub plots: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_x_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_y_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polygon_points: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Grouped overlay BoardText (table cells). Untagged `add_text` labels are omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OverlayGroup {
    pub id: Option<String>,
    pub tag: String,
    pub member_ids: Vec<String>,
}

pub struct Kicad {
    client: KiCadClient,
}

impl Kicad {
    pub async fn connect() -> Result<Self, String> {
        let client = KiCadClient::builder()
            .timeout(Duration::from_secs(180))
            .connect()
            .await
            .map_err(fmt_err)?;
        client.ping().await.map_err(fmt_err)?;
        Ok(Self { client })
    }

    pub async fn summary(&self) -> Result<BoardSummary, String> {
        let version = self.client.get_version().await.map_err(fmt_err)?;
        let has_open_board = self.client.has_open_board().await.unwrap_or(false);
        let project_path = self
            .client
            .get_current_project_path()
            .await
            .ok()
            .map(|p| p.display().to_string());
        if !has_open_board {
            return Ok(BoardSummary {
                kicad_version: version.full_version.clone(),
                net_ipc_persists: net_ipc_persists(&version.full_version),
                project_path,
                has_open_board: false,
                copper_layer_count: None,
                net_count: 0,
                footprint_count: 0,
                track_count: 0,
                via_count: 0,
                zone_count: 0,
                note: version_note(&version.full_version),
            });
        }
        let nets = self.client.get_nets().await.unwrap_or_default();
        let layers = self.client.get_board_enabled_layers().await.ok();
        let footprints = self.footprints().await.unwrap_or_default();
        let tracks = self.tracks().await.unwrap_or_default();
        let vias = self.vias().await.unwrap_or_default();
        let zones = self
            .client
            .get_items_by_type_codes(vec![PcbObjectTypeCode::new_zone().code])
            .await
            .unwrap_or_default();
        Ok(BoardSummary {
            kicad_version: version.full_version.clone(),
            net_ipc_persists: net_ipc_persists(&version.full_version),
            project_path,
            has_open_board: true,
            copper_layer_count: layers.map(|l| l.copper_layer_count),
            net_count: nets.len(),
            footprint_count: footprints.len(),
            track_count: tracks.len(),
            via_count: vias.len(),
            zone_count: zones.len(),
            note: version_note(&version.full_version),
        })
    }

    pub async fn project_dir(&self) -> Result<PathBuf, String> {
        let path = self
            .client
            .get_current_project_path()
            .await
            .map_err(fmt_err)?;
        if path.is_dir() {
            Ok(path)
        } else if let Some(parent) = path.parent() {
            Ok(parent.to_path_buf())
        } else {
            Err("KiCad has no project path — save the board first".into())
        }
    }

    /// Path of the open `.kicad_pcb` on disk (`kicad-cli` needs this).
    pub async fn board_file_path(&self) -> Result<PathBuf, String> {
        let docs = self
            .client
            .get_open_documents(DocumentType::Pcb)
            .await
            .map_err(fmt_err)?;
        let doc = docs
            .into_iter()
            .next()
            .ok_or_else(|| "no open PCB — open a board in KiCad first".to_string())?;
        let name = doc
            .board_filename
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "board has no filename — save it in KiCad first".to_string())?;
        let named = PathBuf::from(&name);
        if named.is_absolute() && named.is_file() {
            return Ok(named);
        }
        let dir = match doc.project.path {
            Some(p) if p.is_dir() => p,
            Some(p) => p.parent().map(Path::to_path_buf).unwrap_or(p),
            None => self.project_dir().await?,
        };
        let candidate = dir.join(&name);
        if candidate.is_file() {
            Ok(candidate)
        } else {
            Err(format!(
                "board file not on disk: {} — save the board in KiCad first",
                candidate.display()
            ))
        }
    }

    pub async fn footprints(&self) -> Result<Vec<FootprintInfo>, String> {
        let items = self
            .client
            .get_items_by_type_codes(vec![PcbObjectTypeCode::new_footprint().code])
            .await
            .map_err(fmt_err)?;
        Ok(items.into_iter().filter_map(footprint_from_item).collect())
    }

    pub async fn board_nets(&self) -> Result<Vec<BoardNet>, String> {
        self.client.get_nets().await.map_err(fmt_err)
    }

    pub async fn pad_netlist(&self) -> Result<Vec<PadNetEntry>, String> {
        self.client.get_pad_netlist().await.map_err(fmt_err)
    }

    pub async fn nets(&self) -> Result<Vec<NetInfo>, String> {
        let nets: Vec<BoardNet> = self.client.get_nets().await.map_err(fmt_err)?;
        let pads = self.pad_netlist().await.unwrap_or_default();
        let mut out = Vec::new();
        for net in nets {
            let matching: Vec<String> = pads
                .iter()
                .filter(|p| p.net_name.as_deref() == Some(net.name.as_str()))
                .map(|p| {
                    let r = p.footprint_reference.as_deref().unwrap_or("?");
                    format!("{r}.{}", p.pad_number)
                })
                .collect();
            out.push(NetInfo {
                name: net.name,
                pad_count: matching.len(),
                pads: matching,
            });
        }
        Ok(out)
    }

    pub async fn tracks(&self) -> Result<Vec<TrackInfo>, String> {
        let items = self
            .client
            .get_items_by_type_codes(vec![PcbObjectTypeCode::new_trace().code])
            .await
            .map_err(fmt_err)?;
        Ok(items.into_iter().filter_map(track_from_item).collect())
    }

    pub async fn vias(&self) -> Result<Vec<ViaInfo>, String> {
        let vias = self.client.get_vias().await.map_err(fmt_err)?;
        Ok(vias.into_iter().map(via_from_pcb).collect())
    }

    pub async fn save(&self) -> Result<(), String> {
        self.client.save_document().await.map_err(fmt_err)
    }

    pub async fn revert_document(&self) -> Result<(), String> {
        self.client.revert_document().await.map_err(fmt_err)
    }

    pub async fn run_action(&self, action: &str) -> Result<(), String> {
        self.client
            .run_action(action)
            .await
            .map(|_| ())
            .map_err(fmt_err)
    }

    pub async fn refresh(&self) -> Result<(), String> {
        use kicad_ipc_rs::model::common::EditorFrameType;
        self.client
            .refresh_editor(EditorFrameType::PcbEditor)
            .await
            .map_err(fmt_err)
    }

    pub async fn begin_commit(&self) -> Result<CommitSession, String> {
        self.client.begin_commit().await.map_err(fmt_err)
    }

    pub async fn end_commit(&self, session: CommitSession, message: &str) -> Result<(), String> {
        self.client
            .end_commit(session, CommitAction::Commit, message.to_string())
            .await
            .map_err(fmt_err)
    }

    pub async fn drop_commit(&self, session: CommitSession) -> Result<(), String> {
        self.client
            .end_commit(session, CommitAction::Drop, "kicad-mcp rollback")
            .await
            .map_err(fmt_err)
    }

    pub async fn create_items(&self, items: Vec<Any>) -> Result<usize, String> {
        Ok(self.create_items_created(items).await?.len())
    }

    pub async fn create_items_created(&self, items: Vec<Any>) -> Result<Vec<Any>, String> {
        self.client.create_items(items, None).await.map_err(fmt_err)
    }

    pub async fn update_items(&self, items: Vec<Any>) -> Result<usize, String> {
        if items.is_empty() {
            return Ok(0);
        }
        let updated = self.client.update_items(items).await.map_err(fmt_err)?;
        Ok(updated.len())
    }

    pub async fn raw_items(&self, type_codes: Vec<i32>) -> Result<Vec<Any>, String> {
        self.client
            .get_items_raw_by_type_codes(type_codes)
            .await
            .map_err(fmt_err)
    }

    pub async fn edge_cuts_ids(&self) -> Result<Vec<String>, String> {
        let items = self
            .client
            .get_items_by_type_codes(vec![PcbObjectTypeCode::new_shape().code])
            .await
            .map_err(fmt_err)?;
        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                PcbItem::BoardGraphicShape(shape)
                    if layer_is_edge_cuts(shape.layer.id, &shape.layer.name) =>
                {
                    shape.id
                }
                _ => None,
            })
            .collect())
    }

    pub async fn copper_zones(&self) -> Result<Vec<crate::copper::ZoneSnap>, String> {
        let raw = self
            .raw_items(vec![PcbObjectTypeCode::new_zone().code])
            .await?;
        Ok(raw
            .into_iter()
            .filter_map(|any| crate::copper::zone_snap_from_any(&any))
            .collect())
    }

    pub async fn zone_ids(&self) -> Result<Vec<String>, String> {
        let items = self
            .client
            .get_items_by_type_codes(vec![PcbObjectTypeCode::new_zone().code])
            .await
            .map_err(fmt_err)?;
        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                PcbItem::Zone(z) => z.id,
                _ => None,
            })
            .collect())
    }

    /// Board overlay graphics on silk / user layers (never Edge.Cuts).
    /// Polygon `polygon_points` is the outline vertex count, not PolySet length.
    /// `tag` is the overlay group name without the `kicad-mcp:` prefix.
    /// Grouped overlay BoardText (table cells) is included as `kind: text`.
    /// Untagged `add_text` labels (5V/GND) are omitted.
    pub async fn board_shapes(
        &self,
        layer: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<ShapeInfo>, String> {
        let filter = match layer {
            Some(name) => Some(crate::graphics::parse_graphic_layer(Some(name))?),
            None => None,
        };
        let want_tag = crate::graphics::parse_overlay_tag(tag)?;
        let id_to_tag = self.overlay_tag_by_member_id().await?;
        let raw = self
            .raw_items(vec![PcbObjectTypeCode::new_shape().code])
            .await?;
        let mut out: Vec<ShapeInfo> = raw
            .into_iter()
            .filter_map(|any| crate::graphics::shape_snap_from_any(&any))
            .filter(|snap| {
                crate::graphics::is_managed_graphic_layer(snap.layer.id, snap.layer.name)
            })
            .filter(|snap| {
                filter
                    .as_ref()
                    .map(|want| snap.layer.id == want.id)
                    .unwrap_or(true)
            })
            .map(|snap| {
                let tag = snap.id.as_ref().and_then(|id| id_to_tag.get(id).cloned());
                shape_from_snap(snap, tag)
            })
            .filter(|info| {
                want_tag
                    .as_ref()
                    .map(|want| info.tag.as_deref() == Some(want.as_str()))
                    .unwrap_or(true)
            })
            .collect();
        if !id_to_tag.is_empty() {
            let items = self
                .client
                .get_items_by_type_codes(vec![PcbObjectTypeCode::new_text().code])
                .await
                .map_err(fmt_err)?;
            let mut texts = Vec::new();
            for item in items {
                let PcbItem::BoardText(t) = item else {
                    continue;
                };
                let Some(id) = t.id.as_ref() else {
                    continue;
                };
                let Some(tag) = id_to_tag.get(id).cloned() else {
                    continue;
                };
                if let Some(want) = want_tag.as_ref() {
                    if tag.as_str() != want.as_str() {
                        continue;
                    }
                }
                let Some(info) = shape_from_overlay_text(t, tag) else {
                    continue;
                };
                if let Some(want) = filter.as_ref() {
                    if crate::graphics::parse_graphic_layer(Some(&info.layer))
                        .map(|l| l.id)
                        .ok()
                        != Some(want.id)
                    {
                        continue;
                    }
                }
                texts.push(info);
            }
            texts.sort_by_key(|s| {
                crate::graphics::overlay_text_read_key(
                    &s.layer,
                    s.x_mm.unwrap_or(0.0),
                    s.y_mm.unwrap_or(0.0),
                )
            });
            out.extend(texts);
        }
        Ok(out)
    }

    /// Ids of managed board graphics (silk / user), never Edge.Cuts.
    pub async fn managed_graphic_ids(
        &self,
        layer: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<String>, String> {
        Ok(self
            .board_shapes(layer, tag)
            .await?
            .into_iter()
            .filter_map(|s| s.id)
            .collect())
    }

    /// Overlay groups we created (`kicad-mcp:<tag>`). Never a user-named group.
    pub async fn overlay_groups(&self) -> Result<Vec<OverlayGroup>, String> {
        let items = self
            .client
            .get_items_by_type_codes(vec![PcbObjectTypeCode::new_group().code])
            .await
            .map_err(fmt_err)?;
        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                PcbItem::Group(g) => {
                    let tag = crate::graphics::tag_from_group_name(&g.name)?;
                    Some(OverlayGroup {
                        id: g.id,
                        tag,
                        member_ids: g.item_ids,
                    })
                }
                _ => None,
            })
            .collect())
    }

    pub async fn overlay_group_ids(&self) -> Result<Vec<String>, String> {
        Ok(self
            .overlay_groups()
            .await?
            .into_iter()
            .filter_map(|g| g.id)
            .collect())
    }

    /// Member + group ids for one overlay tag (and the group object itself).
    pub async fn overlay_ids_for_tag(&self, tag: &str) -> Result<Vec<String>, String> {
        let want = crate::graphics::parse_overlay_tag(Some(tag))?
            .ok_or_else(|| "tag is empty".to_string())?;
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        for g in self.overlay_groups().await? {
            if g.tag != want {
                continue;
            }
            for id in g.member_ids {
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
            if let Some(id) = g.id {
                if seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }

    async fn overlay_tag_by_member_id(&self) -> Result<HashMap<String, String>, String> {
        let mut map = HashMap::new();
        for g in self.overlay_groups().await? {
            for id in g.member_ids {
                map.insert(id, g.tag.clone());
            }
        }
        Ok(map)
    }

    /// Create `kicad-mcp:<tag>` groups. Members must already be on the board
    /// (previous commit). KiCad 10.0.6 `PCB_GROUP::GetLayerSet` is the union of
    /// members found on the board; same-commit members look empty and CreateItems
    /// returns `no overlapping layers`.
    pub async fn create_overlay_groups(
        &self,
        tags: &[(String, Vec<String>)],
    ) -> Result<usize, String> {
        let groups: Vec<_> = tags
            .iter()
            .filter(|(_, members)| !members.is_empty())
            .map(|(tag, members)| {
                crate::graphics::group_any(&crate::graphics::group_name_for_tag(tag), members)
            })
            .collect();
        if groups.is_empty() {
            return Ok(0);
        }
        let session = self.begin_commit().await?;
        match self.create_items(groups).await {
            Ok(n) => {
                self.end_commit(session, "kicad-mcp overlay tag group")
                    .await?;
                let _ = self.refresh().await;
                Ok(n)
            }
            Err(e) => {
                let _ = self.drop_commit(session).await;
                Err(e)
            }
        }
    }

    /// Free board text and text boxes (not footprint Reference/Value fields).
    pub async fn board_text_ids(&self) -> Result<Vec<String>, String> {
        let items = self
            .client
            .get_items_by_type_codes(vec![
                PcbObjectTypeCode::new_text().code,
                PcbObjectTypeCode::new_textbox().code,
            ])
            .await
            .map_err(fmt_err)?;
        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                PcbItem::BoardText(t) => t.id,
                PcbItem::BoardTextBox(t) => t.id,
                _ => None,
            })
            .collect())
    }

    pub async fn refill_all_zones(&self) -> Result<(), String> {
        self.client.refill_all_zones().await.map_err(fmt_err)
    }

    pub async fn enabled_layers(&self) -> Result<(u32, Vec<(i32, String)>), String> {
        let layers = self
            .client
            .get_board_enabled_layers()
            .await
            .map_err(fmt_err)?;
        Ok((
            layers.copper_layer_count,
            layers.layers.into_iter().map(|l| (l.id, l.name)).collect(),
        ))
    }

    pub async fn copper_layer_count(&self) -> Result<u32, String> {
        Ok(self
            .client
            .get_board_enabled_layers()
            .await
            .map_err(fmt_err)?
            .copper_layer_count)
    }

    /// Even count 2–8. Non-copper layers are kept. Removing copper deletes
    /// items on those layers and is not undoable.
    pub async fn set_copper_layer_count(&self, count: u32) -> Result<u32, String> {
        if count < 2 || count > 8 || count % 2 != 0 {
            return Err("copper_layer_count must be 2, 4, 6 or 8".into());
        }
        let current = self
            .client
            .get_board_enabled_layers()
            .await
            .map_err(fmt_err)?;
        if current.copper_layer_count == count {
            return Ok(count);
        }
        let non_copper: Vec<i32> = current
            .layers
            .iter()
            .filter(|l| !crate::copper::is_copper_layer_id(l.id))
            .map(|l| l.id)
            .collect();
        let updated = self
            .client
            .set_board_enabled_layers(count, non_copper)
            .await
            .map_err(fmt_err)?;
        Ok(updated.copper_layer_count)
    }

    pub async fn delete_ids(&self, ids: Vec<String>) -> Result<Vec<String>, String> {
        self.client.delete_items(ids).await.map_err(fmt_err)
    }

    pub async fn footprint_id_by_reference(
        &self,
        reference: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .footprints()
            .await?
            .into_iter()
            .find(|f| f.reference.as_deref() == Some(reference))
            .and_then(|f| f.id))
    }

    #[allow(dead_code)]
    pub async fn open_board_documents(&self) -> Result<Vec<String>, String> {
        let docs = self
            .client
            .get_open_documents(DocumentType::Pcb)
            .await
            .map_err(fmt_err)?;
        Ok(docs.into_iter().filter_map(|d| d.board_filename).collect())
    }
}

fn footprint_from_item(item: PcbItem) -> Option<FootprintInfo> {
    match item {
        PcbItem::Footprint(fp) => Some(footprint_info(fp)),
        _ => None,
    }
}

fn footprint_info(fp: PcbFootprint) -> FootprintInfo {
    FootprintInfo {
        id: fp.id,
        reference: fp.reference,
        value: fp.value,
        x_mm: fp.position_nm.map(|p| nm_to_mm(p.x_nm)),
        y_mm: fp.position_nm.map(|p| nm_to_mm(p.y_nm)),
        rotation_deg: fp.orientation_deg,
        layer: fp.layer.name,
        pad_count: fp.pad_count,
    }
}

fn track_from_item(item: PcbItem) -> Option<TrackInfo> {
    match item {
        PcbItem::Track(t) => Some(track_info(t)),
        _ => None,
    }
}

fn track_info(t: PcbTrack) -> TrackInfo {
    TrackInfo {
        id: t.id,
        net: t.net.map(|n| n.name),
        layer: t.layer.name,
        width_mm: t.width_nm.map(nm_to_mm),
        a_mm: t.start_nm.map(vec_mm),
        b_mm: t.end_nm.map(vec_mm),
    }
}

fn via_from_pcb(v: PcbVia) -> ViaInfo {
    let drill_mm = v
        .pad_stack
        .as_ref()
        .and_then(|s| s.drill.as_ref())
        .and_then(|d| d.diameter_nm)
        .map(|p| nm_to_mm(p.x_nm))
        .filter(|d| *d > 0.0);
    ViaInfo {
        id: v.id,
        net: v.net.map(|n| n.name),
        x_mm: v.position_nm.map(|p| nm_to_mm(p.x_nm)),
        y_mm: v.position_nm.map(|p| nm_to_mm(p.y_nm)),
        drill_mm,
    }
}

fn shape_from_snap(snap: crate::graphics::GraphicSnap, tag: Option<String>) -> ShapeInfo {
    ShapeInfo {
        id: snap.id,
        kind: snap.kind.to_string(),
        layer: snap.layer.name.to_string(),
        plots: snap.layer.plots,
        stroke_mm: snap.stroke_mm,
        origin_x_mm: snap.origin_x_mm,
        origin_y_mm: snap.origin_y_mm,
        width_mm: snap.width_mm,
        height_mm: snap.height_mm,
        x_mm: snap.x_mm,
        y_mm: snap.y_mm,
        radius_mm: snap.radius_mm,
        a_mm: snap.a_mm,
        b_mm: snap.b_mm,
        polygon_points: snap.polygon_points,
        tag,
        text: None,
    }
}

fn shape_from_overlay_text(
    t: kicad_ipc_rs::model::board::PcbBoardText,
    tag: String,
) -> Option<ShapeInfo> {
    let layer = crate::graphics::graphic_layer_from_id(t.layer.id)?;
    if !crate::graphics::is_managed_graphic_layer(layer.id, layer.name) {
        return None;
    }
    Some(ShapeInfo {
        id: t.id,
        kind: "text".into(),
        layer: layer.name.to_string(),
        plots: layer.plots,
        stroke_mm: None,
        origin_x_mm: None,
        origin_y_mm: None,
        width_mm: None,
        height_mm: None,
        x_mm: t.position_nm.map(|p| nm_to_mm(p.x_nm)),
        y_mm: t.position_nm.map(|p| nm_to_mm(p.y_nm)),
        radius_mm: None,
        a_mm: None,
        b_mm: None,
        polygon_points: None,
        tag: Some(tag),
        text: t.text,
    })
}

fn vec_mm(p: Vector2Nm) -> [f64; 2] {
    [nm_to_mm(p.x_nm), nm_to_mm(p.y_nm)]
}

/// KiCad IPC reports Edge.Cuts as proto name `BL_Edge_Cuts`, not the UI name.
fn layer_is_edge_cuts(id: i32, name: &str) -> bool {
    id == crate::outline::BL_EDGE_CUTS
        || name.eq_ignore_ascii_case("Edge.Cuts")
        || name.eq_ignore_ascii_case("BL_Edge_Cuts")
        || name.eq_ignore_ascii_case("Edge_Cuts")
}

fn fmt_err(err: impl std::fmt::Display) -> String {
    let text = err.to_string();
    if text.contains("connect") || text.contains("socket") || text.contains("No such file") {
        format!(
            "{text} — start KiCad 10 (`~/Programme/kicad-10.sh`), open a board, and enable Preferences → Plugins → Enable IPC API"
        )
    } else {
        text
    }
}

pub fn jlc_pretty_dir(project: &Path) -> PathBuf {
    project.join("jlcpcb_parts.pretty")
}

pub fn jlc_sym_path(project: &Path) -> PathBuf {
    project.join("jlcpcb_parts.kicad_sym")
}

#[cfg(test)]
mod tests {
    use super::{layer_is_edge_cuts, net_ipc_persists};

    #[test]
    fn edge_cuts_matches_ipc_proto_name() {
        assert!(layer_is_edge_cuts(47, "BL_Edge_Cuts"));
        assert!(layer_is_edge_cuts(0, "Edge.Cuts"));
        assert!(!layer_is_edge_cuts(3, "BL_F_Cu"));
    }

    #[test]
    fn net_ipc_from_kicad_10() {
        assert!(!net_ipc_persists("9.0.2+dfsg-1"));
        assert!(net_ipc_persists("10.0.5"));
        assert!(net_ipc_persists("10.0.5+dfsg-1"));
    }

    #[tokio::test]
    #[ignore = "needs a running KiCad PCB editor with IPC API"]
    async fn overlay_tag_group_after_members_committed() {
        let k = super::Kicad::connect().await.expect("KiCad IPC");
        assert!(
            k.summary().await.expect("summary").has_open_board,
            "open a board in KiCad first"
        );

        // Drop leftover comments from earlier MCP tests; keep silk and the two 4x5 frames.
        let keep: Vec<_> = k
            .board_shapes(None, None)
            .await
            .expect("shapes")
            .into_iter()
            .filter(|s| {
                s.plots
                    || (s.kind == "rect"
                        && s.width_mm.is_some()
                        && s.height_mm.is_some()
                        && ((s.width_mm.unwrap() - 101.6).abs() < 0.05
                            && (s.height_mm.unwrap() - 127.0).abs() < 0.05
                            || (s.width_mm.unwrap() - 127.0).abs() < 0.05
                                && (s.height_mm.unwrap() - 101.6).abs() < 0.05))
            })
            .filter_map(|s| s.id)
            .collect();
        let leftover: Vec<_> = k
            .board_shapes(Some("Cmts.User"), None)
            .await
            .expect("comments")
            .into_iter()
            .filter_map(|s| s.id)
            .filter(|id| !keep.contains(id))
            .collect();
        let mut cleanup = leftover;
        cleanup.extend(k.overlay_ids_for_tag("mcp-test").await.unwrap_or_default());
        cleanup.sort();
        cleanup.dedup();
        if !cleanup.is_empty() {
            let session = k.begin_commit().await.expect("cleanup commit");
            match k.delete_ids(cleanup).await {
                Ok(_) => {
                    k.end_commit(session, "kicad-mcp live overlay cleanup")
                        .await
                        .expect("end cleanup");
                }
                Err(e) => {
                    let _ = k.drop_commit(session).await;
                    panic!("cleanup leftover overlay failed: {e}");
                }
            }
            let _ = k.refresh().await;
        }

        let before = k.board_shapes(None, None).await.expect("before");
        let silk = before.iter().filter(|s| s.plots).count();
        let frames = before
            .iter()
            .filter(|s| !s.plots && s.kind == "rect")
            .count();
        assert!(silk >= 1, "expected the 40×20 silk test rect to stay");
        assert!(frames >= 2, "expected the two 4×5 comment frames to stay");

        let spec = crate::graphics::ShapeSpec {
            kind: "table".into(),
            center_x_mm: Some(148.5),
            center_y_mm: Some(105.0),
            rows: Some(2),
            cols: Some(3),
            cell_width_mm: Some(8.0),
            cell_height_mm: Some(6.0),
            tag: Some("mcp-test".into()),
            ..Default::default()
        };
        let made = crate::graphics::shape_items(&spec).expect("table");
        assert_eq!(made.len(), 4);
        let items: Vec<_> = made.into_iter().map(|m| m.item).collect();
        let session = k.begin_commit().await.expect("shapes commit");
        let created = match k.create_items_created(items).await {
            Ok(c) => {
                k.end_commit(session, "kicad-mcp live overlay table")
                    .await
                    .expect("end shapes");
                let _ = k.refresh().await;
                c
            }
            Err(e) => {
                let _ = k.drop_commit(session).await;
                panic!("create table failed: {e}");
            }
        };
        let ids: Vec<String> = created
            .iter()
            .filter_map(crate::graphics::graphic_id_from_any)
            .collect();
        assert_eq!(ids.len(), 4, "KiCad must return shape ids");
        k.create_overlay_groups(&[("mcp-test".into(), ids)])
            .await
            .expect("group after members on board");

        let tagged = k
            .board_shapes(None, Some("mcp-test"))
            .await
            .expect("tagged");
        assert_eq!(tagged.len(), 4, "table expands to 4 tagged items");
        assert!(tagged.iter().all(|s| s.tag.as_deref() == Some("mcp-test")));
        assert!(tagged.iter().all(|s| s.layer == "Cmts.User" && !s.plots));

        let after_add = k.board_shapes(None, None).await.expect("after add");
        assert!(after_add.iter().any(|s| s.plots), "silk overlay survived");
        assert!(
            after_add
                .iter()
                .filter(|s| !s.plots && s.kind == "rect" && s.tag.is_none())
                .count()
                >= 2,
            "untagged 4×5 frames survived tagging"
        );

        let ids = k.overlay_ids_for_tag("mcp-test").await.expect("tag ids");
        let session = k.begin_commit().await.expect("clear tag");
        match k.delete_ids(ids).await {
            Ok(_) => {
                k.end_commit(session, "kicad-mcp live clear mcp-test")
                    .await
                    .expect("end clear");
            }
            Err(e) => {
                let _ = k.drop_commit(session).await;
                panic!("clear tag failed: {e}");
            }
        }
        let _ = k.refresh().await;
        let leftover_tag = k
            .board_shapes(None, Some("mcp-test"))
            .await
            .expect("cleared");
        assert!(leftover_tag.is_empty(), "tag mcp-test must be gone");
        let done = k.board_shapes(None, None).await.expect("final");
        assert!(done.iter().any(|s| s.plots), "silk still there after clear");
        assert!(
            done.iter().filter(|s| !s.plots && s.kind == "rect").count() >= 2,
            "4×5 frames still there after clear"
        );
    }
}
