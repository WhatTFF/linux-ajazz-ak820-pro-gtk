use std::fmt;

/// Transport-level failures.
#[derive(Debug)]
pub enum Error {
    /// OS error while opening, reading or writing a hidraw node.
    Io(std::io::Error),
    /// No AK820 screen interface is present (wrong mode, unplugged, dongle only).
    NotFound,
    /// The node exists but the user cannot open it (missing udev rule).
    PermissionDenied(String),
    /// The bulk (0xFF67) interface is missing.
    NoBulkInterface,
    /// The config interface is missing, so clock and device info are unavailable.
    NoConfigInterface,
    /// The device never answered.
    Timeout { cmd: u8, attempts: u32 },
    /// Invalid protocol data.
    Protocol(ak820_proto::Error),
    /// Another instance holds the device lock.
    Locked(String),
    /// The caller asked to stop an upload.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NotFound => f.write_str(
                "AK820 Pro screen not found on USB — use the USB-C cable in wired mode \
                 (the 2.4 GHz dongle and Bluetooth cannot reach the screen)",
            ),
            Error::PermissionDenied(path) => write!(
                f,
                "no permission on {path} — install udev/70-ak820.rules and replug the keyboard"
            ),
            Error::NoBulkInterface => f.write_str("bulk (0xFF67) interface not found"),
            Error::NoConfigInterface => f.write_str("config interface not found"),
            Error::Timeout { cmd, attempts } => {
                write!(
                    f,
                    "no reply to command 0x{cmd:02x} after {attempts} attempts"
                )
            }
            Error::Protocol(e) => write!(f, "{e}"),
            Error::Locked(path) => {
                write!(
                    f,
                    "another AK820 instance is using the keyboard (lock: {path})"
                )
            }
            Error::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Protocol(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

impl From<ak820_proto::Error> for Error {
    fn from(value: ak820_proto::Error) -> Self {
        Error::Protocol(value)
    }
}
