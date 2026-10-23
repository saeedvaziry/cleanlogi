//! Minimal HID++ 2.0 transport for Logitech devices on macOS.
//!
//! Over Bluetooth LE the device exposes HID++ on vendor usage page 0xFF43,
//! usage 0x0202, and only accepts long (20-byte, report ID 0x11) reports.
//! The device index for a directly connected (non-receiver) device is 0xFF.

use anyhow::{Context, Result, anyhow, bail};
use hidapi::{HidApi, HidDevice};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const LOGITECH_VID: u16 = 0x046D;
pub const HIDPP_USAGE_PAGE: u16 = 0xFF43;
pub const HIDPP_USAGE: u16 = 0x0202;

const REPORT_SHORT: u8 = 0x10;
const REPORT_LONG: u8 = 0x11;
const LONG_LEN: usize = 20;
const DEVICE_INDEX_DIRECT: u8 = 0xFF;
const ERROR_FEATURE_INDEX: u8 = 0xFF;
/// Our software ID (low nibble of byte 3). Device events use 0, and other
/// host software uses its own IDs, so replies to us are unambiguous.
const SW_ID: u8 = 0x0A;
const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);

/// A message received from the device: either a reply to a request or an
/// unsolicited event (software ID 0).
#[derive(Debug, Clone)]
pub struct Message {
    pub feature_index: u8,
    pub function: u8,
    pub sw_id: u8,
    pub params: [u8; 16],
}

impl Message {
    fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 4 || (buf[0] != REPORT_LONG && buf[0] != REPORT_SHORT) {
            return None;
        }
        let mut params = [0u8; 16];
        let n = (buf.len() - 4).min(16);
        params[..n].copy_from_slice(&buf[4..4 + n]);
        Some(Self {
            feature_index: buf[2],
            function: buf[3] >> 4,
            sw_id: buf[3] & 0x0F,
            params,
        })
    }
}

/// Information on a HID++ interface found during enumeration.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: std::ffi::CString,
    pub name: String,
    pub product_id: u16,
}

pub fn enumerate(api: &HidApi) -> Vec<DeviceInfo> {
    api.device_list()
        .filter(|d| {
            d.vendor_id() == LOGITECH_VID
                && d.usage_page() == HIDPP_USAGE_PAGE
                && d.usage() == HIDPP_USAGE
        })
        .map(|d| DeviceInfo {
            path: d.path().to_owned(),
            name: d.product_string().unwrap_or("Logitech device").to_string(),
            product_id: d.product_id(),
        })
        .collect()
}

pub struct Device {
    hid: HidDevice,
    device_index: u8,
    /// Events that arrived while waiting for a reply; drained by `next_event`.
    pending: RefCell<VecDeque<Message>>,
}

impl Device {
    pub fn open(api: &HidApi, info: &DeviceInfo) -> Result<Self> {
        let hid = api
            .open_path(&info.path)
            .with_context(|| format!("opening {}", info.name))?;
        Ok(Self {
            hid,
            device_index: DEVICE_INDEX_DIRECT,
            pending: RefCell::new(VecDeque::new()),
        })
    }

    fn write(&self, feature_index: u8, function: u8, params: &[u8]) -> Result<()> {
        let mut buf = [0u8; LONG_LEN];
        buf[0] = REPORT_LONG;
        buf[1] = self.device_index;
        buf[2] = feature_index;
        buf[3] = (function << 4) | SW_ID;
        let n = params.len().min(16);
        buf[4..4 + n].copy_from_slice(&params[..n]);
        self.hid.write(&buf).context("HID write")?;
        Ok(())
    }

    /// Read the next HID++ report, or None once `deadline` passes. On macOS
    /// the handle also receives the mouse's ordinary input reports (pointer
    /// motion, buttons); those are skipped.
    fn read_raw(&self, deadline: Instant) -> Result<Option<Message>> {
        let mut buf = [0u8; 64];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let n = self
                .hid
                .read_timeout(&mut buf, left.as_millis() as i32)
                .context("HID read")?;
            if n == 0 {
                return Ok(None);
            }
            if let Some(m) = Message::parse(&buf[..n]) {
                return Ok(Some(m));
            }
        }
    }

    /// Next device event (software ID 0), waiting at most `timeout`.
    /// Replies addressed to other host software are skipped.
    pub fn next_event(&self, timeout: Duration) -> Result<Option<Message>> {
        if let Some(m) = self.pending.borrow_mut().pop_front() {
            return Ok(Some(m));
        }
        let deadline = Instant::now() + timeout;
        while let Some(m) = self.read_raw(deadline)? {
            if m.sw_id == 0 {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// Send a request and wait for its matching reply. Events arriving in the
    /// meantime are queued for `next_event`.
    pub fn request(&self, feature_index: u8, function: u8, params: &[u8]) -> Result<[u8; 16]> {
        self.write(feature_index, function, params)?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        while let Some(msg) = self.read_raw(deadline)? {
            if msg.feature_index == ERROR_FEATURE_INDEX
                && msg.params[0] == feature_index
                && msg.params[1] == ((function << 4) | SW_ID)
            {
                bail!(
                    "HID++ error {:#04x} ({}) on feature index {:#04x} fn {}",
                    msg.params[2],
                    error_name(msg.params[2]),
                    feature_index,
                    function
                );
            }
            if msg.feature_index == feature_index && msg.function == function && msg.sw_id == SW_ID
            {
                return Ok(msg.params);
            }
            if msg.sw_id == 0 {
                self.pending.borrow_mut().push_back(msg);
            }
        }
        Err(anyhow!(
            "timeout waiting for reply (feature index {:#04x} fn {})",
            feature_index,
            function
        ))
    }

    /// IRoot.getFeature: map a feature ID to its index on this device.
    /// Returns None if the device does not implement the feature.
    pub fn feature_index(&self, feature_id: u16) -> Result<Option<FeatureRef>> {
        let r = self.request(0x00, 0, &feature_id.to_be_bytes())?;
        if r[0] == 0 && feature_id != 0 {
            return Ok(None);
        }
        Ok(Some(FeatureRef {
            id: feature_id,
            index: r[0],
            flags: r[1],
            version: r[2],
        }))
    }

    /// IRoot.ping: returns the HID++ protocol version (major, minor).
    pub fn protocol_version(&self) -> Result<(u8, u8)> {
        let r = self.request(0x00, 1, &[0, 0, 0x5A])?;
        Ok((r[0], r[1]))
    }

    /// Enumerate every feature through IFeatureSet (0x0001).
    pub fn features(&self) -> Result<Vec<FeatureRef>> {
        let fs = self
            .feature_index(0x0001)?
            .ok_or_else(|| anyhow!("device has no IFeatureSet"))?;
        let count = self.request(fs.index, 0, &[])?[0];
        let mut out = vec![FeatureRef {
            id: 0,
            index: 0,
            flags: 0,
            version: 0,
        }];
        for i in 1..=count {
            let r = self.request(fs.index, 1, &[i])?;
            out.push(FeatureRef {
                id: u16::from_be_bytes([r[0], r[1]]),
                index: i,
                flags: r[2],
                version: r[3],
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FeatureRef {
    pub id: u16,
    pub index: u8,
    pub flags: u8,
    pub version: u8,
}

pub fn error_name(code: u8) -> &'static str {
    match code {
        0x01 => "unknown",
        0x02 => "invalid argument",
        0x03 => "out of range",
        0x04 => "hardware error",
        0x05 => "logitech internal",
        0x06 => "invalid feature index",
        0x07 => "invalid function",
        0x08 => "busy",
        0x09 => "unsupported",
        _ => "?",
    }
}

pub fn feature_name(id: u16) -> &'static str {
    match id {
        0x0000 => "Root",
        0x0001 => "FeatureSet",
        0x0003 => "DeviceFwVersion",
        0x0005 => "DeviceName",
        0x0007 => "DeviceFriendlyName",
        0x0011 => "PropertyAccess",
        0x0020 => "ConfigChange",
        0x0021 => "UniqueRandomId",
        0x1004 => "UnifiedBattery",
        0x1602 => "PasswordAuth",
        0x1802 => "DeviceReset",
        0x1803 => "GpioAccess",
        0x1814 => "ChangeHost",
        0x1815 => "HostsInfo",
        0x1891 => "Ota",
        0x18A1 => "LedTest",
        0x19B0 => "HapticFeedback",
        0x19C0 => "ForceSensingButton",
        0x1B04 => "ReprogControlsV4",
        0x1D4B => "WirelessDeviceStatus",
        0x1E00 => "EnableHiddenFeatures",
        0x1E02 => "ManageDeactivatableFeatures",
        0x1EB0 => "TdeAccessToNvm",
        0x2111 => "SmartShiftEnhanced",
        0x2121 => "HiResWheel",
        0x2150 => "Thumbwheel",
        0x2201 => "AdjustableDpi",
        0x2250 => "AnalysisMode",
        _ => "?",
    }
}
