# Checkpoint 2 — Bare metal: DRM/KMS, libinput, libseat

What we built: `src/backend/udev.rs` — vitrine running directly on the
hardware from a TTY, with no display server underneath. This note explains the
four kernel-facing subsystems it talks to, in the order `init_udev` uses them.

## 1. The problem: who may touch the hardware?

`/dev/dri/card1` (the GPU's display engine) and `/dev/input/event*` (raw
keyboards and mice) are privileged. Historically display servers ran as root;
today a **seat manager** brokers access. **libseat** is a small library that
talks to either `seatd` or systemd-logind and hands us open file descriptors
for these devices while we stay an ordinary user. It also owns the concept of
*which VT is active*: when you Ctrl+Alt+F2 away, libseat *pauses* our session
(we lose the devices), and *resumes* it when you switch back. That's the
`SessionEvent::PauseSession / ActivateSession` handler — a compositor must be
able to give up the GPU and re-acquire it at any moment.

A **seat** here = one physical workstation (one screen + input set), almost
always `seat0`. Same word as Wayland's `wl_seat` on purpose — ours is the
kernel-side twin of the one we advertise to clients.

## 2. DRM/KMS: the kernel's display API

DRM (Direct Rendering Manager) is the GPU subsystem; KMS (Kernel Mode
Setting) is its display-configuration half. Its object model, which our
connector-scan code walks:

- **connector** — a physical port (eDP-1 = your laptop panel, HDMI-A-1…).
  We take the first one whose state is `Connected`.
- **mode** — a resolution + refresh combo the display supports; we take the
  one flagged `PREFERRED` (the panel's native mode).
- **encoder** → **crtc** — the CRTC is the scanout engine that reads a
  framebuffer and feeds the connector. The encoder list tells us which CRTCs
  can drive which connector; we claim the first compatible one.
- **framebuffer / plane** — the buffer being scanned out. Page-flipping =
  atomically swapping the scanout buffer at vblank.

`drm.create_surface(crtc, mode, &[connector])` claims that chain for us:
"kernel, please scan out whatever I hand this CRTC, at 1920×1080@60, to eDP-1."

**Read:** kernel docs [DRM KMS overview](https://www.kernel.org/doc/html/latest/gpu/drm-kms.html)
(skim the object diagrams, skip driver internals) and `man drm-kms`.

## 3. GBM + EGL: getting GL without a window system

On the desktop, EGL gave us a GL context *inside a window*. There are no
windows here — **GBM** (Generic Buffer Management, part of Mesa) allocates raw
GPU buffers, and EGL's GBM platform lets GLES render into them. The chain in
code: `GbmDevice::new(fd)` → `EGLDisplay::new(gbm)` → `EGLContext` →
`GlesRenderer`. Buffers are allocated `RENDERING | SCANOUT`: usable as both a
GL render target and a CRTC scanout source — that dual role is the whole trick
of a Linux compositor.

`GbmBufferedSurface` wraps this into a **swapchain**: a small ring of buffers.
`next_buffer()` hands us one plus its **age** — how many frames ago it was last
current. The damage tracker uses age to repaint only what changed since *this
buffer* last saw the screen. Then `queue_buffer()` schedules the page flip.

## 4. vblank: the display is the clock

After queueing a flip, the kernel fires a **VBlank event** when the display
actually latched the new buffer (once per refresh, e.g. 60Hz). Our handler:
`frame_submitted()` (return the old buffer to the swapchain) → render the next
frame → queue again. So the render loop is *paced by the hardware*: draw, flip,
wait for vblank, repeat. Compare with the winit backend where the host
compositor paced us via `request_redraw` — same loop shape, different clock.
This chain — client commit → composite → flip → vblank → frame callback → next
client commit — is the frame-timing story interviewers love to probe.

## 5. libinput: raw input, same handler

libinput reads evdev devices and gives us clean event streams (it does tap
detection, pointer acceleration, palm rejection). Note what we did NOT change:
`process_input_event` is generic over `InputBackend` — the same function
consumes winit events on the desktop and libinput events on the TTY. Two
additions were needed:

- **Relative pointer motion** (a real mouse sends deltas, not positions):
  we integrate the deltas and clamp to the output ourselves.
- **Compositor keybindings**: on a TTY nobody else handles Ctrl+Alt+F2. The
  keyboard filter intercepts `XF86Switch_VT_n` (what xkb turns Ctrl+Alt+Fn
  into) → `session.change_vt(n)`, and Ctrl+Alt+Q → clean shutdown. Everything
  else forwards to the client — a kiosk owns as few keys as possible.

## 6. What we deliberately did not build (know these for interviews)

- **dmabuf clients**: without the `linux-dmabuf` global, GPU-accelerated
  clients fall back to SHM (software) buffers. Fine for a kiosk MVP; adding
  the global + `ImportDma` is the natural next step and unlocks zero-copy.
- **Direct scanout**: `DrmCompositor` can put a fullscreen client's buffer
  straight on the scanout plane, skipping composition entirely — the ideal
  kiosk fast path. We chose `GbmBufferedSurface` (always composite) for
  simplicity; know the trade-off.
- **Hardware cursor plane**: we render no cursor at all — a deliberate kiosk
  choice (signage and touch kiosks hide it), and it saves the cursor-plane
  machinery.
- **Multi-GPU**: anvil's 1,700-line udev backend exists mostly for this;
  a kiosk pins one GPU (`VITRINE_DRM_DEVICE` overrides which).

## Running it

```bash
# from a TTY (Ctrl+Alt+F3, log in):
cd ~/Projects/vitrine && ./target/debug/vitrine --tty -c foot
# escape hatches: Ctrl+Alt+F2 (back to KDE's VT), Ctrl+Alt+Q (quit)
```

## Reading list

1. [DRM/KMS kernel docs](https://www.kernel.org/doc/html/latest/gpu/drm-kms.html) — object model diagrams
2. [libseat/seatd](https://git.sr.ht/~kennylevinsen/seatd) — README explains the seat problem in one page
3. [libinput docs](https://wayland.freedesktop.org/libinput/doc/latest/) — "Architecture" page
4. [The Wayland Book, "Seats" chapter](https://wayland-book.com/seat.html) — ties kernel seats to `wl_seat`
5. `anvil/src/udev.rs` in `~/Projects/smithay-ref` — read it *after* ours; recognize which extra 1,400 lines buy which feature
