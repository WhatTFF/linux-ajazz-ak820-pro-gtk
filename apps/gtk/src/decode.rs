//! Decoding an image or animated GIF into 128×128 frames fitted to the screen.
//!
//! Same fitting logic as `ak820-cli::decode` so the GTK app and the CLI produce
//! byte-identical payloads for the same input.

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;

use ak820_proto::{FRAME_BYTES, H, W};
use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, RgbaImage};

/// One decoded frame, already fitted to the 128×128 screen.
pub struct SourceFrame {
    /// 128×128 RGBA8.
    pub image: RgbaImage,
    /// Delay in milliseconds.
    pub delay_ms: u16,
}

/// How to fit the source into the square screen.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Fill the screen, cropping the overflow (default, like the web app).
    Cover,
    /// Show the whole image, letterboxed on the background colour.
    Contain,
    /// Ignore the aspect ratio.
    Stretch,
}

pub fn load(
    path: &Path,
    fit: Fit,
    background: [u8; 4],
    force_delay: Option<u16>,
) -> Result<Vec<SourceFrame>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let raw: Vec<(RgbaImage, u16)> = match ext.as_str() {
        "gif" => {
            let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            decode_gif(BufReader::new(file))?
        }
        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v" => decode_video(path)?,
        _ => {
            let img = image::ImageReader::open(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .with_guessed_format()
                .map_err(|e| format!("{}: {e}", path.display()))?
                .decode()
                .map_err(|e| format!("{}: {e}", path.display()))?
                .to_rgba8();
            vec![(img, 100)]
        }
    };

    if raw.is_empty() {
        return Err(format!("{}: no frames", path.display()));
    }

    Ok(raw
        .into_iter()
        .map(|(img, delay)| SourceFrame {
            image: fit_to_screen(&img, fit, background),
            delay_ms: force_delay.unwrap_or(delay),
        })
        .collect())
}

/// Decode image/GIF bytes (clipboard, GIPHY) into fitted frames.
pub fn load_bytes(
    bytes: &[u8],
    fit: Fit,
    background: [u8; 4],
    force_delay: Option<u16>,
) -> Result<Vec<SourceFrame>, String> {
    if bytes.is_empty() {
        return Err("empty image".into());
    }
    let raw: Vec<(RgbaImage, u16)> = if bytes.starts_with(b"GIF8") {
        decode_gif(Cursor::new(bytes.to_vec()))?
    } else {
        let img = image::load_from_memory(bytes)
            .map_err(|e| format!("image: {e}"))?
            .to_rgba8();
        vec![(img, 100)]
    };
    if raw.is_empty() {
        return Err("no frames".into());
    }
    Ok(raw
        .into_iter()
        .map(|(img, delay)| SourceFrame {
            image: fit_to_screen(&img, fit, background),
            delay_ms: force_delay.unwrap_or(delay),
        })
        .collect())
}

fn decode_gif<R: std::io::BufRead + std::io::Seek + 'static>(
    reader: R,
) -> Result<Vec<(RgbaImage, u16)>, String> {
    let decoder = GifDecoder::new(reader).map_err(|e| e.to_string())?;
    let frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|e| e.to_string())?;

    Ok(frames
        .into_iter()
        .map(|frame| {
            let (num, den) = frame.delay().numer_denom_ms();
            // A zero denominator (or zero delay) means "use the default"; GIF frames
            // never go faster than 1 ms here.
            let ms = num.checked_div(den).map_or(100, |v| v.max(1));
            let ms = ms.min(u16::MAX as u32) as u16;
            (frame.into_buffer(), ms)
        })
        .collect())
}

/// Extract up to 255 frames at 12 fps with `ffmpeg` (optional dependency).
fn decode_video(path: &Path) -> Result<Vec<(RgbaImage, u16)>, String> {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            "fps=12,scale=128:128:force_original_aspect_ratio=increase,crop=128:128",
            "-frames:v",
            "255",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "video needs ffmpeg installed (Fedora: sudo dnf install ffmpeg)".to_string()
            } else {
                format!("ffmpeg: {e}")
            }
        })?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffmpeg could not read {}: {}",
            path.display(),
            err.lines().last().unwrap_or("unknown error").trim()
        ));
    }
    let frame_len = W * H * 4;
    let frames: Vec<(RgbaImage, u16)> = output
        .stdout
        .chunks_exact(frame_len)
        .filter_map(|chunk| {
            RgbaImage::from_raw(W as u32, H as u32, chunk.to_vec()).map(|img| (img, 83))
        })
        .collect();
    if frames.is_empty() {
        Err(format!("{}: no video frames decoded", path.display()))
    } else {
        Ok(frames)
    }
}

/// Scale + place `img` into a 128×128 RGBA canvas.
pub fn fit_to_screen(img: &RgbaImage, fit: Fit, background: [u8; 4]) -> RgbaImage {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let (sw, sh) = (W as u32, H as u32);

    match fit {
        Fit::Stretch => image::imageops::resize(img, sw, sh, FilterType::Lanczos3),
        Fit::Cover => {
            let scale = f64::max(sw as f64 / w as f64, sh as f64 / h as f64);
            let nw = ((w as f64 * scale).round() as u32).max(sw);
            let nh = ((h as f64 * scale).round() as u32).max(sh);
            let scaled = image::imageops::resize(img, nw, nh, FilterType::Lanczos3);
            let x = (nw - sw) / 2;
            let y = (nh - sh) / 2;
            image::imageops::crop_imm(&scaled, x, y, sw, sh).to_image()
        }
        Fit::Contain => {
            let scale = f64::min(sw as f64 / w as f64, sh as f64 / h as f64).min(1.0);
            let nw = ((w as f64 * scale).round() as u32).clamp(1, sw);
            let nh = ((h as f64 * scale).round() as u32).clamp(1, sh);
            let scaled = image::imageops::resize(img, nw, nh, FilterType::Lanczos3);
            let mut canvas = RgbaImage::from_pixel(sw, sh, image::Rgba(background));
            let x = (sw - nw) / 2;
            let y = (sh - nh) / 2;
            image::imageops::overlay(&mut canvas, &scaled, x as i64, y as i64);
            canvas
        }
    }
}

/// Pack fitted frames into RGB565 little-endian.
/// Pack fitted frames into RGB565 little-endian.
#[allow(dead_code)]
pub fn to_rgb565(frames: &[SourceFrame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .map(|frame| {
            let mut out = Vec::with_capacity(FRAME_BYTES);
            ak820_proto::from_rgba(frame.image.as_raw(), &mut out);
            out
        })
        .collect()
}

/// Parse `#RRGGBB` / `#RGB` into RGBA.
#[allow(dead_code)]
pub fn parse_colour(value: &str) -> Result<[u8; 4], String> {
    let hex = value.trim().trim_start_matches('#');
    let expand = |s: &str| u8::from_str_radix(s, 16).map_err(|_| format!("bad colour {value:?}"));
    match hex.len() {
        3 => Ok([
            expand(&hex[0..1])? * 17,
            expand(&hex[1..2])? * 17,
            expand(&hex[2..3])? * 17,
            255,
        ]),
        6 => Ok([
            expand(&hex[0..2])?,
            expand(&hex[2..4])?,
            expand(&hex[4..6])?,
            255,
        ]),
        _ => Err(format!("bad colour {value:?}, expected #RRGGBB")),
    }
}
