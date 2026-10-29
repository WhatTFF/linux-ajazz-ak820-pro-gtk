//! Procedural generators ported from `ak820-screen` `gen.js`.
//! Renders directly into `image::RgbaImage` buffers (128×128).

use ak820_proto::{H, W};
use image::RgbaImage;

#[derive(Clone)]
pub struct SourceFrame {
    pub image: RgbaImage,
    pub delay_ms: u16,
}

pub const TAU: f64 = std::f64::consts::PI * 2.0;

pub fn hsv(h: f64, s: f64, v: f64) -> [u8; 3] {
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
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

pub fn frac(x: f64) -> f64 {
    x - x.floor()
}
pub fn tri(x: f64) -> f64 {
    1.0 - (2.0 * frac(x) - 1.0).abs()
}
pub fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn per_pixel<F>(mut f: F) -> RgbaImage
where
    F: FnMut(u32, u32) -> image::Rgba<u8>,
{
    let mut img = RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([0, 0, 0, 255]));
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            img.put_pixel(x, y, f(x, y));
        }
    }
    img
}

// ---------- palettes ----------

type PalFn = fn(f64, f64) -> [u8; 3];

const PALETTES: [(&str, PalFn); 7] = [
    ("rainbow", |v, t| hsv(v * 360.0 + t * 360.0, 1.0, 1.0)),
    ("fire", |v, _| {
        [
            (255.0 * clamp01(v * 3.0)) as u8,
            (255.0 * clamp01(v * 3.0 - 1.0)) as u8,
            (255.0 * clamp01(v * 3.0 - 2.0)) as u8,
        ]
    }),
    ("ocean", |v, _| hsv(180.0 + v * 60.0, 0.9, 0.25 + v * 0.75)),
    ("neon", |v, t| {
        hsv(
            280.0 + (v * TAU).sin() * 60.0 + t * 360.0,
            1.0,
            0.4 + 0.6 * v,
        )
    }),
    ("ice", |v, _| {
        hsv(200.0 + v * 30.0, 1.0 - v * 0.8, 0.2 + v * 0.8)
    }),
    ("toxic", |v, _| hsv(90.0 + v * 40.0, 1.0, 0.15 + v * 0.85)),
    ("mono", |v, _| {
        let g = (v * 255.0) as u8;
        [g, g, g]
    }),
];

fn palette(name: &str) -> PalFn {
    PALETTES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .unwrap_or(PALETTES[0].1)
}

// ---------- enums ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    H,
    V,
    D,
    R,
    S,
    C,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Soft {
    Full,
    Pastel,
    Dark,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Rainbow,
    Neon,
    Fire,
    Ocean,
    Toxic,
    Mono,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Green,
    Blue,
    Red,
    Classic,
    Rainbow,
    White,
    Gold,
    Mono,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Trail {
    On,
    Off,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flame {
    Classic,
    Ice,
    Toxic,
    Rainbow,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sky {
    Night,
    Dusk,
    Black,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Love,
    Rainbow,
    Pastel,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

#[derive(Clone)]
pub struct GenParams {
    pub pattern: Pattern,
    pub bands: u8,
    pub soft: Soft,
    pub palette: Palette,
    pub color: Color,
    pub trail: Trail,
    pub flame: Flame,
    pub sky: Sky,
    pub mood: Mood,
    pub size: Size,
    pub text: String,
}

impl Default for GenParams {
    fn default() -> Self {
        Self {
            pattern: Pattern::H,
            bands: 1,
            soft: Soft::Full,
            palette: Palette::Rainbow,
            color: Color::Green,
            trail: Trail::Off,
            flame: Flame::Classic,
            sky: Sky::Night,
            mood: Mood::Love,
            size: Size::Medium,
            text: "AK820".into(),
        }
    }
}

#[derive(Clone)]
pub struct Generator {
    pub id: &'static str,
    pub name: &'static str,
    pub build_fn: fn(&GenParams, u16) -> Vec<SourceFrame>,
    #[allow(dead_code)]
    pub param_labels: &'static [&'static str],
}

pub fn generators() -> Vec<Generator> {
    vec![
        Generator {
            id: "rainbow",
            name: "Rainbow",
            build_fn: rainbow,
            param_labels: &["Pattern", "Bands", "Look"],
        },
        Generator {
            id: "plasma",
            name: "Plasma",
            build_fn: plasma,
            param_labels: &["Colours"],
        },
        Generator {
            id: "matrix",
            name: "Matrix",
            build_fn: matrix,
            param_labels: &["Colour"],
        },
        Generator {
            id: "stars",
            name: "Warp",
            build_fn: stars,
            param_labels: &["Stars", "Trails"],
        },
        Generator {
            id: "fire",
            name: "Fire",
            build_fn: fire,
            param_labels: &["Flame"],
        },
        Generator {
            id: "snow",
            name: "Snow",
            build_fn: snow,
            param_labels: &["Sky"],
        },
        Generator {
            id: "hearts",
            name: "Hearts",
            build_fn: hearts,
            param_labels: &["Colours"],
        },
        Generator {
            id: "bounce",
            name: "Bounce",
            build_fn: bounce,
            param_labels: &["Text", "Size"],
        },
        Generator {
            id: "ripples",
            name: "Ripples",
            build_fn: ripples,
            param_labels: &["Colours"],
        },
        Generator {
            id: "eq",
            name: "Equalizer",
            build_fn: eq,
            param_labels: &["Colours"],
        },
    ]
}

pub fn default_params(id: &str) -> GenParams {
    match id {
        "rainbow" => GenParams {
            pattern: Pattern::H,
            bands: 1,
            soft: Soft::Full,
            ..GenParams::default()
        },
        "plasma" => GenParams {
            palette: Palette::Rainbow,
            ..GenParams::default()
        },
        "matrix" => GenParams {
            color: Color::Green,
            ..GenParams::default()
        },
        "stars" => GenParams {
            color: Color::White,
            trail: Trail::Off,
            ..GenParams::default()
        },
        "fire" => GenParams {
            flame: Flame::Classic,
            ..GenParams::default()
        },
        "snow" => GenParams {
            sky: Sky::Night,
            ..GenParams::default()
        },
        "hearts" => GenParams {
            mood: Mood::Love,
            ..GenParams::default()
        },
        "bounce" => GenParams {
            size: Size::Medium,
            ..GenParams::default()
        },
        "ripples" => GenParams {
            palette: Palette::Rainbow,
            ..GenParams::default()
        },
        "eq" => GenParams {
            color: Color::Classic,
            ..GenParams::default()
        },
        _ => GenParams::default(),
    }
}

// ---------- generators ----------

fn rainbow(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let (s, v) = match p.soft {
        Soft::Full => (1.0, 1.0),
        Soft::Pastel => (0.45, 1.0),
        Soft::Dark => (1.0, 0.6),
    };
    let cx = W as f64 / 2.0;
    let cy = H as f64 / 2.0;
    let maxd = (cx * cx + cy * cy).sqrt();
    (0..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let img = per_pixel(|x, y| {
                let at = match p.pattern {
                    Pattern::H => x as f64 / W as f64,
                    Pattern::V => y as f64 / H as f64,
                    Pattern::D => (x as f64 + y as f64) / ((W + H) as f64),
                    Pattern::R => ((x as f64 - cx).hypot(y as f64 - cy)) / maxd,
                    Pattern::S => {
                        (y as f64 - cy).atan2(x as f64 - cx) / TAU
                            + (x as f64 - cx).hypot(y as f64 - cy) / maxd
                    }
                    Pattern::C => (y as f64 - cy).atan2(x as f64 - cx) / TAU,
                };
                let c = hsv((at * p.bands as f64 - t) * 360.0, s, v);
                image::Rgba([c[0], c[1], c[2], 255])
            });
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn plasma(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let pal = palette(match p.palette {
        Palette::Rainbow => "rainbow",
        Palette::Neon => "neon",
        Palette::Fire => "fire",
        Palette::Ocean => "ocean",
        Palette::Toxic => "toxic",
        Palette::Mono => "mono",
    });
    (0..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let tau = t * TAU;
            let img = per_pixel(|x, y| {
                let v = (x as f64 / 13.0 + tau).sin()
                    + (y as f64 / 9.0 - tau).sin()
                    + ((x as f64 + y as f64) / 17.0 + 2.0 * tau).sin()
                    + (((x as f64 - 64.0 + 30.0 * tau.sin())
                        .hypot(y as f64 - 64.0 + 30.0 * tau.cos()))
                        / 9.0)
                        .sin();
                let c = pal((v + 4.0) / 8.0, t);
                image::Rgba([c[0], c[1], c[2], 255])
            });
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn matrix(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    // ASCII subset of the reference glyph set; our bitmap font covers these.
    let glyphs: Vec<char> = "0123456789ABCDEF<>*+".chars().collect();
    let mut mr = Mulberry::new(1);
    let mut cols: Vec<(f64, f64, u8, u16, u64)> = Vec::with_capacity(16);
    for i in 0..16 {
        let off = mr.next();
        let speed = (1.0 + (mr.next() * 3.0).floor()) as u8;
        let trail = (6.0 + (mr.next() * 8.0).floor()) as u16;
        let seed = (mr.next() * 1e6) as u64;
        cols.push((i as f64 * 8.0, off, speed, trail, seed));
    }
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let mut img = per_pixel(|_, _| image::Rgba([0, 0, 0, 255]));
            for (cx, off, speed, trail, mseed) in &cols {
                let span = 16.0 + *trail as f64;
                let head = frac(off + t * (*speed as f64)) * span;
                for k in 0..*trail {
                    let row = head.floor() - k as f64;
                    if row < 0.0 || row >= 16.0 {
                        continue;
                    }
                    let mut mb2 = Mulberry::new(
                        *mseed + (row as u64 * 31) + ((t * 12.0).floor() as u64 % 12),
                    );
                    let gidx = ((mb2.next() * glyphs.len() as f64) as usize).min(glyphs.len() - 1);
                    let glyph = glyphs[gidx];
                    let fade = 1.0 - k as f64 / *trail as f64;
                    let h = if p.color == Color::Rainbow {
                        cx * 2.8 + t * 360.0
                    } else {
                        match p.color {
                            Color::Green => 120.0,
                            Color::Blue => 200.0,
                            Color::Red => 0.0,
                            _ => 120.0,
                        }
                    };
                    let c = if k == 0 {
                        [230, 255, 230]
                    } else {
                        hsv(h, 1.0, 0.25 + 0.75 * fade)
                    };
                    crate::fx::draw_text_to(
                        &mut img,
                        *cx,
                        row * 8.0,
                        &glyph.to_string(),
                        7.0,
                        image::Rgba([c[0], c[1], c[2], 255]),
                    );
                }
            }
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn stars(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let mut mr = Mulberry::new(1);
    let mut stars: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(140);
    for _ in 0..140 {
        stars.push((
            mr.next() * 2.0 - 1.0,
            mr.next() * 2.0 - 1.0,
            mr.next(),
            mr.next() * 360.0,
        ));
    }
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let mut img = per_pixel(|_, _| image::Rgba([2, 3, 10, 255]));
            for (x, y, z, h) in &stars {
                let z = frac(z - t) * 0.95 + 0.05;
                let z2 = (z + 0.06).min(1.0);
                let sx = 64.0 + (x / z) * 36.0;
                let sy = 64.0 + (y / z) * 36.0;
                if sx < -4.0 || sx > W as f64 + 4.0 || sy < -4.0 || sy > H as f64 + 4.0 {
                    continue;
                }
                let b = 1.0 - z;
                let c = if p.color == Color::Rainbow {
                    hsv(h + t * 360.0, 0.8, b)
                } else if p.color == Color::Gold {
                    hsv(42.0, 0.7, b)
                } else {
                    [(255.0 * b) as u8, (255.0 * b) as u8, (255.0 * b) as u8]
                };
                if p.trail == Trail::On {
                    draw_line(
                        &mut img,
                        64.0 + (x / z2) * 36.0,
                        64.0 + (y / z2) * 36.0,
                        sx,
                        sy,
                        image::Rgba([c[0], c[1], c[2], 255]),
                        0.6 + b * 1.6,
                    );
                } else {
                    let r2 = 0.5 + b * 1.8;
                    fill_rect(
                        &mut img,
                        sx - r2 / 2.0,
                        sy - r2 / 2.0,
                        r2,
                        r2,
                        image::Rgba([c[0], c[1], c[2], 255]),
                    );
                }
            }
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn fire(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let pal = palette(match p.flame {
        Flame::Classic => "fire",
        Flame::Ice => "ice",
        Flame::Toxic => "toxic",
        Flame::Rainbow => "rainbow",
    });
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let tau = t * TAU;
            let img = per_pixel(|x, y| {
                let up = y as f64 / H as f64;
                let n = (x as f64 * 0.19 + y as f64 * 0.31 + 3.0 * tau).sin() * 0.5
                    + (x as f64 * 0.07 - y as f64 * 0.23 + 2.0 * tau + 1.3).sin() * 0.7
                    + (x as f64 * 0.41 + y as f64 * 0.53 + 5.0 * tau + 2.1).sin() * 0.3
                    + (x as f64 * 0.11 + y as f64 * 0.67 + 4.0 * tau).sin() * 0.4;
                let heat = up.powf(1.6) * 1.35 + n * 0.22 * up - 0.18;
                let c = if p.flame == Flame::Rainbow {
                    hsv(x as f64 * 2.8 + t * 360.0, 1.0, clamp01(heat * 1.4))
                } else {
                    pal(clamp01(heat), t)
                };
                image::Rgba([c[0], c[1], c[2], 255])
            });
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn snow(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let mut mr = Mulberry::new(1);
    let mut flakes: Vec<(f64, f64, f64, u16, f64, u16)> = Vec::with_capacity(110);
    for _ in 0..110 {
        flakes.push((
            mr.next() * W as f64,
            mr.next(),
            0.6 + mr.next() * 1.9,
            (1.0 + (mr.next() * 2.0).floor()) as u16,
            mr.next(),
            (1.0 + (mr.next() * 3.0).floor()) as u16,
        ));
    }
    let sky = match p.sky {
        Sky::Night => ["#03081f", "#1b2d5a"],
        Sky::Dusk => ["#2a0f3d", "#c2577a"],
        Sky::Black => ["#000", "#000"],
    };
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let mut img = per_pixel(|_, _| image::Rgba([0, 0, 0, 255]));
            for yy in 0..H as u32 {
                let mix = yy as f64 / H as f64;
                let c0 = parse_hex(sky[0]);
                let c1 = parse_hex(sky[1]);
                let c = [
                    (c0[0] as f64 + (c1[0] as f64 - c0[0] as f64) * mix) as u8,
                    (c0[1] as f64 + (c1[1] as f64 - c0[1] as f64) * mix) as u8,
                    (c0[2] as f64 + (c1[2] as f64 - c0[2] as f64) * mix) as u8,
                ];
                for xx in 0..W as u32 {
                    img.put_pixel(xx, yy, image::Rgba([c[0], c[1], c[2], 255]));
                }
            }
            for (x, y, r, k, ph, m) in &flakes {
                let yy = frac(y + t * (*k as f64)) * (H as f64 + 10.0) - 5.0;
                let xx = x + ((t * (*m as f64) + ph) * TAU).sin() * 4.0;
                let alpha = (0.5 + r / 5.0).min(1.0);
                fill_circle(
                    &mut img,
                    xx,
                    yy,
                    *r,
                    image::Rgba([255, 255, 255, (255.0 * alpha) as u8]),
                );
            }
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn hearts(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let mut mr = Mulberry::new(1);
    let mut hearts: Vec<(f64, f64, f64, f64, f64)> = Vec::with_capacity(26);
    for _ in 0..26 {
        hearts.push((
            mr.next() * W as f64,
            mr.next(),
            7.0 + mr.next() * 12.0,
            mr.next(),
            mr.next(),
        ));
    }
    let bg = if p.mood == Mood::Pastel {
        [255, 240, 245]
    } else {
        [20, 2, 10]
    };
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let mut img = per_pixel(|_, _| image::Rgba([bg[0], bg[1], bg[2], 255]));
            for (x, y, s, ph, h) in &hearts {
                let yy = H as f64 + 12.0 - frac(y + t) * (H as f64 + 30.0);
                let xx = x + ((t * 2.0 + ph) * TAU).sin() * 6.0;
                let c = if p.mood == Mood::Rainbow {
                    hsv(h * 360.0 + t * 360.0, 1.0, 1.0)
                } else if p.mood == Mood::Pastel {
                    hsv(330.0 + h * 60.0, 0.35, 1.0)
                } else {
                    hsv(340.0 + h * 30.0, 0.85, 0.6 + h * 0.4)
                };
                let scale = s * (1.0 + 0.08 * ((t * 4.0 + ph) * TAU).sin());
                draw_heart(
                    &mut img,
                    xx,
                    yy,
                    scale,
                    image::Rgba([c[0], c[1], c[2], 255]),
                );
            }
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn bounce(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let text = if p.text.is_empty() {
        " ".to_string()
    } else {
        p.text.clone()
    };
    let size = match p.size {
        Size::Small => 18.0,
        Size::Medium => 26.0,
        Size::Large => 36.0,
    };
    let text_w = crate::fx::text_width(&text, size, "sans");
    let w = (W as f64 - 2.0).min(text_w);
    let h = size;
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let mut img = per_pixel(|_, _| image::Rgba([0, 0, 0, 255]));
            let x = tri(t * 2.0 + 0.13) * (W as f64 - w);
            let y = tri(t * 3.0 + 0.31) * (H as f64 - h);
            let hits = ((t * 2.0 + 0.13) * 2.0).floor() + ((t * 3.0 + 0.31) * 2.0).floor();
            let c = hsv(hits * 36.0, 1.0, 1.0);
            crate::fx::draw_text_max(
                &mut img,
                x,
                y,
                &text,
                size,
                Some(W as f64 - 2.0),
                [c[0], c[1], c[2]],
            );
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn ripples(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let pal = palette(match p.palette {
        Palette::Rainbow => "rainbow",
        Palette::Ocean => "ocean",
        Palette::Neon => "neon",
        Palette::Mono => "mono",
        _ => "rainbow",
    });
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let tau = t * TAU;
            let ax = 64.0 + 34.0 * tau.cos();
            let ay = 64.0 + 34.0 * tau.sin();
            let bx = 64.0 - 34.0 * tau.cos();
            let by = 64.0 - 34.0 * tau.sin();
            let img = per_pixel(|x, y| {
                let v = ((x as f64 - ax).hypot(y as f64 - ay) * 0.32 - tau * 3.0).sin()
                    + ((x as f64 - bx).hypot(y as f64 - by) * 0.32 - tau * 3.0).sin();
                let c = pal((v + 2.0) / 4.0, t);
                image::Rgba([c[0], c[1], c[2], 255])
            });
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

fn eq(p: &GenParams, n: u16) -> Vec<SourceFrame> {
    let mut mr = Mulberry::new(1);
    let mut waves: Vec<Vec<(f64, f64, f64)>> = Vec::with_capacity(16);
    for _ in 0..16 {
        let mut bar = Vec::with_capacity(3);
        for _ in 0..3 {
            bar.push((
                1.0 + (mr.next() * 4.0).floor(),
                mr.next() * TAU,
                0.3 + mr.next() * 0.5,
            ));
        }
        waves.push(bar);
    }
    (0..n)
        .map(move |i| {
            let t = i as f64 / n as f64;
            let mut img = per_pixel(|_, _| image::Rgba([0, 0, 0, 255]));
            let bw = 8;
            let segs = 16;
            let sh = 8;
            for (idx, wave) in waves.iter().enumerate() {
                let mut v = 0.45;
                for w in wave {
                    v += w.2 * 0.5 * (t * TAU * w.0 + w.1).sin();
                }
                let lit = (clamp01(v) * segs as f64).round() as u32;
                for k in 0..lit {
                    let c = if p.color == Color::Rainbow {
                        hsv(idx as f64 * 22.0 + t * 360.0, 1.0, 1.0)
                    } else if p.color == Color::Mono {
                        [230, 230, 230]
                    } else {
                        hsv(120.0 - (k as f64 / segs as f64) * 120.0, 1.0, 1.0)
                    };
                    let x0 = (idx * bw + 1) as f64;
                    let y0 = H as f64 - (k + 1) as f64 * sh as f64 + 1.0;
                    fill_rect(
                        &mut img,
                        x0,
                        y0,
                        (bw - 2) as f64,
                        (sh - 2) as f64,
                        image::Rgba([c[0], c[1], c[2], 255]),
                    );
                }
            }
            SourceFrame {
                image: img,
                delay_ms: 60,
            }
        })
        .collect()
}

// ---------- drawing helpers ----------

pub struct Mulberry(u32);
impl Mulberry {
    pub fn new(seed: u64) -> Self {
        Self(seed as u32)
    }
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(1 | t);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        t ^= t >> 14;
        (t as f64) / 4294967296.0
    }
}

fn parse_hex(s: &str) -> [u8; 3] {
    let h = s.trim_start_matches('#');
    [
        u8::from_str_radix(&h[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&h[4..6], 16).unwrap_or(0),
    ]
}

pub fn fill_rect(img: &mut RgbaImage, x: f64, y: f64, w: f64, h: f64, c: image::Rgba<u8>) {
    let x0 = x.max(0.0).min(W as f64) as i32;
    let y0 = y.max(0.0).min(H as f64) as i32;
    let x1 = (x + w).ceil().max(0.0).min(W as f64) as i32;
    let y1 = (y + h).ceil().max(0.0).min(H as f64) as i32;
    for yy in y0..y1 {
        for xx in x0..x1 {
            img.put_pixel(xx as u32, yy as u32, c);
        }
    }
}

pub fn fill_circle(img: &mut RgbaImage, cx: f64, cy: f64, r: f64, c: image::Rgba<u8>) {
    let r = r.max(0.0);
    let x0 = (cx - r).max(0.0).min(W as f64) as i32;
    let y0 = (cy - r).max(0.0).min(H as f64) as i32;
    let x1 = (cx + r).ceil().max(0.0).min(W as f64) as i32;
    let y1 = (cy + r).ceil().max(0.0).min(H as f64) as i32;
    for yy in y0..y1 {
        for xx in x0..x1 {
            if ((xx as f64 - cx).hypot(yy as f64 - cy)) <= r {
                img.put_pixel(xx as u32, yy as u32, c);
            }
        }
    }
}

pub fn draw_line(
    img: &mut RgbaImage,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    c: image::Rgba<u8>,
    width: f64,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let steps = dx.abs().max(dy.abs()).max(1.0) as i32;
    for s in 0..=steps {
        let t = s as f64 / steps as f64;
        fill_circle(img, x0 + dx * t, y0 + dy * t, width / 2.0, c);
    }
}

pub fn draw_heart(img: &mut RgbaImage, cx: f64, cy: f64, s: f64, c: image::Rgba<u8>) {
    let s = s.max(1.0);
    // Sample the four cubic bezier segments of the reference heart path.
    let segs = [
        (
            (cx, cy + 0.3 * s),
            (cx, cy),
            (cx - 0.5 * s, cy),
            (cx - 0.5 * s, cy + 0.3 * s),
        ),
        (
            (cx - 0.5 * s, cy + 0.3 * s),
            (cx - 0.5 * s, cy + 0.6 * s),
            (cx, cy + 0.8 * s),
            (cx, cy + s),
        ),
        (
            (cx, cy + s),
            (cx, cy + 0.8 * s),
            (cx + 0.5 * s, cy + 0.6 * s),
            (cx + 0.5 * s, cy + 0.3 * s),
        ),
        (
            (cx + 0.5 * s, cy + 0.3 * s),
            (cx + 0.5 * s, cy),
            (cx, cy),
            (cx, cy + 0.3 * s),
        ),
    ];
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(36);
    for (p0, p1, p2, p3) in segs {
        for j in 0..=8 {
            let t = j as f64 / 8.0;
            let mt = 1.0 - t;
            pts.push((
                mt * mt * mt * p0.0
                    + 3.0 * mt * mt * t * p1.0
                    + 3.0 * mt * t * t * p2.0
                    + t * t * t * p3.0,
                mt * mt * mt * p0.1
                    + 3.0 * mt * mt * t * p1.1
                    + 3.0 * mt * t * t * p2.1
                    + t * t * t * p3.1,
            ));
        }
    }
    let miny = pts
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as i32;
    let maxy = pts
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min(H as f64 - 1.0) as i32;
    for y in miny..=maxy {
        let yf = y as f64 + 0.5;
        let mut xs: Vec<f64> = Vec::new();
        for i in 0..pts.len() {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % pts.len()];
            if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                let t = (yf - y0) / (y1 - y0);
                xs.push(x0 + (x1 - x0) * t);
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks_exact(2) {
            let x0 = pair[0].ceil().max(0.0) as i32;
            let x1 = pair[1].floor().min(W as f64 - 1.0) as i32;
            for x in x0..=x1 {
                img.put_pixel(x as u32, y as u32, c);
            }
        }
    }
}
