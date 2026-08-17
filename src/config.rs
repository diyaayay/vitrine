//! Kiosk configuration, loaded from a TOML file.
//!
//! Search order: `--config <path>` > `./vitrine.toml` > built-in defaults.
//! A `-c <command>` CLI flag overrides `app.command` for quick testing.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub app: AppConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// The program shown fullscreen. `None` leaves vitrine idling on a blank
    /// screen (clients can still connect manually via WAYLAND_DISPLAY).
    pub command: Option<String>,
    pub args: Vec<String>,
    /// The watchdog: relaunch the app whenever it exits. On an unattended
    /// device there is nobody else to do it.
    pub restart: bool,
    /// Delay before a relaunch, so a crash-looping app cannot busy-spin.
    pub restart_delay_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            command: None,
            args: Vec::new(),
            restart: true,
            restart_delay_ms: 1000,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
    }
}
