//! User configuration, stored as TOML at ~/.config/cleanlogi/config.toml.
//! default-config.toml is both the documented template written on first run
//! and the source of the defaults below; a test keeps the two in step.

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
    /// 1..=100; distance per wheel notch when `smooth`.
    pub speed: u8,
    /// CleanLogi turns wheel movement into smooth scrolling; otherwise macOS
    /// handles the wheel natively.
    pub smooth: bool,
    pub invert: bool,
    /// "ratchet" or "freespin": the mode the wheel rests in.
    pub mode: String,
    /// Switch to free-spin automatically when flicking the wheel.
    pub smartshift: bool,
    /// 0..=100, same scale as Logi Options+.
    #[serde(alias = "sensitivity")]
    pub smartshift_sensitivity: u8,
    /// 1..=100.
    #[serde(alias = "torque")]
    pub ratchet_force: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Thumbwheel {
    /// "scroll" (smooth horizontal scrolling), "actions" (run `left`/`right`
    /// once a roll reaches `step` units of rotation) or "native" (the mouse's own).
    pub mode: String,
    /// 1..=100; horizontal scroll distance in "scroll" mode.
    pub speed: u8,
    pub invert: bool,
    pub left: String,
    pub right: String,
    pub step: u16,
    /// Run the action again every `step` units of the same roll, rather than
    /// once per roll.
    pub repeat: bool,
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
            speed: 50,
            smooth: true,
            invert: false,
            mode: "freespin".into(),
            smartshift: true,
            smartshift_sensitivity: 83,
            ratchet_force: 75,
        }
    }
}

impl Default for Thumbwheel {
    fn default() -> Self {
        Self {
            mode: "actions".into(),
            speed: 50,
            invert: false,
            left: "ctrl+left".into(),
            right: "ctrl+right".into(),
            step: 30,
            repeat: false,
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

const TEMPLATE: &str = include_str!("default-config.toml");

impl Config {
    /// Load the config, writing the documented defaults on first run.
    pub fn load() -> Result<Self> {
        let p = path();
        if !p.exists() {
            std::fs::create_dir_all(dir())?;
            std::fs::write(&p, TEMPLATE)?;
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let cfg: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        if !text.starts_with(TEMPLATE_MARKER) {
            // Written by an older version without comments: move the
            // values onto the documented template.
            cfg.save()?;
        }
        Ok(cfg)
    }

    /// Write the config back, changing only values that differ so comments
    /// and layout in the file are kept.
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(dir())?;
        let existing = std::fs::read_to_string(path()).unwrap_or_default();
        let base = if existing.starts_with(TEMPLATE_MARKER) {
            &existing
        } else {
            TEMPLATE
        };
        let mut doc: toml_edit::DocumentMut = base.parse()?;
        let new: toml_edit::DocumentMut = toml::to_string(self)?.parse()?;
        merge(doc.as_table_mut(), new.as_table());
        std::fs::write(path(), doc.to_string())?;
        Ok(())
    }
}

/// First line of the template; files that start with it are kept as-is.
const TEMPLATE_MARKER: &str = "# CleanLogi settings.";

/// Copy values from `new` into `doc`, keeping each existing entry's comments.
fn merge(doc: &mut toml_edit::Table, new: &toml_edit::Table) {
    for (key, item) in new.iter() {
        match (doc.get_mut(key), item) {
            (Some(toml_edit::Item::Table(t)), toml_edit::Item::Table(n)) => merge(t, n),
            (Some(toml_edit::Item::Value(old)), toml_edit::Item::Value(v)) => {
                if old.to_string().trim() != v.to_string().trim() {
                    let decor = old.decor().clone();
                    *old = v.clone();
                    *old.decor_mut() = decor;
                }
            }
            _ => {
                doc.insert(key, item.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_matches_defaults() {
        let parsed: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(parsed, Config::default());
        assert!(TEMPLATE.starts_with(TEMPLATE_MARKER));
    }

    #[test]
    fn merge_keeps_comments() {
        let mut doc: toml_edit::DocumentMut = TEMPLATE.parse().unwrap();
        let mut c = Config {
            dpi: 1600,
            ..Default::default()
        };
        c.scroll.speed = 20;
        let new: toml_edit::DocumentMut = toml::to_string(&c).unwrap().parse().unwrap();
        merge(doc.as_table_mut(), new.as_table());
        let out = doc.to_string();
        assert!(out.contains("dpi = 1600"));
        assert!(out.contains("speed = 20"));
        assert!(out.contains("# Scroll speed, 1 to 100."));
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
    }

    #[test]
    fn old_names_still_load() {
        let c: Config = toml::from_str("[scroll]\nsensitivity = 40\ntorque = 20\n").unwrap();
        assert_eq!(c.scroll.smartshift_sensitivity, 40);
        assert_eq!(c.scroll.ratchet_force, 20);
    }
}
