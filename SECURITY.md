# Security Policy

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Use GitHub's private advisory form:

> <https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk/security/advisories/new>

That creates a private thread only you and the maintainer can see, and it lets
a fix be prepared and released before the details are public.

If you cannot use GitHub, email **emin.by@outlook.com** with
`AK820 GTK security` in the subject.

Please include what you can: the affected version (Settings shows it, or run
`ak820ctl --version`), your distribution and desktop, keyboard firmware and PID,
what you did, and what happened. A proof of concept helps but is not required.

## What to expect

This is a one-person project, so these are intentions rather than a service
level:

- Acknowledgement within about 7 days.
- An assessment and a rough timeline within about 30 days.
- Credit in the release notes and the advisory, unless you ask not to be named.
- Please give 90 days before publishing details, or less if a fix ships sooner.

## In scope

- **The update path** — GitHub release lookup, asset download, `pkexec`
  install, AppImage replacement, and restart. A compromised update is the worst
  case here: it runs with the user's session and can open the keyboard.
- **hidraw transport and udev rule** — anything that lets another local user or
  process read/write the keyboard without permission, escape the single-writer
  lock, or make the app send packets to the wrong device.
- **Upload / clock / restore paths** — malformed images, history payloads, or
  device replies that cause out-of-bounds access, panics, or unintended
  commands.
- **The optional GIPHY search** — URL handling, API-key exposure, and response
  parsing.
- **Local storage** — `settings.json` and history in `~/.local/share/ak820`.
  Anything that lets another local process tamper with them.
- **Data leaving the machine.** There should be no outbound request except the
  update check and an explicit GIPHY search.

## Out of scope

- Anything that requires root, physical access, or a malicious keyboard
  firmware. This app cannot defend against the device or the person holding it.
- A local attacker who can already read your user's files or run code as your
  user.
- Missing hardening with no demonstrated impact.
- Reports produced only by an automated scanner, with no working path to abuse.
- The 2.4 GHz dongle not exposing the screen interface. That is a hardware
  limitation, documented in the README, not a vulnerability.

## Supported versions

Only the newest published release gets fixes. There are no backports to older
versions — the update path exists so people can move forward.
