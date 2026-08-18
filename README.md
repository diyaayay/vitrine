# vitrine

A Wayland **kiosk compositor** in Rust, built on [Smithay](https://github.com/Smithay/smithay).

*Vitrine* (French): a glass display case — which is exactly what a kiosk compositor is.
One screen, one application, nothing else: no desktop, no window chrome, no way for the
user to escape into a shell. The class of display server that drives digital signage,
ticket machines, point-of-sale terminals, and industrial panels.

vitrine boots straight into a single fullscreen Wayland application — on real hardware
via DRM/KMS with no display server underneath, or nested as a window inside an existing
desktop session for development.

## Design

A kiosk is not a small desktop; it is a different product with different invariants:

- **Every window is fullscreen, always.** Clients are told their size in the initial
  `xdg_toplevel` configure; requests to unfullscreen are politely denied.
- **No interactive move or resize.** The xdg-shell `move`/`resize` requests are
  deliberate no-ops — a fullscreen window has nowhere to go.
- **Focus follows the stacking order, not clicks.** The topmost window owns the
  keyboard, unconditionally. There is no click-to-focus because there is nothing to
  click *between*.
- **The compositor owns the application lifecycle.** It launches the configured app
  and (soon) restarts it if it crashes — on an unattended device, nobody is there to.

Prior art in this space: [Ubuntu Frame](https://github.com/canonical/ubuntu-frame)
(Mir-based) and [cage](https://github.com/cage-kiosk/cage) (wlroots-based).

## Architecture

```
                 ┌────────────────────────────────────────────┐
                 │                  vitrine                    │
   Wayland       │  ┌──────────┐  ┌─────────┐  ┌───────────┐  │
   clients ──────┼─▶│ protocol │─▶│  space  │─▶│ renderer  │  │
   (socket)      │  │ handlers │  │ (scene) │  │ (GLES2 +  │  │
                 │  │ xdg-shell│  │         │  │  damage)  │  │
                 │  └──────────┘  └─────────┘  └─────┬─────┘  │
                 │        ▲                          │        │
                 │        │ input events             ▼ frames │
                 │  ┌─────┴──────────────────────────────┐    │
                 │  │  backend:  winit (nested, dev)     │    │
                 │  │            udev (DRM/KMS, metal)   │    │
                 │  └────────────────────────────────────┘    │
                 └────────────────────────────────────────────┘
                       one calloop event loop, single thread
```

## Roadmap

- [x] **0 — Scaffold**: workspace, Smithay 0.7, CI-ready layout
- [x] **1 — Nested compositor** (winit backend): xdg-shell toplevels & popups,
      GLES2 rendering, keyboard/pointer input, kiosk window policy
- [x] **2 — Bare metal** (udev backend): DRM/KMS mode setting, libinput,
      libseat session — runs on a TTY with no display server underneath
      (verified on an Intel Iris Xe laptop, eDP panel at native mode)
- [x] **3 — Kiosk runtime**: TOML config, app launch + crash watchdog
- [x] **4 — Performance**: damage visualization (`--debug-damage`),
      frame-timing statistics
- [ ] **5 — Hardening**: CI (fmt, clippy, build), protocol test client, docs

## Running

Nested, inside an existing Wayland session (development):

```bash
cargo run -- -c foot          # or any Wayland-native app, e.g. konsole
```

vitrine opens a window that *is* its output; the app launches fullscreen inside it.
Point any other client at the compositor manually:

```bash
WAYLAND_DISPLAY=wayland-1 some-app    # socket name is printed at startup
```

On bare metal (checkpoint 2): switch to a free TTY (`Ctrl+Alt+F3`) and run the same
binary — it will pick the DRM backend automatically.

## Performance

vitrine renders incrementally: an `OutputDamageTracker` intersects each
swapchain buffer's **age** with accumulated client damage, so only regions
that changed since that buffer was last on screen are repainted. Two tools
make this observable:

- `--debug-damage` — tints every repainted pixel, so you can *watch* damage
  tracking work: type in the kiosk terminal and only the touched character
  cells flash, while the rest of the screen provably isn't redrawn.
- Frame stats, logged every 5 s (`RUST_LOG=info`):

  ```
  frame stats fps=60.0 avg_render_ms=0.36 max_render_ms=0.48 idle_frames=100%
  ```

  `idle_frames` is the share of frames where nothing needed repainting at
  all — for a kiosk showing static content this should sit near 100%, which
  is the difference between a warm and a cool fanless signage box.

  This instrumentation already earned its keep: it exposed the nested
  backend full-repainting every frame (`idle_frames=0%`) because it passed
  buffer age 0; feeding real swapchain age took idle repaints to 100%.

## Study notes

This project is also a learning log of the Linux graphics stack. Each checkpoint has a
write-up in [`notes/`](notes/) covering the concepts it introduced, with links to specs
and docs.

## License

MIT
