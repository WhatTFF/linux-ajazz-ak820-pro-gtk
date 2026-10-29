//! HSV / colour helpers, ported byte-for-byte from `ak820-screen` `gen.js`.

/// HSV -> [r, g, b] 0..255 (h in degrees).
pub fn hsv(h: f64, s: f64, v: f64) -> [f64; 3] {
    let h = ((h % 360.0) + 360.0) % 360.0 / 60.0;
    let i = h.floor() as i32;
    let f = h - i as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let u = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i % 6 {
        0 => (v, u, p),
        1 => (q, v, p),
        2 => (p, v, u),
        3 => (p, q, v),
        4 => (u, p, v),
        _ => (v, p, q),
    };
    [r * 255.0, g * 255.0, b * 255.0]
}

pub const fn tri(x: f64) -> f64 {
    let frac = x - x.floor();
    1.0 - (2.0 * frac - 1.0).abs()
}

pub const fn clamp01(x: f64) -> f64 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}
