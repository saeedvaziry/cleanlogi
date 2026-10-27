//! Actions a button, gesture or thumbwheel can trigger, and their macOS
//! implementations. Mission Control & co. use the same private Dock
//! notifications Logi Options+ posts (com.apple.expose.awake, ...).

use anyhow::{Result, bail};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::ffi::c_void;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Leave the control undiverted so the device handles it.
    Native,
    None,
    Key {
        code: u16,
        flags: CGEventFlags,
    },
    Dock(&'static str),
    Media(u32),
    /// Click a mouse button by number: 2 = middle, 3 = back, 4 = forward.
    Click(i64),
    Run(String),
    SmartShiftToggle,
    DpiCycle,
}

// NX_KEYTYPE_* values from IOKit/hidsystem/ev_keymap.h
const NX_SOUND_UP: u32 = 0;
const NX_SOUND_DOWN: u32 = 1;
const NX_MUTE: u32 = 7;
const NX_PLAY: u32 = 16;
const NX_NEXT: u32 = 17;
const NX_PREVIOUS: u32 = 18;

const KEY_LEFT: u16 = 0x7B;
const KEY_RIGHT: u16 = 0x7C;

impl Action {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some(cmd) = s.strip_prefix("run:") {
            return Ok(Action::Run(cmd.trim().to_string()));
        }
        Ok(match s.to_ascii_lowercase().as_str() {
            "native" | "" => Action::Native,
            "none" => Action::None,
            // Mouse buttons 4 and 5, which browsers and editors treat as
            // back / forward; a synthetic cmd+[ is ignored by Chromium.
            "back" => Action::Click(3),
            "forward" => Action::Click(4),
            "middle_click" => Action::Click(2),
            "mission_control" => Action::Dock("com.apple.expose.awake"),
            "app_expose" => Action::Dock("com.apple.expose.front.awake"),
            "show_desktop" => Action::Dock("com.apple.showdesktop.awake"),
            "launchpad" => Action::Dock("com.apple.launchpad.toggle"),
            "desktop_left" => key(KEY_LEFT, CGEventFlags::CGEventFlagControl),
            "desktop_right" => key(KEY_RIGHT, CGEventFlags::CGEventFlagControl),
            "play_pause" => Action::Media(NX_PLAY),
            "next_track" => Action::Media(NX_NEXT),
            "prev_track" => Action::Media(NX_PREVIOUS),
            "volume_up" => Action::Media(NX_SOUND_UP),
            "volume_down" => Action::Media(NX_SOUND_DOWN),
            "mute" => Action::Media(NX_MUTE),
            "smartshift_toggle" => Action::SmartShiftToggle,
            "dpi_cycle" => Action::DpiCycle,
            shortcut => parse_shortcut(shortcut)?,
        })
    }

    /// Run a host-side action. Device actions (SmartShift, DPI) are handled by
    /// the device worker and are no-ops here.
    pub fn perform(&self) {
        match self {
            Action::Key { code, flags } => post_key(*code, *flags),
            Action::Dock(name) => unsafe {
                CoreDockSendNotification(
                    CFString::new(name).as_concrete_TypeRef(),
                    std::ptr::null_mut(),
                );
            },
            Action::Media(k) => post_media_key(*k),
            Action::Click(button) => post_click(*button),
            Action::Run(cmd) => {
                let _ = std::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(cmd)
                    .spawn();
            }
            Action::Native | Action::None | Action::SmartShiftToggle | Action::DpiCycle => {}
        }
    }
}

fn key(code: u16, flags: CGEventFlags) -> Action {
    Action::Key { code, flags }
}

fn parse_shortcut(s: &str) -> Result<Action> {
    let mut flags = CGEventFlags::CGEventFlagNull;
    let mut code = None;
    for part in s.split('+').map(str::trim) {
        match part {
            "cmd" | "command" => flags |= CGEventFlags::CGEventFlagCommand,
            "shift" => flags |= CGEventFlags::CGEventFlagShift,
            "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
            "alt" | "opt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
            "fn" => flags |= CGEventFlags::CGEventFlagSecondaryFn,
            k => match keycode(k) {
                Some(c) if code.is_none() => code = Some(c),
                Some(_) => bail!("shortcut {s:?} has more than one key"),
                None => bail!("unknown key {k:?} in {s:?}"),
            },
        }
    }
    match code {
        Some(code) => Ok(Action::Key { code, flags }),
        None => bail!("shortcut {s:?} has no key"),
    }
}

/// macOS virtual key codes (HIToolbox Events.h, kVK_*).
fn keycode(name: &str) -> Option<u16> {
    const LETTERS: &[(char, u16)] = &[
        ('a', 0x00),
        ('s', 0x01),
        ('d', 0x02),
        ('f', 0x03),
        ('h', 0x04),
        ('g', 0x05),
        ('z', 0x06),
        ('x', 0x07),
        ('c', 0x08),
        ('v', 0x09),
        ('b', 0x0B),
        ('q', 0x0C),
        ('w', 0x0D),
        ('e', 0x0E),
        ('r', 0x0F),
        ('y', 0x10),
        ('t', 0x11),
        ('1', 0x12),
        ('2', 0x13),
        ('3', 0x14),
        ('4', 0x15),
        ('6', 0x16),
        ('5', 0x17),
        ('=', 0x18),
        ('9', 0x19),
        ('7', 0x1A),
        ('-', 0x1B),
        ('8', 0x1C),
        ('0', 0x1D),
        (']', 0x1E),
        ('o', 0x1F),
        ('u', 0x20),
        ('[', 0x21),
        ('i', 0x22),
        ('p', 0x23),
        ('l', 0x25),
        ('j', 0x26),
        ('\'', 0x27),
        ('k', 0x28),
        (';', 0x29),
        ('\\', 0x2A),
        (',', 0x2B),
        ('/', 0x2C),
        ('n', 0x2D),
        ('m', 0x2E),
        ('.', 0x2F),
        ('`', 0x32),
    ];
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return LETTERS.iter().find(|(l, _)| *l == c).map(|(_, k)| *k);
    }
    Some(match name {
        "return" | "enter" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "delete" | "backspace" => 0x33,
        "escape" | "esc" => 0x35,
        "forward_delete" => 0x75,
        "home" => 0x73,
        "end" => 0x77,
        "pageup" => 0x74,
        "pagedown" => 0x79,
        "left" => KEY_LEFT,
        "right" => KEY_RIGHT,
        "down" => 0x7D,
        "up" => 0x7E,
        "f1" => 0x7A,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x6D,
        "f11" => 0x67,
        "f12" => 0x6F,
        "f13" => 0x69,
        "f14" => 0x6B,
        "f15" => 0x71,
        "f16" => 0x6A,
        "f17" => 0x40,
        "f18" => 0x4F,
        "f19" => 0x50,
        "f20" => 0x5A,
        _ => return None,
    })
}

fn source() -> Option<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()
}

fn post_key(code: u16, mut flags: CGEventFlags) {
    let Some(src) = source() else { return };
    // Real keyboards flag arrow and navigation keys as keypad + fn keys;
    // the desktop-switching hotkeys only match with those flags present.
    if (0x73..=0x7E).contains(&code) {
        flags |= CGEventFlags::CGEventFlagNumericPad | CGEventFlags::CGEventFlagSecondaryFn;
    }
    for down in [true, false] {
        if let Ok(ev) = CGEvent::new_keyboard_event(src.clone(), code, down) {
            ev.set_flags(flags);
            ev.post(CGEventTapLocation::HID);
        }
    }
}

fn post_click(button: i64) {
    let Some(src) = source() else { return };
    let Ok(here) = CGEvent::new(src.clone()) else {
        return;
    };
    let at = here.location();
    for ty in [CGEventType::OtherMouseDown, CGEventType::OtherMouseUp] {
        if let Ok(ev) = CGEvent::new_mouse_event(src.clone(), ty, at, CGMouseButton::Center) {
            ev.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button);
            ev.post(CGEventTapLocation::HID);
        }
    }
}

fn post_media_key(key: u32) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSPoint;
    objc2::rc::autoreleasepool(|_| {
        for down in [true, false] {
            let state: isize = if down { 0xA } else { 0xB };
            let data1 = ((key as isize) << 16) | (state << 8);
            unsafe {
                let ev: *mut AnyObject = msg_send![
                    class!(NSEvent),
                    otherEventWithType: 14usize, // NSEventTypeSystemDefined
                    location: NSPoint::new(0.0, 0.0),
                    modifierFlags: (state << 8) as usize,
                    timestamp: 0.0f64,
                    windowNumber: 0isize,
                    context: std::ptr::null_mut::<AnyObject>(),
                    subtype: 8i16, // NX_SUBTYPE_AUX_CONTROL_BUTTONS
                    data1: data1,
                    data2: -1isize
                ];
                if ev.is_null() {
                    return;
                }
                let cg: *mut c_void = msg_send![ev, CGEvent];
                if !cg.is_null() {
                    CGEventPost(0, cg); // kCGHIDEventTap
                }
            }
        }
    });
}

/// Whether we may post synthetic input events, without prompting.
pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Whether we may post synthetic input events; prompts the user if not.
pub fn ensure_accessibility() -> bool {
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let opts = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    unsafe { AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef() as *const c_void) }
}

/// Whether we may read the mouse's HID interface (Input Monitoring);
/// prompts the user if not yet decided.
pub fn ensure_input_monitoring() -> bool {
    input_monitoring_granted() || unsafe { IOHIDRequestAccess(LISTEN_EVENT) }
}

pub fn input_monitoring_granted() -> bool {
    unsafe { IOHIDCheckAccess(LISTEN_EVENT) == 0 }
}

const LISTEN_EVENT: u32 = 1; // kIOHIDRequestTypeListenEvent

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CoreDockSendNotification(name: CFStringRef, unknown: *mut c_void);
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shortcuts() {
        let a = Action::parse("cmd+shift+4").unwrap();
        assert_eq!(
            a,
            Action::Key {
                code: 0x15,
                flags: CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagShift
            }
        );
        assert!(Action::parse("cmd+nope").is_err());
        assert_eq!(
            Action::parse("run: echo hi").unwrap(),
            Action::Run("echo hi".into())
        );
        assert_eq!(Action::parse("Native").unwrap(), Action::Native);
        assert_eq!(Action::parse("back").unwrap(), Action::Click(3));
        assert_eq!(Action::parse("forward").unwrap(), Action::Click(4));
    }
}
