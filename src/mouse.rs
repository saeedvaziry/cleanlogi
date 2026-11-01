//! Feature-level API over HID++ for Logitech MX Master 3 and 4 mice.
//!
//! Layouts for the documented features follow the public HID++ 2.0 specs.
//! 0x19B0 (haptics) and 0x19C0 (force-sensing button) are undocumented; their
//! function numbers and payloads were recovered from the Logi Options+ agent
//! (devio::Feature19B0HapticFeedback / devio::Feature19C0ForceSensingButton).

use crate::hidpp::{Device, Message, Transport};
use anyhow::{Result, anyhow, bail};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::time::Duration;

pub mod feat {
    pub const DEVICE_NAME: u16 = 0x0005;
    pub const BATTERY_LEGACY: u16 = 0x1000;
    pub const BATTERY: u16 = 0x1004;
    pub const CHANGE_HOST: u16 = 0x1814;
    pub const HOSTS_INFO: u16 = 0x1815;
    pub const HAPTICS: u16 = 0x19B0;
    pub const FORCE_BUTTON: u16 = 0x19C0;
    pub const REPROG: u16 = 0x1B04;
    pub const WIRELESS_STATUS: u16 = 0x1D4B;
    pub const SMARTSHIFT_LEGACY: u16 = 0x2110;
    pub const SMARTSHIFT: u16 = 0x2111;
    pub const HIRES_WHEEL: u16 = 0x2121;
    pub const THUMBWHEEL: u16 = 0x2150;
    pub const DPI: u16 = 0x2201;
}

/// Shared control IDs (0x1B04); HAPTIC_PANEL is MX Master 4-only.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Diverted main wheel movement; positive = rolled away from the user.
    /// In hi-res mode one notch is `wheel_multiplier()` units.
    Wheel {
        delta: i16,
        hires: bool,
    },
    Battery(Battery),
    /// Device reconnected and asks the host to reapply its configuration.
    Reconnected,
    Other,
}

pub struct Mouse<T = Device> {
    pub dev: T,
    index: HashMap<u16, u8>,
    // Capabilities are fixed for a connection; only successful reads are cached.
    dpi_values: OnceCell<Vec<u16>>,
    controls: OnceCell<Vec<u16>>,
}

impl<T: Transport> Mouse<T> {
    pub fn new(dev: T) -> Result<Self> {
        let index = dev
            .features()?
            .into_iter()
            .map(|f| (f.id, f.index))
            .collect();
        Ok(Self {
            dev,
            index,
            dpi_values: OnceCell::new(),
            controls: OnceCell::new(),
        })
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
        let (feature, function) = if self.has(feat::BATTERY) {
            (feat::BATTERY, 1)
        } else {
            (feat::BATTERY_LEGACY, 0)
        };
        Ok(parse_battery(&self.call(feature, function, &[])?))
    }

    /// 0x2201 getSensorDpiList for the pointer sensor.
    pub fn dpi_values(&self) -> Result<Vec<u16>> {
        if let Some(values) = self.dpi_values.get() {
            return Ok(values.clone());
        }
        let values = parse_dpi_list(&self.call(feat::DPI, 1, &[0])?)?;
        let _ = self.dpi_values.set(values.clone());
        Ok(values)
    }

    /// Clamp to a supported value, rounding down between steps. Returns the
    /// applied DPI without changing the user's preference for other mice.
    pub fn set_dpi(&self, dpi: u16) -> Result<u16> {
        let values = self.dpi_values()?;
        let dpi = values
            .iter()
            .copied()
            .rev()
            .find(|&d| d <= dpi)
            .unwrap_or(values[0]);
        let [hi, lo] = dpi.to_be_bytes();
        self.call(feat::DPI, 3, &[0, hi, lo])?;
        Ok(dpi)
    }

    pub fn set_smartshift(&self, s: SmartShift) -> Result<()> {
        let mode = match s.mode {
            WheelMode::FreeSpin => 1,
            WheelMode::Ratchet => 2,
        };
        if self.has(feat::SMARTSHIFT) {
            self.call(feat::SMARTSHIFT, 2, &[mode, s.threshold, s.torque])?;
        } else {
            // The third byte on 0x2110 is a persistent default, not torque.
            // Leave it unchanged; settings are reapplied on reconnect.
            self.call(feat::SMARTSHIFT_LEGACY, 1, &[mode, s.threshold, 0])?;
        }
        Ok(())
    }

    /// 0x2121 setWheelMode. `diverted` sends wheel movement over HID++ instead
    /// of standard HID (we never want that: macOS handles scrolling).
    /// 0x2121 getWheelCapability: hi-res units per ratchet notch.
    pub fn wheel_multiplier(&self) -> u8 {
        self.call(feat::HIRES_WHEEL, 0, &[])
            .map(|r| r[0].max(1))
            .unwrap_or(1)
    }

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
        if let Some(controls) = self.controls.get() {
            return Ok(controls.clone());
        }
        let n = self.call(feat::REPROG, 0, &[])?[0];
        let controls = (0..n)
            .map(|i| {
                let r = self.call(feat::REPROG, 1, &[i])?;
                Ok(u16::from_be_bytes([r[0], r[1]]))
            })
            .collect::<Result<Vec<_>>>()?;
        let _ = self.controls.set(controls.clone());
        Ok(controls)
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
        } else if is(feat::HIRES_WHEEL) && m.function == 0 {
            // wheelMovement: [resolution bit4 | periods, deltaV i16]
            Event::Wheel {
                delta: i16::from_be_bytes([p[1], p[2]]),
                hires: p[0] & 0x10 != 0,
            }
        } else if is(feat::THUMBWHEEL) && m.function == 0 {
            Event::Thumbwheel {
                rotation: i16::from_be_bytes([p[0], p[1]]),
                status: p[4],
            }
        } else if (is(feat::BATTERY) || is(feat::BATTERY_LEGACY)) && m.function == 0 {
            Event::Battery(parse_battery(p))
        } else if is(feat::WIRELESS_STATUS) && m.function == 0 && p[0] == 1 {
            Event::Reconnected
        } else {
            Event::Other
        }
    }
}

fn parse_battery(r: &[u8]) -> Battery {
    // 0x1004: [soc %, level flags, status, external power]
    // 0x1000: [level %, next level %, status]
    Battery {
        percent: r[0],
        charging: matches!(r[2], 1 | 2),
    }
}

fn parse_dpi_list(r: &[u8; 16]) -> Result<Vec<u16>> {
    let values: Vec<_> = r[1..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| u16::from_be_bytes(b))
        .take_while(|&d| d != 0)
        .collect();
    if r[0] != 0 || values.is_empty() {
        bail!("invalid DPI list for sensor 0");
    }
    match values.as_slice() {
        // A range is [minimum, 0xE000 + step, maximum].
        &[min, step, max] if min < 0xE000 && step > 0xE000 && max < 0xE000 && min <= max => {
            Ok((min..=max).step_by((step - 0xE000) as usize).collect())
        }
        _ if values.iter().all(|&d| d < 0xE000) && values.windows(2).all(|w| w[0] < w[1]) => {
            Ok(values)
        }
        _ => bail!("invalid DPI list for sensor 0"),
    }
}

/// Options+ UI sensitivity (0..=100) to the device auto-disengage threshold.
/// Inverse of devio_feature_xsmartshift::get_sensitivity:
/// sensitivity = (50 - clamp(threshold, 4, 50)) * 100 / 46.
pub fn sensitivity_to_threshold(sensitivity: u8) -> u8 {
    let s = sensitivity.min(100) as f64;
    (50.0 - s * 46.0 / 100.0).round().clamp(4.0, 50.0) as u8
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hidpp::FeatureRef;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct Reply {
        index: u8,
        function: u8,
        params: Vec<u8>,
        response: Result<[u8; 16]>,
    }

    pub(crate) struct MockTransport {
        features: Vec<FeatureRef>,
        replies: RefCell<VecDeque<Reply>>,
        events: RefCell<VecDeque<Message>>,
    }

    fn report(bytes: &[u8]) -> [u8; 16] {
        let mut r = [0; 16];
        r[..bytes.len()].copy_from_slice(bytes);
        r
    }

    pub(crate) fn mouse(features: &[u16]) -> Mouse<MockTransport> {
        Mouse::new(MockTransport {
            features: features
                .iter()
                .enumerate()
                .map(|(i, &id)| FeatureRef {
                    id,
                    index: (i + 1) as u8,
                    flags: 0,
                    version: 0,
                })
                .collect(),
            replies: RefCell::new(VecDeque::new()),
            events: RefCell::new(VecDeque::new()),
        })
        .unwrap()
    }

    impl MockTransport {
        fn index(&self, feature: u16) -> u8 {
            self.features
                .iter()
                .find(|f| f.id == feature)
                .unwrap()
                .index
        }

        pub(crate) fn expect(&self, feature: u16, function: u8, params: &[u8], response: &[u8]) {
            self.replies.borrow_mut().push_back(Reply {
                index: self.index(feature),
                function,
                params: params.to_vec(),
                response: Ok(report(response)),
            });
        }

        pub(crate) fn fail(&self, feature: u16, function: u8, params: &[u8]) {
            self.replies.borrow_mut().push_back(Reply {
                index: self.index(feature),
                function,
                params: params.to_vec(),
                response: Err(anyhow!("simulated HID++ timeout")),
            });
        }

        fn event(&self, feature: u16, function: u8, params: &[u8]) {
            self.events.borrow_mut().push_back(Message {
                feature_index: self.index(feature),
                function,
                sw_id: 0,
                params: report(params),
            });
        }
    }

    impl Transport for MockTransport {
        fn features(&self) -> Result<Vec<FeatureRef>> {
            Ok(self.features.clone())
        }

        fn request(&self, index: u8, function: u8, params: &[u8]) -> Result<[u8; 16]> {
            let reply = self
                .replies
                .borrow_mut()
                .pop_front()
                .expect("unexpected request");
            assert_eq!(
                (index, function, params),
                (reply.index, reply.function, reply.params.as_slice())
            );
            reply.response
        }

        fn next_event(&self, _: Duration) -> Result<Option<Message>> {
            Ok(self.events.borrow_mut().pop_front())
        }
    }

    impl Drop for MockTransport {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                assert!(self.replies.get_mut().is_empty(), "unconsumed requests");
                assert!(self.events.get_mut().is_empty(), "unconsumed events");
            }
        }
    }

    #[test]
    fn battery_status_and_events_for_both_models() {
        for (feature, function, second) in [(feat::BATTERY_LEGACY, 0, 50), (feat::BATTERY, 1, 4)] {
            let m = mouse(&[feat::DEVICE_NAME, feature]);
            for (status, charging) in [
                (0, false),
                (1, true),
                (2, true),
                (3, false),
                (4, false),
                (5, false),
                (7, false),
            ] {
                let payload = [75, second, status, 0];
                let expected = Battery {
                    percent: 75,
                    charging,
                };
                m.dev.expect(feature, function, &[], &payload);
                assert_eq!(m.battery().unwrap(), expected);
                m.dev.event(feature, 0, &payload);
                assert_eq!(
                    m.next_event(Duration::ZERO).unwrap(),
                    Some(Event::Battery(expected))
                );
            }
        }
    }

    #[test]
    fn newer_features_take_precedence() {
        let m = mouse(&[
            feat::BATTERY_LEGACY,
            feat::BATTERY,
            feat::SMARTSHIFT_LEGACY,
            feat::SMARTSHIFT,
        ]);
        m.dev.expect(feat::BATTERY, 1, &[], &[90, 8, 0, 0]);
        assert_eq!(m.battery().unwrap().percent, 90);
        m.dev.expect(feat::SMARTSHIFT, 2, &[2, 12, 75], &[]);
        m.set_smartshift(SmartShift {
            mode: WheelMode::Ratchet,
            threshold: 12,
            torque: 75,
        })
        .unwrap();
    }

    #[test]
    fn smartshift_commands_for_both_models() {
        for (feature, function, torque) in
            [(feat::SMARTSHIFT_LEGACY, 1, 0), (feat::SMARTSHIFT, 2, 75)]
        {
            let m = mouse(&[feature]);
            for (mode, byte) in [(WheelMode::FreeSpin, 1), (WheelMode::Ratchet, 2)] {
                for threshold in [4, 12, 50, 0xFF] {
                    m.dev
                        .expect(feature, function, &[byte, threshold, torque], &[]);
                    m.set_smartshift(SmartShift {
                        mode,
                        threshold,
                        torque: 75,
                    })
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn missing_features_do_not_send_requests() {
        let m = mouse(&[]);
        assert!(m.battery().is_err());
        assert!(m.set_dpi(1000).is_err());
        assert!(
            m.set_smartshift(SmartShift {
                mode: WheelMode::Ratchet,
                threshold: 12,
                torque: 75
            })
            .is_err()
        );
        assert!(m.set_haptics(true, 60).is_err());
        assert!(m.play_waveform(waveform::SHARP_STATE_CHANGE).is_err());
    }

    #[test]
    fn dpi_ranges_and_clamping_for_both_models() {
        for max in [4000u16, 8000] {
            let m = mouse(&[feat::DPI]);
            let [hi, lo] = max.to_be_bytes();
            let range = [0, 0, 200, 0xE0, 50, hi, lo, 0, 0];
            m.dev.expect(feat::DPI, 1, &[0], &range);
            let values = m.dpi_values().unwrap();
            assert_eq!(values, (200..=max).step_by(50).collect::<Vec<_>>());
            for (requested, applied) in [
                (0, 200u16),
                (199, 200),
                (1000, 1000),
                (1029, 1000),
                (4000, 4000),
                (8000, max),
                (u16::MAX, max),
            ] {
                let [hi, lo] = applied.to_be_bytes();
                m.dev.expect(feat::DPI, 3, &[0, hi, lo], &[]);
                assert_eq!(m.set_dpi(requested).unwrap(), applied);
            }
        }
    }

    #[test]
    fn dpi_lists_and_invalid_capabilities() {
        let list = report(&[0, 0x01, 0x90, 0x03, 0x20, 0x06, 0x40, 0, 0]);
        assert_eq!(parse_dpi_list(&list).unwrap(), [400, 800, 1600]);
        for invalid in [
            report(&[]),
            report(&[1, 0x01, 0x90, 0, 0]),             // wrong sensor
            report(&[0, 0, 200, 0xE0, 0, 0x0F, 0xA0]),  // zero step
            report(&[0, 0x0F, 0xA0, 0xE0, 50, 0, 200]), // reversed range
            report(&[0, 0, 200, 0xE0, 50]),             // missing upper bound
            report(&[0, 0x03, 0x20, 0x01, 0x90]),       // unsorted list
        ] {
            assert!(parse_dpi_list(&invalid).is_err());
        }
        let m = mouse(&[feat::DPI]);
        m.dev.expect(feat::DPI, 1, &[0], &list);
        m.dev.expect(feat::DPI, 3, &[0, 0x03, 0x20], &[]);
        assert_eq!(m.set_dpi(1000).unwrap(), 800);
        let m = mouse(&[feat::DPI]);
        m.dev.expect(feat::DPI, 1, &[0], &[]);
        assert!(m.set_dpi(1000).is_err()); // no write after invalid capabilities
    }

    #[test]
    fn failed_control_enumeration_can_be_retried() {
        let m = mouse(&[feat::REPROG]);
        m.dev.expect(feat::REPROG, 0, &[], &[2]);
        m.dev.expect(feat::REPROG, 1, &[0], &[0, 0x53]);
        m.dev.fail(feat::REPROG, 1, &[1]);
        assert!(m.controls().is_err());
        m.dev.expect(feat::REPROG, 0, &[], &[2]);
        m.dev.expect(feat::REPROG, 1, &[0], &[0, 0x53]);
        m.dev.expect(feat::REPROG, 1, &[1], &[0, 0xC3]);
        assert_eq!(m.controls().unwrap(), [cid::BACK, cid::GESTURE]);
        // Once complete, capability reads don't depend on further HID requests.
        assert_eq!(m.controls().unwrap(), [cid::BACK, cid::GESTURE]);
    }

    #[test]
    fn shared_button_and_wheel_commands() {
        let m = mouse(&[feat::REPROG, feat::HIRES_WHEEL, feat::THUMBWHEEL]);
        m.dev.expect(feat::REPROG, 3, &[0, 0xC3, 0x33, 0, 0], &[]);
        m.set_divert(cid::GESTURE, true, true).unwrap();
        for c in [cid::MIDDLE, cid::BACK, cid::FORWARD, cid::MODE_SHIFT] {
            let [hi, lo] = c.to_be_bytes();
            m.dev.expect(feat::REPROG, 3, &[hi, lo, 0x23, 0, 0], &[]);
            m.set_divert(c, true, false).unwrap();
        }
        m.dev.expect(feat::REPROG, 3, &[0, 0xC3, 0x22, 0, 0], &[]);
        m.set_divert(cid::GESTURE, false, false).unwrap();
        m.dev.expect(feat::HIRES_WHEEL, 0, &[], &[15]);
        assert_eq!(m.wheel_multiplier(), 15);
        m.dev.expect(feat::HIRES_WHEEL, 2, &[3], &[]);
        m.set_wheel_mode(true, true, false).unwrap();
        m.dev.expect(feat::THUMBWHEEL, 2, &[1, 0], &[]);
        m.set_thumbwheel(true, false).unwrap();
    }

    #[test]
    fn shared_input_and_reconnection_events() {
        let m = mouse(&[
            feat::REPROG,
            feat::HIRES_WHEEL,
            feat::THUMBWHEEL,
            feat::WIRELESS_STATUS,
        ]);
        for (feature, function, payload, expected) in [
            (
                feat::REPROG,
                0,
                report(&[0, 0xC3, 0, 0x53]),
                Event::Buttons(vec![cid::GESTURE, cid::BACK]),
            ),
            (feat::REPROG, 0, report(&[]), Event::Buttons(vec![])),
            (
                feat::REPROG,
                1,
                report(&[0xFF, 0xF6, 0, 20]),
                Event::RawXY(-10, 20),
            ),
            (
                feat::HIRES_WHEEL,
                0,
                report(&[0x10, 0xFF, 0xF1]),
                Event::Wheel {
                    delta: -15,
                    hires: true,
                },
            ),
            (
                feat::THUMBWHEEL,
                0,
                report(&[0, 30, 0, 0, 1]),
                Event::Thumbwheel {
                    rotation: 30,
                    status: 1,
                },
            ),
            (feat::WIRELESS_STATUS, 0, report(&[1]), Event::Reconnected),
            (feat::WIRELESS_STATUS, 0, report(&[0]), Event::Other),
        ] {
            m.dev.event(feature, function, &payload);
            assert_eq!(m.next_event(Duration::ZERO).unwrap(), Some(expected));
        }
        assert_eq!(m.next_event(Duration::ZERO).unwrap(), None);
    }
}
