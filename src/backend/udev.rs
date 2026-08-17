//! Bare-metal backend: vitrine owns the GPU and input devices directly.
//!
//! No display server underneath — this is the path a real kiosk boots into:
//!
//! - **libseat** opens privileged devices (`/dev/dri/*`, `/dev/input/*`) for
//!   us without root, and pauses/resumes us across VT switches.
//! - **DRM/KMS** is the kernel's display API: we pick a connected
//!   `connector` (the physical port), its preferred `mode` (resolution +
//!   refresh), and a `crtc` (the scanout engine) to drive it.
//! - **GBM** allocates the buffers we render into; a swapchain of them is
//!   page-flipped to the screen.
//! - **vblank** events pace the loop: render → queue flip → kernel presents
//!   at the next refresh → vblank fires → render the next frame.

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer, Bind,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu},
    },
    desktop::space::render_output,
    output::{Mode as WlMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        drm::control::{connector, Device as ControlDevice, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::DisplayHandle,
    },
    utils::{DeviceFd, Transform},
};
use tracing::{error, info, warn};

use crate::{CalloopData, Vitrine};

const CLEAR_COLOR: [f32; 4] = [0.02, 0.02, 0.05, 1.0];
const SUPPORTED_COLOR_FORMATS: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Abgr8888];

/// Everything the render/session callbacks need, shared between the calloop
/// sources (all on one thread, hence Rc<RefCell>).
struct UdevData {
    session: LibSeatSession,
    drm: DrmDevice,
    gbm_surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    renderer: GlesRenderer,
    damage_tracker: OutputDamageTracker,
    output: Output,
}

pub fn init_udev(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = &mut data.state;

    // 1. Session: our ticket to privileged devices while unprivileged.
    let (session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();
    info!(seat_name, "libseat session acquired");

    // 2. Find the GPU. `primary_gpu` follows the udev "boot_vga" tag, which
    //    on hybrid laptops picks the iGPU driving the internal panel.
    let gpu_path = std::env::var("VITRINE_DRM_DEVICE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| primary_gpu(&seat_name).ok().flatten())
        .or_else(|| all_gpus(&seat_name).ok().and_then(|mut g| g.pop()))
        .ok_or("no GPU found")?;
    info!(?gpu_path, "opening DRM device");

    let mut session_for_open = session.clone();
    let fd = session_for_open.open(
        &gpu_path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (mut drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    // 3. GLES2 renderer on top of GBM via EGL.
    // Safety: the display is dropped only when the backend shuts down.
    let egl_display = unsafe { EGLDisplay::new(gbm.clone())? };
    let egl_context = EGLContext::new(&egl_display)?;
    // Safety: we never share this context across threads.
    let renderer = unsafe { GlesRenderer::new(egl_context)? };

    // 4. Mode setting: connector -> mode -> crtc -> surface.
    let res_handles = drm.resource_handles()?;
    let connector = res_handles
        .connectors()
        .iter()
        .filter_map(|h| drm.get_connector(*h, true).ok())
        .find(|c| c.state() == connector::State::Connected)
        .ok_or("no connected display found")?;

    let mode = *connector
        .modes()
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| connector.modes().first())
        .ok_or("connector has no modes")?;

    let crtc = connector
        .encoders()
        .iter()
        .filter_map(|h| drm.get_encoder(*h).ok())
        .find_map(|enc| res_handles.filter_crtcs(enc.possible_crtcs()).first().copied())
        .ok_or("no CRTC available for connector")?;

    let output_name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
    info!(output_name, ?mode, "mode set");

    let drm_surface = drm.create_surface(crtc, mode, &[connector.handle()])?;

    // 5. The swapchain: GBM buffers page-flipped to the crtc.
    let allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let gbm_surface =
        GbmBufferedSurface::new(drm_surface, allocator, SUPPORTED_COLOR_FORMATS, render_formats)?;

    // 6. Advertise the physical output to clients.
    let output = Output::new(
        output_name,
        PhysicalProperties {
            size: connector
                .size()
                .map(|(w, h)| (w as i32, h as i32))
                .unwrap_or((0, 0))
                .into(),
            subpixel: Subpixel::Unknown,
            make: "vitrine".into(),
            model: "kiosk".into(),
        },
    );
    let _global = output.create_global::<Vitrine>(&data.display_handle);
    let wl_mode = WlMode::from(mode);
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), None, Some((0, 0).into()));
    output.set_preferred(wl_mode);
    state.space.map_output(&output, (0, 0));

    // The kiosk needs the session for VT switching from input handling.
    state.session = Some(session.clone());

    let damage_tracker = OutputDamageTracker::from_output(&output);

    let udev = Rc::new(RefCell::new(UdevData {
        session,
        drm,
        gbm_surface,
        renderer,
        damage_tracker,
        output,
    }));

    // 7. Input: libinput reads evdev devices, gated through the session.
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        udev.borrow().session.clone().into(),
    );
    libinput_context
        .udev_assign_seat(&seat_name)
        .map_err(|_| "failed to assign libinput seat")?;
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    event_loop
        .handle()
        .insert_source(libinput_backend, move |event, _, data| {
            data.state.process_input_event(event)
        })?;

    // 8. Session pause/resume: fires when the user VT-switches away/back.
    //    While paused we hold no DRM master — rendering must stop.
    let udev_session = udev.clone();
    event_loop
        .handle()
        .insert_source(notifier, move |event, &mut (), data| match event {
            SessionEvent::PauseSession => {
                info!("session paused (VT switched away)");
                libinput_context.suspend();
                udev_session.borrow_mut().drm.pause();
            }
            SessionEvent::ActivateSession => {
                info!("session resumed");
                if let Err(err) = libinput_context.resume() {
                    error!(?err, "failed to resume libinput");
                }
                let mut udev = udev_session.borrow_mut();
                if let Err(err) = udev.drm.activate(false) {
                    error!(?err, "failed to re-activate DRM device");
                }
                udev.gbm_surface.reset_buffers();
                render_frame(&mut udev, &mut data.state, &mut data.display_handle);
            }
        })?;

    // 9. VBlank: the display presented our queued buffer; render the next.
    let udev_vblank = udev.clone();
    event_loop
        .handle()
        .insert_source(drm_notifier, move |event, _meta, data| match event {
            DrmEvent::VBlank(_crtc) => {
                let mut udev = udev_vblank.borrow_mut();
                if let Err(err) = udev.gbm_surface.frame_submitted() {
                    warn!(?err, "frame_submitted failed");
                }
                render_frame(&mut udev, &mut data.state, &mut data.display_handle);
            }
            DrmEvent::Error(err) => error!(?err, "DRM error"),
        })?;

    // First frame: kick the render loop; vblanks keep it going after this.
    let udev_first = udev.clone();
    event_loop.handle().insert_idle(move |data| {
        let mut udev = udev_first.borrow_mut();
        render_frame(&mut udev, &mut data.state, &mut data.display_handle);
    });

    Ok(())
}

fn render_frame(udev: &mut UdevData, state: &mut Vitrine, display_handle: &mut DisplayHandle) {
    let UdevData {
        gbm_surface,
        renderer,
        damage_tracker,
        output,
        ..
    } = udev;

    // Acquire the next swapchain buffer. Its `age` tells the damage tracker
    // how many frames ago this buffer was last drawn, i.e. which accumulated
    // damage must be repainted into it.
    let (mut dmabuf, age) = match gbm_surface.next_buffer() {
        Ok(ok) => ok,
        Err(err) => {
            warn!(?err, "failed to acquire buffer");
            return;
        }
    };

    let render_result = {
        let mut framebuffer = match renderer.bind(&mut dmabuf) {
            Ok(fb) => fb,
            Err(err) => {
                warn!(?err, "failed to bind buffer");
                return;
            }
        };
        render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
            output,
            renderer,
            &mut framebuffer,
            1.0,
            age as usize,
            [&state.space],
            &[],
            damage_tracker,
            CLEAR_COLOR,
        )
    };

    match render_result {
        Ok(result) => {
            let damage = result.damage.cloned();
            if let Err(err) = gbm_surface.queue_buffer(None, damage, ()) {
                warn!(?err, "failed to queue buffer");
                return;
            }
        }
        Err(err) => {
            warn!(?err, "render failed");
            return;
        }
    }

    // Pace clients: their next frame may start now.
    state.space.elements().for_each(|window| {
        window.send_frame(
            output,
            state.start_time.elapsed(),
            Some(Duration::ZERO),
            |_, _| Some(output.clone()),
        )
    });

    state.space.refresh();
    state.popups.cleanup();
    let _ = display_handle.flush_clients();
}
