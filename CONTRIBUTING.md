# Contributing to AK820 Screen Studio

Thanks for looking. This is a one-person project, so the most useful things you
can do are, in order: **report a bug well**, **tell me what broke on your
keyboard**, and **send a small, focused pull request**.

This file is short on ceremony and long on the one thing that actually bites
contributors: changes to the hidraw transport and clock protocol need a real
AK820 Pro.

---

## Before you write code

Open an issue first, or comment on an existing one. Not to gatekeep — to avoid
you spending an evening on something that is already planned, already tried, or
deliberately not going to happen (see *What will not be merged* below).

Small fixes — a typo, a crash, an obviously wrong string — can go straight to a
pull request without an issue.

## What will not be merged

The README makes promises, and code that breaks them gets closed regardless of
quality:

- Accounts, sign-up, or any kind of login.
- Cloud sync, backup services, telemetry, or a server of any kind.
- Analytics, crash-reporting SDKs, or "anonymous" usage stats.
- Advertising, tracking pixels, or anything from a data broker.
- Bundling vendor binaries or closed-source blobs.
- Anything that uploads keyboard history, screen content, or settings.
- Adding a second outbound network request without discussing it first. The app
  currently talks to GitHub for update checks and to GIPHY only when you search.

## Setting up

```bash
git clone https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk.git
cd linux-ajazz-ak820-pro-gtk

# Fedora
sudo dnf install gtk3-devel glib2-devel cargo rust gcc gcc-c++ pkgconf-pkg-config
# optional, for video import:
sudo dnf install ffmpeg

cargo build -p ak820-gtk -p ak820-cli
./target/debug/ak820-gtk
```

Install the udev rule once and replug the keyboard:

```bash
sudo install -Dm644 udev/70-ak820.rules /usr/lib/udev/rules.d/70-ak820.rules
sudo udevadm control --reload && sudo udevadm trigger
```

The GTK app keeps its state in `$XDG_DATA_HOME/ak820/settings.json` and
`$XDG_DATA_HOME/ak820/history`. Set `XDG_DATA_HOME` to a throwaway directory
when testing changes that write settings or history.

## Checks to run before opening a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p ak820-gtk -p ak820-cli
```

All four must be clean. `cargo test` does not need a keyboard; the protocol,
payload, history and settings tests are pure logic.

## The rule that matters: hardware changes need a real keyboard

The hidraw transport, clock command and upload path cannot be proven by unit
tests. A diff that looks entirely reasonable can still leave the screen blank,
time out on firmware 1.16, or fail only over the 2.4 GHz dongle.

So, if your change touches any of these:

- `crates/ak820-device/` — discovery, report sizing, reads/writes, locking
- `crates/ak820-proto/` — packet framing, payload packing, clock encoding
- `apps/gtk/src/main.rs` upload / sync / restore paths
- `udev/70-ak820.rules` or the systemd units

then:

1. Test on a real AK820 Pro. Not a mock, not "it compiles".
2. Say what firmware version, USB PID and mode (wired / 2.4 GHz dongle) you
   used. `ak820ctl info` prints most of it.
3. For upload changes, say how many frames and whether the screen actually
   updated after unplugging/replugging.
4. For clock changes, check both the cable and the dongle. The dongle bridges
   the clock channel but has no screen interface.
5. Include `RUST_BACKTRACE=1` output for panics, and the relevant `dmesg` or
   journal lines for hidraw permission failures.

Mind the protocol units: delays are milliseconds in the app but stored in
50 ms units on the wire, and the config/bulk report sizes differ by firmware.
Read `PROTOCOL.md` before changing constants.

## Code style

- `cargo fmt` is the formatting authority. `cargo clippy -D warnings` is the
  style guide.
- Match the surrounding code. Comments explain *why*, not *what*.
- No new dependencies without discussing it in an issue first. The app is
  deliberately small and offline-friendly.
- Do not reformat files you are not changing. A pull request that is 90%
  whitespace will be sent back.
- Keep GTK code on the main thread. Blocking USB, network and file work belongs
  in a worker thread with the result delivered through a channel or
  `glib::timeout_add_local`.

## Commits and pull requests

- Keep a pull request to one thing. Two unrelated fixes in one branch will be
  asked to split.
- Commit subjects are short and prefixed, matching the existing history:
  `fix:`, `feat:`, `docs:`, `chore:`, `release:`. The body explains why.
- **Do not bump the version.** The version, tag, README download section and
  release notes are one decision made at release time by the maintainer.
- **Do not update the README for a release.** Same reason. Do update it if your
  change makes something the README says untrue.
- For anything visible in the UI, include a screenshot or a short recording.
- Expect review to take a while. It is one person, and hardware changes get
  read slowly on purpose.

## Security

Do not open a public issue for a security problem. Use the private advisory
form linked in [SECURITY.md](SECURITY.md).

## Licence of your contribution

AK820 Screen Studio is licensed under the **GNU General Public License, version
3.0 only** (see [LICENSE](LICENSE)). There is no CLA to sign and no copyright
assignment.

By opening a pull request you confirm that you wrote the change (or have the
right to submit it) and that you license it to this project under the same
GPL-3.0-only terms. If your employer owns your work, make sure they are fine
with that before you submit.

## Code of conduct

Participation is covered by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). The short
version: be decent, help people with their keyboards, no harassment, no
telemetry.
