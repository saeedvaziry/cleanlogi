//! Feature-level API over HID++ for Logitech mice (built against MX Master 4).
//!
//! Layouts for the documented features follow the public HID++ 2.0 specs.
//! 0x19B0 (haptics) and 0x19C0 (force-sensing button) are undocumented; their
//! function numbers and payloads were recovered from the Logi Options+ agent
//! (devio::Feature19B0HapticFeedback / devio::Feature19C0ForceSensingButton).

use crate::hidpp::{Device, Message};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::time::Duration;

pub mod feat {
    pub const DEVICE_NAME: u16 = 0x0005;
    pub const BATTERY: u16 = 0x1004;
    pub const CHANGE_HOST: u16 = 0x1814;
    pub const HOSTS_INFO: u16 = 0x1815;
    pub const HAPTICS: u16 = 0x19B0;
    pub const FORCE_BUTTON: u16 = 0x19C0;
    pub const REPROG: u16 = 0x1B04;
    pub const WIRELESS_STATUS: u16 = 0x1D4B;
    pub const SMARTSHIFT: u16 = 0x2111;
    pub const HIRES_WHEEL: u16 = 0x2121;
    pub const THUMBWHEEL: u16 = 0x2150;
    pub const DPI: u16 = 0x2201;
}

/// Control IDs (0x1B04) on the MX Master 4.
pub mod cid {
    pub const MIDDLE: u16 = 0x0052;
    pub const BACK: u16 = 0x0053;
    pub const FORWARD: u16 = 0x0056;
    pub const GESTURE: u16 = 0x00C3;
    pub const MODE_SHIFT: u16 = 0x00C4;
    pub const HAPTIC_PANEL: u16 = 0x01A0;
}

/// Haptic waveforms (0x19B0 playWaveform). Order of the Options+
/// logi.protocol.haptics WaveformId enum; the device reports a 15-bit mask.
#[allow(dead_code)]
pub mod waveform {
    pub const SHARP_STATE_CHANGE: u8 = 0;
    pub const DAMP_STATE_CHANGE: u8 = 1;
    pub const SHARP_COLLISION: u8 = 2;
    pub const DAMP_COLLISION: u8 = 3;
    pub const SUBTLE_COLLISION: u8 = 4;
    pub const HAPPY_ALERT: u8 = 5;
    pub const ANGRY_ALERT: u8 = 6;
    pub const COMPLETED: u8 = 7;
    pub const SQUARE: u8 = 8;
    pub const FIREWORK: u8 = 9;
    pub const JINGLE: u8 = 10;
    pub const RINGING: u8 = 11;

    pub fn from_name(name: &str) -> Option<u8> {
        Some(match name {
            "sharp_state_change" => SHARP_STATE_CHANGE,
            "damp_state_change" => DAMP_STATE_CHANGE,
            "sharp_collision" => SHARP_COLLISION,
            "damp_collision" => DAMP_COLLISION,
            "subtle_collision" => SUBTLE_COLLISION,
            "happy_alert" => HAPPY_ALERT,
            "angry_alert" => ANGRY_ALERT,
            "completed" => COMPLETED,
            "square" => SQUARE,
            "firework" => FIREWORK,
            "jingle" => JINGLE,
            "ringing" => RINGING,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelMode {
    FreeSpin,
    Ratchet,
}

#[derive(Debug, Clone, Copy)]
pub struct SmartShift {
    pub mode: WheelMode,
    /// Auto-disengage threshold, 1..=50 (lower = switches to free-spin
    /// sooner). 0xFF keeps the ratchet engaged.
    pub threshold: u8,
    /// Ratchet torque, 1..=100.
    pub torque: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceLevel {
    Light,
    Medium,
    Firm,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Currently held diverted controls (empty when all released).
    Buttons(Vec<u16>),
    /// Raw pointer movement while a raw-XY diverted control is held.
    RawXY(i16, i16),
    /// Diverted thumbwheel rotation; status 1=start 2=active 3=stop.
    Thumbwheel {
        rotation: i16,
        status: u8,
    },
    Battery(Battery),
    /// Device reconnected and asks the host to reapply its configuration.
    Reconnected,
    Other,
}

pub struct Mouse {
    pub dev: Device,
    index: HashMap<u16, u8>,
}

impl Mouse {
    pub fn new(dev: Device) -> Result<Self> {
        let index = dev
            .features()?
            .into_iter()
            .map(|f| (f.id, f.index))
            .collect();
        Ok(Self { dev, index })
    }

    pub fn has(&self, feature: u16) -> bool {
        self.index.contains_key(&feature)
    }

    fn call(&self, feature: u16, function: u8, params: &[u8]) -> Result<[u8; 16]> {
        let idx = *self
            .index
            .get(&feature)
            .ok_or_else(|| anyhow!("device lacks feature {feature:#06x}"))?;
        self.dev.request(idx, function, params)
    }

    pub fn name(&self) -> Result<String> {
        let len = self.call(feat::DEVICE_NAME, 0, &[])?[0] as usize;
        let mut name = Vec::with_capacity(len);
        while name.len() < len {
            let chunk = self.call(feat::DEVICE_NAME, 1, &[name.len() as u8])?;
            let take = (len - name.len()).min(16);
            name.extend_from_slice(&chunk[..take]);
        }
        Ok(String::from_utf8_lossy(&name)
            .trim_end_matches('\0')
            .to_string())
    }

    pub fn battery(&self) -> Result<Battery> {
        Ok(parse_battery(&self.call(feat::BATTERY, 1, &[])?))
    }

    pub fn set_dpi(&self, dpi: u16) -> Result<()> {
        let [hi, lo] = dpi.to_be_bytes();
        self.call(feat::DPI, 3, &[0, hi, lo])?;
        Ok(())
    }

    pub fn set_smartshift(&self, s: SmartShift) -> Result<()> {
        let mode = match s.mode {
            WheelMode::FreeSpin => 1,
            WheelMode::Ratchet => 2,
        };
        self.call(feat::SMARTSHIFT, 2, &[mode, s.threshold, s.torque])?;
        Ok(())
    }

    /// 0x2121 setWheelMode. `diverted` sends wheel movement over HID++ instead
    /// of standard HID (we never want that: macOS handles scrolling).
    pub fn set_wheel_mode(&self, diverted: bool, hires: bool, invert: bool) -> Result<()> {
        let flags = diverted as u8 | (hires as u8) << 1 | (invert as u8) << 2;
        self.call(feat::HIRES_WHEEL, 2, &[flags])?;
        Ok(())
    }

    /// 0x2150 setThumbwheelReporting.
    pub fn set_thumbwheel(&self, diverted: bool, invert: bool) -> Result<()> {
        self.call(feat::THUMBWHEEL, 2, &[diverted as u8, invert as u8])?;
        Ok(())
    }

    /// 0x1B04 setCidReporting. Each flag is paired with its "valid" bit so the
    /// device applies both divert and raw-XY state.
    pub fn set_divert(&self, cid: u16, divert: bool, raw_xy: bool) -> Result<()> {
        let [hi, lo] = cid.to_be_bytes();
        let flags = (divert as u8) | 0x02 | (raw_xy as u8) << 4 | 0x20;
        self.call(feat::REPROG, 3, &[hi, lo, flags, 0, 0])?;
        Ok(())
    }

    pub fn controls(&self) -> Result<Vec<u16>> {
        let n = self.call(feat::REPROG, 0, &[])?[0];
        (0..n)
            .map(|i| {
                let r = self.call(feat::REPROG, 1, &[i])?;
                Ok(u16::from_be_bytes([r[0], r[1]]))
            })
            .collect()
    }

    /// 0x19B0 setHapticConfig: [flags, level]; bit0 = enabled.
    pub fn set_haptics(&self, enabled: bool, level: u8) -> Result<()> {
        self.call(feat::HAPTICS, 2, &[enabled as u8, level.min(100)])?;
        Ok(())
    }

    /// 0x19B0 playHapticWaveForm: [waveform, report_info_event].
    pub fn play_waveform(&self, id: u8) -> Result<()> {
        self.call(feat::HAPTICS, 4, &[id, 0])?;
        Ok(())
    }

    /// 0x19C0 setButtonConfig for button 0 (the haptic sense panel). Levels
    /// follow Options+ map_level_to_force: light = capability minimum,
    /// medium = default, firm = maximum.
    pub fn set_force(&self, level: ForceLevel) -> Result<()> {
        let caps = self.call(feat::FORCE_BUTTON, 1, &[0])?;
        let default = u16::from_be_bytes([caps[2], caps[3]]);
        let max = u16::from_be_bytes([caps[4], caps[5]]);
        let min = u16::from_be_bytes([caps[6], caps[7]]);
        let force = match level {
            ForceLevel::Light => min,
            ForceLevel::Medium => default,
            ForceLevel::Firm => max,
        };
        let [hi, lo] = force.to_be_bytes();
        self.call(feat::FORCE_BUTTON, 3, &[0, hi, lo, 0, 0])?;
        Ok(())
    }

    /// (host count, current host index)
    pub fn hosts(&self) -> Result<(u8, u8)> {
        let r = self.call(feat::CHANGE_HOST, 0, &[])?;
        Ok((r[0], r[1]))
    }

    /// Names of the paired Easy-Switch hosts (0x1815 getHostInfo /
    /// getHostFriendlyName); empty for unpaired channels.
    pub fn host_names(&self, count: u8) -> Vec<String> {
        (0..count)
            .map(|h| self.host_name(h).unwrap_or_default())
            .collect()
    }

    fn host_name(&self, host: u8) -> Result<String> {
        // getHostInfo: [host, paired, bus type, pages, name length, max length]
        let info = self.call(feat::HOSTS_INFO, 1, &[host])?;
        if info[1] == 0 {
            return Ok(String::new());
        }
        let len = info[4] as usize;
        let mut name = Vec::with_capacity(len);
        while name.len() < len {
            // getHostFriendlyName: [host, offset, up to 14 bytes of UTF-8]
            let r = self.call(feat::HOSTS_INFO, 3, &[host, name.len() as u8])?;
            let take = (len - name.len()).min(14);
            name.extend_from_slice(&r[2..2 + take]);
        }
        Ok(String::from_utf8_lossy(&name).into_owned())
    }

    /// Switch the mouse to another Easy-Switch host. The device disconnects
    /// from us immediately, so no reply is expected.
    pub fn change_host(&self, host: u8) -> Result<()> {
        let _ = self.call(feat::CHANGE_HOST, 1, &[host]);
        Ok(())
    }

    pub fn next_event(&self, timeout: Duration) -> Result<Option<Event>> {
        let Some(m) = self.dev.next_event(timeout)? else {
            return Ok(None);
        };
        Ok(Some(self.decode(&m)))
    }

    fn decode(&self, m: &Message) -> Event {
        let is = |f: u16| self.index.get(&f) == Some(&m.feature_index);
        let p = &m.params;
        if is(feat::REPROG) && m.function == 0 {
            let cids = p[..8]
                .chunks(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .filter(|&c| c != 0)
                .collect();
            Event::Buttons(cids)
        } else if is(feat::REPROG) && m.function == 1 {
            Event::RawXY(
                i16::from_be_bytes([p[0], p[1]]),
                i16::from_be_bytes([p[2], p[3]]),
            )
        } else if is(feat::THUMBWHEEL) && m.function == 0 {
            Event::Thumbwheel {
                rotation: i16::from_be_bytes([p[0], p[1]]),
                status: p[4],
            }
        } else if is(feat::BATTERY) && m.function == 0 {
            Event::Battery(parse_battery(p))
        } else if is(feat::WIRELESS_STATUS) && m.function == 0 && p[0] == 1 {
            Event::Reconnected
        } else {
            Event::Other
        }
    }
}

fn parse_battery(r: &[u8]) -> Battery {
    // 0x1004 getStatus: [soc %, level flags, charging status, external power]
    Battery {
        percent: r[0],
        charging: matches!(r[2], 1 | 2),
    }
}

/// Options+ UI sensitivity (0..=100) to the device auto-disengage threshold.
/// Inverse of devio_feature_xsmartshift::get_sensitivity:
/// sensitivity = (50 - clamp(threshold, 4, 50)) * 100 / 46.
pub fn sensitivity_to_threshold(sensitivity: u8) -> u8 {
    let s = sensitivity.min(100) as f64;
    (50.0 - s * 46.0 / 100.0).round().clamp(4.0, 50.0) as u8
}
