//! AJAZZ / Epomaker AK820 Pro TFT screen protocol.
//!
//! Pure, I/O-free implementation of the wire format documented in `PROTOCOL.md`:
//! command framing, the `0x50` animation payload, RGB565 conversion, device-info
//! parsing and clock encoding.
//!
//! Nothing here allocates per pixel or touches a device; `ak820-device` does the I/O.

#![forbid(unsafe_code)]

mod clock;
mod error;
mod info;
mod packet;
mod payload;
mod rgb565;

pub use clock::{CivilTime, clock_payload, weekday_from_ymd};
pub use error::Error;
pub use info::{DeviceInfo, parse_info};
pub use packet::{
    Command, HEADER_LEN, MAGIC_DEVICE, MAGIC_HOST, Reply, command_packet, upload_packet,
};
pub use payload::{Frame, FrameOwned, build_payload, parse_payload, payload_len};
pub use rgb565::{from_rgba, to_rgba};

/// USB vendor id (Sonix).
pub const VID: u16 = 0x0c45;
/// Known product ids: `0x800a` (newer AK820 Pro) and `0x8009` (older AK820).
pub const PIDS: [u16; 2] = [0x800a, 0x8009];
/// 2.4 GHz dongle — answers `GET_DEVICE_INFO` but cannot reach the screen.
pub const DONGLE_PID: u16 = 0xfefe;

/// Usage pages that expose the 32/64-byte config interface.
pub const CONFIG_PAGES: [u16; 6] = [0xff68, 0xff80, 0xff60, 0xff00, 0xff01, 0xff1b];
/// Usage page of the 4104-byte bulk interface used for animation uploads.
pub const BULK_PAGE: u16 = 0xff67;

/// Screen width in pixels.
pub const W: usize = 128;
/// Screen height in pixels.
pub const H: usize = 128;
/// Bytes per frame on the wire: 128 × 128 × RGB565.
pub const FRAME_BYTES: usize = W * H * 2;
/// The animation header is a fixed 256-byte block.
pub const PAYLOAD_HEADER: usize = 256;
/// Hard limit imposed by the one-byte frame counter in the header.
pub const MAX_FRAMES: usize = 255;
/// The screen cannot redraw faster than roughly this; shorter delays are clamped.
pub const MIN_FRAME_MS: u16 = 60;

/// Fallback output-report sizes when the HID report descriptor is unavailable.
pub const FALLBACK_CONFIG_REPORT: usize = 32;
/// Fallback output-report size of the bulk interface.
pub const FALLBACK_BULK_REPORT: usize = 4104;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_consistent() {
        assert_eq!(FRAME_BYTES, 32768);
        assert_eq!(payload_len(1), PAYLOAD_HEADER + FRAME_BYTES);
        assert_eq!(
            payload_len(MAX_FRAMES),
            PAYLOAD_HEADER + MAX_FRAMES * FRAME_BYTES
        );
    }
}
