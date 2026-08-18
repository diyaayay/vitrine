mod backend;
mod config;
mod handlers;
mod input;
mod kiosk;
mod perf;
mod state;

use std::{path::PathBuf, time::Duration};

use smithay::reexports::{
    calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
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
    let nested =
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let use_winit = match (
        args.iter().any(|a| a == "--winit"),
        args.iter().any(|a| a == "--tty"),
    ) {
        (true, _) => true,
        (_, true) => false,
        _ => nested,
    };

    // --debug-damage tints every repainted pixel, making damage tracking
    // visible: regions being redrawn flash, untouched regions stay clean.
    let debug_damage = args.iter().any(|a| a == "--debug-damage");

    if use_winit {
        backend::winit::init_winit(&mut event_loop, &mut data, debug_damage)?;
    } else {
        backend::udev::init_udev(&mut event_loop, &mut data, debug_damage)?;
    }

    tracing::info!(
        ?socket_name,
        backend = if use_winit { "winit" } else { "udev" },
        "vitrine is running"
    );

    // Kiosk runtime: load config (explicit --config must exist; ./vitrine.toml
    // is picked up if present), let a -c flag override the command for quick
    // tests, then hand the app to the supervisor.
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let config = match &config_path {
        Some(path) => config::Config::load(path)?,
        None => {
            let default = PathBuf::from("vitrine.toml");
            if default.exists() {
                config::Config::load(&default)?
            } else {
                config::Config::default()
            }
        }
    };

    let cli_command = args
        .iter()
        .position(|a| a == "-c" || a == "--command")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let (command, command_args) = match (cli_command, config.app.command.clone()) {
        (Some(cmd), _) => (Some(cmd), Vec::new()),
        (None, Some(cmd)) => (Some(cmd), config.app.args.clone()),
        (None, None) => (None, Vec::new()),
    };

    if let Some(command) = command {
        data.state.supervisor = Some(kiosk::AppSupervisor::launch(
            command,
            command_args,
            config.app.restart,
            Duration::from_millis(config.app.restart_delay_ms),
            socket_name.clone(),
        ));
        // The watchdog heartbeat: poll the child twice a second.
        event_loop
            .handle()
            .insert_source(
                Timer::from_duration(Duration::from_millis(500)),
                |_, _, data| {
                    if let Some(supervisor) = data.state.supervisor.as_mut() {
                        supervisor.tick();
                    }
                    TimeoutAction::ToDuration(Duration::from_millis(500))
                },
            )
            .map_err(|e| format!("failed to insert watchdog timer: {e}"))?;
    } else {
        tracing::info!("no kiosk app configured (set app.command in vitrine.toml or pass -c)");
    }

    event_loop.run(None, &mut data, move |_| {})?;

    Ok(())
}
