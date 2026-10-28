//! Direct `hidraw` transport for the AJAZZ / Epomaker AK820 Pro screen.
//!
//! No `libhidapi`, no WebHID: interfaces are found through `/sys/class/hidraw`,
//! their output-report size is read from the HID report descriptor (firmware
//! revisions differ — 32 or 64 bytes on the config interface), and packets are
//! written straight to `/dev/hidrawN`.

#![deny(unsafe_code)]

mod discovery;
mod error;
mod iface;
mod lock;
#[allow(unsafe_code)]
mod sys;
pub use discovery::{IfaceInfo, Interfaces, Presence, find, parse_descriptor, presence};
pub use error::Error;
pub use iface::Iface;
pub use lock::DeviceLock;

use std::path::Path;
use std::time::Duration;

use ak820_proto::{
    CivilTime, DeviceInfo, FRAME_BYTES, MAX_FRAMES, PAYLOAD_HEADER, clock_payload, command_packet,
    parse_info, upload_packet,
};

/// Default reply timeout for control commands.
pub const TIMEOUT: Duration = Duration::from_millis(500);
/// Reply timeout for each `0x50` upload chunk (firmware 1.16 needs the longer one).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_millis(2000);
/// How many times a packet is retried before giving up.
pub const RETRIES: u32 = 3;

/// Node path and output-report size of one interface, as reported by
/// [`Device::interfaces`].
pub type InterfaceSummary<'a> = (&'a Path, usize);

/// An opened keyboard, with whichever interfaces are present.
pub struct Device {
    config: Option<Iface>,
    bulk: Option<Iface>,
}

impl Device {
    /// Find and open the keyboard. Fails if no AK820 screen interface is present.
    pub fn open() -> Result<Self, Error> {
        let interfaces = find();
        let config = match &interfaces.config {
            Some(info) => Some(Iface::open(info)?),
            None => None,
        };
        let bulk = match &interfaces.bulk {
            Some(info) => Some(Iface::open(info)?),
            None => None,
        };
        if config.is_none() && bulk.is_none() {
            return Err(Error::NotFound);
        }
        Ok(Device { config, bulk })
    }

    /// Open without a config interface (upload only).
    pub fn open_bulk() -> Result<Self, Error> {
        let interfaces = find();
        let bulk = match &interfaces.bulk {
            Some(info) => Some(Iface::open(info)?),
            None => return Err(Error::NoBulkInterface),
        };
        Ok(Device { config: None, bulk })
    }

    /// Paths and report sizes of the interfaces in use, for `ak820ctl info`.
    pub fn interfaces(&self) -> (Option<InterfaceSummary<'_>>, Option<InterfaceSummary<'_>>) {
        (
            self.config.as_ref().map(|i| (i.path(), i.report_len())),
            self.bulk.as_ref().map(|i| (i.path(), i.report_len())),
        )
    }

    /// Whether the config interface (clock, device info) is available.
    pub const fn has_config(&self) -> bool {
        self.config.is_some()
    }

    /// Read `GET_DEVICE_INFO`.
    pub fn info(&mut self) -> Result<DeviceInfo, Error> {
        let config = self.config.as_mut().ok_or(Error::NoConfigInterface)?;
        let replies =
            config.command(ak820_proto::Command::Info.byte(), &[], 48, TIMEOUT, RETRIES)?;
        let data: Vec<u8> = replies
            .iter()
            .flat_map(|reply| Iface::reply_data(reply).iter().copied())
            .take(48)
            .collect();
        Ok(parse_info(&data)?)
    }

    /// Push the current local time to the screen clock.
    pub fn sync_clock(&mut self, time: CivilTime) -> Result<(), Error> {
        let config = self.config.as_mut().ok_or(Error::NoConfigInterface)?;
        config.command(
            ak820_proto::Command::Temp.byte(),
            &clock_payload(time),
            24,
            TIMEOUT,
            RETRIES,
        )?;
        Ok(())
    }

    /// Switch to one of the firmware's built-in animations.
    pub fn show_builtin(&mut self, index: u8) -> Result<(), Error> {
        let bulk = self.bulk.as_mut().ok_or(Error::NoBulkInterface)?;
        let packet = command_packet(
            ak820_proto::Command::TftBuiltin.byte(),
            0,
            1,
            &[index],
            bulk.report_len(),
            true,
        )?;
        bulk.transact(&packet, None, TIMEOUT, 0)?;
        Ok(())
    }

    /// Upload a packed `0x50` payload (see [`ak820_proto::build_payload`]).
    ///
    /// `progress` is called after every acknowledged chunk with `(done, total)`.
    pub fn upload(
        &mut self,
        payload: &[u8],
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<(), Error> {
        self.upload_with(payload, &mut |done, total| {
            progress(done, total);
            true
        })
    }

    /// Upload with cancellation: `progress` returns `false` to stop after the current chunk.
    pub fn upload_with(
        &mut self,
        payload: &[u8],
        progress: &mut dyn FnMut(usize, usize) -> bool,
    ) -> Result<(), Error> {
        if payload.len() < PAYLOAD_HEADER || payload.len() < PAYLOAD_HEADER + FRAME_BYTES {
            return Err(Error::Protocol(ak820_proto::Error::PayloadTooShort(
                payload.len(),
            )));
        }
        let frames = payload[0] as usize;
        if frames == 0 || frames > MAX_FRAMES {
            return Err(Error::Protocol(ak820_proto::Error::ZeroFrames));
        }
        let expected = PAYLOAD_HEADER + frames * FRAME_BYTES;
        if payload.len() < expected {
            return Err(Error::Protocol(ak820_proto::Error::Truncated {
                declared: frames,
                available: (payload.len() - PAYLOAD_HEADER) / FRAME_BYTES,
            }));
        }

        let bulk = self.bulk.as_mut().ok_or(Error::NoBulkInterface)?;
        let report_len = bulk.report_len();
        let per = report_len - ak820_proto::HEADER_LEN;
        let total = payload.len().div_ceil(per);
        for idx in 0..total {
            let start = idx * per;
            let end = (start + per).min(payload.len());
            let packet = upload_packet(idx as u16, total as u16, &payload[start..end], report_len)?;
            bulk.transact(
                &packet,
                Some(ak820_proto::Command::AnimAck.byte()),
                UPLOAD_TIMEOUT,
                RETRIES,
            )?;
            if !progress(idx + 1, total) {
                return Err(Error::Cancelled);
            }
        }
        Ok(())
    }

    /// Send an arbitrary command, for debugging (`ak820ctl raw`).
    pub fn raw(
        &mut self,
        cmd: u8,
        data: &[u8],
        content_size: usize,
        bulk: bool,
        retries: u32,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let iface = if bulk {
            self.bulk.as_mut().ok_or(Error::NoBulkInterface)?
        } else {
            self.config.as_mut().ok_or(Error::NoConfigInterface)?
        };
        iface.command(cmd, data, content_size, TIMEOUT, retries)
    }
}
