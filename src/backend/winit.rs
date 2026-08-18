//! Development backend: the compositor runs as an ordinary window inside an
//! existing desktop session. The window *is* our single output.

use std::time::{Duration, Instant};

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer, DebugFlags, ImportDma, Renderer,
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
};

use crate::{perf::FrameStats, CalloopData, Vitrine};

pub fn init_winit(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
    debug_damage: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = &mut data.display_handle;
    let state = &mut data.state;

    let (mut backend, winit) = winit::init::<GlesRenderer>()?;
    if debug_damage {
        backend.renderer().set_debug_flags(DebugFlags::TINT);
    }
    let mut stats = FrameStats::new();

    // Advertise zero-copy GPU buffer imports, limited to the formats this
    // renderer can actually consume.
    let dmabuf_formats = backend.renderer().dmabuf_formats();
    let _dmabuf_global = state
        .dmabuf_state
        .create_global::<Vitrine>(display_handle, dmabuf_formats);

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    // An Output is what we tell clients about the "monitor" they are shown
    // on. For this backend the winit window plays that role.
    let output = Output::new(
        "vitrine-winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "vitrine".into(),
            model: "winit".into(),
        },
    );
    let _global = output.create_global::<Vitrine>(display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, data| {
            let display = &mut data.display_handle;
            let state = &mut data.state;

            match event {
                WinitEvent::Resized { size, .. } => {
                    output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: 60_000,
                        }),
                        None,
                        None,
                        None,
                    );
                    // The output changed, so every kiosk window must be re-fit.
                    state.refit_windows();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);

                    let render_start = Instant::now();
                    // Buffer age tells the damage tracker how stale this
                    // swapchain buffer is; 0 would force a full repaint.
                    let age = backend.buffer_age().unwrap_or(0);
                    let repainted = {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        smithay::desktop::space::render_output::<
                            _,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                            _,
                            _,
                        >(
                            &output,
                            renderer,
                            &mut framebuffer,
                            1.0,
                            age,
                            [&state.space],
                            &[],
                            &mut damage_tracker,
                            [0.02, 0.02, 0.05, 1.0],
                        )
                        .unwrap()
                        .damage
                        .is_some()
                    };
                    backend.submit(Some(&[damage])).unwrap();
                    stats.record(render_start.elapsed(), repainted);

                    // Frame callbacks: tell every client "your frame was shown,
                    // you may draw the next one". This is what paces client
                    // rendering to the display, instead of letting them spin.
                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        )
                    });

                    state.space.refresh();
                    state.popups.cleanup();
                    let _ = display.flush_clients();

                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    Ok(())
}
