mod actions;
mod config;
mod hidpp;
mod mouse;
mod ui;
mod worker;

use anyhow::{Context, Result, bail};
use hidapi::HidApi;
use std::sync::atomic::Ordering;

/// Dev builds (scripts/dev.sh) get their own bundle ID, so their macOS
/// permissions and login item never collide with an installed release.
pub const DEV: bool = option_env!("CLEANLOGI_DEV").is_some();
pub const BUNDLE_ID: &str = if DEV {
    "com.cleanlogi.dev"
} else {
    "com.cleanlogi.app"
};
pub const APP_NAME: &str = if DEV { "cleanlogi Dev" } else { "cleanlogi" };

const USAGE: &str = "\
usage: cleanlogi [command]

  (none)        run the menu-bar app
  features      list HID++ features of connected mice
  dump          print raw state from read-only feature getters
  haptic NAME   play a haptic waveform (e.g. sharp_state_change, happy_alert)
  install       open cleanlogi at login
  uninstall     stop opening cleanlogi at login
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            install_signal_handlers();
            if !actions::ensure_input_monitoring() {
                worker::log!(
                    "Input Monitoring access is needed to talk to the mouse; grant it in \
                     System Settings → Privacy & Security → Input Monitoring"
                );
                // A new grant only applies to a fresh process: exit once it
                // lands and let launchd (KeepAlive) start us again.
                std::thread::spawn(|| {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        if actions::input_monitoring_granted() {
                            worker::log!("Input Monitoring granted, restarting");
                            std::process::exit(1);
                        }
                    }
                });
            }
            if !actions::ensure_accessibility() {
                worker::log!(
                    "Accessibility access is needed to send shortcuts; grant it in \
                     System Settings → Privacy & Security → Accessibility"
                );
            }
            ui::run()
        }
        Some("features") => with_mouse(|dev| {
            let (maj, min) = dev.protocol_version()?;
            println!("HID++ {maj}.{min}");
            for f in dev.features()? {
                println!(
                    "  [{:#04x}] {:#06x} v{} flags={:#04x} {}",
                    f.index,
                    f.id,
                    f.version,
                    f.flags,
                    hidpp::feature_name(f.id)
                );
            }
            Ok(())
        }),
        Some("dump") => with_mouse(dump),
        Some("haptic") => {
            let name = args.get(1).context("haptic needs a waveform name")?;
            let id = mouse::waveform::from_name(name)
                .or_else(|| name.parse().ok())
                .with_context(|| format!("unknown waveform {name}"))?;
            with_mouse(|dev| mouse::Mouse::new(dev)?.play_waveform(id))
        }
        Some("install") => {
            ui::install_launch_agent()?;
            println!("installed {}", ui::launch_agent_path().display());
            Ok(())
        }
        Some("uninstall") => ui::remove_launch_agent(),
        Some(_) => {
            eprint!("{USAGE}");
            bail!("unknown command")
        }
    }
}

fn with_mouse(f: impl Fn(hidpp::Device) -> Result<()>) -> Result<()> {
    let api = HidApi::new()?;
    let devices = hidpp::enumerate(&api);
    if devices.is_empty() {
        bail!("no Logitech HID++ devices found");
    }
    for info in &devices {
        println!("== {} (pid {:#06x})", info.name, info.product_id);
        f(hidpp::Device::open(&api, info)?)?;
    }
    Ok(())
}

fn install_signal_handlers() {
    extern "C" fn on_signal(_: libc::c_int) {
        worker::SHUTDOWN.store(true, Ordering::Relaxed);
    }
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Call read-only getters and print raw replies.
fn dump(dev: hidpp::Device) -> Result<()> {
    let get = |id: u16, func: u8, params: &[u8]| -> Result<()> {
        let Some(f) = dev.feature_index(id)? else {
            return Ok(());
        };
        match dev.request(f.index, func, params) {
            Ok(r) => println!("{id:04x} fn{func} {:<10} -> {}", hex(params), hex(&r)),
            Err(e) => println!("{id:04x} fn{func} {:<10} -> {e}", hex(params)),
        }
        Ok(())
    };
    get(0x0003, 0, &[])?;
    get(0x1004, 1, &[])?;
    get(0x1814, 0, &[])?;
    get(0x1815, 0, &[])?;
    for h in 0..3 {
        get(0x1815, 1, &[h])?;
        get(0x1815, 3, &[h, 0])?;
        get(0x1815, 3, &[h, 14])?;
    }
    get(0x19B0, 0, &[])?;
    get(0x19B0, 1, &[])?;
    get(0x19C0, 1, &[0])?;
    get(0x19C0, 2, &[0])?;
    get(0x2111, 0, &[])?;
    get(0x2111, 1, &[])?;
    get(0x2121, 1, &[])?;
    get(0x2150, 0, &[])?;
    get(0x2150, 1, &[])?;
    get(0x2201, 1, &[0])?;
    get(0x2201, 2, &[0])?;
    if let Some(f) = dev.feature_index(0x1B04)? {
        let n = dev.request(f.index, 0, &[])?[0];
        for i in 0..n {
            let r = dev.request(f.index, 1, &[i])?;
            let rep = dev.request(f.index, 2, &r[0..2])?;
            println!(
                "1b04 cid {:#06x} flags {:#04x} | reporting {}",
                u16::from_be_bytes([r[0], r[1]]),
                r[4],
                hex(&rep[..6])
            );
        }
    }
    Ok(())
}
