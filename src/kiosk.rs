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

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_until(supervisor: &mut AppSupervisor, cond: impl Fn(&AppSupervisor) -> bool) -> bool {
        // Bounded polling: the watchdog is timer-driven in production, so the
        // tests drive tick() the same way the calloop timer would.
        for _ in 0..200 {
            supervisor.tick();
            if cond(supervisor) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn launch(command: &str, restart: bool) -> AppSupervisor {
        AppSupervisor::launch(
            command.into(),
            Vec::new(),
            restart,
            Duration::ZERO,
            OsString::from("wayland-test"),
        )
    }

    #[test]
    fn watchdog_relaunches_exited_app() {
        let mut supervisor = launch("true", true);
        let first_pid = supervisor.child.as_ref().map(|c| c.id());
        assert!(first_pid.is_some(), "launch must spawn the app");

        // `true` exits immediately; the watchdog must notice and respawn.
        let respawned = wait_until(&mut supervisor, |s| {
            s.child
                .as_ref()
                .map(|c| c.id())
                .is_some_and(|id| Some(id) != first_pid)
        });
        assert!(respawned, "expected a new child pid after exit");
    }

    #[test]
    fn no_restart_means_no_respawn() {
        let mut supervisor = launch("true", false);
        let exited = wait_until(&mut supervisor, |s| s.child.is_none());
        assert!(exited, "child exit must be detected");

        for _ in 0..10 {
            supervisor.tick();
        }
        assert!(supervisor.child.is_none(), "restart=false must stay down");
        assert!(supervisor.respawn_at.is_none());
    }

    #[test]
    fn failed_spawn_schedules_retry() {
        let supervisor = launch("/nonexistent/vitrine-test-binary", true);
        assert!(supervisor.child.is_none());
        assert!(
            supervisor.respawn_at.is_some(),
            "a failed launch must be retried, not abandoned"
        );
    }
}
