//! Procedural app/tray icon artwork.
//!
//! Shared between `build.rs` (which writes the `.ico` embedded in the exe) and the
//! application (which renders the window and tray icons at runtime), so there is a
//! single source of truth and no binary assets in the repository.

/// Renders the app icon as straight (non-premultiplied) RGBA8 pixels, row-major.
pub fn render_app_icon(size: u32, muted: bool) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];

    // Gradient endpoints (top-left -> bottom-right).
    let (c0, c1) =
        if muted { ([0x6b, 0x72, 0x80], [0x37, 0x41, 0x51]) } else { ([0x8b, 0x5c, 0xf6], [0x06, 0xb6, 0xd4]) };
    let corner = 0.24 * s;
    let bar_w = 0.12 * s;
    let bar_bottom = 0.76 * s;
    let bars = [(0.31, 0.47), (0.50, 0.25), (0.69, 0.38)];
    let bar_alpha = if muted { 0.55 } else { 0.96 };

    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4]; // premultiplied accumulation
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = px as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SAMPLES as f32;

                    // Rounded-square background.
                    let d = sd_round_box(x - s / 2.0, y - s / 2.0, s / 2.0, s / 2.0, corner);
                    if d > 0.0 {
                        continue;
                    }
                    let t = ((x + y) / (2.0 * s)).clamp(0.0, 1.0);
                    let mut rgb = [
                        lerp(c0[0] as f32, c1[0] as f32, t) / 255.0,
                        lerp(c0[1] as f32, c1[1] as f32, t) / 255.0,
                        lerp(c0[2] as f32, c1[2] as f32, t) / 255.0,
                    ];

                    // Three fader bars.
                    for (cx, top) in bars {
                        let cx = cx * s;
                        let top = top * s;
                        let half_h = (bar_bottom - top) / 2.0;
                        let db = sd_round_box(x - cx, y - (top + half_h), bar_w / 2.0, half_h, bar_w / 2.0);
                        if db <= 0.0 {
                            rgb = mix(rgb, [1.0, 1.0, 1.0], bar_alpha);
                        }
                    }

                    // Mute slash.
                    if muted {
                        let dl = sd_segment(x, y, 0.22 * s, 0.22 * s, 0.78 * s, 0.78 * s);
                        if dl <= 0.055 * s {
                            rgb = [1.0, 1.0, 1.0];
                        }
                    }

                    acc[0] += rgb[0];
                    acc[1] += rgb[1];
                    acc[2] += rgb[2];
                    acc[3] += 1.0;
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let a = acc[3] / n;
            let i = ((py * size + px) * 4) as usize;
            if a > 0.0 {
                out[i] = to_u8(acc[0] / acc[3]);
                out[i + 1] = to_u8(acc[1] / acc[3]);
                out[i + 2] = to_u8(acc[2] / acc[3]);
                out[i + 3] = to_u8(a);
            }
        }
    }
    out
}

fn sd_round_box(px: f32, py: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = px.abs() - hx + r;
    let qy = py.abs() - hy + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn sd_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (pax, pay) = (px - ax, py - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Encodes square RGBA images as a multi-size `.ico` (32-bit BMP entries).
pub fn encode_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    for (size, rgba) in images {
        let size = *size;
        let mut blob = Vec::new();
        let mask_row = size.div_ceil(32) * 4;
        let image_bytes = size * size * 4 + mask_row * size;
        // BITMAPINFOHEADER
        blob.extend_from_slice(&40u32.to_le_bytes());
        blob.extend_from_slice(&(size as i32).to_le_bytes());
        blob.extend_from_slice(&((size * 2) as i32).to_le_bytes());
        blob.extend_from_slice(&1u16.to_le_bytes());
        blob.extend_from_slice(&32u16.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(&image_bytes.to_le_bytes());
        blob.extend_from_slice(&[0u8; 16]);
        // Pixels, bottom-up BGRA.
        for y in (0..size).rev() {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                blob.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        // AND mask (all zero: alpha channel is authoritative).
        blob.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
        entries.push((size, blob.len() as u32));
        blobs.push(blob);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, len) in &entries {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += len;
    }
    for blob in blobs {
        out.extend_from_slice(&blob);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_size_and_transparent_corners() {
        let px = render_app_icon(32, false);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0, "top-left corner should be transparent");
        let center = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(px[center + 3], 255, "center should be opaque");
    }

    #[test]
    fn muted_icon_differs() {
        assert_ne!(render_app_icon(24, false), render_app_icon(24, true));
    }

    #[test]
    fn ico_header_is_well_formed() {
        let ico = encode_ico(&[(16, render_app_icon(16, false)), (256, render_app_icon(256, false))]);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[6 + 16], 0, "256px entries are encoded as 0");
        let first_offset = u32::from_le_bytes(ico[18..22].try_into().unwrap());
        assert_eq!(first_offset, 6 + 32);
    }
}
