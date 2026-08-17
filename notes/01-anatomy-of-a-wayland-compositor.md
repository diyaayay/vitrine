# Checkpoint 1 — Anatomy of a Wayland compositor

What we built: a working compositor that runs nested in a window, accepts real
Wayland clients, renders them fullscreen, and forwards keyboard/pointer input.
This note explains every moving part, in the order data flows through them.

## 1. What a "compositor" actually is

Under X11, three separate programs cooperated: the X server (talks to hardware),
a window manager (decides where windows go), and optionally a compositor
(blends the final image). Wayland collapses all three into **one process**: the
compositor. It owns the screen, the input devices, and the protocol socket that
clients connect to. There is no middleman — which is why "writing a Wayland
compositor" means implementing a display server, a window manager, and a
renderer at once, and why Smithay exists to make that tractable.

**Read:** [The Wayland Book](https://wayland-book.com/) — chapters 1–4 tonight,
the rest across the week. It is short and superb.

## 2. The wire protocol — this is JSON-RPC with different clothes

You already know LSP, so map it directly:

| LSP / JSON-RPC            | Wayland                                        |
|---------------------------|------------------------------------------------|
| JSON over stdio/socket    | packed binary over a Unix socket (`wayland-1`) |
| client → server *request* | client → compositor *request*                  |
| server → client *notification* | compositor → client *event*               |
| methods on the protocol   | requests/events grouped into **interfaces** (`wl_surface`, `xdg_toplevel`…) |
| capabilities negotiation  | the **registry**: compositor advertises **globals**, client binds the ones it wants, with versions |
| spec in markdown/TS types | spec in XML files, code-generated for both sides |

Two differences worth internalizing: Wayland messages address **object
instances** (object id + opcode), not free-standing methods — everything is a
little state machine attached to an object. And it is **asynchronous by
default**: nobody blocks waiting for a reply; replies are just events that
arrive later. (LSP's `initialize` handshake has exactly one Wayland cousin —
see §5.)

**Browse:** [wayland.app](https://wayland.app/protocols/) — every protocol,
every interface, cross-linked. Look up `wl_surface` and `xdg_surface` now;
you'll recognize our handler code in their descriptions.

## 3. Where clients connect: `state.rs`

`init_wayland_listener` does two things you should be able to point at:

- `ListeningSocketSource::new_auto()` creates `$XDG_RUNTIME_DIR/wayland-N` —
  *our* socket, next to KDE's `wayland-0`. Setting `WAYLAND_DISPLAY=wayland-N`
  is all it takes to point a client at us instead of KDE.
- The `Display` is inserted into the event loop so that when client bytes
  arrive, `dispatch_clients` decodes them and calls **our** handler methods.

All the `*State` fields (`CompositorState`, `XdgShellState`, `ShmState`…) are
the globals we advertise. Creating one = announcing to every client "I support
this protocol". Delete one and clients simply won't see that capability —
that's the negotiation model.

## 4. The event loop: calloop, not tokio

A compositor is a latency machine, not a throughput machine. calloop is a
single-threaded poll-based loop: every source (client socket, input devices,
timers, later DRM vblanks) registers a callback, and exactly one thing runs at
a time against `&mut Vitrine`. No `Send`/`Sync`, no locks, no async runtime —
because the work per event is microseconds and the deadline is the next frame.
Your Tokio instincts transfer; the ownership story is just simpler.

## 5. Surfaces, roles, and the xdg-shell handshake: `handlers/xdg_shell.rs`

A `wl_surface` is just "a rectangle of pixels the client will supply". It
becomes a *window* by being assigned a **role** — for normal windows that role
comes from the **xdg-shell** protocol (`xdg_surface` → `xdg_toplevel`).

The handshake matters — it's the one place Wayland is strict:

1. Client creates the toplevel and **commits with no buffer** ("I exist, tell
   me how to look").
2. Compositor answers with a **`configure` event** — size + states. In vitrine
   this is where kiosk policy lives: `new_toplevel` stages
   `size = output, state = Fullscreen` so the *first* configure already says
   "you are fullscreen at 1920×1080". (`handle_xdg_commit` sends it.)
3. Client **`ack_configure`** + commits a buffer of exactly that size.

This is `initialize`/`initialized` with a twist: it repeats. Every resize or
state change is another configure/ack round-trip, and state is
**double-buffered** — `with_pending_state` stages values that only take effect
at the configure/commit boundary, so the client never renders a half-applied
state. Compare `move_request`/`resize_request` in smallvil (interactive grabs,
520 lines) with ours (empty bodies, one comment each): that asymmetry *is* the
kiosk design.

**Read:** the [xdg-shell spec](https://wayland.app/protocols/xdg-shell) —
`xdg_surface.configure` and `xdg_toplevel.configure` sections.

## 6. Pixels: buffers and SHM — `handlers/compositor.rs`

Clients don't send draw commands; they render themselves and hand us finished
pixel buffers. Tonight's path is **SHM**: shared-memory buffers (`ShmState`).
`commit` is the transaction point — the client says "everything I staged is now
one consistent frame", and `on_commit_buffer_handler` imports the attached
buffer so the renderer can turn it into a GL texture. (On real hardware,
GPU-rendered clients use **dma-buf** instead — zero-copy GPU handles. That's a
Day 3 topic.)

## 7. Compositing: `backend/winit.rs`

Each redraw: `render_output` walks the `Space` (our scene: windows + one
output), uploads any changed buffers as GLES2 textures, and draws them over the
clear color. Two concepts to hold onto:

- **Damage tracking** (`OutputDamageTracker`): only regions that changed since
  last frame need repainting. A kiosk showing a static sign should redraw
  *nothing* at idle — this is the difference between 0.3 W and 3 W on a fanless
  signage box. We'll visualize this at checkpoint 4.
- **Frame callbacks** (`send_frame`): the client asked "tell me when to draw
  next". By answering only after we present, we pace clients to the display
  instead of letting them render frames nobody shows. This is Wayland's
  every-frame-is-perfect guarantee, and it's also throttling.

## 8. Input: the seat — `input.rs`

A **seat** is one user's set of devices (keyboard + pointer + touch). The winit
backend feeds us window events tonight; libinput will feed us evdev events on
Day 3 — same `process_input_event` code, which is the point of Smithay's
`InputBackend` abstraction. The keyboard hands each key to the **focused**
surface; our focus rule is `focus_topmost` in `state.rs` (stack-driven, not
click-driven — kiosk policy again). The `FilterResult` in the keyboard closure
is where compositor-level keybindings would intercept keys before clients see
them.

## Reading list (priority order)

1. [The Wayland Book](https://wayland-book.com/) — the mental model
2. [xdg-shell on wayland.app](https://wayland.app/protocols/xdg-shell) — the handshake we implement
3. [Smithay 0.7 docs](https://docs.rs/smithay/0.7.0/smithay/) — start at `wayland::compositor` module docs, they're genuinely explanatory
4. [smallvil source](https://github.com/Smithay/smithay/tree/master/smallvil) (cloned at `~/Projects/smithay-ref`) — diff it against vitrine and make sure every divergence is one you can defend
5. Ubuntu Frame's [README](https://github.com/canonical/ubuntu-frame) — the product we're a study of
