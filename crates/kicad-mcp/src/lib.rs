//! Library surface for the kicad-mcp binary and IPC probes.
//!
//! Board build path (KiCad 10 PCB editor, IPC `CreateItems`):
//! 1. `outline` — Edge.Cuts rectangle, default origin = centre of the A4 sheet.
//! 2. `place` — LCSC `.kicad_mod` pads baked to board millimetres (KiCad does
//!    not parent-transform nested pads; the instance position is only the
//!    anchor `get_footprints` returns).
//! 3. Courtyard overlap is rejected in `place_footprint` / `place_parts` / `place_matrix`.
//! 4. `connect_pins` sets `Pad.net` (UpdateItems). Tracks/vias/zones are CreateItems.
//! 5. Copper: tracks, vias, stitch vias, then zones.

pub mod builtins;
pub mod copper;
pub mod fab;
pub mod kicad;
pub mod mcp;
pub mod nets;
pub mod outline;
pub mod place;
pub mod proto_wire;
pub mod stitch;
