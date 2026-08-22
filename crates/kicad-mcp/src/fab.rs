//! JLCPCB manufacturing bundle from the open KiCad board.
//!
//! Gerbers and drill come from `kicad-cli` (KiCad's own plotter — we do
//! not parse `.kicad_pcb`). Silk omits footprint reference/value text
//! (`--exclude-refdes` / `--exclude-value`) so JLCPCB DFM does not flag
//! silkscreen-to-pad / silkscreen-to-hole on dense boards. BOM and CPL
//! are rewritten to JLCPCB's CSV columns:
//! `<stem>_gerbers.zip`, `<stem>_cpl.csv`, `<stem>_bom.csv`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::builtins::{MOUNTING_HOLE_M3, WIRE_PAD};
use crate::kicad::FootprintInfo;

const JLC_MECH_LAYERS: &str = "F.Paste,B.Paste,F.SilkS,B.SilkS,F.Mask,B.Mask,Edge.Cuts";

const PLOT_EXTS: &[&str] = &[
    "gtl", "gbl", "gts", "gbs", "gto", "gbo", "gtp", "gbp", "gm1", "gm2", "gm3", "gbr", "gbrjob",
    "drl", "gd1", "g1", "g2", "g3", "g4", "g5", "g6", "g7",
];

/// Copper layers for `kicad-cli pcb export gerbers` plus JLCPCB mechanicals.
pub fn jlc_gerber_layers(copper_layer_count: u32) -> String {
    let count = copper_layer_count.clamp(2, 8);
    let mut layers = vec!["F.Cu".to_string()];
    for i in 1..=count.saturating_sub(2) {
        layers.push(format!("In{i}.Cu"));
    }
    layers.push("B.Cu".to_string());
    layers.push(JLC_MECH_LAYERS.to_string());
    layers.join(",")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManufacturingFiles {
    pub gerber_zip: PathBuf,
    pub cpl_csv: PathBuf,
    pub bom_csv: PathBuf,
    pub bom_rows: usize,
    pub cpl_rows: usize,
    pub gerber_files: Vec<String>,
}

pub struct BomRow {
    pub comment: String,
    pub designators: Vec<String>,
    pub footprint: String,
    pub lcsc_part_number: String,
}

pub fn export_manufacturing(
    board_file: &Path,
    out_dir: &Path,
    footprints: &[FootprintInfo],
    copper_layer_count: u32,
) -> Result<ManufacturingFiles, String> {
    if !board_file.is_file() {
        return Err(format!(
            "board file not on disk: {} — save the board in KiCad first",
            board_file.display()
        ));
    }
    let stem = board_file
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .ok_or("board file has no name")?
        .to_string();
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;

    let plot_dir = out_dir.join(format!(".{stem}.kicad-mcp-plot"));
    if plot_dir.exists() {
        fs::remove_dir_all(&plot_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&plot_dir).map_err(|e| e.to_string())?;

    let result = (|| {
        run_kicad_cli(&[
            "pcb",
            "export",
            "gerbers",
            "--output",
            plot_dir.to_str().ok_or("plot dir is not UTF-8")?,
            "--layers",
            &jlc_gerber_layers(copper_layer_count),
            "--exclude-refdes",
            "--exclude-value",
            "--subtract-soldermask",
            "--disable-aperture-macros",
            board_file.to_str().ok_or("board path is not UTF-8")?,
        ])?;
        run_kicad_cli(&[
            "pcb",
            "export",
            "drill",
            "--output",
            plot_dir.to_str().ok_or("plot dir is not UTF-8")?,
            "--format",
            "excellon",
            "--excellon-units",
            "mm",
            "--excellon-zeros-format",
            "decimal",
            "--excellon-separate-th",
            board_file.to_str().ok_or("board path is not UTF-8")?,
        ])?;
        let pos_raw = plot_dir.join("kicad-pos.csv");
        run_kicad_cli(&[
            "pcb",
            "export",
            "pos",
            "--output",
            pos_raw.to_str().ok_or("pos path is not UTF-8")?,
            "--format",
            "csv",
            "--units",
            "mm",
            "--side",
            "both",
            "--exclude-dnp",
            board_file.to_str().ok_or("board path is not UTF-8")?,
        ])?;

        let gerber_names = collect_plot_files(&plot_dir)?;
        if gerber_names.is_empty() {
            return Err("kicad-cli wrote no gerber/drill files".into());
        }
        let gerber_zip = out_dir.join(format!("{stem}_gerbers.zip"));
        zip_files(&plot_dir, &gerber_names, &gerber_zip)?;

        let pos_text = fs::read_to_string(&pos_raw).map_err(|e| e.to_string())?;
        let cpl = ki_pos_to_jlcpcb_cpl(&pos_text)?;
        let cpl_csv = out_dir.join(format!("{stem}_cpl.csv"));
        fs::write(&cpl_csv, &cpl).map_err(|e| e.to_string())?;

        let bom_rows = build_bom_rows(footprints);
        let bom = bom_to_csv(&bom_rows);
        let bom_csv = out_dir.join(format!("{stem}_bom.csv"));
        fs::write(&bom_csv, &bom).map_err(|e| e.to_string())?;

        Ok(ManufacturingFiles {
            gerber_zip,
            cpl_csv,
            bom_csv,
            bom_rows: bom_rows.len(),
            cpl_rows: cpl.lines().count().saturating_sub(1),
            gerber_files: gerber_names,
        })
    })();

    let _ = fs::remove_dir_all(&plot_dir);
    result
}

fn kicad_cli_bin() -> PathBuf {
    let from_env = std::env::var_os("KICAD_CLI").map(PathBuf::from);
    if let Some(p) = from_env {
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    // Debian's /usr/bin/kicad-cli is 9.x here and cannot load a board
    // saved by the KiCad 10 AppImage. Prefer the running AppImage CLI.
    if let Some(appimage) = appimage_kicad_cli() {
        return appimage;
    }
    let debian = PathBuf::from("/usr/bin/kicad-cli");
    if debian.is_file() {
        debian
    } else {
        PathBuf::from("kicad-cli")
    }
}

fn appimage_kicad_cli() -> Option<PathBuf> {
    let tmp = Path::new("/tmp");
    let mut found = Vec::new();
    for entry in fs::read_dir(tmp).ok()?.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(".mount_kicad") {
            continue;
        }
        let cli = entry.path().join("bin").join("kicad-cli");
        if cli.is_file() {
            found.push(cli);
        }
    }
    found.sort();
    found.pop()
}

fn run_kicad_cli(args: &[&str]) -> Result<(), String> {
    let bin = kicad_cli_bin();
    let out = Command::new(&bin).args(args).output().map_err(|e| {
        format!(
            "couldn't run {}: {e} (install the kicad package)",
            bin.display()
        )
    })?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    Err(format!(
        "{} {} failed: {}{}",
        bin.display(),
        args.join(" "),
        stderr.trim(),
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" ({})", stdout.trim())
        }
    ))
}

const DRC_VIOLATION_CAP: usize = 40;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DrcReport {
    pub ok: bool,
    pub error_count: usize,
    pub warning_count: usize,
    pub unconnected_count: usize,
    pub violations: Vec<DrcHit>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DrcHit {
    pub severity: String,
    pub kind: String,
    pub description: String,
}

/// Run `kicad-cli pcb drc` on a board already on disk. Does not parse the
/// `.kicad_pcb` — KiCad's own checker writes JSON.
pub fn run_pcb_drc(board_file: &Path) -> Result<DrcReport, String> {
    if !board_file.is_file() {
        return Err(format!("board file not on disk: {}", board_file.display()));
    }
    let out_path = std::env::temp_dir().join(format!("kicad-mcp-drc-{}.json", std::process::id()));
    let out_s = out_path.to_string_lossy().into_owned();
    let board_s = board_file.to_string_lossy().into_owned();
    run_kicad_cli(&[
        "pcb",
        "drc",
        "--format",
        "json",
        "--units",
        "mm",
        "--severity-all",
        "--refill-zones",
        "--output",
        &out_s,
        &board_s,
    ])?;
    let text = fs::read_to_string(&out_path).map_err(|e| format!("drc json: {e}"))?;
    let _ = fs::remove_file(&out_path);
    parse_drc_json(&text)
}

pub const RENDER_SIDES: &[&str] = &["top", "bottom", "left", "right", "front", "back"];

#[derive(Debug, Clone)]
pub struct RenderOpts {
    pub side: String,
    pub width: u32,
    pub height: u32,
    pub zoom: f64,
    /// Board rotation in degrees, e.g. (-45, 0, 45) for an isometric view.
    pub rotate: Option<(f64, f64, f64)>,
    pub perspective: bool,
    pub floor: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            side: "top".into(),
            width: 1600,
            height: 1600,
            zoom: 1.0,
            rotate: None,
            perspective: false,
            floor: false,
        }
    }
}

/// Raytrace the board on disk to a PNG via `kicad-cli pcb render`.
/// The caller saves the board first (same contract as `run_pcb_drc`).
pub fn render_board_png(
    board_file: &Path,
    out_file: Option<PathBuf>,
    opts: &RenderOpts,
) -> Result<PathBuf, String> {
    if !board_file.is_file() {
        return Err(format!(
            "board file not on disk: {} — save the board in KiCad first",
            board_file.display()
        ));
    }
    if !RENDER_SIDES.contains(&opts.side.as_str()) {
        return Err(format!(
            "side must be one of {} (got {})",
            RENDER_SIDES.join("/"),
            opts.side
        ));
    }
    if !(64..=4096).contains(&opts.width) || !(64..=4096).contains(&opts.height) {
        return Err("width/height must be between 64 and 4096 pixels".into());
    }
    if !(0.05..=20.0).contains(&opts.zoom) {
        return Err("zoom must be between 0.05 and 20".into());
    }
    let out = match out_file {
        Some(p) => p,
        None => {
            let stem = board_file
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("board file has no name")?;
            board_file
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{stem}_render_{}.png", opts.side))
        }
    };
    let args = render_cli_args(board_file, &out, opts)?;
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_kicad_cli(&arg_refs)?;
    let size = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(format!(
            "kicad-cli pcb render wrote no image at {}",
            out.display()
        ));
    }
    Ok(out)
}

fn render_cli_args(
    board_file: &Path,
    out: &Path,
    opts: &RenderOpts,
) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = vec![
        "pcb".into(),
        "render".into(),
        "--output".into(),
        out.to_str().ok_or("output path is not UTF-8")?.into(),
        "--side".into(),
        opts.side.clone(),
        "--background".into(),
        "opaque".into(),
        "--quality".into(),
        "high".into(),
        "--width".into(),
        opts.width.to_string(),
        "--height".into(),
        opts.height.to_string(),
        "--zoom".into(),
        opts.zoom.to_string(),
    ];
    if let Some((rx, ry, rz)) = opts.rotate {
        args.push("--rotate".into());
        args.push(format!("{rx},{ry},{rz}"));
    }
    if opts.perspective {
        args.push("--perspective".into());
    }
    if opts.floor {
        args.push("--floor".into());
    }
    args.push(board_file.to_str().ok_or("board path is not UTF-8")?.into());
    Ok(args)
}

fn parse_drc_json(text: &str) -> Result<DrcReport, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("drc json parse: {e}"))?;
    let mut hits = Vec::new();
    collect_drc_hits(&v, "violations", &mut hits);
    let unconnected = v
        .get("unconnected_items")
        .and_then(|x| x.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let error_count = hits.iter().filter(|h| h.severity == "error").count();
    let warning_count = hits.iter().filter(|h| h.severity == "warning").count();
    if hits.len() > DRC_VIOLATION_CAP {
        hits.truncate(DRC_VIOLATION_CAP);
    }
    Ok(DrcReport {
        ok: error_count == 0,
        error_count,
        warning_count,
        unconnected_count: unconnected,
        violations: hits,
    })
}

fn collect_drc_hits(v: &serde_json::Value, key: &str, hits: &mut Vec<DrcHit>) {
    let Some(arr) = v.get(key).and_then(|x| x.as_array()) else {
        return;
    };
    for item in arr {
        let severity = item
            .get("severity")
            .and_then(|x| x.as_str())
            .unwrap_or("error")
            .to_string();
        let kind = item
            .get("type")
            .or_else(|| item.get("description"))
            .and_then(|x| x.as_str())
            .unwrap_or("drc")
            .to_string();
        let description = item
            .get("description")
            .or_else(|| item.get("comment"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        hits.push(DrcHit {
            severity,
            kind,
            description,
        });
    }
}

fn collect_plot_files(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if PLOT_EXTS.contains(&ext.as_str()) {
            names.push(
                path.file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("plot file name is not UTF-8")?
                    .to_string(),
            );
        }
    }
    names.sort();
    Ok(names)
}

fn zip_files(dir: &Path, names: &[String], zip_path: &Path) -> Result<(), String> {
    let file = fs::File::create(zip_path).map_err(|e| e.to_string())?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in names {
        let bytes = fs::read(dir.join(name)).map_err(|e| e.to_string())?;
        writer
            .start_file(name, options)
            .map_err(|e| e.to_string())?;
        writer.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn exclude_from_bom(value: &str) -> bool {
    value == WIRE_PAD
        || value == MOUNTING_HOLE_M3
        || value.starts_with("MountingHole_")
        || value.starts_with("WirePad_")
}

/// `C5348912_LED-…` → `C5348912`.
pub fn lcsc_from_template(value: &str) -> Option<String> {
    let rest = value.strip_prefix('C')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        Some(format!("C{digits}"))
    }
}

fn footprint_column(value: &str) -> String {
    match value.split_once('_') {
        Some((_, rest)) if !rest.is_empty() => rest.to_string(),
        _ => value.to_string(),
    }
}

pub fn build_bom_rows(footprints: &[FootprintInfo]) -> Vec<BomRow> {
    let mut by_key: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for fp in footprints {
        let Some(reference) = fp.reference.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        let value = fp.value.as_deref().unwrap_or("").trim();
        if value.is_empty() || exclude_from_bom(value) {
            continue;
        }
        let lcsc = lcsc_from_template(value).unwrap_or_default();
        let footprint = footprint_column(value);
        by_key
            .entry((value.to_string(), footprint, lcsc))
            .or_default()
            .push(reference.to_ascii_uppercase());
    }
    let mut rows: Vec<BomRow> = by_key
        .into_iter()
        .map(|((comment, footprint, lcsc), mut designators)| {
            designators.sort();
            BomRow {
                comment,
                designators,
                footprint,
                lcsc_part_number: lcsc,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.footprint
            .cmp(&b.footprint)
            .then(a.comment.cmp(&b.comment))
    });
    rows
}

pub fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn bom_to_csv(rows: &[BomRow]) -> String {
    let mut out = String::from("Comment,Designator,Footprint,LCSC Part #\n");
    for row in rows {
        out.push_str(&format!(
            "{},{},{},{}\n",
            csv_field(&row.comment),
            csv_field(&row.designators.join(", ")),
            csv_field(&row.footprint),
            csv_field(&row.lcsc_part_number)
        ));
    }
    out
}

/// KiCad `pcb export pos --format csv` → JLCPCB CPL header.
pub fn ki_pos_to_jlcpcb_cpl(text: &str) -> Result<String, String> {
    let mut out = String::from("Designator,Mid X,Mid Y,Layer,Rotation\n");
    let mut header: Option<Vec<String>> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let for_header = line.trim_start_matches('#').trim();
        if for_header.is_empty() {
            continue;
        }
        let header_cols = parse_csv_line(for_header);
        let looks_like_header = header_cols.iter().any(|c| {
            matches!(
                c.to_ascii_lowercase().as_str(),
                "ref" | "designator" | "posx" | "mid x" | "side" | "layer"
            )
        });
        if header.is_none() && looks_like_header {
            header = Some(header_cols.iter().map(|c| c.to_ascii_lowercase()).collect());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let cols = parse_csv_line(line);
        let Some(ref hdr) = header else {
            continue;
        };
        if cols.len() < hdr.len() {
            continue;
        }
        let get = |name: &str| -> Option<&str> {
            hdr.iter()
                .position(|h| h == name)
                .and_then(|i| cols.get(i))
                .map(|s| s.as_str())
        };
        let designator = get("ref")
            .or_else(|| get("designator"))
            .unwrap_or("")
            .trim();
        if designator.is_empty() {
            continue;
        }
        if exclude_from_bom(get("val").unwrap_or(""))
            || exclude_from_bom(get("package").unwrap_or(""))
        {
            continue;
        }
        let x = get("posx").or_else(|| get("mid x")).unwrap_or("0");
        let y = get("posy").or_else(|| get("mid y")).unwrap_or("0");
        let rot = get("rot").or_else(|| get("rotation")).unwrap_or("0");
        let side = get("side").or_else(|| get("layer")).unwrap_or("top");
        let layer = match side.trim().to_ascii_lowercase().as_str() {
            "bottom" | "back" | "b.cu" => "Bottom",
            _ => "Top",
        };
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_field(&designator.to_ascii_uppercase()),
            csv_field(x.trim()),
            csv_field(y.trim()),
            layer,
            csv_field(rot.trim())
        ));
    }
    if header.is_none() {
        return Err(
            "kicad-cli position file had no CSV header (Ref,Val,Package,PosX,PosY,Rot,Side)".into(),
        );
    }
    Ok(out)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gerber_layers_include_inners_on_four_layer() {
        assert_eq!(
            jlc_gerber_layers(2),
            "F.Cu,B.Cu,F.Paste,B.Paste,F.SilkS,B.SilkS,F.Mask,B.Mask,Edge.Cuts"
        );
        assert!(jlc_gerber_layers(4).contains("In1.Cu,In2.Cu"));
        assert!(jlc_gerber_layers(4).starts_with("F.Cu,In1.Cu,In2.Cu,B.Cu,"));
    }

    fn fp(reference: &str, value: &str) -> FootprintInfo {
        FootprintInfo {
            id: None,
            reference: Some(reference.into()),
            value: Some(value.into()),
            x_mm: Some(0.0),
            y_mm: Some(0.0),
            rotation_deg: Some(0.0),
            layer: "F.Cu".into(),
            pad_count: 2,
        }
    }

    #[test]
    fn lcsc_from_easyeda_template() {
        assert_eq!(
            lcsc_from_template("C5348912_LED-SMD_4P-L5.0-W4.9-BR").as_deref(),
            Some("C5348912")
        );
        assert_eq!(
            lcsc_from_template("C14663_C0603").as_deref(),
            Some("C14663")
        );
        assert_eq!(lcsc_from_template("WirePad_PTH"), None);
    }

    #[test]
    fn bom_groups_and_skips_builtins() {
        let rows = build_bom_rows(&[
            fp("C2", "C14663_C0603"),
            fp("C1", "C14663_C0603"),
            fp("H1", "MountingHole_M3_NPTH"),
            fp("W1", "WirePad_PTH"),
            fp("D1", "C5348912_LED-SMD_4P-L5.0-W4.9-BR"),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].designators,
            vec!["C1".to_string(), "C2".to_string()]
        );
        assert_eq!(rows[0].lcsc_part_number, "C14663");
        assert_eq!(rows[0].footprint, "C0603");
        assert_eq!(rows[1].lcsc_part_number, "C5348912");
        let csv = bom_to_csv(&rows);
        assert!(csv.starts_with("Comment,Designator,Footprint,LCSC Part #\n"));
        assert!(csv.contains("C1, C2"));
        assert!(!csv.contains("WirePad"));
        assert!(!csv.contains("MountingHole"));
    }

    #[test]
    fn cpl_rewrites_quoted_kicad_pos() {
        let ki = "Ref,Val,Package,PosX,PosY,Rot,Side\n\"C1\",\"C14663_C0603\",\"C14663_C0603\",10.5,-20.25,90.0,top\n";
        let cpl = ki_pos_to_jlcpcb_cpl(ki).unwrap();
        assert!(cpl.contains("C1,10.5,-20.25,Top,90.0"));
    }

    #[test]
    fn cpl_rewrites_kicad_pos_header() {
        let ki = "\
# Footprint positions
# Ref,Val,Package,PosX,PosY,Rot,Side
C1,C14663_C0603,C14663_C0603,10.5,-20.25,90.0,top
D1,C5348912_LED,C5348912_LED,0,0,0,bottom
H1,MountingHole_M3_NPTH,MountingHole_M3_NPTH,1,2,0,top
";
        let cpl = ki_pos_to_jlcpcb_cpl(ki).unwrap();
        assert!(cpl.starts_with("Designator,Mid X,Mid Y,Layer,Rotation\n"));
        assert!(cpl.contains("C1,10.5,-20.25,Top,90.0"));
        assert!(cpl.contains("D1,0,0,Bottom,0"));
        assert!(!cpl.contains("H1"));
    }

    #[test]
    fn render_args_default_and_iso() {
        let board = Path::new("/tmp/x/board.kicad_pcb");
        let out = Path::new("/tmp/x/board_render_top.png");
        let args = render_cli_args(board, out, &RenderOpts::default()).unwrap();
        assert_eq!(args[0], "pcb");
        assert_eq!(args[1], "render");
        assert!(args.contains(&"--side".to_string()));
        assert!(args.contains(&"top".to_string()));
        assert!(!args.iter().any(|a| a == "--rotate"));
        assert_eq!(args.last().unwrap(), "/tmp/x/board.kicad_pcb");

        let iso = RenderOpts {
            rotate: Some((-45.0, 0.0, 45.0)),
            perspective: true,
            ..RenderOpts::default()
        };
        let args = render_cli_args(board, out, &iso).unwrap();
        let i = args.iter().position(|a| a == "--rotate").unwrap();
        assert_eq!(args[i + 1], "-45,0,45");
        assert!(args.iter().any(|a| a == "--perspective"));
    }

    #[test]
    fn render_rejects_bad_side_and_size() {
        let board = Path::new("/nonexistent/board.kicad_pcb");
        let err = render_board_png(board, None, &RenderOpts::default()).unwrap_err();
        assert!(err.contains("not on disk"));
        let mut opts = RenderOpts::default();
        opts.side = "iso".into();
        // Side/size validation happens before the file check would pass anyway;
        // use a file that exists to reach it.
        let this_file = Path::new(file!());
        if this_file.is_file() {
            let err = render_board_png(this_file, None, &opts).unwrap_err();
            assert!(err.contains("side must be one of"));
        }
    }

    #[test]
    fn parses_kicad_drc_json() {
        let json = r#"{
            "violations": [
                {"severity": "error", "type": "clearance", "description": "Track too close"},
                {"severity": "warning", "type": "silk_over_copper", "description": "Silk"}
            ],
            "unconnected_items": [1, 2]
        }"#;
        let r = parse_drc_json(json).unwrap();
        assert!(!r.ok);
        assert_eq!(r.error_count, 1);
        assert_eq!(r.warning_count, 1);
        assert_eq!(r.unconnected_count, 2);
        assert_eq!(r.violations[0].kind, "clearance");
    }
}
