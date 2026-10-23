//! Smooth scrolling. Wheel input adds distance to a backlog, which a thread
//! releases as small pixel scroll events with an ease-out curve, so a notch
//! glides instead of jumping.

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_graphics::event::{CGEvent, CGEventTapLocation, EventField, ScrollEventUnit};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

const FRAME: Duration = Duration::from_millis(8);
/// Share of the remaining distance released each frame.
const EASE: f64 = 0.28;

pub struct Scroller {
    tx: Sender<(f64, f64)>,
}

impl Scroller {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        std::thread::spawn(move || run(rx));
        Self { tx }
    }

    /// Queue a scroll in pixels: positive `dy` scrolls content down (towards
    /// the top of the document), positive `dx` towards its left edge,
    /// matching CGEvent wheel axes.
    pub fn scroll(&self, dy: f64, dx: f64) {
        let _ = self.tx.send((dy, dx));
    }
}

fn run(rx: Receiver<(f64, f64)>) {
    let (mut y, mut x) = (0.0f64, 0.0f64);
    // Fractions of a pixel carried to the next frame.
    let (mut carry_y, mut carry_x) = (0.0f64, 0.0f64);
    loop {
        let idle = y.abs() < 0.5 && x.abs() < 0.5;
        let msg = if idle {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(FRAME)
        };
        match msg {
            Ok((dy, dx)) => {
                y += dy;
                x += dx;
                // Drain anything else queued before emitting a frame.
                while let Ok((dy, dx)) = rx.try_recv() {
                    y += dy;
                    x += dx;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let step_y = step(y);
        let step_x = step(x);
        y -= step_y;
        x -= step_x;
        carry_y += step_y;
        carry_x += step_x;
        let (py, px) = (carry_y.trunc(), carry_x.trunc());
        carry_y -= py;
        carry_x -= px;
        if py != 0.0 || px != 0.0 {
            post(py as i32, px as i32);
        }
    }
}

fn step(remaining: f64) -> f64 {
    if remaining.abs() <= 1.0 {
        remaining
    } else {
        remaining * EASE
    }
}

fn post(dy: i32, dx: i32) {
    let Ok(src) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
        return;
    };
    if let Ok(ev) = CGEvent::new_scroll_event(src, ScrollEventUnit::PIXEL, 2, dy, dx, 0) {
        // Continuous (trackpad-style) events make apps scroll by exactly
        // these pixels instead of applying line-based acceleration.
        ev.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 1);
        ev.post(CGEventTapLocation::HID);
    }
}

/// macOS "Natural scrolling" (System Settings > Mouse); on by default.
pub fn natural_scrolling() -> bool {
    use core_foundation_sys::preferences::{
        CFPreferencesGetAppBooleanValue, kCFPreferencesAnyApplication,
    };
    let key = CFString::new("com.apple.swipescrolldirection");
    let mut valid = 0u8;
    let value = unsafe {
        CFPreferencesGetAppBooleanValue(
            key.as_concrete_TypeRef(),
            kCFPreferencesAnyApplication,
            &mut valid,
        )
    };
    valid == 0 || value != 0
}
