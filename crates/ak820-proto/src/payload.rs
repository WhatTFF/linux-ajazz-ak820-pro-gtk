//! The `0x50` animation payload: 256-byte header + N frames of RGB565.

use crate::{Error, FRAME_BYTES, MAX_FRAMES, PAYLOAD_HEADER};

/// A borrowed frame, ready to be packed.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// Exactly [`crate::FRAME_BYTES`] bytes of RGB565, little-endian (low byte first).
    pub pixels: &'a [u8],
    /// How long this frame stays on screen.
    pub delay_ms: u16,
}

/// An owned frame, produced by [`parse_payload`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameOwned {
    /// RGB565 little-endian pixels.
    pub pixels: Vec<u8>,
    /// Delay reconstructed from the header (the last frame's delay is not stored).
    pub delay_ms: u16,
}

/// Size of a payload holding `frames` frames.
pub const fn payload_len(frames: usize) -> usize {
    PAYLOAD_HEADER + frames * FRAME_BYTES
}

/// Pack frames into the exact byte stream the keyboard expects.
///
/// Header layout (rest filled with `0xFF`):
/// `[0] = N`, `[1..N-1] = delay/2 ms of frames 0..N-2`, `[N] = 0`.
pub fn build_payload(frames: &[Frame<'_>]) -> Result<Vec<u8>, Error> {
    if frames.is_empty() {
        return Err(Error::EmptyAnimation);
    }
    if frames.len() > MAX_FRAMES {
        return Err(Error::TooManyFrames(frames.len()));
    }
    for (index, frame) in frames.iter().enumerate() {
        if frame.pixels.len() != FRAME_BYTES {
            return Err(Error::BadFrameSize {
                index,
                got: frame.pixels.len(),
            });
        }
    }

    let n = frames.len();
    let mut out = vec![0xFFu8; payload_len(n)];
    out[0] = n as u8;
    for (k, frame) in frames[..n - 1].iter().enumerate() {
        out[k + 1] = delay_units(frame.delay_ms);
    }
    out[n] = 0;

    let mut at = PAYLOAD_HEADER;
    for frame in frames {
        out[at..at + FRAME_BYTES].copy_from_slice(frame.pixels);
        at += FRAME_BYTES;
    }
    Ok(out)
}

/// Inverse of [`build_payload`], for history thumbnails and `history export`.
///
/// The last frame's delay is not stored by the protocol, so it is reconstructed the
/// same way the upstream web app does: previous delay, or 100 ms when that is zero.
pub fn parse_payload(payload: &[u8], limit: usize) -> Result<Vec<FrameOwned>, Error> {
    if payload.len() < PAYLOAD_HEADER {
        return Err(Error::PayloadTooShort(payload.len()));
    }
    let declared = payload[0] as usize;
    if declared == 0 {
        return Err(Error::ZeroFrames);
    }
    let available = (payload.len() - PAYLOAD_HEADER) / FRAME_BYTES;
    if declared > available {
        return Err(Error::Truncated {
            declared,
            available,
        });
    }

    let n = declared.min(limit.max(1));
    let mut frames = Vec::with_capacity(n);
    for k in 0..n {
        let at = PAYLOAD_HEADER + k * FRAME_BYTES;
        let pixels = payload[at..at + FRAME_BYTES].to_vec();
        let delay_ms = if declared == 1 {
            500
        } else {
            let raw = if k < declared - 1 {
                payload[k + 1]
            } else {
                let stored = payload[declared - 1];
                if stored == 0 { 50 } else { stored }
            };
            u16::from(raw) * 2
        };
        frames.push(FrameOwned { pixels, delay_ms });
    }
    Ok(frames)
}

/// Milliseconds → the protocol's 2 ms units, clamped to `1..=255`.
const fn delay_units(delay_ms: u16) -> u8 {
    let units = (delay_ms as u32).div_ceil(2);
    if units < 1 {
        1
    } else if units > 255 {
        255
    } else {
        units as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Solid frame; the delay is passed at the call site, not baked in here.
    fn frame(fill: u8, _delay_ms: u16) -> Vec<u8> {
        vec![fill; FRAME_BYTES]
    }

    #[test]
    fn header_layout_matches_protocol() {
        let f0 = frame(0x11, 60);
        let f1 = frame(0x22, 100);
        let f2 = frame(0x33, 40);
        let payload = build_payload(&[
            Frame {
                pixels: &f0,
                delay_ms: 60,
            },
            Frame {
                pixels: &f1,
                delay_ms: 100,
            },
            Frame {
                pixels: &f2,
                delay_ms: 40,
            },
        ])
        .unwrap();

        assert_eq!(payload.len(), payload_len(3));
        assert_eq!(payload[0], 3);
        assert_eq!(&payload[1..3], &[30, 50], "delays are milliseconds / 2");
        assert_eq!(payload[3], 0, "the last frame's delay slot is zeroed");
        assert_eq!(payload[4..PAYLOAD_HEADER], [0xFF; PAYLOAD_HEADER - 4]);
        assert_eq!(payload[PAYLOAD_HEADER], 0x11);
        assert_eq!(payload[PAYLOAD_HEADER + FRAME_BYTES], 0x22);
    }

    #[test]
    fn delays_round_half_up_and_clamp() {
        assert_eq!(delay_units(0), 1);
        assert_eq!(delay_units(1), 1);
        assert_eq!(delay_units(3), 2);
        assert_eq!(delay_units(60), 30);
        assert_eq!(delay_units(510), 255);
        assert_eq!(delay_units(u16::MAX), 255);
    }

    #[test]
    fn round_trip_preserves_pixels() {
        let f0 = frame(0xAB, 80);
        let f1 = frame(0xCD, 120);
        let payload = build_payload(&[
            Frame {
                pixels: &f0,
                delay_ms: 80,
            },
            Frame {
                pixels: &f1,
                delay_ms: 120,
            },
        ])
        .unwrap();
        let parsed = parse_payload(&payload, usize::MAX).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].pixels, f0);
        assert_eq!(parsed[0].delay_ms, 80);
        assert_eq!(parsed[1].pixels, f1);
        assert_eq!(parsed[1].delay_ms, 80, "last delay mirrors the stored one");
    }

    #[test]
    fn single_frame_gets_default_delay() {
        let f = frame(0x7F, 30);
        let payload = build_payload(&[Frame {
            pixels: &f,
            delay_ms: 30,
        }])
        .unwrap();
        assert_eq!(payload[0], 1);
        assert_eq!(payload[1], 0, "single frame writes no delay");
        assert_eq!(
            parse_payload(&payload, usize::MAX).unwrap()[0].delay_ms,
            500
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(build_payload(&[]).unwrap_err(), Error::EmptyAnimation);
        let short = vec![0u8; 10];
        assert_eq!(
            build_payload(&[Frame {
                pixels: &short,
                delay_ms: 10
            }])
            .unwrap_err(),
            Error::BadFrameSize { index: 0, got: 10 }
        );
        assert_eq!(
            parse_payload(&[0u8; 100], 1).unwrap_err(),
            Error::PayloadTooShort(100)
        );
        assert_eq!(
            parse_payload(&[0u8; 256], 1).unwrap_err(),
            Error::ZeroFrames
        );

        let mut truncated = vec![0u8; PAYLOAD_HEADER + FRAME_BYTES];
        truncated[0] = 3;
        assert_eq!(
            parse_payload(&truncated, 1).unwrap_err(),
            Error::Truncated {
                declared: 3,
                available: 1
            }
        );
    }

    #[test]
    fn too_many_frames_rejected() {
        let f = frame(0, 10);
        let frames = vec![
            Frame {
                pixels: &f,
                delay_ms: 10
            };
            MAX_FRAMES + 1
        ];
        assert_eq!(
            build_payload(&frames).unwrap_err(),
            Error::TooManyFrames(256)
        );
    }
}
