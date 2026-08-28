//! Mechanical templates that are not LCSC C-numbers.
//! Written into the open project's `jlcpcb_parts.pretty`.
//!
//! Nothing here is a fixed footprint: every template is generated from
//! parameters. The two default names (`WirePad_PTH`, `MountingHole_M3_NPTH`)
//! are just the default parameter sets; `make_wire_pad` / `make_mounting_hole`
//! write any other size on demand.

use std::path::Path;

pub const WIRE_PAD: &str = "WirePad_PTH";
pub const MOUNTING_HOLE_M3: &str = "MountingHole_M3_NPTH";

/// Default wire pad: 2.5 mm copper / 1.5 mm drill.
pub const WIRE_PAD_DEFAULT_PAD_MM: f64 = 2.5;
pub const WIRE_PAD_DEFAULT_DRILL_MM: f64 = 1.5;
/// Default mounting hole: M3 clearance = 3.2 mm NPTH.
pub const MOUNTING_HOLE_DEFAULT_MM: f64 = 3.2;
/// Extra diameter beyond the hole so a typical machine-screw head
/// (M3 pan/cross ≈ 5.5–6 mm) plus tightening pressure sits on bare
/// laminate, not on a copper pour. 3.2 + 4.3 = 7.5 mm — large enough
/// for the head, small enough to keep 0.5 mm copper-to-edge on a
/// hole that sits ~4.4 mm from Edge.Cuts.
const MOUNTING_HOLE_HEAD_CLEAR_MM: f64 = 4.3;
const KEEPOUT_SEGMENTS: usize = 32;

/// JLCPCB capability limits (standard PCB service).
const MIN_DRILL_MM: f64 = 0.3;
const MAX_DRILL_MM: f64 = 6.3;
/// Minimum annular ring per side for a PTH pad.
const MIN_ANNULAR_MM: f64 = 0.25;

/// Ensure the default wire-pad and M3 NPTH `.kicad_mod` files exist next to
/// LCSC parts. Custom sizes are written by [`make_wire_pad`] /
/// [`make_mounting_hole`].
pub fn ensure_builtin_footprints(pretty_dir: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(pretty_dir).map_err(|e| e.to_string())?;
    let mut written = Vec::new();
    for (name, body) in [
        (
            WIRE_PAD,
            wire_pad_mod(WIRE_PAD, WIRE_PAD_DEFAULT_PAD_MM, WIRE_PAD_DEFAULT_DRILL_MM),
        ),
        (
            MOUNTING_HOLE_M3,
            mounting_hole_mod(MOUNTING_HOLE_M3, MOUNTING_HOLE_DEFAULT_MM),
        ),
    ] {
        let path = pretty_dir.join(format!("{name}.kicad_mod"));
        if !path.exists() {
            std::fs::write(&path, body).map_err(|e| e.to_string())?;
            written.push(name.to_string());
        }
    }
    Ok(written)
}

/// Write a PTH wire pad with the given copper and drill diameter.
/// Returns the template name (`WirePad_PTH` for the default size, otherwise
/// e.g. `WirePad_PTH_3.2_2.0`). Overwrites an existing file of the same name
/// so a size can be regenerated at any time.
pub fn make_wire_pad(pretty_dir: &Path, pad_mm: f64, drill_mm: f64) -> Result<String, String> {
    if !(MIN_DRILL_MM..=MAX_DRILL_MM).contains(&drill_mm) {
        return Err(format!(
            "drill_mm {drill_mm} outside JLCPCB range {MIN_DRILL_MM}–{MAX_DRILL_MM} mm"
        ));
    }
    let min_pad = drill_mm + 2.0 * MIN_ANNULAR_MM;
    if pad_mm < min_pad {
        return Err(format!(
            "pad_mm {pad_mm} too small — needs ≥ {min_pad} mm ({MIN_ANNULAR_MM} mm annular ring around a {drill_mm} mm drill)"
        ));
    }
    let name = if (pad_mm - WIRE_PAD_DEFAULT_PAD_MM).abs() < 1e-9
        && (drill_mm - WIRE_PAD_DEFAULT_DRILL_MM).abs() < 1e-9
    {
        WIRE_PAD.to_string()
    } else {
        format!("{WIRE_PAD}_{}_{}", fmt_mm(pad_mm), fmt_mm(drill_mm))
    };
    std::fs::create_dir_all(pretty_dir).map_err(|e| e.to_string())?;
    let path = pretty_dir.join(format!("{name}.kicad_mod"));
    std::fs::write(&path, wire_pad_mod(&name, pad_mm, drill_mm)).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Write an NPTH mounting hole with the given hole diameter.
/// Returns the template name (`MountingHole_M3_NPTH` for 3.2 mm, otherwise
/// e.g. `MountingHole_4.5_NPTH`). Overwrites an existing file of the same
/// name.
pub fn make_mounting_hole(pretty_dir: &Path, hole_mm: f64) -> Result<String, String> {
    if !(MIN_DRILL_MM..=MAX_DRILL_MM).contains(&hole_mm) {
        return Err(format!(
            "hole_mm {hole_mm} outside JLCPCB range {MIN_DRILL_MM}–{MAX_DRILL_MM} mm"
        ));
    }
    let name = if (hole_mm - MOUNTING_HOLE_DEFAULT_MM).abs() < 1e-9 {
        MOUNTING_HOLE_M3.to_string()
    } else {
        format!("MountingHole_{}_NPTH", fmt_mm(hole_mm))
    };
    std::fs::create_dir_all(pretty_dir).map_err(|e| e.to_string())?;
    let path = pretty_dir.join(format!("{name}.kicad_mod"));
    std::fs::write(&path, mounting_hole_mod(&name, hole_mm)).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Millimetres for template names: `2.5` → "2.5", `2.0` → "2".
fn fmt_mm(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// PTH wire pad footprint: round copper pad `pad_mm` with `drill_mm` hole.
/// Courtyard is pad + 1 mm, text sits just outside the courtyard.
fn wire_pad_mod(name: &str, pad_mm: f64, drill_mm: f64) -> String {
    let cy = (pad_mm + 1.0) / 2.0;
    let text_y = cy + 0.65;
    format!(
        r#"(footprint "{name}"
	(version 20240108)
	(generator "kicad-mcp")
	(layer "F.Cu")
	(descr "PTH wire pad {pad}mm copper / {drill}mm drill")
	(tags "wire pad PTH")
	(attr through_hole exclude_from_bom)
	(fp_text reference "REF**" (at 0 {text_y} unlocked) (layer "F.SilkS")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_text value "{name}" (at 0 -{text_y} unlocked) (layer "F.Fab")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_line (start -{cy} -{cy}) (end {cy} -{cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start {cy} -{cy}) (end {cy} {cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start {cy} {cy}) (end -{cy} {cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start -{cy} {cy}) (end -{cy} -{cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(pad "1" thru_hole circle (at 0 0) (size {pad} {pad}) (drill {drill}) (layers "*.Cu" "*.Mask"))
)
"#,
        name = name,
        pad = fmt_coord(pad_mm),
        drill = fmt_coord(drill_mm),
        cy = fmt_coord(cy),
        text_y = fmt_coord(text_y),
    )
}

/// Copper-free diameter around a mounting hole (hole + screw-head margin).
fn mounting_hole_keepout_mm(hole_mm: f64) -> f64 {
    hole_mm + MOUNTING_HOLE_HEAD_CLEAR_MM
}

fn keepout_circle_pts(radius_mm: f64) -> String {
    (0..KEEPOUT_SEGMENTS)
        .map(|i| {
            let a = (i as f64) * std::f64::consts::TAU / (KEEPOUT_SEGMENTS as f64);
            format!(
                "\t\t\t\t(xy {} {})",
                fmt_coord(radius_mm * a.cos()),
                fmt_coord(radius_mm * a.sin())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// NPTH mounting hole: `hole_mm` drill, oversized copper-free pad + keepout
/// so a typical screw head and tightening pressure do not sit on a pour.
/// Courtyard matches the keepout diameter.
fn mounting_hole_mod(name: &str, hole_mm: f64) -> String {
    let keepout = mounting_hole_keepout_mm(hole_mm);
    let cy = keepout / 2.0;
    let text_y = cy + 0.3;
    format!(
        r#"(footprint "{name}"
	(version 20240108)
	(generator "kicad-mcp")
	(layer "F.Cu")
	(descr "Mounting hole, {hole}mm NPTH, {keepout}mm copper keepout for screw head")
	(tags "mounting hole NPTH keepout")
	(attr exclude_from_bom exclude_from_pos_files)
	(fp_text reference "REF**" (at 0 {text_y} unlocked) (layer "F.SilkS")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_text value "{name}" (at 0 -{text_y} unlocked) (layer "F.Fab")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_line (start -{cy} -{cy}) (end {cy} -{cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start {cy} -{cy}) (end {cy} {cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start {cy} {cy}) (end -{cy} {cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start -{cy} {cy}) (end -{cy} -{cy}) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(pad "" npth circle (at 0 0) (size {keepout} {keepout}) (drill {hole}) (layers "*.Cu" "*.Mask"))
	(zone
		(net 0)
		(net_name "")
		(layers "*.Cu")
		(hatch edge 0.5)
		(connect_pads (clearance 0))
		(min_thickness 0.25)
		(filled_areas_thickness no)
		(keepout (tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
		(fill (thermal_gap 0.5) (thermal_bridge_width 0.5))
		(polygon
			(pts
{pts}
			)
		)
	)
)
"#,
        name = name,
        hole = fmt_coord(hole_mm),
        keepout = fmt_coord(keepout),
        cy = fmt_coord(cy),
        text_y = fmt_coord(text_y),
        pts = keepout_circle_pts(cy),
    )
}

/// Millimetres inside the s-expression: fixed precision, no trailing zeros.
fn fmt_coord(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place;

    #[test]
    fn default_wire_pad_parses() {
        let body = wire_pad_mod(WIRE_PAD, WIRE_PAD_DEFAULT_PAD_MM, WIRE_PAD_DEFAULT_DRILL_MM);
        let pads = place::parse_kicad_mod_pads(&body).unwrap();
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0].number, "1");
        assert_eq!(pads[0].kind, place::ModPadKind::ThruHole);
        assert!((pads[0].width_mm - 2.5).abs() < 1e-6);
        assert_eq!(pads[0].drill_mm, Some(1.5));
        let cy = place::parse_kicad_mod_courtyard(&body).unwrap();
        assert!((cy.max_x - cy.min_x - 3.5).abs() < 1e-6);
    }

    #[test]
    fn default_m3_npth_parses() {
        let body = mounting_hole_mod(MOUNTING_HOLE_M3, MOUNTING_HOLE_DEFAULT_MM);
        let pads = place::parse_kicad_mod_pads(&body).unwrap();
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0].kind, place::ModPadKind::Npth);
        assert_eq!(pads[0].drill_mm, Some(3.2));
        assert!((pads[0].width_mm - 7.5).abs() < 1e-6);
        let cy = place::parse_kicad_mod_courtyard(&body).unwrap();
        assert!((cy.max_x - cy.min_x - 7.5).abs() < 1e-6);
        assert!(body.contains("copperpour not_allowed"));
    }

    #[test]
    fn custom_wire_pad_parses() {
        let body = wire_pad_mod("WirePad_PTH_3.2_2", 3.2, 2.0);
        let pads = place::parse_kicad_mod_pads(&body).unwrap();
        assert_eq!(pads.len(), 1);
        assert!((pads[0].width_mm - 3.2).abs() < 1e-6);
        assert_eq!(pads[0].drill_mm, Some(2.0));
        let cy = place::parse_kicad_mod_courtyard(&body).unwrap();
        assert!((cy.max_x - cy.min_x - 4.2).abs() < 1e-6);
    }

    #[test]
    fn writes_once() {
        let dir = std::env::temp_dir().join(format!("kicad-mcp-builtins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = ensure_builtin_footprints(&dir).unwrap();
        assert_eq!(first.len(), 2);
        let second = ensure_builtin_footprints(&dir).unwrap();
        assert!(second.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn make_wire_pad_names_and_limits() {
        let dir = std::env::temp_dir().join(format!("kicad-mcp-wirepad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Default size keeps the canonical name.
        let name = make_wire_pad(&dir, 2.5, 1.5).unwrap();
        assert_eq!(name, "WirePad_PTH");

        // Custom size gets a parametric name and parses.
        let name = make_wire_pad(&dir, 3.2, 2.0).unwrap();
        assert_eq!(name, "WirePad_PTH_3.2_2");
        let body = std::fs::read_to_string(dir.join("WirePad_PTH_3.2_2.kicad_mod")).unwrap();
        let pads = place::parse_kicad_mod_pads(&body).unwrap();
        assert_eq!(pads[0].drill_mm, Some(2.0));

        // Annular ring too small.
        assert!(make_wire_pad(&dir, 2.1, 2.0).is_err());
        // Drill outside fab limits.
        assert!(make_wire_pad(&dir, 1.0, 0.1).is_err());
        assert!(make_wire_pad(&dir, 8.0, 7.0).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn make_mounting_hole_names_and_limits() {
        let dir = std::env::temp_dir().join(format!("kicad-mcp-mnthole-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // M3 clearance keeps the canonical name.
        let name = make_mounting_hole(&dir, 3.2).unwrap();
        assert_eq!(name, "MountingHole_M3_NPTH");

        // Custom size gets a parametric name and parses.
        let name = make_mounting_hole(&dir, 4.5).unwrap();
        assert_eq!(name, "MountingHole_4.5_NPTH");
        let body = std::fs::read_to_string(dir.join("MountingHole_4.5_NPTH.kicad_mod")).unwrap();
        let pads = place::parse_kicad_mod_pads(&body).unwrap();
        assert_eq!(pads[0].kind, place::ModPadKind::Npth);
        assert_eq!(pads[0].drill_mm, Some(4.5));
        assert!((pads[0].width_mm - 8.8).abs() < 1e-6);
        let cy = place::parse_kicad_mod_courtyard(&body).unwrap();
        assert!((cy.max_x - cy.min_x - 8.8).abs() < 1e-6);

        assert!(make_mounting_hole(&dir, 0.2).is_err());
        assert!(make_mounting_hole(&dir, 7.0).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fmt_mm_trims() {
        assert_eq!(fmt_mm(2.0), "2");
        assert_eq!(fmt_mm(2.5), "2.5");
        assert_eq!(fmt_mm(2.05), "2.05");
    }
}
