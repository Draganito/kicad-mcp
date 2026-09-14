//! KiCad stroke-font (newstroke) layout for silk DFM.
//!
//! Glyphs are BASIC LATIN from KiCad `newstroke_font.cpp` (GPL-2.0-or-later),
//! the same strokes the PCB editor plots. Unknown characters use `?`.
//! Coordinates: Hershey `R`-offset, `STROKE_FONT_SCALE = 1/21`, baseline
//! `FONT_OFFSET = -8`. A `" R"` pair is pen-up. Layout is left-to-right,
//! then the bounding box is centred on the BoardText position (HA/VA
//! center). Back silk mirrors in X about that centre.

include!("silk_newstroke_glyphs.rs");

const SCALE: f64 = 1.0 / 21.0;
const FONT_OFFSET: i32 = -8;
const R: i32 = b'R' as i32;

#[derive(Clone, Debug)]
pub struct CharInk {
    pub ch: char,
    /// Inclusive UTF-8 byte range into the original string.
    pub byte_start: usize,
    pub byte_end: usize,
    pub segments: Vec<[[f64; 2]; 2]>,
}

fn glyph_index(ch: char) -> usize {
    let u = ch as u32;
    if (0x20..=0x7e).contains(&u) {
        (u - 0x20) as usize
    } else {
        (b'?' - b' ') as usize
    }
}

fn decode(raw: &str) -> (f64, Vec<Vec<(f64, f64)>>) {
    let b = raw.as_bytes();
    if b.len() < 2 {
        return (0.0, Vec::new());
    }
    let start_x = (b[0] as i32 - R) as f64 * SCALE;
    let end_x = (b[1] as i32 - R) as f64 * SCALE;
    let width = end_x - start_x;
    let mut strokes: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut i = 2;
    while i + 1 < b.len() {
        let c0 = b[i];
        let c1 = b[i + 1];
        i += 2;
        if c0 == b' ' && c1 == b'R' {
            if !cur.is_empty() {
                strokes.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let x = (c0 as i32 - R) as f64 * SCALE - start_x;
        let y = (c1 as i32 - R + FONT_OFFSET) as f64 * SCALE;
        cur.push((x, y));
    }
    if !cur.is_empty() {
        strokes.push(cur);
    }
    (width, strokes)
}

fn glyph(ch: char) -> (f64, Vec<Vec<(f64, f64)>>) {
    decode(GLYPHS[glyph_index(ch)])
}

/// Stroke segments of `body`, centred at `(cx, cy)`, optional X-mirror and rotation.
pub fn layout_chars(
    body: &str,
    size: f64,
    cx: f64,
    cy: f64,
    mirrored: bool,
    rotation_deg: f64,
) -> Vec<CharInk> {
    if body.is_empty() || size <= 0.0 {
        return Vec::new();
    }
    let mut chars = Vec::new();
    let mut cursor = 0.0_f64;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for (byte_start, ch) in body.char_indices() {
        let byte_end = byte_start + ch.len_utf8();
        let (width, strokes) = glyph(ch);
        let mut segments = Vec::new();
        if ch != ' ' {
            for stroke in &strokes {
                for w in stroke.windows(2) {
                    let a = [cursor + w[0].0 * size, w[0].1 * size];
                    let b = [cursor + w[1].0 * size, w[1].1 * size];
                    segments.push([a, b]);
                    pts.push((a[0], a[1]));
                    pts.push((b[0], b[1]));
                }
            }
        }
        chars.push(CharInk {
            ch,
            byte_start,
            byte_end,
            segments,
        });
        cursor += width * size;
    }
    if pts.is_empty() {
        // Spaces only: nothing to punch.
        return chars;
    }
    let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let ox = cx - (min_x + max_x) / 2.0;
    let oy = cy - (min_y + max_y) / 2.0;
    let (s, c) = rotation_deg.to_radians().sin_cos();
    for ch in &mut chars {
        for seg in &mut ch.segments {
            for p in seg {
                let mut x = p[0] + ox;
                let mut y = p[1] + oy;
                if mirrored {
                    x = 2.0 * cx - x;
                }
                if rotation_deg.abs() > 0.01 {
                    let dx = x - cx;
                    let dy = y - cy;
                    x = cx + dx * c - dy * s;
                    y = cy + dx * s + dy * c;
                }
                p[0] = x;
                p[1] = y;
            }
        }
    }
    chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_has_width_no_strokes() {
        let (w, s) = glyph(' ');
        assert!(w > 0.5, "space width {w}");
        assert!(s.is_empty());
    }

    #[test]
    fn pee_has_a_descender() {
        let (_w, strokes) = glyph('p');
        let min_y = strokes
            .iter()
            .flat_map(|s| s.iter().map(|p| p.1))
            .fold(f64::INFINITY, f64::min);
        assert!(min_y < -0.05, "p descender y={min_y}");
    }

    #[test]
    fn centered_layout_straddles_origin() {
        let chars = layout_chars("p", 1.0, 0.0, 0.0, false, 0.0);
        let pts: Vec<_> = chars
            .iter()
            .flat_map(|c| c.segments.iter().flat_map(|s| [s[0], s[1]]))
            .collect();
        let min_x = pts.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
        let max_x = pts.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let min_y = pts.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        let max_y = pts.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
        assert!(((min_x + max_x) / 2.0).abs() < 0.05, "cx {:?}", (min_x, max_x));
        assert!(((min_y + max_y) / 2.0).abs() < 0.05, "cy {:?}", (min_y, max_y));
    }

    #[test]
    fn mirror_flips_x() {
        let a = layout_chars("P", 1.0, 10.0, 5.0, false, 0.0);
        let b = layout_chars("P", 1.0, 10.0, 5.0, true, 0.0);
        let ax: Vec<_> = a[0]
            .segments
            .iter()
            .map(|s| ((s[0][0] * 100.0).round(), (s[0][1] * 100.0).round()))
            .collect();
        let bx: Vec<_> = b[0]
            .segments
            .iter()
            .map(|s| ((s[0][0] * 100.0).round(), (s[0][1] * 100.0).round()))
            .collect();
        assert_ne!(ax, bx);
        let mean = |cs: &[CharInk]| {
            let n = cs[0].segments.len() as f64;
            cs[0].segments.iter().map(|s| s[0][0]).sum::<f64>() / n
        };
        assert!((mean(&a) + mean(&b) - 20.0).abs() < 0.2);
    }
}
