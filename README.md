<p align="center"><img src="assets/icon.png" width="128" alt=""></p>

# CleanLogi

A tiny menu-bar replacement for Logi Options+ on macOS. One 2 MB app, no
background services, configured with a text file. Made for the MX Master 4
over Bluetooth.

## Install

1. Download the `.dmg` from [Releases](https://github.com/saeedvaziry/cleanlogi/releases/latest), drag CleanLogi to Applications and open it.
2. Allow **Input Monitoring** and **Accessibility** when macOS asks.
3. Turn on **Open at Login** in the menu.

CleanLogi checks for new releases every hour and downloads them in the
background; pick **Restart to Update** in the menu to install.

Needs macOS 15 or later. Quit Logi Options+ first; both can't drive the mouse at
once. [`scripts/remove-logi.sh`](scripts/remove-logi.sh) removes it and its background services.

## Configure

Menu → **Edit Settings…** opens `~/.config/cleanlogi/config.toml`. Every
setting is explained in the file, and changes apply when you save. For example:

```toml
[scroll]
speed = 30              # 1 to 100, higher scrolls further per notch

[thumbwheel]
mode = "scroll"         # horizontal scrolling, or "actions"
speed = 40

[gesture]               # hold the thumb button and move
left = "desktop_left"
right = "desktop_right"
```

## Develop

```sh
scripts/dev.sh       # run "CleanLogi Dev" with logs
scripts/install.sh   # install a local build to /Applications
```

## License

[AGPL-3.0](LICENSE). Not affiliated with Logitech.
