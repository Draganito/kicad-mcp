//! Talk to a running KiCad PCB editor over the official IPC API.
//! One connection, serialized through a mutex — KiCad handles API
//! events on the UI thread and does not want parallel sockets.

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
        let created = self
            .client
            .create_items(items, None)
            .await
            .map_err(fmt_err)?;
        Ok(created.len())
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
            layers
                .layers
                .into_iter()
                .map(|l| (l.id, l.name))
                .collect(),
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
    ViaInfo {
        id: v.id,
        net: v.net.map(|n| n.name),
        x_mm: v.position_nm.map(|p| nm_to_mm(p.x_nm)),
        y_mm: v.position_nm.map(|p| nm_to_mm(p.y_nm)),
    }
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
}
