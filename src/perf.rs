//! Frame-timing statistics — the numbers behind the compositor's performance
//! story: how long compositing takes, and how often damage tracking lets us
//! skip repainting entirely.

use std::time::{Duration, Instant};

use tracing::info;

const REPORT_EVERY: Duration = Duration::from_secs(5);

pub struct FrameStats {
    window_start: Instant,
    frames: u32,
    idle_frames: u32,
    total_render: Duration,
    max_render: Duration,
}

impl FrameStats {
    pub fn new() -> Self {
        Self {
            window_start: Instant::now(),
            frames: 0,
            idle_frames: 0,
            total_render: Duration::ZERO,
            max_render: Duration::ZERO,
        }
    }

    /// Record one frame: how long compositing took and whether anything
    /// actually needed repainting. Logs an aggregate line every few seconds.
    pub fn record(&mut self, render_time: Duration, repainted: bool) {
        self.frames += 1;
        if !repainted {
            self.idle_frames += 1;
        }
        self.total_render += render_time;
        self.max_render = self.max_render.max(render_time);

        let elapsed = self.window_start.elapsed();
        if elapsed >= REPORT_EVERY {
            let fps = f64::from(self.frames) / elapsed.as_secs_f64();
            let avg_ms = (self.total_render / self.frames).as_secs_f64() * 1000.0;
            let max_ms = self.max_render.as_secs_f64() * 1000.0;
            let idle_pct = 100.0 * f64::from(self.idle_frames) / f64::from(self.frames);
            info!(
                fps = format_args!("{fps:.1}"),
                avg_render_ms = format_args!("{avg_ms:.2}"),
                max_render_ms = format_args!("{max_ms:.2}"),
                idle_frames = format_args!("{idle_pct:.0}%"),
                "frame stats"
            );
            *self = Self::new();
        }
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}
