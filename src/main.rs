mod backend;
mod handlers;
mod input;
mod state;

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
pub use state::Vitrine;

pub struct CalloopData {
    state: Vitrine,
    display_handle: DisplayHandle,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;

    let display: Display<Vitrine> = Display::new()?;
    let display_handle = display.handle();
    let state = Vitrine::new(&mut event_loop, display);

    let socket_name = state.socket_name.clone();

    let mut data = CalloopData {
        state,
        display_handle,
    };

    backend::winit::init_winit(&mut event_loop, &mut data)?;

    tracing::info!(?socket_name, "vitrine is running");

    // Kiosk behavior: launch the configured application as a child, wired to
    // our socket. (TOML config + watchdog restart land in a later checkpoint.)
    let mut args = std::env::args().skip(1);
    if let (Some("-c" | "--command"), Some(command)) = (args.next().as_deref(), args.next()) {
        std::process::Command::new(&command)
            .env("WAYLAND_DISPLAY", &socket_name)
            .spawn()
            .map_err(|e| tracing::warn!("failed to launch {command}: {e}"))
            .ok();
    }

    event_loop.run(None, &mut data, move |_| {})?;

    Ok(())
}
