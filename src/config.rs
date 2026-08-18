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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_yields_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.app.command, None);
        assert!(config.app.args.is_empty());
        assert!(config.app.restart, "watchdog must default to on");
        assert_eq!(config.app.restart_delay_ms, 1000);
    }

    #[test]
    fn full_config_parses() {
        let config: Config = toml::from_str(
            r#"
            [app]
            command = "cog"
            args = ["--fullscreen", "https://example.com"]
            restart = false
            restart_delay_ms = 250
            "#,
        )
        .unwrap();
        assert_eq!(config.app.command.as_deref(), Some("cog"));
        assert_eq!(config.app.args, ["--fullscreen", "https://example.com"]);
        assert!(!config.app.restart);
        assert_eq!(config.app.restart_delay_ms, 250);
    }

    #[test]
    fn partial_config_keeps_other_defaults() {
        let config: Config = toml::from_str("[app]\ncommand = \"foot\"\n").unwrap();
        assert_eq!(config.app.command.as_deref(), Some("foot"));
        assert!(config.app.restart, "unset fields must keep their defaults");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        // A typo like `comand` must fail loudly at startup, not silently
        // launch nothing on a kiosk nobody is watching.
        let err = toml::from_str::<Config>("[app]\ncomand = \"foot\"\n");
        assert!(err.is_err());
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(toml::from_str::<Config>("[app\ncommand=").is_err());
    }
}
