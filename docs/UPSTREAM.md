# Upstream tracking

Vendored from **fromagge/ak820-screen** @ `9030b81` (2026-10-21), MIT.

Goal: keep the UI a *mechanical* copy so upstream commits can be merged by hand in minutes.

## What is vendored

| Upstream path | Here | State |
|---|---|---|
| `PROTOCOL.md` | `PROTOCOL.md` | copied, status note prepended |
| `70-ak820.rules` | `udev/70-ak820.rules` | copied + `systemd` tags for autosync |
| `web/index.html` | `apps/desktop/ui/index.html` | verbatim (M2) |
| `web/app.js` | `apps/desktop/ui/app.js` | patched: bootstrap, history, GIPHY, drag-drop |
| `web/gen.js` | `apps/desktop/ui/gen.js` | verbatim |
| `web/fx.js` | `apps/desktop/ui/fx.js` | verbatim |
| `web/ak820.js` | `apps/desktop/ui/ipc.js` | transport replaced with Tauri commands; `buildPayload`/`parsePayload` kept verbatim |
| `ak820.py` | — | behavioural reference only |

## Patch rules

1. Never restructure upstream code. Patch points are marked with `// NATIVE:` comments so a
   three-way merge stays possible.
2. `buildPayload()` and `parsePayload()` must stay byte-identical; Rust golden tests compare
   against fixtures generated from these functions.
3. Any upstream change to `PROTOCOL.md` is reviewed before code changes — the wire format is the
   contract.

## Sync procedure

```bash
git remote add upstream https://github.com/fromagge/ak820-screen
git fetch upstream
git diff upstream/main -- web/ > /tmp/upstream-ui.diff
```

Apply the hunks by hand into `apps/desktop/ui/`, run `cargo test -p ak820-proto` (golden vectors)
and the UI parity screenshot job, then bump the commit hash in `NOTICE`.
