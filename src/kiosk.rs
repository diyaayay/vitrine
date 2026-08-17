//! The application supervisor — the part that makes vitrine a *kiosk* rather
//! than just a compositor: it owns the app's lifecycle, relaunching it on
//! exit and killing it on shutdown.

use std::{
    ffi::OsString,
    process::{Child, Command},
    time::{Duration, Instant},
};

use tracing::{info, warn};

pub struct AppSupervisor {
    command: String,
    args: Vec<String>,
    restart: bool,
    restart_delay: Duration,
    socket_name: OsString,
    child: Option<Child>,
    respawn_at: Option<Instant>,
}

impl AppSupervisor {
    pub fn launch(
        command: String,
        args: Vec<String>,
        restart: bool,
        restart_delay: Duration,
        socket_name: OsString,
    ) -> Self {
        let mut supervisor = Self {
            command,
            args,
            restart,
            restart_delay,
            socket_name,
            child: None,
            respawn_at: None,
        };
        supervisor.spawn();
        supervisor
    }

    fn spawn(&mut self) {
        let result = Command::new(&self.command)
            .args(&self.args)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            // Never let the app fall back to an X server that may exist in
            // the environment — the kiosk app talks to vitrine or nothing.
            .env_remove("DISPLAY")
            .spawn();
        match result {
            Ok(child) => {
                info!(command = %self.command, pid = child.id(), "kiosk app launched");
                self.child = Some(child);
            }
            Err(err) => {
                warn!(command = %self.command, %err, "failed to launch kiosk app");
                self.schedule_respawn();
            }
        }
    }

    fn schedule_respawn(&mut self) {
        if self.restart {
            self.respawn_at = Some(Instant::now() + self.restart_delay);
        }
    }

    /// Poll the child; driven by a calloop timer.
    pub fn tick(&mut self) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    warn!(%status, "kiosk app exited");
                    self.child = None;
                    self.schedule_respawn();
                }
                Ok(None) => {} // still running
                Err(err) => warn!(%err, "failed to poll kiosk app"),
            }
        } else if self.respawn_at.is_some_and(|at| Instant::now() >= at) {
            self.respawn_at = None;
            info!("watchdog relaunching kiosk app");
            self.spawn();
        }
    }
}

impl Drop for AppSupervisor {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
