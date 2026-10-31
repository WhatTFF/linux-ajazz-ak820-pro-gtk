---
name: Bug Report
about: Report a bug to help improve AK820 Screen Studio
title: ''
labels: bug
assignees: ''
---

**Describe the bug**
A clear and concise description of what the bug is.

**To Reproduce**
Steps to reproduce the behavior:
1. Open AK820 Screen Studio
2. Go to '...'
3. Click on '...'
4. See error

**Expected behavior**
A clear and concise description of what you expected to happen.

**Screenshots / screen recordings**
If applicable, add media to help explain your problem.

**Environment (please complete the following):**
- Distribution and version: [e.g. Fedora 45]
- Desktop and display server: [e.g. GNOME 49, Wayland]
- App version: [e.g. 1.0.0]
- Keyboard model and PID: [e.g. AK820 Pro, 0c45:8009]
- Firmware: [e.g. 1.07]
- Connection: [wired USB-C / 2.4 GHz dongle / Bluetooth]
- Keyboard mode switch: [wired / 2.4 GHz / Bluetooth]

**Logs**
Paste any terminal output. For crashes, run with `RUST_BACKTRACE=1`. For
permission or device issues, include:
- `ak820ctl info`
- `lsusb | grep -i 0c45`
- relevant `dmesg` or `journalctl --user` lines

**Additional context**
Add any other context about the problem here.
