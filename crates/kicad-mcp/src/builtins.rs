//! Mechanical templates that are not LCSC C-numbers.
//! Written into the open project's `jlcpcb_parts.pretty`.

use std::path::Path;

pub const WIRE_PAD: &str = "WirePad_PTH";
pub const MOUNTING_HOLE_M3: &str = "MountingHole_M3_NPTH";

/// Ensure wire-pad and M3 NPTH `.kicad_mod` files exist next to LCSC parts.
pub fn ensure_builtin_footprints(pretty_dir: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(pretty_dir).map_err(|e| e.to_string())?;
    let mut written = Vec::new();
    for (name, body) in [
        (WIRE_PAD, WIRE_PAD_MOD),
        (MOUNTING_HOLE_M3, MOUNTING_HOLE_M3_MOD),
    ] {
        let path = pretty_dir.join(format!("{name}.kicad_mod"));
        if !path.exists() {
            std::fs::write(&path, body).map_err(|e| e.to_string())?;
            written.push(name.to_string());
        }
    }
    Ok(written)
}

/// PTH wire pad: 2.5 mm copper / 1.5 mm drill.
const WIRE_PAD_MOD: &str = r#"(footprint "WirePad_PTH"
	(version 20240108)
	(generator "kicad-mcp")
	(layer "F.Cu")
	(descr "PTH wire pad 2.5mm copper / 1.5mm drill")
	(tags "wire pad PTH")
	(attr through_hole exclude_from_bom)
	(fp_text reference "REF**" (at 0 2.4 unlocked) (layer "F.SilkS")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_text value "WirePad_PTH" (at 0 -2.4 unlocked) (layer "F.Fab")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_line (start -1.75 -1.75) (end 1.75 -1.75) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start 1.75 -1.75) (end 1.75 1.75) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start 1.75 1.75) (end -1.75 1.75) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start -1.75 1.75) (end -1.75 -1.75) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(pad "1" thru_hole circle (at 0 0) (size 2.5 2.5) (drill 1.5) (layers "*.Cu" "*.Mask"))
)
"#;

/// KiCad MountingHole_3.2mm_M3 convention: 3.2 mm NPTH, 6.4 mm courtyard.
const MOUNTING_HOLE_M3_MOD: &str = r#"(footprint "MountingHole_M3_NPTH"
	(version 20240108)
	(generator "kicad-mcp")
	(layer "F.Cu")
	(descr "M3 mounting hole, 3.2mm NPTH, no copper")
	(tags "mounting hole M3 NPTH")
	(attr exclude_from_bom exclude_from_pos_files)
	(fp_text reference "REF**" (at 0 3.5 unlocked) (layer "F.SilkS")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_text value "MountingHole_M3_NPTH" (at 0 -3.5 unlocked) (layer "F.Fab")
		(effects (font (size 1 1) (thickness 0.15)))
	)
	(fp_line (start -3.2 -3.2) (end 3.2 -3.2) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start 3.2 -3.2) (end 3.2 3.2) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start 3.2 3.2) (end -3.2 3.2) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(fp_line (start -3.2 3.2) (end -3.2 -3.2) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
	(pad "" npth circle (at 0 0) (size 3.2 3.2) (drill 3.2) (layers "F&B.Cu" "*.Mask"))
)
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place;

    #[test]
    fn wire_pad_parses() {
        let pads = place::parse_kicad_mod_pads(WIRE_PAD_MOD).unwrap();
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0].number, "1");
        assert_eq!(pads[0].kind, place::ModPadKind::ThruHole);
        assert!((pads[0].width_mm - 2.5).abs() < 1e-6);
        assert_eq!(pads[0].drill_mm, Some(1.5));
        let cy = place::parse_kicad_mod_courtyard(WIRE_PAD_MOD).unwrap();
        assert!((cy.max_x - cy.min_x - 3.5).abs() < 1e-6);
    }

    #[test]
    fn m3_npth_parses() {
        let pads = place::parse_kicad_mod_pads(MOUNTING_HOLE_M3_MOD).unwrap();
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0].kind, place::ModPadKind::Npth);
        assert_eq!(pads[0].drill_mm, Some(3.2));
        let cy = place::parse_kicad_mod_courtyard(MOUNTING_HOLE_M3_MOD).unwrap();
        assert!((cy.max_x - cy.min_x - 6.4).abs() < 1e-6);
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
}
