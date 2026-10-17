//! Procedural tray icon: a fan rotor in a ring (same design as `assets/OpenLinkHub.ico`).

/// RGBA pixels for a `size`×`size` icon.
pub fn rgba(size: u32) -> (Vec<u8>, u32) {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let c = size as f32 / 2.0;
    let r_outer = c - 0.5;
    let r_ring = r_outer * 0.80;
    let r_hub = r_outer * 0.22;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let a = dy.atan2(dx);
            // Five swept blades.
            let blade = ((a * 5.0 + d / r_outer * 2.4).sin() + 1.0) / 2.0;
            let (rgb, alpha) = if d > r_outer {
                ([0, 0, 0], 0.0)
            } else if d > r_ring || d < r_hub {
                ([255, 196, 0], 1.0) // ring and hub
            } else if blade > 0.55 {
                ([250, 250, 250], 1.0)
            } else {
                ([24, 24, 28], 1.0)
            };
            // Anti-alias the outer edge.
            let alpha = alpha * (r_outer + 0.5 - d).clamp(0.0, 1.0);
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 3].copy_from_slice(&rgb);
            px[i + 3] = (alpha * 255.0) as u8;
        }
    }
    (px, size)
}
