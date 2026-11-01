//! Device worker: finds the mouse, applies the configuration, diverts the
//! configured controls and turns their events into actions. Runs on its own
//! thread; the UI talks to it through `Command` and receives `Status`.

use crate::actions::{self, Action};
use crate::config::Config;
use crate::hidpp;
use crate::mouse::{self, Battery, Event, ForceLevel, Mouse, SmartShift, WheelMode, cid, feat};
use crate::scroll::{self, Scroller};
use anyhow::Result;
use hidapi::HidApi;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static DEBUG: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var_os("CLEANLOGI_DEBUG").is_some());

const POLL: Duration = Duration::from_millis(100);
const RESCAN: Duration = Duration::from_secs(2);
const BATTERY_REFRESH: Duration = Duration::from_secs(300);
const CONFIG_CHECK: Duration = Duration::from_secs(1);
/// A pause this long ends a thumbwheel roll, in case the mouse doesn't
/// report the start of the next one.
const THUMB_ROLL_GAP: Duration = Duration::from_millis(300);

#[derive(Debug)]
pub enum Command {
    SetDpi(u16),
    SetScrollMode(WheelMode),
    SetSmartShift(bool),
    SetHapticLevel(Option<u8>),
    ChangeHost(u8),
    /// Re-read config.toml and push it to the mouse.
    Reload,
    Quit,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub device: Option<String>,
    pub battery: Option<Battery>,
    pub dpi: Option<u16>,
    pub dpi_values: Vec<u16>,
    pub haptics: bool,
    pub hosts: Option<(u8, u8)>,
    pub host_names: Vec<String>,
    pub config: Config,
}

macro_rules! log {
    ($($t:tt)*) => { eprintln!("[cleanlogi] {}", format!($($t)*)) };
}
pub(crate) use log;

/// Parsed actions for every configurable control.
struct Bindings {
    middle: Action,
    back: Action,
    forward: Action,
    mode_shift: Action,
    haptic_panel: Action,
    gesture: [Action; 5], // click, up, down, left, right
    thumb_left: Action,
    thumb_right: Action,
}

impl Bindings {
    fn from(cfg: &Config) -> Self {
        let p = |s: &str| {
            Action::parse(s).unwrap_or_else(|e| {
                log!("ignoring action {s:?}: {e}");
                Action::None
            })
        };
        let g = &cfg.gesture;
        Self {
            middle: p(&cfg.buttons.middle),
            back: p(&cfg.buttons.back),
            forward: p(&cfg.buttons.forward),
            mode_shift: p(&cfg.buttons.mode_shift),
            haptic_panel: p(&cfg.buttons.haptic_panel),
            gesture: [p(&g.click), p(&g.up), p(&g.down), p(&g.left), p(&g.right)],
            thumb_left: p(&cfg.thumbwheel.left),
            thumb_right: p(&cfg.thumbwheel.right),
        }
    }

    fn for_cid(&self, c: u16) -> Option<&Action> {
        Some(match c {
            cid::MIDDLE => &self.middle,
            cid::BACK => &self.back,
            cid::FORWARD => &self.forward,
            cid::MODE_SHIFT => &self.mode_shift,
            cid::HAPTIC_PANEL => &self.haptic_panel,
            _ => return None,
        })
    }

    fn gesture_diverted(&self) -> bool {
        self.gesture.iter().any(|a| *a != Action::Native)
    }
}

struct Worker<F: Fn(Status)> {
    cfg: Config,
    cfg_mtime: Option<SystemTime>,
    bindings: Bindings,
    status: Status,
    notify: F,
    scroller: Scroller,
    /// Main wheel / thumbwheel movement is diverted to us for smooth scrolling.
    wheel_diverted: bool,
    thumb_scrolls: bool,
    /// Hi-res wheel units per notch.
    wheel_units: f64,
    natural: bool,
    accessibility: bool,
}

pub fn run(rx: Receiver<Command>, notify: impl Fn(Status)) {
    let cfg = Config::load().unwrap_or_else(|e| {
        log!("config error, using defaults: {e:#}");
        Config::default()
    });
    let mut w = Worker {
        bindings: Bindings::from(&cfg),
        status: Status {
            config: cfg.clone(),
            ..Default::default()
        },
        cfg_mtime: mtime(),
        cfg,
        notify,
        scroller: Scroller::new(),
        wheel_diverted: false,
        thumb_scrolls: false,
        wheel_units: 1.0,
        natural: scroll::natural_scrolling(),
        accessibility: actions::accessibility_granted(),
    };
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => return log!("cannot initialise HID: {e}"),
    };
    (w.notify)(w.status.clone());
    while !SHUTDOWN.load(Ordering::Relaxed) {
        let _ = api.refresh_devices();
        if let Some(info) = hidpp::enumerate(&api).into_iter().next() {
            match w.session(&api, &info, &rx) {
                Ok(true) => break, // quit requested
                Ok(false) => {}
                Err(e) => log!("device session ended: {e:#}"),
            }
            w.status.device = None;
            w.status.battery = None;
            w.status.dpi = None;
            w.status.dpi_values.clear();
            w.status.haptics = false;
            w.status.hosts = None;
            w.status.host_names.clear();
            (w.notify)(w.status.clone());
        }
        // Not connected: still honour menu commands so settings persist.
        let until = Instant::now() + RESCAN;
        while Instant::now() < until {
            match rx.recv_timeout(POLL) {
                Ok(Command::Quit) => return,
                Ok(cmd) => w.update_config(&cmd),
                Err(_) => {}
            }
            if SHUTDOWN.load(Ordering::Relaxed) {
                return;
            }
        }
    }
}

fn mtime() -> Option<SystemTime> {
    std::fs::metadata(crate::config::path())
        .and_then(|m| m.modified())
        .ok()
}

/// Per-connection input state.
#[derive(Default)]
struct Input {
    held: Vec<u16>,
    gesture_active: bool,
    gesture_fired: bool,
    gx: i32,
    gy: i32,
    thumb_acc: i32,
    /// The current thumbwheel roll has run its one action (`repeat` off).
    thumb_fired: bool,
    thumb_last: Option<Instant>,
}

impl<F: Fn(Status)> Worker<F> {
    /// Returns Ok(true) when the app should quit.
    fn session(
        &mut self,
        api: &HidApi,
        info: &hidpp::DeviceInfo,
        rx: &Receiver<Command>,
    ) -> Result<bool> {
        let mouse = Mouse::new(hidpp::Device::open(api, info)?)?;
        let name = mouse.name().unwrap_or_else(|_| info.name.clone());
        log!("connected: {name}");
        self.status.device = Some(name);
        self.apply(&mouse);
        let mut input = Input::default();
        let mut last_battery = Instant::now();
        let mut last_cfg_check = Instant::now();
        let result = loop {
            if SHUTDOWN.load(Ordering::Relaxed) {
                break Ok(true);
            }
            match rx.try_recv() {
                Ok(Command::Quit) | Err(TryRecvError::Disconnected) => break Ok(true),
                Ok(cmd) => self.command(&mouse, cmd),
                Err(TryRecvError::Empty) => {}
            }
            if last_cfg_check.elapsed() > CONFIG_CHECK {
                last_cfg_check = Instant::now();
                self.natural = scroll::natural_scrolling();
                if mtime() != self.cfg_mtime {
                    self.reload(&mouse);
                } else if actions::accessibility_granted() != self.accessibility {
                    // Smooth scrolling depends on it: take over or hand back the wheel.
                    self.apply(&mouse);
                }
            }
            if last_battery.elapsed() > BATTERY_REFRESH {
                last_battery = Instant::now();
                if let Ok(b) = mouse.battery() {
                    self.status.battery = Some(b);
                    (self.notify)(self.status.clone());
                }
            }
            match mouse.next_event(POLL) {
                Ok(Some(ev)) => self.event(&mouse, &mut input, ev),
                Ok(None) => {}
                Err(e) => break Err(e),
            }
        };
        if matches!(result, Ok(true)) {
            restore(&mouse);
        }
        result
    }

    /// Push the whole configuration to the device. Every step is independent
    /// so one unsupported setting doesn't block the rest.
    fn apply(&mut self, m: &Mouse) {
        // Posting scroll events needs Accessibility; without it the wheels
        // stay with macOS so scrolling never stops working.
        self.accessibility = actions::accessibility_granted();
        self.natural = scroll::natural_scrolling();
        self.wheel_diverted =
            self.cfg.scroll.smooth && self.accessibility && m.has(feat::HIRES_WHEEL);
        self.thumb_scrolls =
            matches!(self.cfg.thumbwheel.mode.as_str(), "scroll") && self.accessibility;
        if self.wheel_diverted {
            self.wheel_units = m.wheel_multiplier() as f64;
        }
        let c = &self.cfg;
        let b = &self.bindings;
        let step = |what: &str, r: Result<()>| {
            if let Err(e) = r {
                log!("{what}: {e:#}");
            }
        };
        step("dpi", apply_dpi(m, &mut self.status, c.dpi));
        if m.has(feat::SMARTSHIFT) || m.has(feat::SMARTSHIFT_LEGACY) {
            step("smartshift", m.set_smartshift(smartshift_of(c)));
        }
        if m.has(feat::HIRES_WHEEL) {
            let d = self.wheel_diverted;
            step("wheel", m.set_wheel_mode(d, d, !d && c.scroll.invert));
        }
        if m.has(feat::THUMBWHEEL) {
            // Diverted modes apply `invert` themselves; native lets the mouse do it.
            let divert = self.thumb_scrolls || c.thumbwheel.mode == "actions";
            let invert = !divert && c.thumbwheel.invert;
            step("thumbwheel", m.set_thumbwheel(divert, invert));
        }
        self.status.haptics = m.has(feat::HAPTICS);
        if self.status.haptics {
            step("haptics", m.set_haptics(c.haptics.enabled, c.haptics.level));
        }
        if m.has(feat::FORCE_BUTTON) {
            let level = match c.haptics.panel_force.as_str() {
                "light" => ForceLevel::Light,
                "firm" => ForceLevel::Firm,
                _ => ForceLevel::Medium,
            };
            step("force", m.set_force(level));
        }
        if m.has(feat::REPROG) {
            let present = m.controls().unwrap_or_default();
            if *DEBUG {
                log!("controls {present:04x?}");
            }
            for c in [
                cid::MIDDLE,
                cid::BACK,
                cid::FORWARD,
                cid::MODE_SHIFT,
                cid::HAPTIC_PANEL,
            ] {
                if present.contains(&c) {
                    let divert = b.for_cid(c).is_some_and(|a| *a != Action::Native);
                    step(&format!("divert {c:#x}"), m.set_divert(c, divert, false));
                }
            }
            if present.contains(&cid::GESTURE) {
                let d = b.gesture_diverted();
                step("divert gesture", m.set_divert(cid::GESTURE, d, d));
            }
        }
        self.status.battery = m.battery().ok();
        self.status.hosts = m.hosts().ok();
        self.status.host_names = match self.status.hosts {
            Some((n, _)) if m.has(feat::HOSTS_INFO) => m.host_names(n),
            _ => Vec::new(),
        };
        self.status.config = self.cfg.clone();
        (self.notify)(self.status.clone());
    }

    fn reload(&mut self, m: &Mouse) {
        self.cfg_mtime = mtime();
        match Config::load() {
            Ok(c) if c != self.cfg => {
                log!("config reloaded");
                self.bindings = Bindings::from(&c);
                self.cfg = c;
                self.apply(m);
            }
            Ok(_) => {}
            Err(e) => log!("config not reloaded: {e:#}"),
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.cfg.save() {
            log!("saving config: {e:#}");
        }
        self.cfg_mtime = mtime();
        self.status.config = self.cfg.clone();
    }

    /// Record a menu change in the config (used whether or not connected).
    fn update_config(&mut self, cmd: &Command) {
        match *cmd {
            Command::SetDpi(d) => self.cfg.dpi = d,
            Command::SetScrollMode(m) => {
                self.cfg.scroll.mode = match m {
                    WheelMode::Ratchet => "ratchet".into(),
                    WheelMode::FreeSpin => "freespin".into(),
                }
            }
            Command::SetSmartShift(on) => self.cfg.scroll.smartshift = on,
            Command::SetHapticLevel(None) => self.cfg.haptics.enabled = false,
            Command::SetHapticLevel(Some(l)) => {
                self.cfg.haptics.enabled = true;
                self.cfg.haptics.level = l;
            }
            Command::Reload => {
                match Config::load() {
                    Ok(c) => {
                        self.bindings = Bindings::from(&c);
                        self.cfg = c;
                        self.cfg_mtime = mtime();
                        self.status.config = self.cfg.clone();
                        log!("config reloaded");
                    }
                    Err(e) => log!("config not reloaded: {e:#}"),
                }
                return;
            }
            Command::ChangeHost(_) | Command::Quit => return,
        }
        self.save();
        (self.notify)(self.status.clone());
    }

    fn command(&mut self, m: &Mouse, cmd: Command) {
        self.update_config(&cmd);
        let c = &self.cfg;
        let r = match cmd {
            Command::SetDpi(d) => apply_dpi(m, &mut self.status, d).map(|_| {
                (self.notify)(self.status.clone());
            }),
            Command::SetScrollMode(_) | Command::SetSmartShift(_) => {
                m.set_smartshift(smartshift_of(c))
            }
            Command::SetHapticLevel(_) if m.has(feat::HAPTICS) => m
                .set_haptics(c.haptics.enabled, c.haptics.level)
                .and_then(|_| {
                    if c.haptics.enabled {
                        m.play_waveform(mouse::waveform::SHARP_STATE_CHANGE)
                    } else {
                        Ok(())
                    }
                }),
            Command::SetHapticLevel(_) => Ok(()),
            Command::ChangeHost(h) => m.change_host(h),
            Command::Reload => {
                self.apply(m);
                Ok(())
            }
            Command::Quit => Ok(()),
        };
        if let Err(e) = r {
            log!("command failed: {e:#}");
        }
    }

    fn event(&mut self, m: &Mouse, input: &mut Input, ev: Event) {
        if *DEBUG {
            log!("event {ev:?}");
        }
        match ev {
            Event::Buttons(held) => {
                let new: Vec<u16> = held
                    .iter()
                    .copied()
                    .filter(|c| !input.held.contains(c))
                    .collect();
                for c in new {
                    self.pressed(m, input, c);
                }
                if input.gesture_active && !held.contains(&cid::GESTURE) {
                    input.gesture_active = false;
                    if !input.gesture_fired {
                        let a = self.bindings.gesture[0].clone();
                        self.run(m, &a);
                    }
                }
                input.held = held;
            }
            Event::RawXY(dx, dy) if input.gesture_active && !input.gesture_fired => {
                input.gx += dx as i32;
                input.gy += dy as i32;
                let t = self.cfg.gesture.threshold.max(1) as i32;
                let dir = if input.gx.abs() >= input.gy.abs() {
                    match input.gx {
                        x if x >= t => Some(4),
                        x if x <= -t => Some(3),
                        _ => None,
                    }
                } else {
                    match input.gy {
                        y if y >= t => Some(2),
                        y if y <= -t => Some(1),
                        _ => None,
                    }
                };
                if let Some(i) = dir {
                    input.gesture_fired = true;
                    let a = self.bindings.gesture[i].clone();
                    self.run(m, &a);
                    if m.has(feat::HAPTICS)
                        && self.cfg.haptics.enabled
                        && self.cfg.haptics.on_gesture
                    {
                        let w = mouse::waveform::from_name(&self.cfg.haptics.gesture_waveform)
                            .unwrap_or(mouse::waveform::SHARP_STATE_CHANGE);
                        let _ = m.play_waveform(w);
                    }
                }
            }
            Event::Wheel { delta, hires } if self.wheel_diverted => {
                let notches = delta as f64 / if hires { self.wheel_units } else { 1.0 };
                let px = notches * px_per_notch(self.cfg.scroll.speed);
                // Rolling away from you scrolls up (towards the top) unless
                // natural scrolling is on.
                let px = px * direction(self.natural, self.cfg.scroll.invert);
                if scroll::shift_held() {
                    self.scroller.scroll(0.0, px);
                } else {
                    self.scroller.scroll(px, 0.0);
                }
            }
            Event::Thumbwheel { rotation, .. } if self.thumb_scrolls => {
                let px = rotation as f64 * px_per_thumb_unit(self.cfg.thumbwheel.speed);
                self.scroller.scroll(
                    0.0,
                    px * direction(self.natural, self.cfg.thumbwheel.invert),
                );
            }
            Event::Thumbwheel { rotation, status } => {
                if status == 1
                    || input
                        .thumb_last
                        .is_none_or(|t| t.elapsed() > THUMB_ROLL_GAP)
                {
                    input.thumb_acc = 0;
                    input.thumb_fired = false;
                }
                input.thumb_last = Some(Instant::now());
                let rotation = if self.cfg.thumbwheel.invert {
                    -rotation
                } else {
                    rotation
                };
                input.thumb_acc += rotation as i32;
                let step = self.cfg.thumbwheel.step.max(1) as i32;
                // Positive rotation is a roll towards the back of the mouse,
                // which Options+ treats as "left". Without `repeat` the rest
                // of the roll is ignored once its action has run.
                while input.thumb_acc >= step && !input.thumb_fired {
                    input.thumb_acc -= step;
                    input.thumb_fired = !self.cfg.thumbwheel.repeat;
                    let a = self.bindings.thumb_left.clone();
                    self.run(m, &a);
                }
                while input.thumb_acc <= -step && !input.thumb_fired {
                    input.thumb_acc += step;
                    input.thumb_fired = !self.cfg.thumbwheel.repeat;
                    let a = self.bindings.thumb_right.clone();
                    self.run(m, &a);
                }
            }
            Event::Battery(b) => {
                self.status.battery = Some(b);
                (self.notify)(self.status.clone());
            }
            Event::Reconnected => {
                log!("device reconnected, reapplying settings");
                *input = Input::default();
                self.apply(m);
            }
            _ => {}
        }
    }

    fn pressed(&mut self, m: &Mouse, input: &mut Input, c: u16) {
        if c == cid::GESTURE {
            input.gesture_active = true;
            input.gesture_fired = false;
            input.gx = 0;
            input.gy = 0;
            return;
        }
        if let Some(a) = self.bindings.for_cid(c).cloned() {
            self.run(m, &a);
        }
    }

    fn run(&mut self, m: &Mouse, a: &Action) {
        if *DEBUG {
            log!("action {a:?}");
        }
        match a {
            Action::SmartShiftToggle => {
                let mode = match self.cfg.scroll.mode.as_str() {
                    "ratchet" => WheelMode::FreeSpin,
                    _ => WheelMode::Ratchet,
                };
                self.command(m, Command::SetScrollMode(mode));
            }
            Action::DpiCycle => {
                if let Some(next) = next_dpi(
                    &self.cfg.dpi_presets,
                    &self.status.dpi_values,
                    self.status.dpi.unwrap_or(self.cfg.dpi),
                ) {
                    self.command(m, Command::SetDpi(next));
                }
            }
            other => other.perform(),
        }
    }
}

/// Hand every control back to the device so it keeps working after exit.
fn restore<T: hidpp::Transport>(m: &Mouse<T>) {
    log!("restoring native behaviour");
    let present = m.controls().unwrap_or_else(|e| {
        log!("restoring controls: {e:#}");
        Vec::new()
    });
    for c in [
        cid::MIDDLE,
        cid::BACK,
        cid::FORWARD,
        cid::MODE_SHIFT,
        cid::HAPTIC_PANEL,
        cid::GESTURE,
    ] {
        if present.contains(&c) {
            let _ = m.set_divert(c, false, false);
        }
    }
    if m.has(feat::THUMBWHEEL) {
        let _ = m.set_thumbwheel(false, false);
    }
    if m.has(feat::HIRES_WHEEL) {
        let _ = m.set_wheel_mode(false, false, false);
    }
}

fn apply_dpi<T: hidpp::Transport>(m: &Mouse<T>, status: &mut Status, dpi: u16) -> Result<()> {
    let values = m.dpi_values()?;
    let applied = m.set_dpi(dpi)?;
    status.dpi_values = values;
    status.dpi = Some(applied);
    Ok(())
}

fn next_dpi(presets: &[u16], supported: &[u16], current: u16) -> Option<u16> {
    let mut values = presets.iter().copied().filter(|d| supported.contains(d));
    values
        .clone()
        .find(|&d| d > current)
        .or_else(|| values.next())
}

fn smartshift_of(c: &Config) -> SmartShift {
    SmartShift {
        mode: if c.scroll.mode == "ratchet" {
            WheelMode::Ratchet
        } else {
            WheelMode::FreeSpin
        },
        threshold: if c.scroll.smartshift {
            mouse::sensitivity_to_threshold(c.scroll.smartshift_sensitivity)
        } else {
            0xFF
        },
        torque: c.scroll.ratchet_force.clamp(1, 100),
    }
}

/// Pixels per wheel notch for `speed` 1..=100 (50 ≈ three lines).
fn px_per_notch(speed: u8) -> f64 {
    10.0 + speed.clamp(1, 100) as f64 * 0.8
}

/// Pixels per thumbwheel unit (120 units = one full turn); 50 ≈ 600 px/turn.
fn px_per_thumb_unit(speed: u8) -> f64 {
    speed.clamp(1, 100) as f64 / 10.0
}

fn direction(natural: bool, invert: bool) -> f64 {
    if natural ^ invert { -1.0 } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_status_recovers_from_capability_errors() {
        let m = mouse::tests::mouse(&[feat::DPI]);
        let mut status = Status::default();
        status.config.dpi = 8000;
        m.dev.fail(feat::DPI, 1, &[0]);
        assert!(apply_dpi(&m, &mut status, 8000).is_err());
        assert!(status.dpi_values.is_empty());
        assert_eq!(status.dpi, None);

        m.dev
            .expect(feat::DPI, 1, &[0], &[0, 0, 200, 0xE0, 50, 0x0F, 0xA0]);
        m.dev.expect(feat::DPI, 3, &[0, 0x0F, 0xA0], &[]);
        apply_dpi(&m, &mut status, 8000).unwrap();
        assert_eq!(status.dpi, Some(4000));
        assert_eq!(status.config.dpi, 8000);
        assert!(status.dpi_values.contains(&800));
        assert!(!status.dpi_values.contains(&8000));
        assert_eq!(
            next_dpi(&[800, 4000, 8000], &status.dpi_values, 4000),
            Some(800)
        );

        // Failed writes preserve the last known applied DPI and capabilities.
        m.dev.fail(feat::DPI, 3, &[0, 0x03, 0x20]);
        assert!(apply_dpi(&m, &mut status, 800).is_err());
        assert_eq!(status.dpi, Some(4000));
        assert!(status.dpi_values.contains(&800));
        m.dev.expect(feat::DPI, 3, &[0, 0x03, 0x20], &[]);
        apply_dpi(&m, &mut status, 800).unwrap();
        assert_eq!(status.dpi, Some(800));
    }

    #[test]
    fn restore_uses_cached_controls_and_continues_after_write_errors() {
        let m = mouse::tests::mouse(&[feat::REPROG, feat::THUMBWHEEL, feat::HIRES_WHEEL]);
        let controls = [
            cid::MIDDLE,
            cid::BACK,
            cid::FORWARD,
            cid::MODE_SHIFT,
            cid::GESTURE,
        ];
        m.dev.expect(feat::REPROG, 0, &[], &[controls.len() as u8]);
        for (i, c) in controls.iter().enumerate() {
            m.dev.expect(feat::REPROG, 1, &[i as u8], &c.to_be_bytes());
        }
        assert_eq!(m.controls().unwrap(), controls);
        for c in controls {
            let [hi, lo] = c.to_be_bytes();
            let raw = c == cid::GESTURE;
            m.dev.expect(
                feat::REPROG,
                3,
                &[hi, lo, if raw { 0x33 } else { 0x23 }, 0, 0],
                &[],
            );
            m.set_divert(c, true, raw).unwrap();
        }

        // Shutdown must not query capabilities again or address the absent panel.
        for c in controls {
            let [hi, lo] = c.to_be_bytes();
            let params = [hi, lo, 0x22, 0, 0];
            if c == cid::BACK {
                m.dev.fail(feat::REPROG, 3, &params);
            } else {
                m.dev.expect(feat::REPROG, 3, &params, &[]);
            }
        }
        m.dev.expect(feat::THUMBWHEEL, 2, &[0, 0], &[]);
        m.dev.expect(feat::HIRES_WHEEL, 2, &[0], &[]);
        restore(&m);
    }

    #[test]
    fn dpi_cycle_skips_unsupported_presets() {
        let presets = [100, 400, 800, 1025, 4000, 8000];
        let mx3: Vec<_> = (200..=4000).step_by(50).collect();
        assert_eq!(next_dpi(&presets, &mx3, 400), Some(800));
        assert_eq!(next_dpi(&presets, &mx3, 800), Some(4000));
        assert_eq!(next_dpi(&presets, &mx3, 4000), Some(400));
        assert_eq!(next_dpi(&[8000], &mx3, 4000), None);
        assert_eq!(next_dpi(&[], &mx3, 4000), None);
        assert_eq!(next_dpi(&presets, &[], 4000), None);
        let mx4: Vec<_> = (200..=8000).step_by(50).collect();
        assert_eq!(next_dpi(&presets, &mx4, 4000), Some(8000));
        assert_eq!(next_dpi(&presets, &mx4, 8000), Some(400));
    }

    #[test]
    fn smartshift_config_preserves_fixed_ratchet() {
        let mut c = Config::default();
        c.scroll.mode = "ratchet".into();
        c.scroll.smartshift = false;
        let s = smartshift_of(&c);
        assert_eq!(s.mode, WheelMode::Ratchet);
        assert_eq!(s.threshold, 0xFF);
        c.scroll.smartshift = true;
        c.scroll.smartshift_sensitivity = 100;
        assert_eq!(smartshift_of(&c).threshold, 4);
        c.scroll.smartshift_sensitivity = 0;
        assert_eq!(smartshift_of(&c).threshold, 50);
    }
}
