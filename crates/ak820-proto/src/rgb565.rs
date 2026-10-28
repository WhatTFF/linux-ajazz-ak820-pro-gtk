//! RGB ↔ RGB565 conversion, byte-compatible with the upstream JavaScript.

/// Convert RGBA8 pixels to RGB565 little-endian, appending to `out`.
///
/// Same bit truncation as `ak820.js:buildPayload`: `(r & 0xf8) << 8 | (g & 0xfc) << 3 | b >> 3`.
pub fn from_rgba(rgba: &[u8], out: &mut Vec<u8>) {
    out.reserve(rgba.len() / 4 * 2);
    for px in rgba.chunks_exact(4) {
        let v = ((u16::from(px[0]) & 0xf8) << 8)
            | ((u16::from(px[1]) & 0xfc) << 3)
            | (u16::from(px[2]) >> 3);
        out.push(v as u8);
        out.push((v >> 8) as u8);
    }
}

/// Convert RGB565 little-endian pixels back to RGBA8, appending to `out`.
///
/// Same expansion as `ak820.js:parsePayload` (bit replication would be more exact, but
/// matching upstream keeps history thumbnails identical to the website's).
pub fn to_rgba(pixels: &[u8], out: &mut Vec<u8>) {
    out.reserve(pixels.len() / 2 * 4);
    for px in pixels.chunks_exact(2) {
        let v = u16::from(px[0]) | (u16::from(px[1]) << 8);
        let r = (v >> 11) & 31;
        let g = (v >> 5) & 63;
        let b = v & 31;
        out.push((r * 255 / 31) as u8);
        out.push((g * 255 / 63) as u8);
        out.push((b * 255 / 31) as u8);
        out.push(255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(r: u8, g: u8, b: u8) -> [u8; 2] {
        let mut out = Vec::new();
        from_rgba(&[r, g, b, 255], &mut out);
        [out[0], out[1]]
    }

    #[test]
    fn known_colours() {
        assert_eq!(encode(0, 0, 0), [0x00, 0x00]);
        assert_eq!(encode(255, 255, 255), [0xFF, 0xFF]);
        assert_eq!(encode(255, 0, 0), [0x00, 0xF8]);
        assert_eq!(encode(0, 255, 0), [0xE0, 0x07]);
        assert_eq!(encode(0, 0, 255), [0x1F, 0x00]);
    }

    #[test]
    fn little_endian_byte_order() {
        let mut out = Vec::new();
        from_rgba(&[255, 0, 0, 255], &mut out);
        assert_eq!(out, [0x00, 0xF8], "low byte first");
    }

    #[test]
    fn round_trip_is_stable_for_565_values() {
        let mut encoded = Vec::new();
        from_rgba(&[18, 52, 86, 255], &mut encoded);
        let mut rgba = Vec::new();
        to_rgba(&encoded, &mut rgba);
        let mut again = Vec::new();
        from_rgba(&rgba, &mut again);
        assert_eq!(encoded, again);
        assert_eq!(rgba[3], 255);
    }
}
