# ak820-gtk

[![CI](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/actions/workflows/ci.yml/badge.svg)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/WhatTFF/linux-ajazz-ak820-pro-gtk?include_prereleases&label=release&color=ff6a2b)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20x86__64-lightgrey.svg)](#download)

GTK3 control app for the **AJAZZ / Epomaker AK820 Pro** keyboard screen — a native Linux GUI that
reuses the shared protocol, hidraw transport and on-disk history/settings crates from the
Rust/Tauri project [`linux-ajazz-ak820-pro-app`](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app).

Put GIFs and images on the 0.85" 128×128 screen, then forget about it: the app keeps the
keyboard clock in step with your system clock automatically.

> Unofficial. Not affiliated with AJAZZ or Epomaker. Protocol recovered by the upstream project
> from the vendor's own WebHID driver — see [PROTOCOL.md](PROTOCOL.md).

**Source, issues and releases:** <https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk> ·
[releases](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/releases) ·
[issues](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/issues)

## Download

[![Download the source tarball](https://img.shields.io/badge/download-source-294172?style=for-the-badge&logo=github&logoColor=white)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/releases)

Every [release](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/releases) carries a source
tarball plus a `SHA256SUMS` file; the newest release is listed first.

## Status

Working on hardware (fw 1.07, PID `0c45:8009`):

- `ak820ctl` — `info`, `time`, `upload`, `builtin`, `history list|export|restore|clear`, `raw`
- `ak820-gtk` — GTK3 window: device info, clock sync, built-in animations, image/GIF upload with
  live preview and progress bar, upload history, settings
- **Settings**: start at login, start minimized, minimize to tray, sync on start, sync on connect,
  restore last image on connect, autosync interval, update checks
- clock autosync units (`systemd/`), udev rule

Not done yet: RPM/DEB/AppImage packaging, Flatpak, COPR, GIPHY key in the keyring,
SQLite-backed history. See the upstream project's [PLAN.md](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/blob/main/PLAN.md).

## Install (from source, Fedora)

```bash
sudo dnf install gtk3-devel gdk3-devel glib2-devel cargo rust gcc gcc-c++ pkgconf-pkg-config
cargo build --release -p ak820-gtk -p ak820-cli

sudo install -Dm755 target/release/ak820-gtk /usr/local/bin/
sudo install -Dm755 target/release/ak820ctl /usr/local/bin/
sudo install -Dm644 udev/70-ak820.rules /usr/lib/udev/rules.d/70-ak820.rules
sudo udevadm control --reload && sudo udevadm trigger
```

Unplug and replug the keyboard after installing the rule. Then:

```bash
ak820ctl info                 # interfaces, firmware, frame limit
ak820ctl upload cat.gif       # PNG / JPEG / WebP / GIF
ak820ctl time                 # push the clock once
./target/release/ak820-gtk    # the app
```

## Clock autosync

Two ways, pick one:

- **In the app** (default): ⚙ Settings → autosync clock. The clock is pushed when
  the app opens, then every 60 s while it runs (only when the keyboard is present;
  silent when it is not).
- **With systemd**: the units below, for machines where the app is not running.

```bash
mkdir -p ~/.config/systemd/user
cp systemd/ak820-*.service systemd/ak820-clock.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ak820-clock.timer
systemctl --user enable ak820-resume.service
```

The timer pushes the clock every 60 s while the keyboard is connected (it exits silently when it
is not, and when `autosync_clock` is switched off in the app), `ak820-resume.service` covers
suspend/resume, and the udev rule starts `ak820-connect.service` on plug-in, which also
re-uploads the newest history entry when `restore_on_connect` is enabled in
`~/.local/share/ak820/settings.json`.

## Layout

```
crates/ak820-proto    packet framing, animation payload, RGB565, device info (pure, unit-tested)
crates/ak820-device   sysfs interface discovery, hidraw I/O, retries, single-writer lock
crates/ak820-store    history on disk (same format as the Python CLI), settings
crates/ak820-cli      ak820ctl
apps/gtk              GTK3 control application
systemd/ udev/        autosync units and device permissions
```

## Tests

```bash
cargo test --workspace                            # pure logic, no hardware needed
```

## Credits

Upstream protocol research, web UI, generators and effects: **fromagge/ak820-screen** (MIT).
The shared Rust crates are ported from [`WhatTFF/linux-ajazz-ak820-pro-app`](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app)
(MIT). See [NOTICE](NOTICE) and [docs/UPSTREAM.md](docs/UPSTREAM.md).
