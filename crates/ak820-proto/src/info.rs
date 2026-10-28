//! `GET_DEVICE_INFO` reply decoding.

use crate::Error;

/// Decoded 48-byte device info block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// USB vendor id.
    pub vid: u16,
    /// USB product id.
    pub pid: u16,
    /// Firmware version in hundredths (`107` → `1.07`).
    pub firmware_hundredths: u16,
    /// 0 = wired, 2 = 2.4 GHz dongle (per the vendor driver).
    pub work_mode: u8,
    /// Battery percentage.
    pub battery: u8,
    /// Charging flag.
    pub charging: bool,
    /// Active profile index.
    pub profile: u8,
    /// Maximum number of animation frames the firmware accepts.
    pub max_frames: u16,
    /// Screen direction; `0xFF` means the device has no screen.
    pub tft_direction: u8,
    /// `1` means the firmware wants 2 s timeouts.
    pub frame_version: u8,
}

impl DeviceInfo {
    /// `true` when the device reports a usable TFT screen.
    pub const fn has_screen(&self) -> bool {
        self.tft_direction != 0xFF
    }
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04x}:{:04x} fw {}.{:02} mode {} battery {}%{} profile {} frames {}",
            self.vid,
            self.pid,
            self.firmware_hundredths / 100,
            self.firmware_hundredths % 100,
            self.work_mode,
            self.battery,
            if self.charging { " charging" } else { "" },
            self.profile,
            self.max_frames
        )
    }
}

/// Parse the concatenated `data` payloads of the two `GET_DEVICE_INFO` replies.
pub fn parse_info(data: &[u8]) -> Result<DeviceInfo, Error> {
    // Offsets 22..23 and 28/30 are read, so 31 bytes are required.
    if data.len() < 31 {
        return Err(Error::PayloadTooShort(data.len()));
    }
    let firmware_hundredths =
        u16::from(data[8] & 0x0F) + u16::from(data[8] >> 4) * 10 + u16::from(data[9]) * 100;
    Ok(DeviceInfo {
        vid: u16::from(data[4]) | (u16::from(data[5]) << 8),
        pid: u16::from(data[6]) | (u16::from(data[7]) << 8),
        firmware_hundredths,
        work_mode: data[16],
        battery: data[17],
        charging: data[18] != 0,
        profile: data[19],
        max_frames: u16::from(data[22]) | (u16::from(data[23]) << 8),
        tft_direction: data[28],
        frame_version: data[30],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes captured from a real AK820 Pro (fw 1.07, PID 0x8009).
    const SAMPLE: [u8; 31] = {
        let mut d = [0u8; 31];
        d[4] = 0x45;
        d[5] = 0x0C;
        d[6] = 0x09;
        d[7] = 0x80;
        d[8] = 0x07;
        d[9] = 0x01;
        d[16] = 0x00;
        d[17] = 100;
        d[18] = 1;
        d[19] = 0;
        d[22] = 0xFF;
        d[23] = 0x00;
        d[28] = 0x00;
        d[30] = 0x00;
        d
    };

    #[test]
    fn decodes_captured_block() {
        let info = parse_info(&SAMPLE).unwrap();
        assert_eq!(info.vid, 0x0c45);
        assert_eq!(info.pid, 0x8009);
        assert_eq!(info.firmware_hundredths, 107);
        assert_eq!(info.max_frames, 255);
        assert_eq!(info.battery, 100);
        assert!(info.charging);
        assert!(info.has_screen());
        assert_eq!(
            info.to_string(),
            "0c45:8009 fw 1.07 mode 0 battery 100% charging profile 0 frames 255"
        );
    }

    #[test]
    fn no_screen_is_detected() {
        let mut d = SAMPLE;
        d[28] = 0xFF;
        assert!(!parse_info(&d).unwrap().has_screen());
    }

    #[test]
    fn firmware_bcd_decoding() {
        let mut d = SAMPLE;
        d[8] = 0x16;
        d[9] = 0x01;
        assert_eq!(parse_info(&d).unwrap().firmware_hundredths, 116);
    }

    #[test]
    fn short_input_rejected() {
        assert_eq!(
            parse_info(&[0u8; 30]).unwrap_err(),
            Error::PayloadTooShort(30)
        );
    }
}
