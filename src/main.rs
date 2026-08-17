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

    // Backend selection: inside an existing session (a Wayland or X display
    // is reachable) we nest via winit; on a bare TTY we take the hardware
    // path. --winit / --tty force either.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let use_winit = match (
        args.iter().any(|a| a == "--winit"),
        args.iter().any(|a| a == "--tty"),
    ) {
        (true, _) => true,
        (_, true) => false,
        _ => nested,
    };

    if use_winit {
        backend::winit::init_winit(&mut event_loop, &mut data)?;
    } else {
        backend::udev::init_udev(&mut event_loop, &mut data)?;
    }

    tracing::info!(?socket_name, backend = if use_winit { "winit" } else { "udev" }, "vitrine is running");

    // Kiosk behavior: launch the configured application as a child, wired to
    // our socket. (TOML config + watchdog restart land in a later checkpoint.)
    let command = args
        .iter()
        .position(|a| a == "-c" || a == "--command")
        .and_then(|i| args.get(i + 1));
    if let Some(command) = command {
        std::process::Command::new(command)
            .env("WAYLAND_DISPLAY", &socket_name)
            .spawn()
            .map_err(|e| tracing::warn!("failed to launch {command}: {e}"))
            .ok();
    }

    event_loop.run(None, &mut data, move |_| {})?;

    Ok(())
}
