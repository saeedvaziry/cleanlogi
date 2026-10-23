//! Menu-bar UI. The tray lives on the main thread (AppKit requirement); all
//! device work happens on the worker thread.

use crate::config;
use crate::mouse::WheelMode;
use crate::worker::{self, Command, Status};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

enum UserEvent {
    Status(Box<Status>),
    Menu(MenuEvent),
    Exit,
}

const HAPTIC_LEVELS: [(&str, Option<u8>); 4] = [
    ("Off", None),
    ("Low", Some(30)),
    ("Medium", Some(60)),
    ("High", Some(100)),
];

struct Ui {
    tray: TrayIcon,
    header: MenuItem,
    dpi: Vec<(u16, CheckMenuItem)>,
    ratchet: CheckMenuItem,
    freespin: CheckMenuItem,
    smartshift: CheckMenuItem,
    haptics: Vec<(Option<u8>, CheckMenuItem)>,
    hosts: Vec<CheckMenuItem>,
    edit: MenuId,
    reload: MenuId,
    login: CheckMenuItem,
    quit: MenuId,
}

pub fn run() -> Result<()> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    let proxy = event_loop.create_proxy();
    let status_proxy = proxy.clone();
    let worker = std::thread::spawn(move || {
        worker::run(rx, |s| {
            let _ = status_proxy.send_event(UserEvent::Status(Box::new(s)));
        });
        let _ = status_proxy.send_event(UserEvent::Exit);
    });
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e));
    }));

    let presets = config::Config::load()
        .map(|c| c.dpi_presets)
        .unwrap_or_default();
    let mut ui: Option<Ui> = None;
    let mut worker = Some(worker);
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => match Ui::build(&presets) {
                Ok(u) => ui = Some(u),
                Err(e) => {
                    worker::log!("cannot create menu bar item: {e:#}");
                    *flow = ControlFlow::Exit;
                }
            },
            Event::UserEvent(UserEvent::Status(s)) => {
                if let Some(u) = &ui {
                    u.show(&s);
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if let Some(u) = &ui {
                    u.clicked(&e.id, &tx);
                }
            }
            Event::UserEvent(UserEvent::Exit) => {
                if let Some(w) = worker.take() {
                    let _ = w.join();
                }
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}

impl Ui {
    fn build(presets: &[u16]) -> Result<Self> {
        let menu = Menu::new();
        let header = MenuItem::new("Looking for mouse…", false, None);

        let dpi: Vec<_> = presets
            .iter()
            .map(|&d| (d, CheckMenuItem::new(format!("{d} DPI"), true, false, None)))
            .collect();
        let dpi_menu = Submenu::new("Pointer Speed", true);
        for (_, item) in &dpi {
            dpi_menu.append(item)?;
        }

        let ratchet = CheckMenuItem::new("Ratchet", true, false, None);
        let freespin = CheckMenuItem::new("Free Spin", true, false, None);
        let smartshift = CheckMenuItem::new("SmartShift", true, false, None);
        let scroll_menu = Submenu::with_items(
            "Scroll Wheel",
            true,
            &[
                &ratchet,
                &freespin,
                &PredefinedMenuItem::separator(),
                &smartshift,
            ],
        )?;

        let haptics: Vec<_> = HAPTIC_LEVELS
            .iter()
            .map(|&(name, lvl)| (lvl, CheckMenuItem::new(name, true, false, None)))
            .collect();
        let haptic_menu = Submenu::new("Haptic Feedback", true);
        for (_, item) in &haptics {
            haptic_menu.append(item)?;
        }

        let hosts: Vec<_> = (1..=3)
            .map(|i| CheckMenuItem::new(format!("Computer {i}"), false, false, None))
            .collect();
        let host_menu = Submenu::new("Easy-Switch", true);
        for item in &hosts {
            host_menu.append(item)?;
        }

        let edit = MenuItem::new("Edit Settings…", true, None);
        let reload = MenuItem::new("Reload Settings", true, None);
        let login = CheckMenuItem::new("Open at Login", true, launch_agent_path().exists(), None);
        let quit = MenuItem::new(format!("Quit {}", crate::APP_NAME), true, None);

        menu.append_items(&[
            &header,
            &PredefinedMenuItem::separator(),
            &dpi_menu,
            &scroll_menu,
            &haptic_menu,
            &host_menu,
            &PredefinedMenuItem::separator(),
            &edit,
            &reload,
            &login,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon_templated(mouse_icon())
            .with_tooltip(crate::APP_NAME)
            .build()?;

        Ok(Self {
            tray,
            header,
            dpi,
            ratchet,
            freespin,
            smartshift,
            haptics,
            hosts,
            edit: edit.id().clone(),
            reload: reload.id().clone(),
            login,
            quit: quit.id().clone(),
        })
    }

    fn show(&self, s: &Status) {
        let c = &s.config;
        let battery = s
            .battery
            .map(|b| format!("{}%{}", b.percent, if b.charging { " ⚡" } else { "" }));
        let header = match (&s.device, &battery) {
            (Some(name), Some(b)) => format!("{name} — {b}"),
            (Some(name), None) => name.clone(),
            (None, _) => "No mouse connected".into(),
        };
        self.header.set_text(&header);
        let _ = self.tray.set_tooltip(Some(&header));
        // Only surface the battery in the menu bar when it needs attention.
        let low = s.battery.filter(|b| b.percent <= 20 && !b.charging);
        self.tray
            .set_title(low.map(|b| format!("{}%", b.percent)).as_deref());

        for (d, item) in &self.dpi {
            item.set_checked(*d == c.dpi);
        }
        self.ratchet.set_checked(c.scroll.mode == "ratchet");
        self.freespin.set_checked(c.scroll.mode != "ratchet");
        self.smartshift.set_checked(c.scroll.smartshift);
        for (lvl, item) in &self.haptics {
            let on = match lvl {
                None => !c.haptics.enabled,
                Some(l) => c.haptics.enabled && *l == c.haptics.level,
            };
            item.set_checked(on);
        }
        let (count, current) = s.hosts.unwrap_or((0, 0));
        for (i, item) in self.hosts.iter().enumerate() {
            let name = s.host_names.get(i).filter(|n| !n.is_empty());
            item.set_text(match name {
                Some(n) => format!("{} · {n}", i + 1),
                None => format!("Computer {}", i + 1),
            });
            item.set_enabled(s.device.is_some() && (i as u8) < count);
            item.set_checked(s.device.is_some() && i as u8 == current);
        }
    }

    fn clicked(&self, id: &MenuId, tx: &Sender<Command>) {
        let send = |c| {
            let _ = tx.send(c);
        };
        if let Some((d, _)) = self.dpi.iter().find(|(_, i)| i.id() == id) {
            send(Command::SetDpi(*d));
        } else if id == self.ratchet.id() {
            send(Command::SetScrollMode(WheelMode::Ratchet));
        } else if id == self.freespin.id() {
            send(Command::SetScrollMode(WheelMode::FreeSpin));
        } else if id == self.smartshift.id() {
            send(Command::SetSmartShift(self.smartshift.is_checked()));
        } else if let Some((lvl, _)) = self.haptics.iter().find(|(_, i)| i.id() == id) {
            send(Command::SetHapticLevel(*lvl));
        } else if let Some(i) = self.hosts.iter().position(|h| h.id() == id) {
            send(Command::ChangeHost(i as u8));
        } else if *id == self.reload {
            send(Command::Reload);
        } else if *id == self.edit {
            let _ = std::process::Command::new("open")
                .arg("-t")
                .arg(config::path())
                .spawn();
        } else if id == self.login.id() {
            let r = if self.login.is_checked() {
                install_launch_agent()
            } else {
                remove_launch_agent()
            };
            if let Err(e) = r {
                worker::log!("open at login: {e:#}");
            }
            self.login.set_checked(launch_agent_path().exists());
        } else if *id == self.quit {
            send(Command::Quit);
        }
    }
}

const AGENT_LABEL: &str = crate::BUNDLE_ID;

pub fn launch_agent_path() -> PathBuf {
    config::dir()
        .parent()
        .and_then(|p| p.parent())
        .map(|home| {
            home.join("Library/LaunchAgents")
                .join(format!("{AGENT_LABEL}.plist"))
        })
        .unwrap_or_default()
}

pub fn install_launch_agent() -> Result<()> {
    let exe = std::env::current_exe()?;
    let log = launch_agent_path()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("Logs/{AGENT_LABEL}.log"));
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{AGENT_LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        exe.display(),
        log.display()
    );
    let path = launch_agent_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, plist)?;
    Ok(())
}

pub fn remove_launch_agent() -> Result<()> {
    let path = launch_agent_path();
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// A small mouse glyph drawn at 2x (36x36) as a template image.
fn mouse_icon() -> Icon {
    const S: usize = 36;
    let mut rgba = vec![0u8; S * S * 4];
    // Body: rounded rectangle centred in the canvas.
    let (cx, cy, hw, hh, r) = (18.0f32, 18.0f32, 9.5f32, 14.5f32, 9.0f32);
    let stroke = 2.6f32;
    for y in 0..S {
        for x in 0..S {
            let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let qx = px.abs() - (hw - r);
            let qy = py.abs() - (hh - r);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let sdf = outside + qx.max(qy).min(0.0) - r;
            let mut a = (stroke / 2.0 + 0.5 - sdf.abs()).clamp(0.0, 1.0);
            // Button split and scroll wheel.
            if px.abs() < 1.1 && py > -hh && py < -5.0 {
                a = a.max((1.1 - px.abs()).clamp(0.0, 1.0));
            }
            let i = (y * S + x) * 4;
            rgba[i + 3] = (a * 255.0) as u8;
        }
    }
    Icon::from_rgba(rgba, S as u32, S as u32).expect("valid icon")
}
