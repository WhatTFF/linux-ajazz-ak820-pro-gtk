//! `ak820ctl` — command line control for the AJAZZ / Epomaker AK820 Pro screen.

mod decode;

use ak820_store::history;

use std::io::Write;
use std::path::{Path, PathBuf};

use ak820_device::{Device, DeviceLock};
use ak820_proto::{CivilTime, Frame, MAX_FRAMES, build_payload, parse_payload};
use chrono::{Datelike, Timelike};
use clap::{Parser, Subcommand};
use decode::Fit;

#[derive(Parser)]
#[command(
    name = "ak820ctl",
    version,
    about = "Put images, GIFs and text on the AK820 Pro screen, and keep its clock in sync",
    after_help = "The screen is only reachable over the USB-C cable in wired mode.\n\
                  Install udev/70-ak820.rules once so the keyboard can be opened without root."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show interfaces, firmware and screen limits
    Info,
    /// Push this computer's clock to the keyboard
    Time {
        /// Print the encoded payload instead of sending it
        #[arg(long)]
        print: bool,
        /// Print nothing on success (for the autosync timer)
        #[arg(long)]
        quiet: bool,
    },
    /// Re-upload the newest history entry (used when the keyboard is plugged in)
    RestoreLatest {
        /// Print nothing on success
        #[arg(long)]
        quiet: bool,
    },
    /// Upload an image or animated GIF
    Upload {
        /// PNG, JPEG, WebP or GIF
        file: PathBuf,
        /// How to fit the image into the 128×128 screen
        #[arg(long, value_enum, default_value = "cover")]
        fit: Fit,
        /// Background colour for --fit contain
        #[arg(long, default_value = "#000000")]
        bg: String,
        /// Force one delay for every frame (milliseconds)
        #[arg(long)]
        delay: Option<u16>,
        /// Lower the frame cap (the device reports its own limit)
        #[arg(long)]
        max_frames: Option<usize>,
        /// Build the payload and report its size without touching the keyboard
        #[arg(long)]
        dry_run: bool,
        /// Do not store this upload in history
        #[arg(long)]
        no_history: bool,
        /// Name shown in `ak820ctl history`
        #[arg(long)]
        name: Option<String>,
    },
    /// Select one of the firmware's built-in animations
    Builtin {
        /// Animation index
        index: u8,
    },
    /// Inspect or restore previous uploads
    History {
        #[command(subcommand)]
        command: HistoryCmd,
    },
    /// Build the raw 0x50 payload for an image (debug, writes a .bin)
    Payload {
        file: PathBuf,
        /// Where to write the payload
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Send an arbitrary command (debug)
    Raw {
        /// Command byte, e.g. 0x10
        cmd: String,
        /// Hex payload, e.g. "5a015a"
        #[arg(long)]
        data: Option<String>,
        /// Payload length to announce (defaults to the data length)
        #[arg(long)]
        size: Option<usize>,
        /// Use the bulk interface
        #[arg(long)]
        bulk: bool,
        /// Retries; 0 sends without waiting for a reply
        #[arg(long, default_value_t = 3)]
        retries: u32,
    },
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// List stored uploads, newest first
    List,
    /// Write a stored upload back out as a GIF
    Export {
        /// Entry number (1 = newest)
        index: usize,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Send a stored upload to the keyboard again
    Restore {
        /// Entry number (1 = newest)
        index: usize,
    },
    /// Delete all stored uploads
    Clear,
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("ak820ctl: {err}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Cmd::Info => cmd_info(),
        Cmd::Time { print, quiet } => cmd_time(print, quiet),
        Cmd::RestoreLatest { quiet } => cmd_restore_latest(quiet),
        Cmd::Upload {
            file,
            fit,
            bg,
            delay,
            max_frames,
            dry_run,
            no_history,
            name,
        } => cmd_upload(
            &file,
            fit,
            &bg,
            delay,
            max_frames,
            dry_run,
            no_history,
            name.as_deref(),
        ),
        Cmd::Builtin { index } => cmd_builtin(index),
        Cmd::History { command } => cmd_history(command),
        Cmd::Payload { file, out } => cmd_payload(&file, &out),
        Cmd::Raw {
            cmd,
            data,
            size,
            bulk,
            retries,
        } => cmd_raw(&cmd, data.as_deref(), size, bulk, retries),
    }
}

// --- commands ---------------------------------------------------------------------

fn cmd_info() -> Result<(), String> {
    let interfaces = ak820_device::find();
    if interfaces.nodes.is_empty() {
        return Err(ak820_device::Error::NotFound.to_string());
    }
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let (config, bulk) = device.interfaces();
    let show = |role: &str, iface: Option<(&std::path::Path, usize)>| match iface {
        Some((path, len)) => println!("{role} iface: {} ({len} B)", path.display()),
        None => println!("{role} iface: none"),
    };
    show("config", config);
    show("bulk  ", bulk);

    let info = device.info().map_err(|e| e.to_string())?;
    let rows: [(&str, String); 10] = [
        ("vid", format!("{:04x}", info.vid)),
        ("pid", format!("{:04x}", info.pid)),
        (
            "firmware",
            format!(
                "{}.{:02}",
                info.firmware_hundredths / 100,
                info.firmware_hundredths % 100
            ),
        ),
        ("workMode", info.work_mode.to_string()),
        ("battery", info.battery.to_string()),
        ("charging", u8::from(info.charging).to_string()),
        ("profile", info.profile.to_string()),
        ("tftMaxFrames", info.max_frames.to_string()),
        ("tftDirection", info.tft_direction.to_string()),
        ("frameVersion", info.frame_version.to_string()),
    ];
    for (key, value) in rows {
        println!("  {key:<13} {value}");
    }
    if !info.has_screen() {
        println!("  warning: this device reports no TFT screen (tftDirection 0xFF)");
    }
    Ok(())
}

fn cmd_time(print: bool, quiet: bool) -> Result<(), String> {
    // The GUI can switch the systemd timer off without touching systemd itself.
    if quiet && !ak820_store::settings::Settings::load().autosync_clock {
        return Ok(());
    }
    let now = chrono::Local::now();
    let time = CivilTime {
        year: now.year() as u16,
        month: now.month() as u8,
        day: now.day() as u8,
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
        weekday: now.weekday().num_days_from_sunday() as u8,
    };
    let payload = ak820_proto::clock_payload(time);
    if print {
        println!(
            "{}",
            payload
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        return Ok(());
    }
    match sync_quiet(time) {
        Ok(()) => {}
        // A missing keyboard is normal for the timer: wireless mode, unplugged, or the
        // GUI holding the lock. Stay silent instead of failing the unit every minute.
        Err(_) if quiet => return Ok(()),
        Err(e) => return Err(e.to_string()),
    }
    if !quiet {
        println!(
            "clock synced to {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year, time.month, time.day, time.hour, time.minute, time.second
        );
    }
    Ok(())
}

fn sync_quiet(time: CivilTime) -> Result<(), ak820_device::Error> {
    let _lock = DeviceLock::acquire()?;
    let mut device = Device::open()?;
    device.sync_clock(time)
}

fn restore_quiet(payload: &[u8]) -> Result<(), ak820_device::Error> {
    let _lock = DeviceLock::acquire()?;
    let mut device = Device::open()?;
    device.upload(payload, &mut |_, _| {})
}

fn cmd_restore_latest(quiet: bool) -> Result<(), String> {
    let settings = ak820_store::settings::Settings::load();
    if !settings.restore_on_connect {
        if !quiet {
            println!("restore_on_connect is disabled; nothing to do");
        }
        return Ok(());
    }
    let payload = history::load(1).map_err(|e| format!("nothing to restore: {e}"))?;
    match restore_quiet(&payload) {
        Ok(()) => {}
        Err(_) if quiet => return Ok(()),
        Err(e) => return Err(e.to_string()),
    }
    if !quiet {
        println!("restored the newest history entry");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_upload(
    file: &Path,
    fit: Fit,
    bg: &str,
    delay: Option<u16>,
    max_frames: Option<usize>,
    dry_run: bool,
    no_history: bool,
    name: Option<&str>,
) -> Result<(), String> {
    let background = decode::parse_colour(bg)?;
    let source = decode::load(file, fit, background, delay)?;
    let delays: Vec<u16> = source.iter().map(|frame| frame.delay_ms).collect();
    let packed = decode::to_rgb565(&source);
    drop(source);

    let label = name.map(str::to_owned).unwrap_or_else(|| {
        file.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string()
    });

    let frames: Vec<Frame<'_>> = packed
        .iter()
        .zip(delays.iter())
        .map(|(pixels, &delay_ms)| Frame { pixels, delay_ms })
        .collect();

    let limit = max_frames.unwrap_or(MAX_FRAMES).min(MAX_FRAMES);
    if frames.len() > limit {
        eprintln!(
            "  {} frames exceeds the limit of {limit}; truncating",
            frames.len()
        );
    }
    let frames = &frames[..frames.len().min(limit)];
    let payload = build_payload(frames).map_err(|e| e.to_string())?;
    eprintln!("  {} frame(s), {} bytes", frames.len(), payload.len());

    if dry_run {
        println!("dry run: {} bytes, {} frames", payload.len(), frames.len());
        return Ok(());
    }

    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let mut last_pct = u32::MAX;
    device
        .upload(&payload, &mut |done, total| {
            let pct = (done as u32 * 100) / total as u32;
            if pct != last_pct {
                last_pct = pct;
                eprint!("\r  uploading {pct:3}%");
                let _ = std::io::stderr().flush();
            }
        })
        .map_err(|e| e.to_string())?;
    eprintln!("\r  uploading 100%");

    if !no_history {
        match history::save(&payload, &label) {
            Ok(path) => eprintln!("  history: {}", path.display()),
            Err(e) => eprintln!("  history: not saved ({e})"),
        }
    }
    println!("done");
    Ok(())
}

fn cmd_builtin(index: u8) -> Result<(), String> {
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    device.show_builtin(index).map_err(|e| e.to_string())?;
    println!("built-in animation {index} selected");
    Ok(())
}

fn cmd_history(command: HistoryCmd) -> Result<(), String> {
    match command {
        HistoryCmd::List => {
            let entries = history::list();
            if entries.is_empty() {
                println!("no uploads in {}", history::dir().display());
                return Ok(());
            }
            for entry in entries {
                println!("{:>2}  {:>9} B  {}", entry.index, entry.bytes, entry.name);
            }
        }
        HistoryCmd::Export { index, out } => {
            let payload = history::load(index)?;
            export_gif(&payload, &out)?;
            println!("wrote {}", out.display());
        }
        HistoryCmd::Restore { index } => {
            let payload = history::load(index)?;
            let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
            let mut device = Device::open().map_err(|e| e.to_string())?;
            device
                .upload(&payload, &mut |_, _| {})
                .map_err(|e| e.to_string())?;
            println!("restored entry {index}");
        }
        HistoryCmd::Clear => {
            let removed = history::clear()?;
            println!("removed {removed} upload(s)");
        }
    }
    Ok(())
}

fn export_gif(payload: &[u8], out: &Path) -> Result<(), String> {
    use image::Delay;
    use image::codecs::gif::{GifEncoder, Repeat};

    let frames = parse_payload(payload, usize::MAX).map_err(|e| e.to_string())?;
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut encoder = GifEncoder::new(file);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|e| e.to_string())?;
    for frame in frames {
        let mut rgba = Vec::with_capacity(128 * 128 * 4);
        ak820_proto::to_rgba(&frame.pixels, &mut rgba);
        let image = image::RgbaImage::from_raw(128, 128, rgba).ok_or("bad frame size")?;
        let delay = Delay::from_numer_denom_ms(frame.delay_ms as u32, 1);
        encoder
            .encode_frame(image::Frame::from_parts(image, 0, 0, delay))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn cmd_payload(file: &Path, out: &Path) -> Result<(), String> {
    let source = decode::load(file, Fit::Cover, [0, 0, 0, 255], None)?;
    let delays: Vec<u16> = source.iter().map(|f| f.delay_ms).collect();
    let packed = decode::to_rgb565(&source);
    let frames: Vec<Frame<'_>> = packed
        .iter()
        .zip(delays.iter())
        .map(|(pixels, &delay_ms)| Frame { pixels, delay_ms })
        .collect();
    let payload = build_payload(&frames).map_err(|e| e.to_string())?;
    std::fs::write(out, &payload).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "{} frames, {} bytes → {}",
        frames.len(),
        payload.len(),
        out.display()
    );
    Ok(())
}

fn cmd_raw(
    cmd: &str,
    data: Option<&str>,
    size: Option<usize>,
    bulk: bool,
    retries: u32,
) -> Result<(), String> {
    let cmd = parse_byte(cmd)?;
    let payload = match data {
        Some(hex) => parse_hex(hex)?,
        None => Vec::new(),
    };
    let content_size = size.unwrap_or(payload.len());

    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let replies = device
        .raw(cmd, &payload, content_size, bulk, retries)
        .map_err(|e| e.to_string())?;
    if replies.is_empty() {
        println!("sent 0x{cmd:02x}, no reply expected");
        return Ok(());
    }
    for reply in replies {
        let hex: String = reply.iter().map(|b| format!("{b:02x}")).collect();
        println!("{hex}");
    }
    Ok(())
}

fn parse_byte(value: &str) -> Result<u8, String> {
    let trimmed = value.trim();
    let trimmed = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    u8::from_str_radix(trimmed, 16).map_err(|_| format!("bad command byte {value:?}"))
}

fn parse_hex(value: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if cleaned.len() % 2 != 0 {
        return Err(format!("hex payload {value:?} has an odd number of digits"));
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|_| format!("bad hex payload {value:?}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_input() {
        assert_eq!(parse_hex("5a015a").unwrap(), [0x5A, 0x01, 0x5A]);
        assert_eq!(parse_hex("5a 01:5a").unwrap(), [0x5A, 0x01, 0x5A]);
        assert!(parse_hex("5a0").is_err());
        assert!(parse_hex("zz").is_err());
    }

    #[test]
    fn parses_command_bytes() {
        assert_eq!(parse_byte("0x10").unwrap(), 0x10);
        assert_eq!(parse_byte("50").unwrap(), 0x50);
        assert!(parse_byte("nope").is_err());
    }
}
