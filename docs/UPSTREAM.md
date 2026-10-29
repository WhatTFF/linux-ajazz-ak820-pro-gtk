# Upstream tracking

Portions are derived from **fromagge/ak820-screen** @ `9030b81` (2026-10-21), MIT.
The GTK application and Rust transport are this project's implementation.

## What is derived from upstream

| Upstream path | Here | State |
|---|---|---|
| `PROTOCOL.md` | `PROTOCOL.md` | copied, status note prepended |
| `70-ak820.rules` | `udev/70-ak820.rules` | copied + `systemd` tags for autosync |
| `web/gen.js` | `apps/gtk/src/gen.rs` | ported generators, same parameters/visuals |
| `web/fx.js` | `apps/gtk/src/fx.rs` | ported effects and text drawing |
| `web/app.js` | `apps/gtk/src/main.rs` | GTK3 implementation of the same UI concepts |
| `ak820.py` | — | behavioural reference only |

The original Tauri UI is not vendored here. See
[WhatTFF/linux-ajazz-ak820-pro-app](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app)
for the WebKit version.

## Patch rules

1. The protocol is the contract. Any upstream change to `PROTOCOL.md` is reviewed before code
   changes.
2. `crates/ak820-proto` golden tests must keep passing; payloads are compared against fixtures
   generated from the upstream JavaScript.
3. Generators and effects should stay visually equivalent to upstream. If a change needs a new
   GTK control, add it to `show_gen_params` / the effects card rather than diverging silently.

## Sync procedure

```bash
git remote add upstream https://github.com/fromagge/ak820-screen
git fetch upstream
git diff upstream/main -- web/gen.js web/fx.js PROTOCOL.md udev/70-ak820.rules \
  > /tmp/upstream.diff
```

Apply the hunks by hand, run `cargo test --workspace`, then bump the commit hash in `NOTICE`.

## License

Upstream is MIT. This project is GPL-3.0-only. The MIT notice is preserved in
[LICENSE-MIT](../LICENSE-MIT) and [NOTICE](../NOTICE).
