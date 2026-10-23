<p align="center"><img src="assets/icon.png" width="128" alt=""></p>

# cleanlogi

A tiny menu-bar replacement for Logi Options+ on macOS. One 2 MB app, no
background services, configured with a text file. Made for the MX Master 4
over Bluetooth.

## Install

1. Download the `.dmg` from [Releases](https://github.com/saeedvaziry/cleanlogi/releases/latest), drag cleanlogi to Applications and open it.
2. Allow **Input Monitoring** and **Accessibility** when macOS asks.
3. Turn on **Open at Login** in the menu.

Needs macOS 15 or later. Quit Logi Options+ first; both can't drive the mouse at
once. [`scripts/remove-logi.sh`](scripts/remove-logi.sh) removes it and its background services.

## Configure

Menu → **Edit Settings…** opens `~/.config/cleanlogi/config.toml`. Changes apply when you save.

```toml
dpi = 1600

[buttons]
back = "back"
forward = "forward"
haptic_panel = "cmd+shift+4"

[gesture]          # hold the thumb button and move
up = "mission_control"
left = "desktop_left"
right = "desktop_right"

[thumbwheel]
mode = "native"    # horizontal scroll, or "actions" with left/right
```

Actions: any shortcut (`cmd+shift+4`, `ctrl+left`, `f11`), `run:<command>`,
`back`, `forward`, `mission_control`, `app_expose`, `show_desktop`,
`launchpad`, `desktop_left`, `desktop_right`, `play_pause`, `next_track`,
`prev_track`, `volume_up`, `volume_down`, `mute`, `middle_click`,
`smartshift_toggle`, `dpi_cycle`, `none`, `native`.

## Develop

```sh
scripts/dev.sh       # run "cleanlogi Dev" with logs
scripts/install.sh   # install a local build to /Applications
```

## License

[AGPL-3.0](LICENSE). Not affiliated with Logitech.
