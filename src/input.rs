use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, Event, InputBackend, InputEvent, KeyState,
            KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        },
        session::Session,
    },
    input::{
        keyboard::{keysyms as xkb, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::SERIAL_COUNTER,
};
use tracing::info;

use crate::state::Vitrine;

/// Compositor-level keybindings, intercepted before clients ever see the key.
/// A kiosk forwards *everything* to the application — these two chords are
/// development/maintenance hatches, the kiosk equivalent of a service door.
enum KeyAction {
    /// Ctrl+Alt+F1..F12 (xkb reports these as XF86Switch_VT_n)
    VtSwitch(i32),
    /// Ctrl+Alt+Q
    Quit,
}

impl Vitrine {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let pressed = event.state() == KeyState::Pressed;

                let action = self.seat.get_keyboard().unwrap().input::<KeyAction, _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, modifiers, handle| {
                        if pressed {
                            let sym = handle.modified_sym().raw();
                            if (xkb::KEY_XF86Switch_VT_1..=xkb::KEY_XF86Switch_VT_12).contains(&sym)
                            {
                                return FilterResult::Intercept(KeyAction::VtSwitch(
                                    (sym - xkb::KEY_XF86Switch_VT_1 + 1) as i32,
                                ));
                            }
                            if modifiers.ctrl
                                && modifiers.alt
                                && (sym == xkb::KEY_q || sym == xkb::KEY_Q)
                            {
                                return FilterResult::Intercept(KeyAction::Quit);
                            }
                        }
                        FilterResult::Forward
                    },
                );

                match action {
                    Some(KeyAction::VtSwitch(vt)) => {
                        if let Some(session) = self.session.as_mut() {
                            info!(vt, "switching VT");
                            let _ = session.change_vt(vt);
                        }
                    }
                    Some(KeyAction::Quit) => {
                        info!("exit chord pressed, shutting down");
                        self.loop_signal.stop();
                    }
                    None => {}
                }
            }
            // Relative motion: what a physical mouse on the TTY produces.
            // We integrate deltas ourselves and clamp to the output.
            InputEvent::PointerMotion { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let mut pos = pointer.current_location() + event.delta();
                if let Some(output) = self.space.outputs().next() {
                    let geo = self.space.output_geometry(output).unwrap();
                    pos.x = pos
                        .x
                        .clamp(geo.loc.x as f64, (geo.loc.x + geo.size.w) as f64);
                    pos.y = pos
                        .y
                        .clamp(geo.loc.y as f64, (geo.loc.y + geo.size.h) as f64);
                }

                let serial = SERIAL_COUNTER.next_serial();
                let under = self.surface_under(pos);
                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            // Absolute positioning: winit windows and touchscreens.
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.space.outputs().next() else {
                    return;
                };
                let output_geo = self.space.output_geometry(output).unwrap();
                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();
                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                // Kiosk policy: clicks are forwarded, never used for focus —
                // focus is owned by the stacking order (see focus_topmost).
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();

                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = event.amount_v120(Axis::Horizontal) {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = event.amount_v120(Axis::Vertical) {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}
