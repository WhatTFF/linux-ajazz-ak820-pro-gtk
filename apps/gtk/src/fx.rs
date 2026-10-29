//! Text layout + post-effects ported from `ak820-screen` `fx.js`.
//! Text is rendered with cairo; everything else is pure pixel-buffer
//! manipulation on `image::RgbaImage`.

use crate::gen::{hsv, TAU};
use ak820_proto::{H, W};
use image::RgbaImage;

const ANIMATED_FX: [&str; 9] = [
    "spin", "shake", "breathe", "pulse", "glitch", "cycle", "wash", "border", "sparkle",
];

pub fn is_animated(fx: &Fx) -> bool {
    ANIMATED_FX.iter().any(|k| fx.get(k)) || (fx.caption.is_some() && fx.cap_rainbow)
}

// ---------- text (cairo) ----------

use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};

fn font_context(surface: &ImageSurface, family: &str, size: f64) -> Context {
    let ctx = Context::new(surface);
    ctx.select_font_face(family, FontSlant::Normal, FontWeight::Bold);
    ctx.set_font_size(size);
    ctx
}

/// Advance width of `text` in the given font.
pub fn text_width(text: &str, size: f64, family: &str) -> f64 {
    let surface = ImageSurface::create(Format::A8, 1, 1).expect("cairo surface");
    let ctx = font_context(&surface, family, size);
    ctx.text_extents(text).x_advance
}

fn wrap_ctx(ctx: &Context, text: &str, max_w: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let next = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && ctx.text_extents(&next).x_advance > max_w {
                lines.push(line.clone());
                line = word.to_string();
            } else {
                line = next;
            }
        }
        lines.push(line);
    }
    lines
}

/// Largest size <= maxSize where the wrapped text fits the box.
pub fn fit_text(
    _ctx: &(),
    text: &str,
    family: &str,
    max_size: f64,
    box_w: f64,
    box_h: f64,
) -> (f64, Vec<String>) {
    let surface = ImageSurface::create(Format::A8, 1, 1).expect("cairo surface");
    let ctx = font_context(&surface, family, max_size);
    let mut size = max_size;
    loop {
        ctx.set_font_size(size);
        let lines = wrap_ctx(&ctx, text, box_w);
        let lh = size * 1.12;
        let fits = lines.len() as f64 * lh <= box_h
            && lines.iter().all(|l| ctx.text_extents(l).x_advance <= box_w);
        if fits || size <= 8.0 {
            return (size, lines);
        }
        size -= 1.0;
    }
}

pub struct TextOpts {
    pub size: f64,
    pub family: String,
    pub cx: Option<f64>,
    pub cy: f64,
    pub fill: [u8; 3],
    pub rainbow: bool,
    pub phase: f64,
    pub wave: Option<f64>,
    pub outline: Option<[u8; 3]>,
    pub limit: Option<usize>,
}

/// Draw centred lines. `cy` is the vertical centre of the text block.
pub fn draw_lines_to_surface(lines: &[String], o: &TextOpts) -> RgbaImage {
    let mut surface =
        ImageSurface::create(Format::ARgb32, W as i32, H as i32).expect("cairo surface");
    {
        let ctx = font_context(&surface, &o.family, o.size);
        ctx.set_line_join(cairo::LineJoin::Round);
        ctx.set_line_width((o.size / 7.0).max(2.0));
        let lh = o.size * 1.12;
        let total: f64 = lines
            .iter()
            .map(|l| l.chars().count() as f64)
            .sum::<f64>()
            .max(1.0);
        let per_char = o.rainbow || o.wave.is_some() || o.limit.is_some();
        let mut ci = 0usize;
        for (li, line) in lines.iter().enumerate() {
            let y0 = o.cy + (li as f64 - (lines.len() - 1) as f64 / 2.0) * lh;
            let ext = ctx.text_extents(line);
            let baseline = y0 - ext.y_bearing - ext.height / 2.0;
            let x = o.cx.unwrap_or(W as f64 / 2.0) - ext.x_advance / 2.0;
            if !per_char {
                if let Some(oc) = o.outline {
                    ctx.set_source_rgb(
                        oc[0] as f64 / 255.0,
                        oc[1] as f64 / 255.0,
                        oc[2] as f64 / 255.0,
                    );
                    ctx.move_to(x, baseline);
                    ctx.text_path(line);
                    ctx.stroke();
                }
                ctx.set_source_rgb(
                    o.fill[0] as f64 / 255.0,
                    o.fill[1] as f64 / 255.0,
                    o.fill[2] as f64 / 255.0,
                );
                ctx.move_to(x, baseline);
                ctx.show_text(line);
                continue;
            }
            let mut cx = x;
            for ch in line.chars() {
                if o.limit.is_some_and(|lim| ci >= lim) {
                    break;
                }
                let y = baseline
                    + o.wave
                        .map_or(0.0, |w| (TAU * (o.phase - ci as f64 / 9.0)).sin() * w);
                let s = ch.to_string();
                if let Some(oc) = o.outline {
                    ctx.set_source_rgb(
                        oc[0] as f64 / 255.0,
                        oc[1] as f64 / 255.0,
                        oc[2] as f64 / 255.0,
                    );
                    ctx.move_to(cx, y);
                    ctx.text_path(&s);
                    ctx.stroke();
                }
                if o.rainbow {
                    let c = hsv(ci as f64 / total * 360.0 - o.phase * 360.0, 1.0, 1.0);
                    ctx.set_source_rgb(
                        c[0] as f64 / 255.0,
                        c[1] as f64 / 255.0,
                        c[2] as f64 / 255.0,
                    );
                } else {
                    ctx.set_source_rgb(
                        o.fill[0] as f64 / 255.0,
                        o.fill[1] as f64 / 255.0,
                        o.fill[2] as f64 / 255.0,
                    );
                }
                ctx.move_to(cx, y);
                ctx.show_text(&s);
                cx += ctx.text_extents(&s).x_advance;
                ci += 1;
            }
        }
    }
    argb_to_rgba(&mut surface)
}

fn pixel_parts(data: &[u8], off: usize) -> (u8, u8, u8, u8) {
    // Cairo ARGB32 is premultiplied BGRA on little-endian.
    #[cfg(target_endian = "little")]
    {
        (data[off + 2], data[off + 1], data[off], data[off + 3])
    }
    #[cfg(target_endian = "big")]
    {
        (data[off + 1], data[off + 2], data[off + 3], data[off])
    }
}

fn argb_to_rgba(surface: &mut ImageSurface) -> RgbaImage {
    let stride = surface.get_stride() as usize;
    let mut img = RgbaImage::from_pixel(W as u32, H as u32, image::Rgba([0, 0, 0, 0]));
    {
        let data = surface.get_data().expect("cairo data");
        for y in 0..H as u32 {
            for x in 0..W as u32 {
                let off = y as usize * stride + x as usize * 4;
                let (r, g, b, a) = pixel_parts(&data, off);
                if a > 0 {
                    let inv = 255.0 / a as f64;
                    img.put_pixel(
                        x,
                        y,
                        image::Rgba([
                            (r as f64 * inv).min(255.0) as u8,
                            (g as f64 * inv).min(255.0) as u8,
                            (b as f64 * inv).min(255.0) as u8,
                            a,
                        ]),
                    );
                }
            }
        }
    }
    img
}

/// Source-over composite `src` onto `dst`.
pub fn composite_over(dst: &mut RgbaImage, src: &RgbaImage) {
    for (x, y, p) in src.enumerate_pixels() {
        let a = p.0[3] as f64 / 255.0;
        if a <= 0.0 {
            continue;
        }
        let d = dst.get_pixel(x, y).0;
        dst.put_pixel(
            x,
            y,
            image::Rgba([
                (d[0] as f64 * (1.0 - a) + p.0[0] as f64 * a).round() as u8,
                (d[1] as f64 * (1.0 - a) + p.0[1] as f64 * a).round() as u8,
                (d[2] as f64 * (1.0 - a) + p.0[2] as f64 * a).round() as u8,
                255,
            ]),
        );
    }
}

/// Draw text with its top-left at `(x, y)`, scaled down to `max_w` if needed.
pub fn draw_text_max(
    dst: &mut RgbaImage,
    x: f64,
    y: f64,
    text: &str,
    size: f64,
    max_w: Option<f64>,
    color: [u8; 3],
) {
    draw_text_family(dst, x, y, text, size, max_w, color, "sans");
}

#[allow(clippy::too_many_arguments)]
fn draw_text_family(
    dst: &mut RgbaImage,
    x: f64,
    y: f64,
    text: &str,
    size: f64,
    max_w: Option<f64>,
    color: [u8; 3],
    family: &str,
) {
    let mut surface =
        ImageSurface::create(Format::ARgb32, W as i32, H as i32).expect("cairo surface");
    {
        let ctx = font_context(&surface, family, size);
        let tw = ctx.text_extents(text).x_advance;
        let scale = match max_w {
            Some(m) if tw > m && tw > 0.0 => m / tw,
            _ => 1.0,
        };
        ctx.set_source_rgb(
            color[0] as f64 / 255.0,
            color[1] as f64 / 255.0,
            color[2] as f64 / 255.0,
        );
        let ascent = ctx.font_extents().ascent;
        ctx.save();
        ctx.translate(x, y);
        if scale < 1.0 {
            ctx.scale(scale, 1.0);
        }
        ctx.move_to(0.0, ascent);
        ctx.show_text(text);
        ctx.restore();
    }
    surface.flush();
    let stride = surface.get_stride() as usize;
    {
        let data = surface.get_data().expect("cairo data");
        for yy in 0..H as u32 {
            for xx in 0..W as u32 {
                let off = yy as usize * stride + xx as usize * 4;
                let (r, g, b, a) = pixel_parts(&data, off);
                let alpha = a as f64 / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let inv = 255.0 / a as f64;
                let sr = (r as f64 * inv).min(255.0);
                let sg = (g as f64 * inv).min(255.0);
                let sb = (b as f64 * inv).min(255.0);
                let d = dst.get_pixel(xx, yy).0;
                dst.put_pixel(
                    xx,
                    yy,
                    image::Rgba([
                        (d[0] as f64 * (1.0 - alpha) + sr * alpha).round() as u8,
                        (d[1] as f64 * (1.0 - alpha) + sg * alpha).round() as u8,
                        (d[2] as f64 * (1.0 - alpha) + sb * alpha).round() as u8,
                        255,
                    ]),
                );
            }
        }
    }
}

pub fn draw_text_to(
    dst: &mut RgbaImage,
    x: f64,
    y: f64,
    text: &str,
    size: f64,
    color: image::Rgba<u8>,
) {
    draw_text_family(
        dst,
        x,
        y,
        text,
        size,
        None,
        [color.0[0], color.0[1], color.0[2]],
        "monospace",
    );
}

// ---------- effects state ----------

#[derive(Clone, Copy, PartialEq)]
pub enum CapPos {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone)]
pub struct Fx {
    pub cycle: bool,
    pub wash: bool,
    pub border: bool,
    pub spin: u8,
    pub shake: bool,
    pub breathe: bool,
    pub pulse: bool,
    pub glitch: bool,
    pub sparkle: bool,
    pub scan: bool,
    pub vignette: bool,
    pub invert: bool,
    pub sepia: bool,
    pub gray: bool,
    pub posterize: bool,
    pub hue: f64,
    pub pixel: u8,
    pub caption: Option<String>,
    pub cap_pos: CapPos,
    pub cap_color: [u8; 3],
    pub cap_rainbow: bool,
    pub loop_len: u16,
}

impl Default for Fx {
    fn default() -> Self {
        Self {
            cycle: false,
            wash: false,
            border: false,
            spin: 0,
            shake: false,
            breathe: false,
            pulse: false,
            glitch: false,
            sparkle: false,
            scan: false,
            vignette: false,
            invert: false,
            sepia: false,
            gray: false,
            posterize: false,
            hue: 0.0,
            pixel: 1,
            caption: None,
            cap_pos: CapPos::Bottom,
            cap_color: [255, 255, 255],
            cap_rainbow: false,
            loop_len: 24,
        }
    }
}

impl Fx {
    pub fn get(&self, k: &str) -> bool {
        match k {
            "spin" => self.spin > 0,
            "shake" => self.shake,
            "breathe" => self.breathe,
            "pulse" => self.pulse,
            "glitch" => self.glitch,
            "sparkle" => self.sparkle,
            "cycle" => self.cycle,
            "wash" => self.wash,
            "border" => self.border,
            _ => false,
        }
    }
}

pub struct Motion {
    pub bri: f64,
    pub hue: f64,
    pub dx: f64,
    pub dy: f64,
    pub spin: f64,
    pub zoom: f64,
}

pub fn motion(fx: &Fx, t: f64) -> Motion {
    let s = (TAU * t).sin();
    Motion {
        bri: if fx.pulse { 1.0 + 0.4 * s } else { 1.0 },
        hue: fx.hue + if fx.cycle { 360.0 * t } else { 0.0 },
        dx: if fx.shake {
            ((TAU * t * 5.0).sin() * 4.0 + (TAU * t * 13.0).sin() * 2.0).round()
        } else {
            0.0
        },
        dy: if fx.shake {
            ((TAU * t * 7.0).cos() * 4.0).round()
        } else {
            0.0
        },
        spin: if fx.spin > 0 {
            360.0 * t * fx.spin as f64
        } else {
            0.0
        },
        zoom: if fx.breathe {
            1.0 + 0.16 * (1.0 - (TAU * t).cos()) / 2.0
        } else {
            1.0
        },
    }
}

#[derive(Clone, Copy)]
pub enum Fit {
    Cover,
    Contain,
    Stretch,
}

#[derive(Clone)]
pub struct FrameOpts {
    pub fit: Fit,
    pub rot: u32,
    pub mirror: bool,
    pub bg: [u8; 3],
    pub zoom: f64,
    pub panx: f64,
    pub pany: f64,
    pub bri: f64,
    pub con: f64,
    pub sat: f64,
}

pub fn draw_frame(
    bmp: &RgbaImage,
    o: &FrameOpts,
    fx: &Fx,
    t: f64,
    i: usize,
    backdrop: Option<&RgbaImage>,
    out: &mut RgbaImage,
) {
    let m = motion(fx, t);
    if let Some(bg) = backdrop {
        copy_into(bg, out);
    } else {
        fill_rgba(out, image::Rgba([o.bg[0], o.bg[1], o.bg[2], 255]));
    }

    let bri = o.bri / 100.0 * m.bri;
    let con = o.con / 100.0;
    let sat = o.sat / 100.0;
    let angle = (o.rot as f64 + m.spin).to_radians();
    let (sin_a, cos_a) = angle.sin_cos();
    let zoom = (o.zoom / 100.0) * m.zoom;
    let cx = W as f64 / 2.0 + (o.panx / 100.0) * W as f64 / 2.0 + m.dx;
    let cy = H as f64 / 2.0 + (o.pany / 100.0) * H as f64 / 2.0 + m.dy;
    let (sw, sh) = (bmp.width() as f64, bmp.height() as f64);
    let ox = (sw - 1.0) / 2.0;
    let oy = (sh - 1.0) / 2.0;

    for yy in 0..H as i32 {
        for xx in 0..W as i32 {
            let px = xx as f64 + 0.5 - cx;
            let py = yy as f64 + 0.5 - cy;
            let rx = px * cos_a + py * sin_a;
            let ry = -px * sin_a + py * cos_a;
            let rx = if o.mirror { -rx } else { rx };
            let src_x = rx / zoom + ox;
            let src_y = ry / zoom + oy;
            if src_x >= 0.0 && src_x < sw && src_y >= 0.0 && src_y < sh {
                let mut c = sample_bilinear(bmp, src_x, src_y);
                c[0] = adj(c[0], bri, con);
                c[1] = adj(c[1], bri, con);
                c[2] = adj(c[2], bri, con);
                let lum = 0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64;
                for ch in &mut c {
                    *ch = (lum + (*ch as f64 - lum) * sat).clamp(0.0, 255.0) as u8;
                }
                let mut c = hue_rotate(c, m.hue);
                c = apply_filters(c, fx);
                out.put_pixel(xx as u32, yy as u32, image::Rgba([c[0], c[1], c[2], 255]));
            }
        }
    }
    post(out, fx, t, i);
}

/// Bilinear sample; coordinates are in pixel-centre space.
fn sample_bilinear(img: &RgbaImage, x: f64, y: f64) -> [u8; 3] {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let x0 = x0 as i64;
    let y0 = y0 as i64;
    let w = img.width() as i64;
    let h = img.height() as i64;
    let get = |xx: i64, yy: i64| -> [f64; 3] {
        let p = img
            .get_pixel(xx.clamp(0, w - 1) as u32, yy.clamp(0, h - 1) as u32)
            .0;
        [p[0] as f64, p[1] as f64, p[2] as f64]
    };
    let p00 = get(x0, y0);
    let p10 = get(x0 + 1, y0);
    let p01 = get(x0, y0 + 1);
    let p11 = get(x0 + 1, y0 + 1);
    let mut out = [0u8; 3];
    for k in 0..3 {
        let v = p00[k] * (1.0 - fx) * (1.0 - fy)
            + p10[k] * fx * (1.0 - fy)
            + p01[k] * (1.0 - fx) * fy
            + p11[k] * fx * fy;
        out[k] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// CSS-style grayscale / sepia / invert filters, in that order.
fn apply_filters(c: [u8; 3], fx: &Fx) -> [u8; 3] {
    let mut c = c;
    if fx.gray {
        let l = (0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64).round() as u8;
        c = [l, l, l];
    }
    if fx.sepia {
        let r = 0.393 * c[0] as f64 + 0.769 * c[1] as f64 + 0.189 * c[2] as f64;
        let g = 0.349 * c[0] as f64 + 0.686 * c[1] as f64 + 0.168 * c[2] as f64;
        let b = 0.272 * c[0] as f64 + 0.534 * c[1] as f64 + 0.131 * c[2] as f64;
        c = [r.min(255.0) as u8, g.min(255.0) as u8, b.min(255.0) as u8];
    }
    if fx.invert {
        c = [255 - c[0], 255 - c[1], 255 - c[2]];
    }
    c
}

fn adj(v: u8, bri: f64, con: f64) -> u8 {
    let mut x = v as f64 / 255.0 * bri;
    x = (x - 0.5) * con + 0.5;
    x = x.clamp(0.0, 1.0);
    (x * 255.0).round() as u8
}

fn hue_rotate(c: [u8; 3], deg: f64) -> [u8; 3] {
    if deg.abs() < 0.5 {
        return c;
    }
    let (r, g, b) = (
        c[0] as f64 / 255.0,
        c[1] as f64 / 255.0,
        c[2] as f64 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let hsv_h = if (max - min).abs() < 1e-6 {
        0.0
    } else {
        let d = max - min;
        let hh = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        hh * 60.0
    };
    let h = (hsv_h + deg).rem_euclid(360.0);
    let cc = hsv(h, 1.0, (max + min) / 2.0);
    [cc[0], cc[1], cc[2]]
}

fn copy_into(src: &RgbaImage, dst: &mut RgbaImage) {
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            dst.put_pixel(x, y, *src.get_pixel(x, y));
        }
    }
}

fn fill_rgba(img: &mut RgbaImage, c: image::Rgba<u8>) {
    for p in img.pixels_mut() {
        *p = c;
    }
}

pub fn post(img: &mut RgbaImage, fx: &Fx, t: f64, i: usize) {
    if fx.wash {
        wash(img, t);
    }
    if fx.pixel > 1 {
        pixelate(img, fx.pixel);
    }
    if fx.posterize {
        posterize(img);
    }
    if fx.glitch {
        glitch(img, i);
    }
    if fx.sparkle {
        sparkle(img, t);
    }
    if fx.scan {
        scan(img);
    }
    if fx.vignette {
        vignette(img);
    }
    if let Some(cap) = &fx.caption {
        if !cap.is_empty() {
            caption(img, cap, fx, t);
        }
    }
    if fx.border {
        border(img, t);
    }
}

fn wash(img: &mut RgbaImage, t: f64) {
    // Approximates canvas `globalCompositeOperation = 'color'`: keep the
    // gradient's hue/saturation and the destination's luminosity.
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let p = (x as f64 + y as f64) / (W + H) as f64;
            let hue = (p * 360.0 - t * 360.0).rem_euclid(360.0);
            let src = hsv(hue, 1.0, 1.0);
            let d = img.get_pixel(x, y).0;
            let lum_d = 0.299 * d[0] as f64 + 0.587 * d[1] as f64 + 0.114 * d[2] as f64;
            let lum_s = 0.299 * src[0] as f64 + 0.587 * src[1] as f64 + 0.114 * src[2] as f64;
            let k = if lum_s > 1.0 { lum_d / lum_s } else { 0.0 };
            let a = 0.8;
            let blend = [
                (src[0] as f64 * k).min(255.0),
                (src[1] as f64 * k).min(255.0),
                (src[2] as f64 * k).min(255.0),
            ];
            img.put_pixel(
                x,
                y,
                image::Rgba([
                    (d[0] as f64 * (1.0 - a) + blend[0] * a).round() as u8,
                    (d[1] as f64 * (1.0 - a) + blend[1] * a).round() as u8,
                    (d[2] as f64 * (1.0 - a) + blend[2] * a).round() as u8,
                    255,
                ]),
            );
        }
    }
}

fn pixelate(img: &mut RgbaImage, p: u8) {
    let s = (W as f64 / p as f64).max(4.0) as u32;
    let mut tmp = RgbaImage::from_pixel(s, s, image::Rgba([0, 0, 0, 255]));
    for yy in 0..s {
        for xx in 0..s {
            let ox = (xx as f64 / s as f64 * W as f64) as u32;
            let oy = (yy as f64 / s as f64 * H as f64) as u32;
            tmp.put_pixel(xx, yy, *img.get_pixel(ox, oy));
        }
    }
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            img.put_pixel(
                x,
                y,
                *tmp.get_pixel(
                    (x as f64 / W as f64 * s as f64) as u32,
                    (y as f64 / H as f64 * s as f64) as u32,
                ),
            );
        }
    }
}

fn posterize(img: &mut RgbaImage) {
    let step = 255.0 / 3.0;
    for p in img.pixels_mut() {
        let c = p.0;
        for (i, ch) in p.0.iter_mut().take(3).enumerate() {
            *ch = ((c[i] as f64 / step).round() * step).clamp(0.0, 255.0) as u8;
        }
    }
}

fn glitch(img: &mut RgbaImage, i: usize) {
    let seed = i as u64 * 7919 + 17;
    let mut mb = Mulberry::new(seed);
    if mb.next() > 0.6 {
        return;
    }
    let src: Vec<image::Rgba<u8>> = img.pixels().cloned().collect();
    let slices = (2.0 + mb.next() * 4.0) as u32;
    for _ in 0..slices {
        let y0 = (mb.next() * H as f64) as u32;
        let h = (2.0 + mb.next() * 10.0) as u32;
        let dx = ((mb.next() - 0.5) * 28.0) as i32;
        for y in y0..(y0 + h).min(H as u32) {
            for x in 0..W as u32 {
                let sx = (x as i32 - dx).clamp(0, W as i32 - 1) as u32;
                img.put_pixel(x, y, src[(y * W as u32 + sx) as usize]);
            }
        }
    }
    let shift = 1.0 + (mb.next() * 3.0).floor();
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let sp = src[(y * W as u32 + (x as f64 + shift) as u32).min(W as u32 - 1) as usize];
            let mut p = img.get_pixel(x, y).0;
            p[0] = sp.0[0];
            let sp2 =
                src[(y * W as u32 + (x as f64 - shift).max(0.0) as u32).min(W as u32 - 1) as usize];
            p[2] = sp2.0[2];
            img.put_pixel(x, y, image::Rgba([p[0], p[1], p[2], 255]));
        }
    }
}

fn blend_px(img: &mut RgbaImage, x: i32, y: i32, c: [u8; 3], a: f64) {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for (k, ch) in p.0.iter_mut().take(3).enumerate() {
        *ch = (*ch as f64 * (1.0 - a) + c[k] as f64 * a).round() as u8;
    }
    p.0[3] = 255;
}

fn blend_line(img: &mut RgbaImage, x0: f64, y0: f64, x1: f64, y1: f64, c: [u8; 3], a: f64) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs())).max(1.0) as i32;
    for s in 0..=steps {
        let t = s as f64 / steps as f64;
        blend_px(
            img,
            (x0 + (x1 - x0) * t).round() as i32,
            (y0 + (y1 - y0) * t).round() as i32,
            c,
            a,
        );
    }
}

fn sparkle(img: &mut RgbaImage, t: f64) {
    // Four-point stars, like the reference's rotated needles.
    let mut mb = Mulberry::new(99);
    for _ in 0..22 {
        let x = mb.next() * W as f64;
        let y = mb.next() * H as f64;
        let ph = mb.next();
        let size = 2.0 + mb.next() * 4.0;
        let a = (TAU * (t * 2.0 + ph)).sin().max(0.0);
        if a < 0.05 {
            continue;
        }
        let len = size * a;
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            blend_line(img, x, y, x + dx * len, y + dy * len, [255, 255, 255], a);
        }
    }
}

fn scan(img: &mut RgbaImage) {
    for y in (1..H as u32).step_by(2) {
        for x in 0..W as u32 {
            let p = img.get_pixel(x, y).0;
            img.put_pixel(
                x,
                y,
                image::Rgba([
                    (p[0] as f64 * 0.65) as u8,
                    (p[1] as f64 * 0.65) as u8,
                    (p[2] as f64 * 0.65) as u8,
                    255,
                ]),
            );
        }
    }
}

fn vignette(img: &mut RgbaImage) {
    let cx = W as f64 / 2.0;
    let cy = H as f64 / 2.0;
    let r0 = W as f64 * 0.25;
    let r1 = W as f64 * 0.75;
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let d = (x as f64 - cx).hypot(y as f64 - cy);
            let a = ((d - r0) / (r1 - r0)).clamp(0.0, 1.0) * 0.85;
            let p = img.get_pixel(x, y).0;
            img.put_pixel(
                x,
                y,
                image::Rgba([
                    (p[0] as f64 * (1.0 - a)).round() as u8,
                    (p[1] as f64 * (1.0 - a)).round() as u8,
                    (p[2] as f64 * (1.0 - a)).round() as u8,
                    255,
                ]),
            );
        }
    }
}

fn border(img: &mut RgbaImage, t: f64) {
    // Conic rainbow border, 6px wide.
    let cx = W as f64 / 2.0;
    let cy = H as f64 / 2.0;
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let edge = (x as f64)
                .min(y as f64)
                .min((W - 1) as f64 - x as f64)
                .min((H - 1) as f64 - y as f64);
            if edge >= 6.0 {
                continue;
            }
            let a = ((y as f64 - cy).atan2(x as f64 - cx) / TAU - t).rem_euclid(1.0);
            let c = hsv(a * 360.0, 1.0, 0.55);
            img.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], 255]));
        }
    }
}

pub fn caption(img: &mut RgbaImage, cap: &str, fx: &Fx, t: f64) {
    let (size, lines) = fit_text(&(), cap, "sans", 20.0, W as f64 - 10.0, H as f64 / 3.0);
    let block = lines.len() as f64 * size * 1.12;
    let cy = match fx.cap_pos {
        CapPos::Top => 6.0 + block / 2.0,
        CapPos::Middle => H as f64 / 2.0,
        CapPos::Bottom => H as f64 - 6.0 - block / 2.0,
    };
    let surf = draw_lines_to_surface(
        &lines,
        &TextOpts {
            size,
            family: "sans-serif".to_string(),
            cx: None,
            cy,
            fill: fx.cap_color,
            rainbow: fx.cap_rainbow,
            phase: t,
            wave: None,
            outline: Some([0, 0, 0]),
            limit: None,
        },
    );
    composite_over(img, &surf);
}

struct Mulberry(u32);
impl Mulberry {
    fn new(seed: u64) -> Self {
        Self(seed as u32)
    }
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(1 | t);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        t ^= t >> 14;
        (t as f64) / 4294967296.0
    }
}
