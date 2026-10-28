# ak820-gtk

GTK3 control app for the **AJAZZ / Epomaker AK820 Pro** TFT screen — a native Linux GUI that
reuses the shared protocol, hidraw transport and on-disk history/settings crates from the
Rust/Tauri project [`linux-ajazz-ak820-pro-app`](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app).

## What it does

- Read device info (firmware, battery, max frames, screen direction)
- Sync the keyboard's screen clock to the system clock (one-shot or background autosync)
- Select one of the firmware's built-in animations
- Upload images or animated GIFs to the 128×128 screen, with a live preview and progress bar
- Browse and restore previous uploads from on-disk history
- Save settings (autosync, restore-on-connect, start-at-login, ...)

## Build

Requires GTK3 development headers (`gtk3-devel`, `gdk3-devel`, `glib2-devel`) and Rust.

```bash
cargo build --release -p ak820-gtk
```

The binary is `target/release/ak820-gtk`.

## Device access

The app talks to the keyboard directly over `hidraw`. You need the udev rule so a normal user
can open the device:

```bash
sudo install -Dm644 udev/70-ak820.rules /usr/lib/udev/rules.d/70-ak820.rules
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the keyboard (USB-C cable, wired mode — the screen is not reachable over
the 2.4 GHz dongle or Bluetooth).

## Layout

```
crates/ak820-proto    packet framing, animation payload, RGB565 (pure, unit-tested)
crates/ak820-device   hidraw interface discovery, I/O, retries, single-writer lock
crates/ak820-store    history on disk + settings
crates/ak820-cli      ak820ctl (command-line parity tool)
apps/gtk              GTK3 GUI application
udev/                 device permission rule
systemd/              clock autosync units (adapt paths for ak820-gtk)
```

## Protocol

See `PROTOCOL.md` and the upstream project's `PROTOCOL.md`. Screen protocol recovered from the
vendor's WebHID driver; unofficial, not affiliated with AJAZZ or Epomaker.

## License

MIT. The shared crates carry the same MIT license as the upstream Rust/Tauri project.
