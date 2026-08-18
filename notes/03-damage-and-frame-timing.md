# Checkpoint 4 — Damage tracking, buffer age, and frame timing

What we built: `--debug-damage` (tint repainted pixels via the renderer's
`DebugFlags::TINT`) and `src/perf.rs` (per-frame stats aggregated to a log
line every 5 s). Small code, but it forced the deepest concept so far.

## 1. Damage: the currency of compositor performance

A naive compositor redraws the whole screen every frame. A real one tracks
**damage** — the set of rectangles that changed — from two directions:

- **Client damage**: `wl_surface.damage_buffer` — the client tells us which
  part of its new buffer differs ("I redrew the cursor cell only").
- **Output damage**: what the *screen* needs repainted this frame, which is
  client damage plus compositor-side changes (a window mapped/moved/died).

`OutputDamageTracker` does the bookkeeping: every `render_output` call, it
collects new damage, intersects it with what the target buffer is missing,
draws only that, and returns `damage: None` when there is nothing to do —
the "idle frame" our stats count.

## 2. Buffer age — why the tracker must know history

We render into a **swapchain** (2–3 buffers rotating). When a buffer comes
back to us, it still holds the frame from 2–3 flips ago — it is *stale by N
frames*, and N is its **age**. The damage tracker keeps recent per-frame
damage lists, so given age N it repaints the union of the last N frames'
damage into this buffer — cheap, and every pixel ends up current.

Age 0 is the escape hatch meaning "contents unknown, repaint everything."

**The bug the stats caught (tell this story in interviews):** our nested
backend passed a hardcoded age of `0` — copied faithfully from the smallvil
example — so every frame was a full repaint: `idle_frames=0%`,
avg 0.65 ms. Switching to the real `backend.buffer_age()` made an idle
terminal read `idle_frames=100%`, avg 0.36 ms. One line; measured, not
vibes. (The udev backend was already correct: `GbmBufferedSurface`'s
`next_buffer()` hands back the true age.)

## 3. Why `idle_frames` is *the* kiosk metric

A signage box shows static content for hours. Frames with zero repaint cost
are the difference between a fanless box idling cool and one drawing GPU
power 60×/s to redraw an unchanged menu. (The next optimization tier —
deliberately not built — is skipping the page flip entirely on idle frames
and waking only on damage, taking even the flip overhead to zero. Know the
trade-off: you must then re-arm rendering on the first commit that brings
new damage.)

## 4. How `--debug-damage` works

`Renderer::set_debug_flags(DebugFlags::TINT)` makes the GLES renderer
multiply every pixel it draws with a tint. Repainted regions flash; skipped
regions stay clean. It visualizes exactly the pixels our damage logic
touches — a debugging tool for correctness ("why is this region
repainting every frame?") as much as a demo.

## Reading

1. [`wl_surface.damage_buffer`](https://wayland.app/protocols/wayland#wl_surface:request:damage_buffer) — client-side damage
2. Smithay's [`OutputDamageTracker` docs](https://docs.rs/smithay/0.7.0/smithay/backend/renderer/damage/struct.OutputDamageTracker.html) — the age/damage algorithm described precisely
3. `EGL_EXT_buffer_age` extension spec — where the age concept comes from
