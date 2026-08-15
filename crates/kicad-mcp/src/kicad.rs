//! Talk to a running KiCad PCB editor over the official IPC API.
//! One connection, serialized through a mutex — KiCad handles API
//! events on the UI thread and does not want parallel sockets.

use std::path::{Path, PathBuf};

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
    pub project_path: Option<String>,
    pub has_open_board: bool,
    pub copper_layer_count: Option<u32>,
    pub net_count: usize,
    pub footprint_count: usize,
    pub track_count: usize,
    pub via_count: usize,
    pub zone_count: usize,
}

#[derive(Debug, Serialize)]
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
        let client = KiCadClient::connect().await.map_err(fmt_err)?;
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
                kicad_version: version.full_version,
                project_path,
                has_open_board: false,
                copper_layer_count: None,
                net_count: 0,
                footprint_count: 0,
                track_count: 0,
                via_count: 0,
                zone_count: 0,
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
            kicad_version: version.full_version,
            project_path,
            has_open_board: true,
            copper_layer_count: layers.map(|l| l.copper_layer_count),
            net_count: nets.len(),
            footprint_count: footprints.len(),
            track_count: tracks.len(),
            via_count: vias.len(),
            zone_count: zones.len(),
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

    pub async fn refill_all_zones(&self) -> Result<(), String> {
        self.client.refill_all_zones().await.map_err(fmt_err)
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
            "{text} — start KiCad 9+, open a board, and enable Preferences → Plugins → Enable IPC API"
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
    use super::layer_is_edge_cuts;

    #[test]
    fn edge_cuts_matches_ipc_proto_name() {
        assert!(layer_is_edge_cuts(47, "BL_Edge_Cuts"));
        assert!(layer_is_edge_cuts(0, "Edge.Cuts"));
        assert!(!layer_is_edge_cuts(3, "BL_F_Cu"));
    }
}
