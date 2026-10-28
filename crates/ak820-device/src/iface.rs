//! One open hidraw interface: framing, write, wait-for-reply, retries.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ak820_proto::{HEADER_LEN, MAGIC_DEVICE, Reply, command_packet};

use crate::{Error, IfaceInfo, sys};

const READ_BUF: usize = 8192;

/// An opened hidraw node.
pub struct Iface {
    path: PathBuf,
    report_len: usize,
    file: File,
}

impl Iface {
    /// Open a node discovered by [`crate::find`].
    pub fn open(info: &IfaceInfo) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&info.path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    Error::PermissionDenied(info.path.display().to_string())
                }
                _ => Error::Io(e),
            })?;
        Ok(Iface {
            path: info.path.clone(),
            report_len: info.report_len,
            file,
        })
    }

    /// `/dev/hidrawN` of this interface.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Output report size in bytes.
    pub fn report_len(&self) -> usize {
        self.report_len
    }

    /// Discard anything the device queued before we sent a command.
    pub fn drain(&mut self) {
        let mut buf = [0u8; READ_BUF];
        loop {
            match sys::poll_readable(self.file.as_raw_fd(), Duration::ZERO) {
                Ok(true) => {}
                _ => return,
            }
            if self.read_into(&mut buf).is_err() {
                return;
            }
        }
    }

    /// Write one packet, prefixed with the report id (0) that hidraw expects.
    fn write_packet(&mut self, packet: &[u8]) -> Result<(), Error> {
        let mut buf = Vec::with_capacity(packet.len() + 1);
        buf.push(0);
        buf.extend_from_slice(packet);
        self.file.write_all(&buf)?;
        Ok(())
    }

    fn read_into(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.file.read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Wait up to `timeout` for a reply whose command byte matches `cmd`.
    pub fn wait(&mut self, cmd: u8, timeout: Duration) -> Result<Option<Vec<u8>>, Error> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; READ_BUF];
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            if !sys::poll_readable(self.file.as_raw_fd(), deadline - now)? {
                continue;
            }
            let n = self.read_into(&mut buf)?;
            let reply = &buf[..n];
            if reply.len() > 1 && reply[0] == MAGIC_DEVICE && reply[1] == cmd {
                return Ok(Some(reply.to_vec()));
            }
        }
    }

    /// Write `packet` and optionally wait for a reply, retrying on timeout.
    pub fn transact(
        &mut self,
        packet: &[u8],
        reply_cmd: Option<u8>,
        timeout: Duration,
        retries: u32,
    ) -> Result<Option<Vec<u8>>, Error> {
        for _ in 0..=retries {
            self.drain();
            self.write_packet(packet)?;
            let Some(cmd) = reply_cmd else {
                return Ok(None);
            };
            if let Some(reply) = self.wait(cmd, timeout)? {
                return Ok(Some(reply));
            }
        }
        Err(Error::Timeout {
            cmd: reply_cmd.unwrap_or(0),
            attempts: retries + 1,
        })
    }

    /// Send a chunked command and collect the reply payloads.
    ///
    /// Mirrors the upstream transport: `content_size` may exceed `data.len()`
    /// (device info is requested as 48 bytes with an empty payload).
    pub fn command(
        &mut self,
        cmd: u8,
        data: &[u8],
        content_size: usize,
        timeout: Duration,
        retries: u32,
    ) -> Result<Vec<Vec<u8>>, Error> {
        if self.report_len <= HEADER_LEN {
            return Err(Error::Protocol(ak820_proto::Error::ReportTooSmall(
                self.report_len,
            )));
        }
        let per = self.report_len - HEADER_LEN;
        let total = content_size.div_ceil(per).max(1);
        let reply_cmd = (retries > 0).then_some(cmd);
        let mut replies = Vec::with_capacity(total);

        for n in 0..total {
            let addr = n * per;
            let start = addr.min(data.len());
            let end = (addr + per).min(data.len());
            let announced = (content_size.saturating_sub(addr)).min(per);
            let packet = command_packet(
                cmd,
                addr,
                announced,
                &data[start..end],
                self.report_len,
                n + 1 == total,
            )?;
            if let Some(reply) = self.transact(&packet, reply_cmd, timeout, retries)? {
                replies.push(reply);
            }
        }
        Ok(replies)
    }

    /// Payload bytes of a reply, skipping its 8-byte header.
    pub fn reply_data(reply: &[u8]) -> &[u8] {
        Reply::parse(reply).map(|r| r.data).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_data_skips_header() {
        let reply = [0x55, 0x10, 0x02, 0x00, 0x00, 0, 0, 0, 0xAA, 0xBB];
        assert_eq!(Iface::reply_data(&reply), &[0xAA, 0xBB]);
        assert!(Iface::reply_data(&[0x00]).is_empty());
    }
}
