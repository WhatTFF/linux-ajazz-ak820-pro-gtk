//! Decoding an image or animated GIF into 128×128 frames.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use ak820_proto::{FRAME_BYTES, H, W};
use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, RgbaImage};

/// One decoded frame, already fitted to the screen.
pub struct SourceFrame {
    /// 128×128 RGBA8.
    pub image: RgbaImage,
    /// Delay in milliseconds.
    pub delay_ms: u16,
}

/// How to fit the source into the square screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Fit {
    /// Fill the screen, cropping the overflow (default, like the web app).
    Cover,
    /// Show the whole image, letterboxed on the background colour.
    Contain,
    /// Ignore the aspect ratio.
    Stretch,
}

/// Load `path` into fitted frames. Animated GIFs keep their per-frame delays.
pub fn load(
    path: &Path,
    fit: Fit,
    background: [u8; 4],
    force_delay: Option<u16>,
) -> Result<Vec<SourceFrame>, String> {
    let is_gif = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"));

    let raw: Vec<(RgbaImage, u16)> = if is_gif {
        decode_gif(path)?
    } else {
        let img = image::ImageReader::open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .with_guessed_format()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .decode()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .to_rgba8();
        vec![(img, 100)]
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

fn decode_gif(path: &Path) -> Result<Vec<(RgbaImage, u16)>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let decoder =
        GifDecoder::new(BufReader::new(file)).map_err(|e| format!("{}: {e}", path.display()))?;
    let frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|e| format!("{}: {e}", path.display()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(parse_colour("#ff0000").unwrap(), [255, 0, 0, 255]);
        assert_eq!(parse_colour("00ff00").unwrap(), [0, 255, 0, 255]);
        assert_eq!(parse_colour("#abc").unwrap(), [0xAA, 0xBB, 0xCC, 255]);
        assert!(parse_colour("#12345").is_err());
    }

    #[test]
    fn every_fit_produces_a_128_square() {
        let src = RgbaImage::from_pixel(300, 100, image::Rgba([200, 10, 10, 255]));
        for fit in [Fit::Cover, Fit::Contain, Fit::Stretch] {
            let out = fit_to_screen(&src, fit, [0, 0, 0, 255]);
            assert_eq!((out.width(), out.height()), (128, 128), "{fit:?}");
        }
    }

    #[test]
    fn contain_letterboxes_with_background() {
        let src = RgbaImage::from_pixel(128, 64, image::Rgba([255, 0, 0, 255]));
        let out = fit_to_screen(&src, Fit::Contain, [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 255, 255], "top is background");
        assert_eq!(
            out.get_pixel(64, 64).0,
            [255, 0, 0, 255],
            "middle is the image"
        );
    }

    #[test]
    fn rgb565_output_is_frame_sized() {
        let frames = vec![SourceFrame {
            image: RgbaImage::from_pixel(128, 128, image::Rgba([255, 255, 255, 255])),
            delay_ms: 100,
        }];
        let packed = to_rgb565(&frames);
        assert_eq!(packed[0].len(), FRAME_BYTES);
        assert!(packed[0].iter().all(|&b| b == 0xFF));
    }
}
