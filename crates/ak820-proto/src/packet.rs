//! Packet framing: the 8-byte host header, chunking and reply parsing.

use crate::Error;

/// Bytes of protocol header in every packet.
pub const HEADER_LEN: usize = 8;
/// Magic byte of a host → device packet.
pub const MAGIC_HOST: u8 = 0xAA;
/// Magic byte of a device → host reply.
pub const MAGIC_DEVICE: u8 = 0x55;

/// Command bytes used by the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Command {
    /// `GET_DEVICE_INFO`, 48 bytes, config interface.
    Info = 0x10,
    /// `SET_TEMPORARY_COMMAND_DATA` (clock / stats), config interface.
    Temp = 0x34,
    /// Reply the device sends after each `0x50` chunk.
    AnimAck = 0x41,
    /// `SET_TFT_USER_ANIMATION`, bulk interface.
    TftAnim = 0x50,
    /// `SET_TFT_BUILT_IN_INDEX`, bulk interface.
    TftBuiltin = 0x51,
}

impl Command {
    /// The raw command byte.
    pub const fn byte(self) -> u8 {
        self as u8
    }

    /// Human-readable name for logs and errors.
    pub const fn name(self) -> &'static str {
        match self {
            Command::Info => "GET_DEVICE_INFO",
            Command::Temp => "SET_TEMPORARY_COMMAND_DATA",
            Command::AnimAck => "ANIM_ACK",
            Command::TftAnim => "SET_TFT_USER_ANIMATION",
            Command::TftBuiltin => "SET_TFT_BUILT_IN_INDEX",
        }
    }
}

/// Build a standard `kX`/`AX` packet.
///
/// `AA cmd len addr_lo addr_hi 00 last 00 <chunk>`, zero-padded to `report_len`.
///
/// `len` is the announced payload length of this chunk, which is *not* always
/// `chunk.len()`: `GET_DEVICE_INFO` announces 48 bytes while sending none.
pub fn command_packet(
    cmd: u8,
    addr: usize,
    len: usize,
    chunk: &[u8],
    report_len: usize,
    last: bool,
) -> Result<Vec<u8>, Error> {
    if report_len <= HEADER_LEN {
        return Err(Error::ReportTooSmall(report_len));
    }
    let mut packet = vec![0u8; report_len];
    packet[0] = MAGIC_HOST;
    packet[1] = cmd;
    packet[2] = len as u8;
    packet[3] = (addr & 0xFF) as u8;
    packet[4] = (addr >> 8) as u8;
    packet[5] = 0;
    packet[6] = u8::from(last);
    packet[7] = 0;
    packet[HEADER_LEN..HEADER_LEN + chunk.len()].copy_from_slice(chunk);
    Ok(packet)
}

/// Build one `0x50` animation chunk.
///
/// Custom header: `AA 50 idx_lo idx_hi total_lo total_hi 50 06`.
pub fn upload_packet(
    idx: u16,
    total: u16,
    chunk: &[u8],
    report_len: usize,
) -> Result<Vec<u8>, Error> {
    if report_len <= HEADER_LEN {
        return Err(Error::ReportTooSmall(report_len));
    }
    let mut packet = vec![0u8; report_len];
    packet[0] = MAGIC_HOST;
    packet[1] = Command::TftAnim.byte();
    packet[2] = idx as u8;
    packet[3] = (idx >> 8) as u8;
    packet[4] = total as u8;
    packet[5] = (total >> 8) as u8;
    packet[6] = 0x50;
    packet[7] = 0x06;
    packet[HEADER_LEN..HEADER_LEN + chunk.len()].copy_from_slice(chunk);
    Ok(packet)
}

/// A parsed device reply (`55 cmd len/type addr_lo addr_hi … data`).
#[derive(Debug, PartialEq, Eq)]
pub struct Reply<'a> {
    /// Reply command byte.
    pub cmd: u8,
    /// `len`/type byte.
    pub len: u8,
    /// Address field from bytes 3–4.
    pub addr: u16,
    /// Payload bytes (everything from offset 8).
    pub data: &'a [u8],
}

impl<'a> Reply<'a> {
    /// Parse a raw read buffer; returns `None` if it is not a valid reply.
    pub fn parse(buf: &'a [u8]) -> Option<Self> {
        if buf.len() < HEADER_LEN || buf[0] != MAGIC_DEVICE {
            return None;
        }
        Some(Reply {
            cmd: buf[1],
            len: buf[2],
            addr: u16::from(buf[3]) | (u16::from(buf[4]) << 8),
            data: &buf[HEADER_LEN..],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_header() {
        let pkt = command_packet(Command::Info.byte(), 0, 48, &[], 64, true).unwrap();
        assert_eq!(pkt.len(), 64);
        assert_eq!(&pkt[..8], &[0xAA, 0x10, 0x30, 0x00, 0x00, 0x00, 0x01, 0x00]);
        assert!(pkt[8..].iter().all(|&b| b == 0));
    }

    #[test]
    fn upload_header() {
        let pkt = upload_packet(2, 7, &[1, 2, 3], 4104).unwrap();
        assert_eq!(&pkt[..8], &[0xAA, 0x50, 0x02, 0x00, 0x07, 0x00, 0x50, 0x06]);
        assert_eq!(&pkt[8..11], &[1, 2, 3]);
        assert_eq!(pkt[11], 0, "rest is zero-padded");
    }

    #[test]
    fn reply_parsing() {
        let buf = [0x55, 0x10, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 1, 2, 3];
        let reply = Reply::parse(&buf).unwrap();
        assert_eq!(reply.cmd, 0x10);
        assert_eq!(reply.len, 0x30);
        assert_eq!(reply.addr, 0);
        assert_eq!(reply.data, &[1, 2, 3]);
        assert!(Reply::parse(&[0xAA, 0x10]).is_none());
        assert!(Reply::parse(&[0xAA, 0x10, 0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn tiny_reports_rejected() {
        assert_eq!(
            command_packet(0x10, 0, 0, &[], 8, false).unwrap_err(),
            Error::ReportTooSmall(8)
        );
        assert_eq!(
            upload_packet(0, 1, &[], 4).unwrap_err(),
            Error::ReportTooSmall(4)
        );
    }
}
