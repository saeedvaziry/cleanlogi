//! User configuration, stored as TOML at ~/.config/cleanlogi/config.toml.
//! Defaults mirror the Logi Options+ profile this tool replaces.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Pointer resolution, 200..=8000 in steps of 50.
    pub dpi: u16,
    /// Values offered in the menu and stepped through by `dpi_cycle`.
    pub dpi_presets: Vec<u16>,
    pub scroll: Scroll,
    pub thumbwheel: Thumbwheel,
    pub haptics: Haptics,
    pub buttons: Buttons,
    pub gesture: Gesture,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Scroll {
    /// "ratchet" or "freespin": the mode the wheel rests in.
    pub mode: String,
    /// Switch to free-spin automatically when flicking the wheel.
    pub smartshift: bool,
    /// SmartShift sensitivity, 0..=100 (same scale as Logi Options+).
    pub sensitivity: u8,
    /// Ratchet force, 1..=100.
    pub torque: u8,
    /// High-resolution wheel reports (smoother, more events per notch).
    pub hires: bool,
    /// Invert the main wheel direction relative to the macOS setting.
    pub invert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Thumbwheel {
    /// "native" leaves the thumbwheel as horizontal scrolling; "actions" runs
    /// `left`/`right` every `step` units of rotation.
    pub mode: String,
    pub left: String,
    pub right: String,
    pub step: u16,
    pub invert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Haptics {
    pub enabled: bool,
    /// Strength, 0..=100.
    pub level: u8,
    /// Pulse when a gesture fires.
    pub on_gesture: bool,
    /// Waveform name (see README) used for gesture feedback.
    pub gesture_waveform: String,
    /// Haptic sense panel press force: "light", "medium" or "firm".
    pub panel_force: String,
}

/// Button assignments. Each value is an action string (see actions.rs);
/// "native" leaves the button with its built-in behaviour.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Buttons {
    pub middle: String,
    pub back: String,
    pub forward: String,
    pub mode_shift: String,
    pub haptic_panel: String,
}

/// Gesture button: hold and move the mouse, or click it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Gesture {
    pub click: String,
    pub up: String,
    pub down: String,
    pub left: String,
    pub right: String,
    /// Movement (raw sensor counts) needed to trigger a direction.
    pub threshold: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dpi: 1000,
            dpi_presets: vec![400, 800, 1000, 1200, 1600, 2000, 2400, 3200, 4000],
            scroll: Scroll::default(),
            thumbwheel: Thumbwheel::default(),
            haptics: Haptics::default(),
            buttons: Buttons::default(),
            gesture: Gesture::default(),
        }
    }
}

impl Default for Scroll {
    fn default() -> Self {
        Self {
            mode: "freespin".into(),
            smartshift: true,
            sensitivity: 83,
            torque: 75,
            hires: false,
            invert: false,
        }
    }
}

impl Default for Thumbwheel {
    fn default() -> Self {
        Self {
            mode: "actions".into(),
            left: "ctrl+left".into(),
            right: "ctrl+right".into(),
            step: 15,
            invert: false,
        }
    }
}

impl Default for Haptics {
    fn default() -> Self {
        Self {
            enabled: true,
            level: 60,
            on_gesture: true,
            gesture_waveform: "sharp_state_change".into(),
            panel_force: "medium".into(),
        }
    }
}

impl Default for Buttons {
    fn default() -> Self {
        Self {
            middle: "native".into(),
            back: "back".into(),
            forward: "forward".into(),
            mode_shift: "smartshift_toggle".into(),
            haptic_panel: "cmd+shift+4".into(),
        }
    }
}

impl Default for Gesture {
    fn default() -> Self {
        Self {
            click: "mission_control".into(),
            up: "mission_control".into(),
            down: "app_expose".into(),
            left: "desktop_left".into(),
            right: "desktop_right".into(),
            threshold: 60,
        }
    }
}

pub fn dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    home.join(".config/cleanlogi")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

impl Config {
    /// Load the config, writing the defaults on first run.
    pub fn load() -> Result<Self> {
        let p = path();
        if !p.exists() {
            let c = Self::default();
            c.save()?;
            return Ok(c);
        }
        let text =
            std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(dir())?;
        let body = toml::to_string_pretty(self)?;
        std::fs::write(path(), format!("{HEADER}\n{body}"))?;
        Ok(())
    }
}

const HEADER: &str = "\
# cleanlogi configuration. Changes are picked up automatically.
#
# Actions: native | none | back | forward | middle_click
#   mission_control | app_expose | show_desktop | launchpad
#   desktop_left | desktop_right | smartshift_toggle | dpi_cycle
#   play_pause | next_track | prev_track | volume_up | volume_down | mute
#   any shortcut such as \"cmd+shift+4\", \"ctrl+left\", \"f11\"
#   run:<shell command>, e.g. \"run:open -a Terminal\"
";
